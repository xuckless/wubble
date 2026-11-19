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
//! unless `blocker.lists_auto_update` is on or the user presses "Update now"
//! (TODO.md, Decided). Switching lists on or off rebuilds the engine on a
//! worker thread and swaps it in ([`Blocking::reselect`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use wubble_blocker::catalog::Selection;
use wubble_blocker::{
    Blocker, Cosmetic, Decision, Due, ListRow, NetRequest, ResourceType, Shields, SiteOverrides,
    Source,
};
use wubble_engine::filter::{
    ContentFilter, FrameCosmetic, RequestAction, RequestInfo, RequestKind,
};
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
    /// Which lists are on (`blocker.lists`, `blocker.cookie_banners`,
    /// `blocker.strip_params`, `blocker.custom_lists`).
    pub selection: Selection,
}

impl Default for BlockerSettings {
    fn default() -> Self {
        Self {
            enabled: BlockerEnabled::default(),
            exceptions: BlockerExceptions::default(),
            lists_auto_update: BlockerListsAutoUpdate::default(),
            selection: Selection::default(),
        }
    }
}

/// Fetches one URL; the network is reached only through this.
type Fetch = fn(&str) -> Result<String, String>;

/// Distinct blocked URLs a page keeps track of; past this the count stops
/// growing, so a long-lived page's beacons with cache-busting query strings
/// cannot grow it without bound.
const MAX_PAGE_BLOCKS: usize = 4096;

/// What was blocked on a view's page since its last main-frame request.
/// Counted by distinct URL: Blink may ask for a blocked resource twice (its
/// preload scanner, then the parser once the first load failed), which is
/// one thing blocked, not two.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageBlocks {
    /// A hash of each blocked URL, at most [`MAX_PAGE_BLOCKS`].
    urls: HashSet<u64>,
    /// Distinct blocked URLs per host.
    by_host: BTreeMap<String, u32>,
}

impl PageBlocks {
    /// Records a blocked request.
    fn record(&mut self, url: &str) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        url.hash(&mut h);
        if self.urls.len() < MAX_PAGE_BLOCKS && self.urls.insert(h.finish()) {
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

/// Frame URLs whose cosmetic rules are kept: a page asks again for each batch
/// of new class names, and adblock rebuilds a URL's data on every query.
const COSMETIC_CACHE: usize = 64;

/// The one owner of which lists are on, and where they live.
#[derive(Default)]
struct ListState {
    selection: Selection,
    /// Set by [`Blocking::start`]; nothing is rebuilt before.
    profile: Option<PathBuf>,
    /// Rules per list id, counted from the list texts after each build.
    rules: BTreeMap<String, u32>,
}

/// The blocker shared with CEF's IO thread, and what each view's page had
/// blocked, for the shell to show.
pub struct Blocking {
    blocker: RwLock<Arc<Blocker>>,
    overrides: RwLock<SiteOverrides>,
    enabled: AtomicBool,
    /// Per view ([`wubble_engine::view::View::browser_id`]), since its last
    /// main-frame request; dropped when the view goes ([`Self::keep_views`]).
    blocked: Mutex<HashMap<i32, PageBlocks>>,
    lists: Mutex<ListState>,
    /// Counts the requests for a rebuild: only the newest one is swapped in.
    generation: AtomicU64,
    /// Held while an engine compiles: a burst of toggles compiles the first
    /// and the last selection only.
    compiling: Mutex<()>,
    /// Rebuilds waiting or running.
    rebuilding: AtomicUsize,
    /// An update from the network is running.
    updating: AtomicBool,
    /// Cosmetic data per frame URL, cleared when the blocker is replaced.
    cosmetic: Mutex<HashMap<String, Arc<Cosmetic>>>,
    /// Called (on a worker thread) when the lists' state changed.
    changed: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Blocking {
    /// A blocker with no lists (allows everything) until [`Self::start`].
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            blocker: RwLock::new(Arc::new(Blocker::from_lists(&[]))),
            overrides: RwLock::new(SiteOverrides::default()),
            enabled: AtomicBool::new(BlockerEnabled::default()),
            blocked: Mutex::new(HashMap::new()),
            lists: Mutex::new(ListState::default()),
            generation: AtomicU64::new(0),
            compiling: Mutex::new(()),
            rebuilding: AtomicUsize::new(0),
            updating: AtomicBool::new(false),
            cosmetic: Mutex::new(HashMap::new()),
            changed: Mutex::new(None),
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
    pub(crate) fn start_with(
        self: &Arc<Self>,
        profile: &Path,
        settings: &BlockerSettings,
        fetch: Fetch,
    ) -> Option<JoinHandle<()>> {
        let (dir, _) = paths(profile);
        {
            let mut lists = self.state();
            lists.selection = settings.selection.clone();
            lists.profile = Some(profile.to_owned());
        }
        let sources = wubble_blocker::sources(&dir, &settings.selection);
        let compiled = self.load_cached(settings, &sources, &paths(profile).1);
        let update = settings.lists_auto_update;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        let work = move || {
            if compiled {
                this.count_rules(&sources, generation);
            } else {
                this.run_rebuild(generation);
            }
            if update {
                loop {
                    this.update_once(fetch, Due::At(unix_now()));
                    std::thread::sleep(UPDATE_CHECK);
                }
            }
        };
        let spawned = std::thread::Builder::new()
            .name("wubble-blocker".into())
            .spawn(work);
        match spawned {
            // Counting alone is not worth the caller waiting for.
            Ok(thread) if compiled && !update => {
                drop(thread);
                None
            }
            Ok(thread) => Some(thread),
            Err(e) => {
                eprintln!("wubble: blocker lists not loaded: {e}");
                None
            }
        }
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

    fn state(&self) -> std::sync::MutexGuard<'_, ListState> {
        self.lists.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Compiles the current selection and swaps the result in, unless a newer
    /// rebuild was asked for meanwhile (`generation` is no longer the newest).
    /// Slow (seconds): worker threads only.
    fn rebuild(&self, generation: u64) {
        {
            let _one_at_a_time = self
                .compiling
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let newest = || self.generation.load(Ordering::SeqCst) == generation;
            let (profile, selection) = {
                let lists = self.state();
                (lists.profile.clone(), lists.selection.clone())
            };
            if let Some(profile) = profile.filter(|_| newest()) {
                let (dir, cache) = paths(&profile);
                let sources = wubble_blocker::sources(&dir, &selection);
                let blocker = Blocker::compile(&sources, Some(&cache));
                if newest() {
                    self.replace_blocker(blocker);
                    self.count_rules(&sources, generation);
                }
            }
        }
    }

    /// [`Self::rebuild`] counted as running, then the page told.
    fn run_rebuild(&self, generation: u64) {
        self.begin_rebuild();
        self.rebuild(generation);
        self.end_rebuild();
    }

    fn begin_rebuild(&self) {
        self.rebuilding.fetch_add(1, Ordering::SeqCst);
    }

    fn end_rebuild(&self) {
        self.rebuilding.fetch_sub(1, Ordering::SeqCst);
        self.notify();
    }

    /// Records the rules in each source list, unless a newer rebuild than
    /// `generation` was asked for (its counts would be overwritten).
    fn count_rules(&self, sources: &[Source], generation: u64) {
        let rules = wubble_blocker::rule_counts(sources);
        let mut lists = self.state();
        if self.generation.load(Ordering::SeqCst) == generation {
            lists.rules = rules;
            drop(lists);
            self.notify();
        }
    }

    /// Switches to `selection` (the list settings changed): rebuilds on a
    /// worker thread; requests pass on the old engine until the swap. The
    /// thread, if one was started.
    pub fn reselect(self: &Arc<Self>, selection: Selection) -> Option<JoinHandle<()>> {
        {
            let mut lists = self.state();
            if lists.selection == selection || lists.profile.is_none() {
                return None;
            }
            lists.selection = selection;
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let this = self.clone();
        // Counted now, so the page shows "rebuilding" before the thread runs.
        self.begin_rebuild();
        let spawned = std::thread::Builder::new()
            .name("wubble-blocker-rebuild".into())
            .spawn(move || {
                this.rebuild(generation);
                this.end_rebuild();
            });
        spawned
            .inspect_err(|e| {
                self.end_rebuild();
                eprintln!("wubble: blocker not rebuilt: {e}");
            })
            .ok()
    }

    /// Fetches the lists that are `due` into the profile; rebuilds if any
    /// changed. Returns how many were written and how many were asked for.
    fn update_once(&self, fetch: Fetch, due: Due) -> (usize, usize) {
        let (profile, selection) = {
            let lists = self.state();
            (lists.profile.clone(), lists.selection.clone())
        };
        let Some(profile) = profile else {
            return (0, 0);
        };
        let specs = wubble_blocker::specs(&selection);
        let written = wubble_blocker::refresh(&paths(&profile).0, &specs, due, &fetch).len();
        if written > 0 {
            let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
            self.run_rebuild(generation);
        }
        (written, specs.len())
    }

    /// "Update now": fetches every selected list, rebuilds, then calls `done`
    /// with a sentence for the user. Only the user's button starts this (and
    /// the opt-in auto-update does the same on a schedule). `None` while an
    /// update already runs.
    pub fn update_now(
        self: &Arc<Self>,
        done: impl FnOnce(String) + Send + 'static,
    ) -> Option<JoinHandle<()>> {
        self.update_now_with(wubble_blocker::fetch_http, done)
    }

    fn update_now_with(
        self: &Arc<Self>,
        fetch: Fetch,
        done: impl FnOnce(String) + Send + 'static,
    ) -> Option<JoinHandle<()>> {
        if self.updating.swap(true, Ordering::SeqCst) {
            return None;
        }
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("wubble-blocker-update".into())
            .spawn(move || {
                let (written, asked) = this.update_once(fetch, Due::All);
                this.updating.store(false, Ordering::SeqCst);
                this.notify();
                done(update_report(written, asked));
            });
        spawned
            .inspect_err(|e| {
                self.updating.store(false, Ordering::SeqCst);
                eprintln!("wubble: blocker update not started: {e}");
            })
            .ok()
    }

    /// A rebuild or an update is running.
    pub fn is_busy(&self) -> bool {
        self.rebuilding.load(Ordering::SeqCst) > 0 || self.updating.load(Ordering::SeqCst)
    }

    /// The settings page's rows: every list, on or off, with what is known of it.
    pub fn list_rows(&self) -> Vec<ListRow> {
        let (profile, selection, rules) = {
            let lists = self.state();
            (
                lists.profile.clone(),
                lists.selection.clone(),
                lists.rules.clone(),
            )
        };
        let fetched = profile.map_or_else(BTreeMap::new, |profile| {
            let specs: Vec<_> = wubble_blocker::catalog::LISTS
                .iter()
                .map(wubble_blocker::ListSpec::from)
                .chain(
                    selection
                        .custom
                        .iter()
                        .map(|u| wubble_blocker::ListSpec::custom(u)),
                )
                .collect();
            wubble_blocker::last_fetch(&paths(&profile).0, &specs)
        });
        wubble_blocker::rows(&selection, &fetched, &rules)
    }

    /// Calls `f` whenever the lists' state changes (a rebuild or update ended,
    /// counts arrived), from a worker thread.
    pub fn on_lists_changed(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.changed.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(f));
    }

    fn notify(&self) {
        let f = self
            .changed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(f) = f {
            f();
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
        self.cosmetic
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The cosmetic data of a frame's URL, as a page with shields on gets it.
    fn cosmetic_of(&self, frame_url: &str) -> Arc<Cosmetic> {
        self.cosmetic_with(&self.current(), frame_url)
    }

    /// [`Self::cosmetic_of`] computed on `blocker`; cached only while it is
    /// still the current engine (checked under the cache lock, which
    /// [`Self::replace_blocker`] takes after the swap), so a swap during the
    /// computation cannot leave stale rules behind.
    fn cosmetic_with(&self, blocker: &Arc<Blocker>, frame_url: &str) -> Arc<Cosmetic> {
        if let Some(found) = self
            .cosmetic
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(frame_url)
        {
            return found.clone();
        }
        let made = Arc::new(blocker.cosmetic(frame_url, Shields::On));
        let mut cache = self.cosmetic.lock().unwrap_or_else(PoisonError::into_inner);
        if Arc::ptr_eq(blocker, &self.current()) {
            if cache.len() >= COSMETIC_CACHE {
                cache.clear();
            }
            cache.insert(frame_url.to_owned(), made.clone());
        }
        made
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

    /// Forgets what views other than `live` had blocked: called when a
    /// view goes, so a closed or slept tab's page does not stay counted.
    pub fn keep_views(&self, live: &[i32]) {
        self.counts().retain(|id, _| live.contains(id));
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

/// `--measure blocker`: the bundled default lists' cold compile, cache
/// size, load from the cache and mean cost of one request decision over a
/// fixed set of URLs. Printed for a person to read; nothing asserts on it.
pub fn measure() -> String {
    use std::time::Instant;
    let sources = wubble_blocker::bundled_sources(&Selection::default());
    let start = Instant::now();
    let blocker = Blocker::from_sources(&sources);
    let compile = start.elapsed();
    let bytes = blocker.to_bytes();
    let start = Instant::now();
    let loaded = Blocker::from_bytes(&bytes).map(Blocker::with_bundled_resources);
    let load = start.elapsed();
    let Ok(loaded) = loaded else {
        return "measure: blocker cache did not load".into();
    };
    const HOSTS: [&str; 8] = [
        "news.example",
        "cdn.news.example",
        "pagead2.googlesyndication.com",
        "www.google-analytics.com",
        "static.doubleclick.net",
        "connect.facebook.net",
        "fonts.gstatic.com",
        "images.shop.example",
    ];
    const KINDS: [ResourceType; 4] = [
        ResourceType::Script,
        ResourceType::Image,
        ResourceType::Xhr,
        ResourceType::Stylesheet,
    ];
    let urls: Vec<(String, ResourceType)> = (0..10_000usize)
        .map(|i| {
            let host = HOSTS[i % HOSTS.len()];
            let kind = KINDS[(i / HOSTS.len()) % KINDS.len()];
            (
                format!("https://{host}/assets/{i}/file.js?v={i}&utm_source=x"),
                kind,
            )
        })
        .collect();
    let mut blocked = 0usize;
    let start = Instant::now();
    for (url, kind) in &urls {
        let req = NetRequest {
            url,
            source_url: "https://news.example/article",
            resource_type: *kind,
        };
        if loaded.decide(req, Shields::On) != Decision::Allow {
            blocked += 1;
        }
    }
    let decide = start.elapsed();
    format!(
        "measure blocker ({} build): {} bundled lists\n  cold compile {:.0} ms\n  cache {:.1} MB, load {:.1} ms with resources\n  decide {:.2} us per request (mean of {}, {blocked} not allowed)",
        if cfg!(debug_assertions) {
            "dev, opt-level 1"
        } else {
            "release"
        },
        sources.len(),
        compile.as_secs_f64() * 1e3,
        bytes.len() as f64 / 1e6,
        load.as_secs_f64() * 1e3,
        decide.as_secs_f64() * 1e6 / urls.len() as f64,
        urls.len(),
    )
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
        let action = match self.current().decide(net, self.shields_for(top)) {
            Decision::Allow => return RequestAction::Allow,
            Decision::Rewrite { url } => return RequestAction::Redirect(url),
            // A `$redirect` stub (uBO's `noop.js`, `googletagmanager_gtm.js`,
            // ...) stands in for the blocked resource so the page does not
            // break; a page itself is never replaced by one.
            Decision::Redirect { data_url } if request.kind != RequestKind::Document => {
                RequestAction::Serve(data_url)
            }
            Decision::Block | Decision::Redirect { .. } => RequestAction::Block,
        };
        // Counted as blocked either way, as uBO and Brave count redirects:
        // the real resource was not loaded.
        self.counts()
            .entry(request.browser_id)
            .or_default()
            .record(request.url);
        action
    }

    fn frame_cosmetic(&self, top_url: &str, frame_url: &str) -> FrameCosmetic {
        if self.shields_for(top_url) == Shields::Off || !is_web(frame_url) {
            return FrameCosmetic::default();
        }
        let cosmetic = self.cosmetic_of(frame_url);
        let (mut css, procedural) = cosmetic.procedural();
        css.splice(0..0, cosmetic.rules());
        FrameCosmetic {
            css,
            procedural,
            script: cosmetic.scriptlets(),
            generic: !cosmetic.generichide,
        }
    }

    fn generic_hide(
        &self,
        top_url: &str,
        frame_url: &str,
        classes: &[String],
        ids: &[String],
    ) -> Vec<String> {
        if self.shields_for(top_url) == Shields::Off || !is_web(frame_url) {
            return Vec::new();
        }
        let cosmetic = self.cosmetic_of(frame_url);
        let classes: Vec<&str> = classes.iter().map(String::as_str).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        self.current()
            .generic_selectors(&classes, &ids, &cosmetic)
            .iter()
            .map(|s| wubble_blocker::hide_rule(s))
            .collect()
    }
}

/// What "Update now" tells the user.
fn update_report(written: usize, asked: usize) -> String {
    match (written, asked) {
        (_, 0) => "No filter lists to update".into(),
        (0, _) => "Could not download the filter lists".into(),
        (n, m) if n == m => format!("Updated {n} filter list{}", if n == 1 { "" } else { "s" }),
        (n, m) => format!("Updated {n} of {m} filter lists"),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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

    fn blocking(list: &str) -> Arc<Blocking> {
        let b = Blocking::new();
        b.replace_blocker(Blocker::from_lists(&[list]).with_bundled_resources());
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

    use RequestAction::{Allow, Block, Redirect, Serve};
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
    }

    /// uBO's `$redirect` stubs are served in place of the resource (and
    /// counted as blocked); a page is never replaced by one.
    #[test]
    fn redirect_resources_are_served_except_for_pages() {
        let b = blocking(TEST_LIST);
        let img = "https://img.test/a.png";
        assert!(
            matches!(check(&b, img, SITE, Image), Serve(u) if u.starts_with("data:image/gif;base64,"))
        );
        assert_eq!(b.blocked(1), 1);
        let gtm = "https://www.googletagmanager.com/gtm.js?id=GTM-X";
        let b = blocking("||googletagmanager.com/gtm.js$script,redirect=googletagmanager_gtm.js\n");
        assert!(
            matches!(check(&b, gtm, SITE, Script), Serve(u) if u.starts_with("data:application/javascript;base64,"))
        );
        let b = blocking("||img.test^$document,redirect=noop.html\n");
        assert_eq!(
            check(&b, "https://img.test/", "https://img.test/", Document),
            Block
        );
    }

    #[test]
    fn a_long_lived_page_s_blocks_stay_bounded_and_go_with_the_view() {
        let b = blocking(TEST_LIST);
        for i in 0..MAX_PAGE_BLOCKS + 10 {
            check(
                &b,
                &format!("https://ads.example/beacon?t={i}"),
                SITE,
                Image,
            );
        }
        assert_eq!(b.blocked(1), MAX_PAGE_BLOCKS as u32);
        assert_eq!(
            b.blocked_hosts(1),
            [("ads.example".to_owned(), MAX_PAGE_BLOCKS as u32)]
        );
        b.check(&RequestInfo {
            browser_id: 2,
            ..req("https://ads.example/x.js", SITE, Script)
        });
        b.keep_views(&[2]);
        assert_eq!((b.blocked(1), b.blocked(2)), (0, 1));
        assert_eq!(b.counts().len(), 1);
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
        assert_eq!(
            b.frame_cosmetic("https://example.com/", "https://example.com/"),
            FrameCosmetic::default()
        );
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

    const COSMETIC_LIST: &str = "\
example.com##.sponsor
##.cookie-notice
###consent
##div[class*=\"adbox\"]
example.com#@#.cookie-notice
";

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn cosmetic_rules_are_selected_per_frame_host() {
        let b = blocking(COSMETIC_LIST);
        let frame = |top, url| b.frame_cosmetic(top, url);
        let page = frame("https://example.com/a", "https://example.com/a");
        assert!(
            page.css
                .contains(&".sponsor{display:none!important}".to_owned())
        );
        assert!(page.generic);
        let other = frame("https://other.org/", "https://other.org/");
        assert!(
            !other
                .css
                .contains(&".sponsor{display:none!important}".to_owned())
        );
        // A frame on another host gets that host's rules, under the page's shields.
        let sub = frame("https://other.org/", "https://example.com/ad");
        assert!(
            sub.css
                .contains(&".sponsor{display:none!important}".to_owned())
        );
        // Not web: nothing, and no collector.
        assert_eq!(
            frame("file:///example.com/", "file:///example.com/"),
            FrameCosmetic::default()
        );
        assert_eq!(
            frame("https://example.com/", "about:blank"),
            FrameCosmetic::default()
        );
        let mut o = SiteOverrides::default();
        o.insert("example.com");
        b.set_overrides(o);
        assert_eq!(
            frame("https://example.com/a", "https://example.com/a"),
            FrameCosmetic::default()
        );
    }

    #[test]
    fn generic_selectors_answer_the_names_a_frame_found() {
        let b = blocking(COSMETIC_LIST);
        let hide = |top: &str, classes: &[&str], ids: &[&str]| {
            b.generic_hide(top, top, &names(classes), &names(ids))
        };
        assert_eq!(
            hide("https://a.test/", &["cookie-notice", "x"], &["consent"]),
            [
                "#consent{display:none!important}",
                ".cookie-notice{display:none!important}"
            ]
        );
        // The host's exception wins.
        assert_eq!(
            hide("https://example.com/", &["cookie-notice"], &["consent"]),
            ["#consent{display:none!important}"]
        );
        assert!(hide("https://a.test/", &["fine"], &[]).is_empty());
        let mut o = SiteOverrides::default();
        o.insert("a.test");
        b.set_overrides(o);
        assert!(hide("https://a.test/", &["cookie-notice"], &[]).is_empty());
        b.set_enabled(false);
        assert!(hide("https://b.test/", &["cookie-notice"], &[]).is_empty());
    }

    #[test]
    fn a_stale_engine_leaves_nothing_in_the_cosmetic_cache() {
        let b = blocking(COSMETIC_LIST);
        let url = "https://example.com/";
        let old = b.current();
        b.replace_blocker(Blocker::from_lists(&["example.com##.other\n"]));
        // The lookup began on the old engine and ends after the swap.
        assert!(!b.cosmetic_with(&old, url).hide_selectors.is_empty());
        assert!(b.cosmetic.lock().unwrap().is_empty());
        assert_eq!(
            b.frame_cosmetic(url, url).css,
            [".other{display:none!important}"]
        );
    }

    #[test]
    fn a_replaced_blocker_clears_the_cosmetic_cache() {
        let b = blocking(COSMETIC_LIST);
        let url = "https://example.com/";
        assert!(!b.frame_cosmetic(url, url).css.is_empty());
        assert_eq!(b.cosmetic.lock().unwrap().len(), 1);
        b.replace_blocker(Blocker::from_lists(&["example.com##.other\n"]));
        assert!(b.cosmetic.lock().unwrap().is_empty());
        assert_eq!(
            b.frame_cosmetic(url, url).css,
            [".other{display:none!important}"]
        );
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
            selection: Selection::none(),
            ..BlockerSettings::default()
        }
    }

    #[test]
    fn the_defaults_block_and_never_fetch() {
        let d = BlockerSettings::default();
        assert!(d.enabled && !d.lists_auto_update);
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
        // uBO's filters replace it with their `adsbygoogle` stub.
        assert!(matches!(
            check(&b, ad, "https://news.example/", Script),
            Serve(u) if u.starts_with("data:application/javascript;base64,")
        ));
        assert_eq!(b.blocked(1), 1);
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

    /// A started blocker on a profile with no catalog list, for the rebuild tests.
    fn started(p: &Profile) -> Arc<Blocking> {
        let b = Blocking::new();
        {
            let mut lists = b.state();
            lists.selection = Selection::none();
            lists.profile = Some(p.0.clone());
        }
        b
    }

    fn custom_list(p: &Profile, url: &str, text: &str) {
        let (dir, _) = paths(&p.0);
        std::fs::create_dir_all(&dir).unwrap();
        let file = wubble_blocker::ListSpec::custom(url).file_name();
        std::fs::write(dir.join(file), text).unwrap();
    }

    fn selecting(url: &str) -> Selection {
        Selection {
            custom: vec![url.into()],
            ..Selection::none()
        }
    }

    const A: &str = "https://lists.test/a.txt";
    const B: &str = "https://lists.test/b.txt";

    #[test]
    fn an_update_fetches_lists_and_swaps_the_blocker() {
        let p = Profile::new("update");
        let (dir, _) = paths(&p.0);
        let b = started(&p);
        b.state().selection = Selection::default();
        fn fake(_: &str) -> Result<String, String> {
            Ok("||ads.example^\n".into())
        }
        let (written, asked) = b.update_once(fake, Due::At(unix_now()));
        assert!(written > 0 && written == asked, "{written} of {asked}");
        assert!(dir.join("easylist.txt").exists());
        assert_eq!(check(&b, "https://ads.example/x.js", SITE, Script), Block);
        // Everything is fresh now: a second update fetches nothing.
        assert_eq!(b.update_once(no_network, Due::At(unix_now())).0, 0);
    }

    #[test]
    fn update_now_fetches_everything_selected_and_reports() {
        let p = Profile::new("update-now");
        let b = started(&p);
        b.state().selection = selecting(A);
        fn fake(_: &str) -> Result<String, String> {
            Ok("||fresh.test^\n".into())
        }
        let (tx, rx) = std::sync::mpsc::channel();
        b.update_now_with(fake, move |text| tx.send(text).unwrap())
            .expect("starts")
            .join()
            .unwrap();
        assert_eq!(rx.recv().unwrap(), "Updated 1 filter list");
        assert_eq!(check(&b, "https://fresh.test/x.js", SITE, Script), Block);
        assert!(!b.is_busy());
        // Fresh lists are fetched again when the user asks.
        let (tx, rx) = std::sync::mpsc::channel();
        b.update_now_with(fake, move |text| tx.send(text).unwrap())
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(rx.recv().unwrap(), "Updated 1 filter list");
    }

    #[test]
    fn update_reports_read_as_sentences() {
        assert_eq!(update_report(0, 0), "No filter lists to update");
        assert_eq!(update_report(0, 3), "Could not download the filter lists");
        assert_eq!(update_report(3, 3), "Updated 3 filter lists");
        assert_eq!(update_report(2, 3), "Updated 2 of 3 filter lists");
    }

    #[test]
    fn reselecting_rebuilds_without_a_restart() {
        let p = Profile::new("reselect");
        custom_list(&p, A, "||a.test^\n");
        custom_list(&p, B, "||b.test^\n");
        let b = started(&p);
        b.reselect(selecting(A)).unwrap().join().unwrap();
        let blocked = |host: &str| check(&b, &format!("https://{host}/x.js"), SITE, Script);
        assert_eq!((blocked("a.test"), blocked("b.test")), (Block, Allow));
        // The same selection again is nothing to do.
        assert!(b.reselect(selecting(A)).is_none());
        b.reselect(selecting(B)).unwrap().join().unwrap();
        assert_eq!((blocked("a.test"), blocked("b.test")), (Allow, Block));
        // The rows know how many rules each list has.
        let id = wubble_blocker::ListSpec::custom(B).id;
        let rows = b.list_rows();
        assert_eq!(rows.iter().find(|r| r.id == id).unwrap().rules, Some(1));
    }

    /// Two selections asked for while one engine compiles: only the newest is built.
    #[test]
    fn the_newest_selection_wins() {
        let p = Profile::new("newest");
        custom_list(&p, A, "||a.test^\n");
        custom_list(&p, B, "||b.test^\n");
        let b = started(&p);
        let busy = b.compiling.lock().unwrap();
        let first = b.reselect(selecting(A)).unwrap();
        let second = b.reselect(selecting(B)).unwrap();
        assert!(b.is_busy());
        drop(busy);
        first.join().unwrap();
        second.join().unwrap();
        assert!(!b.is_busy());
        let blocked = |host: &str| check(&b, &format!("https://{host}/x.js"), SITE, Script);
        assert_eq!((blocked("a.test"), blocked("b.test")), (Allow, Block));
    }
}
