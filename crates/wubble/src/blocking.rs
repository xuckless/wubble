//! The content blocker in the engine's request path.
//!
//! [`Blocking`] implements wubble-engine's [`ContentFilter`] on top of
//! wubble-blocker (Brave's adblock-rust). CEF asks it about every request on
//! its IO thread, so everything [`ContentFilter::check`] reads is shared and
//! read-only: the compiled [`Blocker`] behind an `RwLock<Arc<_>>` that a list
//! update swaps in one pointer move, the site overrides, the on/off flag.
//! Nothing here waits on the UI thread.
//!
//! Lists are uBlock Origin's default set (`wubble_blocker::catalog`): the
//! snapshot bundled in the binary, or the copy downloaded into
//! `<profile>/blocklists` (plus the user's own `*.txt` there). The compiled
//! engine is cached in `<profile>/blocker-engine.bin`. Nothing is downloaded
//! unless `blocker.lists_auto_update` is on (TODO.md, Decided).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wubble_blocker::catalog::Selection;
use wubble_blocker::{
    Blocker, Decision, Due, NetRequest, ResourceType, Shields, SiteOverrides, Source,
};
use wubble_engine::filter::{ContentFilter, RequestAction, RequestInfo, RequestKind};
use wubble_store::{
    BlockerCookieBanners, BlockerCustomLists, BlockerEnabled, BlockerExceptions, BlockerLists,
    BlockerListsAutoUpdate, BlockerStripParams, Setting,
};

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
    pub cookie_banners: bool,
    pub strip_params: bool,
    pub custom_lists: Vec<String>,
    /// Catalog lists switched from their default.
    pub lists: BTreeMap<String, u32>,
}

impl Default for BlockerSettings {
    fn default() -> Self {
        Self {
            enabled: BlockerEnabled::default(),
            exceptions: BlockerExceptions::default(),
            lists_auto_update: BlockerListsAutoUpdate::default(),
            cookie_banners: BlockerCookieBanners::default(),
            strip_params: BlockerStripParams::default(),
            custom_lists: BlockerCustomLists::default(),
            lists: BlockerLists::default(),
        }
    }
}

impl BlockerSettings {
    /// Which lists these settings turn on.
    pub fn selection(&self) -> Selection {
        Selection {
            lists: self.lists.clone(),
            cookie_banners: self.cookie_banners,
            strip_params: self.strip_params,
            custom: self.custom_lists.clone(),
        }
    }
}

/// Fetches one URL; the network is reached only through this.
type Fetch = fn(&str) -> Result<String, String>;

/// What was blocked on a view's page since its last main-frame request.
/// Counted by distinct URL: Blink may ask for a blocked resource twice (its
/// preload scanner, then the parser once the first load failed), which is
/// one thing blocked, not two.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageBlocks {
    urls: BTreeSet<String>,
    /// Distinct blocked URLs per host.
    by_host: BTreeMap<String, u32>,
}

impl PageBlocks {
    /// Records a blocked request.
    fn record(&mut self, url: &str) {
        if self.urls.insert(url.to_owned()) {
            let host = wubble_core::zoom::host_of(url).unwrap_or_default();
            *self.by_host.entry(host).or_insert(0) += 1;
        }
    }

    /// Distinct URLs blocked.
    pub fn count(&self) -> u32 {
        u32::try_from(self.urls.len()).unwrap_or(u32::MAX)
    }

    /// Blocked URLs per host, most first (then by name).
    pub fn hosts(&self) -> Vec<(String, u32)> {
        let mut hosts: Vec<(String, u32)> =
            self.by_host.iter().map(|(h, n)| (h.clone(), *n)).collect();
        hosts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        hosts
    }
}

/// The blocker shared with CEF's IO thread, and what each view's page had
/// blocked, for the shell to show.
pub struct Blocking {
    blocker: RwLock<Arc<Blocker>>,
    overrides: RwLock<SiteOverrides>,
    enabled: AtomicBool,
    /// Per view ([`wubble_engine::view::View::browser_id`]), since its last
    /// main-frame request.
    blocked: Mutex<HashMap<i32, PageBlocks>>,
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

    /// Applies the stored settings and loads the selected lists: from the
    /// cache at once when it matches them, else compiled on a background
    /// thread (requests pass meanwhile). With `lists_auto_update` on, that
    /// thread also fetches due lists now and every [`UPDATE_CHECK`].
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
        let selection = settings.selection();
        let sources = wubble_blocker::sources(&dir, &selection);
        let compiled = self.load_cached(settings, &sources, &cache);
        let update = settings.lists_auto_update;
        if compiled && !update {
            return None;
        }
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("wubble-blocker".into())
            .spawn(move || {
                if !compiled {
                    this.replace_blocker(Blocker::compile(&sources, Some(&cache)));
                }
                if update {
                    loop {
                        this.update_once(&dir, &cache, &selection, fetch);
                        std::thread::sleep(UPDATE_CHECK);
                    }
                }
            });
        spawned
            .inspect_err(|e| eprintln!("wubble: blocker lists not loaded: {e}"))
            .ok()
    }

    /// The settings, then the cached blocker if it matches the sources.
    /// Returns whether the lists are loaded.
    fn load_cached(&self, settings: &BlockerSettings, sources: &[Source], cache: &Path) -> bool {
        self.set_enabled(settings.enabled);
        self.set_exceptions(&settings.exceptions);
        match Blocker::from_cache(sources, cache) {
            Some(blocker) => {
                self.replace_blocker(blocker);
                true
            }
            None => false,
        }
    }

    /// Fetches the due lists into `dir`; recompiles and swaps if any changed.
    fn update_once(&self, dir: &Path, cache: &Path, selection: &Selection, fetch: Fetch) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let specs = wubble_blocker::specs(selection);
        if !wubble_blocker::refresh(dir, &specs, Due::At(now), &fetch).is_empty() {
            let sources = wubble_blocker::sources(dir, selection);
            self.replace_blocker(Blocker::compile(&sources, Some(cache)));
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

    /// `--smoke block`: only `list` (with uBO's resources) blocks, shields
    /// on, nothing read from disk or fetched.
    pub fn use_list(&self, list: &str) {
        self.set_enabled(true);
        self.set_overrides(SiteOverrides::default());
        self.replace_blocker(Blocker::from_sources(&[Source::text(list)]));
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

    /// Distinct URLs blocked in a view since its last main-frame request.
    pub fn blocked(&self, browser_id: i32) -> u32 {
        self.counts().get(&browser_id).map_or(0, PageBlocks::count)
    }

    /// What a view's page had blocked, per host.
    pub fn blocked_hosts(&self, browser_id: i32) -> Vec<(String, u32)> {
        self.counts()
            .get(&browser_id)
            .map(PageBlocks::hosts)
            .unwrap_or_default()
    }

    fn counts(&self) -> std::sync::MutexGuard<'_, HashMap<i32, PageBlocks>> {
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
            self.counts()
                .insert(request.browser_id, PageBlocks::default());
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
                self.counts()
                    .entry(request.browser_id)
                    .or_default()
                    .record(request.url);
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

    /// A small list for the request-path tests.
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
        let on = |id, url| RequestInfo {
            browser_id: id,
            ..req(url, SITE, Script)
        };
        b.check(&on(1, "https://ads.example/x.js"));
        b.check(&on(1, "https://ads.example/y.js"));
        b.check(&on(2, "https://ads.example/x.js"));
        b.check(&on(1, "https://fine.test/"));
        assert_eq!((b.blocked(1), b.blocked(2), b.blocked(3)), (2, 1, 0));
        b.check(&RequestInfo {
            browser_id: 1,
            ..req("https://next.test/", "https://next.test/", Document)
        });
        assert_eq!((b.blocked(1), b.blocked(2)), (0, 1));
    }

    /// Blink's preload scanner and its parser may both ask for a blocked
    /// script: one thing blocked.
    #[test]
    fn the_same_url_twice_counts_once() {
        let b = blocking(TEST_LIST);
        for _ in 0..3 {
            assert_eq!(check(&b, "https://ads.example/x.js", SITE, Script), Block);
        }
        assert_eq!(b.blocked(1), 1);
    }

    #[test]
    fn blocked_urls_are_broken_down_by_host() {
        let b = blocking("||ads.example^\n||tracker.net^\n");
        for url in [
            "https://ads.example/a.js",
            "https://ads.example/b.js",
            "https://ads.example/b.js",
            "https://tracker.net/p.gif",
            "https://fine.test/",
        ] {
            check(&b, url, SITE, Image);
        }
        assert_eq!(
            b.blocked_hosts(1),
            [("ads.example".to_owned(), 2), ("tracker.net".to_owned(), 1)]
        );
        assert!(b.blocked_hosts(9).is_empty());
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

    /// Every catalog list off: only the profile's own files.
    fn only_own_lists() -> BlockerSettings {
        BlockerSettings {
            lists: wubble_blocker::catalog::LISTS
                .iter()
                .map(|l| (l.id.to_owned(), 0))
                .collect(),
            ..BlockerSettings::default()
        }
    }

    #[test]
    fn the_defaults_select_ubo_s_lists_and_both_optional_categories() {
        let d = BlockerSettings::default();
        assert!(d.enabled && d.cookie_banners && d.strip_params && !d.lists_auto_update);
        assert_eq!(d.selection(), Selection::default());
    }

    #[test]
    fn the_bundled_lists_block_out_of_the_box() {
        let p = Profile::new("bundled");
        let b = Blocking::new();
        let compile = b
            .start_with(&p.0, &BlockerSettings::default(), no_network)
            .expect("compiles");
        compile.join().expect("no fetch");
        let ad = "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js";
        assert_eq!(check(&b, ad, "https://news.example/", Script), Block);
        // Tracking parameters are stripped from a page load.
        let page = "https://news.example/a?utm_source=x&id=1";
        assert_eq!(
            check(&b, page, page, Document),
            Redirect("https://news.example/a?id=1".into())
        );
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
            ..only_own_lists()
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
        fn fake(_: &str) -> Result<String, String> {
            Ok("||ads.example^\n".into())
        }
        let selection = Selection::default();
        b.update_once(&dir, &cache, &selection, fake);
        assert!(dir.join("easylist.txt").exists());
        assert_eq!(check(&b, "https://ads.example/x.js", SITE, Script), Block);
        // Everything is fresh now: a second update fetches nothing.
        b.update_once(&dir, &cache, &selection, no_network);
    }
}
