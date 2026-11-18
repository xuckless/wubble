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

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;

use wubble_core::model::{Command, Model};
use wubble_core::omnibox;
use wubble_core::tab::TabId;
use wubble_store::{
    ClosedTab, CompactMode, HistoryHit, KeybindingOverrides, LiveSuggestions, SearchEngineChoice,
    Setting, SidebarWidth, Store, Transition,
};

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
        Command::Tick { .. } | Command::LoadingChanged { .. } | Command::OpenCommandBar
    )
}

/// Whether committing `new` in a tab whose last commit was `previous` is a
/// history visit: a web page, and not the same page again or a move to
/// another fragment of it (a restored tab reloading, an in-page link).
pub fn visit_to_record(previous: Option<&str>, new: &str) -> bool {
    let page = |url: &str| url.split_once('#').map_or(url, |(page, _)| page).to_owned();
    wubble_store::is_recordable(new) && previous.map(page) != Some(page(new))
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

/// UI state saved with the session: the typed settings `SidebarWidth` and
/// `CompactMode` (Expanded or Hidden; peeking is never stored).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiState {
    pub sidebar_width: f32,
    pub compact: bool,
}

/// The typed settings the host reads at start.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// `None`: not chosen yet (first run).
    pub engine: Option<omnibox::SearchEngine>,
    pub live_suggestions: bool,
    pub ui: UiState,
    /// Opaque entries, decoded by `input::keymap`.
    pub keybinding_overrides: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: SearchEngineChoice::default(),
            live_suggestions: LiveSuggestions::default(),
            ui: UiState {
                sidebar_width: SidebarWidth::default(),
                compact: CompactMode::default(),
            },
            keybinding_overrides: KeybindingOverrides::default(),
        }
    }
}

/// What the store thread hands over at start.
#[derive(Debug)]
pub struct Loaded {
    pub model: Model,
    pub settings: Settings,
}

impl Default for Loaded {
    fn default() -> Self {
        Self {
            model: Model::new(std::time::Instant::now()),
            settings: Settings::default(),
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
        reply: Reply<Vec<HistoryHit>>,
    },
    PushClosed(ClosedTab),
    PopClosed(Reply<Option<ClosedTab>>),
    SetEngine(omnibox::SearchEngine),
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
        reply: impl FnOnce(Vec<HistoryHit>) + Send + 'static,
    ) {
        self.send(Request::Query {
            prefix: prefix.into(),
            now_ms,
            limit,
            reply: Box::new(reply),
        });
    }

    pub fn push_closed(&self, tab: ClosedTab) {
        self.send(Request::PushClosed(tab));
    }

    /// `reply` runs on the store thread.
    pub fn pop_closed(&self, reply: impl FnOnce(Option<ClosedTab>) + Send + 'static) {
        self.send(Request::PopClosed(Box::new(reply)));
    }

    pub fn set_engine(&self, engine: omnibox::SearchEngine) {
        self.send(Request::SetEngine(engine));
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
    let store = Store::open(path).await?;
    let model = store.load_session().await?;
    let settings = Settings {
        engine: store.setting::<SearchEngineChoice>().await?,
        live_suggestions: store.setting::<LiveSuggestions>().await?,
        ui: UiState {
            sidebar_width: store.setting::<SidebarWidth>().await?,
            compact: store.setting::<CompactMode>().await?,
        },
        keybinding_overrides: store.setting::<KeybindingOverrides>().await?,
    };
    Ok((store, Loaded { model, settings }))
}

async fn handle(store: &mut Store, request: Request) -> wubble_store::Result<()> {
    match request {
        Request::Save { model, ui } => {
            store.save_session(&model).await?;
            store.set_setting::<SidebarWidth>(ui.sidebar_width).await?;
            store.set_setting::<CompactMode>(ui.compact).await?;
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
            let hits = store.query_history(&prefix, now_ms, limit).await;
            match hits {
                Ok(hits) => reply(hits),
                Err(e) => {
                    reply(vec![]);
                    return Err(e);
                }
            }
        }
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
        Request::SetEngine(engine) => {
            store
                .set_setting::<SearchEngineChoice>(Some(engine))
                .await?;
        }
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
            Command::LoadingChanged {
                tab: t,
                loading: true,
            },
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
            compact: true,
        };
        client.save(&model, ui);
        client.set_engine(omnibox::SearchEngine::Google);
        client.record_visit("https://a.test/", "", 5, Transition::Typed);
        client.update_title("https://a.test/", "A");
        client.push_closed(closed("https://gone.test/"));
        let (hits_tx, hits_rx) = mpsc::channel();
        client.query_history("a.t", 10, 5, move |hits| {
            hits_tx.send(hits).unwrap();
        });
        let hits = hits_rx.recv().unwrap();
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
        assert_eq!(loaded.settings.engine, Some(omnibox::SearchEngine::Google));
        assert_eq!(loaded.settings.ui, ui);
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
        client.save(&Model::new(std::time::Instant::now()), Settings::default().ui);
        client.shutdown();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"not a database, and longer than one header"
        );
    }
}
