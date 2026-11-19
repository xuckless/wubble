//! The windowed browser: one [`Model`], one view per live tab, the pump.
//!
//! Every change to tabs is a [`Command`] passed to [`Host::dispatch`], which
//! runs `model::apply` and executes the returned effects in [`Host::execute`]
//! (through the pure [`call`] table). UI callbacks and engine events only
//! ever become commands. The sidebar is re-derived from the model after each
//! change (`shell::view_model`). Only the views on screen (the visible tab,
//! or each pane of a shown split) take frames, layout and input; the others
//! are hidden (`was_hidden`). Keys go to the focused one, the visible tab.
//!
//! Persistence runs at the edge: the session is saved through the store
//! thread (`persist`) when the save policy says so and at shutdown, visits
//! and titles are recorded as the engine reports them, and closed tabs go
//! on the store's recently-closed list. Keys become actions through the
//! keymap, and every action runs through [`Host::perform`].

mod autofill;
mod autoscroll;
mod ax;
mod boost;
mod builtins;
mod busy;
mod devtools;
mod downloads;
mod drag;
mod external;
mod gallery;
mod gesture;
mod gpu;
mod hover;
mod look;
mod media;
mod memory;
mod overlay;
mod pages;
mod panes;
mod parity;
mod preview;
mod print;
mod settings;
mod snapshots;
pub(crate) mod start;
mod status;
mod surfaces;
mod swipe;
mod tab_drag;
mod tasks;
mod tooltip;
mod videos;
mod window;
pub(crate) use boost::KIT;
pub use media::MediaObserver;
pub use swipe::Swipe;

pub use builtins::Builtin;
pub(crate) use window::WindowHost;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use slint::ComponentHandle;
use slint::platform::PointerEventButton;
use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use wubble_core::action::Action;
use wubble_core::archive::{self, ArchiveAfter};
use wubble_core::busy::Reason;
use wubble_core::devtools::PANEL_TAB;
use wubble_core::keys::{Keymap, Platform};
use wubble_core::lifecycle::Effect as Life;
use wubble_core::model::{self, Command, Effect, Model};
use wubble_core::neterror::CertExceptions;
use wubble_core::omnibox::{self, HistoryEntry, SearchEngine, Target};
use wubble_core::pages::InternalPage;
use wubble_core::region::{Region, next_region};
use wubble_core::site::{Permission, SitePermissions};
use wubble_core::space::{ContainerId, Item, SpaceId};
use wubble_core::tab::{Liveness, TabId};
use wubble_core::touch::InputEvent;
use wubble_core::window::{WindowId, WindowKind};
use wubble_core::zoom::ZoomTable;
use wubble_engine::app::{Engine, PumpOutcome, PumpTimer, RequestMode, ViewId};
use wubble_engine::input::{KeyId, KeyInput, Point};
use wubble_engine::nav::{Cursor, Phase, PopupRequest, WebEvent};
use wubble_engine::view::{View, ViewConfig};
use wubble_gfx::pacing::{self, BeginFrames, FrameStats, PaceEffect, PaceEvent};
use wubble_store::{ArchivedTab, ClosedTab, Transition};
use wubble_ui::{AppWindow, CommandBarState, FocusRegion, GlassKeys, SidebarMode};

use crate::bar::{self, BarInputs, BarMode, BarRow, Choice, RowTarget};
use crate::block_smoke::{self, BlockSmoke};
use crate::blocking::Blocking;
use crate::clipboard::Bridge as _;
use crate::ime::Ime;
use crate::input::{self, InputState, KeyFocus, KeyRoute, PendingKeys};
use crate::measure;
use crate::nav_smoke::{self, NavSmoke, SmokeAction, SmokeInput};
use crate::page_error::ErrorEvent;
use crate::page_smoke::{self, PageSmoke};
use crate::pages_smoke::{self, PagesSmoke};
use crate::perform::{self, Plan, ViewOp};
use crate::persist::{self, Loaded, SaveState, StoreClient, TypedNav, UiState};
use crate::pixels;
use crate::screen::{Screen, ScreenEvent};
use crate::session_smoke::{self, SessionSmoke};
use crate::settings_smoke::{self, SettingsSmoke};
use crate::shell::AudioMap;
use crate::split_smoke::{self, SplitSmoke};
use crate::surface::{FindEvent, Surfaces};
use crate::tabs_smoke::{self, Seen, TabsSmoke};
use crate::{
    NAV_PAGE, SMOKE_MAX_IDLE_RUNS, SMOKE_PAGE, TAB3_PAGE, TIMERS_PAGE, Wake, deliver,
    display_refresh_mhz, page_url, physical, quit, with_host,
};

/// Pop-ups a tab holds back at most; later ones are dropped.
const MAX_HELD_POPUPS: usize = 8;

/// How far ahead `--smoke tabs` ticks the lifecycle: past the discard delay.
const SMOKE_DISCARD_AFTER: Duration = Duration::from_secs(20 * 60);

/// What executing one effect asks of the engine or the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// Open a view for the tab unless it has one.
    OpenView {
        tab: TabId,
        url: String,
    },
    /// Close the tab's view if it has one.
    CloseView(TabId),
    Load {
        tab: TabId,
        url: String,
    },
    SetVisible {
        tab: TabId,
        visible: bool,
    },
    /// Freeze or thaw the tab's page (`View::set_frozen`, CDP).
    SetFrozen {
        tab: TabId,
        frozen: bool,
    },
    /// Keep a small picture of the tab's last frame (`host/snapshots.rs`).
    CaptureSnapshot(TabId),
    ReleaseSnapshot(TabId),
    ShowCommandBar,
    /// Open, close or raise the platform window of a model window.
    OpenWindow(WindowId),
    CloseWindow(WindowId),
    FocusWindow(WindowId),
    /// The last Main window closed.
    Quit,
    /// Delete the container's web storage (`Engine::forget_container`).
    ForgetContainer(ContainerId),
    Nothing,
}

/// The engine work an effect stands for. Freeze goes through the DevTools
/// protocol (CEF 156 has no freeze API; `wubble_engine::cdp`); snapshots are
/// the host's own GPU textures (`host/snapshots.rs`).
/// An internal page (`InternalPage`, wubble://) has no view: the shell draws it, so creating
/// or restoring its tab opens none. Only a tab already on an internal page navigates to one
/// (the model opens it in its own tab otherwise), so there is no view to load into.
pub fn call(effect: &Effect, model: &Model) -> Call {
    let internal = |url: &str| InternalPage::from_url(url).is_some();
    match effect {
        Effect::CreateTab { url, .. } if internal(url) => Call::Nothing,
        Effect::CreateTab { tab, url, .. } => Call::OpenView {
            tab: *tab,
            url: url.clone(),
        },
        Effect::CloseTab(tab) => Call::CloseView(*tab),
        Effect::Navigate { url, .. } if internal(url) => Call::Nothing,
        Effect::Navigate { tab, url } => Call::Load {
            tab: *tab,
            url: url.clone(),
        },
        Effect::ShowCommandBar => Call::ShowCommandBar,
        // Which view is shown follows the lifecycle effects; focus and
        // frames follow `Model::visible_tab`.
        Effect::ActivateTab(_) | Effect::SpaceChanged(_) => Call::Nothing,
        Effect::WindowOpened(w) => Call::OpenWindow(*w),
        Effect::WindowClosed(w) => Call::CloseWindow(*w),
        Effect::FocusWindow(w) => Call::FocusWindow(*w),
        Effect::LastWindowClosed => Call::Quit,
        Effect::Tidied { .. } => Call::Nothing,
        Effect::ContainerDeleted(c) => Call::ForgetContainer(*c),
        Effect::Lifecycle { tab, effect } => match effect {
            Life::Show => Call::SetVisible {
                tab: *tab,
                visible: true,
            },
            Life::Hide => Call::SetVisible {
                tab: *tab,
                visible: false,
            },
            Life::Discard | Life::ReleaseTexture => Call::CloseView(*tab),
            // A live view's crash shows the error panel, whose Reload
            // recovers (`page_error`); this opens a view only where none is.
            Life::Restore => {
                model
                    .tabs
                    .get(tab)
                    .filter(|t| !internal(&t.url))
                    .map_or(Call::Nothing, |t| Call::OpenView {
                        tab: *tab,
                        url: t.url.clone(),
                    })
            }
            Life::Freeze | Life::Unfreeze => Call::SetFrozen {
                tab: *tab,
                frozen: *effect == Life::Freeze,
            },
            Life::CaptureSnapshot => Call::CaptureSnapshot(*tab),
            Life::ReleaseSnapshot => Call::ReleaseSnapshot(*tab),
        },
    }
}

/// The loaded session and its first effects. The store brings the visible
/// tabs (each pane of a shown split) back Active, but they have no view yet:
/// they are restored, unless `opening` (a URL from the command line) is about
/// to take their place. Then they are marked Discarded, what they really are,
/// and get their views when activated again instead of loading in the background.
pub fn startup(mut model: Model, opening: bool) -> (Model, Vec<Effect>) {
    let visible = model.visible_tabs();
    if opening {
        for t in &visible {
            if let Some(t) = model.tabs.get_mut(t) {
                t.lifecycle.liveness = Liveness::Discarded;
            }
        }
        return (model, vec![]);
    }
    let restore = visible
        .into_iter()
        .map(|tab| Effect::Lifecycle {
            tab,
            effect: Life::Restore,
        })
        .collect();
    (model, restore)
}

/// What the command bar was opened for and what it lists.
struct Bar {
    mode: BarMode,
    query: String,
    rows: Vec<BarRow>,
    /// The store's latest history hits; ranked against each new query until
    /// that query's own hits arrive.
    history: Vec<HistoryEntry>,
    /// The icons of those hits' origins, by hit URL; the history rows draw them.
    icons: HashMap<String, wubble_store::Favicon>,
    /// The bookmarked links, from the same reply.
    bookmarks: Vec<omnibox::Bookmark>,
    /// The engine's live suggestions for the query (`suggest_fx`; empty unless the setting is on).
    suggestions: Vec<String>,
    /// Bumped per query; a history reply for an older one is dropped.
    generation: u64,
}

/// What a windowed run is for, as far as the host cares.
pub struct Checks {
    /// Accelerated frames after which the loop quits (`--smoke frames`).
    pub quit_after: Option<u64>,
    /// Snapshot the window before quitting and require the content area to
    /// show the page. Only for the bundled page: its colours are known.
    pub check_pixels: bool,
    pub nav_smoke: Option<NavSmoke>,
    pub tabs_smoke: Option<TabsSmoke>,
    pub session_smoke: Option<SessionSmoke>,
    pub page_smoke: Option<PageSmoke>,
    /// `--smoke parity` (`host/parity.rs`).
    pub parity: Option<parity::ParityRun>,
    pub block_smoke: Option<BlockSmoke>,
    pub split_smoke: Option<SplitSmoke>,
    pub settings_smoke: Option<SettingsSmoke>,
    pub start_smoke: Option<crate::start_smoke::StartSmoke>,
    pub pages_smoke: Option<PagesSmoke>,
    pub gallery_smoke: Option<crate::gallery_smoke::Stage>,
    pub autofill_smoke: Option<crate::autofill_smoke::AutofillSmoke>,
    pub windows_smoke: Option<crate::windows_smoke::WindowsSmoke>,
    pub media_smoke: Option<crate::media_smoke::MediaSmoke>,
    pub builtin_smoke: Option<crate::builtin_smoke::BuiltinSmoke>,
    /// `--smoke video` and `--smoke boost`.
    pub video_smoke: Option<crate::video_smoke::VideoSmoke>,
    /// `--smoke ax`.
    pub ax_smoke: Option<crate::ax_smoke::AxSmoke>,
    /// `--smoke overlay` (`host/overlay.rs`).
    pub overlay_smoke: Option<overlay::Run>,
    /// SponsorBlock's endpoint for `--smoke video` (its loopback fake).
    pub sponsor_endpoint: Option<String>,
    /// Views paint on external begin frames from [`Host::pace`].
    pub begin_frames: bool,
}

/// The last frame shown in a pane: the image, the part of it to show, and what the pane shows
/// of it while its size changes (`wubble_core::stretch`; the view's state carries the size CEF
/// painted, the engine's `ViewState` the size it wants).
struct PaneFrame {
    image: slint::Image,
    clip: wubble_gfx::FrameSize,
    view: wubble_core::stretch::PaneView,
    springs: wubble_core::stretch::PaneSprings,
    /// The stale frame (and its clip) fading out over this one while `view` is fading.
    prev: Option<(slint::Image, wubble_gfx::FrameSize)>,
    /// The frame cut along a bowed string, while one bends this pane's edge.
    masked: Option<wubble_gfx::mask::Masked>,
}

impl PaneFrame {
    /// The stale frame still fading out over this one: its image, clip and opacity.
    fn fading(&self) -> Option<(&slint::Image, wubble_gfx::FrameSize, f32)> {
        let (image, clip) = self.prev.as_ref().filter(|_| self.view.is_fading())?;
        Some((image, *clip, (1.0 - self.springs.fade()) as f32))
    }
}

/// The external begin-frame driver's state and its timer.
struct Pace {
    state: BeginFrames,
    timer: slint::Timer,
}

/// Everything the pump drives. Owned by `HOST` in `main.rs`.
pub struct Host {
    pub engine: Engine,
    /// The window the host works on now (`window.rs`); the others are parked.
    w: WindowHost,
    parked: BTreeMap<WindowId, WindowHost>,
    /// Makes the platform window of each model window that opens.
    factory: crate::WindowFactory,
    /// The engine's single-shot pump timer.
    pub timer: slint::Timer,
    /// The one owner of tabs and Spaces.
    model: Model,
    /// The view of every live tab.
    views: BTreeMap<TabId, ViewId>,
    /// Effects not yet executed: they wait until the window can size a view.
    pending: VecDeque<Effect>,
    /// CEF's frame rate, known once winit has created the window.
    frame_rate: Option<u32>,
    bar: Bar,
    /// The view last told about keyboard focus.
    focus_view: Option<ViewId>,
    /// The pane the last press took the keys to when it is not the model's shown page: the
    /// DevTools panel (`PANEL_TAB`) or the window's peek. Ignored once that pane is gone.
    key_pane: Option<TabId>,
    pub stats: FrameStats,
    last_present: Option<Instant>,
    idle_runs: u32,
    checks: Checks,
    pub nav_title: Option<String>,
    pub tabs_passed: bool,
    pub session_passed: bool,
    /// `--smoke session`: whether the store returned the icon the first run filed.
    session_icon: Option<bool>,
    pub split_passed: bool,
    /// `--smoke parity` passed: the known gaps that failed as expected.
    pub parity_passed: Option<Vec<&'static str>>,
    pub windows_passed: bool,
    pub settings_passed: bool,
    pub start_passed: bool,
    pub pages_passed: bool,
    /// `--smoke gallery` shot every scene.
    pub gallery_passed: bool,
    /// The scene on screen waited out [`gallery::SETTLE`] (`host/gallery.rs`).
    gallery_timer: slint::Timer,
    /// When the gallery's scene was shown: a burst's frames are timed from it.
    gallery_burst_start: std::time::Instant,
    /// `--smoke settings`: the stored value of the setting it flipped, read back.
    settings_read: Option<String>,
    pub(crate) input: InputState,
    keymap: Keymap,
    /// Page-first actions waiting to hear the page left their key alone.
    page_first: PendingKeys,
    store: StoreClient,
    /// Session changes not yet saved.
    save: SaveState,
    /// `None` until the first-run choice.
    /// A page's drag inside the window, and the pointer it last went to
    /// (relative to the drag's page).
    autoscroll: autoscroll::AutoscrollPointer,
    page_drag: crate::page_drag::PageDrag,
    drag_point: Point,
    /// A drag from another app over the window (`drag.rs`).
    external_drag: Option<drag::ExternalDrag>,
    /// Touchpad pinch and history swipe (`gesture.rs`).
    gestures: gesture::Gestures,
    /// The print sheet (`print.rs`).
    print: wubble_core::print::Sheet,
    /// Opened print sheets: the pages field restarts with each.
    print_serial: i32,
    /// The private folder printer jobs' PDFs go to, made with the first (removed on exit).
    print_dir: Option<tempfile::TempDir>,
    search_engine: Option<SearchEngine>,
    live_suggestions: bool,
    /// The live suggestion fetch (debounce, worker); sends nothing unless `live_suggestions`.
    suggest: crate::suggest_fx::SuggestFx,
    /// Which commit is a typed visit.
    typed_nav: TypedNav,
    /// Each tab's last committed URL: a commit that only changes the
    /// fragment, or repeats it (a restored tab), is no new visit.
    committed: BTreeMap<TabId, String>,
    /// Why the loop was quit early, if it was.
    pub failure: Option<String>,
    /// The page's own UI per tab (one owner); only the visible tab's shows.
    surfaces: BTreeMap<TabId, Surfaces>,
    /// Pop-ups each tab's page asked for and the policy held back (`wubble_core::popup`),
    /// until the page commits another URL; the address pill offers them.
    held_popups: BTreeMap<TabId, Vec<PopupRequest>>,
    /// A tab dragged from a sidebar row (`drag.rs`): one pointer, so one for every window.
    tab_drag: wubble_core::tabdrag::Drag,
    /// Where the pointer was at the drag's last event, window px.
    drag_at: wubble_core::tabdrag::Point,
    /// The window the drag started in: its pointer drives the drag, its ghost shows it.
    drag_window: WindowId,
    /// Asks the drag again once the pointer has rested on a Space's icon for the hover delay.
    drag_timer: slint::Timer,
    /// Window and page fullscreen.
    screen: Screen,
    /// The page's IME composition.
    ime: Ime,
    /// Sound per live tab, as its view reported it.
    audio: AudioMap,
    /// Each tab's page icon: the store's at start, then as its view last downloaded it (and
    /// filed in the store); shown on its row.
    favicons: BTreeMap<TabId, slint::Image>,
    /// Per-site zoom; stored through the store, applied on each commit.
    site_zoom: ZoomTable,
    /// The zoom of sites without their own (`settings::ZOOM_DEFAULT`).
    zoom_default: u32,
    /// The settings page and the stored setting rows.
    settings: settings::SettingsPage,
    /// The history, bookmarks and passwords pages.
    pages: pages::PagesState,
    /// Bumped per notice and per find-bar focus request.
    toast_serial: i32,
    find_focus: i32,
    /// The zoom `--smoke page` passed with.
    pub page_zoom: Option<u32>,
    /// The blocked count `--smoke block` passed with.
    pub blocked_seen: Option<u32>,
    /// `--smoke autofill` reached its end.
    pub autofill_passed: bool,
    /// `--smoke media` reached its end.
    pub media_passed: bool,
    /// `--smoke builtins`: what it checked, once it passed.
    pub builtins_passed: Option<String>,
    /// Password autofill per tab (`host/autofill.rs`).
    autofill: autofill::Autofill,
    /// The content blocker, whose per-view counts the tab rows show.
    blocking: Arc<Blocking>,
    /// Remembered permission decisions (one owner; stored through the store).
    site_permissions: SitePermissions,
    /// Certificate errors the user proceeded past, for this run only.
    cert_exceptions: CertExceptions,
    downloads: downloads::Downloads,
    /// The media downloader: each tab's streams and the download jobs (`host/media.rs`).
    media: media::Media,
    pace: Option<Pace>,
    /// The system clipboard, for the pages' copies and pastes and the shell's copies.
    clipboard: crate::clipboard::Clipboard,
    /// Auto-archive: how long a tab may go unvisited (`archive.after`).
    archive_after: ArchiveAfter,
    /// Tabs being filed in the Archive: they close once the store has them, and do not go on
    /// the recently-closed list as well.
    /// The tab's `last_active_ms` when it was filed: a tab visited meanwhile stays open.
    archiving: HashMap<TabId, i64>,
    /// The `routing.rules` entries as stored (`routing::resolve` reads them).
    routing_rules: Vec<String>,
    /// The layout before the last tidy, to undo it with: one slot.
    tidy_undo: Option<(SpaceId, Vec<Item>)>,
    /// The tab hover preview and the thumbnails it shows.
    preview: preview::Preview,
    /// The link status bubble (`host/status.rs`).
    status: status::Status,
    /// The window's tooltip (`host/tooltip.rs`).
    tips: tooltip::Tips,
    /// How links to other apps are opened (`host/external.rs`).
    pub(crate) external_opener: external::Opener,
    /// The GPU process's id, watched on the lifecycle tick (`host/gpu.rs`).
    gpu: gpu::GpuWatch,
    /// Probed busy reasons per frame (`host/busy.rs`).
    busy: busy::BusyFrames,
    /// The memory pressure source and its gauge, read on the lifecycle tick.
    pressure: busy::PressureGauge,
    /// Pictures of hidden tabs (`host/snapshots.rs`).
    snapshots: snapshots::Snapshots,
    /// The Task manager page's meter (`host/tasks.rs`).
    tasks: tasks::TasksPage,
    /// DevTools: the dock's state and its panel (`host/devtools.rs`).
    devtools: devtools::Docked,
    /// Dark mode for pages, reader mode, link hints, screenshots (`host/builtins.rs`).
    builtins: builtins::Builtins,
    /// Boosts and the page kit (`host/boost.rs`).
    boosts: boost::Boosts,
    /// Video controls, SponsorBlock, picture-in-picture, media keys (`host/videos.rs`).
    videos: videos::Videos,
    /// Pages for screen readers (`host/ax.rs`).
    ax: ax::WebAx,
    /// Reduced motion and touch sizing (`look.rs`).
    look: look::Look,
    /// Shift+click opens a peek (`glance.enabled`).
    glance_enabled: bool,
    /// `--smoke video` or `--smoke boost` passed.
    pub video_passed: Option<String>,
    /// `--smoke ax` passed, with the page's item count.
    pub ax_passed: Option<usize>,
    /// `--smoke overlay` passed.
    pub overlay_passed: bool,
    overlay_timer: slint::Timer,
}

impl Host {
    /// A host on the loaded session. Only the visible tab gets a view (once
    /// the window exists); the other restored tabs stay Discarded until
    /// activated, through the `Life::Restore` mapping. `open` is loaded in a
    /// new tab, which is then the only one with a view (see [`startup`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: Engine,
        w: WindowHost,
        factory: crate::WindowFactory,
        loaded: Loaded,
        store: StoreClient,
        open: Option<String>,
        checks: Checks,
        downloads_dir: Option<PathBuf>,
        blocking: Arc<Blocking>,
        media_observer: Arc<media::MediaObserver>,
    ) -> Self {
        let Loaded {
            model,
            settings,
            site_permissions,
            downloads,
            favicons,
            boosts,
        } = loaded;
        let favicons = model
            .tabs
            .values()
            .filter_map(|t| {
                let icon = favicons.get(&t.url)?;
                Some((
                    t.id,
                    crate::bind::favicon_image(icon.width, icon.height, &icon.rgba),
                ))
            })
            .collect();
        let committed = model.tabs.values().map(|t| (t.id, t.url.clone())).collect();
        let (model, restore) = startup(model, open.is_some());
        let dock_port = settings.page_features().remote_debugging_port;
        // The shell draws the page's dialogs, menus and prompts.
        let mut engine = engine;
        let clipboard = crate::clipboard::Clipboard::new(engine.cache_dir());
        engine.set_request_mode(RequestMode::AskHost);
        let begin_frames = checks.begin_frames;
        let videos = match checks.sponsor_endpoint.clone() {
            Some(endpoint) => {
                videos::Videos::for_smoke(endpoint, crate::video_smoke::SMOKE_VIDEO.into())
            }
            None => videos::Videos::default(),
        };
        // `--smoke parity` records the links it would hand to the OS (and saves without the save
        // dialog, below).
        let external_opener: external::Opener = if checks.parity.is_some() {
            parity::record_opened
        } else {
            external::open_with_os
        };
        let mut host = Self {
            engine,
            w,
            parked: BTreeMap::new(),
            factory,
            timer: slint::Timer::default(),
            model,
            views: BTreeMap::new(),
            pending: restore.into(),
            frame_rate: None,
            bar: Bar {
                mode: BarMode::NewTab,
                query: String::new(),
                rows: vec![],
                history: vec![],
                icons: HashMap::new(),
                bookmarks: vec![],
                suggestions: vec![],
                generation: 0,
            },
            focus_view: None,
            key_pane: None,
            stats: FrameStats::default(),
            last_present: None,
            idle_runs: 0,
            checks,
            nav_title: None,
            tabs_passed: false,
            session_passed: false,
            session_icon: None,
            split_passed: false,
            parity_passed: None,
            windows_passed: false,
            settings_passed: false,
            start_passed: false,
            pages_passed: false,
            gallery_passed: false,
            gallery_timer: slint::Timer::default(),
            gallery_burst_start: std::time::Instant::now(),
            settings_read: None,
            input: InputState::default(),
            // `replay_settings` below applies the stored overrides, engine, zoom, ...
            keymap: Keymap::defaults(Platform::current()),
            page_first: PendingKeys::default(),
            store,
            save: SaveState::default(),
            autoscroll: Default::default(),
            page_drag: Default::default(),
            drag_point: Point { x: 0.0, y: 0.0 },
            external_drag: None,
            gestures: Default::default(),
            print: Default::default(),
            print_serial: 0,
            print_dir: None,
            search_engine: None,
            live_suggestions: false,
            suggest: crate::suggest_fx::SuggestFx::new(crate::suggest_fx::Http::new(), |e| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.suggest_event(e));
                });
            }),
            typed_nav: TypedNav::default(),
            committed,
            failure: None,
            surfaces: BTreeMap::new(),
            held_popups: BTreeMap::new(),
            tab_drag: wubble_core::tabdrag::Drag::Idle,
            drag_at: wubble_core::tabdrag::Point { x: 0.0, y: 0.0 },
            drag_window: WindowId(0),
            drag_timer: slint::Timer::default(),
            screen: Screen::default(),
            ime: Ime::default(),
            audio: AudioMap::new(),
            favicons,
            site_zoom: ZoomTable::new(),
            zoom_default: wubble_core::zoom::DEFAULT_PERCENT,
            settings: settings::SettingsPage::new(settings.stored),
            pages: pages::PagesState::default(),
            toast_serial: 0,
            find_focus: 0,
            page_zoom: None,
            blocked_seen: None,
            autofill_passed: false,
            media_passed: false,
            builtins_passed: None,
            autofill: autofill::Autofill::new(),
            blocking,
            site_permissions,
            cert_exceptions: CertExceptions::default(),
            downloads: downloads::Downloads::new(downloads_dir, downloads),
            media: media::Media::new(media_observer),
            pace: begin_frames.then(|| Pace {
                state: BeginFrames::Off,
                timer: slint::Timer::default(),
            }),
            clipboard,
            archive_after: ArchiveAfter::Off,
            archiving: HashMap::new(),
            routing_rules: vec![],
            tidy_undo: None,
            preview: preview::Preview::new(),
            status: status::Status::new(),
            tips: tooltip::Tips::new(),
            external_opener,
            gpu: gpu::GpuWatch::default(),
            busy: busy::BusyFrames::default(),
            pressure: busy::PressureGauge::system(),
            snapshots: snapshots::Snapshots::default(),
            tasks: tasks::TasksPage::default(),
            devtools: devtools::Docked::new(dock_port),
            builtins: builtins::Builtins::default(),
            boosts: boost::Boosts::new(boosts),
            videos,
            ax: ax::WebAx::default(),
            video_passed: None,
            ax_passed: None,
            overlay_passed: false,
            overlay_timer: slint::Timer::default(),
            look: look::Look::new(),
            glance_enabled: true,
        };
        host.replay_settings();
        // `--smoke parity` drives Save As without an OS dialog.
        host.downloads.never_ask = host.checks.parity.is_some();
        if host.checks.check_pixels {
            // The glass check is a correctness check of the scene's full rendering: the governor
            // would otherwise pick the level from how fast this machine runs (and at `Flat` the
            // glass is legitimately one flat colour).
            host.w
                .glass
                .borrow_mut()
                .set_setting(wubble_core::apply::GlassSetting::Full);
        }
        if let Some(window) = host.w.window.upgrade() {
            host.render_downloads(&window);
        }
        host.session_icon_setup();
        match open {
            Some(url) => host.open_routed(url, None),
            None => host.after_change(),
        }
        host
    }

    /// Makes window `id` the one the host works on (`window.rs`). False if it has no window
    /// (closed, or not opened yet): the host stays on the one it was on.
    pub fn enter(&mut self, id: WindowId) -> bool {
        if self.w.id == id {
            return true;
        }
        let Some(mut next) = self.parked.remove(&id) else {
            return false;
        };
        std::mem::swap(&mut self.w, &mut next);
        self.parked.insert(next.id, next);
        true
    }

    /// Opens the platform windows of the session's other windows (the focused one exists).
    pub fn open_session_windows(&mut self) {
        let ids: Vec<WindowId> = self.model.windows.iter().map(|w| w.id).collect();
        for id in ids {
            self.open_window(id);
        }
        self.after_change();
    }

    /// Opens model window `id`'s platform window, unless it has one: a Main window takes the
    /// sidebar of the focused Main window.
    fn open_window(&mut self, id: WindowId) {
        let Some(w) = self.model.window(id) else {
            return;
        };
        if self.w.id == id || self.parked.contains_key(&id) {
            return;
        }
        let (kind, geometry) = (w.kind(), w.geometry);
        let ui = self.ui_state();
        let opened = match self.factory.open(id, kind, ui, geometry) {
            Ok(opened) => opened,
            Err(e) => {
                eprintln!("wubble: window {}: {e}", id.0);
                return;
            }
        };
        look::paint(&opened.handle, self.look());
        if let Err(e) = opened.handle.show() {
            eprintln!("wubble: window {}: {e}", id.0);
            return;
        }
        self.parked.insert(id, opened);
    }

    /// Closes window `id`'s platform window. Its frames and rings go with it; the tabs it
    /// showed are hidden (or closed) by the model.
    fn close_window(&mut self, id: WindowId) {
        if self.w.id == id {
            let Some(&other) = self.parked.keys().next() else {
                return;
            };
            self.enter(other);
        }
        if let Some(gone) = self.parked.remove(&id) {
            self.aux().remove_window(id);
            let _ = gone.handle.hide();
            // picture-in-picture's window went, whoever closed it
            if gone.kind == WindowKind::Pip {
                self.pip_window_closed();
            }
        }
    }

    /// Raises and focuses window `id`.
    fn raise_window(&mut self, id: WindowId) {
        let handle = if self.w.id == id {
            Some(&self.w.handle)
        } else {
            self.parked.get(&id).map(|w| &w.handle)
        };
        if let Some(handle) = handle {
            handle.window().with_winit_window(|w| w.focus_window());
        }
    }

    /// The sidebar to keep: the focused Main window's (a Little or Popup window has none).
    pub fn ui_state(&mut self) -> UiState {
        let here = self.w.id;
        self.enter(self.model.main_window());
        let ui = ui_state(&self.w.handle);
        self.enter(here);
        ui
    }

    /// Records every open window's size and (where the platform tells) position in the model,
    /// for the session. Read on save rather than on each resize event.
    pub fn record_geometry(&mut self) {
        let here = self.w.id;
        let mut changed = vec![];
        for id in self.window_ids() {
            if !self.enter(id) {
                continue;
            }
            let geometry = window_geometry(&self.w.handle);
            if self
                .model
                .window(id)
                .is_some_and(|w| w.geometry != Some(geometry))
            {
                changed.push(Command::ResizeWindow {
                    window: id,
                    geometry,
                });
            }
        }
        self.enter(here);
        // not `dispatch`: nothing on screen changes, and a save is what asked
        for command in changed {
            let current = std::mem::replace(&mut self.model, Model::new(Instant::now()));
            let (model, effects) = model::apply(current, command);
            self.model = model;
            self.pending.extend(effects);
        }
    }

    /// Enters the model's focused window, if it has one open.
    pub fn enter_focused(&mut self) {
        self.enter(self.model.focused());
    }

    /// The only way tabs change: apply, then execute the effects.
    pub fn dispatch(&mut self, command: Command) {
        // the window manager's close of the Pip window is the user's close: it pauses the video
        if let Command::CloseWindow(id) = command
            && self.model.pip_window() == Some(id)
            && self.pip_user_closed()
        {
            return;
        }
        // A tab is never opened or loaded for a link another app handles: the user asked for
        // it (typed, a bookmark, a menu row), so it goes to the app as if clicked.
        if let Some((opener, url)) = external_target(&command)
            && let Some(tab) = opener.or_else(|| self.shown())
        {
            let url = url.to_owned();
            self.external_link(tab, url, Some(true));
            return;
        }
        if persist::changes_session(&command) {
            self.save = self.save.changed();
        }
        if let Command::CloseTab(tab) = command {
            self.remember_closed(tab);
        }
        if matches!(
            command,
            Command::ActivateTab(_)
                | Command::CloseTab(_)
                | Command::SwitchSpace(_)
                | Command::MoveTab { .. }
        ) {
            self.hover_dismiss();
        }
        let mut queue = VecDeque::from([command]);
        while let Some(command) = queue.pop_front() {
            // `apply` takes the model by value; a fresh one holds its place meanwhile.
            let placeholder = Model::new(Instant::now());
            let current = std::mem::replace(&mut self.model, placeholder);
            let (model, effects) = model::apply(current, command);
            self.model = model;
            self.pending.extend(effects);
            queue.extend(self.flush());
        }
        // Surfaces outlive a sleeping tab's view, not the tab.
        let tabs = &self.model.tabs;
        self.surfaces.retain(|tab, _| tabs.contains_key(tab));
        self.held_popups.retain(|tab, _| tabs.contains_key(tab));
        self.preview.forget_except(|tab| tabs.contains_key(tab));
        self.after_change();
        if persist::should_save(self.save) {
            self.save_now();
        }
    }

    /// A row dragged to slot `to` of the sidebar's flat tab list.
    pub fn move_tab(&mut self, tab: TabId, to: usize) {
        if let Some(to) = crate::shell::move_dest(&self.model, tab, to) {
            self.dispatch(Command::MoveTab { tab, to });
        }
    }

    /// Executes queued effects once views can be opened. Returns commands
    /// that must follow (a tab whose view could not be opened is closed).
    fn flush(&mut self) -> Vec<Command> {
        let Some(frame_rate) = self.frame_rate else {
            return vec![];
        };
        let mut follow = vec![];
        while let Some(effect) = self.pending.pop_front() {
            if let Effect::Tidied { space, before } = &effect {
                self.tidied(*space, before.clone());
            }
            let c = call(&effect, &self.model);
            follow.extend(self.execute(c, frame_rate));
        }
        follow
    }

    /// The only place effects touch the engine or open the bar.
    fn execute(&mut self, c: Call, frame_rate: u32) -> Option<Command> {
        match c {
            Call::OpenView { tab, url } => {
                // sized for the window that shows it
                if let Some(id) = self.model.window_of(tab) {
                    self.enter(id);
                }
                let window = self.w.window.upgrade()?;
                if self.views.contains_key(&tab) {
                    return None;
                }
                let config = ViewConfig {
                    url: url.clone(),
                    layout: self.pane_layout(&window, tab),
                    frame_rate,
                    begin_frames: self.pace.is_some(),
                    gpu: self.w.presenters[0].context().clone(),
                    // The tab's own cookies, storage and cache: its container's.
                    container: self
                        .model
                        .tabs
                        .get(&tab)
                        .map_or(ContainerId::DEFAULT, |t| t.container),
                };
                match self.engine.open_view(config) {
                    Ok(id) => {
                        measure::mark(measure::Stage::FirstView);
                        self.views.insert(tab, id);
                        // Page drags run inside the window (`page_drag`).
                        if let Some(view) = self.engine.view(id) {
                            view.set_drag_source(true);
                        }
                        self.builtins_view_opened(tab, &url);
                        self.ax_view_opened(tab);
                        self.pace(PaceEvent::Wake);
                    }
                    Err(e) => {
                        eprintln!("wubble: tab {}: {e}; closing it", tab.0);
                        return Some(Command::CloseTab(tab));
                    }
                }
            }
            Call::CloseView(tab) => {
                if self.views.contains_key(&tab) {
                    self.downloads_closing(tab);
                }
                if let Some(id) = self.views.remove(&tab) {
                    self.engine.close_view(id);
                    self.view_gone(tab);
                }
            }
            // A live tab without a view (it showed an internal page) gets one for the page.
            Call::Load { tab, url } => {
                let live = self
                    .model
                    .tabs
                    .get(&tab)
                    .is_some_and(|t| t.lifecycle.liveness == Liveness::Active);
                if self.views.contains_key(&tab) {
                    self.builtins_loading(tab, &url);
                }
                match self.tab_view(tab) {
                    Some(view) => view.load(&url),
                    None if live => return self.execute(Call::OpenView { tab, url }, frame_rate),
                    None => {}
                }
            }
            Call::SetVisible { tab, visible } => {
                if let Some(view) = self.tab_view(tab) {
                    view.set_visible(visible);
                }
                self.pace(PaceEvent::Wake);
            }
            Call::SetFrozen { tab, frozen } => {
                if let Some(view) = self.tab_view(tab) {
                    view.set_frozen(frozen);
                }
            }
            Call::CaptureSnapshot(tab) => self.capture_snapshot(tab),
            Call::ReleaseSnapshot(tab) => self.snapshots.release(tab),
            // Nothing left to show (a new window on an empty Space, its last tab closed): the
            // window showing the active Space opens the start page (TODO.md Decided "Start
            // page"). Later, outside the host: it is a command of its own.
            Call::ShowCommandBar => {
                let id = self.model.main_window();
                let _ = slint::invoke_from_event_loop(move || {
                    crate::with_host_in(id, |host| {
                        host.dispatch(new_tab(InternalPage::Start.url().into(), None));
                    });
                });
            }
            Call::Quit => quit(),
            Call::ForgetContainer(c) => self.engine.forget_container(c),
            Call::OpenWindow(id) => self.open_window(id),
            Call::CloseWindow(id) => self.close_window(id),
            Call::FocusWindow(id) => self.raise_window(id),
            Call::Nothing => {}
        }
        None
    }

    /// Re-derives the shell from the model.
    fn after_change(&mut self) {
        for id in self.window_ids() {
            if self.enter(id) {
                self.render_window();
            }
        }
        self.enter_focused();
        // once per change, not per window: a tab switch is picture-in-picture's cue
        self.media_after_change();
    }

    /// The open windows, the one the host is in first.
    fn window_ids(&self) -> Vec<WindowId> {
        std::iter::once(self.w.id)
            .chain(self.parked.keys().copied())
            .collect()
    }

    /// Re-derives the current window's shell from the model.
    fn render_window(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        self.dock_follows_tab();
        // Another tab's frame (and cursor) must not stay on screen while
        // input already goes to the tabs shown now: blank until they paint.
        let kept = self.framed_tabs();
        let before = self.w.frames.len();
        self.w.frames.retain(|tab, _| kept.contains(tab));
        window::free_slots(&mut self.w.slots, &kept);
        if self.w.frames.len() != before {
            window.set_web_cursor(Cursor::Default as i32);
            for tab in self.shown_tabs() {
                if let Some(view) = self.tab_view(tab) {
                    view.reset_cursor();
                }
            }
        }
        self.stand_in_snapshots(&window);
        self.settle_panes(&window);
        // one keyboard focus: the focused window's page
        if self.w.id == self.model.focused() {
            self.sync_focus(&window);
        }
        let tabs = &self.model.tabs;
        self.favicons.retain(|tab, _| tabs.contains_key(tab));
        let blocked = self.blocked_counts();
        crate::bind::render(
            &window,
            &crate::shell::view_model(&self.model, self.w.id, &self.audio, &blocked),
            &self.favicons,
        );
        let held = self
            .shown()
            .and_then(|t| self.held_popups.get(&t))
            .map_or(0, Vec::len);
        window.set_address_popups_blocked(i32::try_from(held).unwrap_or(i32::MAX));
        self.render_surfaces(&window);
        self.render_builtins(&window);
        self.render_video(&window);
        self.render_pip(&window);
        self.render_ax(&window);
        self.render_boost(&window);
        let nav = self.view().map(|view| {
            (
                view.nav().page.clone(),
                matches!(view.nav().phase, Phase::Loading { .. }),
            )
        });
        crate::bind::render_history(
            &window,
            nav.as_ref().map(|n| &n.0),
            nav.as_ref().is_some_and(|n| n.1),
        );
        self.render_pages();
    }

    /// The blocker's count for `tab`, if it has a view.
    fn blocked_of(&mut self, tab: TabId) -> Option<u32> {
        let id = *self.views.get(&tab)?;
        let browser = self.engine.view(id)?.browser_id();
        Some(self.blocking.blocked(browser))
    }

    /// The blocker's count for `tab` and its breakdown by host, if it has a view.
    fn blocked_breakdown(&mut self, tab: TabId) -> Option<(u32, Vec<(String, u32)>)> {
        let id = *self.views.get(&tab)?;
        let browser = self.engine.view(id)?.browser_id();
        Some((
            self.blocking.blocked(browser),
            self.blocking.blocked_hosts(browser),
        ))
    }

    /// Drops the blocker's counts of views that are gone.
    pub(super) fn forget_gone_blocks(&mut self) {
        let ids: Vec<_> = self.views.values().copied().collect();
        let live: Vec<i32> = ids
            .into_iter()
            .filter_map(|id| Some(self.engine.view(id)?.browser_id()))
            .collect();
        self.blocking.keep_views(&live);
    }

    /// The blocker's count for each tab with a view.
    fn blocked_counts(&mut self) -> crate::shell::BlockedMap {
        let tabs: Vec<_> = self.views.keys().copied().collect();
        tabs.into_iter()
            .filter_map(|tab| Some((tab, self.blocked_of(tab)?)))
            .collect()
    }

    /// Shows `icon` on `tab`'s row if it is still its page's icon (a late image of the previous
    /// page is dropped).
    pub(super) fn show_favicon(&mut self, tab: TabId, icon: wubble_engine::favicon::FaviconImage) {
        if self
            .tab_view(tab)
            .is_none_or(|view| view.nav().page.favicon != icon.url)
        {
            return;
        }
        if let Some(url) = self.tab_view(tab).map(|view| view.nav().page.url.clone()) {
            self.store.put_favicon(&url, &icon, now_ms());
        }
        self.favicons.insert(
            tab,
            crate::bind::favicon_image(icon.width, icon.height, &icon.rgba),
        );
        self.after_change();
    }

    fn tab_view(&mut self, tab: TabId) -> Option<&mut View> {
        if tab == PANEL_TAB {
            return self.panel_view();
        }
        let id = *self.views.get(&tab)?;
        self.engine.view(id)
    }

    /// Tells the model whether the tab's page is loading and whether it holds
    /// a download the engine runs (which a discard would end).
    fn report_busy(&mut self, tab: TabId) {
        let loading = self
            .tab_view(tab)
            .is_some_and(|v| matches!(v.nav().phase, Phase::Loading { .. }));
        let download = self.downloads.holds(tab);
        self.set_busy(tab, Reason::Loading, loading);
        self.set_busy(tab, Reason::Download, download);
    }

    /// The page of the window the host is in that gets keys: in the focused window, the
    /// model's visible tab.
    pub(crate) fn shown(&self) -> Option<TabId> {
        self.model.window_tab(self.w.id)
    }

    /// The visible tab's view: the focused page, the one that gets keys.
    pub(crate) fn view(&mut self) -> Option<&mut View> {
        self.tab_view(self.shown()?)
    }

    /// The page that gets keys: the window's peek after a press in it, else the shown page.
    pub(super) fn key_page(&self) -> Option<TabId> {
        input::key_page(
            self.key_pane,
            self.model.glance_of(self.w.id).map(|g| g.tab),
            self.model.window_key_tab(self.w.id),
        )
    }

    /// The view that gets keys: the DevTools panel's after a press in it, else the key page's.
    pub(super) fn focused_view(&mut self) -> Option<&mut View> {
        if self.keys_to_panel().is_some() {
            return self.panel_view();
        }
        self.tab_view(self.key_page()?)
    }

    /// The visible tab's view, else any live tab's: for work that needs a view but none in
    /// particular (a retried download started from an internal page).
    fn any_view(&mut self) -> Option<&mut View> {
        let tab = self
            .shown()
            .filter(|t| self.views.contains_key(t))
            .or_else(|| self.views.keys().next().copied())?;
        self.tab_view(tab)
    }

    fn tab_of(&self, view: ViewId) -> Option<TabId> {
        if self.devtools.view == Some(view) {
            return Some(PANEL_TAB);
        }
        self.views
            .iter()
            .find_map(|(&tab, &id)| (id == view).then_some(tab))
    }

    pub fn pump(&mut self, wake: Wake) {
        let outcome = match wake {
            Wake::Schedule(delay_ms) => self.engine.schedule_pump(delay_ms),
            Wake::TimerFired => self.engine.pump_timer_fired(),
        };
        let PumpOutcome {
            timer,
            worked,
            events,
        } = outcome;
        match timer {
            Some(PumpTimer::Start(delay)) => {
                self.timer.start(slint::TimerMode::SingleShot, delay, || {
                    deliver(Wake::TimerFired)
                });
            }
            Some(PumpTimer::Cancel) => self.timer.stop(),
            None => {}
        }
        if !worked {
            return;
        }
        measure::count(measure::Count::PumpWorked);
        for (view, event) in events {
            if let Some(window) = self.enter_view(view) {
                self.on_web_event(&window, view, event);
            }
        }
        for (view, event) in self.engine.take_page_events() {
            if let Some(window) = self.enter_view(view) {
                self.on_page_event(&window, view, event);
            }
        }
        self.media_pump();
        self.enter_focused();
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        self.close_tabs_of_gone_views();
        self.smoke_step(SmokeInput::Worked);
        self.tabs_smoke_step();
        self.windows_smoke_step();
        self.session_smoke_step();
        self.settings_smoke_step();
        self.pages_smoke_step();
        self.start_smoke_step();
        if self.frame_rate.is_some() {
            self.gallery_step(crate::gallery_smoke::Event::Ready);
            self.overlay_ready();
        }
        let presented = self.stats.accelerated;
        if let Err(e) = self.show_frame(&window) {
            self.fail(e);
        }
        let presented = self.stats.accelerated != presented;
        self.split_smoke_step(&window);
        self.autofill_smoke_worked(presented);
        self.builtin_smoke_worked();
        self.video_smoke_worked();
        self.ax_smoke_worked(&window);
        let framed = self.shown().is_some_and(|t| self.w.frames.contains_key(&t));
        let popup_framed = window.get_popup_frame().size().width > 0;
        if self.checks.block_smoke.is_some() {
            let blocked = self.visible_blocked();
            self.block_smoke_step(block_smoke::Input::Worked { blocked });
        }
        if self.checks.page_smoke.is_some() {
            let view_width = self.shown().map_or(0.0, |t| {
                self.pane_layout(&window, t).size.width as f32 / window.window().scale_factor()
            });
            self.page_smoke_step(page_smoke::Input::Worked {
                framed,
                popup_framed,
                view_width,
                panel: self.dock_seen(),
            });
        }
        self.parity_step(crate::parity::Input::Worked);
        if self.checks.quit_after.is_some() {
            self.idle_runs += 1;
            if self.idle_runs > SMOKE_MAX_IDLE_RUNS {
                self.fail(format!(
                    "no new frame after {SMOKE_MAX_IDLE_RUNS} runs of CEF work ({} accelerated so far)",
                    self.stats.accelerated
                ));
            }
        }
    }

    /// A page that closed itself (`window.close()`) takes its tab with it.
    fn close_tabs_of_gone_views(&mut self) {
        let gone: Vec<TabId> = self
            .views
            .iter()
            .filter(|&(_, &id)| !self.engine.has_view(id))
            .map(|(&tab, _)| tab)
            .collect();
        for tab in gone {
            self.views.remove(&tab);
            self.view_gone(tab);
            self.dispatch(Command::CloseTab(tab));
        }
        // the frontend closed itself
        if self
            .devtools
            .view
            .is_some_and(|id| !self.engine.has_view(id))
        {
            self.devtools.view = None;
            self.panel_closed();
        }
    }

    /// Presents the new frames of every window's panes; whether any window got one. Ends in
    /// the focused window.
    fn present_windows(&mut self) -> Result<bool, String> {
        let mut presented = false;
        for id in self.window_ids() {
            if self.enter(id)
                && let Some(window) = self.w.window.upgrade()
            {
                presented |= self.present_panes(&window)?;
            }
        }
        self.enter_focused();
        Ok(presented)
    }

    /// Enters the window showing `view`'s tab (the focused one for a tab no window shows) and
    /// returns it.
    fn enter_view(&mut self, view: ViewId) -> Option<AppWindow> {
        let window = self
            .tab_of(view)
            .and_then(|tab| self.model.window_of(tab))
            .unwrap_or_else(|| self.model.focused());
        if !self.enter(window) {
            self.enter_focused();
        }
        self.w.window.upgrade()
    }

    fn show_frame(&mut self, window: &AppWindow) -> Result<(), String> {
        if self.frame_rate.is_none() {
            // Before winit has created the window there is no monitor to
            // ask; CEF's next pump (at most `MAX_TIMER_DELAY` away) retries.
            let Some(refresh) = window.window().with_winit_window(display_refresh_mhz) else {
                return Ok(());
            };
            let frame_rate = pacing::frame_rate_for(refresh);
            eprintln!("wubble: CEF frame rate {frame_rate} (display {refresh:?} mHz)");
            self.frame_rate = Some(frame_rate);
            let follow = self.flush();
            for command in follow {
                self.dispatch(command);
            }
            self.after_change();
        }
        let new_cpu = self
            .stats
            .record_cpu_fallback(self.engine.cpu_fallback_frames());
        if new_cpu > 0 {
            eprintln!("wubble: {new_cpu} CPU fallback frame(s) (OnPaint), not shown");
        }
        if !self.present_windows()? {
            return Ok(());
        }
        let now = Instant::now();
        self.stats
            .record_accelerated(self.last_present.map(|last| now - last));
        self.last_present = Some(now);
        self.idle_runs = 0;
        // Once: a frame the loop shows before the quit takes effect (on macOS the pump runs
        // outside winit's handler, so one more can arrive) is not checked again.
        if self
            .checks
            .quit_after
            .is_some_and(|n| self.stats.accelerated == n)
        {
            if let Some(tab) = self.shown()
                && let Some(frame) = self.w.frames.get(&tab)
            {
                let layout = self.pane_layout(window, tab);
                eprintln!(
                    "smoke: content {}x{} @{} frame {}x{} clip {}x{}",
                    layout.size.width,
                    layout.size.height,
                    layout.scale,
                    frame.view.shown().w,
                    frame.view.shown().h,
                    frame.clip.width,
                    frame.clip.height
                );
            }
            if self.checks.check_pixels {
                let shot = snapshot(window)?;
                let sample = self.check_pixels(window, &shot)?;
                println!(
                    "smoke: pixels ok distinct={} mean_luma={}",
                    sample.distinct, sample.mean_luma
                );
                self.check_glass(window, &shot)?;
            }
            quit();
        }
        Ok(())
    }

    /// `--smoke frames`: with the scene, the sidebar's glass must show something (the frosted
    /// backdrop), not a flat fill, and be the scene's current frame. The
    /// smoke pins the glass at `Full` (`Host::new`), so the verdict never depends on the
    /// quality governor; the line says the level it checked.
    fn check_glass(&self, window: &AppWindow, shot: &Snapshot) -> Result<(), String> {
        if !self.w.glass.borrow().is_scene() {
            println!("smoke: glass fallback");
            return Ok(());
        }
        let key = window.global::<GlassKeys>().get_sidebar();
        let side = self
            .w
            .panels
            .borrow()
            .get(key)
            .cloned()
            .ok_or("the sidebar's glass never reached the scene")?;
        // The window's glass under the sidebar, left of the page card: the band below the tab
        // list and above the Space switcher, glass and no text.
        let scale = window.window().scale_factor();
        let x = side.x + side.radius;
        let w = window.get_content_x() - side.radius - x;
        let (y, h) = (side.y + side.height * 0.45, side.height * 0.25);
        let region = pixels::Region {
            x: physical(x, scale),
            y: physical(y, scale),
            width: physical(w, scale),
            height: physical(h, scale),
        };
        let sample = pixels::check_varied(shot, region)?;
        // What the window shows there is the scene's frame of this render, not an older one.
        let (sw, sh, scene) = self
            .w
            .glass
            .borrow()
            .readback()
            .ok_or("the scene has no frame")?;
        let diff = pixels::check_shows_scene(shot, (sw, sh, &scene), region)?;
        println!(
            "smoke: glass ok quality={} distinct={} diff={diff}",
            self.w
                .glass
                .borrow()
                .quality()
                .map_or("n/a".into(), |q| format!("{q:?}")),
            sample.distinct
        );
        Ok(())
    }

    fn on_web_event(&mut self, window: &AppWindow, view: ViewId, event: WebEvent) {
        // Events of a view whose tab is already closed are dropped.
        let Some(tab) = self.tab_of(view) else {
            return;
        };
        if tab == PANEL_TAB {
            return self.on_panel_event(event);
        }
        crate::load_run::web_event(&event);
        self.smoke_step(SmokeInput::Event(&event));
        self.page_smoke_step(page_smoke::Input::Web(&event));
        self.block_smoke_step(block_smoke::Input::Web(&event));
        if let WebEvent::TitleChanged(title) = &event {
            self.ax_smoke_step(crate::ax_smoke::Input::Title(title));
        }
        self.parity_web(&event);
        self.builtins_web(tab, &event);
        self.builtin_smoke_step(crate::builtin_smoke::Input::Web(&event));
        if self.checks.video_smoke.is_some() {
            self.video_smoke_step(crate::video_smoke::Input::Web(&event));
        }
        match event {
            WebEvent::CursorChanged(cursor) => {
                // A page beside the focused one shows its cursor too: the pointer is over it.
                if !self.shown_tabs().contains(&tab) {
                    return;
                }
                window.set_web_cursor(cursor as i32);
                self.autoscroll_input(window, wubble_core::autoscroll::Input::Cursor(cursor.pan()));
                // Slint applies a TouchArea's cursor only while it handles a
                // mouse event (i-slint-core 1.19 `window.rs`, after
                // `process_mouse_input`), and CEF reports the shape after
                // the move that caused it. Replay the pointer where it is,
                // once the host is free again, so the shape shows without
                // waiting for the mouse to move.
                if let Some(position) = self.input.pointer {
                    let _ = self.w.window.upgrade_in_event_loop(move |window| {
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::PointerMoved { position })
                    });
                }
            }
            WebEvent::UrlChanged(url) => {
                self.held_popups.remove(&tab);
                self.media_navigated(tab, &url);
                self.autofill_navigated(tab, &url);
                self.apply_error(tab, ErrorEvent::Committed(url.clone()));
                self.record_visit(tab, &url);
                self.restore_zoom(tab, &url);
                self.dispatch(Command::UrlChanged { tab, url });
            }
            WebEvent::TitleChanged(title) => {
                self.autofill_smoke_step(crate::autofill_smoke::Input::Title(&title));
                if let Some(url) = self.committed.get(&tab)
                    && wubble_store::is_recordable(url)
                {
                    self.store.update_title(url, &title);
                }
                self.dispatch(Command::TitleChanged { tab, title });
            }
            WebEvent::PopupRequested(request) => self.popup_requested(tab, request),
            WebEvent::LoadStarted => self.apply_error(tab, ErrorEvent::LoadStarted),
            WebEvent::LoadingChanged { loading, .. } => {
                if !loading {
                    self.typed_nav.ended(tab);
                }
                self.report_busy(tab);
            }
            WebEvent::RendererGone => {
                self.apply_error(tab, ErrorEvent::Crashed);
                self.busy_view_gone(tab);
                self.dispatch(Command::RendererGone(tab));
            }
            WebEvent::KeyUnhandled(key) => {
                if Some(tab) == self.shown()
                    && let Some(action) = self.page_first.unhandled(key)
                {
                    self.perform(action);
                }
            }
            // A page without an icon shows the neutral glyph; a new icon replaces the old one
            // once it has downloaded (`show_favicon`).
            WebEvent::FaviconChanged(url) => {
                if url.is_empty() && self.favicons.remove(&tab).is_some() {
                    self.after_change();
                }
            }
        }
    }

    // ----- pop-ups

    /// A page's `window.open` or `target=_blank`: what the policy table says (a tab, a Popup
    /// window, or held back for the address pill).
    fn popup_requested(&mut self, tab: TabId, request: PopupRequest) {
        use wubble_core::popup::{self, Outcome};
        // A `target=_blank` mailto: link opens the app and no tab, as in Chromium.
        if wubble_engine::page::is_external(&request.url) {
            self.external_link(tab, request.url, Some(request.gesture));
            return;
        }
        let site = self.site_of(tab);
        let remembered = site
            .as_deref()
            .and_then(|o| self.site_permissions.get(o, Permission::Popups));
        let glance = self.glance_enabled;
        match popup::decide(request.opened, request.gesture, remembered, glance) {
            Outcome::Tab { foreground: true } => self.dispatch(new_tab(request.url, Some(tab))),
            Outcome::Glance => match self.model.window_of(tab) {
                Some(window) => self.dispatch(Command::OpenGlance {
                    window,
                    opener: tab,
                    url: request.url,
                }),
                None => self.dispatch(new_tab(request.url, Some(tab))),
            },
            Outcome::Tab { foreground: false } => self.dispatch(Command::BackgroundTab {
                url: request.url,
                opener: tab,
            }),
            Outcome::PopupWindow => self.dispatch(Command::OpenPopupWindow {
                url: request.url,
                opener: tab,
                size: request.size,
            }),
            Outcome::Blocked => {
                let held = self.held_popups.entry(tab).or_default();
                if held.len() < MAX_HELD_POPUPS {
                    held.push(request);
                }
                self.after_change();
            }
        }
    }

    /// Opens what `tab`'s page asked for and the policy held back, as if it had been clicked.
    fn open_held_popups(&mut self, tab: TabId) {
        for request in self.held_popups.remove(&tab).unwrap_or_default() {
            self.popup_requested(
                tab,
                PopupRequest {
                    gesture: true,
                    ..request
                },
            );
        }
        self.after_change();
    }

    /// The origin of `tab`'s page, for remembered site choices.
    fn site_of(&self, tab: TabId) -> Option<String> {
        let url = self.model.tabs.get(&tab)?.url.as_str();
        wubble_core::site::origin_of(url)
    }

    // ----- browsing data

    /// `Action::ClearBrowsingData`: what can be cleared for `tab`, as the shell's menu near the
    /// top of the window. Choosing a row is the confirmation.
    pub(super) fn clear_data_menu(&mut self, tab: TabId) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let origin = self.site_of(tab);
        let entries = crate::menu::clear_entries(&self.model, tab, origin.as_deref());
        let size = window.window().size();
        let at = ((size.width / 2) as i32, (size.height / 5) as i32);
        self.apply_menu(tab, crate::menu::MenuEvent::ShellRequested { at, entries });
        self.render_surfaces(&window);
    }

    /// Deletes what the clear menu's row named, then says so.
    pub(super) fn clear_data(&mut self, tab: TabId, what: crate::menu::ClearData) {
        use crate::menu::ClearData;
        let done = match what {
            ClearData::Containers(containers) => {
                match self.engine.clear_browsing_data(&containers) {
                    Ok(()) => Some("Cookies and cached files deleted"),
                    Err(e) => {
                        eprintln!("wubble: clear browsing data: {e}");
                        None
                    }
                }
            }
            ClearData::Site(origin) => self
                .tab_view(tab)
                .and_then(|v| v.forget_site(&origin))
                .map(|_| "Site data deleted"),
        };
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, done.unwrap_or("Nothing was deleted"));
        }
    }

    /// A later start on this profile: its URL in a Little window, or the browser brought
    /// forward.
    pub(crate) fn open_external(&mut self, message: crate::instance::Message) {
        match message {
            crate::instance::Message::Open(url) => {
                let space =
                    wubble_core::routing::resolve(&self.model, &self.routing_rules, &url).space;
                self.dispatch(Command::OpenLittle { url, space })
            }
            crate::instance::Message::Focus => self.raise_window(self.model.focused()),
        }
    }

    /// The pill's pop-up badge, at the badge.
    pub(crate) fn popups_menu_requested(&mut self, x: f32, y: f32) {
        let (Some(window), Some(tab)) = (self.w.window.upgrade(), self.shown()) else {
            return;
        };
        let count = self.held_popups.get(&tab).map_or(0, Vec::len);
        if count == 0 {
            return;
        }
        let scale = window.window().scale_factor();
        let at = ((x * scale) as i32, (y * scale) as i32);
        let entries = crate::menu::popup_entries(count, self.site_of(tab).as_deref());
        self.apply_menu(tab, crate::menu::MenuEvent::ShellRequested { at, entries });
        self.render_surfaces(&window);
    }

    // ----- command bar

    /// Ctrl+T or the new-tab row: the bar is opening on an empty query.
    pub fn bar_for_new_tab(&mut self, window: &AppWindow) {
        self.open_bar(window, BarMode::NewTab, String::new());
    }

    /// Ctrl+L or the address pill: the bar opens on the visible tab's URL.
    pub fn bar_for_address(&mut self, window: &AppWindow) {
        // a Little or Popup window's address is read-only
        if self.w.kind != wubble_core::window::WindowKind::Main {
            return;
        }
        let visible = self.shown();
        match visible.and_then(|t| self.model.tabs.get(&t)) {
            Some(tab) => {
                let url = tab.url.clone();
                self.open_bar(window, BarMode::Navigate(tab.id), url);
            }
            None => self.bar_for_new_tab(window),
        }
    }

    pub(super) fn open_bar(&mut self, window: &AppWindow, mode: BarMode, query: String) {
        self.bar.mode = mode;
        self.suggest_event(wubble_core::suggest::FetchEvent::Closed);
        window.set_command_query(query.as_str().into());
        self.bar_query(window, &query);
    }

    /// Renders the rows for `query` at once from what is at hand, and asks
    /// the store for its history hits, which re-render when they arrive.
    pub fn bar_query(&mut self, window: &AppWindow, query: &str) {
        self.bar.query = query.into();
        self.bar.generation += 1;
        self.suggest_event(wubble_core::suggest::FetchEvent::Typed(query.into()));
        self.render_bar(window);
        if query.trim().is_empty() {
            return;
        }
        let generation = self.bar.generation;
        self.store
            .query_history(query, now_ms(), bar::HISTORY_HITS, move |hits| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.history_arrived(generation, hits));
                });
            });
    }

    fn history_arrived(&mut self, generation: u64, hits: persist::Hits) {
        if generation != self.bar.generation {
            return;
        }
        self.bar.history = hits.entries;
        self.bar.icons = hits.icons;
        self.bar.bookmarks = hits.bookmarks;
        if let Some(window) = self.w.window.upgrade() {
            self.render_bar(&window);
        }
    }

    /// One event of the live suggestion fetch; re-renders the rows when its list changed.
    pub fn suggest_event(&mut self, event: wubble_core::suggest::FetchEvent) {
        let settings = wubble_core::suggest::SuggestSettings {
            engine: self.search_engine.unwrap_or(SearchEngine::DuckDuckGo),
            live: if self.live_suggestions && self.search_engine.is_some() {
                wubble_core::suggest::LiveSuggestions::On
            } else {
                wubble_core::suggest::LiveSuggestions::Off
            },
        };
        if let Some(list) = self.suggest.event(event, &settings)
            && list != self.bar.suggestions
        {
            self.bar.suggestions = list;
            if let Some(window) = self.w.window.upgrade() {
                self.render_bar(&window);
            }
        }
    }

    fn render_bar(&mut self, window: &AppWindow) {
        let tabs = bar::open_tabs(&self.model);
        let inputs = BarInputs {
            tabs: &tabs,
            history: &self.bar.history,
            bookmarks: &self.bar.bookmarks,
            engine: self.search_engine,
            keymap: &self.keymap,
            suggestions: &self.bar.suggestions,
            now_ms: now_ms(),
        };
        self.bar.rows = bar::bar_rows(&inputs, &self.bar.query);
        crate::bind::render_bar(window, &self.bar.rows, &self.bar.icons, &self.favicons);
    }

    /// The bar closed (Esc, a click outside, or a chosen row): nothing more may be fetched.
    pub fn bar_closed(&mut self) {
        self.suggest_event(wubble_core::suggest::FetchEvent::Closed);
    }

    pub fn bar_chosen(&mut self, index: i32) {
        let row = usize::try_from(index)
            .ok()
            .and_then(|i| self.bar.rows.get(i));
        if let Some(row) = row {
            let target = row.target.clone();
            self.choose(&target);
        }
        // after reading the row: clearing the suggestions re-renders the rows
        self.bar_closed();
    }

    fn choose(&mut self, target: &RowTarget) {
        match bar::target_command(target, self.bar.mode, self.search_engine) {
            Some(Choice::Model(command)) => {
                self.dispatch(command);
                if bar::is_typed(target) {
                    self.typed_nav.started(self.shown());
                }
            }
            Some(Choice::Perform(action)) => self.perform(action),
            Some(Choice::SetEngine(engine)) => {
                self.write_setting(wubble_core::settings::SEARCH_ENGINE.key, engine.id());
                // The text that offered the choice is searched with it.
                if let omnibox::Intent::Search(q) = omnibox::classify(&self.bar.query)
                    && !q.is_empty()
                {
                    self.choose(&RowTarget::Omni(Target::Search(q)));
                }
            }
            None => {}
        }
    }

    // ----- actions

    /// The one place an [`Action`] runs, from a key or the command bar.
    pub fn perform(&mut self, action: Action) {
        match perform::plan(action, &self.model) {
            Plan::Model(command) => self.dispatch(command),
            Plan::View(op) => {
                if let Some(tab) = self.shown() {
                    self.typed_nav.ended(tab);
                }
                let Some(tab) = self.shown() else {
                    return;
                };
                let zoom_default = self.zoom_default;
                let Some(view) = self.tab_view(tab) else {
                    return;
                };
                match op {
                    ViewOp::Reload => view.reload(),
                    ViewOp::HardReload => view.hard_reload(),
                    ViewOp::Stop => view.stop(),
                    ViewOp::Back => view.back(),
                    ViewOp::Forward => view.forward(),
                    ViewOp::ZoomIn => view.zoom_in(),
                    ViewOp::ZoomOut => view.zoom_out(),
                    // To the user's default, which CEF's own reset (always 100 %) ignores.
                    ViewOp::ZoomReset => view.set_zoom_percent(zoom_default),
                    ViewOp::ToggleMute => {
                        let muted = view.is_muted();
                        view.set_muted(!muted);
                    }
                    ViewOp::Edit(command) => view.edit(command),
                }
                if matches!(op, ViewOp::ZoomIn | ViewOp::ZoomOut | ViewOp::ZoomReset) {
                    let percent = view.zoom_percent();
                    self.user_zoomed(tab, percent);
                }
            }
            Plan::Print => {
                if let Some(tab) = self.shown() {
                    self.print(tab);
                }
            }
            Plan::Find(event) => self.find_event(event),
            Plan::Fullscreen => {
                if let Some(window) = self.w.window.upgrade() {
                    self.apply_screen(&window, ScreenEvent::Toggle);
                }
            }
            Plan::Copy(text) => self.copy(&text),
            Plan::BookmarkPage => self.bookmark_page(),
            Plan::DevTools => self.toggle_devtools(),
            Plan::DockNudge(dir) => self.source_nudge(f64::from(dir)),
            Plan::OpenBeside => self.open_beside(),
            Plan::Builtin(builtin) => self.builtin(builtin),
            // Later, outside the host: opening the bar calls back into it.
            Plan::FocusAddress => {
                let _ = self
                    .w
                    .window
                    .upgrade_in_event_loop(|window| window.invoke_request_address_focus());
            }
            // Later, outside the host, as FocusAddress: the address region opens the bar.
            Plan::CycleRegion(back) => {
                let _ = self.w.window.upgrade_in_event_loop(move |window| {
                    let at = Region::of(
                        window.get_sidebar_focused(),
                        window.get_command_bar_state() != CommandBarState::Closed,
                    );
                    let shown = window.get_sidebar_mode() != SidebarMode::Hidden
                        && !window.get_page_fullscreen();
                    window.invoke_focus_region(match next_region(at, back, shown) {
                        Region::Address => FocusRegion::Address,
                        Region::Sidebar => FocusRegion::Sidebar,
                        Region::Page => FocusRegion::Page,
                    });
                });
            }
            Plan::ReopenClosed => self.store.pop_closed(|closed| {
                if let Some(closed) = closed {
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| host.reopen_closed(closed));
                    });
                }
            }),
            Plan::ToggleSidebar => {
                if let Some(window) = self.w.window.upgrade() {
                    window.invoke_toggle_sidebar();
                }
            }
            Plan::UndoTidy => self.undo_tidy(),
            Plan::CloseTabs(tabs) => {
                for tab in tabs {
                    self.dispatch(Command::CloseTab(tab));
                }
            }
            Plan::Quit => quit(),
            Plan::Nothing => {}
            Plan::NotYet => eprintln!(
                "wubble: {} is not available yet: {}",
                action.id(),
                perform::not_yet(action).unwrap_or_default()
            ),
        }
    }

    /// A tab off the recently-closed list, in the active Space (its own may
    /// be gone), in its container if that still exists.
    fn reopen_closed(&mut self, closed: ClosedTab) {
        self.reopen(closed.url, None, Some(closed.container));
    }

    /// Opens `url` again, the one way a tab comes back: in `space` if it still exists (the view
    /// moves there), else the active one; in `container` if that still exists.
    pub(super) fn reopen(
        &mut self,
        url: String,
        space: Option<SpaceId>,
        container: Option<ContainerId>,
    ) {
        self.open_in(url, space, container, None);
    }

    fn open_in(
        &mut self,
        url: String,
        space: Option<SpaceId>,
        container: Option<ContainerId>,
        opener: Option<TabId>,
    ) {
        let container = container.filter(|c| self.model.containers.iter().any(|k| k.id == *c));
        let space = space
            .filter(|s| self.model.spaces.iter().any(|k| k.id == *s))
            .filter(|s| *s != self.model.active_space());
        if let Some(space) = space {
            self.dispatch(Command::SwitchSpace(space));
        }
        // a tab that opens in another Space is not that page's child
        let opener = opener.filter(|_| space.is_none());
        self.dispatch(Command::NewTab {
            url,
            container,
            opener,
        });
    }

    /// A link the user opened in a new tab (or one from the command line): where the routing
    /// rules say, else beside `opener` in the active Space. Typed addresses, pop-ups,
    /// background tabs and same-tab navigation are not routed.
    pub(super) fn open_routed(&mut self, url: String, opener: Option<TabId>) {
        let routed = wubble_core::routing::resolve(&self.model, &self.routing_rules, &url);
        self.open_in(url, routed.space, routed.container, opener);
    }

    // ----- tidy

    /// The tidy just ran: keep its undo and say what it did.
    fn tidied(&mut self, space: SpaceId, before: Vec<Item>) {
        let after = self
            .model
            .spaces
            .iter()
            .find(|s| s.id == space)
            .map(|s| s.items.as_slice())
            .unwrap_or_default();
        let (tabs, folders) = tidy_summary(&before, after);
        self.tidy_undo = Some((space, before));
        if let Some(window) = self.w.window.upgrade() {
            let text = format!(
                "Grouped {tabs} tab{} into {folders} folder{}. Undo from the command bar",
                if tabs == 1 { "" } else { "s" },
                if folders == 1 { "" } else { "s" },
            );
            self.toast(&window, &text);
        }
    }

    fn undo_tidy(&mut self) {
        let Some((space, items)) = self.tidy_undo.take() else {
            if let Some(window) = self.w.window.upgrade() {
                self.toast(&window, "Nothing to undo");
            }
            return;
        };
        self.dispatch(Command::SetLayout { space, items });
    }

    // ----- archive

    /// Files the tabs that went unvisited too long in the Archive and closes them, each once
    /// the store has it (a failed write keeps the tab).
    fn archive_tick(&mut self, now: i64) {
        let audible: HashSet<TabId> = self
            .model
            .tabs
            .values()
            .filter(|t| t.busy.contains(Reason::Audible))
            .map(|t| t.id)
            .collect();
        for tab in archive::due(&self.model, self.archive_after, now, &audible) {
            let Some(t) = self.model.tabs.get(&tab) else {
                continue;
            };
            if self.archiving.insert(tab, t.last_active_ms).is_some() {
                continue;
            }
            let entry = ArchivedTab {
                id: 0,
                url: t.url.clone(),
                title: t.title.clone(),
                container: t.container,
                space: self.model.location(tab).map(|(space, _)| space),
                last_active_ms: t.last_active_ms,
                archived_at_ms: now,
            };
            self.store.push_archive(entry, move |row| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.archived(tab, row));
                });
            });
        }
    }

    /// The store answered for `tab` with its archive row: close the tab if it is filed and
    /// still unvisited, else drop the row so the tab is not listed twice.
    fn archived(&mut self, tab: TabId, row: Option<i64>) {
        let Some(row) = row else {
            self.archiving.remove(&tab);
            return;
        };
        // unvisited since it was filed (its stamp is unchanged) and still closable
        let unvisited = self.archiving.get(&tab).is_some_and(|stamp| {
            self.model
                .tabs
                .get(&tab)
                .is_some_and(|t| t.last_active_ms == *stamp)
        }) && !self.model.visible_tabs().contains(&tab);
        if unvisited {
            self.dispatch(Command::CloseTab(tab));
        } else {
            self.store.delete_archive(row);
        }
        self.archiving.remove(&tab);
        if self.shown_page() == Some(InternalPage::Archive) {
            self.archive_refetch();
        }
    }

    // ----- persistence

    /// Called before a tab closes: it goes on the recently-closed list.
    fn remember_closed(&mut self, tab: TabId) {
        self.media_tab_closed(tab);
        self.committed.remove(&tab);
        self.typed_nav.ended(tab);
        if self.archiving.contains_key(&tab) {
            return; // filed in the Archive instead
        }
        let Some(t) = self.model.tabs.get(&tab) else {
            return;
        };
        if t.url.is_empty() {
            return;
        }
        self.store.push_closed(ClosedTab {
            url: t.url.clone(),
            title: t.title.clone(),
            container: t.container,
            space: self.model.location(tab).map(|(space, _)| space),
            closed_at_ms: now_ms(),
        });
    }

    /// The engine committed `url` in `tab`: a history visit unless it only
    /// moved within the page. Typed if the bar started it. The title follows
    /// with `TitleChanged`.
    fn record_visit(&mut self, tab: TabId, url: &str) {
        let previous = self.committed.insert(tab, url.to_string());
        let typed = self.typed_nav.committed(tab);
        if persist::visit_to_record(previous.as_deref(), url) {
            let transition = if typed {
                Transition::Typed
            } else {
                Transition::Link
            };
            self.store.record_visit(url, "", now_ms(), transition);
        }
    }

    fn save_now(&mut self) {
        self.record_geometry();
        let ui = self.ui_state();
        self.store.save(&self.model, ui);
        self.save = SaveState::default();
    }

    /// Orderly shutdown: the final save, then the store thread finishes and
    /// closes the database. `ui` was read before the window went away.
    pub fn shutdown(&mut self, ui: UiState) {
        self.media_shutdown();
        self.save_download_progress();
        self.store.save(&self.model, ui);
        std::mem::replace(&mut self.store, StoreClient::none()).shutdown();
    }

    /// The coarse lifecycle tick; also the save policy's idle clock.
    pub fn tick(&mut self, now: Instant) {
        self.follow_frame_rate();
        self.devtools_tick();
        let pressure = self.read_pressure();
        let wall_ms = now_ms();
        self.dispatch(Command::Tick {
            now,
            wall_ms,
            pressure,
        });
        self.archive_tick(wall_ms);
        self.save = self.save.ticked();
        if persist::should_save(self.save) {
            self.save_now();
        }
        self.save_download_progress();
        self.passwords_tick(now);
        self.gpu_check();
        self.look_tick();
        self.ax_tick();
    }

    // ----- smokes

    fn smoke_step(&mut self, input: SmokeInput) {
        let Some(state) = self.checks.nav_smoke.take() else {
            return;
        };
        let want = nav_smoke::title_of(NAV_PAGE).unwrap_or_default();
        let (state, action) = nav_smoke::next(state, input, want);
        self.checks.nav_smoke = Some(state);
        match action {
            Some(SmokeAction::LoadSecondPage) => {
                if let Some(tab) = self.shown() {
                    self.dispatch(Command::Navigate {
                        tab,
                        url: page_url(NAV_PAGE),
                    });
                }
            }
            Some(SmokeAction::Pass(title)) => {
                self.nav_title = Some(title);
                quit();
            }
            Some(SmokeAction::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn seen(&self, heard: &tabs_smoke::Heard) -> Seen {
        let order = self.model.order(self.model.active_space());
        let visible = self.shown();
        let shown = visible
            .and_then(|t| self.views.get(&t))
            .is_some_and(|&id| self.engine.is_visible(id));
        let painted = visible
            .is_some_and(|t| self.w.frames.contains_key(&t) && !self.snapshots.standing_in(t));
        Seen {
            titles: order
                .iter()
                .filter_map(|t| self.model.tabs.get(t))
                .map(|t| t.title.clone())
                .collect(),
            views: self.engine.view_count(),
            live_tabs: self.model.live_tabs(),
            visible_views: self.engine.visible_views(),
            shown_is_visible: shown,
            stale_frame: {
                let kept = self.framed_tabs();
                self.w.frames.keys().any(|t| !kept.contains(t))
            },
            painted,
            restored_from_snapshot: visible.is_some_and(|t| heard.replaced.contains(&t)),
            frozen: order.iter().map(|t| heard.frozen.get(t).copied()).collect(),
        }
    }

    /// Chromium answered a freeze or thaw of `tab`'s page.
    pub(super) fn frozen_answered(&mut self, tab: TabId, frozen: bool, ok: bool) {
        if !ok {
            eprintln!("wubble: tab {}: freeze {frozen} refused", tab.0);
        }
        if let Some(smoke) = self.checks.tabs_smoke.as_mut() {
            smoke.heard.frozen.insert(tab, (frozen, ok));
        }
    }

    /// What `--smoke windows` waits on, across every window.
    fn windows_seen(&mut self) -> crate::windows_smoke::Seen {
        // the tabs on screen with a page: a start page has no view and no frame
        let internal: Vec<TabId> = self
            .model
            .tabs
            .values()
            .filter(|t| InternalPage::from_url(&t.url).is_some())
            .map(|t| t.id)
            .collect();
        let web = |t: &TabId| !internal.contains(t);
        let shown = self.model.visible_tabs();
        let visible: Vec<TabId> = shown.iter().copied().filter(|t| web(t)).collect();
        let views_agree = self.model.tabs.keys().all(|t| {
            let shown = self
                .views
                .get(t)
                .is_some_and(|&id| self.engine.is_visible(id));
            shown == visible.contains(t)
        }) && self.engine.visible_views() == visible.len();
        self.record_geometry();
        let newest = self.model.windows.iter().map(|w| w.id).max();
        let (mut framed, mut bars, mut start_in_newest, mut ghosts) = (true, 0, false, 0);
        for id in self.window_ids() {
            if self.enter(id) {
                let tabs = self.model.window_tabs(id);
                framed &= tabs
                    .iter()
                    .filter(|t| web(t))
                    .all(|t| self.w.frames.contains_key(t));
                start_in_newest |=
                    Some(id) == newest && self.shown_page() == Some(InternalPage::Start);
                if let Some(window) = self.w.window.upgrade() {
                    let open = window.get_command_bar_state() != CommandBarState::Closed;
                    bars += usize::from(open);
                    ghosts += usize::from(!window.get_drag_ghost().is_empty());
                }
            }
        }
        self.enter_focused();
        crate::windows_smoke::Seen {
            titled: self
                .model
                .tabs
                .values()
                .filter(|t| !t.title.is_empty())
                .count(),
            model_windows: self.model.windows.len(),
            open_windows: 1 + self.parked.len(),
            views_agree,
            framed,
            shown: shown.len(),
            bars,
            start_in_newest,
            ghosts,
            geometry: self.model.windows.iter().all(|w| w.geometry.is_some()),
        }
    }

    fn windows_smoke_step(&mut self) {
        use crate::windows_smoke::{self, Action};
        let Some(state) = self.checks.windows_smoke.take() else {
            return;
        };
        let seen = self.windows_seen();
        let (state, action) = windows_smoke::next(state, &seen);
        self.checks.windows_smoke = Some(state);
        // the window the smoke opened: the newest
        let second = self.model.windows.iter().map(|w| w.id).max();
        match action {
            Some(Action::OpenPage) => self.dispatch(new_tab(page_url(SMOKE_PAGE), None)),
            Some(Action::NewWindow) => self.dispatch(Command::NewWindow { space: None }),
            Some(Action::MoveTab) => {
                let visible = self.model.visible_tabs();
                let hidden = self
                    .model
                    .tabs
                    .keys()
                    .copied()
                    .find(|t| !visible.contains(t));
                if let (Some(tab), Some(to)) = (hidden, second) {
                    self.smoke_drag(tab, to);
                }
            }
            Some(Action::CheckAndClose) => {
                for id in self.window_ids() {
                    if !self.enter(id) {
                        continue;
                    }
                    let Some(window) = self.w.window.upgrade() else {
                        continue;
                    };
                    let checked =
                        snapshot(&window).and_then(|shot| self.check_pixels(&window, &shot));
                    match checked {
                        Ok(sample) => println!(
                            "smoke: windows pixels ok window={} distinct={}",
                            id.0, sample.distinct
                        ),
                        Err(e) => return self.fail(format!("window {}: {e}", id.0)),
                    }
                }
                self.enter_focused();
                if let Some(second) = second {
                    self.dispatch(Command::CloseWindow(second));
                }
            }
            Some(Action::OpenLittle) => {
                self.open_external(crate::instance::Message::Open(page_url(SMOKE_PAGE)));
            }
            Some(Action::Promote) => {
                let little = self
                    .model
                    .windows
                    .iter()
                    .find(|w| w.kind() == wubble_core::window::WindowKind::Little)
                    .map(|w| w.id);
                if let Some(window) = little {
                    let space = self.model.active_space();
                    self.dispatch(Command::PromoteLittle { window, space });
                }
            }
            Some(Action::Pass) => {
                self.windows_passed = true;
                quit();
            }
            Some(Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn tabs_smoke_step(&mut self) {
        let Some(heard) = self.checks.tabs_smoke.as_ref().map(|s| &s.heard) else {
            return;
        };
        let seen = self.seen(heard);
        let Some(state) = self.checks.tabs_smoke.take() else {
            return;
        };
        let want =
            crate::TABS_SMOKE_PAGES.map(|page| nav_smoke::title_of(page).unwrap_or_default());
        let (state, action) = tabs_smoke::next(state, &seen, &want);
        self.checks.tabs_smoke = Some(state);
        let nth =
            |host: &Self, i: usize| host.model.order(host.model.active_space()).get(i).copied();
        match action {
            Some(tabs_smoke::Action::OpenPages) => {
                for page in [NAV_PAGE, TAB3_PAGE] {
                    self.dispatch(new_tab(page_url(page), None));
                }
            }
            Some(tabs_smoke::Action::Activate(i)) => {
                if let Some(tab) = nth(self, i) {
                    self.dispatch(Command::ActivateTab(tab));
                }
            }
            Some(tabs_smoke::Action::DiscardHidden) => {
                self.tick(Instant::now() + SMOKE_DISCARD_AFTER);
            }
            Some(tabs_smoke::Action::Close(i)) => {
                if let Some(tab) = nth(self, i) {
                    self.dispatch(Command::CloseTab(tab));
                }
            }
            Some(tabs_smoke::Action::OpenTimers) => {
                for _ in 0..2 {
                    self.dispatch(new_tab(page_url(TIMERS_PAGE), None));
                }
                if let Some(tab) = nth(self, 0) {
                    self.dispatch(Command::ActivateTab(tab));
                }
            }
            // Straight to the engine: a lifecycle tick would freeze the control tab too.
            Some(tabs_smoke::Action::Freeze(i, frozen)) => {
                if let Some(view) = nth(self, i).and_then(|tab| self.tab_view(tab)) {
                    view.set_frozen(frozen);
                }
            }
            Some(tabs_smoke::Action::Pass) => {
                self.tabs_passed = true;
                quit();
            }
            Some(tabs_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn split_smoke_step(&mut self, window: &AppWindow) {
        let Some(state) = self.checks.split_smoke.take() else {
            return;
        };
        let order = self.model.order(self.model.active_space());
        let seen = split_smoke::Seen {
            tabs: order.len(),
            panes: self
                .pane_rects(window)
                .into_iter()
                .map(|r| split_smoke::PaneSeen {
                    want: self.pane_layout(window, r.tab).frame_size(),
                    got: self.frame_size(r.tab),
                })
                .collect(),
            visible_views: self.engine.visible_views(),
            at_rest: self.w.panes_last.is_none()
                && self.w.split_commands.is_empty()
                && !self.w.panes.held()
                && self.w.frames.values().all(|f| f.view.is_live()),
            masked: self
                .w
                .frames
                .values()
                .filter(|f| f.masked.is_some())
                .count(),
        };
        let (state, action) = split_smoke::next(state, &seen);
        self.checks.split_smoke = Some(state);
        match action {
            Some(split_smoke::Action::OpenSecond) => {
                self.dispatch(new_tab(page_url(SMOKE_PAGE), None));
            }
            Some(split_smoke::Action::Split) => {
                if let Some(&first) = order.first() {
                    self.dispatch(Command::SplitWith(first));
                }
            }
            Some(split_smoke::Action::CheckPixels) => {
                let checked = snapshot(window).and_then(|shot| {
                    let panes = self.pane_rects(window).len();
                    self.check_pixels(window, &shot).map(|s| (panes, s))
                });
                match checked {
                    Ok((panes, sample)) => println!(
                        "smoke: split panes={panes} pixels ok distinct={} mean_luma={}",
                        sample.distinct, sample.mean_luma
                    ),
                    Err(e) => self.fail(e),
                }
            }
            // The seam's gestures through the host's entry points the shell's `split-grab`,
            // `split-drag` and `split-release` call (the Slint side, hit strip included, is
            // tests/shell.rs `split_seam_drag`). Synthetic pointer events would race the real
            // pointer: the host replays it on a cursor change.
            Some(pull @ (split_smoke::Action::Grab | split_smoke::Action::Bend)) => {
                let by = match pull {
                    split_smoke::Action::Grab => split_smoke::DRAG,
                    _ => split_smoke::BEND,
                };
                let height = window.get_content_height();
                match self.w.panes.seams(height.into()).first() {
                    Some(&(x, _, _)) => {
                        let (x, y) = (x as f32, height / 2.0);
                        self.split_grab(0, x, y);
                        self.split_drag(x - by, y);
                    }
                    None => self.fail("split: no seam to grab".into()),
                }
            }
            Some(split_smoke::Action::Release) => self.split_release(),
            Some(split_smoke::Action::Resized(left, right)) => {
                println!("smoke: split resized panes {left}+{right}");
            }
            Some(split_smoke::Action::Unsplit) => {
                if let Some(tab) = self.shown() {
                    self.dispatch(Command::Unsplit(tab));
                }
            }
            Some(split_smoke::Action::Pass) => {
                self.split_passed = true;
                quit();
            }
            Some(split_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn settings_smoke_step(&mut self) {
        let Some(state) = self.checks.settings_smoke.take() else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            self.checks.settings_smoke = Some(state);
            return;
        };
        let seen = settings_smoke::Seen {
            ready: self.frame_rate.is_some(),
            on_settings: self
                .shown()
                .and_then(|t| self.model.tabs.get(&t))
                .is_some_and(|t| t.url == InternalPage::Settings.url()),
            page_open: window.get_settings_open(),
            rows: slint::Model::row_count(&window.get_settings_rows()),
            views: self.engine.view_count(),
            stored: self.settings_read.clone(),
            blocking: self.blocking.is_enabled(),
        };
        let (state, action) = settings_smoke::next(state, &seen);
        self.checks.settings_smoke = Some(state);
        match action {
            Some(settings_smoke::Action::Open) => {
                if crate::shot::requested() {
                    let host = std::cell::RefCell::new(&mut *self);
                    crate::shot::seed(&mut |c| host.borrow_mut().dispatch(c), || {
                        host.borrow().model.clone()
                    });
                }
                self.perform(Action::OpenSettings)
            }
            Some(settings_smoke::Action::Flip { key, value }) => {
                // The toggle's own callback, as a click sends it: later, outside the host,
                // which it calls back into. The read is queued after the write it causes.
                let _ = self.w.window.upgrade_in_event_loop(move |window| {
                    window.invoke_setting_changed(key.into(), value.into());
                    with_host(|host| {
                        host.store.read_settings(move |rows| {
                            let value = rows.get(key).cloned().unwrap_or_default();
                            let _ = slint::invoke_from_event_loop(move || {
                                with_host(|host| {
                                    host.settings_read = Some(value);
                                    host.settings_smoke_step();
                                });
                            });
                        })
                    });
                });
            }
            Some(settings_smoke::Action::Pass) => {
                self.settings_passed = true;
                quit();
            }
            Some(settings_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn pages_smoke_step(&mut self) {
        use slint::Model;
        let Some(state) = self.checks.pages_smoke.take() else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            self.checks.pages_smoke = Some(state);
            return;
        };
        let page = self.shown_page();
        let rows = match page {
            Some(InternalPage::History) => window.get_history_lines().row_count(),
            Some(InternalPage::Bookmarks) => window.get_bookmark_lines().row_count(),
            Some(InternalPage::Downloads) => window.get_downloads().row_count(),
            Some(InternalPage::Passwords) => window.get_logins().row_count(),
            Some(InternalPage::Archive) => window.get_archive_lines().row_count(),
            Some(InternalPage::Tasks) => window.get_tasks().row_count(),
            Some(InternalPage::Settings | InternalPage::Start) | None => 0,
        };
        let seen = pages_smoke::Seen {
            ready: self.frame_rate.is_some(),
            page,
            rows,
            unlocked: window.get_passwords_view().state == wubble_ui::VaultState::Unlocked,
            views: self.engine.view_count(),
        };
        let (state, action) = pages_smoke::next(state, &seen);
        self.checks.pages_smoke = Some(state);
        match action {
            Some(pages_smoke::Action::Open(page)) => self.perform(match page {
                InternalPage::History => Action::OpenHistory,
                InternalPage::Bookmarks => Action::OpenBookmarks,
                InternalPage::Downloads => Action::OpenDownloads,
                InternalPage::Passwords => Action::OpenPasswords,
                InternalPage::Archive => Action::OpenArchive,
                InternalPage::Tasks => Action::OpenTasks,
                InternalPage::Settings => Action::OpenSettings,
                InternalPage::Start => Action::NewTab,
            }),
            // The page's own button callback, as a click sends it: later, outside the host.
            Some(pages_smoke::Action::Unlock) => {
                let _ = self
                    .w
                    .window
                    .upgrade_in_event_loop(|window| window.invoke_passwords_unlock());
            }
            Some(pages_smoke::Action::Pass) => {
                self.pages_passed = true;
                quit();
            }
            Some(pages_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn visible_blocked(&mut self) -> u32 {
        let tab = self.shown();
        tab.and_then(|t| self.blocked_of(t)).unwrap_or(0)
    }

    fn block_smoke_step(&mut self, input: block_smoke::Input) {
        let Some(state) = self.checks.block_smoke.take() else {
            return;
        };
        let (state, action) = block_smoke::next(state, input);
        self.checks.block_smoke = Some(state);
        match action {
            Some(block_smoke::Action::Pass(blocked)) => {
                self.blocked_seen = Some(blocked);
                quit();
            }
            Some(block_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    fn page_smoke_step(&mut self, input: page_smoke::Input) {
        let Some(state) = self.checks.page_smoke.take() else {
            return;
        };
        let (state, actions) = page_smoke::next(state, input);
        self.checks.page_smoke = Some(state);
        let click = |host: &mut Self, button, at| {
            let pane = host.model.visible_tab().map_or(-1, crate::shell::pane_id);
            host.pointer_moved(pane, at);
            host.pointer_button(pane, button, true, at);
            host.pointer_button(pane, button, false, at);
        };
        for action in actions {
            match action {
                page_smoke::Action::Click(at) => click(self, PointerEventButton::Left, at),
                page_smoke::Action::RightClick(at) => click(self, PointerEventButton::Right, at),
                page_smoke::Action::AcceptDialog => self.dialog_answered(true, "", ""),
                page_smoke::Action::DismissMenu => {
                    if self.menu_dismissed() != 1 {
                        self.fail("the context menu was not answered exactly once".into());
                    }
                }
                page_smoke::Action::CancelMenu(cancel) => {
                    let Some(view) = self.view() else { continue };
                    match cancel {
                        // The engine's own paths, as a window blur or a tab switch takes them.
                        page_smoke::Cancel::Blur => view.focus(false),
                        page_smoke::Cancel::Hide => view.set_visible(false),
                        page_smoke::Cancel::Press => {
                            click(self, PointerEventButton::Left, page_smoke::BLANK_AT)
                        }
                    }
                }
                page_smoke::Action::Restore(cancel) => match cancel {
                    page_smoke::Cancel::Blur => {
                        // Back to what the shell's focus says.
                        self.input.forget_view_focus();
                        if let Some(window) = self.w.window.upgrade() {
                            self.sync_focus(&window);
                        }
                    }
                    page_smoke::Cancel::Hide => {
                        if let Some(view) = self.view() {
                            view.set_visible(true);
                        }
                    }
                    page_smoke::Cancel::Press => {}
                },
                page_smoke::Action::MenuGone => {
                    let open = self
                        .shown()
                        .and_then(|t| self.surfaces.get(&t))
                        .is_some_and(|s| s.menu != crate::menu::Menu::Closed);
                    if open || self.menu_dismissed() != 0 {
                        self.fail(
                            "CEF closed the context menu but the shell still answered it".into(),
                        );
                    }
                }
                page_smoke::Action::Find(query) => {
                    self.perform(Action::Find);
                    self.find_event(FindEvent::Query(query.into()));
                }
                page_smoke::Action::ZoomIn => self.perform(Action::ZoomIn),
                page_smoke::Action::ToggleDevTools => self.perform(Action::DevTools),
                page_smoke::Action::Pass(percent) => {
                    self.page_zoom = Some(percent);
                    quit();
                }
                page_smoke::Action::Fail(reason) => self.fail(reason),
            }
        }
    }

    /// `--smoke session`: the first run files an icon and a visit for a page; the second asks
    /// the store for them through the history path.
    fn session_icon_setup(&mut self) {
        let Some(smoke) = self.checks.session_smoke else {
            return;
        };
        match smoke.phase {
            crate::cli::Phase::Write => {
                let at = now_ms();
                self.store.record_visit(
                    session_smoke::ICON_PAGE,
                    "icon smoke",
                    at,
                    Transition::Typed,
                );
                self.store
                    .put_favicon(session_smoke::ICON_PAGE, &session_smoke::smoke_icon(), at);
            }
            crate::cli::Phase::Read => {
                self.store
                    .query_history("favicon.wubble.test", now_ms(), 5, |hits| {
                        let found = session_smoke::icon_restored(&hits);
                        let _ = slint::invoke_from_event_loop(move || {
                            with_host(|host| host.session_icon = Some(found));
                        });
                    });
            }
        }
    }

    fn session_smoke_step(&mut self) {
        let Some(state) = self.checks.session_smoke.take() else {
            return;
        };
        let want = [page_url(SMOKE_PAGE), page_url(NAV_PAGE)];
        let order = self.model.order(self.model.active_space());
        let tabs: Vec<_> = order
            .iter()
            .filter_map(|t| self.model.tabs.get(t))
            .collect();
        let seen = session_smoke::Seen {
            ready: self.frame_rate.is_some(),
            urls: tabs.iter().map(|t| t.url.clone()).collect(),
            views: self.engine.view_count(),
            discarded: tabs
                .iter()
                .map(|t| t.lifecycle.liveness == Liveness::Discarded)
                .collect(),
            visible: order.iter().position(|&t| Some(t) == self.shown()),
            icon: self.session_icon,
        };
        let (state, action) = session_smoke::next(state, &seen, &want);
        self.checks.session_smoke = Some(state);
        match action {
            Some(session_smoke::Action::OpenSecond) => {
                let [_, second] = want;
                self.dispatch(new_tab(second, None));
            }
            Some(session_smoke::Action::Pass) => {
                self.session_passed = true;
                quit();
            }
            Some(session_smoke::Action::Fail(reason)) => self.fail(reason),
            None => {}
        }
    }

    // ----- input

    /// Tells the visible view about keyboard focus: it has it while the
    /// window has focus and the web content is the shell's focused item.
    /// A view that stops being visible loses focus first.
    pub fn sync_focus(&mut self, window: &AppWindow) {
        let target = if let Some(panel) = self.keys_to_panel() {
            Some(panel)
        } else {
            self.key_page().and_then(|t| self.views.get(&t)).copied()
        };
        if target != self.focus_view {
            if let Some(view) = self.focus_view.and_then(|id| self.engine.view(id)) {
                view.focus(false);
            }
            self.input.forget_view_focus();
            self.page_first.clear();
            self.focus_view = target;
        }
        // The page's own context menu takes the shell's focus but not the page's.
        let page_menu = target
            .and(self.key_page())
            .and_then(|t| self.surfaces.get(&t))
            .is_some_and(|s| s.menu.keeps_page_focus());
        let focused = self.input.window_focused && (window.get_web_focused() || page_menu);
        let view_open = self.focused_view().is_some();
        if let Some(focused) = self.input.focus_change(focused, view_open)
            && let Some(view) = self.focused_view()
        {
            view.focus(focused);
            // Slint enables IME only for its own text fields (and disables
            // it when one loses focus); the page needs it on the raw window.
            if focused {
                window
                    .window()
                    .with_winit_window(|w| w.set_ime_allowed(true));
            }
        }
    }

    /// Raw winit events, before Slint sees them: keys for the page,
    /// modifiers and window focus.
    pub fn window_event(&mut self, event: &WindowEvent) -> EventResult {
        let Some(window) = self.w.window.upgrade() else {
            return EventResult::Propagate;
        };
        self.clipboard.open(window.window());
        self.add_aux_window(&window);
        if let Some(e) = crate::run_state::event_of(event, || {
            window
                .window()
                .with_winit_window(|w| w.is_minimized())
                .flatten()
        }) {
            self.w.glass.borrow_mut().run_event(e, &window);
        }
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.input.keys = modifiers.state(),
            WindowEvent::Touch(t) => {
                let at = t
                    .location
                    .to_logical::<f64>(window.window().scale_factor().into());
                self.input_kind(InputEvent::Touch { x: at.x, y: at.y });
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = position.to_logical(window.window().scale_factor().into());
                self.input_kind(InputEvent::MouseMove {
                    x: f64::from(position.x),
                    y: f64::from(position.y),
                });
                self.input.pointer = Some(slint::LogicalPosition::new(position.x, position.y));
                self.w
                    .glass
                    .borrow_mut()
                    .pointer_move(position.x, position.y);
                self.tab_drag_pointer(position.x, position.y, false);
            }
            // macOS only in winit 0.30; Wayland's comes through `crate::wl`.
            WindowEvent::PinchGesture { delta, phase, .. } => self.pinch_gesture(*delta, *phase),
            WindowEvent::HoveredFile(_)
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFileCancelled => self.file_drop(&window, event),
            WindowEvent::CursorLeft { .. } => {
                self.input.pointer = None;
                self.w.glass.borrow_mut().pointer_leave();
            }
            // A left press on the shell sends a wave through the scene: through the glass under
            // it, else through the liquid. Not over the page, which covers the scene (the
            // design's web view never ripples the backdrop). Raw, so the side buttons navigate
            // over the shell too, without a wave.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                if input::taps_backdrop(*button)
                    && !self
                        .input
                        .pointer
                        .is_some_and(|p| input::over_content(&window, p))
                {
                    self.w.glass.borrow_mut().tap();
                }
                if let Some(action) = input::button_action(*button) {
                    self.perform(action);
                }
            }
            // A tab drag's release, wherever the pointer is (the pressed row may be gone).
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                let at = self
                    .input
                    .pointer
                    .map_or((self.drag_at.x as f32, self.drag_at.y as f32), |p| {
                        (p.x, p.y)
                    });
                self.tab_drag_pointer(at.0, at.1, true);
            }
            WindowEvent::Focused(focused) => {
                let current = self.model.focused() == self.w.id;
                if let Some(focused) = input::window_focus(current, *focused) {
                    self.input.window_focused = focused;
                    if focused && !current {
                        self.dispatch(Command::FocusWindow(self.w.id));
                    }
                    if self.model.focused() == self.w.id {
                        self.sync_focus(&window);
                    }
                }
            }
            WindowEvent::Ime(ime) => {
                if self.ime_event(&window, ime) {
                    return EventResult::PreventDefault;
                }
            }
            // Synthetic presses (keys already held when the window gained
            // focus, X11) never reach the page.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let key = self.input.key_input(event);
                return self.key(&window, &key);
            }
            _ => {}
        }
        EventResult::Propagate
    }

    /// One key from the keyboard (a raw winit key event, or `--smoke parity`'s): where it goes
    /// is `input::key_route`'s decision.
    pub(crate) fn key(&mut self, window: &AppWindow, key: &KeyInput) -> EventResult {
        // Link hints take every key while their labels show.
        if self.hint_key(key) {
            return EventResult::PreventDefault;
        }
        // Esc ends a page drag with nothing dropped, as in Chromium, or drops a tab drag.
        if key.pressed
            && key.unmodified == Key::Named(NamedKey::Escape)
            && (self.drag_cancelled() || self.cancel_tab_drag())
        {
            return EventResult::PreventDefault;
        }
        // Esc leaves page fullscreen before anything else sees it; it closes the peek only
        // when no shell field, sheet or menu holds the focus.
        if key.pressed && key.unmodified == Key::Named(NamedKey::Escape) {
            match input::esc_to(
                matches!(self.screen, Screen::Page { .. }),
                window.get_web_focused(),
                self.model.glance_of(self.w.id).is_some(),
            ) {
                input::EscTo::Fullscreen => {
                    self.apply_screen(window, ScreenEvent::Escape);
                    return EventResult::PreventDefault;
                }
                input::EscTo::Peek => {
                    self.dispatch(Command::CloseGlance(self.w.id));
                    return EventResult::PreventDefault;
                }
                input::EscTo::Route => {}
            }
        }
        // The keybinding editor's recorder takes keys before any shortcut does.
        if key.pressed
            && self.capture_key(input::chord_of(
                &key.unmodified,
                key.modifiers.keys,
                key.text.as_deref(),
            ))
        {
            return EventResult::PreventDefault;
        }
        let focus = if window.get_web_focused() {
            KeyFocus::Page
        } else if window.get_command_bar_state() != CommandBarState::Closed {
            KeyFocus::Field
        } else {
            KeyFocus::Shell
        };
        // Actions run on the press; releases follow the focus.
        let binding = key
            .pressed
            .then(|| input::chord_of(&key.unmodified, key.modifiers.keys, key.text.as_deref()))
            .flatten()
            .and_then(|chord| self.keymap.action_for(chord));
        let binding = if matches!(self.screen, Screen::Page { .. }) {
            input::page_fullscreen_binding(binding)
        } else {
            binding
        };
        let route = input::key_route(focus, binding, input::is_modifier(&key.unmodified));
        if matches!(
            route,
            KeyRoute::Page | KeyRoute::PageAndShell | KeyRoute::PageFirst(_)
        ) {
            self.pace(PaceEvent::Wake);
            if let Some(view) = self.focused_view() {
                view.key(key);
            }
            // A key down the page sees ends its autoscroll.
            if key.pressed {
                self.autoscroll_input(window, wubble_core::autoscroll::Input::Pressed);
            }
        }
        match route {
            KeyRoute::Perform(action) => {
                self.perform(action);
                EventResult::PreventDefault
            }
            // Runs on the view's `KeyUnhandled`, if the page sends it.
            KeyRoute::PageFirst(action) => {
                if let Some(id) = KeyId::of(key) {
                    self.page_first.push(id, action);
                }
                EventResult::PreventDefault
            }
            KeyRoute::Page => EventResult::PreventDefault,
            KeyRoute::PageAndShell | KeyRoute::Shell => EventResult::Propagate,
        }
    }

    /// Tells every view the frame rate if the window is now on a display
    /// with another refresh rate. Polled from the lifecycle tick: winit has
    /// no event for a move to another monitor at the same scale, and on
    /// Wayland the first rate comes from the fastest monitor because the
    /// surface has not entered an output yet.
    fn follow_frame_rate(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let rate = window
            .window()
            .with_winit_window(display_refresh_mhz)
            .map(pacing::frame_rate_for);
        if let (Some(current), Some(rate)) = (self.frame_rate, rate)
            && rate != current
        {
            self.frame_rate = Some(rate);
            for id in self.views.values().copied().collect::<Vec<_>>() {
                if let Some(view) = self.engine.view(id) {
                    view.set_frame_rate(rate);
                }
            }
        }
    }

    /// Steps the begin-frame driver, if this run uses external begin frames
    /// (views open only once the frame rate is known).
    pub fn pace(&mut self, event: PaceEvent) {
        let (Some(pace), Some(rate)) = (self.pace.as_mut(), self.frame_rate) else {
            return;
        };
        let period = Duration::from_secs_f64(1.0 / f64::from(rate));
        let (state, effects) = pacing::transition(pace.state, event, period);
        pace.state = state;
        for effect in effects {
            match effect {
                // Every page on screen (the panes of a split) paints.
                PaceEffect::Send => {
                    for tab in self.shown_tabs() {
                        if let Some(view) = self.tab_view(tab) {
                            view.send_begin_frame();
                        }
                    }
                }
                PaceEffect::ArmTimer(delay) => {
                    if let Some(pace) = &self.pace {
                        pace.timer.start(slint::TimerMode::SingleShot, delay, || {
                            with_host(|host| host.pace(PaceEvent::Timer))
                        });
                    }
                }
            }
        }
    }

    /// The view of the page on screen in pane `pane` (a tab id from the shell).
    fn pane_view(&mut self, pane: i32) -> Option<&mut View> {
        let tab = crate::shell::tab_id(pane).filter(|t| self.shown_tabs().contains(t))?;
        self.tab_view(tab)
    }

    pub fn pointer_moved(&mut self, pane: i32, point: Point) {
        if self.drag_pointer(point, false) {
            return;
        }
        if let (Some(window), Some(tab)) = (self.w.window.upgrade(), crate::shell::tab_id(pane)) {
            self.autoscroll_pointer(&window, tab, point);
        }
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        self.pace(PaceEvent::Wake);
        if let Some(view) = self.pane_view(pane) {
            view.mouse_move(point, &modifiers, now);
        }
        if let Some(tab) = crate::shell::tab_id(pane) {
            self.tip_pointer(tab, point.x, point.y);
        }
        self.media_pointer(pane, Some(point));
    }

    pub fn pointer_left(&mut self, pane: i32, point: Point) {
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        self.pace(PaceEvent::Wake);
        if let Some(view) = self.pane_view(pane) {
            view.mouse_leave(point, &modifiers, now);
        }
        self.media_pointer(pane, None);
    }

    /// A press in a pane that is not focused focuses it first (the design's pane focus on
    /// pointer down), so keys follow the page that was clicked.
    pub fn pointer_button(
        &mut self,
        pane: i32,
        button: PointerEventButton,
        pressed: bool,
        point: Point,
    ) {
        let Some(button) = input::mouse_button(button) else {
            return;
        };
        if !pressed && self.drag_pointer(point, true) {
            return;
        }
        if pressed && let Some(window) = self.w.window.upgrade() {
            self.autoscroll_input(&window, wubble_core::autoscroll::Input::Pressed);
        }
        if pressed {
            self.tip_dismiss();
        }
        if pressed && let Some(tab) = crate::shell::tab_id(pane) {
            // keys follow the pane that was pressed: the DevTools panel, the peek or a page
            let to = (tab == PANEL_TAB || self.model.is_glance(tab)).then_some(tab);
            if self.key_pane != to && self.shown_tabs().contains(&tab) {
                self.key_pane = to;
                if let Some(window) = self.w.window.upgrade() {
                    self.sync_focus(&window);
                }
            }
            // a press in the peek stays a peek: its buttons promote it
            if self.shown() != Some(tab)
                && !self.model.is_glance(tab)
                && self.model.window_tabs(self.w.id).contains(&tab)
            {
                self.dispatch(Command::ActivateTab(tab));
            }
        }
        let sent = if pressed {
            vec![button]
        } else {
            input::releases(self.input.buttons, button)
        };
        for button in sent {
            self.input.buttons = self.input.buttons.with(button, pressed);
            let (modifiers, now) = (self.input.modifiers(), self.input.now());
            self.pace(PaceEvent::Wake);
            if let Some(view) = self.pane_view(pane) {
                view.mouse_button(button, pressed, point, &modifiers, now);
            }
        }
    }

    pub fn pointer_cancelled(&mut self, pane: i32) {
        self.input.buttons = Default::default();
        if let Some(view) = self.pane_view(pane) {
            view.capture_lost();
        }
    }

    pub fn scrolled(&mut self, pane: i32, point: Point, dx: f32, dy: f32) {
        self.hints_scrolled();
        let modifiers = self.input.modifiers();
        self.pace(PaceEvent::Wake);
        if let Some(view) = self.pane_view(pane) {
            view.wheel(point, dx, dy, &modifiers);
        }
    }

    pub fn fail(&mut self, reason: String) {
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
        quit();
    }
}

/// What of the shell's state is saved with the session: the sidebar's mode
/// (peeking is a separate property, so a peek saves as hidden) and the seam's
/// saved width (the live width is 0 while the sidebar is hidden).
/// A window's inner size in logical px and its outer position in physical px, where the
/// platform tells it (not Wayland).
pub fn window_geometry(window: &AppWindow) -> wubble_core::window::Geometry {
    let win = window.window();
    let size = win.size().to_logical(win.scale_factor());
    let pos = win
        .with_winit_window(|w| w.outer_position().ok())
        .flatten()
        .map(|p| (p.x, p.y));
    wubble_core::window::Geometry {
        width: size.width.round() as u32,
        height: size.height.round() as u32,
        pos,
    }
}

/// Puts a window at its saved size and position, before it shows.
pub fn place_window(window: &AppWindow, g: wubble_core::window::Geometry) {
    let win = window.window();
    win.set_size(slint::LogicalSize::new(g.width as f32, g.height as f32));
    if let Some((x, y)) = g.pos {
        win.set_position(slint::PhysicalPosition::new(x, y));
    }
}

pub fn ui_state(window: &AppWindow) -> UiState {
    UiState {
        sidebar_width: window.get_sidebar_saved(),
        mode: window.get_sidebar_mode().into(),
    }
}

/// Wall-clock time for history and the recently-closed list.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

type Snapshot = slint::SharedPixelBuffer<slint::Rgba8Pixel>;

/// The window as it renders now. Smoke only.
fn snapshot(window: &AppWindow) -> Result<Snapshot, String> {
    window
        .window()
        .take_snapshot()
        .map_err(|e| format!("cannot snapshot the window: {e}"))
}

/// How many tabs a tidy moved into folders, and into how many folders: the tabs of `after`'s
/// folders that were top-level tabs in `before`.
fn tidy_summary(before: &[Item], after: &[Item]) -> (usize, usize) {
    let moved = |tabs: &[TabId]| {
        tabs.iter()
            .filter(|t| before.iter().any(|i| matches!(i, Item::Tab(b) if b == *t)))
            .count()
    };
    let (mut tabs, mut folders) = (0, 0);
    for item in after {
        if let Item::Folder(f) = item {
            let n = moved(&f.tabs);
            tabs += n;
            folders += usize::from(n > 0);
        }
    }
    (tabs, folders)
}

/// The URL a command would open a tab for or load, when another app handles it (and the tab
/// the user asked from, if any): such a command is handed to `Host::external_link` instead.
fn external_target(command: &Command) -> Option<(Option<TabId>, &str)> {
    let (opener, url) = match command {
        Command::Navigate { tab, url } => (Some(*tab), url),
        Command::NewTab { url, opener, .. } => (*opener, url),
        Command::BackgroundTab { url, opener } => (Some(*opener), url),
        Command::OpenBeside { url } => (None, url),
        _ => return None,
    };
    let external = InternalPage::from_url(url).is_none() && wubble_engine::page::is_external(url);
    external.then_some((opener, url.as_str()))
}

fn new_tab(url: String, opener: Option<TabId>) -> Command {
    Command::NewTab {
        url,
        container: None,
        opener,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::sleep::Pressure;
    use wubble_core::space::{ContainerId, SpaceId};

    fn one_tab() -> (Model, TabId) {
        let (m, _) = model::apply(
            Model::new(Instant::now()),
            new_tab("https://a.test/".into(), None),
        );
        let t = m.visible_tab().unwrap();
        (m, t)
    }

    #[test]
    fn links_to_other_apps_open_no_tab_and_load_nothing() {
        let t = TabId(7);
        let nav = |url: &str| Command::Navigate {
            tab: t,
            url: url.into(),
        };
        assert_eq!(
            external_target(&nav("mailto:a@b.test")),
            Some((Some(t), "mailto:a@b.test"))
        );
        assert_eq!(
            external_target(&new_tab("magnet:?xt=1".into(), None)),
            Some((None, "magnet:?xt=1"))
        );
        assert_eq!(
            external_target(&Command::BackgroundTab {
                url: "tel:1".into(),
                opener: t
            }),
            Some((Some(t), "tel:1"))
        );
        assert_eq!(external_target(&nav("https://a.test/")), None);
        assert_eq!(external_target(&nav("wubble://settings")), None);
        assert_eq!(external_target(&Command::CloseTab(t)), None);
    }

    #[test]
    fn model_effects_map_to_engine_calls() {
        let (m, t) = one_tab();
        let url = || "https://a.test/".to_string();
        let cases = [
            (
                Effect::CreateTab {
                    tab: t,
                    url: url(),
                    container: ContainerId::DEFAULT,
                },
                Call::OpenView { tab: t, url: url() },
            ),
            (Effect::CloseTab(t), Call::CloseView(t)),
            (
                Effect::Navigate { tab: t, url: url() },
                Call::Load { tab: t, url: url() },
            ),
            (Effect::ShowCommandBar, Call::ShowCommandBar),
            (Effect::ActivateTab(t), Call::Nothing),
            (Effect::SpaceChanged(SpaceId(0)), Call::Nothing),
            (
                Effect::WindowOpened(WindowId(1)),
                Call::OpenWindow(WindowId(1)),
            ),
            (
                Effect::WindowClosed(WindowId(1)),
                Call::CloseWindow(WindowId(1)),
            ),
            (
                Effect::FocusWindow(WindowId(1)),
                Call::FocusWindow(WindowId(1)),
            ),
            (Effect::LastWindowClosed, Call::Quit),
            (
                Effect::ContainerDeleted(ContainerId(3)),
                Call::ForgetContainer(ContainerId(3)),
            ),
        ];
        for (effect, want) in cases {
            assert_eq!(call(&effect, &m), want, "{effect:?}");
        }
    }

    #[test]
    fn lifecycle_effects_map_to_engine_calls() {
        let (m, t) = one_tab();
        let cases = [
            (
                Life::Show,
                Call::SetVisible {
                    tab: t,
                    visible: true,
                },
            ),
            (
                Life::Hide,
                Call::SetVisible {
                    tab: t,
                    visible: false,
                },
            ),
            (Life::Discard, Call::CloseView(t)),
            (Life::ReleaseTexture, Call::CloseView(t)),
            (
                Life::Restore,
                Call::OpenView {
                    tab: t,
                    url: "https://a.test/".into(),
                },
            ),
            (
                Life::Freeze,
                Call::SetFrozen {
                    tab: t,
                    frozen: true,
                },
            ),
            (
                Life::Unfreeze,
                Call::SetFrozen {
                    tab: t,
                    frozen: false,
                },
            ),
            (Life::CaptureSnapshot, Call::CaptureSnapshot(t)),
            (Life::ReleaseSnapshot, Call::ReleaseSnapshot(t)),
        ];
        for (effect, want) in cases {
            let effect = Effect::Lifecycle { tab: t, effect };
            assert_eq!(call(&effect, &m), want, "{effect:?}");
        }
    }

    /// Executes `effects` against a fake engine: the set of tabs with a
    /// view and which of them are visible, as `Host::execute` would.
    fn simulate(model: &Model, effects: &[Effect], views: &mut BTreeMap<TabId, bool>) {
        for effect in effects {
            match call(effect, model) {
                Call::OpenView { tab, .. } => {
                    views.entry(tab).or_insert(true);
                }
                Call::CloseView(tab) => {
                    views.remove(&tab);
                }
                Call::SetVisible { tab, visible } => {
                    if let Some(v) = views.get_mut(&tab) {
                        *v = visible;
                    }
                }
                Call::Load { .. }
                | Call::SetFrozen { .. }
                | Call::CaptureSnapshot(_)
                | Call::ReleaseSnapshot(_)
                | Call::ShowCommandBar
                | Call::OpenWindow(_)
                | Call::CloseWindow(_)
                | Call::FocusWindow(_)
                | Call::Quit
                | Call::ForgetContainer(_)
                | Call::Nothing => {}
            }
        }
    }

    #[test]
    fn views_follow_the_live_tabs_through_a_session() {
        let mut m = Model::new(Instant::now());
        let clock = Instant::now();
        let mut views = BTreeMap::new();
        let later = |mins: u64| Command::Tick {
            now: clock + Duration::from_secs(mins * 60),
            wall_ms: 0,
            pressure: Pressure::None,
        };
        let script = [
            new_tab("a".into(), None),
            new_tab("b".into(), None),
            new_tab("c".into(), Some(TabId(1))),
            Command::ActivateTab(TabId(1)),
            later(20),
            Command::ActivateTab(TabId(2)),
            Command::RendererGone(TabId(1)),
            Command::NewSpace {
                name: "W".into(),
                tint: wubble_core::space::Rgb(0, 0, 0),
                container: ContainerId::DEFAULT,
            },
            Command::SwitchSpace(SpaceId(1)),
            new_tab("w".into(), None),
            Command::SwitchSpace(SpaceId(0)),
            Command::CloseTab(TabId(2)),
            later(60),
            Command::ActivateTab(TabId(1)),
            Command::CloseTab(TabId(1)),
        ];
        for command in script {
            let label = format!("{command:?}");
            let (next, effects) = model::apply(m, command);
            m = next;
            simulate(&m, &effects, &mut views);
            assert_eq!(views.len(), m.live_tabs(), "{label}: view count");
            let visible: Vec<TabId> = views.iter().filter(|(_, v)| **v).map(|(t, _)| *t).collect();
            assert_eq!(
                visible,
                m.visible_tab().into_iter().collect::<Vec<_>>(),
                "{label}: visible views"
            );
        }
    }

    /// Two tabs as the store loads them: `b` visible and Active, `a`
    /// Discarded, neither with a view.
    fn loaded_session() -> (Model, TabId, TabId) {
        let m = [
            new_tab("https://a.test/".into(), None),
            new_tab("https://b.test/".into(), None),
        ]
        .into_iter()
        .fold(Model::new(std::time::Instant::now()), |m, c| {
            model::apply(m, c).0
        });
        let order = m.order(m.active_space());
        let (a, b) = (order[0], order[1]);
        let mut m = m;
        for t in m.tabs.values_mut() {
            t.lifecycle.liveness = if t.id == b {
                Liveness::Active
            } else {
                Liveness::Discarded
            };
            t.lifecycle.hidden_since = None;
        }
        (m, a, b)
    }

    #[test]
    fn a_session_restores_only_its_visible_tab() {
        let (m, _, b) = loaded_session();
        let (m, restore) = startup(m, false);
        let mut views = BTreeMap::new();
        simulate(&m, &restore.into_iter().collect::<Vec<_>>(), &mut views);
        assert_eq!(views, BTreeMap::from([(b, true)]));
        assert_eq!(m.live_tabs(), 1);
    }

    #[test]
    fn a_split_session_restores_and_shows_every_pane() {
        let (m, a, b) = loaded_session();
        let (mut m, _) = model::apply(m, Command::SplitWith(a));
        for t in m.tabs.values_mut() {
            t.lifecycle.liveness = Liveness::Active;
            t.lifecycle.hidden_since = None;
        }
        m.check().unwrap();
        let (m, restore) = startup(m, false);
        let mut views = BTreeMap::new();
        simulate(&m, &restore, &mut views);
        assert_eq!(views, BTreeMap::from([(a, true), (b, true)]));
        // a tab outside the split hides both panes; back in, both show
        let (m, fx) = model::apply(m, new_tab("https://c.test/".into(), None));
        simulate(&m, &fx, &mut views);
        let shown: Vec<TabId> = views.iter().filter(|(_, v)| **v).map(|(t, _)| *t).collect();
        assert_eq!(shown, m.visible_tabs());
        let (m, fx) = model::apply(m, Command::ActivateTab(b));
        simulate(&m, &fx, &mut views);
        let shown: Vec<TabId> = views.iter().filter(|(_, v)| **v).map(|(t, _)| *t).collect();
        assert_eq!(shown, vec![a, b]);
    }

    #[test]
    fn a_url_to_open_leaves_the_restored_tabs_without_views() {
        let (m, _, b) = loaded_session();
        let (mut m, restore) = startup(m, true);
        assert!(restore.is_empty());
        let mut views = BTreeMap::new();
        let script = [
            new_tab("https://c.test/".into(), None),
            Command::ActivateTab(b),
        ];
        for (i, command) in script.into_iter().enumerate() {
            let (next, effects) = model::apply(m, command);
            m = next;
            simulate(&m, &effects, &mut views);
            assert_eq!(views.len(), m.live_tabs(), "step {i}: view count");
            if i == 0 {
                assert!(!views.contains_key(&b), "b loaded in the background");
                assert_eq!(views.len(), 1);
            }
        }
        // Activated again, b gets its view and is the visible one.
        assert_eq!(views.get(&b), Some(&true));
    }

    #[test]
    fn restoring_a_tab_the_model_forgot_does_nothing() {
        let effect = Effect::Lifecycle {
            tab: TabId(9),
            effect: Life::Restore,
        };
        assert_eq!(call(&effect, &Model::new(Instant::now())), Call::Nothing);
    }

    #[test]
    fn restore_reloads_the_url_the_model_last_heard() {
        let (m, t) = one_tab();
        let (m, _) = model::apply(
            m,
            Command::UrlChanged {
                tab: t,
                url: "https://a.test/later".into(),
            },
        );
        let effect = Effect::Lifecycle {
            tab: t,
            effect: Life::Restore,
        };
        assert_eq!(
            call(&effect, &m),
            Call::OpenView {
                tab: t,
                url: "https://a.test/later".into()
            }
        );
    }
}
