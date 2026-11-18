//! The edge between the host and `wubble-store`: when to save (a pure
//! policy) and the one thread that owns the database.
//!
//! The store's methods are async but need no runtime. A dedicated thread
//! creates the [`Store`], drives every request with `pollster::block_on` and
//! answers through callbacks, which hop back to the Slint thread themselves;
//! the UI thread never waits on the disk, except once after the window has
//! closed ([`StoreClient::shutdown`]).
//!
//! Save policy, idea only (no code copied): a dirty count plus "save when
//! enough changed or the session went quiet", always on orderly shutdown, as
//! Chromium's `ImportantFileWriter::ScheduleWrite` (BSD-3-Clause) and
//! Firefox's `SessionSaver` (MPL-2.0) do with timers. Here the "quiet" clock
//! is the host's coarse lifecycle tick, counted, never read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use wubble_core::apply;
use wubble_core::download::{self, DownloadEntry};
use wubble_core::model::{Command, Model};
use wubble_core::omnibox::HistoryEntry;
use wubble_core::pages::InternalPage;
use wubble_core::seam::SidebarMode;
use wubble_core::site::{Decision, Permission, SitePermissions};
use wubble_core::tab::TabId;
use wubble_engine::favicon::FaviconImage;
use wubble_store::{
    BlockerEnabled, BlockerExceptions, BlockerListsAutoUpdate, ClosedTab, Favicon, Setting,
    SidebarModeSetting, SidebarWidth, Store, Transition,
};

use crate::blocking::BlockerSettings;

/// Session changes after which the session is saved at once.
pub const SAVE_AFTER_CHANGES: u32 = 20;

/// Unsaved session changes, and whether a lifecycle tick has gone by
/// without a change since the last of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SaveState {
    pub changes: u32,
    pub quiet: bool,
}

impl SaveState {
    /// One more change; the session is no longer quiet.
    pub fn changed(self) -> Self {
        Self {
            changes: self.changes.saturating_add(1),
            quiet: false,
        }
    }

    /// A lifecycle tick went by.
    pub fn ticked(self) -> Self {
        Self {
            quiet: true,
            ..self
        }
    }
}

/// Save when something changed and either enough did or it has gone quiet.
/// After a save the host starts again from `SaveState::default()`.
pub fn should_save(s: SaveState) -> bool {
    s.changes > 0 && (s.changes >= SAVE_AFTER_CHANGES || s.quiet)
}

/// Whether `command` can change what `save_session` stores. Ticks and
/// loading only move lifecycle state, which is not stored (tabs come back
/// Discarded); opening the bar changes nothing.
pub fn changes_session(command: &Command) -> bool {
    !matches!(
        command,
        Command::Tick { .. }
            | Command::BusyChanged { .. }
            | Command::OpenCommandBar
            | Command::SetSleepPolicy(_)
    )
}

/// Whether committing `new` in a tab whose last commit was `previous` is a
/// history visit: a web page, and not the same page again or a move to
/// another fragment of it (a restored tab reloading, an in-page link).
pub fn visit_to_record(previous: Option<&str>, new: &str) -> bool {
    let page = |url: &str| url.split_once('#').map_or(url, |(page, _)| page).to_owned();
    wubble_store::is_recordable(new)
        && InternalPage::from_url(new).is_none()
        && previous.map(page) != Some(page(new))
}

/// The tab whose navigation the command bar started, until that navigation
/// commits a URL (a typed visit) or ends without one: it loaded the same URL
/// again, was cancelled, became a download, or another navigation replaced
/// it. Only then is the tab's next visit a link again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypedNav(Option<TabId>);

impl TypedNav {
    /// The bar started a navigation in `tab`.
    pub fn started(&mut self, tab: Option<TabId>) {
        self.0 = tab;
    }

    /// `tab` committed a URL: whether that visit was typed.
    pub fn committed(&mut self, tab: TabId) -> bool {
        let typed = self.0 == Some(tab);
        self.ended(tab);
        typed
    }

    /// `tab` stopped loading, closed, or started a navigation of its own
    /// (back, forward, reload).
    pub fn ended(&mut self, tab: TabId) {
        if self.0 == Some(tab) {
            self.0 = None;
        }
    }
}

/// UI state saved with the session: the typed settings `SidebarWidth` (the
/// extended width) and `SidebarModeSetting` (peeking is never stored).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiState {
    pub sidebar_width: f32,
    pub mode: SidebarMode,
}

/// The settings read at start. Everything but what is needed before the host exists (the
/// shell's sidebar, the blocker) reaches the host as text rows, which it replays through
/// `apply::effect_of`, the same path a change on the settings page takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub ui: UiState,
    pub blocker: BlockerSettings,
    /// Every stored setting row, `key -> text` (`Store::all_settings`).
    pub stored: HashMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui: UiState {
                sidebar_width: SidebarWidth::default(),
                mode: SidebarModeSetting::default(),
            },
            blocker: BlockerSettings::default(),
            stored: HashMap::new(),
        }
    }
}

impl Settings {
    /// The page features CEF takes as start switches (`restart` rows).
    pub fn page_features(&self) -> wubble_engine::app::PageFeatures {
        use wubble_core::settings as defs;
        let on = |def| apply::value_of(def, &self.stored) == "1";
        wubble_engine::app::PageFeatures {
            overlay_scrollbars: on(&defs::OVERLAY_SCROLLBARS),
            middle_click_autoscroll: on(&defs::MIDDLE_CLICK_AUTOSCROLL),
        }
    }

    /// Whether the wgpu glass scene runs (`settings::GLASS_SCENE`, read before the window).
    pub fn glass_scene(&self) -> bool {
        apply::value_of(&wubble_core::settings::GLASS_SCENE, &self.stored) == "1"
    }
}

/// Download entries read at start (newest first).
pub const DOWNLOADS_KEPT: usize = 50;

/// Icon bytes kept across runs: 256 icons of 64x64, the most the store holds.
pub const FAVICON_CAP_BYTES: u64 = 4 << 20;

/// History hits for a query, with the icon of each hit's origin that has one.
#[derive(Debug, Default)]
pub struct Hits {
    pub entries: Vec<HistoryEntry>,
    /// Keyed by the hit's URL.
    pub icons: HashMap<String, Favicon>,
}

/// What the store thread hands over at start.
#[derive(Debug)]
pub struct Loaded {
    pub model: Model,
    pub settings: Settings,
    pub site_permissions: SitePermissions,
    /// Newest first; what was unfinished in the last run has failed.
    pub downloads: Vec<DownloadEntry>,
    /// The stored icon of each tab, keyed by the tab's URL (a tab on a page
    /// without an origin, or without an icon, is absent). Pruned to
    /// [`FAVICON_CAP_BYTES`] first.
    pub favicons: HashMap<String, Favicon>,
}

impl Default for Loaded {
    fn default() -> Self {
        Self {
            model: Model::new(std::time::Instant::now()),
            settings: Settings::default(),
            site_permissions: SitePermissions::default(),
            downloads: vec![],
            favicons: HashMap::new(),
        }
    }
}

type Reply<T> = Box<dyn FnOnce(T) + Send>;

enum Request {
    Save {
        model: Box<Model>,
        ui: UiState,
    },
    RecordVisit {
        url: String,
        title: String,
        at_ms: i64,
        transition: Transition,
    },
    UpdateTitle {
        url: String,
        title: String,
    },
    Query {
        prefix: String,
        now_ms: i64,
        limit: usize,
        reply: Reply<Hits>,
    },
    PutFavicon {
        page_url: String,
        icon: Favicon,
        now_ms: i64,
    },
    PushClosed(ClosedTab),
    PopClosed(Reply<Option<ClosedTab>>),
    /// One settings row, validated by the client (`apply::accepts`) and again by the store.
    SetSetting {
        key: &'static str,
        value: String,
    },
    /// Every stored setting row, after the writes queued before it.
    ReadSettings(Reply<HashMap<String, String>>),
    /// Expire visits by the stored retention.
    ExpireHistory {
        now_ms: i64,
    },
    ForgetSitePermission {
        origin: String,
        kind: Permission,
    },
    SetSitePermission {
        origin: String,
        kind: Permission,
        decision: Decision,
        at_ms: i64,
    },
    AddDownload(DownloadEntry),
    UpdateDownload(DownloadEntry),
    RemoveDownload(i64),
}

/// The host's handle on the store thread. If the database could not be
/// opened or loaded the thread has ended and every request is dropped, so
/// the browser runs without persistence and never writes over that file.
pub struct StoreClient {
    tx: mpsc::Sender<Request>,
    thread: Option<JoinHandle<()>>,
}

impl StoreClient {
    /// Starts the store thread on the database at `path`. The receiver gets
    /// the loaded session and settings, or why there are none.
    pub fn spawn(path: PathBuf) -> (StoreClient, mpsc::Receiver<Result<Loaded, String>>) {
        let (tx, rx) = mpsc::channel();
        let (loaded_tx, loaded_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("wubble-store".into())
            .spawn(move || serve(&path, rx, loaded_tx));
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(e) => {
                eprintln!("wubble: cannot start the store thread: {e}");
                None
            }
        };
        (StoreClient { tx, thread }, loaded_rx)
    }

    /// A client whose requests go nowhere.
    pub fn none() -> StoreClient {
        StoreClient {
            tx: mpsc::channel().0,
            thread: None,
        }
    }

    fn send(&self, request: Request) {
        // Fails only once the thread has ended (no database): nothing to do.
        let _ = self.tx.send(request);
    }

    pub fn save(&self, model: &Model, ui: UiState) {
        self.send(Request::Save {
            model: Box::new(model.clone()),
            ui,
        });
    }

    pub fn record_visit(&self, url: &str, title: &str, at_ms: i64, transition: Transition) {
        self.send(Request::RecordVisit {
            url: url.into(),
            title: title.into(),
            at_ms,
            transition,
        });
    }

    pub fn update_title(&self, url: &str, title: &str) {
        self.send(Request::UpdateTitle {
            url: url.into(),
            title: title.into(),
        });
    }

    /// `reply` runs on the store thread.
    pub fn query_history(
        &self,
        prefix: &str,
        now_ms: i64,
        limit: usize,
        reply: impl FnOnce(Hits) + Send + 'static,
    ) {
        self.send(Request::Query {
            prefix: prefix.into(),
            now_ms,
            limit,
            reply: Box::new(reply),
        });
    }

    /// Files the page's downloaded icon under its origin.
    pub fn put_favicon(&self, page_url: &str, icon: &FaviconImage, now_ms: i64) {
        self.send(Request::PutFavicon {
            page_url: page_url.into(),
            icon: Favicon {
                icon_url: icon.url.clone(),
                width: icon.width,
                height: icon.height,
                rgba: icon.rgba.clone(),
            },
            now_ms,
        });
    }

    pub fn push_closed(&self, tab: ClosedTab) {
        self.send(Request::PushClosed(tab));
    }

    /// `reply` runs on the store thread.
    pub fn pop_closed(&self, reply: impl FnOnce(Option<ClosedTab>) + Send + 'static) {
        self.send(Request::PopClosed(Box::new(reply)));
    }

    /// Writes one settings row; the one write path of every setting the host changes. A key
    /// that is no setting or a value its kind refuses is not sent: `false`.
    pub fn set_setting(&self, key: &str, value: &str) -> bool {
        let Some(def) = wubble_core::settings::find(key).filter(|d| d.kind.accepts(value)) else {
            return false;
        };
        self.send(Request::SetSetting {
            key: def.key,
            value: value.into(),
        });
        true
    }

    /// `reply` runs on the store thread, after every request sent before this one.
    pub fn read_settings(&self, reply: impl FnOnce(HashMap<String, String>) + Send + 'static) {
        self.send(Request::ReadSettings(Box::new(reply)));
    }

    pub fn expire_history(&self, now_ms: i64) {
        self.send(Request::ExpireHistory { now_ms });
    }

    pub fn forget_site_permission(&self, origin: &str, kind: Permission) {
        self.send(Request::ForgetSitePermission {
            origin: origin.into(),
            kind,
        });
    }

    pub fn set_site_permission(
        &self,
        origin: &str,
        kind: Permission,
        decision: Decision,
        at_ms: i64,
    ) {
        self.send(Request::SetSitePermission {
            origin: origin.into(),
            kind,
            decision,
            at_ms,
        });
    }

    pub fn add_download(&self, entry: &DownloadEntry) {
        self.send(Request::AddDownload(entry.clone()));
    }

    pub fn update_download(&self, entry: &DownloadEntry) {
        self.send(Request::UpdateDownload(entry.clone()));
    }

    pub fn remove_download(&self, id: i64) {
        self.send(Request::RemoveDownload(id));
    }

    /// Lets the thread finish what is queued, close the database and end.
    /// Blocks until it has; a thread that already ended returns at once.
    pub fn shutdown(self) {
        let StoreClient { tx, thread } = self;
        drop(tx);
        if let Some(thread) = thread
            && thread.join().is_err()
        {
            eprintln!("wubble: the store thread panicked; the last changes may be lost");
        }
    }
}

/// The store thread: open, load, then serve requests until the client is gone.
fn serve(path: &Path, rx: mpsc::Receiver<Request>, loaded: mpsc::Sender<Result<Loaded, String>>) {
    let mut store = match pollster::block_on(open_and_load(path)) {
        Ok((store, value)) => {
            let _ = loaded.send(Ok(value));
            store
        }
        Err(e) => {
            let _ = loaded.send(Err(format!("{}: {e}", path.display())));
            return;
        }
    };
    while let Ok(first) = rx.recv() {
        let batch: Vec<Request> = std::iter::once(first).chain(rx.try_iter()).collect();
        for request in latest_save_only(batch) {
            if let Err(e) = pollster::block_on(handle(&mut store, request)) {
                eprintln!("wubble: store: {e}");
            }
        }
    }
}

/// Drops every save in `batch` but the last: each carries the whole model,
/// so a slow disk never queues stale copies.
fn latest_save_only(batch: Vec<Request>) -> impl Iterator<Item = Request> {
    let last = batch
        .iter()
        .rposition(|r| matches!(r, Request::Save { .. }));
    batch
        .into_iter()
        .enumerate()
        .filter(move |(i, r)| !matches!(r, Request::Save { .. }) || Some(*i) == last)
        .map(|(_, r)| r)
}

async fn open_and_load(path: &Path) -> wubble_store::Result<(Store, Loaded)> {
    let mut store = Store::open(path).await?;
    let model = store.load_session(std::time::Instant::now()).await?;
    let settings = Settings {
        ui: UiState {
            sidebar_width: store.setting::<SidebarWidth>().await?,
            mode: store.setting::<SidebarModeSetting>().await?,
        },
        blocker: BlockerSettings {
            enabled: store.setting::<BlockerEnabled>().await?,
            exceptions: store.setting::<BlockerExceptions>().await?,
            lists_auto_update: store.setting::<BlockerListsAutoUpdate>().await?,
        },
        stored: store.all_settings().await?,
    };
    let site_permissions = store.site_permissions().await?.into_iter().collect();
    // No download of the last run is alive any more.
    let stored = store.downloads(DOWNLOADS_KEPT).await?;
    let downloads = download::lost_all(stored.clone());
    for (old, new) in stored.iter().zip(&downloads) {
        if old != new {
            store.update_download(new).await?;
        }
    }
    store.prune_favicons(FAVICON_CAP_BYTES).await?;
    let urls: Vec<&str> = model.tabs.values().map(|t| t.url.as_str()).collect();
    let favicons = store.favicons_for(&urls).await?;
    let loaded = Loaded {
        model,
        settings,
        site_permissions,
        downloads,
        favicons,
    };
    Ok((store, loaded))
}

async fn handle(store: &mut Store, request: Request) -> wubble_store::Result<()> {
    match request {
        Request::Save { model, ui } => {
            store.save_session(&model).await?;
            store.set_setting::<SidebarWidth>(ui.sidebar_width).await?;
            store.set_setting::<SidebarModeSetting>(ui.mode).await?;
        }
        Request::RecordVisit {
            url,
            title,
            at_ms,
            transition,
        } => {
            store.record_visit(&url, &title, at_ms, transition).await?;
        }
        Request::UpdateTitle { url, title } => store.update_title(&url, &title).await?,
        Request::Query {
            prefix,
            now_ms,
            limit,
            reply,
        } => {
            let mut failure = None;
            let entries = store
                .query_history(&prefix, now_ms, limit)
                .await
                .unwrap_or_else(|e| {
                    failure = Some(e);
                    Vec::new()
                });
            let urls: Vec<&str> = entries.iter().map(|h| h.url.as_str()).collect();
            let icons = match store.favicons_for(&urls).await {
                Ok(icons) => icons,
                Err(e) => {
                    failure = failure.or(Some(e));
                    HashMap::new()
                }
            };
            reply(Hits { entries, icons });
            if let Some(e) = failure {
                return Err(e);
            }
        }
        Request::PutFavicon {
            page_url,
            icon,
            now_ms,
        } => store.put_favicon(&page_url, &icon, now_ms).await?,
        Request::PushClosed(tab) => store.push_closed(&tab).await?,
        Request::PopClosed(reply) => {
            let tab = store.pop_closed().await;
            match tab {
                Ok(tab) => reply(tab),
                Err(e) => {
                    reply(None);
                    return Err(e);
                }
            }
        }
        Request::SetSetting { key, value } => store.set_text(key, &value).await?,
        Request::ReadSettings(reply) => match store.all_settings().await {
            Ok(rows) => reply(rows),
            Err(e) => {
                reply(HashMap::new());
                return Err(e);
            }
        },
        Request::ExpireHistory { now_ms } => {
            store.expire_history(now_ms).await?;
        }
        Request::ForgetSitePermission { origin, kind } => {
            store.forget_site_permission(&origin, kind).await?;
        }
        Request::SetSitePermission {
            origin,
            kind,
            decision,
            at_ms,
        } => {
            store
                .set_site_permission(&origin, kind, decision, at_ms)
                .await?;
        }
        Request::AddDownload(entry) => store.add_download(&entry).await?,
        Request::UpdateDownload(entry) => store.update_download(&entry).await?,
        Request::RemoveDownload(id) => store.remove_download(id).await?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use wubble_core::model::apply;
    use wubble_core::sleep::Pressure;
    use wubble_core::space::ContainerId;
    use wubble_core::tab::{Liveness, TabId};

    use super::*;

    fn state(changes: u32, quiet: bool) -> SaveState {
        SaveState { changes, quiet }
    }

    #[test]
    fn save_policy() {
        let n = SAVE_AFTER_CHANGES;
        let cases = [
            (state(0, false), false),
            (state(0, true), false),
            (state(1, false), false),
            (state(n - 1, false), false),
            (state(n, false), true),
            (state(1, true), true),
            (state(n - 1, true), true),
        ];
        for (s, want) in cases {
            assert_eq!(should_save(s), want, "{s:?}");
        }
    }

    #[test]
    fn a_change_ends_the_quiet() {
        let s = SaveState::default().changed().ticked();
        assert!(should_save(s));
        let s = s.changed();
        assert_eq!(s, state(2, false));
        assert!(!should_save(s));
        assert_eq!(SaveState::default().ticked(), state(0, true));
    }

    #[test]
    fn a_typed_navigation_marks_only_its_own_commit() {
        let (a, b) = (TabId(1), TabId(2));
        let mut typed = TypedNav::default();
        typed.started(Some(a));
        assert!(!typed.committed(b), "another tab's commit is a link");
        assert!(typed.committed(a));
        assert!(!typed.committed(a), "the next commit is a link again");
    }

    #[test]
    fn a_typed_navigation_without_a_commit_does_not_mark_the_next_one() {
        // Enter on the URL already shown: the load finishes, no URL commits.
        let t = TabId(1);
        let mut typed = TypedNav::default();
        typed.started(Some(t));
        typed.ended(t);
        assert!(!typed.committed(t), "a later link click is not typed");
        // Another tab finishing does not end it.
        typed.started(Some(t));
        typed.ended(TabId(2));
        assert!(typed.committed(t));
    }

    #[test]
    fn which_commands_change_the_session() {
        let t = TabId(1);
        let quiet = [
            Command::Tick {
                now: Instant::now(),
                pressure: Pressure::None,
            },
            Command::BusyChanged { tab: t, busy: true },
            Command::OpenCommandBar,
        ];
        for c in &quiet {
            assert!(!changes_session(c), "{c:?}");
        }
        let changing = [
            Command::NewTab {
                url: "u".into(),
                container: None,
                opener: None,
            },
            Command::CloseTab(t),
            Command::ActivateTab(t),
            Command::PinTab(t),
            Command::Navigate {
                tab: t,
                url: "u".into(),
            },
            Command::UrlChanged {
                tab: t,
                url: "u".into(),
            },
            Command::TitleChanged {
                tab: t,
                title: "T".into(),
            },
            Command::RendererGone(t),
        ];
        for c in &changing {
            assert!(changes_session(c), "{c:?}");
        }
    }

    #[test]
    fn visits_are_new_web_pages() {
        let cases = [
            (None, "https://a.test/", true),
            (Some("https://a.test/"), "https://b.test/", true),
            (Some("https://a.test/"), "https://a.test/?q=1", true),
            (Some("https://a.test/"), "https://a.test/", false),
            (Some("https://a.test/"), "https://a.test/#part", false),
            (Some("https://a.test/#one"), "https://a.test/#two", false),
            (Some("https://a.test/#one"), "https://a.test/", false),
            (None, "about:blank", false),
            (None, "wubble://settings", false),
            (None, "data:text/html,x", false),
            (Some("data:text/html,x"), "file:///tmp/x", false),
        ];
        for (previous, new, want) in cases {
            assert_eq!(
                visit_to_record(previous, new),
                want,
                "{previous:?} -> {new}"
            );
        }
    }

    fn closed(url: &str) -> ClosedTab {
        ClosedTab {
            url: url.into(),
            title: String::new(),
            container: ContainerId::DEFAULT,
            space: None,
            closed_at_ms: 1,
        }
    }

    fn save_of(url: &str) -> Request {
        let (model, _) = apply(
            Model::new(std::time::Instant::now()),
            Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            },
        );
        Request::Save {
            model: Box::new(model),
            ui: Settings::default().ui,
        }
    }

    #[test]
    fn only_the_newest_queued_save_runs() {
        let batch = vec![save_of("a"), Request::PushClosed(closed("x")), save_of("b")];
        let kept: Vec<String> = latest_save_only(batch)
            .map(|r| match r {
                Request::Save { model, .. } => model.tabs.values().next().unwrap().url.clone(),
                Request::PushClosed(t) => format!("closed {}", t.url),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(kept, ["closed x", "b"]);
    }

    fn start(path: &Path) -> (StoreClient, Loaded) {
        let (client, loaded) = StoreClient::spawn(path.to_path_buf());
        let loaded = loaded.recv().unwrap().unwrap();
        (client, loaded)
    }

    #[test]
    fn a_session_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p/wubble.db");
        let (client, loaded) = start(&path);
        assert!(loaded.model.tabs.is_empty(), "a fresh database");
        assert_eq!(loaded.settings, Settings::default());

        let mut model = Model::new(std::time::Instant::now());
        for url in ["https://a.test/", "https://b.test/"] {
            model = apply(
                model,
                Command::NewTab {
                    url: url.into(),
                    container: None,
                    opener: None,
                },
            )
            .0;
        }
        let ui = UiState {
            sidebar_width: 300.0,
            mode: SidebarMode::Rail,
        };
        client.save(&model, ui);
        assert!(client.set_setting("search.engine", "google"));
        assert!(client.set_setting("zoom.sites", r#"{"a.test":125}"#));
        assert!(
            !client.set_setting("zoom.default", "9000"),
            "refused before sending"
        );
        assert!(!client.set_setting("no.such", "1"));
        client.record_visit("https://a.test/", "", 5, Transition::Typed);
        client.update_title("https://a.test/", "A");
        client.push_closed(closed("https://gone.test/"));
        client.set_site_permission("https://a.test", Permission::Camera, Decision::Allow, 5);
        let running = DownloadEntry {
            id: 1,
            url: "https://a.test/f.zip".into(),
            name: "f.zip".into(),
            path: "/d/f.zip".into(),
            received: 3,
            total: Some(9),
            state: download::DownloadState::Running,
            started_ms: 5,
        };
        client.add_download(&running);
        let (hits_tx, hits_rx) = mpsc::channel();
        client.query_history("a.t", 10, 5, move |hits| {
            hits_tx.send(hits).unwrap();
        });
        let hits = hits_rx.recv().unwrap().entries;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "A");
        assert_eq!(hits[0].typed_count, 1);
        client.shutdown();

        let (client, loaded) = start(&path);
        let urls: Vec<&str> = loaded
            .model
            .order(loaded.model.active_space)
            .iter()
            .map(|t| loaded.model.tabs[t].url.as_str())
            .collect();
        assert_eq!(urls, ["https://a.test/", "https://b.test/"]);
        let visible = loaded.model.visible_tab().unwrap();
        for (id, tab) in &loaded.model.tabs {
            let want = if *id == visible {
                Liveness::Active
            } else {
                Liveness::Discarded
            };
            assert_eq!(tab.lifecycle.liveness, want);
        }
        let stored = &loaded.settings.stored;
        assert_eq!(stored["search.engine"], "google");
        assert_eq!(stored["zoom.sites"], r#"{"a.test":125}"#);
        assert!(!stored.contains_key("zoom.default"));
        assert_eq!(loaded.settings.ui, ui);
        assert_eq!(
            loaded
                .site_permissions
                .get("https://a.test", Permission::Camera),
            Some(Decision::Allow)
        );
        // The download did not survive the restart: it is listed as failed.
        assert_eq!(
            loaded.downloads,
            vec![DownloadEntry {
                state: download::DownloadState::Failed(download::LOST.into()),
                ..running
            }]
        );
        let (pop_tx, pop_rx) = mpsc::channel();
        client.pop_closed(move |tab| pop_tx.send(tab).unwrap());
        assert_eq!(pop_rx.recv().unwrap(), Some(closed("https://gone.test/")));
        client.shutdown();
    }

    #[test]
    fn an_unreadable_database_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wubble.db");
        std::fs::write(&path, b"not a database, and longer than one header").unwrap();
        let (client, loaded) = StoreClient::spawn(path.clone());
        assert!(loaded.recv().unwrap().is_err());
        client.save(
            &Model::new(std::time::Instant::now()),
            Settings::default().ui,
        );
        client.shutdown();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"not a database, and longer than one header"
        );
    }

    fn icon(url: &str, shade: u8) -> FaviconImage {
        FaviconImage {
            url: url.into(),
            width: 2,
            height: 2,
            rgba: vec![shade; 16],
        }
    }

    fn model_of(urls: &[&str]) -> Model {
        urls.iter().fold(Model::new(Instant::now()), |model, url| {
            apply(
                model,
                Command::NewTab {
                    url: (*url).into(),
                    container: None,
                    opener: None,
                },
            )
            .0
        })
    }

    #[test]
    fn a_saved_icon_comes_back_with_its_tab() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wubble.db");
        let (client, loaded) = start(&path);
        assert!(loaded.favicons.is_empty());
        let model = model_of(&["https://a.test/page", "https://b.test/", "about:blank"]);
        client.save(&model, Settings::default().ui);
        client.put_favicon("https://a.test/other", &icon("https://a.test/i.png", 7), 1);
        client.shutdown();

        let (client, loaded) = start(&path);
        // Filed by origin, handed back under each tab's own URL; a tab with no
        // icon or no origin has no entry.
        assert_eq!(loaded.favicons.len(), 1);
        let got = &loaded.favicons["https://a.test/page"];
        assert_eq!(got.icon_url, "https://a.test/i.png");
        assert_eq!(
            (got.width, got.height, got.rgba.clone()),
            (2, 2, vec![7; 16])
        );
        client.shutdown();
    }

    #[test]
    fn history_hits_carry_their_icons() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _) = start(&dir.path().join("wubble.db"));
        client.record_visit("https://a.test/x", "X", 5, Transition::Typed);
        client.record_visit("https://b.test/", "B", 5, Transition::Typed);
        client.put_favicon("https://a.test/", &icon("https://a.test/i.png", 3), 6);
        let (tx, rx) = mpsc::channel();
        client.query_history("a.test", 10, 5, move |hits| tx.send(hits).unwrap());
        let hits = rx.recv().unwrap();
        assert_eq!(hits.entries.len(), 1);
        assert_eq!(hits.icons.len(), 1);
        assert_eq!(hits.icons["https://a.test/x"].rgba, vec![3; 16]);
        let (tx, rx) = mpsc::channel();
        client.query_history("b.test", 10, 5, move |hits| tx.send(hits).unwrap());
        let hits = rx.recv().unwrap();
        assert_eq!(hits.entries.len(), 1);
        assert!(hits.icons.is_empty(), "no icon stored for b.test");
        client.shutdown();
    }

    #[test]
    fn startup_prunes_icons_over_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wubble.db");
        let (client, _) = start(&path);
        client.put_favicon("https://a.test/", &icon("https://a.test/i.png", 1), 1);
        client.shutdown();
        // Prune directly at a cap below one icon: what the start does with the real cap.
        let mut store = pollster::block_on(Store::open(&path)).unwrap();
        assert_eq!(pollster::block_on(store.prune_favicons(15)).unwrap(), 1);
        drop(store);
        let (client, loaded) = start(&path);
        assert!(loaded.favicons.is_empty());
        client.shutdown();
        const _: () = assert!(FAVICON_CAP_BYTES >= 64 * 64 * 4);
    }

    #[test]
    fn a_setting_written_is_read_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (client, loaded) = start(&dir.path().join("wubble.db"));
        assert!(loaded.settings.stored.is_empty());
        assert!(client.set_setting("blocker.enabled", "0"));
        let (tx, rx) = mpsc::channel();
        client.read_settings(move |rows| tx.send(rows).unwrap());
        assert_eq!(rx.recv().unwrap()["blocker.enabled"], "0");
        client.set_site_permission("https://a.test", Permission::Camera, Decision::Allow, 5);
        client.forget_site_permission("https://a.test", Permission::Camera);
        client.shutdown();
        let (client, loaded) = start(&dir.path().join("wubble.db"));
        assert!(!loaded.settings.blocker.enabled);
        assert_eq!(
            loaded
                .site_permissions
                .get("https://a.test", Permission::Camera),
            None
        );
        let features = loaded.settings.page_features();
        assert!(features.overlay_scrollbars && features.middle_click_autoscroll);
        client.shutdown();
    }
}
