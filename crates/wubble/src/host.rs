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
mod downloads;
mod pages;
mod panes;
mod settings;
mod surfaces;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use slint::ComponentHandle;
use slint::platform::PointerEventButton;
use slint::winit_030::winit::event::{ElementState, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use wubble_core::action::Action;
use wubble_core::keys::{Keymap, Platform};
use wubble_core::lifecycle::Effect as Life;
use wubble_core::model::{self, Command, Effect, Model};
use wubble_core::neterror::CertExceptions;
use wubble_core::omnibox::{self, HistoryEntry, SearchEngine, Target};
use wubble_core::pages::InternalPage;
use wubble_core::site::SitePermissions;
use wubble_core::sleep::Pressure;
use wubble_core::space::MAX_PANES;
use wubble_core::tab::{Liveness, TabId};
use wubble_core::zoom::ZoomTable;
use wubble_engine::app::{Engine, PumpOutcome, PumpTimer, RequestMode, ViewId};
use wubble_engine::input::{KeyId, Point};
use wubble_engine::nav::{Cursor, Phase, WebEvent};
use wubble_engine::view::{View, ViewConfig};
use wubble_gfx::FramePresenter;
use wubble_gfx::pacing::{self, BeginFrames, FrameStats, PaceEffect, PaceEvent};
use wubble_store::{ClosedTab, Transition};
use wubble_ui::glass::GlassRegistry;
use wubble_ui::{AppWindow, CommandBarState, GlassKeys};

use crate::backdrop::Glass;
use crate::bar::{self, BarInputs, BarMode, BarRow, Choice, RowTarget};
use crate::block_smoke::{self, BlockSmoke};
use crate::blocking::Blocking;
use crate::ime::Ime;
use crate::input::{self, InputState, KeyFocus, KeyRoute, PendingKeys};
use crate::nav_smoke::{self, NavSmoke, SmokeAction, SmokeInput};
use crate::page_error::ErrorEvent;
use crate::page_smoke::{self, PageSmoke};
use crate::pages_smoke::{self, PagesSmoke};
use crate::panes::Panes;
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
    NAV_PAGE, SMOKE_MAX_IDLE_RUNS, SMOKE_PAGE, TAB3_PAGE, Wake, deliver, display_refresh_mhz,
    page_url, physical, quit, with_host,
};

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
    ShowCommandBar,
    Nothing,
}

/// The engine work an effect stands for. Freeze has no CEF call (CEF 156
/// has no freeze API; hiding already throttles) and snapshots have no UI
/// yet, so those are recorded no-ops (TODO.md "Tabs in the host").
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
            Life::Freeze | Life::Unfreeze | Life::CaptureSnapshot | Life::ReleaseSnapshot => {
                Call::Nothing
            }
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
    pub block_smoke: Option<BlockSmoke>,
    pub split_smoke: Option<SplitSmoke>,
    pub settings_smoke: Option<SettingsSmoke>,
    pub pages_smoke: Option<PagesSmoke>,
    pub autofill_smoke: Option<crate::autofill_smoke::AutofillSmoke>,
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
    window: slint::Weak<AppWindow>,
    /// The engine's single-shot pump timer.
    pub timer: slint::Timer,
    /// One ring of textures per pane on screen (each pane has its own frame size), and the tab
    /// each slot presents; a tab keeps its slot while it stays on screen.
    presenters: [FramePresenter; MAX_PANES],
    slots: [Option<TabId>; MAX_PANES],
    /// The popup widget's frames: a ring of their own, since a popup's size
    /// differs from the page's and changes with every popup.
    popup_presenter: FramePresenter,
    /// The window's glass, shared with the shell's rendering notifier.
    glass: Rc<RefCell<Glass>>,
    /// The glass the panels report (the list's one owner), for the pixel smoke.
    panels: Rc<RefCell<GlassRegistry>>,
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
    /// The last frame of each tab on screen (the `panes` the shell shows).
    frames: BTreeMap<TabId, PaneFrame>,
    /// Where the pages on screen sit: the split physics, stepped per rendered frame.
    panes: Panes,
    /// The previous pane frame's time while the strings or a pane's stretch move.
    panes_last: Option<Instant>,
    /// What the strings settled, waiting to be dispatched outside rendering.
    split_commands: Vec<Command>,
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
    pub settings_passed: bool,
    pub pages_passed: bool,
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
    search_engine: Option<SearchEngine>,
    live_suggestions: bool,
    /// Which commit is a typed visit.
    typed_nav: TypedNav,
    /// Each tab's last committed URL: a commit that only changes the
    /// fragment, or repeats it (a restored tab), is no new visit.
    committed: BTreeMap<TabId, String>,
    /// Why the loop was quit early, if it was.
    pub failure: Option<String>,
    /// The page's own UI per tab (one owner); only the visible tab's shows.
    surfaces: BTreeMap<TabId, Surfaces>,
    /// What of `surfaces` the shell last drew.
    drawn: surfaces::Drawn,
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
    /// Password autofill per tab (`host/autofill.rs`).
    autofill: autofill::Autofill,
    /// The content blocker, whose per-view counts the tab rows show.
    blocking: Arc<Blocking>,
    /// Remembered permission decisions (one owner; stored through the store).
    site_permissions: SitePermissions,
    /// Certificate errors the user proceeded past, for this run only.
    cert_exceptions: CertExceptions,
    downloads: downloads::Downloads,
    pace: Option<Pace>,
}

impl Host {
    /// A host on the loaded session. Only the visible tab gets a view (once
    /// the window exists); the other restored tabs stay Discarded until
    /// activated, through the `Life::Restore` mapping. `open` is loaded in a
    /// new tab, which is then the only one with a view (see [`startup`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: Engine,
        window: &AppWindow,
        presenters: [FramePresenter; MAX_PANES + 1],
        glass: Rc<RefCell<Glass>>,
        panels: Rc<RefCell<GlassRegistry>>,
        loaded: Loaded,
        store: StoreClient,
        open: Option<String>,
        checks: Checks,
        downloads_dir: Option<PathBuf>,
        blocking: Arc<Blocking>,
    ) -> Self {
        let Loaded {
            model,
            settings,
            site_permissions,
            downloads,
            favicons,
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
        let [a, b, c, popup_presenter] = presenters;
        // The shell draws the page's dialogs, menus and prompts.
        let mut engine = engine;
        engine.set_request_mode(RequestMode::AskHost);
        let begin_frames = checks.begin_frames;
        let mut host = Self {
            engine,
            window: window.as_weak(),
            timer: slint::Timer::default(),
            presenters: [a, b, c],
            slots: [None; MAX_PANES],
            popup_presenter,
            glass,
            panels,
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
                generation: 0,
            },
            focus_view: None,
            frames: BTreeMap::new(),
            panes: Panes::default(),
            panes_last: None,
            split_commands: vec![],
            stats: FrameStats::default(),
            last_present: None,
            idle_runs: 0,
            checks,
            nav_title: None,
            tabs_passed: false,
            session_passed: false,
            session_icon: None,
            split_passed: false,
            settings_passed: false,
            pages_passed: false,
            settings_read: None,
            input: InputState::default(),
            // `replay_settings` below applies the stored overrides, engine, zoom, ...
            keymap: Keymap::defaults(Platform::current()),
            page_first: PendingKeys::default(),
            store,
            save: SaveState::default(),
            search_engine: None,
            live_suggestions: false,
            typed_nav: TypedNav::default(),
            committed,
            failure: None,
            surfaces: BTreeMap::new(),
            drawn: Default::default(),
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
            autofill: autofill::Autofill::new(),
            blocking,
            site_permissions,
            cert_exceptions: CertExceptions::default(),
            downloads: downloads::Downloads::new(downloads_dir, downloads),
            pace: begin_frames.then(|| Pace {
                state: BeginFrames::Off,
                timer: slint::Timer::default(),
            }),
        };
        host.replay_settings();
        host.render_downloads(window);
        host.session_icon_setup();
        match open {
            Some(url) => host.dispatch(new_tab(url, None)),
            None => host.after_change(),
        }
        host
    }

    /// The only way tabs change: apply, then execute the effects.
    pub fn dispatch(&mut self, command: Command) {
        if persist::changes_session(&command) {
            self.save = self.save.changed();
        }
        if let Command::CloseTab(tab) = command {
            self.remember_closed(tab);
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
            let c = call(&effect, &self.model);
            follow.extend(self.execute(c, frame_rate));
        }
        follow
    }

    /// The only place effects touch the engine or open the bar.
    fn execute(&mut self, c: Call, frame_rate: u32) -> Option<Command> {
        match c {
            Call::OpenView { tab, url } => {
                let window = self.window.upgrade()?;
                if self.views.contains_key(&tab) {
                    return None;
                }
                let config = ViewConfig {
                    url,
                    layout: self.pane_layout(&window, tab),
                    frame_rate,
                    begin_frames: self.pace.is_some(),
                    gpu: self.presenters[0].context().clone(),
                };
                match self.engine.open_view(config) {
                    Ok(id) => {
                        self.views.insert(tab, id);
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
            // Later, outside the host: opening the bar calls back into it.
            Call::ShowCommandBar => {
                let _ = self
                    .window
                    .upgrade_in_event_loop(|window| window.invoke_request_new_tab());
            }
            Call::Nothing => {}
        }
        None
    }

    /// Re-derives the shell from the model.
    fn after_change(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        // Another tab's frame (and cursor) must not stay on screen while
        // input already goes to the tabs shown now: blank until they paint.
        let kept = self.framed_tabs();
        let before = self.frames.len();
        self.frames.retain(|tab, _| kept.contains(tab));
        if self.frames.len() != before {
            window.set_web_cursor(Cursor::Default as i32);
        }
        self.settle_panes(&window);
        self.sync_focus(&window);
        let tabs = &self.model.tabs;
        self.favicons.retain(|tab, _| tabs.contains_key(tab));
        let blocked = self.blocked_counts();
        crate::bind::render(
            &window,
            &crate::shell::view_model(&self.model, &self.audio, &blocked),
            &self.favicons,
        );
        self.render_surfaces(&window);
        let page = self.view().map(|view| view.nav().page.clone());
        crate::bind::render_history(&window, page.as_ref());
        self.render_pages();
    }

    /// The blocker's count for `tab`, if it has a view.
    fn blocked_of(&mut self, tab: TabId) -> Option<u32> {
        let id = *self.views.get(&tab)?;
        let browser = self.engine.view(id)?.browser_id();
        Some(self.blocking.blocked(browser))
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
        let id = *self.views.get(&tab)?;
        self.engine.view(id)
    }

    /// Tells the model whether the tab is busy: its page is loading, or it
    /// holds a download the engine runs (which a discard would end).
    fn report_busy(&mut self, tab: TabId) {
        let loading = self
            .tab_view(tab)
            .is_some_and(|v| matches!(v.nav().phase, Phase::Loading { .. }));
        let busy = loading || self.downloads.holds(tab);
        self.dispatch(Command::BusyChanged { tab, busy });
    }

    /// The visible tab's view: the focused page, the one that gets keys.
    pub(crate) fn view(&mut self) -> Option<&mut View> {
        self.tab_view(self.model.visible_tab()?)
    }

    /// The visible tab's view, else any live tab's: for work that needs a view but none in
    /// particular (a retried download started from an internal page).
    fn any_view(&mut self) -> Option<&mut View> {
        let tab = self
            .model
            .visible_tab()
            .filter(|t| self.views.contains_key(t))
            .or_else(|| self.views.keys().next().copied())?;
        self.tab_view(tab)
    }

    fn tab_of(&self, view: ViewId) -> Option<TabId> {
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
        let Some(window) = self.window.upgrade() else {
            return;
        };
        for (view, event) in events {
            self.on_web_event(&window, view, event);
        }
        for (view, event) in self.engine.take_page_events() {
            self.on_page_event(&window, view, event);
        }
        self.close_tabs_of_gone_views();
        self.smoke_step(SmokeInput::Worked);
        self.tabs_smoke_step();
        self.session_smoke_step();
        self.settings_smoke_step();
        self.pages_smoke_step();
        if let Err(e) = self.show_frame(&window) {
            self.fail(e);
        }
        self.split_smoke_step(&window);
        self.autofill_smoke_worked();
        let framed = self
            .model
            .visible_tab()
            .is_some_and(|t| self.frames.contains_key(&t));
        let popup_framed = window.get_popup_frame().size().width > 0;
        if self.checks.block_smoke.is_some() {
            let blocked = self.visible_blocked();
            self.block_smoke_step(block_smoke::Input::Worked { blocked });
        }
        if self.checks.page_smoke.is_some() {
            let view_width = self.model.visible_tab().map_or(0.0, |t| {
                self.pane_layout(&window, t).size.width as f32 / window.window().scale_factor()
            });
            self.page_smoke_step(page_smoke::Input::Worked {
                framed,
                popup_framed,
                view_width,
            });
        }
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
        if !self.present_panes(window)? {
            return Ok(());
        }
        let now = Instant::now();
        self.stats
            .record_accelerated(self.last_present.map(|last| now - last));
        self.last_present = Some(now);
        self.idle_runs = 0;
        if self
            .checks
            .quit_after
            .is_some_and(|n| self.stats.accelerated >= n)
        {
            if let Some(tab) = self.model.visible_tab()
                && let Some(frame) = self.frames.get(&tab)
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

    /// `--smoke frames`: with the scene, the sidebar's glass must show something (the
    /// refracted, moving backdrop), not a flat fill, and be the scene's current frame.
    fn check_glass(&self, window: &AppWindow, shot: &Snapshot) -> Result<(), String> {
        if !self.glass.borrow().is_scene() {
            println!("smoke: glass fallback");
            return Ok(());
        }
        let key = window.global::<GlassKeys>().get_sidebar();
        let side = self
            .panels
            .borrow()
            .get(key)
            .cloned()
            .ok_or("the sidebar's glass never reached the scene")?;
        // The band below the tab list and above the Space switcher: glass, no text.
        let scale = window.window().scale_factor();
        let (x, w) = (side.x + side.radius, side.width - 2.0 * side.radius);
        let (y, h) = (side.y + side.height * 0.45, side.height * 0.25);
        let region = pixels::Region {
            x: physical(x, scale),
            y: physical(y, scale),
            width: physical(w, scale),
            height: physical(h, scale),
        };
        let sample = pixels::check_varied(shot, region)?;
        // What the window shows there is the scene's frame of this render, not an older one.
        let (_, _, scene) = self
            .glass
            .borrow()
            .readback()
            .ok_or("the scene has no frame")?;
        let diff = pixels::check_shows_scene(shot, &scene, region)?;
        println!("smoke: glass ok distinct={} diff={diff}", sample.distinct);
        Ok(())
    }

    fn on_web_event(&mut self, window: &AppWindow, view: ViewId, event: WebEvent) {
        // Events of a view whose tab is already closed are dropped.
        let Some(tab) = self.tab_of(view) else {
            return;
        };
        self.smoke_step(SmokeInput::Event(&event));
        self.page_smoke_step(page_smoke::Input::Web(&event));
        self.block_smoke_step(block_smoke::Input::Web(&event));
        match event {
            WebEvent::CursorChanged(cursor) => {
                // A page beside the focused one shows its cursor too: the pointer is over it.
                if !self.shown_tabs().contains(&tab) {
                    return;
                }
                window.set_web_cursor(cursor as i32);
                // Slint applies a TouchArea's cursor only while it handles a
                // mouse event (i-slint-core 1.19 `window.rs`, after
                // `process_mouse_input`), and CEF reports the shape after
                // the move that caused it. Replay the pointer where it is,
                // once the host is free again, so the shape shows without
                // waiting for the mouse to move.
                if let Some(position) = self.input.pointer {
                    let _ = self.window.upgrade_in_event_loop(move |window| {
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::PointerMoved { position })
                    });
                }
            }
            WebEvent::UrlChanged(url) => {
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
            WebEvent::PopupRequested { url } => self.dispatch(new_tab(url, Some(tab))),
            WebEvent::LoadStarted => self.apply_error(tab, ErrorEvent::LoadStarted),
            WebEvent::LoadingChanged { loading, .. } => {
                if !loading {
                    self.typed_nav.ended(tab);
                }
                self.report_busy(tab);
            }
            WebEvent::RendererGone => {
                self.apply_error(tab, ErrorEvent::Crashed);
                self.dispatch(Command::RendererGone(tab));
            }
            WebEvent::KeyUnhandled(key) => {
                if Some(tab) == self.model.visible_tab()
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

    // ----- command bar

    /// Ctrl+T or the new-tab row: the bar is opening on an empty query.
    pub fn bar_for_new_tab(&mut self, window: &AppWindow) {
        self.open_bar(window, BarMode::NewTab, String::new());
    }

    /// Ctrl+L or the address pill: the bar opens on the visible tab's URL.
    pub fn bar_for_address(&mut self, window: &AppWindow) {
        let visible = self.model.visible_tab();
        match visible.and_then(|t| self.model.tabs.get(&t)) {
            Some(tab) => {
                let url = tab.url.clone();
                self.open_bar(window, BarMode::Navigate(tab.id), url);
            }
            None => self.bar_for_new_tab(window),
        }
    }

    fn open_bar(&mut self, window: &AppWindow, mode: BarMode, query: String) {
        self.bar.mode = mode;
        window.set_command_query(query.as_str().into());
        self.bar_query(window, &query);
    }

    /// Renders the rows for `query` at once from what is at hand, and asks
    /// the store for its history hits, which re-render when they arrive.
    pub fn bar_query(&mut self, window: &AppWindow, query: &str) {
        self.bar.query = query.into();
        self.bar.generation += 1;
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
        if let Some(window) = self.window.upgrade() {
            self.render_bar(&window);
        }
    }

    fn render_bar(&mut self, window: &AppWindow) {
        let tabs = bar::open_tabs(&self.model);
        let suggestions = bar::live_suggestions(self.live_suggestions, &self.bar.query);
        let inputs = BarInputs {
            tabs: &tabs,
            history: &self.bar.history,
            engine: self.search_engine,
            keymap: &self.keymap,
            suggestions: &suggestions,
            now_ms: now_ms(),
        };
        self.bar.rows = bar::bar_rows(&inputs, &self.bar.query);
        crate::bind::render_bar(window, &self.bar.rows, &self.bar.icons);
    }

    pub fn bar_chosen(&mut self, index: i32) {
        let row = usize::try_from(index)
            .ok()
            .and_then(|i| self.bar.rows.get(i));
        if let Some(row) = row {
            let target = row.target.clone();
            self.choose(&target);
        }
    }

    fn choose(&mut self, target: &RowTarget) {
        match bar::target_command(target, self.bar.mode, self.search_engine) {
            Some(Choice::Model(command)) => {
                self.dispatch(command);
                if bar::is_typed(target) {
                    self.typed_nav.started(self.model.visible_tab());
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
                if let Some(tab) = self.model.visible_tab() {
                    self.typed_nav.ended(tab);
                }
                let Some(tab) = self.model.visible_tab() else {
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
                }
                if matches!(op, ViewOp::ZoomIn | ViewOp::ZoomOut | ViewOp::ZoomReset) {
                    let percent = view.zoom_percent();
                    self.user_zoomed(tab, percent);
                }
            }
            Plan::Print => {
                if let Some(tab) = self.model.visible_tab() {
                    self.print(tab);
                }
            }
            Plan::Find(event) => self.find_event(event),
            Plan::Fullscreen => {
                if let Some(window) = self.window.upgrade() {
                    self.apply_screen(&window, ScreenEvent::Toggle);
                }
            }
            Plan::Copy(text) => self.copy(&text),
            Plan::BookmarkPage => self.bookmark_page(),
            Plan::OpenBeside => self.open_beside(),
            // Later, outside the host: opening the bar calls back into it.
            Plan::FocusAddress => {
                let _ = self
                    .window
                    .upgrade_in_event_loop(|window| window.invoke_request_address_focus());
            }
            Plan::ReopenClosed => self.store.pop_closed(|closed| {
                if let Some(closed) = closed {
                    let _ = slint::invoke_from_event_loop(move || {
                        with_host(|host| host.reopen(closed));
                    });
                }
            }),
            Plan::ToggleSidebar => {
                if let Some(window) = self.window.upgrade() {
                    window.invoke_toggle_sidebar();
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
    fn reopen(&mut self, closed: ClosedTab) {
        let container = self
            .model
            .containers
            .iter()
            .any(|c| c.id == closed.container)
            .then_some(closed.container);
        self.dispatch(Command::NewTab {
            url: closed.url,
            container,
            opener: None,
        });
    }

    // ----- persistence

    /// Called before a tab closes: it goes on the recently-closed list.
    fn remember_closed(&mut self, tab: TabId) {
        self.committed.remove(&tab);
        self.typed_nav.ended(tab);
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
        if let Some(window) = self.window.upgrade() {
            self.store.save(&self.model, ui_state(&window));
            self.save = SaveState::default();
        }
    }

    /// Orderly shutdown: the final save, then the store thread finishes and
    /// closes the database. `ui` was read before the window went away.
    pub fn shutdown(&mut self, ui: UiState) {
        self.save_download_progress();
        self.store.save(&self.model, ui);
        std::mem::replace(&mut self.store, StoreClient::none()).shutdown();
    }

    /// The coarse lifecycle tick; also the save policy's idle clock.
    pub fn tick(&mut self, now: Instant) {
        self.follow_frame_rate();
        self.dispatch(Command::Tick {
            now,
            pressure: Pressure::None,
        });
        self.save = self.save.ticked();
        if persist::should_save(self.save) {
            self.save_now();
        }
        self.save_download_progress();
        self.passwords_tick(now);
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
                if let Some(tab) = self.model.visible_tab() {
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

    fn seen(&self) -> Seen {
        let order = self.model.order(self.model.active_space);
        let shown = self
            .model
            .visible_tab()
            .and_then(|t| self.views.get(&t))
            .is_some_and(|&id| self.engine.is_visible(id));
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
                self.frames.keys().any(|t| !kept.contains(t))
            },
        }
    }

    fn tabs_smoke_step(&mut self) {
        let Some(state) = self.checks.tabs_smoke.take() else {
            return;
        };
        let want =
            crate::TABS_SMOKE_PAGES.map(|page| nav_smoke::title_of(page).unwrap_or_default());
        let (state, action) = tabs_smoke::next(state, &self.seen(), &want);
        self.checks.tabs_smoke = Some(state);
        let nth = |host: &Self, i: usize| host.model.order(host.model.active_space).get(i).copied();
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
        let order = self.model.order(self.model.active_space);
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
            at_rest: self.panes_last.is_none()
                && self.split_commands.is_empty()
                && !self.panes.held()
                && self.frames.values().all(|f| f.view.is_live()),
            masked: self.frames.values().filter(|f| f.masked.is_some()).count(),
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
                match self.panes.seams(height.into()).first() {
                    Some(&(x, _)) => {
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
                if let Some(tab) = self.model.visible_tab() {
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
        let Some(window) = self.window.upgrade() else {
            self.checks.settings_smoke = Some(state);
            return;
        };
        let seen = settings_smoke::Seen {
            ready: self.frame_rate.is_some(),
            on_settings: self
                .model
                .visible_tab()
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
                let _ = self.window.upgrade_in_event_loop(move |window| {
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
        let Some(window) = self.window.upgrade() else {
            self.checks.pages_smoke = Some(state);
            return;
        };
        let page = self.shown_page();
        let rows = match page {
            Some(InternalPage::History) => window.get_history_lines().row_count(),
            Some(InternalPage::Bookmarks) => window.get_bookmark_lines().row_count(),
            Some(InternalPage::Downloads) => window.get_downloads().row_count(),
            Some(InternalPage::Passwords) => window.get_logins().row_count(),
            Some(InternalPage::Settings) | None => 0,
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
                InternalPage::Settings => Action::OpenSettings,
            }),
            // The page's own button callback, as a click sends it: later, outside the host.
            Some(pages_smoke::Action::Unlock) => {
                let _ = self
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
        let tab = self.model.visible_tab();
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
                        if let Some(window) = self.window.upgrade() {
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
                        .model
                        .visible_tab()
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
        let order = self.model.order(self.model.active_space);
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
            visible: order
                .iter()
                .position(|&t| Some(t) == self.model.visible_tab()),
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
        let target = self
            .model
            .visible_tab()
            .and_then(|t| self.views.get(&t))
            .copied();
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
            .and(self.model.visible_tab())
            .and_then(|t| self.surfaces.get(&t))
            .is_some_and(|s| s.menu.keeps_page_focus());
        let focused = self.input.window_focused && (window.get_web_focused() || page_menu);
        let view_open = self.view().is_some();
        if let Some(focused) = self.input.focus_change(focused, view_open)
            && let Some(view) = self.view()
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
        let Some(window) = self.window.upgrade() else {
            return EventResult::Propagate;
        };
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.input.keys = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let position = position.to_logical(window.window().scale_factor().into());
                self.input.pointer = Some(slint::LogicalPosition::new(position.x, position.y));
                self.glass.borrow_mut().pointer_move(position.x, position.y);
            }
            WindowEvent::CursorLeft { .. } => {
                self.input.pointer = None;
                self.glass.borrow_mut().pointer_leave();
            }
            // A left press anywhere sends a wave through the scene (the page gets it too). Raw,
            // so the side buttons navigate over the shell too, without a wave.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                if input::taps_backdrop(*button) {
                    self.glass.borrow_mut().tap();
                }
                if let Some(action) = input::button_action(*button) {
                    self.perform(action);
                }
            }
            WindowEvent::Focused(focused) => {
                self.input.window_focused = *focused;
                self.sync_focus(&window);
            }
            WindowEvent::Ime(ime) => {
                if self.ime_event(&window, ime) {
                    return EventResult::PreventDefault;
                }
            }
            // Esc leaves page fullscreen before anything else sees it.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } if event.state == ElementState::Pressed
                && event.logical_key == Key::Named(NamedKey::Escape)
                && matches!(self.screen, Screen::Page { .. }) =>
            {
                self.apply_screen(&window, ScreenEvent::Escape);
                return EventResult::PreventDefault;
            }
            // Synthetic presses (keys already held when the window gained
            // focus, X11) never reach the page.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let key = self.input.key_input(event);
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
                    .then(|| {
                        input::chord_of(&key.unmodified, key.modifiers.keys, key.text.as_deref())
                    })
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
                    if let Some(view) = self.view() {
                        view.key(&key);
                    }
                }
                match route {
                    KeyRoute::Perform(action) => {
                        self.perform(action);
                        return EventResult::PreventDefault;
                    }
                    // Runs on the view's `KeyUnhandled`, if the page sends it.
                    KeyRoute::PageFirst(action) => {
                        if let Some(id) = KeyId::of(&key) {
                            self.page_first.push(id, action);
                        }
                        return EventResult::PreventDefault;
                    }
                    KeyRoute::Page => return EventResult::PreventDefault,
                    KeyRoute::PageAndShell | KeyRoute::Shell => {}
                }
            }
            _ => {}
        }
        EventResult::Propagate
    }

    /// Tells every view the frame rate if the window is now on a display
    /// with another refresh rate. Polled from the lifecycle tick: winit has
    /// no event for a move to another monitor at the same scale, and on
    /// Wayland the first rate comes from the fastest monitor because the
    /// surface has not entered an output yet.
    fn follow_frame_rate(&mut self) {
        let Some(window) = self.window.upgrade() else {
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
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        self.pace(PaceEvent::Wake);
        if let Some(view) = self.pane_view(pane) {
            view.mouse_move(point, &modifiers, now);
        }
    }

    pub fn pointer_left(&mut self, pane: i32, point: Point) {
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        self.pace(PaceEvent::Wake);
        if let Some(view) = self.pane_view(pane) {
            view.mouse_leave(point, &modifiers, now);
        }
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
        if pressed
            && let Some(tab) = crate::shell::tab_id(pane)
            && self.model.visible_tab() != Some(tab)
            && self.model.visible_tabs().contains(&tab)
        {
            self.dispatch(Command::ActivateTab(tab));
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
            (Life::Freeze, Call::Nothing),
            (Life::Unfreeze, Call::Nothing),
            (Life::CaptureSnapshot, Call::Nothing),
            (Life::ReleaseSnapshot, Call::Nothing),
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
                Call::Load { .. } | Call::ShowCommandBar | Call::Nothing => {}
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
        let order = m.order(m.active_space);
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
