//! The content blocker in the engine's request path.
//!
//! [`Blocking`] implements wubble-engine's [`ContentFilter`] on top of
//! wubble-blocker (Brave's adblock-rust). CEF asks it about every request on
//! its IO thread, so everything [`ContentFilter::check`] reads is shared and
//! read-only: the compiled [`Blocker`] behind an `RwLock<Arc<_>>` that a list
//! update swaps in one pointer move, the site overrides, the on/off flag.
//! Nothing here waits on the UI thread.
//!
//! Lists come from `<profile>/blocklists/*.txt` (plus `resources.json`), the
//! compiled engine is cached in `<profile>/blocker-engine.bin`. Nothing is
//! downloaded unless `blocker.lists_auto_update` is on (TODO.md, Decided).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wubble_blocker::{
    Blocker, DEFAULT_LISTS, Decision, NetRequest, ResourceType, Shields, SiteOverrides,
};
use wubble_engine::filter::{ContentFilter, RequestAction, RequestInfo, RequestKind};
use wubble_store::{BlockerEnabled, BlockerExceptions, BlockerListsAutoUpdate, Setting};

/// The list directory inside a profile.
const LISTS_DIR: &str = "blocklists";
/// The compiled-engine cache inside a profile.
const CACHE_FILE: &str = "blocker-engine.bin";
/// How often the update thread looks for lists that are due. The lists'
/// own intervals are days; this only bounds how late a due list is fetched.
const UPDATE_CHECK: Duration = Duration::from_secs(6 * 60 * 60);

/// The blocker settings, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockerSettings {
    pub enabled: bool,
    /// Hosts the blocker skips (with their subdomains).
    pub exceptions: Vec<String>,
    pub lists_auto_update: bool,
}

impl Default for BlockerSettings {
    fn default() -> Self {
        Self {
            enabled: BlockerEnabled::default(),
            exceptions: BlockerExceptions::default(),
            lists_auto_update: BlockerListsAutoUpdate::default(),
        }
    }
}

/// Fetches one URL; the network is reached only through this.
type Fetch = fn(&str) -> Result<String, String>;

/// The blocker shared with CEF's IO thread, and the per-view counts of
/// blocked requests for the shell to show.
pub struct Blocking {
    blocker: RwLock<Arc<Blocker>>,
    overrides: RwLock<SiteOverrides>,
    enabled: AtomicBool,
    /// Requests blocked per view ([`wubble_engine::view::View::browser_id`])
    /// since its last main-frame request.
    blocked: Mutex<HashMap<i32, u32>>,
}

impl Blocking {
    /// A blocker with no lists (allows everything) until [`Self::start`].
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            blocker: RwLock::new(Arc::new(Blocker::from_lists(&[]))),
            overrides: RwLock::new(SiteOverrides::default()),
            enabled: AtomicBool::new(BlockerEnabled::default()),
            blocked: Mutex::new(HashMap::new()),
        })
    }

    /// Applies the stored settings and loads the lists of `profile`: from
    /// the cache at once when it matches them, else compiled on a
    /// background thread. With `lists_auto_update` on, that thread also
    /// fetches due lists now and every [`UPDATE_CHECK`].
    pub fn start(self: &Arc<Self>, profile: &Path, settings: &BlockerSettings) {
        self.start_with(profile, settings, wubble_blocker::fetch_http);
    }

    /// [`Self::start`] with the fetcher given; returns the thread, if one runs.
    fn start_with(
        self: &Arc<Self>,
        profile: &Path,
        settings: &BlockerSettings,
        fetch: Fetch,
    ) -> Option<JoinHandle<()>> {
        let (dir, cache) = paths(profile);
        let compiled = self.load_cached(settings, &dir, &cache);
        let update = settings.lists_auto_update;
        if compiled && !update {
            return None;
        }
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("wubble-blocker".into())
            .spawn(move || {
                if !compiled {
                    this.replace_blocker(Blocker::compile_dir(&dir, &cache));
                }
                if update {
                    loop {
                        this.update_once(&dir, &cache, fetch);
                        std::thread::sleep(UPDATE_CHECK);
                    }
                }
            });
        spawned
            .inspect_err(|e| eprintln!("wubble: blocker lists not loaded: {e}"))
            .ok()
    }

    /// The settings, then the cached blocker if it matches the lists.
    /// Returns whether the lists are loaded.
    fn load_cached(&self, settings: &BlockerSettings, dir: &Path, cache: &Path) -> bool {
        self.set_enabled(settings.enabled);
        self.set_exceptions(&settings.exceptions);
        match Blocker::from_cache(dir, cache) {
            Some(blocker) => {
                self.replace_blocker(blocker);
                true
            }
            None => false,
        }
    }

    /// Fetches the due lists into `dir`; recompiles and swaps if any changed.
    fn update_once(&self, dir: &Path, cache: &Path, fetch: Fetch) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if !wubble_blocker::refresh(dir, DEFAULT_LISTS, now, &fetch).is_empty() {
            self.replace_blocker(Blocker::compile_dir(dir, cache));
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The hosts the blocker skips (`settings::BLOCKER_EXCEPTIONS`).
    pub fn set_exceptions(&self, hosts: &[String]) {
        let mut overrides = SiteOverrides::default();
        for host in hosts {
            overrides.insert(host);
        }
        self.set_overrides(overrides);
    }

    pub fn set_overrides(&self, overrides: SiteOverrides) {
        *self
            .overrides
            .write()
            .unwrap_or_else(PoisonError::into_inner) = overrides;
    }

    /// `--smoke block`: only `list` blocks, shields on, nothing read from disk or fetched.
    pub fn use_list(&self, list: &str) {
        self.set_enabled(true);
        self.set_overrides(SiteOverrides::default());
        self.replace_blocker(Blocker::from_lists(&[list]));
    }

    /// Swaps in a new blocker; requests already being checked finish on the
    /// old one. The lock is held for the pointer move only: the old engine
    /// is dropped after it is released.
    pub fn replace_blocker(&self, blocker: Blocker) {
        let new = Arc::new(blocker);
        let old = std::mem::replace(
            &mut *self.blocker.write().unwrap_or_else(PoisonError::into_inner),
            new,
        );
        drop(old);
    }

    /// Requests blocked in a view since its last main-frame request.
    pub fn blocked(&self, browser_id: i32) -> u32 {
        self.counts().get(&browser_id).copied().unwrap_or(0)
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, HashMap<i32, u32>> {
        self.blocked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn current(&self) -> Arc<Blocker> {
        self.blocker
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn shields_for(&self, page_url: &str) -> Shields {
        if !self.enabled.load(Ordering::Relaxed) || !is_web(page_url) {
            return Shields::Off;
        }
        self.overrides
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .shields_for(page_url)
    }
}

/// Only web traffic is filtered: not `file:`, `data:`, `chrome:` and the like.
fn is_web(url: &str) -> bool {
    ["http://", "https://", "ws://", "wss://"]
        .iter()
        .any(|scheme| {
            url.get(..scheme.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
        })
}

impl ContentFilter for Blocking {
    // Runs on CEF's IO thread and must not panic: the release profile aborts
    // on panic, so welding's catch-and-allow does not hold there. Hence the
    // poison-tolerant locks and no `expect`, `unwrap` or indexing on this path.
    fn check(&self, request: &RequestInfo<'_>) -> RequestAction {
        if request.kind == RequestKind::Document {
            self.counts().insert(request.browser_id, 0);
        }
        if !is_web(request.url) {
            return RequestAction::Allow;
        }
        // A page without a web URL (a bundled or `file:` page) is judged by
        // the request alone.
        let top = if is_web(request.top_url) {
            request.top_url
        } else {
            request.url
        };
        let net = NetRequest {
            url: request.url,
            source_url: top,
            resource_type: resource_type(request.kind),
        };
        match self.current().decide(net, self.shields_for(top)) {
            Decision::Allow => RequestAction::Allow,
            Decision::Rewrite { url } => RequestAction::Redirect(url),
            // A `$redirect` resource is a `data:` URL, which Chromium may
            // refuse as a redirect target for a web request (not verified);
            // blocked until it is served through a resource handler (TODO.md, M4).
            Decision::Block | Decision::Redirect { .. } => {
                *self.counts().entry(request.browser_id).or_insert(0) += 1;
                RequestAction::Block
            }
        }
    }

    fn page_script(&self, top_url: &str) -> Option<String> {
        let shields = self.shields_for(top_url);
        if shields == Shields::Off {
            return None;
        }
        self.current().cosmetic(top_url, shields).page_script()
    }
}

/// The engine's request kinds are the blocker's resource types; the two
/// enums exist because wubble-engine may not depend on wubble-blocker.
fn resource_type(kind: RequestKind) -> ResourceType {
    match kind {
        RequestKind::Document => ResourceType::Document,
        RequestKind::Subdocument => ResourceType::Subdocument,
        RequestKind::Stylesheet => ResourceType::Stylesheet,
        RequestKind::Script => ResourceType::Script,
        RequestKind::Image => ResourceType::Image,
        RequestKind::Font => ResourceType::Font,
        RequestKind::Object => ResourceType::Object,
        RequestKind::Media => ResourceType::Media,
        RequestKind::Xhr => ResourceType::Xhr,
        RequestKind::Ping => ResourceType::Ping,
        RequestKind::Other => ResourceType::Other,
    }
}

/// The list directory and the cache inside `profile`.
fn paths(profile: &Path) -> (PathBuf, PathBuf) {
    (profile.join(LISTS_DIR), profile.join(CACHE_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in test list. Tests only: no list ships with wubble.
    const TEST_LIST: &str = "\
||ads.example^
@@||ads.example/ok^
||tracker.net^$third-party
||img.test^$image,redirect=1x1.gif
||q.test^$removeparam=utm
example.com##.sponsor
";

    const RES: &str = r#"[{"name":"1x1.gif","aliases":[],"kind":{"mime":"image/gif"},"content":"R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"}]"#;

    fn blocking(list: &str) -> Arc<Blocking> {
        let b = Blocking::new();
        b.replace_blocker(
            Blocker::from_lists(&[list])
                .with_resources_json(RES)
                .unwrap(),
        );
        b
    }

    const SITE: &str = "https://site.org/";

    fn req<'a>(url: &'a str, top_url: &'a str, kind: RequestKind) -> RequestInfo<'a> {
        RequestInfo {
            url,
            top_url,
            kind,
            browser_id: 1,
        }
    }

    fn check(b: &Blocking, url: &str, top: &str, kind: RequestKind) -> RequestAction {
        b.check(&req(url, top, kind))
    }

    use RequestAction::{Allow, Block, Redirect};
    use RequestKind::*;

    #[test]
    fn decisions_follow_the_test_list() {
        let b = blocking(TEST_LIST);
        assert_eq!(check(&b, "https://ads.example/x.js", SITE, Script), Block);
        assert_eq!(
            check(&b, "https://ads.example/ok/x.js", SITE, Script),
            Allow
        );
        assert_eq!(check(&b, "https://fine.test/x.js", SITE, Script), Allow);
        assert_eq!(
            check(
                &b,
                "https://q.test/?utm=1&a=2",
                "https://q.test/?utm=1&a=2",
                Document
            ),
            Redirect("https://q.test/?a=2".into())
        );
        // A `$redirect` data URL is blocked, not redirected.
        assert_eq!(check(&b, "https://img.test/a.png", SITE, Image), Block);
    }

    #[test]
    fn first_and_third_party_use_the_top_page() {
        let b = blocking(TEST_LIST);
        let url = "https://tracker.net/p.js";
        assert_eq!(check(&b, url, SITE, Script), Block);
        assert_eq!(check(&b, url, "https://www.tracker.net/", Script), Allow);
        // No web top page: judged by the request alone, so first party.
        assert_eq!(check(&b, url, "file:///page.html", Script), Allow);
    }

    #[test]
    fn non_web_requests_are_allowed() {
        let b = blocking("*\n|data:\n");
        assert_eq!(check(&b, "data:text/plain,ads", SITE, Script), Allow);
        assert_eq!(check(&b, "file:///ads.js", SITE, Script), Allow);
        assert_eq!(check(&b, "chrome://gpu", SITE, Document), Allow);
    }

    /// Each request kind reaches adblock as its own resource type.
    #[test]
    fn every_request_kind_maps_to_its_filter_type() {
        let table = [
            (Subdocument, "subdocument"),
            (Stylesheet, "stylesheet"),
            (Script, "script"),
            (Image, "image"),
            (Font, "font"),
            (Object, "object"),
            (Media, "media"),
            (Xhr, "xmlhttprequest"),
            (Ping, "ping"),
            (Other, "other"),
        ];
        for (kind, option) in table {
            let b = blocking(&format!("||t.test^${option}\n"));
            assert_eq!(check(&b, "https://t.test/a", SITE, kind), Block, "{kind:?}");
            let other = if kind == Script { Image } else { Script };
            assert_eq!(
                check(&b, "https://t.test/a", SITE, other),
                Allow,
                "{kind:?}"
            );
        }
        // Documents: `$removeparam` acts on them only.
        let b = blocking("||q.test^$removeparam=utm,document\n");
        assert!(matches!(
            check(
                &b,
                "https://q.test/?utm=1",
                "https://q.test/?utm=1",
                Document
            ),
            Redirect(_)
        ));
    }

    #[test]
    fn the_allowlist_and_the_switch_turn_blocking_off() {
        let b = blocking(TEST_LIST);
        let mut o = SiteOverrides::default();
        o.insert("site.org");
        b.set_overrides(o);
        let ads = "https://ads.example/x.js";
        assert_eq!(check(&b, ads, "https://site.org/", Script), Allow);
        assert_eq!(check(&b, ads, "https://sub.site.org/", Script), Allow);
        assert_eq!(check(&b, ads, "https://notsite.org/", Script), Block);
        b.set_enabled(false);
        assert_eq!(check(&b, ads, "https://notsite.org/", Script), Allow);
        assert_eq!(b.page_script("https://example.com/"), None);
    }

    #[test]
    fn blocked_requests_are_counted_per_view_and_reset_by_a_new_page() {
        let b = blocking(TEST_LIST);
        let ads = "https://ads.example/x.js";
        let on = |id| RequestInfo {
            browser_id: id,
            ..req(ads, SITE, Script)
        };
        b.check(&on(1));
        b.check(&on(1));
        b.check(&on(2));
        b.check(&RequestInfo {
            browser_id: 1,
            ..req("https://fine.test/", SITE, Script)
        });
        assert_eq!((b.blocked(1), b.blocked(2), b.blocked(3)), (2, 1, 0));
        b.check(&RequestInfo {
            browser_id: 1,
            ..req("https://next.test/", "https://next.test/", Document)
        });
        assert_eq!((b.blocked(1), b.blocked(2)), (0, 1));
    }

    #[test]
    fn the_smoke_list_blocks_its_host_from_a_bundled_page_and_nothing_else() {
        let b = Blocking::new();
        b.set_enabled(false);
        b.use_list(crate::block_smoke::BLOCK_LIST);
        let bundled = "data:text/html,page";
        let hit = |url| check(&b, url, bundled, Image);
        assert_eq!(hit("https://blocked.wubble.test/ad.png"), Block);
        assert_eq!(hit("https://sub.blocked.wubble.test/ad.js"), Block);
        assert_eq!(hit("https://other.test/ad.png"), Allow);
        assert_eq!(hit("data:image/gif;base64,R0lGODlhAQABAAAAACw="), Allow);
    }

    #[test]
    fn a_replaced_blocker_decides_the_next_request() {
        let b = blocking(TEST_LIST);
        let url = "https://new.test/x.js";
        assert_eq!(check(&b, url, SITE, Script), Allow);
        b.replace_blocker(Blocker::from_lists(&["||new.test^\n"]));
        assert_eq!(check(&b, url, SITE, Script), Block);
    }

    #[test]
    fn cosmetic_rules_are_selected_per_host() {
        let b = blocking(TEST_LIST);
        let script = b.page_script("https://example.com/a").unwrap();
        assert!(script.contains(".sponsor{display:none!important}"));
        assert_eq!(b.page_script("https://other.org/"), None);
        assert_eq!(b.page_script("file:///example.com/"), None);
        let mut o = SiteOverrides::default();
        o.insert("example.com");
        b.set_overrides(o);
        assert_eq!(b.page_script("https://example.com/a"), None);
    }

    struct Profile(PathBuf);

    impl Profile {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("wubble-blocking-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn no_network(url: &str) -> Result<String, String> {
        panic!("fetched {url} with auto-update off")
    }

    #[test]
    fn start_reads_settings_and_profile_lists_without_fetching() {
        let p = Profile::new("start");
        let (dir, _) = paths(&p.0);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mine.txt"), "||ads.example^\n").unwrap();
        let settings = BlockerSettings {
            exceptions: vec!["site.org".into()],
            ..BlockerSettings::default()
        };
        assert!(!settings.lists_auto_update);
        let ads = "https://ads.example/x.js";
        // No cache yet: compiled on a thread that never fetches.
        let b = Blocking::new();
        let compile = b.start_with(&p.0, &settings, no_network).expect("compiles");
        compile.join().expect("no fetch");
        assert_eq!(check(&b, ads, "https://other.org/", Script), Block);
        assert_eq!(check(&b, ads, "https://site.org/", Script), Allow);
        // The cache matches now: loaded at once, no thread.
        let b = Blocking::new();
        assert!(b.start_with(&p.0, &settings, no_network).is_none());
        assert_eq!(check(&b, ads, "https://other.org/", Script), Block);
    }

    #[test]
    fn an_update_fetches_lists_and_swaps_the_blocker() {
        let p = Profile::new("update");
        let (dir, cache) = paths(&p.0);
        let b = Blocking::new();
        fn fake(url: &str) -> Result<String, String> {
            Ok(if url.ends_with(".json") {
                "[]".into()
            } else {
                "||ads.example^\n".into()
            })
        }
        b.update_once(&dir, &cache, fake);
        assert!(dir.join("easylist.txt").exists());
        assert_eq!(check(&b, "https://ads.example/x.js", SITE, Script), Block);
        // Everything is fresh now: a second update fetches nothing.
        b.update_once(&dir, &cache, no_network);
    }
}
