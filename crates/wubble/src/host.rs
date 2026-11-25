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

mod agent;
mod autofill;
mod autoscroll;
mod ax;
mod boost;
mod builtins;
mod busy;
mod cursor;
mod data;
mod devtools;
mod downloads;
mod drag;
pub(crate) mod ext_tabs;
mod ext_ui;
mod external;
mod gallery;
pub(crate) mod gallery_orbs;
mod gesture;
mod gpu;
mod harness;
mod hosting;
mod layers;
mod look;
mod media;
mod memory;
mod overlay;
mod page_layer;
mod pages;
mod panes;
mod parity;
mod pinch;
mod popups;
mod preview;
mod print;
mod settings;
mod shortcuts;
mod snapshots;
pub(crate) mod start;
mod status;
mod surfaces;
mod swipe;
mod tab_drag;
mod tasks;
mod tooltip;
mod touch;
mod updates;
mod videos;
mod window;
pub(crate) use boost::KIT;
pub(crate) use data::DataSmoke;
pub use hosting::McpPlace;
pub use media::MediaObserver;
pub(crate) use parity::ParityRun;
pub(crate) use pinch::fixture as pinch_fixture;
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
use wubble_core::closed;
use wubble_core::devtools::PANEL_TAB;
use wubble_core::extensions::shell::POPUP_TAB;
use wubble_core::keys::{Keymap, Platform};
use wubble_core::lifecycle::Effect as Life;
use wubble_core::model::{self, Command, Effect, Model};
use wubble_core::neterror::CertExceptions;
use wubble_core::omnibox::{self, HistoryEntry, SearchEngine, Target};
use wubble_core::pages::InternalPage;
use wubble_core::region::{Region, next_region};
use wubble_core::site::{PrivateChoices, SiteChange, SitePermissions, SiteView, Stored};
use wubble_core::space::{ContainerId, Item, Side, SpaceId, Storage};
use wubble_core::tab::{Liveness, TabId};
use wubble_core::touch::{FingerInput, InputEvent};
use wubble_core::window::{Geometry, WindowId, WindowKind, WindowState};
use wubble_core::zoom::ZoomTable;
use wubble_engine::app::{Engine, PumpOutcome, PumpTimer, RequestMode, ViewId};
use wubble_engine::input::{KeyId, KeyInput, Point};
use wubble_engine::nav::{Cursor, Phase, PopupRequest, WebEvent};
use wubble_engine::view::{Desktop, View, ViewConfig};
use wubble_gfx::pacing::{self, BeginFrames, FrameStats, PaceEffect, PaceEvent, Pacing};
use wubble_gfx::scene::run::RunEvent;
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

/// How long the window rests after a move before the pages hear of its new place.
const DESKTOP_SETTLE: Duration = Duration::from_millis(100);

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
    ForgetContainer(ContainerId, Storage),
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
        Effect::ContainerDeleted { id, storage } => Call::ForgetContainer(*id, *storage),
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
    /// Tab search's recently closed tabs, newest first, as the store last listed them.
    closed: Vec<omnibox::SearchClosed>,
    /// Opened by Ctrl+E: every query is a search (`omnibox::as_search`).
    search: bool,
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
    pub passwords_smoke: Option<crate::passwords_smoke::PasswordsSmoke>,
    pub data_smoke: Option<DataSmoke>,
    /// `--smoke pinch` (`host/pinch.rs`).
    pub pinch_smoke: Option<crate::pinch_smoke::Run>,
    pub gallery_smoke: Option<crate::gallery_smoke::Stage>,
    pub autofill_smoke: Option<crate::autofill_smoke::AutofillSmoke>,
    pub windows_smoke: Option<crate::windows_smoke::WindowsSmoke>,
    /// `--smoke popup` (`host/popups.rs`).
    pub popup_smoke: Option<crate::popup_smoke::PopupSmoke>,
    pub media_smoke: Option<crate::media_smoke::MediaSmoke>,
    /// `--smoke media=probe`.
    pub media_probe: Option<crate::media_smoke::Probe>,
    pub builtin_smoke: Option<crate::builtin_smoke::BuiltinSmoke>,
    /// `--smoke video` and `--smoke boost`.
    pub video_smoke: Option<crate::video_smoke::VideoSmoke>,
    /// `--smoke ax`.
    pub ax_smoke: Option<crate::ax_smoke::AxSmoke>,
    /// `--smoke mcp`: the host answers each approval sheet with the pointer (`host/agent.rs`).
    pub mcp_smoke: Option<crate::mcp_smoke::McpSmoke>,
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
    /// CEF's paint count of the frame `image` holds (`NativeFrame::generation`): what a render
    /// of the pane draws, for the page layer's `Event::Presenting`.
    generation: u64,
}

impl PaneFrame {
    /// The stale frame still fading out over this one: its image, clip and opacity.
    fn fading(&self) -> Option<(&slint::Image, wubble_gfx::FrameSize, f32)> {
        let (image, clip) = self.prev.as_ref().filter(|_| self.view.is_fading())?;
        Some((image, *clip, (1.0 - self.springs.fade()) as f32))
    }
}

/// One window's external begin-frame driver (`Pacing::Shell`): its state and its timer.
#[derive(Default)]
pub(crate) struct Pace {
    state: BeginFrames,
    timer: slint::Timer,
}

/// Everything the pump drives. Owned by `HOST` in `main.rs`.
pub struct Host {
    pub engine: Engine,
    /// The window the host works on now (`window.rs`); the others are parked.
    w: WindowHost,
    parked: BTreeMap<WindowId, WindowHost>,
    /// Tabs whose pane draws its status bubble, as the kit reported (`status-drawn`): a bubble
    /// that leaves without ever having drawn has no fade to wait for.
    status_drawn: HashSet<TabId>,
    /// Makes the platform window of each model window that opens.
    factory: crate::WindowFactory,
    /// The engine's single-shot pump timer.
    pub timer: slint::Timer,
    /// The one owner of tabs and Spaces.
    model: Model,
    /// The one owner of which overlays are open or leaving, and of the hover and timed
    /// surfaces (`wubble_core::overlay::Ui`).
    ui: wubble_core::overlay::Ui,
    /// What the store's open and leaving overlays show (`layers::Contents`): written only by
    /// the store's `Mount` and `Unmount` executors.
    contents: layers::Contents,
    /// The store's one clock: the zero of every `now` its actions carry (`Host::surface_now`).
    ui_epoch: Instant,
    /// The store's one timer, armed by its `Effect::Arm` (`host/layers.rs`).
    ui_timer: slint::Timer,
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
    /// `--smoke pinch` passed, with its last scale.
    pub pinch_passed: Option<String>,
    /// `--smoke pinch`'s questions to the pages.
    pinch_asked: pinch::Asked,
    /// `--smoke parity` passed: the known gaps that failed as expected.
    pub parity_passed: Option<Vec<&'static str>>,
    pub windows_passed: bool,
    pub popup_passed: bool,
    pub settings_passed: bool,
    pub start_passed: bool,
    pub pages_passed: bool,
    pub passwords_passed: bool,
    pub data_passed: bool,
    /// `--smoke gallery` shot every scene.
    pub gallery_passed: bool,
    /// The scene on screen waited out [`gallery::SETTLE`] (`host/gallery.rs`).
    gallery_timer: slint::Timer,
    /// When the gallery's scene was shown: a burst's frames are timed from it.
    gallery_burst_start: std::time::Instant,
    /// `--smoke settings`: the stored value of the setting it flipped, read back.
    settings_read: Option<String>,
    /// Steps of the settings smoke's script the window has taken.
    settings_acted: u32,
    /// ... and where the page's scroll was at the last look.
    settings_scroll_seen: f32,
    pub(crate) input: InputState,
    keymap: Keymap,
    /// Page-first actions waiting to hear the page left their key alone.
    page_first: PendingKeys,
    store: StoreClient,
    /// Session changes not yet saved.
    save: SaveState,
    /// `None` until the first-run choice.
    autoscroll: autoscroll::AutoscrollPointer,
    /// Every drag (a tab row's, a page's: `wubble_core::drag`): one pointer, so one for every
    /// window (`drag.rs`, `tab_drag.rs`).
    drag: wubble_core::drag::Drag<wubble_engine::page::DragData>,
    /// Where a page drag's pointer last was, window px.
    drag_win: (f32, f32),
    /// The page drag's image, decoded once.
    drag_ghost: Option<drag::Ghost>,
    /// The bookmarks page's folder row under a drag, as the page reports it.
    bookmark_drop: Option<i64>,
    /// A drag from another app over the window (`drag.rs`).
    external_drag: Option<drag::ExternalDrag>,
    /// Touchpad pinch and history swipe (`gesture.rs`).
    gestures: gesture::Gestures,
    /// Opened print sheets (`print.rs`; the sheet is the content map's): the pages field restarts with each.
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
    /// A pop-up browser Chromium created, with its URL, while the command placing it runs: the
    /// tab it creates shows this view instead of a new one (`host/popups.rs`).
    adopting: Option<(String, ViewId)>,
    /// Where the pointer was at the tab drag's last event, window px.
    drag_at: wubble_core::tabdrag::Point,
    /// The window the drag started in: its pointer drives the drag, its ghost shows it.
    drag_window: WindowId,
    /// Asks the drag again once the pointer has rested on a Space's icon for the hover delay.
    drag_timer: slint::Timer,
    /// The page's IME composition.
    ime: Ime,
    /// Sound per live tab, as its view reported it.
    audio: AudioMap,
    /// Sites whose pages are muted (`settings::SOUND_MUTED_SITES`); applied on each commit.
    muted_sites: Vec<String>,
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
    /// Bumped per find-bar focus request.
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
    /// `settings::BLOCKER_EXCEPTIONS` and `settings::SLEEP_NEVER_SITES` as stored; the
    /// blocker and the model run them with the private windows' switches applied.
    blocker_hosts: Vec<String>,
    never_sleep_sites: Vec<String>,
    /// Every per-site choice made in private windows: in memory only (`write_site`), read over
    /// the stored ones (`site_view`), dropped with the private windows' container.
    private_choices: PrivateChoices,
    /// Certificate errors the user proceeded past, for this run only.
    cert_exceptions: CertExceptions,
    downloads: downloads::Downloads,
    /// The media downloader: each tab's streams and the download jobs (`host/media.rs`).
    media: media::Media,
    /// Who paces CEF's frames; with the shell's, each window drives the pages it shows
    /// (`WindowHost::pace`) and [`Host::unshown`] the agents' pages no window shows.
    pacing: Pacing,
    /// `Pacing::Shell`: the begin frames of agent pages no window shows, at
    /// [`pacing::UNSHOWN_FRAME_RATE`], running while there is one.
    unshown: slint::Timer,
    /// The system clipboard, for the pages' copies and pastes and the shell's copies.
    clipboard: crate::clipboard::Clipboard,
    /// When each tab's page's clipboard read started, until it is answered; one past
    /// `clipboard::READ_IN_FLIGHT` no longer counts (`wubble_core::clipboard::in_flight`).
    clipboard_reads: BTreeMap<TabId, Duration>,
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
    /// This run's closes still on the recently-closed list, for undo close tab and the
    /// reopen's place (`wubble_core::closed`).
    close_undo: closed::Undo,
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
    /// The updater (`host/updates.rs`).
    updater: crate::updater::Updater,
    /// Pages for screen readers (`host/ax.rs`).
    ax: ax::WebAx,
    /// Reduced motion and touch sizing (`look.rs`).
    look: look::Look,
    /// Shift+click opens a peek (`glance.enabled`).
    glance_enabled: bool,
    /// The pill shows whole URLs (`settings::UI_FULL_URLS`).
    full_urls: bool,
    /// The extensions' shell: the list, the footer popover, the popup, the install prompt
    /// (`host/ext_ui.rs`).
    ext_ui: ext_ui::ExtUi,
    /// Whether the MCP server is hosted (`host/hosting.rs`).
    hosting: hosting::McpHosting,
    /// `--smoke video` or `--smoke boost` passed.
    pub video_passed: Option<String>,
    /// `--smoke ax` passed, with the page's item count.
    pub ax_passed: Option<usize>,
    /// Agent sessions over MCP (`host/agent.rs`).
    agents: agent::Agents,
    /// `--smoke mcp` passed, with its step count.
    pub mcp_passed: Option<usize>,
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
        let installed = settings.extensions();
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
            status_drawn: HashSet::new(),
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
                closed: vec![],
                search: false,
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
            pinch_passed: None,
            pinch_asked: pinch::Asked::default(),
            parity_passed: None,
            windows_passed: false,
            popup_passed: false,
            settings_passed: false,
            start_passed: false,
            pages_passed: false,
            passwords_passed: false,
            data_passed: false,
            gallery_passed: false,
            gallery_timer: slint::Timer::default(),
            gallery_burst_start: std::time::Instant::now(),
            settings_read: None,
            settings_acted: 0,
            settings_scroll_seen: 0.0,
            input: InputState::default(),
            // `replay_settings` below applies the stored overrides, engine, zoom, ...
            keymap: Keymap::defaults(Platform::current()),
            page_first: PendingKeys::default(),
            store,
            save: SaveState::default(),
            autoscroll: Default::default(),
            drag: Default::default(),
            drag_win: (0.0, 0.0),
            drag_ghost: None,
            bookmark_drop: None,
            external_drag: None,
            gestures: Default::default(),
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
            adopting: None,
            drag_at: wubble_core::tabdrag::Point { x: 0.0, y: 0.0 },
            drag_window: WindowId(0),
            drag_timer: slint::Timer::default(),
            ime: Ime::default(),
            audio: AudioMap::new(),
            muted_sites: Vec::new(),
            favicons,
            site_zoom: ZoomTable::new(),
            zoom_default: wubble_core::zoom::DEFAULT_PERCENT,
            settings: settings::SettingsPage::new(settings.stored),
            pages: pages::PagesState::default(),
            find_focus: 0,
            page_zoom: None,
            blocked_seen: None,
            autofill_passed: false,
            media_passed: false,
            builtins_passed: None,
            autofill: autofill::Autofill::new(),
            blocking,
            site_permissions,
            blocker_hosts: vec![],
            never_sleep_sites: vec![],
            private_choices: PrivateChoices::default(),
            cert_exceptions: CertExceptions::default(),
            downloads: downloads::Downloads::new(downloads_dir, downloads),
            media: media::Media::new(media_observer),
            pacing: if begin_frames {
                Pacing::Shell
            } else {
                Pacing::CefTimer
            },
            unshown: slint::Timer::default(),
            clipboard,
            clipboard_reads: Default::default(),
            archive_after: ArchiveAfter::Off,
            archiving: HashMap::new(),
            routing_rules: vec![],
            tidy_undo: None,
            close_undo: closed::Undo::default(),
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
            updater: crate::updater::Updater::production(|e| {
                let _ = slint::invoke_from_event_loop(move || {
                    with_host(|host| host.update_event(e));
                });
            }),
            ax: ax::WebAx::default(),
            video_passed: None,
            ax_passed: None,
            agents: agent::Agents::default(),
            mcp_passed: None,
            overlay_passed: false,
            overlay_timer: slint::Timer::default(),
            look: look::Look::new(),
            glance_enabled: true,
            full_urls: false,
            ext_ui: ext_ui::ExtUi::new(installed),
            hosting: Default::default(),
            ui: Default::default(),
            contents: Default::default(),
            ui_epoch: Instant::now(),
            ui_timer: slint::Timer::default(),
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

    /// Opens the platform windows of the session's other windows (the first, `self.w`, is
    /// open); each is settled once it exists, and the focused one raised.
    pub fn open_session_windows(&mut self) {
        let ids: Vec<WindowId> = self.model.windows.iter().map(|w| w.id).collect();
        for id in ids {
            self.open_window(id);
        }
        if let Some(w) = self.model.window(self.w.id) {
            let focused = self.model.focused() == w.id;
            let geometry = w.geometry.as_ref().map(|g| g.restoring(focused));
            settle_when_shown(w.id, self.w.window.clone(), geometry, focused);
        }
        self.after_change();
    }

    /// Opens model window `id`'s platform window, unless it has one: a Main window takes the
    /// sidebar of the focused Main window. It is settled once it exists
    /// ([`settle_when_shown`]), and raised if it is the focused window.
    fn open_window(&mut self, id: WindowId) {
        let Some(w) = self.model.window(id) else {
            return;
        };
        if self.w.id == id || self.parked.contains_key(&id) {
            return;
        }
        let focused = self.model.focused() == id;
        let (kind, geometry) = (w.kind(), w.geometry.as_ref().map(|g| g.restoring(focused)));
        let ui = self.ui_state();
        let opened = match self.factory.open(id, kind, ui, geometry.clone()) {
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
        settle_when_shown(id, opened.window.clone(), geometry, focused);
        self.parked.insert(id, opened);
    }

    /// Closes window `id`'s platform window. Its frames and rings go with it; the tabs it
    /// showed are hidden (or closed) by the model.
    fn close_window(&mut self, id: WindowId) {
        // a drag it runs ends now: no release can reach it
        self.window_lost(id, true);
        // its overlays go with it
        let here = self.w.id;
        if self.enter(id) {
            self.act(wubble_core::overlay::Action::WindowGone(id));
        }
        self.enter(here);
        // its pages' layers go while it is still a window (none with `--page-layer=off`)
        if self
            .window_where(|w| w.id == id && !w.layers.is_empty())
            .is_some()
        {
            let here = self.w.id;
            self.enter(id);
            if let Some(window) = self.w.window.upgrade() {
                self.layers_gone(&window, |_, _| true);
            }
            self.enter(here);
        }
        if self.w.id == id {
            let Some(&other) = self.parked.keys().next() else {
                return;
            };
            self.enter(other);
        }
        if let Some(gone) = self.parked.remove(&id) {
            self.aux().remove_window(id);
            crate::orbs::remove(id);
            let _ = gone.handle.hide();
            // its page fullscreen goes with it: the page leaves (a tab moved out lives on), and
            // its late `Fullscreen(false)` finds no owner
            if let Screen::Page { tab, .. } = gone.screen
                && let Some(v) = self.tab_view(tab)
            {
                v.exit_fullscreen();
            }
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

    /// Records every open window's size, (where the platform tells) position, show state and
    /// monitor in the model, for the session. Read on save, and before a new window takes the
    /// focused one's size, rather than on each resize event.
    pub fn record_geometry(&mut self) {
        let here = self.w.id;
        let mut changed = vec![];
        for id in self.window_ids() {
            if self.enter(id) {
                changed.push(Command::ResizeWindow {
                    window: id,
                    geometry: window_geometry(&self.w.handle),
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
        // a close that would cancel private downloads asks first (Chrome's "N downloads will
        // be cancelled"); the answer closes (`close_cancelling_downloads`)
        if let Command::CloseWindow(id) = command
            && self.close_asks(wubble_core::window::Closing::Window(id))
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
        // a new window opens at the focused window's size, cascaded (`Geometry::cascaded`)
        if matches!(command, Command::NewWindow { .. }) {
            self.record_geometry();
        }
        let pre_close = matches!(
            command,
            Command::CloseTab(_)
                | Command::CloseTabs(_)
                | Command::CloseOthers(_)
                | Command::DeleteSpace(_)
                | Command::CloseWindow(_)
                | Command::CloseGlance(_)
        )
        .then(|| {
            self.model
                .tabs
                .keys()
                .copied()
                .map(|tab| (tab, self.closed_tab_entry(tab, &command)))
                .collect::<Vec<_>>()
        });
        if matches!(
            command,
            Command::ActivateTab(_)
                | Command::CloseTab(_)
                | Command::CloseTabs(_)
                | Command::CloseOthers(_)
                | Command::DeleteSpace(_)
                | Command::CloseGlance(_)
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
        if let Some(before) = pre_close {
            for (tab, closed) in before {
                if !self.model.tabs.contains_key(&tab) {
                    self.finish_closed(tab, closed);
                }
            }
        }
        // Surfaces outlive a sleeping tab's view, not the tab.
        let tabs = &self.model.tabs;
        self.surfaces.retain(|tab, _| tabs.contains_key(tab));
        self.committed.retain(|tab, _| tabs.contains_key(tab));
        self.held_popups.retain(|tab, _| tabs.contains_key(tab));
        self.preview.forget_except(|tab| tabs.contains_key(tab));
        // a tab moved into or out of a session's Space: its navigation rule follows the model
        self.agent_rules();
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
                let layout = self.pane_layout(&window, tab);
                let opened = match self.adopted(&url) {
                    Some(id) => self.adopt_view(id, layout, frame_rate),
                    None => {
                        // The tab's own cookies, storage and cache: its container's.
                        // an unknown tab gets no view (no Default stand-in)
                        let container = self.model.container_of(tab)?;
                        let config = ViewConfig {
                            url: url.clone(),
                            layout,
                            frame_rate,
                            begin_frames: self.begin_frames(),
                            gpu: self.w.presenters[0].context().clone(),
                            container,
                            // a private window's container is kept in memory only
                            storage: self.model.storage_of(container),
                        };
                        self.engine.open_view(config)
                    }
                };
                match opened {
                    Ok(id) => {
                        measure::mark(measure::Stage::FirstView);
                        self.views.insert(tab, id);
                        // Page drags run inside the window (`drag.rs`); a window its page opens
                        // is a browser of its own (`host/popups.rs`).
                        if let Some(view) = self.engine.view(id) {
                            view.set_drag_source(true);
                            view.allow_popup_browsers(true);
                        }
                        self.builtins_view_opened(tab, &url);
                        self.ax_view_opened(tab);
                        // the blocker knows a private tab's view from its first request
                        self.forget_gone_blocks();
                        self.agent_rules();
                        self.pace(PaceEvent::Wake);
                        self.pace_unshown();
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
                // An agent session's page is never shown, yet must lay out, keep its
                // accessibility tree and paint for screenshots: visible to CEF, in no window
                // (`host/agent.rs`).
                let driven = self.agent_drives(tab);
                if let Some(view) = self.tab_view(tab) {
                    // Shown in a pane, it must paint now: a hidden view's show asks CEF for a
                    // frame (`View::set_visible`), but this one was never hidden, and a still
                    // page paints nothing new, so its pane kept the small snapshot it stood in.
                    if visible && driven && view.is_visible() {
                        view.repaint();
                    }
                    view.set_visible(visible || driven);
                    // an agent's page in no window keeps painting, slowly
                    if driven {
                        view.set_frame_rate(pacing::view_frame_rate(frame_rate, visible));
                    }
                }
                // the driver of the window that shows it
                let back = self.w.id;
                if let Some(id) = self.model.window_of(tab) {
                    self.enter(id);
                }
                self.pace(PaceEvent::Wake);
                self.enter(back);
                self.pace_unshown();
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
            Call::ForgetContainer(c, storage) => {
                // the private windows' container leaves no download rows and no site choices
                // behind
                if storage == Storage::Memory {
                    self.forget_private_downloads(c);
                    self.private_choices = PrivateChoices::default();
                    self.boosts_forget_private();
                    self.run_site_lists();
                }
                self.engine.forget_container(c, storage);
            }
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
        self.ext_push();
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
        // a page that left its pane leaves its layer first (its frame goes to the shell)
        self.layers_gone(&window, |_, t| !kept.contains(&t));
        let before = self.w.frames.len();
        self.w.frames.retain(|tab, _| kept.contains(tab));
        window::free_slots(&mut self.w.slots, &kept);
        if self.w.frames.len() != before {
            self.set_page_cursor(&window, Cursor::Default);
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
        let mut view = crate::shell::view_model(
            &self.model,
            self.w.id,
            &self.audio,
            &blocked,
            self.full_urls,
        );
        self.agent_marks(&mut view);
        self.site_sound_marks(&mut view);
        crate::bind::render(&window, &view, &self.favicons);
        crate::bind::render_spaces_menu(
            &window,
            &self.spaces_popup(),
            &self
                .keymap
                .shortcut_for(wubble_core::action::Action::NewSpace)
                .map(|c| c.to_string())
                .unwrap_or_default(),
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
        self.render_extensions_in(&window);
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
        let ids: Vec<_> = self.views.iter().map(|(&t, &id)| (t, id)).collect();
        let (mut live, mut private) = (vec![], vec![]);
        for (tab, id) in ids {
            if let Some(browser) = self.engine.view(id).map(|v| v.browser_id()) {
                live.push(browser);
                if self.model.side_of_tab(tab) == Side::Private {
                    private.push(browser);
                }
            }
        }
        self.blocking.keep_views(&live, &private);
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
        if self.records(tab)
            && let Some(url) = self.tab_view(tab).map(|view| view.nav().page.url.clone())
        {
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
        if tab == POPUP_TAB {
            return self.popup_view();
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
        if self.keys_to_popup().is_some() {
            return self.popup_view();
        }
        self.tab_view(self.key_page()?)
    }

    fn tab_of(&self, view: ViewId) -> Option<TabId> {
        if self.devtools.view == Some(view) {
            return Some(PANEL_TAB);
        }
        if self.popup_view_is(view) {
            return Some(POPUP_TAB);
        }
        self.views
            .iter()
            .find_map(|(&tab, &id)| (id == view).then_some(tab))
    }

    /// Runs the pump's timer request.
    pub(super) fn arm_pump(&mut self, timer: Option<PumpTimer>) {
        match timer {
            Some(PumpTimer::Start(delay)) => {
                self.timer.start(slint::TimerMode::SingleShot, delay, || {
                    deliver(Wake::TimerFired)
                });
            }
            Some(PumpTimer::Cancel) => self.timer.stop(),
            None => {}
        }
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
        self.arm_pump(timer);
        if !worked {
            return;
        }
        // An agent's page no window shows paints at most at `pacing::UNSHOWN_FRAME_RATE`, and
        // no pane takes its frames: taken here (and dropped), a frame keeps the pump at that
        // pace, and a page that paints nothing lets it go quiet.
        let mut painted = false;
        for id in self.unshown_painting() {
            painted |= self
                .engine
                .view(id)
                .is_some_and(|v| v.take_frame().is_some());
        }
        if painted {
            let period = Duration::from_secs_f64(1.0 / f64::from(pacing::UNSHOWN_FRAME_RATE));
            let timer = self.engine.page_painted(period).timer;
            self.arm_pump(timer);
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
        self.ext_requests();
        self.media_pump();
        self.agent_check();
        self.enter_focused();
        self.agent_sheet_pump();
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        self.close_tabs_of_gone_views();
        self.smoke_step(SmokeInput::Worked);
        self.tabs_smoke_step();
        self.windows_smoke_step();
        self.popup_smoke_step();
        self.session_smoke_step();
        self.settings_smoke_step();
        self.pages_smoke_step();
        self.passwords_smoke_step();
        self.data_smoke_step();
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
        self.pinch_smoke_worked(&window);
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
        // the popup's page closed itself
        self.popup_view_gone();
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
        if tab == POPUP_TAB {
            return self.on_popup_web_event(event);
        }
        crate::load_run::web_event(&event);
        self.smoke_step(SmokeInput::Event(&event));
        self.page_smoke_step(page_smoke::Input::Web(&event));
        self.block_smoke_step(block_smoke::Input::Web(&event));
        if let WebEvent::TitleChanged(title) = &event {
            self.ax_smoke_step(crate::ax_smoke::Input::Title(title));
        }
        self.parity_web(&event);
        let taken_back = self.agent_web(tab, &event);
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
                self.set_page_cursor(window, cursor);
                self.autoscroll_input(window, wubble_core::autoscroll::Input::Cursor(cursor.pan()));
            }
            WebEvent::CursorImage(image) => {
                if self.shown_tabs().contains(&tab) {
                    self.set_cursor_image(image);
                }
            }
            WebEvent::UrlChanged(url) => {
                if taken_back {
                    return;
                }
                self.held_popups.remove(&tab);
                self.media_navigated(tab, &url);
                self.autofill_navigated(tab, &url);
                self.apply_error(tab, ErrorEvent::Committed(url.clone()));
                self.record_visit(tab, &url);
                self.restore_zoom(tab, &url);
                self.restore_site_sound(tab, &url);
                self.dispatch(Command::UrlChanged { tab, url });
            }
            WebEvent::TitleChanged(title) => {
                self.autofill_smoke_step(crate::autofill_smoke::Input::Title(&title));
                if self.records(tab)
                    && let Some(url) = self.committed.get(&tab)
                    && wubble_store::is_recordable(url)
                {
                    self.store.update_title(url, &title);
                }
                self.dispatch(Command::TitleChanged { tab, title });
            }
            WebEvent::PopupRequested(request) => self.popup_requested(tab, request),
            WebEvent::PopupOpened { request, view } => self.place_opened(tab, request, Some(view)),
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
            WebEvent::KeyUnhandled { key, editable } => {
                if Some(tab) == self.key_page()
                    && let Some(action) = self.page_first.unhandled(key)
                {
                    self.perform_unhandled(action, editable);
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

    /// Whether what `tab` does goes into the user's records (history visits, titles, favicons):
    /// not an agent session's loads, not a private window's, and not an unknown tab's (fails
    /// closed: `Model::side_of_tab`).
    pub(super) fn records(&self, tab: TabId) -> bool {
        !self.agent_drives(tab) && self.model.side_of_tab(tab) == Side::Normal
    }

    /// The origin of `tab`'s page, for remembered site choices.
    fn site_of(&self, tab: TabId) -> Option<String> {
        let url = self.model.tabs.get(&tab)?.url.as_str();
        wubble_core::site::origin_of(url)
    }

    /// Every per-site table as stored.
    fn stored_sites(&self) -> Stored<'_> {
        let (theme_default, themes) = self.page_themes();
        Stored {
            permissions: &self.site_permissions,
            zoom: &self.site_zoom,
            zoom_default: self.zoom_default,
            themes,
            theme_default,
            blocker: &self.blocker_hosts,
            never_sleep: &self.never_sleep_sites,
        }
    }

    /// The one read of the per-site tables for `tab`: the stored ones, under its own choices
    /// for a private tab (an unknown tab reads as private: `Model::side_of_tab`).
    pub(super) fn site_view(&self, tab: TabId) -> SiteView<'_> {
        self.site_view_of(self.model.side_of_tab(tab))
    }

    /// What every tab on `side` reads.
    fn site_view_of(&self, side: Side) -> SiteView<'_> {
        SiteView {
            stored: self.stored_sites(),
            private: (side == Side::Private).then_some(&self.private_choices),
        }
    }

    /// The one write of the per-site tables, for a change made in `tab`: a private tab's to
    /// its own table, in memory only; any other's to the store and the settings (which apply
    /// it back as the tables).
    pub(super) fn write_site(&mut self, tab: TabId, change: SiteChange) {
        if self.model.side_of_tab(tab) == Side::Private {
            let mut private = std::mem::take(&mut self.private_choices);
            private.write(&self.stored_sites(), change.clone());
            self.private_choices = private;
            match change {
                SiteChange::Theme { .. } => self.resync_page_themes(),
                SiteChange::Blocker(_) | SiteChange::NeverSleep(_) => {
                    self.run_site_lists();
                    self.render_pages();
                }
                SiteChange::Permission(..) | SiteChange::Zoom { .. } => {}
            }
            return;
        }
        let entry =
            |list: &[String], key: &'static str, host: String| (key, list.contains(&host), host);
        let toggle = match change {
            SiteChange::Permission(site, kind, decision) => {
                self.site_permissions.set(&site, kind, decision);
                self.store
                    .set_site_permission(&site, kind, decision, now_ms());
                None
            }
            SiteChange::Zoom { page, percent } => {
                if let Some(host) = wubble_core::zoom::host_of(&page) {
                    let table = wubble_core::zoom::next_table(
                        self.site_zoom.clone(),
                        &host,
                        percent,
                        self.zoom_default,
                    );
                    // Applied as the setting: the table becomes `site_zoom` there.
                    self.write_setting(
                        wubble_core::settings::SITE_ZOOM.key,
                        &wubble_store::text_of::<wubble_store::SiteZoom>(&table),
                    );
                }
                None
            }
            SiteChange::Theme { page, theme } => {
                if let Some(host) = wubble_core::zoom::host_of(&page) {
                    let (default, themes) = self.page_themes();
                    let table =
                        wubble_core::pagetheme::next_table(themes.clone(), &host, theme, default);
                    // Applied as the setting: the table becomes the built-ins' there.
                    self.write_setting(
                        wubble_core::settings::PAGE_THEME_SITES.key,
                        &wubble_store::text_of::<wubble_store::PageThemeSites>(&table),
                    );
                }
                None
            }
            SiteChange::Blocker(host) => Some(entry(
                &self.blocker_hosts,
                wubble_core::settings::BLOCKER_EXCEPTIONS.key,
                host,
            )),
            SiteChange::NeverSleep(site) => Some(entry(
                &self.never_sleep_sites,
                wubble_core::settings::SLEEP_NEVER_SITES.key,
                site,
            )),
        };
        // a list's row: the stored setting the settings page shows
        if let Some((key, listed, host)) = toggle {
            if listed {
                self.setting_entry_removed(key, &host);
            } else {
                self.setting_entry_added(key, &host);
            }
            self.render_pages();
        }
    }

    /// The blocker and the model run, per side, what that side's tabs read (`SiteView`): the
    /// stored lists for normal tabs, with the private windows' switches for private ones. A
    /// private switch never reaches a normal tab, its menus or the stored settings.
    pub(super) fn run_site_lists(&mut self) {
        let (normal, private) = (
            self.site_view_of(Side::Normal),
            self.site_view_of(Side::Private),
        );
        let blocker = [normal.blocker(), private.blocker()];
        let never = [
            (Side::Normal, normal.never_sleep()),
            (Side::Private, private.never_sleep()),
        ];
        self.blocking.set_exceptions(&blocker[0]);
        self.blocking.set_private_exceptions(&blocker[1]);
        for (side, sites) in never {
            if sites != self.model.never_sites(side) {
                self.dispatch(Command::SetNeverSleepSites { side, sites });
            }
        }
    }

    // ----- browsing data

    /// `Action::ClearBrowsingData`: what can be cleared for `tab`, as the shell's menu near the
    /// top of the window. Choosing a row is the confirmation.
    pub(super) fn clear_data_menu(&mut self, tab: TabId) {
        let origin = self.site_of(tab);
        let entries = crate::menu::clear_entries(&self.model, tab, origin.as_deref());
        self.ask_in_menu(tab, entries);
    }

    /// A question as the shell's menu near the top of the window, over `tab`'s page: its rows
    /// say what each answer does, choosing one is the answer, and dismissing it does nothing
    /// (Clear Browsing Data, the private downloads a close would cancel). Whether it opened.
    pub(super) fn ask_in_menu(&mut self, tab: TabId, entries: Vec<crate::menu::Entry>) -> bool {
        let Some(window) = self.w.window.upgrade() else {
            return false;
        };
        let size = window.window().size();
        let at = ((size.width / 2) as i32, (size.height / 5) as i32);
        // the app's own question: no overlay refuses it (a page's dialog waiting keeps it asked)
        self.open_shell_menu(tab, at, entries, true)
    }

    /// Deletes what the clear menu's row named, then says so.
    pub(super) fn clear_data(&mut self, tab: TabId, what: crate::menu::ClearData) {
        use crate::menu::ClearData;
        let done = match what {
            ClearData::Containers(containers) => {
                // each brought up with its own storage: a private one in memory, never on disk
                let containers: Vec<_> = containers
                    .iter()
                    .map(|&c| (c, self.model.storage_of(c)))
                    .collect();
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
                let routed = wubble_core::routing::resolve(&self.model, &self.routing_rules, &url);
                self.dispatch(Command::OpenLittle {
                    url,
                    space: routed.space,
                    container: routed.container,
                })
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
        self.open_shell_menu(tab, at, entries, false);
    }

    // ----- command bar

    /// The new-tab row: the bar is opening on an empty query. On a start page (already a new
    /// tab) its capsule takes the keys instead, for the start tab.
    pub fn bar_for_new_tab(&mut self, window: &AppWindow) {
        if self.shown_page() == Some(InternalPage::Start) {
            return self.bar_for_address(window);
        }
        self.open_bar(window, BarMode::NewTab, String::new());
    }

    /// Ctrl+L or the address pill: the bar opens on the visible tab's URL. On a start page the
    /// bar is its capsule (app.slint, wubble_core::chrome::Placement): it goes to the start tab
    /// on an empty query, or keeps what was typed there (Ctrl+L again selects it).
    pub fn bar_for_address(&mut self, window: &AppWindow) {
        // a Little or Popup window's address is read-only
        if self.w.kind != wubble_core::window::WindowKind::Main {
            return;
        }
        let visible = self.shown();
        match visible.and_then(|t| self.model.tabs.get(&t)) {
            Some(tab) if InternalPage::from_url(&tab.url) == Some(InternalPage::Start) => {
                if self.bar.mode != BarMode::Navigate(tab.id) {
                    self.open_bar(window, BarMode::Navigate(tab.id), String::new());
                }
            }
            Some(tab) => {
                let url = tab.url.clone();
                self.open_bar(window, BarMode::Navigate(tab.id), url);
            }
            None => self.open_bar(window, BarMode::NewTab, String::new()),
        }
    }

    /// Mod+Shift+A (tab search): the bar on every open tab, the recently closed ones following
    /// once the store has listed them. A Little or Popup window has no bar.
    pub fn bar_for_tabs(&mut self, window: &AppWindow) {
        if self.w.kind != wubble_core::window::WindowKind::Main {
            return;
        }
        self.bar.closed.clear();
        self.open_bar(window, BarMode::Tabs, String::new());
        self.list_closed();
    }

    /// Asks the store for the recently-closed list; tab search re-renders when it arrives.
    fn list_closed(&mut self) {
        self.store.recently_closed(move |list| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.closed_arrived(list));
            });
        });
    }

    fn closed_arrived(&mut self, list: persist::ClosedList) {
        if self.bar.mode != BarMode::Tabs {
            return;
        }
        self.bar.closed = list
            .entries
            .into_iter()
            .map(|c| omnibox::SearchClosed {
                title: c.title,
                url: c.url,
                closed_at_ms: c.closed_at_ms,
            })
            .collect();
        self.bar.icons = list.icons;
        if let Some(window) = self.w.window.upgrade() {
            self.render_bar(&window);
        }
    }

    /// Tab search's close button or Shift+Delete on row `index`: its tab closes (onto the
    /// recently-closed list, which is listed again) and the bar stays open on the rows left.
    pub fn bar_result_closed(&mut self, index: i32) {
        if self.bar.mode != BarMode::Tabs {
            return;
        }
        let tab = usize::try_from(index)
            .ok()
            .and_then(|i| self.bar.rows.get(i))
            .and_then(BarRow::icon_tab);
        let Some(tab) = tab else {
            return;
        };
        self.dispatch(Command::CloseTab(tab));
        if let Some(window) = self.w.window.upgrade() {
            self.render_bar(&window);
        }
        self.list_closed();
    }

    pub(super) fn open_bar(&mut self, window: &AppWindow, mode: BarMode, query: String) {
        // over the page the bar takes the place of another overlay, or goes again while a page's
        // dialog waits (wubble_core::overlay); the start page's capsule is the page's own field
        let over = window.get_command_placement() == wubble_ui::CommandPlacement::Overlay;
        if over && !self.overlay_open(wubble_core::overlay::Kind::CommandBar, None) {
            let _ = self
                .w
                .window
                .upgrade_in_event_loop(|window| window.invoke_dismiss_command_bar());
            return;
        }
        self.bar.mode = mode;
        self.bar.search = false;
        self.suggest_event(wubble_core::suggest::FetchEvent::Closed);
        window.set_command_query(query.as_str().into());
        self.bar_query(window, &query);
    }

    /// Renders the rows for `query` at once from what is at hand, and asks
    /// the store for its history hits, which re-render when they arrive.
    pub fn bar_query(&mut self, window: &AppWindow, query: &str) {
        self.bar.query = if self.bar.search {
            omnibox::as_search(query)
        } else {
            query.into()
        };
        self.bar.generation += 1;
        // tab search matches what is at hand: no history, no engine suggestions
        if self.bar.mode == BarMode::Tabs {
            self.render_bar(window);
            return;
        }
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

    /// What this window's bar consults: a private window's suggests from this machine only.
    fn suggest_settings(&self) -> wubble_core::suggest::SuggestSettings {
        wubble_core::suggest::SuggestSettings {
            engine: self.search_engine.unwrap_or(SearchEngine::DuckDuckGo),
            live: wubble_core::suggest::LiveSuggestions::of(
                self.live_suggestions,
                self.search_engine.is_some(),
                self.model.side_of_window(self.w.id),
            ),
        }
    }

    /// One event of the live suggestion fetch; re-renders the rows when its list changed.
    pub fn suggest_event(&mut self, event: wubble_core::suggest::FetchEvent) {
        let settings = self.suggest_settings();
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
        if self.bar.mode == BarMode::Tabs {
            let tabs = bar::search_tabs(&self.model, |t| self.agent_drives(t));
            self.bar.rows = bar::tab_search_rows(&tabs, &self.bar.closed, &self.bar.query);
            crate::bind::render_bar(
                window,
                &self.bar.rows,
                &self.bar.icons,
                &self.favicons,
                true,
            );
            return;
        }
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
        crate::bind::render_bar(
            window,
            &self.bar.rows,
            &self.bar.icons,
            &self.favicons,
            false,
        );
    }

    /// The bar closed (Esc, a click outside, or a chosen row): nothing more may be fetched.
    pub fn bar_closed(&mut self) {
        self.bar.search = false;
        self.suggest_event(wubble_core::suggest::FetchEvent::Closed);
    }

    /// The address menu's edit of the bar's field. The field's own edits run later, outside
    /// the host, as the field then reports its text back into it; Delete is done here.
    pub(super) fn field_edit(&mut self, edit: crate::menu::FieldEdit) {
        use crate::menu::FieldEdit as E;
        let op = match edit {
            E::Cut => wubble_ui::FieldEdit::Cut,
            E::Copy => wubble_ui::FieldEdit::Copy,
            E::Paste => wubble_ui::FieldEdit::Paste,
            E::SelectAll => wubble_ui::FieldEdit::SelectAll,
            E::Delete => {
                self.field_delete();
                return;
            }
        };
        let _ = self
            .w
            .window
            .upgrade_in_event_loop(move |window| window.invoke_command_field_edit(op));
    }

    /// The menu's Delete: the selected text leaves the bar's field, the caret where it began.
    fn field_delete(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let text = window.get_command_query().to_string();
        let (start, end) = (
            window.get_command_selection_start(),
            window.get_command_selection_end(),
        );
        let range = usize::try_from(start)
            .ok()
            .zip(usize::try_from(end).ok())
            .filter(|&(a, b)| a < b && text.get(a..b).is_some());
        let Some((a, b)) = range else {
            return;
        };
        let left = format!("{}{}", &text[..a], &text[b..]);
        window.set_command_query(left.as_str().into());
        self.bar_query(&window, &left);
        let _ = self
            .w
            .window
            .upgrade_in_event_loop(move |window| window.invoke_command_caret(start));
    }

    /// Paste and Go (the address menu's row, `Action::PasteAndGo`): `text` loads as if typed in
    /// `field`. In the bar, as its mode loads a typed row, and the bar goes as after Enter; from
    /// the pill, in the visible tab (a new tab without one).
    pub(super) fn paste_and_go(&mut self, field: crate::menu::AddressField, text: &str) {
        let target = RowTarget::Omni(match omnibox::classify(text) {
            omnibox::Intent::Url(url) => Target::Url(url),
            omnibox::Intent::Search(q) => Target::Search(q),
        });
        if field == crate::menu::AddressField::Pill {
            self.bar.mode = self.shown().map_or(BarMode::NewTab, BarMode::Navigate);
        }
        self.choose(&target);
        if field == crate::menu::AddressField::Bar {
            self.bar_closed();
            // later, outside the host: the bar's close calls back into it
            let _ = self
                .w
                .window
                .upgrade_in_event_loop(|window| window.invoke_command_pick());
        }
    }

    /// Ctrl+E, once the bar is open on the visible tab (`Plan::FocusSearch`): the field empties
    /// and what is typed in it from now on is searched, never opened as an address.
    pub(crate) fn bar_for_search(&mut self, window: &AppWindow) {
        // a Little or Popup window's address is read-only: no bar opened there
        if self.w.kind != WindowKind::Main {
            return;
        }
        self.bar.search = true;
        window.set_command_query("".into());
        self.bar_query(window, "");
    }

    /// The keys held as a row is chosen: where it opens (`omnibox::disposition`), and whether
    /// Ctrl asks for `www.` and `.com` around the typed name.
    fn enter_keys(&self) -> (omnibox::Disposition, bool) {
        let keys = self.input.keys;
        let mods = omnibox::EnterMods {
            ctrl: keys.control_key(),
            alt: keys.alt_key(),
            shift: keys.shift_key(),
            cmd: keys.super_key(),
        };
        let platform = wubble_core::keys::Platform::current();
        (omnibox::disposition(mods, platform), mods.ctrl)
    }

    pub fn bar_chosen(&mut self, index: i32) {
        let row = usize::try_from(index)
            .ok()
            .and_then(|i| self.bar.rows.get(i));
        if let Some(row) = row {
            let (disposition, complete) = self.enter_keys();
            // Ctrl+Enter on the typed name: www. and .com around it (Chromium's desired TLD)
            let completed = (complete && !self.bar.search && index == 0)
                .then(|| omnibox::with_desired_tld(&self.bar.query))
                .flatten();
            let target = match completed {
                Some(url) => RowTarget::Omni(Target::Url(url)),
                None => row.target.clone(),
            };
            self.choose_as(&target, disposition);
        }
        // after reading the row: clearing the suggestions re-renders the rows
        self.bar_closed();
    }

    fn choose(&mut self, target: &RowTarget) {
        self.choose_as(target, omnibox::Disposition::AsOpened);
    }

    fn choose_as(&mut self, target: &RowTarget, disposition: omnibox::Disposition) {
        let choice =
            bar::target_command_as(target, self.bar.mode, self.search_engine, disposition);
        match choice {
            Some(Choice::Model(command)) => {
                self.dispatch(command);
                if bar::is_typed(target) {
                    self.typed_nav.started(self.shown());
                }
            }
            Some(Choice::Perform(action)) => self.perform(action),
            Some(Choice::ReopenClosed { url, closed_at_ms }) => {
                self.store.take_closed(&url, closed_at_ms, |entry| {
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| host.restore_closed(closed::Event::Taken, entry));
                    });
                });
            }
            Some(Choice::NewWindow(url)) => {
                self.dispatch(Command::NewWindow { space: None });
                self.dispatch(new_tab(url, None));
                if bar::is_typed(target) {
                    self.typed_nav.started(self.shown());
                }
            }
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
            // added last: shown at once (an empty Space opens its start page)
            Plan::NewSpace(command) => {
                self.dispatch(command);
                if let Some(space) = self.model.spaces.last().map(|s| s.id) {
                    self.dispatch(Command::SwitchSpace(space));
                }
            }
            Plan::View(op) => {
                let Some(tab) = self.key_page().or_else(|| self.shown()) else {
                    return;
                };
                self.typed_nav.ended(tab);
                if matches!(op, ViewOp::ToggleMute) {
                    self.toggle_mute(tab);
                    return;
                }
                let zoom_default = self.zoom_default;
                let Some(view) = self.tab_view(tab) else {
                    return;
                };
                match op {
                    ViewOp::Reload => view.reload(),
                    ViewOp::HardReload => view.hard_reload(),
                    // DevTools' empty-cache reload: the clear goes to the browser first
                    ViewOp::EmptyCacheHardReload => {
                        view.cdp("Network.clearBrowserCache", &serde_json::json!({}));
                        view.hard_reload();
                    }
                    ViewOp::Stop => view.stop(),
                    ViewOp::Back => view.back(),
                    ViewOp::Forward => view.forward(),
                    ViewOp::ZoomIn => view.zoom_in(),
                    ViewOp::ZoomOut => view.zoom_out(),
                    // To the user's default, which CEF's own reset (always 100 %) ignores.
                    ViewOp::ZoomReset => view.set_zoom_percent(zoom_default),
                    // the user's mute: `toggle_mute` above
                    ViewOp::ToggleMute => {}
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
            Plan::DevToolsConsole => self.devtools_console(),
            Plan::PasteAndGo => {
                let text = self.w.window.upgrade().and_then(|window| {
                    self.clipboard
                        .load(window.window(), crate::clipboard::Buffer::Clipboard)
                });
                if let Some(text) = text.filter(|t| !t.trim().is_empty()) {
                    self.paste_and_go(crate::menu::AddressField::Pill, &text);
                }
            }
            Plan::FocusSearch => self.focus_search(),
            Plan::SavePage { tab, url } => self.save_as(tab, url),
            Plan::OpenFile => self.open_file(),
            Plan::CaretBrowsing => self.caret_event(wubble_core::caret::Event::Toggle),
            Plan::RaiseWindow(window) => {
                self.dispatch(Command::FocusWindow(window));
                self.raise_window(window);
            }
            Plan::Minimize => {
                if let Some(window) = self.w.window.upgrade() {
                    window.window().set_minimized(true);
                }
            }
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
            // Later, outside the host, as FocusAddress: the bar opens itself, then calls back.
            Plan::SearchTabs => {
                let _ = self
                    .w
                    .window
                    .upgrade_in_event_loop(|window| window.invoke_request_tab_search());
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
            Plan::ReopenClosed => {
                self.close_undo(closed::Event::Reopen);
            }
            Plan::UndoClose => {
                self.close_undo(closed::Event::Command);
            }
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
            Plan::StopAgents => self.agents_stop(),
            Plan::Quit => {
                if !self.close_asks(wubble_core::window::Closing::App) {
                    quit();
                }
            }
            Plan::Nothing => {}
        }
    }

    /// A page-first action whose key the page left alone: Mod+Z's undo close tab is told
    /// whether the page's focus was on an editable element, so a field keeps its undo.
    pub(super) fn perform_unhandled(&mut self, action: Action, editable: bool) {
        match action {
            Action::UndoCloseTab => {
                self.close_undo(closed::Event::Key { editable });
            }
            action => self.perform(action),
        }
    }

    /// Steps the closed-tab machine and runs its effects. The restores are returned: they
    /// need the entry the store popped (`restore_closed`).
    fn close_undo(&mut self, event: closed::Event) -> Vec<closed::Effect> {
        let (state, effects) = closed::transition(std::mem::take(&mut self.close_undo), event);
        self.close_undo = state;
        let mut restores = vec![];
        for effect in effects {
            match effect {
                closed::Effect::Pop => self.store.pop_closed(|entry| {
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| host.restore_closed(closed::Event::Popped, entry));
                    });
                }),
                closed::Effect::FieldUndo => self.perform(Action::Undo),
                closed::Effect::NothingToUndo => {
                    if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, "Nothing to undo");
                    }
                }
                restore @ closed::Effect::Restore { .. } => restores.push(restore),
            }
        }
        restores
    }

    /// A recently-closed entry the store took off its list (the newest, popped, or the one tab
    /// search chose, taken; `event` tells the machine which): a close of this run back in its
    /// Space at its place (pinned, in its folder), an older one in the active Space; in its
    /// container if that still exists. The one restore of a closed tab, for Mod+Shift+T, Mod+Z
    /// and tab search alike.
    fn restore_closed(
        &mut self,
        event: fn(Option<(String, i64)>) -> closed::Event,
        closed: Option<ClosedTab>,
    ) {
        let popped = closed.as_ref().map(|c| (c.url.clone(), c.closed_at_ms));
        let restores = self.close_undo(event(popped));
        let Some(closed) = closed else {
            return;
        };
        for restore in restores {
            let closed::Effect::Restore { space, place } = restore else {
                continue;
            };
            let newest = self.model.tabs.keys().next_back().copied();
            self.reopen(closed.url.clone(), space, Some(closed.container));
            let tab = self.model.tabs.keys().next_back().copied();
            if let (Some(to), Some(tab)) = (place, tab.filter(|t| Some(*t) != newest)) {
                self.dispatch(Command::MoveTab { tab, to });
            }
        }
    }

    /// Opens `url` again, the one way a tab comes back: in `space` if it still exists (the view
    /// moves there), else the last normal window's; in `container` if that still exists. A
    /// closed tab is a normal one (private tabs are never kept), so it never reopens in a
    /// private window: from one, its Space shows in the last normal window
    /// (`Crossing::ToNormalWindow`), and a container on the other side is refused.
    pub(super) fn reopen(
        &mut self,
        url: String,
        space: Option<SpaceId>,
        container: Option<ContainerId>,
    ) {
        let space = space
            .filter(|s| self.model.spaces.iter().any(|k| k.id == *s))
            .unwrap_or_else(|| self.model.normal_space());
        self.open_in(url, Some(space), container, None);
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
    /// In a private window no rule applies: its links stay in it (Chrome's incognito window).
    pub(super) fn open_routed(&mut self, url: String, opener: Option<TabId>) {
        let private = opener.map_or_else(
            || self.model.side_of_space(self.model.active_space()) == Side::Private,
            |t| self.model.side_of_tab(t) == Side::Private,
        );
        let routed = if private {
            wubble_core::routing::Routed::default()
        } else {
            wubble_core::routing::resolve(&self.model, &self.routing_rules, &url)
        };
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

    /// Snapshot of a tab for the recently-closed list, taken before the model drops it.
    /// `command` is about to run. None for a tab filed in the Archive, one an agent session
    /// drives, and what `Model::closes_into_recent` leaves out (a peek's, an agent Space's being
    /// deleted, an empty one).
    /// With it, the close as undo close tab keeps it (its place).
    fn closed_tab_entry(
        &self,
        tab: TabId,
        command: &Command,
    ) -> Option<(ClosedTab, closed::Closed)> {
        // an agent's and a private window's closed tabs are none of the user's records
        if self.archiving.contains_key(&tab) || !self.records(tab) {
            return None;
        }
        let undo = closed::Closed::of(&self.model, tab, command, now_ms())?;
        let t = self.model.tabs.get(&tab)?;
        let entry = ClosedTab {
            url: undo.url.clone(),
            title: t.title.clone(),
            container: t.container,
            space: Some(undo.space),
            closed_at_ms: undo.closed_at_ms,
        };
        Some((entry, undo))
    }

    /// Cleans up per-tab host state for a tab the model removed and records it if eligible.
    fn finish_closed(&mut self, tab: TabId, closed: Option<(ClosedTab, closed::Closed)>) {
        self.media_tab_closed(tab);
        self.committed.remove(&tab);
        self.typed_nav.ended(tab);
        if let Some((entry, undo)) = closed {
            self.store.push_closed(entry);
            self.close_undo(closed::Event::Closed(undo));
        }
    }

    /// The engine committed `url` in `tab`: a history visit unless it only
    /// moved within the page. Typed if the bar started it. The title follows
    /// with `TitleChanged`.
    fn record_visit(&mut self, tab: TabId, url: &str) {
        let previous = self.committed.insert(tab, url.to_string());
        if previous.as_deref().is_some_and(|p| p != url) {
            self.drag_gone(tab);
        }
        let typed = self.typed_nav.committed(tab);
        if persist::visit_to_record(previous.as_deref(), url) && self.records(tab) {
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
        self.store.save(&self.model, &self.agents.spaces(), ui);
        self.save = SaveState::default();
    }

    /// Orderly shutdown: the final save, then the store thread finishes and
    /// closes the database. `ui` was read before the window went away.
    pub fn shutdown(&mut self, ui: UiState) {
        // agent sessions end with the browser, their Spaces closed before the last save
        self.agents_quit();
        self.mcp_quit();
        self.media_shutdown();
        self.save_download_progress();
        self.store.save(&self.model, &self.agents.spaces(), ui);
        std::mem::replace(&mut self.store, StoreClient::none()).shutdown();
    }

    /// The coarse lifecycle tick; also the save policy's idle clock.
    pub fn tick(&mut self, now: Instant) {
        self.follow_frame_rate();
        self.follow_desktop();
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
            shown_title: visible
                .and_then(|t| self.model.tabs.get(&t))
                .map(|t| t.title.clone())
                .unwrap_or_default(),
            settled: self.w.window.upgrade().is_some_and(|w| page_at_rest(&w)),
            shown_index: visible.and_then(|v| order.iter().position(|&t| t == v)),
        }
    }

    /// `--smoke tabs`: the card shows solid page 1 edge to edge, and none of solid page 0, the
    /// page shown before it (QA HIGH-04).
    fn check_no_bleed(&self) -> Result<(), String> {
        let window = self.w.window.upgrade().ok_or("no window")?;
        let shot = snapshot(&window)?;
        let scale = window.window().scale_factor();
        let region = pixels::Region {
            x: physical(window.get_content_x(), scale),
            y: physical(window.get_content_y(), scale),
            width: physical(window.get_content_width(), scale),
            height: physical(window.get_content_height(), scale),
        };
        let [before, shown] = tabs_smoke::SOLID;
        let bled = pixels::count_near(&shot, region, before);
        let own = pixels::count_near(&shot, region, shown);
        let all = (pixels::GRID * pixels::GRID) as usize;
        if bled > 0 {
            return Err(format!(
                "the fresh tab's card shows the page before it: {bled} of {all} samples"
            ));
        }
        if own < all {
            return Err(format!(
                "the fresh tab's card does not show its page: {own} of {all} samples"
            ));
        }
        println!("smoke: tabs no bleed ok samples={all}");
        Ok(())
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
        let (mut framed, mut bars, mut ghosts, mut entering) = (true, 0, 0, 0);
        let (mut start_in_newest, mut capsule_in_newest) = (false, false);
        for id in self.window_ids() {
            if self.enter(id) {
                let tabs = self.model.window_tabs(id);
                framed &= tabs
                    .iter()
                    .filter(|t| web(t))
                    .all(|t| self.w.frames.contains_key(t));
                let start = self.shown_page() == Some(InternalPage::Start);
                start_in_newest |= Some(id) == newest && start;
                if let Some(window) = self.w.window.upgrade() {
                    let (state, placement) = (
                        window.get_command_bar_state(),
                        window.get_command_placement(),
                    );
                    bars += usize::from(crate::windows_smoke::is_bar(state, placement, start));
                    capsule_in_newest |= Some(id) == newest
                        && start
                        && state == CommandBarState::Open
                        && placement == wubble_ui::CommandPlacement::InPage;
                    ghosts += usize::from(!window.get_drag_ghost().is_empty());
                    entering += usize::from(!page_at_rest(&window));
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
            capsule_in_newest,
            ghosts,
            entering,
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

    /// `--measure tabs`: opens `pages` as tabs after the window's one, once, and shows the
    /// first again.
    pub(crate) fn measure_open_tabs(&mut self, pages: &[String]) {
        let order = self.model.order(self.model.active_space());
        if order.len() != 1 {
            return;
        }
        for url in pages {
            self.dispatch(new_tab(url.clone(), None));
        }
        self.dispatch(Command::ActivateTab(order[0]));
    }

    /// `--measure tabs`: shows the next tab of the window's Space (the first after the last).
    pub(crate) fn measure_next_tab(&mut self) {
        let order = self.model.order(self.model.active_space());
        let next = self
            .shown()
            .and_then(|t| order.iter().position(|&o| o == t))
            .map_or(0, |i| (i + 1) % order.len().max(1));
        if let Some(&tab) = order.get(next) {
            self.dispatch(Command::ActivateTab(tab));
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
        // the switch's fade runs on frames: a still page asks for none of its own
        if matches!(state.stage, tabs_smoke::Stage::Solid(_))
            && let Some(window) = self.w.window.upgrade()
        {
            window.window().request_redraw();
        }
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
            Some(tabs_smoke::Action::OpenSolid(i)) => {
                self.dispatch(new_tab(page_url(&tabs_smoke::solid_page(i)), None));
            }
            Some(tabs_smoke::Action::CheckNoBleed) => {
                if let Err(e) = self.check_no_bleed() {
                    self.fail(e);
                }
            }
            // as a pinned site would: the tab its link opens goes last, under another tab
            Some(tabs_smoke::Action::OpenLinkPage) => {
                self.dispatch(new_tab(page_url(&tabs_smoke::link_page()), None));
                if let Some(tab) = self.shown() {
                    self.dispatch(Command::PinTab(tab));
                }
            }
            // a real click through the page's pointer path: the link opens with a gesture
            Some(tabs_smoke::Action::ClickLink(middle)) => {
                if let (Some(tab), Some(window)) = (self.shown(), self.w.window.upgrade()) {
                    let pane = crate::shell::pane_id(tab);
                    let at = Point {
                        x: window.get_content_width() / 2.0,
                        y: window.get_content_height() / 2.0,
                    };
                    let button = if middle {
                        PointerEventButton::Middle
                    } else {
                        PointerEventButton::Left
                    };
                    self.pointer_moved(pane, at);
                    self.pointer_button(pane, button, true, at);
                    self.pointer_button(pane, button, false, at);
                }
            }
            Some(tabs_smoke::Action::CloseShown) => {
                if let Some(tab) = self.shown() {
                    self.dispatch(Command::CloseTab(tab));
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
                    Some(seam) => {
                        let (x, y) = (seam.x as f32, height / 2.0);
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
            // By its chord through the host's key path, as a hand presses it (Ctrl+Shift+\ on
            // a US layout; QA CRIT-04).
            Some(split_smoke::Action::Unsplit) => {
                match self
                    .keymap
                    .shortcut_for(wubble_core::action::Action::Unsplit)
                    .and_then(crate::parity::key_inputs)
                {
                    Some(keys) => self.smoke_keys(keys),
                    None => self.fail("split: no keys for the unsplit chord".into()),
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
        let scroll = window.global::<wubble_ui::PageScroll>().get_y();
        let found = settings_smoke::drive::look(&window);
        let look = found.clone().unwrap_or_default();
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
            scroll,
            still: (scroll - self.settings_scroll_seen).abs() < 0.01,
            sidebar_row: settings_smoke::drive::sidebar_row(&window),
            sidebar_shell: wubble_core::seam::SidebarMode::from(window.get_sidebar_mode())
                .id()
                .to_owned(),
            sidebar_stored: self
                .settings
                .stored
                .get(wubble_core::settings::SIDEBAR_MODE.key)
                .cloned()
                .unwrap_or_default(),
            shortcuts_found: settings_smoke::drive::shortcuts_found(&window),
            capture: match self.settings.capture() {
                wubble_core::keys::Capture::Idle => "idle",
                wubble_core::keys::Capture::Armed(_) => "armed",
                wubble_core::keys::Capture::Conflict { .. } => "conflict",
            },
            runs: settings_smoke::PROBED
                .iter()
                .map(|c| {
                    let chord = wubble_core::keys::Chord::parse(c).expect("a probed chord parses");
                    (*c, self.keymap.action_for(chord).map(|(a, _)| a.id()))
                })
                .collect(),
            select_found: found.is_some(),
            select_open: look.0,
            select_shown: look.1,
            engine_stored: wubble_core::apply::value_of(
                &wubble_core::settings::SEARCH_ENGINE,
                &self.settings.stored,
            )
            .to_owned(),
            engine_applied: self.search_engine.map_or("", |e| e.id()).to_owned(),
            acted: self.settings_acted,
        };
        self.settings_scroll_seen = scroll;
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
                self.perform(Action::OpenSettings);
                // from the event loop, outside the host the shell's callbacks call into
                let window = self.w.window.clone();
                slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                    wheel_burst(window, settings_smoke::TICKS)
                });
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
            Some(settings_smoke::Action::Drive(settings_smoke::Do::Chord(chord))) => {
                // the keyboard's way in: the host's key path, then the step is done
                match wubble_core::keys::Chord::parse(chord)
                    .ok()
                    .and_then(crate::parity::key_inputs)
                {
                    Some(keys) => self.smoke_keys(keys),
                    None => self.fail(format!("no keys for the chord {chord:?}")),
                }
                let _ = slint::invoke_from_event_loop(|| {
                    with_host(|host| host.settings_acted += 1);
                });
            }
            Some(settings_smoke::Action::Drive(act)) => {
                // as the pointer and keys come: from the event loop, outside the host; the step
                // counts as done a turn of the loop later, when what it set going has settled
                let _ = self.w.window.upgrade_in_event_loop(move |window| {
                    let done = || {
                        let _ = slint::invoke_from_event_loop(|| {
                            with_host(|host| host.settings_acted += 1);
                        });
                    };
                    if let Err(why) = settings_smoke::drive::run(&window, act, done) {
                        with_host(|host| host.fail(why));
                    }
                });
            }
            Some(settings_smoke::Action::Read { key }) => {
                self.settings_read = None;
                self.store.read_settings(move |rows| {
                    let value = rows.get(key).cloned().unwrap_or_default();
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| host.settings_read = Some(value));
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
            Some(InternalPage::AgentLog) => window.get_agent_log().row_count(),
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
                InternalPage::AgentLog => Action::OpenAgentLog,
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

    /// `--smoke passwords`: what the passwords page shows, and the page's own callbacks.
    fn passwords_smoke_step(&mut self) {
        use crate::passwords_smoke::{self as smoke, Shown};
        use slint::Model;
        let Some(state) = self.checks.passwords_smoke.take() else {
            return;
        };
        let Some(window) = self.w.window.upgrade() else {
            self.checks.passwords_smoke = Some(state);
            return;
        };
        let view = window.get_passwords_view();
        let seen = smoke::Seen {
            ready: self.frame_rate.is_some(),
            on_page: self.shown_page() == Some(InternalPage::Passwords),
            shown: match view.state {
                wubble_ui::VaultState::Locked => Shown::Locked,
                wubble_ui::VaultState::Unlocked => Shown::Unlocked,
                wubble_ui::VaultState::Unavailable => Shown::Unavailable,
                wubble_ui::VaultState::Unreadable => Shown::Unreadable,
                wubble_ui::VaultState::KeyLost => Shown::KeyLost,
            },
            body: view.body.to_string(),
            notice: view.notice.to_string(),
            rows: window
                .get_logins()
                .iter()
                .map(|l| smoke::Row {
                    id: l.id.to_string(),
                    origin: l.origin.to_string(),
                    username: l.username.to_string(),
                    revealed: l.has_revealed.then(|| l.revealed.to_string()),
                })
                .collect(),
        };
        let (state, actions) = smoke::next(state, &seen);
        self.checks.passwords_smoke = Some(state);
        let weak = self.w.window.clone();
        // the page's own callbacks, as a click sends them: later, outside the host
        let call = |f: Box<dyn FnOnce(AppWindow) + Send>| {
            let _ = weak.upgrade_in_event_loop(f);
        };
        for action in actions {
            match action {
                smoke::Action::OpenPage => self.perform(Action::OpenPasswords),
                smoke::Action::Leave => self.perform(Action::NewTab),
                smoke::Action::Unlock => call(Box::new(|w| w.invoke_passwords_unlock())),
                smoke::Action::Add {
                    site,
                    user,
                    password,
                } => call(Box::new(move |w| {
                    w.invoke_login_added(site.into(), user.into(), password.into())
                })),
                smoke::Action::Edit {
                    id,
                    site,
                    user,
                    password,
                } => call(Box::new(move |w| {
                    w.invoke_login_edited(id.into(), site.into(), user.into(), password.into())
                })),
                smoke::Action::Reveal(id) => {
                    call(Box::new(move |w| w.invoke_login_reveal(id.into())))
                }
                smoke::Action::Delete(id) => {
                    call(Box::new(move |w| w.invoke_login_deleted(id.into())))
                }
                smoke::Action::Pass => {
                    println!("smoke: passwords {:?} ok", state.phase);
                    self.passwords_passed = true;
                    quit();
                }
                smoke::Action::Fail(reason) => self.fail(reason),
            }
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
                    let menu =
                        wubble_core::overlay::Layer::new(wubble_core::overlay::Kind::Menu, None);
                    let open = self.open_content(self.w.id, menu).is_some();
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
        let target = if let Some(panel) = self.keys_to_panel().or_else(|| self.keys_to_popup()) {
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
        let menu = wubble_core::overlay::Layer::new(wubble_core::overlay::Kind::Menu, None);
        let page_menu = target.and(self.key_page()).is_some_and(|key| {
            matches!(
                self.open_content(self.w.id, menu),
                Some(layers::Content::Menu { tab, menu }) if *tab == key && menu.keeps_page_focus()
            )
        });
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

    /// This window was hidden (occluded, minimised, sized to nothing) or shown again: its pages
    /// stop painting while nobody can see them. CEF paints windowless pages, so Chromium never
    /// learns that the window is covered; only this tells it. A page playing sound (its video
    /// would be suspended) and an agent's (screenshots) keep painting.
    fn occlusion(&mut self, hidden: bool) {
        self.w.hidden = hidden;
        if hidden {
            for tab in self.shown_tabs() {
                let audible = self
                    .model
                    .tabs
                    .get(&tab)
                    .is_some_and(|t| t.busy.contains(Reason::Audible));
                let pip = self.model.pip_tab() == Some(tab);
                if self.w.occluded.contains(&tab)
                    || !hidden_window_stops(audible, self.agent_drives(tab), pip)
                {
                    continue;
                }
                if let Some(view) = self.tab_view(tab)
                    && view.is_visible()
                {
                    view.set_visible(false);
                    self.w.occluded.push(tab);
                }
            }
        } else {
            let shown = self.shown_tabs();
            for tab in std::mem::take(&mut self.w.occluded) {
                if shown.contains(&tab)
                    && let Some(view) = self.tab_view(tab)
                {
                    view.set_visible(true);
                }
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
        // The first event of a window finds its desktop (winit has no event for the window's
        // creation); a move or a scale change finds a new one.
        if self.w.desktop.is_none() || matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
            self.follow_desktop();
        } else if matches!(event, WindowEvent::Moved(_)) {
            // A drag sends dozens a second, each a screen change for every page: the place
            // the window rests at is told once.
            let id = self.w.id;
            self.w
                .desktop_settle
                .start(slint::TimerMode::SingleShot, DESKTOP_SETTLE, move || {
                    crate::with_host_in(id, Host::follow_desktop)
                });
        }
        self.input.touching = matches!(event, WindowEvent::Touch(_));
        if let Some(e) = crate::run_state::event_of(event, || {
            window
                .window()
                .with_winit_window(|w| w.is_minimized())
                .flatten()
        }) {
            self.w
                .glass
                .borrow_mut()
                .run_event(e, crate::run_state::wake_of(event), &window);
            match e {
                RunEvent::Hidden => self.occlusion(true),
                RunEvent::Shown => self.occlusion(false),
                _ => {}
            }
        }
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.input.keys = modifiers.state(),
            // Slint hands the page a wheel as pixels either way (`TouchArea.scroll-event`), so
            // what the device sent is read here, before Slint sees the event.
            WindowEvent::MouseWheel { delta, phase, .. } => {
                self.input.wheel_precise = input::wheel_is_precise(delta);
                // A touchpad scroll's stop (Wayland's `axis_stop`; X11 and Windows send none,
                // macOS glides itself): the page glides on. Queued, so the stop frame's own
                // scroll, which Slint sends after this, comes first.
                #[cfg(target_os = "linux")]
                if *phase == slint::winit_030::winit::event::TouchPhase::Ended {
                    let id = self.w.id;
                    let _ = slint::invoke_from_event_loop(move || {
                        crate::with_host_in(id, |host| {
                            let now = host.fling_now();
                            host.fling(wubble_core::fling::FlingInput::Lift { now });
                        })
                    });
                }
                #[cfg(not(target_os = "linux"))]
                let _ = phase;
            }
            WindowEvent::Touch(t) => {
                let at = t
                    .location
                    .to_logical::<f64>(window.window().scale_factor().into());
                self.input_kind(InputEvent::Touch { x: at.x, y: at.y });
                self.touch(&window, t);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = position.to_logical(window.window().scale_factor().into());
                self.input_kind(InputEvent::MouseMove {
                    x: f64::from(position.x),
                    y: f64::from(position.y),
                });
                self.set_pointer(
                    &window,
                    Some(slint::LogicalPosition::new(position.x, position.y)),
                );
                self.w
                    .glass
                    .borrow_mut()
                    .pointer_move(position.x, position.y);
                self.tab_drag_pointer(position.x, position.y, false);
                self.drag_window_pointer(false);
                // X11 moves the pointer with a page's finger: no mouse moves for the page beside
                // its touch.
                if self.w.fingers.on_page() {
                    return EventResult::PreventDefault;
                }
            }
            // macOS only in winit 0.30; Wayland's comes through `crate::wl`.
            WindowEvent::PinchGesture { delta, phase, .. } => self.pinch_gesture(*delta, *phase),
            WindowEvent::HoveredFile(_)
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFileCancelled => self.file_drop(&window, event),
            WindowEvent::CursorLeft { .. } => {
                self.set_pointer(&window, None);
                self.w.glass.borrow_mut().pointer_leave();
            }
            // A left press on a favourite sends a wave through its glass (chrome.tsx `Fav`
            // onPointerDown: the design's only wave in the window). Not over the page, which
            // covers the scene. Raw, so the side buttons navigate over the shell too, without a
            // wave.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                if input::taps_backdrop(*button)
                    && window.get_fav_hovered()
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
                self.drag_window_pointer(true);
                self.drag_released_elsewhere();
                self.drag_window_released();
            }
            WindowEvent::Focused(focused) => {
                // before Slint sees it: the start page's capsule keeps the bar through it
                window.set_window_active(*focused);
                // a drag of the window ends with nothing dropped (an OS drag goes on), what ends
                // with the window's keys is cancelled, and the pages' fingers are cancelled
                if !*focused {
                    self.window_lost(self.w.id, false);
                    self.fingers(FingerInput::OnScreen(vec![]));
                }
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
        // A key stops a glide, as in Chrome (a PageDown is not added to it); a modifier alone
        // or a held key's repeats do not.
        if key.pressed && !key.repeat && !input::is_modifier(&key.unmodified) {
            self.fling(wubble_core::fling::FlingInput::Interrupt);
        }
        // Link hints take every key while their labels show.
        if self.hint_key(key) {
            return EventResult::PreventDefault;
        }
        // A draft in the agent sheet's list editor takes Escape before anything: it drops the
        // draft, never the whole sheet (which would deny) nor the page's load.
        if key.unmodified == Key::Named(NamedKey::Escape)
            && self.agent_sheet_open()
            && window.get_agent_sheet_editing()
        {
            return EventResult::Propagate;
        }
        // Esc closes the topmost shell overlay first, wherever the keys are (the page, a sheet,
        // a menu), but the page's Esc never answers a notice or a permission prompt:
        // wubble_core::overlay.
        if key.pressed
            && key.unmodified == Key::Named(NamedKey::Escape)
            && key.modifiers.keys.is_empty()
            && self.overlay_escape(window)
        {
            return EventResult::PreventDefault;
        }
        // Esc leaves page fullscreen before anything else sees it; it closes the peek only
        // when no shell field, sheet or menu holds the focus.
        if key.pressed && key.unmodified == Key::Named(NamedKey::Escape) {
            match input::esc_to(
                matches!(self.w.screen, Screen::Page { .. }),
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
            match window.get_command_placement() {
                wubble_ui::CommandPlacement::InPage => KeyFocus::Address,
                wubble_ui::CommandPlacement::Overlay => KeyFocus::Field,
            }
        } else {
            KeyFocus::Shell
        };
        // Actions run on the press; releases follow the focus.
        let binding = key
            .pressed
            .then(|| input::chord_of(&key.unmodified, key.modifiers.keys, key.text.as_deref()))
            .flatten()
            .and_then(|chord| self.keymap.action_for(chord));
        let binding = if matches!(self.w.screen, Screen::Page { .. }) {
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
            let shown = self.shown_tabs();
            for (tab, id) in self.views.iter().map(|(t, v)| (*t, *v)).collect::<Vec<_>>() {
                let shown = shown.contains(&tab) || !self.agent_drives(tab);
                if let Some(view) = self.engine.view(id) {
                    view.set_frame_rate(pacing::view_frame_rate(rate, shown));
                }
            }
        }
    }

    /// Tells every view on screen where the window and its monitor are on the desktop when that
    /// changed (the page's `screen.*` and `window.screenX`). Winit's `Moved` and
    /// `ScaleFactorChanged` say the window moved or its scale changed; the lifecycle tick
    /// finds the rest (the window dragged to another monitor at the same scale, the first
    /// output a Wayland surface enters).
    fn follow_desktop(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let desktop = window.window().with_winit_window(desktop_of).flatten();
        if desktop != self.w.desktop {
            self.w.desktop = desktop;
            let rects = self.pane_rects(&window);
            self.lay_out_views_with(&window, &rects);
        }
    }

    /// Views open on external begin frames (`Pacing::Shell`).
    pub(super) fn begin_frames(&self) -> bool {
        self.pacing == Pacing::Shell
    }

    /// The display's refresh period, once the frame rate is known.
    pub(crate) fn refresh(&self) -> Option<Duration> {
        self.frame_rate
            .map(|rate| Duration::from_secs_f64(1.0 / f64::from(rate)))
    }

    /// Steps this window's begin-frame driver, if this run uses external begin frames
    /// (views open only once the frame rate is known).
    pub fn pace(&mut self, event: PaceEvent) {
        if !self.begin_frames() {
            return;
        }
        let Some(period) = self.refresh() else {
            return;
        };
        let (state, effects) = pacing::transition(self.w.pace.state, event, period);
        self.w.pace.state = state;
        for effect in effects {
            match effect {
                // Every page on screen (the panes of a split) paints.
                PaceEffect::Send => {
                    let mut sent = false;
                    for tab in self.shown_tabs() {
                        if let Some(view) = self.tab_view(tab) {
                            sent |= view.send_begin_frame();
                        }
                    }
                    if !sent {
                        self.pace(PaceEvent::Skipped);
                    }
                }
                PaceEffect::ArmTimer(delay) => {
                    let id = self.w.id;
                    self.w
                        .pace
                        .timer
                        .start(slint::TimerMode::SingleShot, delay, move || {
                            crate::with_host_in(id, |host| host.pace(PaceEvent::Timer))
                        });
                }
            }
        }
    }

    /// `Pacing::Shell`: the pages that paint with no window showing them (an agent session's,
    /// `pacing::view_frame_rate`), which no window's driver reaches.
    fn unshown_painting(&self) -> Vec<ViewId> {
        let shown: Vec<TabId> = self
            .model
            .windows
            .iter()
            .flat_map(|w| self.model.window_tabs(w.id))
            .collect();
        self.views
            .iter()
            .filter(|&(&tab, _)| !shown.contains(&tab) && self.agent_drives(tab))
            .map(|(_, &id)| id)
            .collect()
    }

    /// `Pacing::Shell`: begins the clock of the pages no window shows, if there is one; it
    /// stops itself once there is none.
    fn pace_unshown(&mut self) {
        if !self.begin_frames() || self.unshown.running() || self.unshown_painting().is_empty() {
            return;
        }
        let every = Duration::from_secs_f64(1.0 / f64::from(pacing::UNSHOWN_FRAME_RATE));
        self.unshown.start(slint::TimerMode::Repeated, every, || {
            with_host(Host::unshown_tick)
        });
    }

    /// One begin frame for each page no window shows; the clock stops when there is none.
    fn unshown_tick(&mut self) {
        let views = self.unshown_painting();
        if views.is_empty() {
            self.unshown.stop();
        }
        for id in views {
            if let Some(view) = self.engine.view(id) {
                view.send_begin_frame();
            }
        }
    }

    /// The view of the page on screen in pane `pane` (a tab id from the shell).
    fn pane_view(&mut self, pane: i32) -> Option<&mut View> {
        let tab = crate::shell::tab_id(pane).filter(|t| self.shown_tabs().contains(t))?;
        self.tab_view(tab)
    }

    pub fn pointer_moved(&mut self, pane: i32, point: Point) {
        // Slint's mouse made from a finger: the page has the finger as touch (`host/touch.rs`).
        if self.input.touching {
            return;
        }
        if self.drag_pointer(point, false) {
            return;
        }
        self.pointer_over_page(true);
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

    /// Where the pointer is in the window, or None once it left: kept here and forwarded to the
    /// window only while the active tab has an open or collapsed video overlay.
    pub(crate) fn set_pointer(&mut self, window: &AppWindow, at: Option<slint::LogicalPosition>) {
        self.input.pointer = at;
        if self.video_overlay_active() {
            self.sync_pointer(window);
        }
    }

    /// Writes the stored pointer position into Slint (used while a video overlay is active and
    /// once when an overlay opens).
    pub(super) fn sync_pointer(&self, window: &AppWindow) {
        let at = self.input.pointer;
        window.set_pointer_in(at.is_some());
        if let Some(p) = at {
            window.set_pointer_x(p.x);
            window.set_pointer_y(p.y);
        }
    }

    pub fn pointer_left(&mut self, pane: i32, point: Point) {
        if self.input.touching {
            return;
        }
        self.pointer_over_page(false);
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
        // Slint's press made from a finger lands it on the page, as touch (`host/touch.rs`); the
        // rest of its mouse input is dropped. A pane that takes no touch only takes the focus.
        if self.input.touching {
            if pressed {
                match crate::shell::tab_id(pane).filter(|&t| self.touchable(t)) {
                    Some(tab) => self.fingers(FingerInput::Landed {
                        pane: tab,
                        point: (point.x, point.y),
                    }),
                    None => self.page_pressed(crate::shell::tab_id(pane)),
                }
            }
            return;
        }
        let Some(button) = input::mouse_button(button) else {
            return;
        };
        if !pressed && self.drag_pointer(point, true) {
            return;
        }
        if pressed {
            self.page_pressed(crate::shell::tab_id(pane));
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

    /// A press on a pane (a button, a finger): autoscroll and tips hear it, a glide stops, and
    /// the pane takes the keys and becomes the shown tab.
    fn page_pressed(&mut self, tab: Option<TabId>) {
        if let Some(window) = self.w.window.upgrade() {
            self.autoscroll_input(&window, wubble_core::autoscroll::Input::Pressed);
        }
        self.tip_dismiss();
        self.fling(wubble_core::fling::FlingInput::Interrupt);
        let Some(tab) = tab else {
            return;
        };
        // keys follow the pane that was pressed: the DevTools panel, the peek or a page
        let to = (tab == PANEL_TAB || tab == POPUP_TAB || self.model.is_glance(tab)).then_some(tab);
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

    pub fn pointer_cancelled(&mut self, pane: i32) {
        if self.input.touching {
            return;
        }
        self.input.buttons = Default::default();
        if let Some(view) = self.pane_view(pane) {
            view.capture_lost();
        }
    }

    pub fn scrolled(&mut self, pane: i32, point: Point, dx: f32, dy: f32) {
        self.hints_scrolled();
        let (modifiers, precise) = (self.input.modifiers(), self.input.wheel_precise);
        // A Ctrl (or Cmd) scroll zooms the page: it does not glide on as a scroll.
        let now = self.fling_now();
        self.fling(
            if modifiers.keys.control_key() || modifiers.keys.super_key() {
                wubble_core::fling::FlingInput::Interrupt
            } else {
                wubble_core::fling::FlingInput::Scroll {
                    target: gesture::FlingAt { pane, point },
                    dx: dx.into(),
                    dy: dy.into(),
                    now,
                }
            },
        );
        self.pace(PaceEvent::Wake);
        if let Some(view) = self.pane_view(pane) {
            view.wheel(point, dx, dy, precise, &modifiers);
        }
    }

    pub fn fail(&mut self, reason: String) {
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
        quit();
    }
}

/// Where the window and its monitor are on the desktop. The monitor is the window's, else the
/// primary or the first listed (Wayland names the window's only once its surface has entered an
/// output); the window's position is unknown on Wayland (`inner_position` is unsupported), where
/// the page sees it at the monitor's origin, as in Chrome. The window's place is its content's
/// (`inner_position`), which the views' offsets are measured from: with a native frame (X11) the
/// outer corner would put every view a title bar too high. So `window.screenX/Y` is the content's
/// corner, and winit has no work area, so none is reported.
fn desktop_of(window: &slint::winit_030::winit::window::Window) -> Option<Desktop> {
    let monitor = window
        .current_monitor()
        .or_else(|| window.primary_monitor())
        .or_else(|| window.available_monitors().next())?;
    let (at, size) = (monitor.position(), monitor.size());
    Some(Desktop {
        monitor_origin: (at.x, at.y),
        monitor_size: wubble_gfx::FrameSize::new(size.width, size.height),
        window_origin: window.inner_position().ok().map(|p| (p.x, p.y)),
    })
}

/// What of the shell's state is saved with the session: the sidebar's mode
/// (peeking is a separate property, so a peek saves as hidden) and the seam's
/// saved width (the live width is 0 while the sidebar is hidden).
/// A window's inner size in logical px, its outer position in physical px where the platform
/// tells it (not Wayland), its show state and the name of the monitor it is on. While it is not
/// normal the size and position are the screen's; the model keeps the last normal ones
/// (`Geometry::observed`).
pub fn window_geometry(window: &AppWindow) -> Geometry {
    let win = window.window();
    let size = win.size().to_logical(win.scale_factor());
    let (pos, minimized, monitor) = win
        .with_winit_window(|w| {
            (
                w.outer_position().ok().map(|p| (p.x, p.y)),
                w.is_minimized() == Some(true),
                w.current_monitor().and_then(|m| m.name()),
            )
        })
        .unwrap_or((None, false, None));
    let state = if win.is_fullscreen() {
        WindowState::Fullscreen
    } else if win.is_maximized() {
        WindowState::Maximized
    } else if minimized {
        WindowState::Minimized
    } else {
        WindowState::Normal
    };
    Geometry {
        width: size.width.round() as u32,
        height: size.height.round() as u32,
        pos,
        state,
        monitor,
    }
}

/// What a window does in each show state before it shows (Slint keeps it for the native
/// window) and once its native window exists ([`settle_window`]): one row per state.
const SHOW_STEPS: [(WindowState, ShowBefore, ShowAfter); 4] = [
    (
        WindowState::Normal,
        ShowBefore::Normal,
        ShowAfter::OntoDisplay,
    ),
    (
        WindowState::Maximized,
        ShowBefore::Maximized,
        ShowAfter::RestoredOntoDisplay,
    ),
    (
        WindowState::Fullscreen,
        ShowBefore::Fullscreen,
        ShowAfter::ToMonitor,
    ),
    // winit has no minimized attribute for a window not made yet
    (
        WindowState::Minimized,
        ShowBefore::Normal,
        ShowAfter::OntoDisplayThenMinimize,
    ),
];

#[derive(Clone, Copy)]
enum ShowBefore {
    Normal,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Copy)]
enum ShowAfter {
    /// Where it landed, brought onto a display (`window::visible_on`).
    OntoDisplay,
    /// The same, then minimized.
    OntoDisplayThenMinimize,
    /// Its restored bounds brought onto a display: out of maximized, moved, maximized again.
    RestoredOntoDisplay,
    /// Fullscreen on the monitor it was on, else on the display of its restored bounds.
    ToMonitor,
}

fn show_steps(state: WindowState) -> (ShowBefore, ShowAfter) {
    SHOW_STEPS.iter().find(|(s, _, _)| *s == state).map_or(
        (ShowBefore::Normal, ShowAfter::OntoDisplay),
        |&(_, b, a)| (b, a),
    )
}

/// Puts a window at its saved size and position and in its show state, before it shows
/// ([`SHOW_STEPS`]; the rest once its native window exists, [`settle_when_shown`]).
pub fn place_window(window: &AppWindow, g: &Geometry) {
    let win = window.window();
    win.set_size(slint::LogicalSize::new(g.width as f32, g.height as f32));
    if let Some((x, y)) = g.pos {
        win.set_position(slint::PhysicalPosition::new(x, y));
    }
    match show_steps(g.state).0 {
        ShowBefore::Normal => {}
        ShowBefore::Maximized => win.set_maximized(true),
        ShowBefore::Fullscreen => win.set_fullscreen(true),
    }
}

/// Settles window `id` at `g` once Slint has made its native window (it never does inside
/// `show`: on the event loop's next turn, or later), then, with `raise`, raises it if it is
/// still the focused window. One task per window, holding the window weakly while it waits:
/// one closed meanwhile is let go and holds up no other.
pub fn settle_when_shown(
    id: WindowId,
    window: slint::Weak<AppWindow>,
    g: Option<Geometry>,
    raise: bool,
) {
    if g.is_none() && !raise {
        return;
    }
    let task = slint::spawn_local(async move {
        let created = std::future::poll_fn(|cx| {
            let Some(handle) = window.upgrade() else {
                return std::task::Poll::Ready(None);
            };
            // polled once and dropped: the waker stays with the window, the handle does not
            let mut created = std::pin::pin!(handle.window().winit_window());
            created.as_mut().poll(cx).map(Result::ok)
        })
        .await;
        let Some(w) = created else {
            return;
        };
        if let Some(g) = &g {
            settle_window(&w, g);
        }
        drop(w);
        if raise {
            with_host(move |host| {
                if host.model.focused() == id {
                    host.raise_window(id);
                }
            });
        }
    });
    if let Err(e) = task {
        eprintln!("wubble: window {}: cannot settle it: {e}", id.0);
    }
}

/// Settles a native window at `g`, by its state's row of [`SHOW_STEPS`]: a window off every
/// display is brought back onto one (`window::visible_on`, with Chrome's macOS rule there),
/// in every state; a fullscreen one goes to its monitor (on Wayland the only way to pick its
/// output); a minimized one is minimized.
fn settle_window(w: &slint::winit_030::winit::window::Window, g: &Geometry) {
    use slint::winit_030::winit;
    use wubble_core::window::{self as win, Display, Rect};
    // a monitor reporting no area (being unplugged, or not yet configured) is none
    let monitors: Vec<winit::monitor::MonitorHandle> = w
        .available_monitors()
        .filter(|m| m.size().width > 0 && m.size().height > 0)
        .collect();
    let displays: Vec<Display> = monitors
        .iter()
        .map(|m| Display {
            name: m.name(),
            area: Rect {
                x: m.position().x,
                y: m.position().y,
                width: crate::slint_int(m.size().width),
                height: crate::slint_int(m.size().height),
            },
        })
        .collect();
    // `bounds` brought onto its display, if that moves or shrinks it
    let onto = |bounds: Rect| {
        let d = win::display_for(&bounds, g.monitor.as_deref(), &displays)?;
        let to = win::visible_on(bounds, &d.area, cfg!(target_os = "macos"));
        (to != bounds).then_some(to)
    };
    // where it landed (Wayland tells no position: nothing to bring back there)
    let live = || {
        let at = w.outer_position().ok()?;
        let size = w.outer_size();
        Some(Rect {
            x: at.x,
            y: at.y,
            width: crate::slint_int(size.width),
            height: crate::slint_int(size.height),
        })
    };
    // its restored bounds, in physical px
    let restored = || {
        let (x, y) = g.pos?;
        let px = |v: u32| (f64::from(v) * w.scale_factor()).round() as i32;
        Some(Rect {
            x,
            y,
            width: px(g.width),
            height: px(g.height),
        })
    };
    // moved, and its `inner` size shrunk where `visible_on` shrank it (never grown)
    let move_to = |from: Rect, to: Rect, inner: winit::dpi::PhysicalSize<u32>| {
        w.set_outer_position(winit::dpi::PhysicalPosition::new(to.x, to.y));
        if (to.width, to.height) != (from.width, from.height) {
            let less = |by: i32| u32::try_from(by.max(0)).unwrap_or(0);
            let _ = w.request_inner_size(winit::dpi::PhysicalSize::new(
                inner.width.saturating_sub(less(from.width - to.width)),
                inner.height.saturating_sub(less(from.height - to.height)),
            ));
        }
    };
    let after = show_steps(g.state).1;
    match after {
        ShowAfter::OntoDisplay | ShowAfter::OntoDisplayThenMinimize => {
            if let Some(from) = live()
                && let Some(to) = onto(from)
            {
                move_to(from, to, w.inner_size());
            }
            if matches!(after, ShowAfter::OntoDisplayThenMinimize) {
                w.set_minimized(true);
            }
        }
        ShowAfter::RestoredOntoDisplay => {
            if let Some(from) = restored()
                && let Some(to) = onto(from)
            {
                // the restored bounds are the inner size (the window draws its own frame)
                let inner = winit::dpi::PhysicalSize::new(
                    u32::try_from(from.width).unwrap_or(0),
                    u32::try_from(from.height).unwrap_or(0),
                );
                w.set_maximized(false);
                move_to(from, to, inner);
                w.set_maximized(true);
            }
        }
        ShowAfter::ToMonitor => {
            let named = g
                .monitor
                .as_deref()
                .and_then(|n| monitors.iter().position(|m| m.name().as_deref() == Some(n)));
            let by_bounds = || {
                let d = win::display_for(&restored()?, None, &displays)?;
                displays.iter().position(|x| std::ptr::eq(x, d))
            };
            let Some(m) = named.or_else(by_bounds).map(|i| &monitors[i]) else {
                return;
            };
            if w.current_monitor().as_ref() != Some(m) {
                w.set_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(m.clone()))));
            }
        }
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
/// `--smoke settings`: `left` wheel ticks down over the window's content, through the window's
/// input as the pointer would send them, from where it rests (never moved over what opens
/// there), a tick apart.
fn wheel_burst(window: slint::Weak<AppWindow>, left: u32) {
    let Some(w) = window.upgrade() else {
        return;
    };
    if left == 0 {
        return;
    }
    let size = w.window().size().to_logical(w.window().scale_factor());
    w.window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(size.width * 0.6, size.height / 2.0),
            delta_x: 0.0,
            delta_y: -settings_smoke::TICK,
        });
    slint::Timer::single_shot(settings_smoke::TICK_EVERY, move || {
        wheel_burst(window, left - 1)
    });
}

/// The page a tab switch showed has finished entering and the one it hid has left (the
/// driver's `step_page`): what a smoke checks the pixels of is drawn at rest.
fn page_at_rest(window: &AppWindow) -> bool {
    window.get_page_enter() >= 1.0
        && window.get_exit_t() >= 1.0
        && window.get_exit_page().size().width == 0
}

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
        Command::OpenBeside { url } | Command::ExtensionTab { url, .. } => (None, url),
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

/// Whether a shown page stops painting while its window is hidden: not one playing sound (its
/// media would be suspended), an agent session's (its screenshots) or the one in picture in
/// picture (shown in its own window).
fn hidden_window_stops(audible: bool, agent: bool, pip: bool) -> bool {
    !(audible || agent || pip)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hidden_window_stops_its_pages_but_sound_agents_and_pip() {
        assert!(super::hidden_window_stops(false, false, false));
        assert!(!super::hidden_window_stops(true, false, false));
        assert!(!super::hidden_window_stops(false, true, false));
        assert!(!super::hidden_window_stops(false, false, true));
    }

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
                Effect::ContainerDeleted {
                    id: ContainerId(3),
                    storage: Storage::Disk,
                },
                Call::ForgetContainer(ContainerId(3), Storage::Disk),
            ),
            (
                Effect::ContainerDeleted {
                    id: ContainerId(4),
                    storage: Storage::Memory,
                },
                Call::ForgetContainer(ContainerId(4), Storage::Memory),
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
                | Call::ForgetContainer(..)
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
