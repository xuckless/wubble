//! One windowless browser: frames, resize, HiDPI scale, close, navigation
//! and input.
//!
//! The lifecycle is the pure [`transition`], navigation the pure
//! [`nav::transition`]; [`View`] wraps welding's `PlatformCefProducer` (which
//! owns the CEF render handler and the dmabuf custody) and runs the effects.
//! Input goes straight to the view's CEF `BrowserHost`, translated by
//! [`crate::input`]. Views are owned by [`crate::app::Engine`], which closes
//! them on shutdown and reports each one closed to its lifecycle.
//!
//! Everything else a page asks or tells (dialogs, menus, downloads,
//! permissions, find, zoom, fullscreen, mute, IME, drags) is translated by
//! the pure [`route`] into [`PageEvent`]s, through the state machines in
//! `popup`, `download`, `find`, `media`, `zoom` and `drag`; the host answers
//! requests with the `answer_*` methods. The handler wiring follows CEF's
//! cefclient (`tests/cefclient/browser/client_handler.cc`, BSD-3-Clause),
//! through the welding fork.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cef::{ImplBrowser, ImplBrowserHost, ImplFrame, ImplPreferenceManager, ImplValue};
use welding::{
    CefRuntime, CefSurfaceEvent, CefSurfaceProducer, HostWgpuContext, NavigationEvent,
    PlatformCefProducer, WeldError,
};
use winit::keyboard::PhysicalKey;
use wubble_gfx::pacing::{self, ContentEvent, ContentRate};
use wubble_gfx::{FrameSize, NativeFrame};

use crate::app::{EngineError, RequestMode};
use crate::cdp::{self, CdpId, EvalId};
use crate::download::{self, Download, DownloadEffect, DownloadInput};
use crate::drag::{self, DragSource, DragSourceInput};
use crate::filter::{ContentFilter, ResponseObserver};
use crate::find::{self, Find, FindEffect, FindInput};
use crate::input::{
    self, CefKey, ClickTracker, KeyId, KeyInput, Modifiers, MouseButton, Point, WheelRemainder,
};
use crate::media::{self, AudioProbe, Fullscreen, FullscreenInput, MediaEffect, ProbeInput};
use crate::nav::{self, NavEffect, NavInput, NavState, WebEvent};
use crate::page::{
    ClipboardRequest, ContextMenu, DownloadId, DragData, DragOps, EditCommand, EditFlags,
    FrameChannel, LogLevel, MediaFlags, PageEvent, PasteFrom, PermissionKind, Rect, RequestId,
};
use crate::pinch::{self, Pinch, TouchKind};
use crate::popup::{self, PopupWidget};
use crate::zoom::{self, Zoom};

/// Where a view sits in the host window (both physical pixels), and where the window and its
/// monitor are on the desktop: what the page reads as `screen.*`, `window.screenX/Y`,
/// `outerWidth` and `MouseEvent.screenX/Y`, and what Blink places popups such as `<select>`
/// lists against; see welding's `ScreenArea`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenArea {
    pub window: FrameSize,
    /// The view's top-left corner in the window.
    pub origin: (i32, i32),
    /// `None` until the host knows (the window is then the screen, at 0,0).
    pub desktop: Option<Desktop>,
}

/// The window's monitor and the window on the desktop, physical pixels in the platform's
/// global space (winit's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Desktop {
    pub monitor_origin: (i32, i32),
    pub monitor_size: FrameSize,
    /// `None` where the platform will not say (Wayland): the window is taken at the monitor's
    /// origin, as Chrome reports it there.
    pub window_origin: Option<(i32, i32)>,
}

impl ScreenArea {
    fn to_welding(self) -> welding::ScreenArea {
        welding::ScreenArea {
            window: dpi_size(self.window),
            origin: self.origin,
            desktop: self.desktop.map(|d| welding::Desktop {
                monitor_origin: d.monitor_origin,
                monitor_size: dpi_size(d.monitor_size),
                window_origin: d.window_origin,
            }),
        }
    }
}

/// Physical size and device scale of the rectangle a view fills, and where
/// it sits in the window (`None` when the host does not say).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub size: FrameSize,
    pub scale: f32,
    pub area: Option<ScreenArea>,
}

impl Layout {
    /// The layout CEF can use: at least 1x1 pixels.
    fn clamped(self) -> Self {
        Self {
            size: self.size.at_least_one(),
            ..self
        }
    }

    /// The physical size CEF paints for this layout, which can exceed `size`
    /// by a pixel at fractional scales or odd sizes. welding reports the view
    /// to CEF in DIP as `round(size / scale)` (welding 0.15 `ViewMetrics::
    /// logical`), and Chromium's OSR view paints `ScaleToCeiledSize(dip,
    /// scale)` (CEF `RenderWidgetHostViewOSR::SizeInPixels`). Same f32 math.
    pub fn frame_size(self) -> FrameSize {
        let axis = |physical: u32| {
            let dip = (physical as f32 / self.scale).round().max(1.0);
            (dip * self.scale).ceil() as u32
        };
        FrameSize::new(axis(self.size.width), axis(self.size.height))
    }
}

/// Where a view is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewState {
    /// CEF paints at `layout`.
    Live {
        layout: Layout,
    },
    /// CEF was told about `wanted`; frames at the old size may still arrive.
    Resizing {
        wanted: Layout,
    },
    /// Close was requested; waiting for CEF to drop the browser.
    Closing,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewEvent {
    /// The host's content rectangle or scale factor (possibly unchanged).
    Layout(Layout),
    /// CEF delivered a frame of this physical size.
    Frame(FrameSize),
    CloseRequested,
    /// CEF destroyed the browser (`OnBeforeClose`), asked to or not.
    GoneFromCef,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewEffect {
    Resize(FrameSize),
    SetScale(f32),
    SetScreenArea(ScreenArea),
    /// Hand the frame to the host.
    Present,
    Close,
    /// Tell the engine lifecycle this browser is gone.
    ReportClosed,
}

/// The layout CEF is painting at or was last asked for.
fn target(state: ViewState) -> Option<Layout> {
    match state {
        ViewState::Live { layout } => Some(layout),
        ViewState::Resizing { wanted } => Some(wanted),
        ViewState::Closing | ViewState::Closed => None,
    }
}

/// Every (state, event) pair not matched is a no-op.
pub fn transition(state: ViewState, event: ViewEvent) -> (ViewState, Vec<ViewEffect>) {
    use {ViewEffect::*, ViewEvent::*, ViewState::*};
    match (state, event, target(state)) {
        (Live { .. } | Resizing { .. }, Layout(new), Some(current)) => {
            let new = new.clamped();
            if new == current {
                return (state, vec![]);
            }
            let mut effects = vec![];
            if new.size != current.size {
                effects.push(Resize(new.size));
            }
            if new.scale != current.scale {
                effects.push(SetScale(new.scale));
            }
            if let Some(area) = new.area.filter(|_| new.area != current.area) {
                effects.push(SetScreenArea(area));
            }
            // Moving the view without resizing it needs no frame at a new size: it stays Live.
            let resized = new.size != current.size || new.scale != current.scale;
            match state {
                Live { .. } if !resized => (Live { layout: new }, effects),
                _ => (Resizing { wanted: new }, effects),
            }
        }
        // Frames at a stale size are still shown (at their own size, top left) rather than
        // dropped, so a resize never flashes an empty view.
        (Live { .. }, Frame(_), _) => (state, vec![Present]),
        (Resizing { wanted }, Frame(size), _) if size == wanted.frame_size() => {
            (Live { layout: wanted }, vec![Present])
        }
        (Resizing { .. }, Frame(_), _) => (state, vec![Present]),
        (Live { .. } | Resizing { .. }, CloseRequested, _) => (Closing, vec![Close]),
        // From Live or Resizing the page closed itself (`window.close()`).
        (Live { .. } | Resizing { .. } | Closing, GoneFromCef, _) => (Closed, vec![ReportClosed]),
        _ => (state, vec![]),
    }
}

/// What a new view starts with.
#[derive(Clone)]
pub struct ViewConfig {
    pub url: String,
    pub layout: Layout,
    /// CEF `windowless_frame_rate`; see `wubble_gfx::pacing::frame_rate_for`.
    pub frame_rate: u32,
    /// CEF paints only on [`View::send_begin_frame`] (external begin
    /// frames) instead of on its own timer at `frame_rate`.
    pub begin_frames: bool,
    /// The device frames are shown on. On Linux and macOS welding copies
    /// each frame onto it inside CEF's paint callback, before CEF reuses the
    /// buffer.
    pub gpu: HostWgpuContext,
    /// Whose cookies, storage and cache the view uses (`crate::storage`):
    /// the tab's container.
    pub container: wubble_core::space::ContainerId,
    /// Where that container keeps them: the engine opens its context from
    /// this value, and refuses a context already open with the other one.
    pub storage: wubble_core::space::Storage,
}

/// One live windowless browser.
pub struct View {
    producer: PlatformCefProducer,
    /// Saved at creation because welding zeroes its own copy on close.
    browser_id: i32,
    state: ViewState,
    /// What CEF was last told (`was_hidden`). Hidden views hand out no frames.
    visible: bool,
    nav: NavState,
    /// Emitted by navigation effects, handed out by [`Self::drain_events`].
    events: Vec<WebEvent>,
    /// Looked up on first input; CEF's own handle, released with the view.
    host: Option<cef::BrowserHost>,
    clicks: ClickTracker,
    wheel: WheelRemainder,
    /// A trackpad pinch over the page (`pinch.rs`).
    pinch: Pinch,
    /// The page's root horizontal scroll offset (DIP), as CEF last reported it.
    scroll_x: f64,
    /// The accessibility tree wanted ([`View::set_accessibility`]), and whether CEF has been
    /// told: the browser host may not exist yet when it is asked.
    accessibility: (bool, bool),
    /// KEYUPs owed for keys pressed and not yet released, sent if focus
    /// leaves first so no key sticks.
    held_keys: Vec<(PhysicalKey, CefKey)>,
    /// Emitted page events, handed out by [`Self::drain_events`].
    page: Vec<PageEvent>,
    popup: PopupWidget,
    /// Downloads not yet ended.
    downloads: HashMap<DownloadId, Download>,
    find: Find,
    fullscreen: Fullscreen,
    muted: bool,
    audio: AudioProbe,
    /// Freeze requests awaiting their CDP reply.
    pending: crate::cdp::Pending,
    zoom: Zoom,
    drag: DragSource,
    /// Asked for a page script on each main-frame commit.
    filter: Option<Arc<dyn ContentFilter>>,
    /// Icon downloads CEF finished, moved to `page` by [`Self::drain_events`].
    favicons: crate::favicon::Sink,
    /// The scripts every new document runs first ([`Self::set_document_scripts`]).
    doc_scripts: cdp::DocScripts,
    /// The navigations the page asked for, from its DevTools events.
    nav_requests: cdp::NavRequests,
    /// The client DevTools windows are created with: it reports them to the
    /// engine (`app::extra`), which must close them before `cef::shutdown`.
    devtools_client: cef::Client,
    /// Where the page may go ([`View::set_navigation_rule`]), read by welding's guard.
    rule: NavigationRule,
    /// Which windows the page opens become CEF browsers ([`View::allow_popup_browsers`]).
    popups: Popups,
    /// Pop-up browsers adopted as views, until the engine takes them ([`View::take_opened`]).
    opened: Vec<(View, crate::nav::PopupRequest)>,
    /// How lively the page is, and the frame-rate ceiling the host asked for: CEF is told
    /// `content.frame_rate(ceiling)` ([`Self::content`], [`Self::set_frame_rate`]).
    content: ContentRate,
    ceiling: u32,
    /// The begin frames offered so far ([`pacing::begin_frame_due`]).
    begin_ticks: u32,
}

/// A view's pop-up switch and, by CEF's pop-up id, the hooks its factory made for each pop-up
/// CEF is creating: shared with its `welding::PopupFactory`, read on CEF's UI thread.
#[derive(Clone, Default)]
struct Popups {
    on: Arc<std::sync::atomic::AtomicBool>,
    pending: Arc<std::sync::Mutex<HashMap<i32, Pending>>>,
}

/// A pop-up CEF is creating: its navigation rule and pop-up switch, and its opener's layout
/// when the factory answered.
struct Pending {
    rule: NavigationRule,
    popups: Popups,
    layout: Layout,
}

/// What a view's pop-ups inherit from it, for its `welding::PopupFactory`.
#[derive(Clone)]
struct PopupHooks {
    layout: Layout,
    pacing: (u32, bool),
    request_filter: Option<welding::RequestFilter>,
    response_observer: Option<welding::ResponseObserver>,
}

/// Where a view's page may go: a host's rule over a top-level navigation's URL, or none
/// (anywhere). Shared with the view's `welding::NavigationGuard`, which reads it on CEF's UI
/// thread before each navigation is sent.
pub type Rule = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type NavigationRule = Arc<std::sync::RwLock<Option<Rule>>>;

/// The second frame stream: the popup widget's newest frame and where to
/// draw it (physical pixels, relative to the view; may leave the view).
pub struct PopupLayer {
    pub frame: NativeFrame,
    pub rect: Rect,
}

/// A page drag's data as CEF holds it: opaque, equal only to itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldDrag(welding::HeldDrag);

/// Where a drag over the view is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// The drag entered, carrying what is dragged.
    Enter(DragData),
    /// The view's own page drag entered it again: CEF gets the data the page
    /// gave, with the types [`DragData`] does not carry.
    EnterSource,
    /// Another view's page drag entered this view, with that page's data as
    /// it gave it ([`View::held_drag`]).
    EnterFrom(HeldDrag),
    Over,
    Leave,
    Drop,
}

impl View {
    pub(crate) fn open(
        runtime: &CefRuntime,
        config: ViewConfig,
        profile: welding::SharedProfile,
        filter: Option<Arc<dyn ContentFilter>>,
        observer: Option<Arc<dyn ResponseObserver>>,
        devtools_client: cef::Client,
    ) -> Result<Self, EngineError> {
        let layout = config.layout.clamped();
        let hooks = PopupHooks {
            layout,
            pacing: (config.frame_rate, config.begin_frames),
            request_filter: filter.clone().map(crate::filter::request_filter),
            response_observer: observer.map(crate::filter::response_observer),
        };
        let rule: NavigationRule = Arc::default();
        let popups = Popups::default();
        let mut surface =
            surface_config(&hooks, guard(&rule), popup_factory(&hooks, &rule, &popups));
        surface.initial_url = config.url;
        surface.profile = Some(profile);
        let mut producer = new_producer(runtime, surface, &config.gpu)?;
        if let Some(area) = layout.area {
            // The creation config has no screen area; CEF reads it later.
            if let Err(e) = producer.set_screen_area(area.to_welding()) {
                eprintln!("wubble-engine: initial screen area failed: {e}");
            }
        }
        Ok(Self::with_producer(
            producer,
            layout,
            filter,
            (rule, popups),
            devtools_client,
            config.frame_rate,
        ))
    }

    /// A view of `producer`'s browser: one the engine created, or a pop-up CEF created.
    fn with_producer(
        producer: PlatformCefProducer,
        layout: Layout,
        filter: Option<Arc<dyn ContentFilter>>,
        (rule, popups): (NavigationRule, Popups),
        devtools_client: cef::Client,
        ceiling: u32,
    ) -> Self {
        let browser_id = producer.browser_id();
        Self {
            producer,
            browser_id,
            filter,
            state: ViewState::Live { layout },
            visible: true,
            nav: NavState::default(),
            events: Vec::new(),
            host: None,
            clicks: ClickTracker::default(),
            wheel: WheelRemainder::default(),
            pinch: Pinch::new(pinch::touchpad_available()),
            scroll_x: 0.0,
            accessibility: (false, true),
            held_keys: Vec::new(),
            page: Vec::new(),
            popup: PopupWidget::default(),
            downloads: HashMap::new(),
            find: Find::default(),
            fullscreen: Fullscreen::default(),
            muted: false,
            audio: AudioProbe::default(),
            pending: Default::default(),
            zoom: Zoom::default(),
            drag: DragSource::default(),
            favicons: Default::default(),
            doc_scripts: cdp::DocScripts::default(),
            nav_requests: cdp::NavRequests::default(),
            devtools_client,
            rule,
            popups,
            opened: Vec::new(),
            content: ContentRate::default(),
            ceiling,
            begin_ticks: 0,
        }
    }

    /// Whether a window the page opens with a click (`window.open`, `target=_blank`) becomes
    /// a browser Chromium creates beside this one (`wubble_core::popup::creates_browser`),
    /// reported as [`WebEvent::PopupOpened`] with its view, or stays a request the host opens
    /// itself ([`WebEvent::PopupRequested`]). Off for a new view.
    pub fn allow_popup_browsers(&mut self, on: bool) {
        self.popups
            .on
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The pop-up browsers this view's page opened since the last call, as views, with what
    /// the page asked for.
    pub(crate) fn take_opened(&mut self) -> Vec<(View, crate::nav::PopupRequest)> {
        std::mem::take(&mut self.opened)
    }

    /// CEF created the pop-up `opened` (its factory said yes): its producer becomes a view.
    fn adopt_popup(&mut self, opened: welding::OpenedWindow) {
        let pending = self
            .popups
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&opened.popup_id);
        let Some(producer) = self.producer.take_popup_browser(opened.popup_id) else {
            return;
        };
        let Some(pending) = pending else {
            // never asked of the factory: nothing to show it with
            let mut producer = producer;
            let _ = producer.close();
            return;
        };
        // sized as its opener until the host lays it out
        let layout = match self.state {
            ViewState::Live { layout } | ViewState::Resizing { wanted: layout } => layout,
            ViewState::Closing | ViewState::Closed => pending.layout,
        };
        let child = Self::with_producer(
            producer,
            Layout {
                area: None,
                ..layout
            },
            self.filter.clone(),
            (pending.rule, pending.popups),
            self.devtools_client.clone(),
            self.ceiling,
        );
        let request = crate::nav::PopupRequest {
            url: opened.url,
            gesture: opened.user_gesture,
            opened: popup_opened(opened.kind),
            size: opened.size,
            position: opened.position,
        };
        self.opened.push((child, request));
    }

    /// Where the page may go from now on: each top-level navigation (each hop of a redirect)
    /// to a URL `rule` refuses is cancelled before it is sent, and reported as
    /// [`PageEvent::NavigationRefused`]; `None` lets it go anywhere. Same-document changes
    /// (the history API) and pages restored from the back-forward cache are not asked.
    pub fn set_navigation_rule(&mut self, rule: Option<Rule>) {
        *self.rule.write().unwrap_or_else(|e| e.into_inner()) = rule;
    }

    /// CEF's identifier for this view, as [`crate::filter::RequestInfo::browser_id`]
    /// reports it.
    pub fn browser_id(&self) -> i32 {
        self.browser_id
    }

    /// Navigation as the host last heard of it.
    pub fn nav(&self) -> &NavState {
        &self.nav
    }

    pub fn load(&mut self, url: &str) {
        self.apply_nav(NavInput::Load(url.into()));
    }

    /// Ignored when there is nothing to go back to.
    pub fn back(&mut self) {
        self.sync_history();
        self.apply_nav(NavInput::Back);
    }

    /// Ignored when there is nothing to go forward to.
    pub fn forward(&mut self) {
        self.sync_history();
        self.apply_nav(NavInput::Forward);
    }

    pub fn reload(&mut self) {
        self.apply_nav(NavInput::Reload);
    }

    /// The host shows the default pointer over this view now (another tab
    /// was shown): CEF's next shape is news again, even the one it had.
    pub fn reset_cursor(&mut self) {
        self.apply_nav(NavInput::Cursor(crate::nav::Cursor::Default));
    }

    /// Reloads without the cache (CEF `ReloadIgnoreCache`).
    pub fn hard_reload(&mut self) {
        self.apply_nav(NavInput::HardReload);
    }

    /// Prints the page to a PDF file as `options` ask (the print sheet's).
    /// Completion arrives as [`PageEvent::PdfPrinted`]. The system print
    /// dialog is not offered: it needs an embedder print handler.
    pub fn print_to_pdf(
        &mut self,
        path: &Path,
        options: &wubble_core::print::PdfOptions,
    ) -> Result<(), String> {
        use wubble_core::print::PdfMargins as M;
        let options = welding::PdfOptions {
            landscape: options.landscape,
            background: options.background,
            scale: options.scale,
            paper_inches: options.paper_inches,
            margins: match options.margins {
                M::Default => welding::PdfMargins::Default,
                M::None => welding::PdfMargins::None,
                M::Custom(m) => welding::PdfMargins::Custom(m),
            },
            page_ranges: options.page_ranges.clone(),
        };
        self.producer.print_to_pdf(path, &options).map_err(|e| {
            let error = format!("view {}: print to PDF: {e}", self.browser_id);
            eprintln!("wubble-engine: {error}");
            error
        })
    }

    /// Opens DevTools in its own native window (a non-windowless `WindowInfo`;
    /// DevTools is the allowed non-Slint exception). Focuses it if open.
    /// Verified on Linux and macOS (macOS needs the CEF-aware application
    /// class, `app::prepare_process`); unverified on Windows.
    pub fn open_devtools(&mut self) {
        self.open_devtools_at(None);
    }

    /// [`Self::open_devtools`], revealing the element under `at` (CSS pixels of the page) in
    /// the Elements panel (`ShowDevTools`'s `inspect_element_at`).
    pub fn open_devtools_at(&mut self, at: Option<(i32, i32)>) {
        // The engine's DevTools client: the engine counts the window until it
        // closes (`app::extra`).
        let window = cef::WindowInfo::default();
        let settings = cef::BrowserSettings::default();
        let mut client = self.devtools_client.clone();
        let at = at.map(|(x, y)| cef::Point { x, y });
        self.send(|host| {
            host.show_dev_tools(
                Some(&window),
                Some(&mut client),
                Some(&settings),
                at.as_ref(),
            )
        });
    }

    pub fn close_devtools(&mut self) {
        self.send(|host| host.close_dev_tools());
    }

    /// Freezes the page (no JS, no timers) or thaws it, through CDP `Page.setWebLifecycleState`
    /// (`cdp.rs`). [`PageEvent::Frozen`] follows with Chromium's answer. Freezing also hides
    /// the page; thawing does not show it ([`Self::set_visible`] does).
    pub fn set_frozen(&mut self, frozen: bool) {
        if !self.is_open() {
            return;
        }
        let (id, message) = self.pending.request(frozen);
        if let Err(e) = self.producer.send_devtools_message(&message) {
            eprintln!(
                "wubble-engine: view {}: freeze {frozen}: {e}",
                self.browser_id
            );
            self.pending.cancel(id);
            self.page.push(PageEvent::Frozen { frozen, ok: false });
        }
    }

    /// Asks the page for its CDP target id (`Target.getTargetInfo`); [`PageEvent::TargetId`]
    /// follows. The docked DevTools frontend names its page by it.
    pub fn request_target_id(&mut self) {
        if !self.is_open() {
            self.page.push(PageEvent::TargetId(Err(())));
            return;
        }
        let (id, message) = self.pending.request_target_id();
        if let Err(e) = self.producer.send_devtools_message(&message) {
            eprintln!("wubble-engine: view {}: target id: {e}", self.browser_id);
            self.pending.cancel(id);
            self.page.push(PageEvent::TargetId(Err(())));
        }
    }

    pub fn has_devtools(&mut self) -> bool {
        let mut open = false;
        self.send(|host| open = host.has_dev_tools() != 0);
        open
    }

    /// Ignored unless loading.
    pub fn stop(&mut self) {
        self.apply_nav(NavInput::Stop);
    }

    /// `now` is any monotonic time (it only times double clicks).
    pub fn mouse_move(&mut self, point: Point, modifiers: &Modifiers, now: Duration) {
        self.clicks.moved(now, point);
        self.content(ContentEvent::Input);
        let event = input::mouse_event(point, modifiers);
        self.send(|host| host.send_mouse_move_event(Some(&event), 0));
    }

    pub fn mouse_leave(&mut self, point: Point, modifiers: &Modifiers, now: Duration) {
        self.clicks.moved(now, point);
        let event = input::mouse_event(point, modifiers);
        self.send(|host| host.send_mouse_move_event(Some(&event), 1));
    }

    /// The next press starts a new click series (count 1), however soon and near it comes: an
    /// agent's separate clicks are each their own.
    pub fn forget_clicks(&mut self) {
        self.clicks = ClickTracker::default();
    }

    /// `modifiers` include the button itself while it is down.
    pub fn mouse_button(
        &mut self,
        button: MouseButton,
        pressed: bool,
        point: Point,
        modifiers: &Modifiers,
        now: Duration,
    ) {
        let count = if pressed {
            self.clicks.press(now, point, button)
        } else {
            self.clicks.count()
        };
        self.content(ContentEvent::Input);
        let event = input::mouse_event(point, modifiers);
        let button = input::cef_button(button);
        self.send(|host| {
            host.send_mouse_click_event(Some(&event), button, (!pressed).into(), count)
        });
    }

    /// The page's root horizontal scroll offset (DIP) as last reported: a change during a
    /// sideways swipe means the page took it.
    pub fn scroll_x(&self) -> f64 {
        self.scroll_x
    }

    /// Deltas in logical pixels, positive = up / left; `precise` for a touchpad's, not a
    /// notched wheel's (`input::wheel_deltas`).
    pub fn wheel(&mut self, point: Point, dx: f32, dy: f32, precise: bool, modifiers: &Modifiers) {
        self.content(ContentEvent::Input);
        let (dx, dy, flags) = input::wheel_deltas(dx, dy, modifiers, precise, &mut self.wheel);
        let event = cef::MouseEvent {
            modifiers: flags,
            ..input::mouse_event(point, modifiers)
        };
        self.send(|host| host.send_mouse_wheel_event(Some(&event), dx, dy));
    }

    /// A finger of a touchscreen on the page: finger `id` (small and stable while it is down)
    /// at `point`, view-relative. Blink makes its scrolls, taps, long presses and pinches.
    pub fn touch(
        &mut self,
        id: i32,
        kind: TouchKind,
        point: Point,
        pressure: f32,
        modifiers: &Modifiers,
    ) {
        self.content(ContentEvent::Input);
        let event = input::touch_event(id, kind, point, pressure, modifiers);
        self.send(|host| host.send_touch_event(Some(&event)));
    }

    /// A trackpad pinch over the page (`pinch.rs`): Blink's pinch zoom, never the page zoom.
    /// `Begin` carries the pointer, view-relative.
    pub fn pinch(&mut self, gesture: pinch::Gesture) {
        if !self.is_open() {
            return;
        }
        let input = match gesture {
            pinch::Gesture::Begin(at) => {
                let size = target(self.state).map_or((0.0, 0.0), |l| {
                    let scale = l.scale.max(f32::MIN_POSITIVE);
                    (l.size.width as f32 / scale, l.size.height as f32 / scale)
                });
                pinch::Input::Begin { at, size }
            }
            pinch::Gesture::Scaled(f) => pinch::Input::Scaled(f),
            pinch::Gesture::End => pinch::Input::End,
        };
        let effects = self.pinch.input(input, &mut || CdpId(cdp::next_id()));
        self.execute_pinch(effects);
    }

    /// `zoom.js`'s word on a Ctrl+wheel: the pinch's own wheel's, or a real Ctrl+wheel the
    /// page left alone, which zooms the page one step ([`PageEvent::WheelZoom`]).
    fn zoom_answer(&mut self, body: &str, mode: RequestMode) {
        let Some(answer) = pinch::Answer::parse(body) else {
            return;
        };
        if let Some(effects) = self.pinch.answer(answer) {
            return self.execute_pinch(effects);
        }
        let zoom_in = match answer {
            pinch::Answer::In => true,
            pinch::Answer::Out => false,
            pinch::Answer::Kept => return,
        };
        self.emit_page(PageEvent::WheelZoom { zoom_in }, mode);
    }

    fn execute_pinch(&mut self, effects: Vec<pinch::Effect>) {
        use pinch::Effect;
        for effect in effects {
            match effect {
                Effect::Wheel { at, dy } => {
                    let event = input::ctrl_wheel_event(at);
                    self.send(|host| host.send_mouse_wheel_event(Some(&event), 0, dy));
                }
                Effect::Touch { kind, points } => {
                    for (id, p) in (0..).zip(points) {
                        let event = input::touch_event(id, kind, p, 1.0, &Modifiers::default());
                        self.send(|host| host.send_touch_event(Some(&event)));
                    }
                }
                Effect::Hover(at) => {
                    let event = input::mouse_event(at, &Modifiers::default());
                    self.send(|host| host.send_mouse_move_event(Some(&event), 0));
                }
                Effect::Cdp { id, method, params } => {
                    let message = cdp::message(id, method, &params);
                    if let Err(e) = self.producer.send_devtools_message(&message) {
                        eprintln!("wubble-engine: view {}: {method}: {e}", self.browser_id);
                        let failed = Err(e.to_string());
                        let mut ids = || CdpId(cdp::next_id());
                        if let Some(more) = self.pinch.reply(id, &failed, &mut ids) {
                            self.execute_pinch(more);
                        }
                    }
                }
            }
        }
    }

    pub fn key(&mut self, key: &KeyInput) {
        self.content(ContentEvent::Input);
        self.held_keys
            .retain(|(physical, _)| *physical != key.physical);
        if key.pressed {
            let release = KeyInput {
                pressed: false,
                repeat: false,
                ..key.clone()
            };
            if let Some(&up) = input::key_events(&release).first() {
                self.held_keys.push((key.physical, up));
            }
        }
        for event in input::key_events(key) {
            let event = cef::KeyEvent::from(event);
            self.send(|host| host.send_key_event(Some(&event)));
        }
    }

    /// Keyboard focus. Losing it releases every key still held.
    pub fn focus(&mut self, focused: bool) {
        if !focused {
            for (_, up) in std::mem::take(&mut self.held_keys) {
                let event = cef::KeyEvent::from(up);
                self.send(|host| host.send_key_event(Some(&event)));
            }
        }
        self.send(|host| host.set_focus(focused.into()));
    }

    /// The pointer grab ended without a release (Slint's `cancel`).
    pub fn capture_lost(&mut self) {
        self.send(|host| host.send_capture_lost_event());
    }

    /// Runs `f` on the browser host while the view is open.
    fn send(&mut self, f: impl FnOnce(&cef::BrowserHost)) {
        if !self.is_open() {
            return;
        }
        if self.host.is_none() {
            self.host = cef::browser_host_get_browser_by_identifier(self.browser_id)
                .and_then(|browser| browser.host());
        }
        if let Some(host) = &self.host {
            f(host);
        }
    }

    /// Shows or hides the page (CEF `WasHidden`, as cefclient's OSR windows
    /// do). A hidden browser stops painting and Chromium throttles its timers;
    /// on show it is asked to repaint so a frame arrives without waiting for
    /// the page to change.
    pub fn set_visible(&mut self, visible: bool) {
        if visible == self.visible || !self.is_open() {
            return;
        }
        self.visible = visible;
        // Trait call, as in `take_frame`.
        let mut result = CefSurfaceProducer::set_visible(&mut self.producer, visible);
        if visible && result.is_ok() {
            result = self.producer.request_repaint();
        }
        if let Err(e) = result {
            eprintln!(
                "wubble-engine: view {}: set_visible({visible}) failed: {e}",
                self.browser_id
            );
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Not yet asked to close and not gone from CEF.
    pub fn is_open(&self) -> bool {
        matches!(
            self.state,
            ViewState::Live { .. } | ViewState::Resizing { .. }
        )
    }

    /// Asks CEF for one frame (`SendExternalBeginFrame`); only for a view
    /// opened with `begin_frames`. A calm page takes only its rate's share of them: whether
    /// this one went out.
    pub fn send_begin_frame(&mut self) -> bool {
        let tick = self.begin_ticks;
        self.begin_ticks = tick.wrapping_add(1);
        let due =
            pacing::begin_frame_due(self.ceiling, self.content.frame_rate(self.ceiling), tick);
        if due {
            self.send(|host| host.send_external_begin_frame());
        }
        due
    }

    /// The page layer's `Effect::Buffers` (`wubble_core::page_layer`): this view's frames are
    /// copied into OS buffers (`on`), [`wubble_core::page_layer::POOL_SLOTS`] of them chosen by
    /// `pick_slot`, which each frame then carries for the layer; or into textures of their
    /// own. `Err` where welding has none (every platform but macOS).
    pub fn set_os_buffers(&mut self, on: bool) -> Result<(), String> {
        use wubble_core::page_layer::{POOL_SLOTS, pick_slot};
        let request = on.then_some(welding::OsBufferRequest {
            slots: POOL_SLOTS,
            choose: pick_slot,
        });
        self.producer
            .set_os_buffers(request)
            .map_err(|e| e.to_string())
    }

    /// Frames welding dropped since the last call because every OS buffer of
    /// [`Self::set_os_buffers`] was in use (0 without them; macOS only).
    pub fn take_os_buffers_busy(&mut self) -> u64 {
        self.producer.take_os_buffers_busy()
    }

    /// Whether this view's newest frame was dropped for a busy pool with none after it, since
    /// the last call: what the layer shows is a frame behind until the page paints again.
    pub fn take_os_buffers_lost(&mut self) -> bool {
        self.producer.take_os_buffers_lost()
    }

    /// Text on every page of the profile is drawn at least `px` CSS pixels tall (0: no
    /// minimum): Chromium's `webkit.webprefs.minimum_font_size` (Chrome's "Minimum font size"),
    /// set on this view's request context; Chromium pushes it to the profile's open pages.
    /// `Err` when the view is not open yet or CEF refuses it.
    pub fn set_minimum_font_size(&mut self, px: u32) -> Result<(), String> {
        const PREF: &str = "webkit.webprefs.minimum_font_size";
        let px = i32::try_from(px).map_err(|e| e.to_string())?;
        let mut result = Err(format!("{PREF}: the view is not open"));
        self.send(|host| {
            result = (|| {
                let context = host.request_context().ok_or("no request context")?;
                let mut value = cef::value_create().ok_or("CEF made no value")?;
                value.set_int(px);
                // Not `CefString::default()`: that passes a null `error`, and CEF's C wrapper
                // refuses the call outright when an out-parameter is null.
                let mut error = cef::CefString::from("");
                let name = cef::CefString::from(PREF);
                if context.set_preference(Some(&name), Some(&mut value), Some(&mut error)) == 0 {
                    return Err(format!("{PREF}: {error}"));
                }
                Ok(())
            })();
        });
        result
    }

    /// The frame-rate ceiling, for a window that moved to a display with another refresh
    /// rate. CEF is told the rate the page's content has under it (`SetWindowlessFrameRate`).
    pub fn set_frame_rate(&mut self, frame_rate: u32) {
        self.ceiling = frame_rate;
        let rate = self.content.frame_rate(frame_rate);
        self.send(|host| host.set_windowless_frame_rate(rate as i32));
    }

    /// Input or a frame: the page's frame rate follows what changes in it
    /// (`pacing::ContentRate`). CEF is told only when the rate changes.
    fn content(&mut self, event: ContentEvent) {
        let before = self.content.frame_rate(self.ceiling);
        self.content = self.content.after(event, std::time::Instant::now());
        if self.content.frame_rate(self.ceiling) != before {
            self.set_frame_rate(self.ceiling);
        }
    }

    /// Follows the host's content rectangle; unchanged layouts cost nothing.
    pub fn set_layout(&mut self, layout: Layout) {
        self.apply(ViewEvent::Layout(layout));
    }

    /// The frame size CEF paints at or was last asked for: the one owner of
    /// a pane's wanted size. `None` once closing.
    pub fn wanted_frame(&self) -> Option<FrameSize> {
        target(self.state).map(Layout::frame_size)
    }

    /// The newest frame CEF painted since the last call, if it is to be
    /// shown. Always `None` while hidden.
    pub fn take_frame(&mut self) -> Option<NativeFrame> {
        if !self.visible {
            return None;
        }
        // Trait call: Windows' producer has an inherent method of the same
        // name that returns its own frame type.
        let frame = NativeFrame::from(CefSurfaceProducer::acquire_native_frame(
            &mut self.producer,
        )?);
        let size = frame.size();
        self.content(ContentEvent::Frame {
            large: pacing::large_damage(
                frame.damage().map(|d| (d.width, d.height)),
                (size.width, size.height),
            ),
        });
        let effects = self.apply(ViewEvent::Frame(size));
        effects.contains(&ViewEffect::Present).then_some(frame)
    }

    pub(crate) fn close(&mut self) {
        self.apply(ViewEvent::CloseRequested);
    }

    /// CPU `OnPaint` frames CEF sent instead of shared textures so far;
    /// `None` where welding does not count them.
    pub fn cpu_fallback_frames(&self) -> Option<u64> {
        self.producer.cpu_paint_count()
    }

    /// Takes the callbacks welding queued since the last call and returns
    /// what the host should hear, in order, and whether the view reported
    /// itself closed (true exactly once). Events nothing uses are dropped
    /// here so the queue does not grow. With [`RequestMode::DefaultNow`]
    /// every request is answered with its default at once and not reported.
    pub(crate) fn drain_events(
        &mut self,
        mode: RequestMode,
    ) -> (Vec<WebEvent>, Vec<PageEvent>, bool) {
        let mut closed = false;
        while let Some(event) = self.producer.poll_web_event() {
            match event {
                CefSurfaceEvent::BrowserClosed => {
                    closed |= self
                        .apply(ViewEvent::GoneFromCef)
                        .contains(&ViewEffect::ReportClosed);
                }
                CefSurfaceEvent::Navigation(event) => {
                    for routed in route(event) {
                        self.apply_route(routed, mode);
                    }
                }
                CefSurfaceEvent::KeyUnhandled {
                    windows_key_code,
                    modifiers,
                    focus_on_editable_field,
                } => self.events.push(WebEvent::KeyUnhandled {
                    key: KeyId::reported(windows_key_code, modifiers),
                    editable: focus_on_editable_field,
                }),
                CefSurfaceEvent::FrameMessage {
                    frame_id,
                    frame_url,
                    is_main,
                    pid,
                    text,
                } => {
                    let frame = crate::page::FrameInfo {
                        id: frame_id,
                        url: frame_url,
                        is_main,
                        pid,
                    };
                    match FrameChannel::split(&text) {
                        Some((FrameChannel::Cosmetic, body)) => self.answer_cosmetic(&frame, body),
                        Some((FrameChannel::Zoom, body)) => self.zoom_answer(body, mode),
                        _ => {
                            if let Some(event) = frame_message(frame, text) {
                                self.emit_page(event, mode);
                            }
                        }
                    }
                }
                // The audible probe's answer, or one of `eval`'s: one id counter for both.
                CefSurfaceEvent::ScriptCompleted { id, result } => {
                    let id = id.get();
                    if matches!(self.audio, AudioProbe::Waiting { id: w, .. } if w == id) {
                        let audible = result.ok().and_then(|json| json.trim().parse().ok());
                        self.apply_probe(ProbeInput::Answered { id, audible });
                    } else {
                        self.page.push(PageEvent::EvalResult {
                            id: EvalId(id),
                            result,
                        });
                    }
                }
                _ => {}
            }
        }
        while let Some(text) = self.producer.poll_devtools_message() {
            self.nav_requests.read(&text);
            if let Some(opened) = cdp::chooser_opened(&text) {
                self.page.push(PageEvent::ChooserIntercepted(opened));
                continue;
            }
            // the engine's own requests first (freeze, target id), then `View::cdp`'s calls
            if let Some(event) = self.pending.reply(&text) {
                self.page.push(event);
            } else if let Some((id, result)) = cdp::reply(&text) {
                let mut ids = || CdpId(cdp::next_id());
                if let Some(effects) = self.pinch.reply(id, &result, &mut ids) {
                    if let Err(e) = &result {
                        eprintln!("wubble-engine: view {}: pinch: {e}", self.browser_id);
                    }
                    self.execute_pinch(effects);
                    continue;
                }
                match self.doc_scripts.reply(id, &result) {
                    Some(calls) => {
                        if let Err(e) = &result {
                            eprintln!(
                                "wubble-engine: view {}: page script refused: {e}",
                                self.browser_id
                            );
                        }
                        self.send_doc_calls(calls)
                    }
                    None => self.page.push(PageEvent::CdpReply { id, result }),
                }
            }
        }
        if let Some(shape) = self.producer.poll_cursor_shape() {
            // Before the cursor change: the host applies the image with it, and
            // alone when the cursor was an image already.
            if let welding::CursorShape::Image(image) = &shape {
                self.events.push(WebEvent::CursorImage(image.clone()));
            }
            self.apply_nav(NavInput::Cursor(shape.into()));
        }
        let placement = self.producer.popup_rect().map(Rect::from);
        let (next, event) = popup::transition(self.popup, placement);
        self.popup = next;
        self.page.extend(event);
        let images = std::mem::take(&mut *self.favicons.lock().unwrap_or_else(|e| e.into_inner()));
        self.page.extend(images);
        if let Some(composition) = self.producer.poll_ime_composition() {
            self.page.push(crate::ime::bounds(composition));
        }
        (
            std::mem::take(&mut self.events),
            std::mem::take(&mut self.page),
            closed,
        )
    }

    fn apply_route(&mut self, routed: Route, mode: RequestMode) {
        match routed {
            Route::ScrollX(x) => self.scroll_x = x,
            Route::PopupCreated(opened) => self.adopt_popup(opened),
            Route::PopupAborted(id) => {
                self.popups
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
            }
            Route::Crashed => {
                eprintln!("wubble-engine: view {}: renderer gone", self.browser_id);
                // A host that asks about requests draws its own crash page,
                // whose Reload recovers; reloading here would hide it.
                self.apply_nav(NavInput::RendererTerminated {
                    reload: mode == RequestMode::DefaultNow,
                });
            }
            Route::Nav(input) => {
                let address = matches!(input, NavInput::Address(_));
                if address {
                    self.apply_probe(ProbeInput::Navigated);
                }
                self.apply_nav(input);
                if address {
                    self.sync_history();
                }
            }
            Route::Page(event) => {
                if let PageEvent::LoadFailed { url, code, text } = &event {
                    eprintln!(
                        "wubble-engine: view {}: {url} failed: {text} ({code})",
                        self.browser_id
                    );
                }
                self.emit_page(event, mode);
            }
            Route::Download(id, input) => self.apply_download(id, input, mode),
            Route::Find(input) => self.apply_find(input),
            Route::Fullscreen(input) => {
                let (next, effects) = media::fullscreen(self.fullscreen, input);
                self.fullscreen = next;
                self.execute_media(effects);
            }
            Route::DragStarted(event) => {
                let (next, report) = drag::transition(self.drag, DragSourceInput::Started);
                self.drag = next;
                if report {
                    self.page.push(event);
                }
            }
            Route::ObserveZoom => self.observe_zoom(),
            Route::PageCommitted => {
                self.tell_accessibility();
                self.page.push(PageEvent::Committed);
            }
        }
    }

    /// Answers a frame's `cosmetic.js` with the content filter's rules
    /// (`cosmetic::Request`): the data goes back on the channel, scriptlets
    /// run in the frame from here.
    fn answer_cosmetic(&mut self, frame: &crate::page::FrameInfo, body: &str) {
        use crate::cosmetic::{Request, reply_text};
        use crate::filter::FrameCosmetic;
        let Some(filter) = self.filter.clone() else {
            return;
        };
        // A main frame is its own page; a subframe's page is the view's.
        let top = if frame.is_main {
            frame.url.clone()
        } else {
            self.nav.page.url.clone()
        };
        let reply = match Request::parse(body) {
            Some(Request::Hello) => {
                let mut given = filter.frame_cosmetic(&top, &frame.url);
                if let Some(script) = given.script.take() {
                    self.run_script_in(&frame.id, &script);
                }
                given
            }
            // `generic` stays off: the page already reports names.
            Some(Request::Names { classes, ids }) => FrameCosmetic {
                css: filter.generic_hide(&top, &frame.url, &classes, &ids),
                ..FrameCosmetic::default()
            },
            None => return,
        };
        if reply != FrameCosmetic::default() {
            self.send_frame_message(&frame.id, FrameChannel::Cosmetic, &reply_text(&reply));
        }
    }

    /// Reports `event`; in [`RequestMode::DefaultNow`] a request is answered
    /// with its default instead.
    fn emit_page(&mut self, event: PageEvent, mode: RequestMode) {
        if mode == RequestMode::DefaultNow && event.request().is_some() {
            self.decline(&event);
        } else {
            self.page.push(event);
        }
    }

    /// Answers a request as a dismissed prompt would (welding gives the same
    /// answers when the view closes): for requests the host has no prompt
    /// for. Anything that is not a request is ignored.
    pub fn decline(&mut self, request: &PageEvent) {
        match *request {
            PageEvent::DialogRequested { id, kind, .. } => {
                self.answer_dialog(id, kind.default_accept(), "");
            }
            PageEvent::ContextMenuRequested { id, .. } => {
                self.answer_context_menu(id, None);
            }
            PageEvent::PermissionRequested { id, .. } => {
                self.answer_permission(id, &[]);
            }
            PageEvent::FileChooserRequested { id, .. } => {
                self.answer_file_chooser(id, None);
            }
            PageEvent::AuthRequested { id, .. } => {
                self.answer_auth(id, None);
            }
            PageEvent::CertificateError { id, .. } => {
                self.answer_certificate(id, false);
            }
            PageEvent::DownloadRequested { id, .. } => {
                self.answer_download_path(id, None);
            }
            _ => {}
        }
    }

    /// Hands `text` to the context script in the frame `frame` (a
    /// [`PageEvent::FrameMessage`]'s frame id). False when that frame is
    /// gone. `text` is never logged.
    pub fn send_frame_message(&mut self, frame: &str, channel: FrameChannel, text: &str) -> bool {
        match self.producer.send_frame_message(frame, &channel.join(text)) {
            Ok(()) => true,
            Err(e) => {
                eprintln!(
                    "wubble-engine: view {}: frame message not sent: {e}",
                    self.browser_id
                );
                false
            }
        }
    }

    // ── Answers. Each returns false for a stale id (already answered,
    // dismissed by the page, view closing), which is logged.

    /// `accept`: OK / leave the page; `text`: a prompt's input.
    pub fn answer_dialog(&mut self, id: RequestId, accept: bool, text: &str) -> bool {
        let result = self.producer.answer_dialog(id.0, accept, text);
        self.answered("JS dialog", id, result)
    }

    /// A command from the menu's items, or `None` to dismiss it.
    pub fn answer_context_menu(&mut self, id: RequestId, command: Option<i32>) -> bool {
        let result = self.producer.answer_context_menu(id.0, command);
        self.answered("context menu", id, result)
    }

    /// Grants the request only when `granted` covers everything it asked
    /// for, otherwise denies all of it: CEF takes one answer per request
    /// (a getUserMedia for camera and microphone cannot be granted the
    /// camera alone).
    pub fn answer_permission(&mut self, id: RequestId, granted: &[PermissionKind]) -> bool {
        let result = self.producer.answer_permission(id.0, granted);
        self.answered("permission request", id, result)
    }

    /// The chosen paths, or `None` to cancel.
    pub fn answer_file_chooser(&mut self, id: RequestId, paths: Option<&[PathBuf]>) -> bool {
        let result = self.producer.answer_file_chooser(id.0, paths);
        self.answered("file chooser", id, result)
    }

    /// `(user, password)`, or `None` to cancel. Neither is logged.
    pub fn answer_auth(&mut self, id: RequestId, credentials: Option<(&str, &str)>) -> bool {
        let result = match credentials {
            Some((user, password)) => self.producer.answer_auth(id.0, user, password),
            None => self.producer.cancel_auth(id.0),
        };
        self.answered("auth challenge", id, result)
    }

    /// Proceed past the certificate error, or cancel the load.
    pub fn answer_certificate(&mut self, id: RequestId, proceed: bool) -> bool {
        let result = self.producer.answer_certificate(id.0, proceed);
        self.answered("certificate error", id, result)
    }

    /// Where the download goes (used as given; no file dialog), or `None`
    /// to cancel it.
    pub fn answer_download_path(&mut self, id: RequestId, path: Option<&Path>) -> bool {
        let waiting = self.downloads.iter().find_map(|(&download, &state)| {
            (state == Download::Requested { request: id }).then_some(download)
        });
        let Some(download) = waiting else {
            eprintln!(
                "wubble-engine: view {}: no download waits on request {}",
                self.browser_id, id.0
            );
            return false;
        };
        let result = self.producer.answer_download_path(id.0, path);
        let ok = self.answered("download path", id, result);
        if ok && path.is_none() {
            self.apply_download(download, DownloadInput::Declined, RequestMode::AskHost);
        }
        ok
    }

    fn answered(&self, what: &str, id: RequestId, result: Result<(), WeldError>) -> bool {
        result
            .inspect_err(|e| {
                eprintln!(
                    "wubble-engine: view {}: {what} {} not answered: {e}",
                    self.browser_id, id.0
                )
            })
            .is_ok()
    }

    // ── Downloads.

    pub fn pause_download(&mut self, id: DownloadId) {
        self.apply_download(id, DownloadInput::Pause, RequestMode::AskHost);
    }

    pub fn resume_download(&mut self, id: DownloadId) {
        self.apply_download(id, DownloadInput::Resume, RequestMode::AskHost);
    }

    pub fn cancel_download(&mut self, id: DownloadId) {
        self.apply_download(id, DownloadInput::Cancel, RequestMode::AskHost);
    }

    /// Asks Chromium for a fresh frame of the page (after the GPU process restarted, the
    /// frames it had handed over are gone).
    pub fn repaint(&mut self) {
        if let Err(e) = self.producer.request_repaint() {
            eprintln!("wubble-engine: view {}: repaint: {e}", self.browser_id);
        }
    }

    /// Fetches the image at `url` at full size with the page's cookies (CEF
    /// `BrowserHost::DownloadImage`); it arrives as
    /// [`PageEvent::ImageDownloaded`]. For Copy Image.
    pub fn download_image(&mut self, url: &str) {
        let sink = self.favicons.clone();
        self.send(|host| crate::favicon::fetch(host, url, &sink, crate::favicon::Purpose::Full));
    }

    /// Downloads `url` as a new download (CEF `BrowserHost::StartDownload`);
    /// it arrives as a fresh `DownloadRequested`. A retry starts over.
    pub fn start_download(&mut self, url: &str) {
        let url = cef::CefString::from(url);
        self.send(|host| host.start_download(Some(&url)));
    }

    fn apply_download(&mut self, id: DownloadId, input: DownloadInput, mode: RequestMode) {
        let (next, effects) = download::transition(self.downloads.get(&id).copied(), id, input);
        match next {
            Some(state) => self.downloads.insert(id, state),
            None => self.downloads.remove(&id),
        };
        for effect in effects {
            let result = match effect {
                DownloadEffect::Emit(event) => {
                    self.emit_page(event, mode);
                    continue;
                }
                DownloadEffect::Pause => self.producer.pause_download(id.0),
                DownloadEffect::Resume => self.producer.resume_download(id.0),
                DownloadEffect::Cancel => self.producer.cancel_download(id.0),
            };
            if let Err(e) = result {
                eprintln!(
                    "wubble-engine: view {}: download {}: {e}",
                    self.browser_id, id.0
                );
            }
        }
    }

    // ── Find in page.

    /// Starts a search, or steps forward when `text` is the current one.
    /// Empty text stops.
    pub fn find(&mut self, text: &str, match_case: bool) {
        self.apply_find(FindInput::Start {
            text: text.into(),
            match_case,
        });
    }

    pub fn find_next(&mut self) {
        self.apply_find(FindInput::Next);
    }

    pub fn find_previous(&mut self) {
        self.apply_find(FindInput::Previous);
    }

    /// `clear_selection` false leaves the active match selected.
    pub fn stop_find(&mut self, clear_selection: bool) {
        self.apply_find(FindInput::Stop { clear_selection });
    }

    fn apply_find(&mut self, input: FindInput) {
        let (next, effects) = find::transition(std::mem::take(&mut self.find), input);
        self.find = next;
        for effect in effects {
            let result = match effect {
                FindEffect::Emit(event) => {
                    self.page.push(event);
                    continue;
                }
                FindEffect::Find {
                    text,
                    forward,
                    match_case,
                    find_next,
                } => self.producer.find(&text, forward, match_case, find_next),
                FindEffect::StopFinding { clear_selection } => {
                    self.producer.stop_finding(clear_selection)
                }
            };
            if let Err(e) = result {
                eprintln!("wubble-engine: view {}: find: {e}", self.browser_id);
            }
        }
    }

    // ── Zoom.

    pub fn zoom_in(&mut self) {
        self.zoom_step(welding::ZoomCommand::In);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_step(welding::ZoomCommand::Out);
    }

    /// Turns the page's accessibility tree on or off (CEF's TreeOnly mode for a windowless
    /// browser: the renderer computes the whole tree and reports it as
    /// [`PageEvent::AxTree`] and [`PageEvent::AxLocation`]). Off by default: it costs CPU and
    /// memory on every page, so only while an assistive technology runs or a setting asks.
    pub fn set_accessibility(&mut self, on: bool) {
        if self.accessibility != (on, true) {
            self.accessibility = (on, false);
            self.tell_accessibility();
        }
    }

    /// Tells CEF the accessibility state wanted, once its browser host exists (else at the next
    /// commit).
    fn tell_accessibility(&mut self) {
        let (on, told) = self.accessibility;
        if told {
            return;
        }
        let state = if on {
            cef::State::ENABLED
        } else {
            cef::State::DISABLED
        };
        self.send(|host| host.set_accessibility_state(state));
        self.accessibility.1 = self.host.is_some();
    }

    /// Clamped to 25..=500 %.
    pub fn set_zoom_percent(&mut self, percent: u32) {
        if self.is_open() {
            let result = self.producer.set_zoom_level(zoom::level(percent));
            self.zoomed(result);
        }
    }

    pub fn zoom_percent(&self) -> u32 {
        self.zoom.percent
    }

    fn zoom_step(&mut self, command: welding::ZoomCommand) {
        if self.is_open() {
            let result = self.producer.zoom(command);
            self.zoomed(result);
        }
    }

    /// CEF applies zoom at once on its UI thread (the engine's), so the
    /// level is read straight back.
    fn zoomed(&mut self, result: Result<(), WeldError>) {
        match result {
            Ok(()) => self.observe_zoom(),
            Err(e) => eprintln!("wubble-engine: view {}: zoom: {e}", self.browser_id),
        }
    }

    fn observe_zoom(&mut self) {
        if let Some(event) = self.zoom.observe(self.producer.zoom_level()) {
            self.page.push(event);
        }
    }

    // ── Fullscreen and mute.

    /// Leaves page fullscreen (map Esc to this). Ignored unless fullscreen;
    /// [`PageEvent::FullscreenChanged`] follows.
    pub fn exit_fullscreen(&mut self) {
        let (next, effects) = media::fullscreen(self.fullscreen, FullscreenInput::Exit);
        self.fullscreen = next;
        self.execute_media(effects);
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen == Fullscreen::On
    }

    pub fn set_muted(&mut self, muted: bool) {
        let (next, effects) = media::mute(self.muted, muted);
        self.muted = next;
        self.execute_media(effects);
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Asks the page whether it plays sound ([`media::AUDIBLE_PROBE`]);
    /// [`PageEvent::Audible`] follows when the answer changed. Ignored while
    /// the last probe is unanswered or the view is closing.
    pub fn probe_audible(&mut self) {
        if !self.is_open() || !self.audio.idle() {
            return;
        }
        let id = cdp::next_id();
        match self
            .producer
            .request_script_result(welding::WebRequestId::new(id), media::AUDIBLE_PROBE)
        {
            Ok(()) => self.apply_probe(ProbeInput::Sent(id)),
            // No main frame yet (still loading), say: the next probe retries.
            Err(e) => eprintln!(
                "wubble-engine: view {}: audible probe: {e}",
                self.browser_id
            ),
        }
    }

    /// Deletes everything `origin` (`scheme://host[:port]`) stored in this
    /// view's container: cookies, local storage, IndexedDB, cache storage,
    /// service workers (CDP `Storage.clearDataForOrigin`). Done when the
    /// [`PageEvent::CdpReply`] with the id returned arrives; `None` when the
    /// view could not take the call.
    pub fn forget_site(&mut self, origin: &str) -> Option<CdpId> {
        let params = serde_json::json!({ "origin": origin, "storageTypes": "all" });
        self.cdp("Storage.clearDataForOrigin", &params)
    }

    /// How many navigations of its own the page has asked for so far (`cdp::NavRequests`).
    pub fn navigations_requested(&self) -> u64 {
        self.nav_requests.count()
    }

    /// Sends one DevTools protocol call to this page (`cdp.rs`); its answer
    /// arrives as [`PageEvent::CdpReply`] with the id returned. `None` when
    /// the view cannot take it (closing, or CEF refused the message).
    pub fn cdp(&mut self, method: &str, params: &serde_json::Value) -> Option<CdpId> {
        if !self.is_open() {
            return None;
        }
        let id = CdpId(cdp::next_id());
        match self
            .producer
            .send_devtools_message(&cdp::message(id, method, params))
        {
            Ok(()) => Some(id),
            Err(e) => {
                eprintln!("wubble-engine: view {}: {method}: {e}", self.browser_id);
                None
            }
        }
    }

    /// Runs `script`, a JavaScript expression, in the page's main frame; its
    /// value comes back as JSON text in [`PageEvent::EvalResult`] with the id
    /// returned. `None` when there is no main frame to run it in yet.
    pub fn eval(&mut self, script: &str) -> Option<EvalId> {
        if !self.is_open() {
            return None;
        }
        let id = cdp::next_id();
        // A newline, so a trailing line comment cannot swallow the wrapper's `)`.
        let script = format!("{script}\n");
        match self
            .producer
            .request_script_result(welding::WebRequestId::new(id), &script)
        {
            Ok(()) => Some(EvalId(id)),
            Err(e) => {
                eprintln!("wubble-engine: view {}: script: {e}", self.browser_id);
                None
            }
        }
    }

    /// Every new document of this view (each frame) runs `sources` first, before its own
    /// scripts, replacing what an earlier call set (`cdp::DocScripts`). The page now on screen
    /// is not touched: [`Self::run_script`] that.
    pub fn set_document_scripts(&mut self, sources: &[String]) {
        let calls = self.doc_scripts.set(sources);
        self.send_doc_calls(calls);
    }

    fn send_doc_calls(&mut self, calls: Vec<cdp::DocCall>) {
        for call in calls {
            let (method, params) = call.method();
            if let Some(id) = self.cdp(method, &params) {
                self.doc_scripts.sent(id, &call);
            }
        }
    }

    /// Runs `script` (statements) in the frame `frame` (a context menu's
    /// [`ContextMenu::frame`]), the main frame when it is empty or gone.
    pub fn run_script_in(&mut self, frame: &str, script: &str) {
        if !self.is_open() {
            return;
        }
        let url = self.nav.page.url.clone();
        if let Err(e) = self.producer.execute_script_in_frame(frame, script, &url) {
            eprintln!(
                "wubble-engine: view {}: script not run: {e}",
                self.browser_id
            );
        }
    }

    /// Runs `script` (statements) in the page's main frame, with no answer.
    pub fn run_script(&mut self, script: &str) {
        if !self.is_open() {
            return;
        }
        let url = self.nav.page.url.clone();
        if let Err(e) = self.producer.execute_script(script, &url) {
            eprintln!(
                "wubble-engine: view {}: script not run: {e}",
                self.browser_id
            );
        }
    }

    fn apply_probe(&mut self, input: ProbeInput) {
        let (next, event) = media::probe(self.audio, input);
        self.audio = next;
        self.page.extend(event);
    }

    fn execute_media(&mut self, effects: Vec<MediaEffect>) {
        for effect in effects {
            match effect {
                MediaEffect::Emit(event) => self.page.push(event),
                // true: the host resizes the view back.
                MediaEffect::ExitFullscreen => self.send(|host| host.exit_fullscreen(1)),
                MediaEffect::SetMuted(muted) => {
                    self.send(|host| host.set_audio_muted(muted.into()))
                }
            }
        }
    }

    /// Answers a frame's clipboard request ([`PageEvent::Clipboard`]): what to paste, or a
    /// read's result, under the request's `token`; `None` refuses it (a read's promise
    /// rejects). Also a paste the host starts itself (no request, no token: Paste as Plain
    /// Text).
    pub fn answer_clipboard(
        &mut self,
        frame: &str,
        payload: Option<&wubble_core::clipboard::Payload>,
        token: &str,
    ) {
        let json = payload.map_or_else(|| "null".to_owned(), |p| p.to_json());
        self.send_frame_message(frame, FrameChannel::Clipboard, &format!("{token}\n{json}"));
    }

    /// The identifier of the page's focused frame (CEF's `GetFocusedFrame`), as frame messages
    /// name their frame. Before any frame took focus it is the main frame (CEF's
    /// `CefBrowserHostBase::GetFocusedFrame` falls back to it); `None` only without a browser.
    pub fn focused_frame(&mut self) -> Option<String> {
        let mut id = None;
        self.send(|host| {
            id = host
                .browser()
                .and_then(|b| b.focused_frame())
                .map(|f| welding::cef_string(f.identifier()));
        });
        id
    }

    // ── Editing, IME, drags, popup frames.

    /// Runs an edit command on the focused frame.
    pub fn edit(&mut self, command: EditCommand) {
        self.send(|host| {
            let Some(frame) = host.browser().and_then(|b| b.focused_frame()) else {
                return;
            };
            match command {
                EditCommand::Undo => frame.undo(),
                EditCommand::Redo => frame.redo(),
                EditCommand::Cut => frame.cut(),
                EditCommand::Copy => frame.copy(),
                EditCommand::Paste => frame.paste(),
                EditCommand::PasteMatchStyle => frame.paste_and_match_style(),
                EditCommand::Delete => frame.del(),
                EditCommand::SelectAll => frame.select_all(),
            }
        });
    }

    /// Sets or updates the composition (preedit) text; `selection` is a
    /// character range within it. CEF draws it in the page.
    pub fn ime_set_composition(&mut self, text: &str, selection: (u32, u32)) {
        self.ime(|p| p.ime_set_composition(text, selection));
    }

    /// Commits the composition, or inserts `text` when there is none.
    pub fn ime_commit(&mut self, text: &str) {
        self.ime(|p| p.ime_commit_text(text));
    }

    /// Ends the composition, keeping its text.
    pub fn ime_finish(&mut self, keep_selection: bool) {
        self.ime(|p| p.ime_finish_composing(keep_selection));
    }

    pub fn ime_cancel(&mut self) {
        self.ime(|p| p.ime_cancel_composition());
    }

    fn ime(&mut self, f: impl FnOnce(&mut PlatformCefProducer) -> Result<(), WeldError>) {
        if self.is_open()
            && let Err(e) = f(&mut self.producer)
        {
            eprintln!("wubble-engine: view {}: IME: {e}", self.browser_id);
        }
    }

    /// Something dragged over the view (from another app or view). `point`
    /// in view logical pixels, like mouse input.
    pub fn drag(
        &mut self,
        phase: DragPhase,
        point: Point,
        modifiers: &Modifiers,
        allowed: DragOps,
    ) {
        if !self.is_open() {
            return;
        }
        let (kind, payload, from) = match phase {
            DragPhase::Enter(data) => (welding::DragEventKind::Enter, Some(data.into()), None),
            DragPhase::EnterSource => (welding::DragEventKind::Enter, None, None),
            DragPhase::EnterFrom(held) => (welding::DragEventKind::Enter, None, Some(held.0)),
            DragPhase::Over => (welding::DragEventKind::Over, None, None),
            DragPhase::Leave => (welding::DragEventKind::Leave, None, None),
            DragPhase::Drop => (welding::DragEventKind::Drop, None, None),
        };
        let (x, y) = self.physical(point);
        let event = welding::DragInput {
            kind,
            payload,
            from,
            x,
            y,
            modifiers: weld_modifiers(modifiers),
            allowed_operations: welding::DragOperations(allowed.0),
        };
        if let Err(e) = self.producer.send_drag_input(event) {
            eprintln!("wubble-engine: view {}: drag: {e}", self.browser_id);
        }
    }

    /// The page drag this view's page runs, as CEF holds it, for another
    /// view's [`DragPhase::EnterFrom`]; `None` when none runs.
    pub fn held_drag(&self) -> Option<HeldDrag> {
        self.producer.held_drag().map(HeldDrag)
    }

    /// Lets pages start drags out of the view ([`PageEvent::DragStarted`]).
    /// Off by default: Slint 1.19 has no OS drag source, and CEF waits for
    /// every drag it hands over to be ended.
    pub fn set_drag_source(&mut self, enabled: bool) {
        if let Err(e) = self.producer.set_drag_source(enabled) {
            eprintln!("wubble-engine: view {}: drag source: {e}", self.browser_id);
        }
    }

    /// The host's drag loop for [`PageEvent::DragStarted`] ended at `point`
    /// with `operation` (`DragOps::NONE` when cancelled). Ignored unless a
    /// page drag is running.
    pub fn drag_source_ended(&mut self, point: Point, operation: DragOps) {
        let (next, accepted) = drag::transition(self.drag, DragSourceInput::Ended);
        self.drag = next;
        if !accepted || !self.is_open() {
            return;
        }
        let (x, y) = self.physical(point);
        if let Err(e) = self
            .producer
            .finish_drag_source(x, y, welding::DragOperations(operation.0))
        {
            eprintln!("wubble-engine: view {}: drag source: {e}", self.browser_id);
        }
    }

    /// welding takes physical pixels for drags.
    fn physical(&self, point: Point) -> (i32, i32) {
        let scale = self.producer.scale_factor();
        (
            (point.x * scale).round() as i32,
            (point.y * scale).round() as i32,
        )
    }

    /// The popup widget's newest frame and placement, if one is showing
    /// and painted since the last call. Always `None` while hidden, and on
    /// Windows (welding has no native popup frame there).
    pub fn take_popup_frame(&mut self) -> Option<PopupLayer> {
        if !self.visible {
            return None;
        }
        let (frame, rect) = self.producer.acquire_native_popup()?;
        Some(PopupLayer {
            frame: NativeFrame::from(frame),
            rect: rect.into(),
        })
    }

    /// Reads the history flags back from CEF. A same-document navigation (a
    /// fragment, `history.pushState`) changes them without a loading-state
    /// callback, so Back would otherwise be refused after one.
    fn sync_history(&mut self) {
        let Some(browser) = cef::browser_host_get_browser_by_identifier(self.browser_id) else {
            return;
        };
        let loading = matches!(self.nav.phase, nav::Phase::Loading { .. });
        self.apply_nav(NavInput::Loading {
            loading,
            can_go_back: browser.can_go_back() != 0,
            can_go_forward: browser.can_go_forward() != 0,
        });
    }

    fn apply_nav(&mut self, input: NavInput) {
        let (next, effects) = nav::transition(std::mem::take(&mut self.nav), input);
        self.nav = next;
        for effect in effects {
            self.execute_nav(effect);
        }
    }

    /// The only place navigation effects touch CEF.
    fn execute_nav(&mut self, effect: NavEffect) {
        let result = match &effect {
            NavEffect::Emit(event) => {
                self.events.push(event.clone());
                return;
            }
            NavEffect::Load(url) => self.producer.navigate_to_url(url),
            NavEffect::Back => self.producer.go_back(),
            NavEffect::Forward => self.producer.go_forward(),
            NavEffect::Reload => self.producer.reload(),
            // welding's producer has no ignore-cache reload; CEF's browser does.
            NavEffect::HardReload => {
                if let Some(browser) = cef::browser_host_get_browser_by_identifier(self.browser_id)
                {
                    browser.reload_ignore_cache();
                }
                Ok(())
            }
            NavEffect::Stop => self.producer.stop(),
            NavEffect::FetchFavicon(url) => {
                let sink = self.favicons.clone();
                self.send(|host| {
                    crate::favicon::fetch(host, url, &sink, crate::favicon::Purpose::Icon)
                });
                Ok(())
            }
            NavEffect::Repaint => self.producer.request_repaint(),
        };
        if let Err(e) = result {
            eprintln!(
                "wubble-engine: view {}: {effect:?} failed: {e}",
                self.browser_id
            );
        }
    }

    /// Runs the transition and its CEF effects. Returns the effects meant for
    /// the caller (`Present`, `ReportClosed`).
    fn apply(&mut self, event: ViewEvent) -> Vec<ViewEffect> {
        let (next, effects) = transition(self.state, event);
        self.state = next;
        effects
            .into_iter()
            .filter(|&effect| !self.execute(effect))
            .collect()
    }

    /// The only place view effects touch CEF. Returns false for effects the
    /// caller handles.
    fn execute(&mut self, effect: ViewEffect) -> bool {
        let result = match effect {
            ViewEffect::Resize(size) => self.producer.resize(dpi_size(size)),
            ViewEffect::SetScale(scale) => self.producer.set_scale_factor(scale),
            ViewEffect::SetScreenArea(area) => self.producer.set_screen_area(area.to_welding()),
            ViewEffect::Close => self.producer.close(),
            ViewEffect::Present | ViewEffect::ReportClosed => return false,
        };
        // welding's producer calls fail only without its `cef-runtime`
        // feature, which wubble always enables; log rather than unwind.
        if let Err(e) = result {
            eprintln!(
                "wubble-engine: view {}: {effect:?} failed: {e}",
                self.browser_id
            );
        }
        true
    }
}

/// Where one welding event goes: navigation, a page event, or one of the
/// page state machines.
#[derive(Clone, Debug, PartialEq)]
enum Route {
    Nav(NavInput),
    Page(PageEvent),
    Download(DownloadId, DownloadInput),
    Find(FindInput),
    Fullscreen(FullscreenInput),
    DragStarted(PageEvent),
    /// Zoom is per host and may change on navigation without a callback.
    ObserveZoom,
    /// The renderer process ended.
    Crashed,
    /// The main frame committed a load (welding reports `LoadStart` for the
    /// main frame only).
    PageCommitted,
    /// The root scroll offset's x.
    ScrollX(f64),
    /// CEF created a window the page opened as a browser (`View::adopt_popup`).
    PopupCreated(welding::OpenedWindow),
    /// One the factory said yes to was not created after all.
    PopupAborted(i32),
}

/// Translates one welding event. Pure; events nothing uses map to nothing.
/// welding's frame message as the host's event, by its channel: the frame as
/// CEF named it. `None` for a message on no channel or a clipboard request
/// that is not one.
fn frame_message(frame: crate::page::FrameInfo, text: String) -> Option<PageEvent> {
    let (channel, body) = FrameChannel::split(&text)?;
    Some(match channel {
        FrameChannel::Autofill => PageEvent::FrameMessage {
            frame,
            text: wubble_core::secret::Secret::from(body.to_owned()),
        },
        FrameChannel::Probes => PageEvent::Probe {
            frame,
            report: body.to_owned(),
        },
        FrameChannel::Clipboard => PageEvent::Clipboard {
            frame: frame.id,
            request: clipboard_request(body)?,
        },
        // Answered by the engine itself (`View::answer_cosmetic`, `View::zoom_answer`).
        FrameChannel::Cosmetic | FrameChannel::Zoom => return None,
    })
}

/// Where `clipboard.js`'s `paste` comes from, by its text; another text is the clipboard.
const PASTES: [(&str, PasteFrom); 2] = [
    ("plain", PasteFrom::PlainText),
    ("primary", PasteFrom::Primary),
];

/// `clipboard.js`'s `kind\ntext`: a copy's text is the page JSON
/// (`wubble_core::clipboard::Payload::to_json`).
fn clipboard_request(body: &str) -> Option<ClipboardRequest> {
    use wubble_core::clipboard::{Payload, Reader};
    let (kind, text) = body.split_once('\n')?;
    // A document's nonce and a count, as `clipboard.js` makes them; nothing else is echoed.
    let token = |t: &str| {
        let fits = !t.is_empty() && t.len() <= 64;
        (fits && t.bytes().all(|b| b.is_ascii_hexdigit() || b == b'.')).then(|| t.to_owned())
    };
    Some(match kind {
        "copy" => ClipboardRequest::Copy(Payload::from_json(text)?),
        // `navigator.clipboard.write`'s markup is the page's own, never the user's selection:
        // Chromium sanitises it, and the host has no sanitiser, so it is left out.
        "write" => ClipboardRequest::Copy(Payload {
            html: None,
            ..Payload::from_json(text)?
        }),
        "image" => ClipboardRequest::CopyImage(text.into()),
        "primary" if text.len() <= wubble_core::clipboard::PRIMARY_CAP => {
            ClipboardRequest::Primary(text.into())
        }
        "paste" => {
            let (asked, from) = text.split_once(' ')?;
            ClipboardRequest::Paste(
                PASTES
                    .iter()
                    .find_map(|(t, f)| (*t == from).then_some(*f))
                    .unwrap_or(PasteFrom::Clipboard),
                token(asked)?,
            )
        }
        "read" => ClipboardRequest::Read(token(text)?, Reader::Text),
        "items" => ClipboardRequest::Read(token(text)?, Reader::AsyncApi),
        _ => return None,
    })
}

fn route(event: NavigationEvent) -> Vec<Route> {
    use NavigationEvent as E;
    let one = |r| vec![r];
    match event {
        E::AddressChanged { url } => vec![Route::Nav(NavInput::Address(url)), Route::ObserveZoom],
        E::TitleChanged { title } => one(Route::Nav(NavInput::Title(title))),
        E::LoadStart { .. } => one(Route::PageCommitted),
        E::LoadingStateChanged {
            loading,
            can_go_back,
            can_go_forward,
        } => {
            let mut out = vec![Route::Nav(NavInput::Loading {
                loading,
                can_go_back,
                can_go_forward,
            })];
            if !loading {
                out.push(Route::ObserveZoom);
            }
            out
        }
        E::FaviconUrlsChanged { urls } => one(Route::Nav(NavInput::Favicon(
            urls.into_iter().next().unwrap_or_default(),
        ))),
        E::NewWindowRequested {
            url,
            user_gesture,
            kind,
            size,
            position,
        } => one(Route::Nav(NavInput::Popup(crate::nav::PopupRequest {
            url,
            gesture: user_gesture,
            opened: popup_opened(kind),
            size,
            position,
        }))),
        E::PopupBrowserCreated(opened) => one(Route::PopupCreated(opened)),
        E::PopupAborted { popup_id } => one(Route::PopupAborted(popup_id)),
        E::ContentProcessTerminated { .. } => one(Route::Crashed),
        // ERR_ABORTED (-3) is a navigation replaced by another one.
        E::LoadError {
            url,
            error_code,
            error_text,
        } if error_code != -3 => one(Route::Page(PageEvent::LoadFailed {
            url,
            code: error_code,
            text: error_text,
        })),
        E::ConsoleMessage {
            level,
            message,
            source,
            line,
        } => one(Route::Page(PageEvent::Console {
            level: LogLevel::from_severity(level),
            message,
            source,
            line,
        })),
        E::JsDialogRequested {
            id,
            kind,
            origin_url,
            message,
            default_text,
        } => one(Route::Page(PageEvent::DialogRequested {
            id: RequestId(id),
            kind,
            origin: origin_url,
            message,
            default_text,
        })),
        E::JsDialogsReset => one(Route::Page(PageEvent::DialogsReset)),
        E::ContextMenuRequested {
            id,
            x,
            y,
            targets,
            link_url,
            source_url,
            page_url,
            selection_text,
            frame_url,
            frame_id,
            media,
            has_image_contents,
            editable,
            edit_flags,
            media_flags,
            misspelled_word,
            suggestions,
            items,
            ..
        } => one(Route::Page(PageEvent::ContextMenuRequested {
            id: RequestId(id),
            menu: Box::new(ContextMenu {
                x,
                y,
                link_url,
                media_url: source_url,
                media: media.into(),
                has_image: has_image_contents,
                page_url,
                frame_url,
                frame: frame_id,
                selection_text,
                editable: editable || targets.contains(&welding::ContextMenuTarget::Editable),
                edit: EditFlags(edit_flags),
                media_flags: MediaFlags(media_flags),
                misspelled_word,
                suggestions,
                items: items.into_iter().map(Into::into).collect(),
            }),
        })),
        E::ContextMenuDismissed => one(Route::Page(PageEvent::ContextMenuDismissed)),
        E::StatusChanged { text } => one(Route::Page(PageEvent::LinkStatus(text))),
        E::Tooltip { text } => one(Route::Page(PageEvent::Tooltip(text))),
        E::NavigationRefused { url, redirect } => {
            one(Route::Page(PageEvent::NavigationRefused { url, redirect }))
        }
        E::ProtocolExecution { url, user_gesture } => {
            one(Route::Page(PageEvent::ExternalProtocol {
                url,
                gesture: user_gesture,
            }))
        }
        E::PermissionRequested {
            id,
            origin,
            permissions,
            ..
        } => one(Route::Page(PageEvent::PermissionRequested {
            id: RequestId(id),
            origin,
            permissions,
        })),
        E::AuthChallenged {
            id,
            origin_url,
            host,
            port,
            realm,
            scheme,
            is_proxy,
        } => one(Route::Page(PageEvent::AuthRequested {
            id: RequestId(id),
            url: origin_url,
            host,
            port,
            realm,
            scheme,
            proxy: is_proxy,
        })),
        E::CertificateError {
            id,
            url,
            error_code,
        } => one(Route::Page(PageEvent::CertificateError {
            id: RequestId(id),
            url,
            code: error_code,
        })),
        E::PdfPrintFinished { path, ok } => one(Route::Page(PageEvent::PdfPrinted { path, ok })),
        E::RequestsDismissed { ids } => one(Route::Page(PageEvent::RequestsDismissed(
            ids.into_iter().map(RequestId).collect(),
        ))),
        E::FileChooserRequested {
            id,
            mode,
            title,
            default_path,
            accept,
        } => one(Route::Page(PageEvent::FileChooserRequested {
            id: RequestId(id),
            mode,
            title,
            default_path,
            accept,
        })),
        E::DownloadRequested {
            id,
            download,
            url,
            original_url,
            suggested_filename,
            total_bytes_expected,
        } => one(Route::Download(
            DownloadId(download),
            DownloadInput::Offered {
                request: RequestId(id),
                url,
                original_url,
                suggested_name: suggested_filename,
                total: total_bytes_expected,
            },
        )),
        E::DownloadStarted {
            id,
            url,
            suggested_filename,
            destination_path,
            total_bytes_expected,
        } => one(Route::Download(
            DownloadId(id),
            DownloadInput::Started {
                url,
                suggested_name: suggested_filename,
                path: destination_path,
                total: total_bytes_expected,
            },
        )),
        E::DownloadProgress {
            id,
            bytes_received,
            total_bytes_expected,
        } => one(Route::Download(
            DownloadId(id),
            DownloadInput::Progress {
                received: bytes_received,
                total: total_bytes_expected,
            },
        )),
        E::DownloadFinished {
            id,
            destination_path,
            error,
        } => one(Route::Download(
            DownloadId(id),
            match error {
                None => DownloadInput::Done {
                    path: destination_path,
                },
                Some(error) => DownloadInput::Interrupted { error },
            },
        )),
        E::DownloadInterrupted { id, reason } => one(Route::Download(
            DownloadId(id),
            DownloadInput::Interrupted {
                error: crate::download::interrupt_reason_text(reason),
            },
        )),
        E::DownloadCancelled { id, .. } => {
            one(Route::Download(DownloadId(id), DownloadInput::Cancelled))
        }
        E::FindResult {
            count,
            active_match,
            final_update,
        } => one(Route::Find(FindInput::Result {
            count,
            active: active_match,
            done: final_update,
        })),
        E::FullscreenChanged { fullscreen } => {
            one(Route::Fullscreen(FullscreenInput::Changed(fullscreen)))
        }
        E::DragStarted {
            payload,
            allowed_operations,
            x,
            y,
        } => one(Route::DragStarted(PageEvent::DragStarted {
            data: payload.into(),
            allowed: DragOps(allowed_operations.0),
            x,
            y,
        })),
        E::DragCursor { operation } => {
            one(Route::Page(PageEvent::DragCursor(DragOps(operation.0))))
        }
        E::ScrollOffset { x, .. } => one(Route::ScrollX(x)),
        E::AccessibilityTree { json } => one(Route::Page(PageEvent::AxTree(json))),
        E::AccessibilityLocation { json } => one(Route::Page(PageEvent::AxLocation(json))),
        _ => vec![],
    }
}

/// welding's modifier set for drags.
/// What the page asked for, in the popup policy's terms. Anything else opens as a tab would.
fn popup_opened(kind: welding::NewWindowKind) -> wubble_core::popup::Opened {
    use welding::NewWindowKind as K;
    use wubble_core::popup::Opened;
    match kind {
        K::BackgroundTab => Opened::BackgroundTab,
        K::Popup => Opened::PopupWindow,
        K::Window => Opened::Window,
        K::ForegroundTab | K::Other => Opened::ForegroundTab,
    }
}

fn weld_modifiers(m: &Modifiers) -> welding::EventModifiers {
    welding::EventModifiers {
        shift: m.keys.shift_key(),
        ctrl: m.keys.control_key(),
        alt: m.keys.alt_key(),
        meta: m.keys.super_key(),
        left_mouse_button: m.buttons.left,
        middle_mouse_button: m.buttons.middle,
        right_mouse_button: m.buttons.right,
    }
}

/// The navigation guard reading `rule`.
fn guard(rule: &NavigationRule) -> welding::NavigationGuard {
    let read = Arc::clone(rule);
    welding::NavigationGuard::new(move |url| {
        let rule = read.read().unwrap_or_else(|e| e.into_inner());
        rule.as_ref().is_none_or(|allows| allows(url))
    })
}

/// A view's welding config but its URL and profile: the engine's views and their pop-ups'.
fn surface_config(
    hooks: &PopupHooks,
    navigation_guard: welding::NavigationGuard,
    popup_factory: welding::PopupFactory,
) -> welding::CefSurfaceConfig {
    let (frame_rate, begin_frames) = hooks.pacing;
    welding::CefSurfaceConfig {
        initial_size: dpi_size(hooks.layout.size),
        scale_factor: hooks.layout.scale,
        windowless_frame_rate: frame_rate,
        external_begin_frame: begin_frames,
        // Every request is held for the engine, which answers it with
        // the host's answer or (RequestMode::DefaultNow) its default.
        handle_permission_requests: true,
        handle_auth_challenges: true,
        ask_download_path: true,
        request_filter: hooks.request_filter.clone(),
        navigation_guard: Some(navigation_guard),
        // CDP: `View::set_frozen`, `View::cdp` (page dark mode, screenshots).
        devtools_protocol: true,
        response_observer: hooks.response_observer.clone(),
        popup_factory: Some(popup_factory),
        ..welding::CefSurfaceConfig::default()
    }
}

/// Answers each window the page opens, on CEF's UI thread: while the view allows it and
/// `wubble_core::popup::creates_browser` says so, CEF creates it as a browser of its own with a
/// view's config, whose navigation rule starts as its opener's (an agent session's pop-up is
/// held to the session's sites from its first navigation). Its hooks wait in `popups` for
/// [`View::adopt_popup`]. A link to another app stays a request (the host hands it over).
fn popup_factory(
    hooks: &PopupHooks,
    opener_rule: &NavigationRule,
    popups: &Popups,
) -> welding::PopupFactory {
    let (hooks, opener_rule, popups) = (hooks.clone(), Arc::clone(opener_rule), popups.clone());
    welding::PopupFactory::new(move |opened| {
        let wanted = popups.on.load(std::sync::atomic::Ordering::Relaxed)
            && wubble_core::popup::creates_browser(popup_opened(opened.kind), opened.user_gesture)
            && !crate::page::is_external(&opened.url);
        if !wanted {
            return None;
        }
        let inherited = opener_rule
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let rule: NavigationRule = Arc::new(std::sync::RwLock::new(inherited));
        let own = Popups::default();
        let config = surface_config(&hooks, guard(&rule), popup_factory(&hooks, &rule, &own));
        popups
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                opened.popup_id,
                Pending {
                    rule,
                    popups: own,
                    layout: hooks.layout,
                },
            );
        Some(config)
    })
}

#[cfg(not(windows))]
fn new_producer(
    runtime: &CefRuntime,
    surface: welding::CefSurfaceConfig,
    gpu: &HostWgpuContext,
) -> Result<PlatformCefProducer, EngineError> {
    let config = welding::PlatformCefConfig { surface };
    // Linux and macOS copy each frame inside the paint callback on `gpu`.
    PlatformCefProducer::new(runtime, config, gpu).map_err(|e| EngineError::View(e.to_string()))
}

/// welding's Windows producer also needs the host's `HostWgpuContext`, and
/// wubble-gfx has no Windows import path yet (D3D11, M0 Windows item).
#[cfg(windows)]
fn new_producer(
    _runtime: &CefRuntime,
    _surface: welding::CefSurfaceConfig,
    _gpu: &HostWgpuContext,
) -> Result<PlatformCefProducer, EngineError> {
    Err(EngineError::View(
        "windowless views are not supported on Windows yet".into(),
    ))
}

fn dpi_size(size: FrameSize) -> dpi::PhysicalSize<u32> {
    dpi::PhysicalSize::new(size.width, size.height)
}

#[cfg(test)]
mod tests {
    use super::{ViewEffect::*, ViewEvent::*, ViewState::*, *};

    const SMALL: FrameSize = FrameSize::new(800, 600);
    const BIG: FrameSize = FrameSize::new(1024, 768);

    fn layout(size: FrameSize, scale: f32) -> super::Layout {
        super::Layout {
            size,
            scale,
            area: None,
        }
    }

    const AREA: ScreenArea = ScreenArea {
        window: FrameSize::new(1000, 700),
        origin: (10, 20),
        desktop: None,
    };
    const OTHER_AREA: ScreenArea = ScreenArea {
        window: FrameSize::new(1000, 700),
        origin: (30, 20),
        desktop: None,
    };
    /// The same view after the window moved on the desktop.
    const MOVED_AREA: ScreenArea = ScreenArea {
        desktop: Some(Desktop {
            monitor_origin: (0, 0),
            monitor_size: FrameSize::new(1920, 1080),
            window_origin: Some((300, 200)),
        }),
        ..AREA
    };

    fn placed(size: FrameSize, scale: f32, area: ScreenArea) -> super::Layout {
        super::Layout {
            area: Some(area),
            ..layout(size, scale)
        }
    }

    fn live() -> ViewState {
        Live {
            layout: layout(SMALL, 1.0),
        }
    }

    fn states() -> Vec<ViewState> {
        vec![
            live(),
            Resizing {
                wanted: layout(BIG, 1.0),
            },
            Closing,
            Closed,
        ]
    }

    fn events() -> Vec<ViewEvent> {
        vec![
            Layout(layout(SMALL, 1.0)),
            Layout(layout(BIG, 1.0)),
            Layout(layout(SMALL, 2.0)),
            Layout(placed(SMALL, 1.0, AREA)),
            Layout(placed(SMALL, 1.0, OTHER_AREA)),
            Layout(layout(FrameSize::new(0, 0), 1.0)),
            Frame(SMALL),
            Frame(BIG),
            CloseRequested,
            GoneFromCef,
        ]
    }

    fn expected(state: ViewState, event: ViewEvent) -> (ViewState, Vec<ViewEffect>) {
        let resizing = |size, scale| Resizing {
            wanted: layout(size, scale),
        };
        let one = FrameSize::new(1, 1);
        match (state, event) {
            (Live { .. }, Layout(l)) if l == layout(SMALL, 1.0) => (state, vec![]),
            (Live { .. }, Layout(l)) if l == layout(BIG, 1.0) => {
                (resizing(BIG, 1.0), vec![Resize(BIG)])
            }
            (Live { .. }, Layout(l)) if l == layout(SMALL, 2.0) => {
                (resizing(SMALL, 2.0), vec![SetScale(2.0)])
            }
            // A new area is announced; layouts without one never are.
            (Live { .. }, Layout(l)) if l == placed(SMALL, 1.0, AREA) => {
                (Live { layout: l }, vec![SetScreenArea(AREA)])
            }
            (Live { .. }, Layout(l)) if l == placed(SMALL, 1.0, OTHER_AREA) => {
                (Live { layout: l }, vec![SetScreenArea(OTHER_AREA)])
            }
            (Resizing { .. }, Layout(l)) if l == placed(SMALL, 1.0, AREA) => (
                Resizing { wanted: l },
                vec![Resize(SMALL), SetScreenArea(AREA)],
            ),
            (Resizing { .. }, Layout(l)) if l == placed(SMALL, 1.0, OTHER_AREA) => (
                Resizing { wanted: l },
                vec![Resize(SMALL), SetScreenArea(OTHER_AREA)],
            ),
            (Live { .. }, Frame(_)) => (state, vec![Present]),
            (Live { .. } | Resizing { .. }, CloseRequested) => (Closing, vec![Close]),
            (Resizing { .. }, Layout(l)) if l == layout(SMALL, 1.0) => {
                (resizing(SMALL, 1.0), vec![Resize(SMALL)])
            }
            (Resizing { .. }, Layout(l)) if l == layout(BIG, 1.0) => (state, vec![]),
            (Resizing { .. }, Layout(l)) if l == layout(SMALL, 2.0) => {
                (resizing(SMALL, 2.0), vec![Resize(SMALL), SetScale(2.0)])
            }
            // The zero size, clamped.
            (Live { .. } | Resizing { .. }, Layout(_)) => (resizing(one, 1.0), vec![Resize(one)]),
            (Resizing { .. }, Frame(BIG)) => (live_at(BIG), vec![Present]),
            (Resizing { .. }, Frame(_)) => (state, vec![Present]),
            (Live { .. } | Resizing { .. } | Closing, GoneFromCef) => (Closed, vec![ReportClosed]),
            _ => (state, vec![]),
        }
    }

    fn live_at(size: FrameSize) -> ViewState {
        Live {
            layout: layout(size, 1.0),
        }
    }

    #[test]
    fn table_covers_every_pair() {
        for state in states() {
            for event in events() {
                assert_eq!(
                    transition(state, event),
                    expected(state, event),
                    "{state:?} + {event:?}"
                );
            }
        }
    }

    fn run(start: ViewState, events: &[ViewEvent]) -> (ViewState, Vec<ViewEffect>) {
        events
            .iter()
            .fold((start, vec![]), |(state, mut all), &event| {
                let (next, effects) = transition(state, event);
                all.extend(effects);
                (next, all)
            })
    }

    #[test]
    fn the_screen_area_is_announced_only_when_it_changes() {
        let a = Layout(placed(SMALL, 1.0, AREA));
        let (state, effects) = run(live(), &[a, a]);
        assert_eq!(effects, vec![SetScreenArea(AREA)]);
        let b = Layout(placed(SMALL, 1.0, OTHER_AREA));
        let (_, effects) = run(state, &[b, b]);
        assert_eq!(effects, vec![SetScreenArea(OTHER_AREA)]);
        // A layout that does not say where the view is keeps what CEF has.
        let (_, effects) = run(state, &[Layout(layout(SMALL, 1.0))]);
        assert_eq!(effects, vec![]);
    }

    #[test]
    fn a_window_moved_on_the_desktop_is_announced_without_a_resize() {
        let (state, _) = run(live(), &[Layout(placed(SMALL, 1.0, AREA))]);
        let (state, effects) = run(state, &[Layout(placed(SMALL, 1.0, MOVED_AREA))]);
        assert_eq!(effects, vec![SetScreenArea(MOVED_AREA)]);
        assert!(matches!(state, Live { .. }));
    }

    #[test]
    fn moving_the_view_does_not_leave_live() {
        let (state, _) = run(live(), &[Layout(placed(SMALL, 1.0, AREA))]);
        assert!(matches!(state, Live { .. }));
        let (state, _) = run(state, &[Layout(placed(BIG, 1.0, AREA))]);
        assert!(matches!(state, Resizing { .. }));
    }

    #[test]
    fn repeated_layouts_resize_once() {
        let big = Layout(layout(BIG, 1.0));
        let (state, effects) = run(live(), &[big, big, big]);
        assert_eq!(
            state,
            Resizing {
                wanted: layout(BIG, 1.0)
            }
        );
        assert_eq!(effects, vec![Resize(BIG)]);
    }

    #[test]
    fn a_second_resize_replaces_the_pending_one() {
        let other = FrameSize::new(640, 480);
        let (state, effects) = run(
            live(),
            &[
                Layout(layout(BIG, 1.0)),
                Layout(layout(other, 1.0)),
                Frame(BIG),
            ],
        );
        assert_eq!(
            state,
            Resizing {
                wanted: layout(other, 1.0)
            }
        );
        assert_eq!(effects, vec![Resize(BIG), Resize(other), Present]);
        assert_eq!(transition(state, Frame(other)).0, live_at(other));
    }

    #[test]
    fn stale_frames_are_presented_until_the_size_matches() {
        let (state, effects) = run(
            live(),
            &[Layout(layout(BIG, 1.0)), Frame(SMALL), Frame(SMALL)],
        );
        assert_eq!(
            state,
            Resizing {
                wanted: layout(BIG, 1.0)
            }
        );
        assert_eq!(effects, vec![Resize(BIG), Present, Present]);
        assert_eq!(transition(state, Frame(BIG)), (live_at(BIG), vec![Present]));
    }

    #[test]
    fn frame_size_matches_what_cef_paints() {
        let cases = [
            // (physical, scale) -> CEF frame
            ((800, 600), 1.0, (800, 600)),
            ((800, 600), 2.0, (800, 600)),
            ((1001, 600), 2.0, (1002, 600)),
            ((1001, 601), 1.25, (1002, 602)),
            ((1000, 750), 1.25, (1000, 750)),
            ((1000, 777), 1.5, (1001, 777)),
            ((1, 1), 2.0, (2, 2)),
            // Above scale 2 CEF can paint a pixel short of the rectangle
            // (333 DIP at 3.0 is 999 px), so the shell fills it with a
            // current frame instead of showing it at its own size.
            ((1000, 600), 3.0, (999, 600)),
            ((1001, 600), 2.5, (1000, 600)),
        ];
        for ((w, h), scale, (fw, fh)) in cases {
            let layout = layout(FrameSize::new(w, h), scale);
            assert_eq!(layout.frame_size(), FrameSize::new(fw, fh), "{layout:?}");
        }
    }

    #[test]
    fn a_frame_a_pixel_short_of_the_rectangle_is_current() {
        let wanted = layout(FrameSize::new(1000, 600), 3.0);
        let (state, _) = run(live(), &[Layout(wanted)]);
        assert_eq!(
            transition(state, Frame(FrameSize::new(999, 600))),
            (Live { layout: wanted }, vec![Present])
        );
    }

    #[test]
    fn resizing_ends_on_the_frame_cef_paints_at_fractional_scale() {
        let wanted = layout(FrameSize::new(1001, 601), 1.25);
        let (state, _) = run(live(), &[Layout(wanted)]);
        assert_eq!(state, Resizing { wanted });
        assert_eq!(
            transition(state, Frame(FrameSize::new(1002, 602))),
            (Live { layout: wanted }, vec![Present])
        );
    }

    #[test]
    fn closing_ignores_layout_and_frames() {
        for event in [Layout(layout(BIG, 2.0)), Frame(SMALL), CloseRequested] {
            assert_eq!(transition(Closing, event), (Closing, vec![]));
        }
    }

    #[test]
    fn report_closed_exactly_once_on_every_path() {
        let paths: [&[ViewEvent]; 3] = [
            &[CloseRequested, GoneFromCef, GoneFromCef],
            &[
                Layout(layout(BIG, 1.0)),
                CloseRequested,
                Frame(BIG),
                GoneFromCef,
            ],
            &[
                CloseRequested,
                CloseRequested,
                GoneFromCef,
                CloseRequested,
                GoneFromCef,
            ],
        ];
        for path in paths {
            let (state, effects) = run(live(), path);
            assert_eq!(state, Closed, "{path:?}");
            let reports = effects.iter().filter(|e| **e == ReportClosed).count();
            let closes = effects.iter().filter(|e| **e == Close).count();
            assert_eq!((reports, closes), (1, 1), "{path:?}");
        }
    }

    #[test]
    fn a_frame_message_carries_cefs_frame() {
        let info = crate::page::FrameInfo {
            id: "7-3".into(),
            url: "https://a.test/f".into(),
            is_main: false,
            pid: 4242,
        };
        let event = frame_message(info.clone(), "autofill\n{}".into()).unwrap();
        let PageEvent::FrameMessage { frame, text } = &event else {
            panic!("{event:?}");
        };
        assert_eq!(frame, &info);
        assert_eq!(text.expose(), "{}");
        assert!(event.request().is_none());
        // The text never shows in diagnostics.
        let secret = frame_message(info, "autofill\npw-value".into());
        assert!(!format!("{secret:?}").contains("pw-value"));
    }

    #[test]
    fn clipboard_messages_become_clipboard_requests() {
        let at = |text: &str| {
            let frame = crate::page::FrameInfo {
                id: "7".into(),
                url: "u".into(),
                is_main: true,
                pid: 0,
            };
            frame_message(frame, text.into())
        };
        let request = |text: &str| match at(text) {
            Some(PageEvent::Clipboard { frame, request }) => {
                assert_eq!(frame, "7");
                Some(request)
            }
            other => {
                assert_eq!(other, None);
                None
            }
        };
        use wubble_core::clipboard::{Payload, Reader};
        let copied = Payload {
            text: Some("a\nb".into()),
            ..Payload::default()
        };
        let cases = [
            (
                "clipboard\ncopy\n{\"text\":\"a\\nb\"}",
                Some(ClipboardRequest::Copy(copied)),
            ),
            ("clipboard\ncopy\na\nb", None),
            (
                "clipboard\nwrite\n{\"text\":\"a\\nb\",\"html\":\"<b>x</b>\"}",
                Some(ClipboardRequest::Copy(Payload {
                    text: Some("a\nb".into()),
                    ..Payload::default()
                })),
            ),
            (
                "clipboard\nimage\nhttps://a.test/c.png",
                Some(ClipboardRequest::CopyImage("https://a.test/c.png".into())),
            ),
            (
                "clipboard\nprimary\na",
                Some(ClipboardRequest::Primary("a".into())),
            ),
            (
                "clipboard\npaste\nab12.3 primary",
                Some(ClipboardRequest::Paste(PasteFrom::Primary, "ab12.3".into())),
            ),
            (
                "clipboard\npaste\nab12.3 plain",
                Some(ClipboardRequest::Paste(
                    PasteFrom::PlainText,
                    "ab12.3".into(),
                )),
            ),
            (
                "clipboard\npaste\nab12.3 ",
                Some(ClipboardRequest::Paste(
                    PasteFrom::Clipboard,
                    "ab12.3".into(),
                )),
            ),
            // a paste names its document
            ("clipboard\npaste\nprimary", None),
            ("clipboard\npaste\n plain", None),
            ("clipboard\nread\nab12.3\nx", None),
            (
                "clipboard\nread\nab12.4",
                Some(ClipboardRequest::Read("ab12.4".into(), Reader::Text)),
            ),
            (
                "clipboard\nitems\nab12.5",
                Some(ClipboardRequest::Read("ab12.5".into(), Reader::AsyncApi)),
            ),
            ("clipboard\nread\nx", None),
            ("clipboard\nread\n", None),
            (&*format!("clipboard\nread\n{}", "a".repeat(65)), None),
            (
                &*format!(
                    "clipboard\nprimary\n{}",
                    "a".repeat(wubble_core::clipboard::PRIMARY_CAP + 1)
                ),
                None,
            ),
            ("clipboard\nformat\nc:", None),
        ];
        for (text, want) in cases {
            assert_eq!(request(text), want, "{text}");
        }
        // `zoom.js`'s answers are the view's own (`View::zoom_answer`, `pinch::Answer`).
        assert_eq!(at("zoom\nin"), None);
        // No channel, or an unknown one: nothing.
        assert_eq!(at("{}"), None);
        assert_eq!(at("other\n{}"), None);
    }

    #[test]
    fn channels_round_trip() {
        for channel in [
            FrameChannel::Autofill,
            FrameChannel::Clipboard,
            FrameChannel::Zoom,
            FrameChannel::Probes,
            FrameChannel::Cosmetic,
        ] {
            assert_eq!(
                FrameChannel::split(&channel.join("x\ny")),
                Some((channel, "x\ny"))
            );
        }
    }

    fn route_one(event: NavigationEvent) -> Route {
        let mut routed = route(event);
        assert_eq!(routed.len(), 1, "{routed:?}");
        routed.remove(0)
    }

    #[test]
    fn popups_carry_the_gesture_kind_and_size() {
        use wubble_core::popup::Opened;
        let ask = |kind, size| {
            route_one(NavigationEvent::NewWindowRequested {
                url: "https://p.test/".into(),
                user_gesture: true,
                kind,
                size,
                position: Some((40, 60)),
            })
        };
        assert_eq!(
            ask(welding::NewWindowKind::Popup, Some((400, 300))),
            Route::Nav(NavInput::Popup(crate::nav::PopupRequest {
                url: "https://p.test/".into(),
                gesture: true,
                opened: Opened::PopupWindow,
                size: Some((400, 300)),
                position: Some((40, 60)),
            }))
        );
        for (kind, opened) in [
            (welding::NewWindowKind::ForegroundTab, Opened::ForegroundTab),
            (welding::NewWindowKind::BackgroundTab, Opened::BackgroundTab),
            (welding::NewWindowKind::Window, Opened::Window),
            (welding::NewWindowKind::Other, Opened::ForegroundTab),
        ] {
            assert_eq!(popup_opened(kind), opened);
        }
    }

    #[test]
    fn a_created_popup_browser_is_adopted() {
        let opened = welding::OpenedWindow {
            popup_id: 7,
            url: String::new(),
            target: "signin".into(),
            kind: welding::NewWindowKind::Popup,
            user_gesture: true,
            size: Some((500, 600)),
            position: None,
        };
        assert_eq!(
            route_one(NavigationEvent::PopupBrowserCreated(opened.clone())),
            Route::PopupCreated(opened)
        );
        assert_eq!(
            route_one(NavigationEvent::PopupAborted { popup_id: 7 }),
            Route::PopupAborted(7)
        );
    }

    #[test]
    fn pdf_completion_routes_to_a_page_event() {
        assert_eq!(
            route_one(NavigationEvent::PdfPrintFinished {
                path: "/tmp/a.pdf".into(),
                ok: true
            }),
            Route::Page(PageEvent::PdfPrinted {
                path: "/tmp/a.pdf".into(),
                ok: true
            })
        );
    }

    #[test]
    fn favicon_images_are_not_requests() {
        let icon = PageEvent::FaviconImage(crate::favicon::FaviconImage {
            url: "u".into(),
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        });
        assert_eq!(icon.request(), None);
    }

    #[test]
    fn requests_route_to_page_events_with_their_ids() {
        assert_eq!(
            route_one(NavigationEvent::JsDialogRequested {
                id: 3,
                kind: welding::JsDialogKind::Prompt,
                origin_url: "https://a/".into(),
                message: "name?".into(),
                default_text: "x".into(),
            }),
            Route::Page(PageEvent::DialogRequested {
                id: RequestId(3),
                kind: welding::JsDialogKind::Prompt,
                origin: "https://a/".into(),
                message: "name?".into(),
                default_text: "x".into(),
            })
        );
        assert_eq!(
            route_one(NavigationEvent::CertificateError {
                id: 4,
                url: "https://b/".into(),
                error_code: -202,
            }),
            Route::Page(PageEvent::CertificateError {
                id: RequestId(4),
                url: "https://b/".into(),
                code: -202,
            })
        );
        assert_eq!(
            route_one(NavigationEvent::PermissionRequested {
                id: 5,
                origin: "https://c/".into(),
                permissions: vec![welding::PermissionKind::Geolocation],
                raw: 0x100,
            }),
            Route::Page(PageEvent::PermissionRequested {
                id: RequestId(5),
                origin: "https://c/".into(),
                permissions: vec![PermissionKind::Geolocation],
            })
        );
        assert_eq!(
            route_one(NavigationEvent::AuthChallenged {
                id: 6,
                origin_url: "https://d/".into(),
                host: "d".into(),
                port: 443,
                realm: "r".into(),
                scheme: "basic".into(),
                is_proxy: false,
            }),
            Route::Page(PageEvent::AuthRequested {
                id: RequestId(6),
                url: "https://d/".into(),
                host: "d".into(),
                port: 443,
                realm: "r".into(),
                scheme: "basic".into(),
                proxy: false,
            })
        );
        assert_eq!(
            route_one(NavigationEvent::FileChooserRequested {
                id: 7,
                mode: welding::FileChooserMode::OpenMultiple,
                title: String::new(),
                default_path: PathBuf::new(),
                accept: vec![welding::AcceptFilter {
                    filter: "image/*".into(),
                    extensions: vec![".png".into()],
                    description: String::new(),
                }],
            }),
            Route::Page(PageEvent::FileChooserRequested {
                id: RequestId(7),
                mode: welding::FileChooserMode::OpenMultiple,
                title: String::new(),
                default_path: PathBuf::new(),
                accept: vec![welding::AcceptFilter {
                    filter: "image/*".into(),
                    extensions: vec![".png".into()],
                    description: String::new(),
                }],
            })
        );
    }

    #[test]
    fn a_context_menu_carries_its_items_and_flags() {
        let routed = route_one(NavigationEvent::ContextMenuRequested {
            id: 8,
            x: 10,
            y: 20,
            targets: vec![
                welding::ContextMenuTarget::Page,
                welding::ContextMenuTarget::Editable,
            ],
            link_url: "https://l/".into(),
            source_url: "https://i.png".into(),
            page_url: "https://p/".into(),
            selection_text: "sel".into(),
            frame_url: "https://p/".into(),
            frame_id: "7-ABC".into(),
            media: welding::ContextMenuMedia::Image,
            has_image_contents: true,
            editable: false,
            edit_flags: EditFlags::CAN_PASTE,
            media_flags: 0,
            misspelled_word: "teh".into(),
            suggestions: vec!["the".into()],
            items: vec![welding::MenuItem {
                command_id: 114,
                label: "Paste".into(),
                kind: welding::MenuItemKind::Command,
                enabled: true,
                checked: false,
            }],
        });
        let Route::Page(PageEvent::ContextMenuRequested { id, menu }) = routed else {
            panic!("{routed:?}")
        };
        assert_eq!(id, RequestId(8));
        assert_eq!((menu.x, menu.y), (10, 20));
        assert_eq!(menu.media_url, "https://i.png");
        assert_eq!(menu.frame, "7-ABC");
        assert_eq!(menu.media, crate::page::MediaKind::Image);
        assert!(menu.editable, "the Editable target counts as editable");
        assert!(menu.edit.allows(EditFlags::CAN_PASTE));
        assert_eq!(menu.suggestions, vec!["the".to_string()]);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].command, 114);
    }

    #[test]
    fn status_tooltip_and_external_links_are_page_events() {
        assert_eq!(
            route_one(NavigationEvent::StatusChanged {
                text: "https://a/".into()
            }),
            Route::Page(PageEvent::LinkStatus("https://a/".into()))
        );
        assert_eq!(
            route_one(NavigationEvent::Tooltip {
                text: String::new()
            }),
            Route::Page(PageEvent::Tooltip(String::new()))
        );
        assert_eq!(
            route_one(NavigationEvent::ProtocolExecution {
                url: "mailto:a@b".into(),
                user_gesture: Some(true),
            }),
            Route::Page(PageEvent::ExternalProtocol {
                url: "mailto:a@b".into(),
                gesture: Some(true),
            })
        );
    }

    /// A main-frame commit is reported, the end of the load is not.
    #[test]
    fn a_main_frame_commit_is_reported() {
        assert_eq!(
            route_one(NavigationEvent::LoadStart {
                url: "https://site.test/".into()
            }),
            Route::PageCommitted
        );
        assert!(
            route(NavigationEvent::LoadEnd {
                url: "https://site.test/".into(),
                http_status: 200,
            })
            .is_empty()
        );
    }

    #[test]
    fn stateful_events_route_to_their_machines() {
        assert_eq!(
            route_one(NavigationEvent::DownloadRequested {
                id: 2,
                download: 11,
                url: "u".into(),
                original_url: "o".into(),
                suggested_filename: "f".into(),
                total_bytes_expected: None,
            }),
            Route::Download(
                DownloadId(11),
                DownloadInput::Offered {
                    request: RequestId(2),
                    url: "u".into(),
                    original_url: "o".into(),
                    suggested_name: "f".into(),
                    total: None,
                }
            )
        );
        assert_eq!(
            route_one(NavigationEvent::DownloadProgress {
                id: 11,
                bytes_received: 5,
                total_bytes_expected: Some(9),
            }),
            Route::Download(
                DownloadId(11),
                DownloadInput::Progress {
                    received: 5,
                    total: Some(9)
                }
            )
        );
        assert_eq!(
            route_one(NavigationEvent::DownloadCancelled {
                id: 11,
                destination_path: PathBuf::new(),
            }),
            Route::Download(DownloadId(11), DownloadInput::Cancelled)
        );
        assert_eq!(
            route_one(NavigationEvent::FindResult {
                count: 3,
                active_match: 1,
                final_update: true,
            }),
            Route::Find(FindInput::Result {
                count: 3,
                active: 1,
                done: true
            })
        );
        assert_eq!(
            route_one(NavigationEvent::FullscreenChanged { fullscreen: true }),
            Route::Fullscreen(FullscreenInput::Changed(true))
        );
        assert_eq!(
            route_one(NavigationEvent::DownloadInterrupted { id: 11, reason: 20 }),
            Route::Download(
                DownloadId(11),
                DownloadInput::Interrupted {
                    error: "Network error".into()
                }
            )
        );
        assert_eq!(
            route_one(NavigationEvent::RequestsDismissed { ids: vec![3, 4] }),
            Route::Page(PageEvent::RequestsDismissed(vec![
                RequestId(3),
                RequestId(4)
            ]))
        );
        assert_eq!(
            route_one(NavigationEvent::JsDialogsReset),
            Route::Page(PageEvent::DialogsReset)
        );
        assert_eq!(
            route_one(NavigationEvent::ContextMenuDismissed),
            Route::Page(PageEvent::ContextMenuDismissed)
        );
    }

    #[test]
    fn console_and_load_errors_are_reported_but_not_aborts() {
        assert_eq!(
            route_one(NavigationEvent::ConsoleMessage {
                level: 4,
                message: "boom".into(),
                source: "s.js".into(),
                line: 7,
            }),
            Route::Page(PageEvent::Console {
                level: LogLevel::Error,
                message: "boom".into(),
                source: "s.js".into(),
                line: 7,
            })
        );
        assert_eq!(
            route_one(NavigationEvent::LoadError {
                url: "https://x/".into(),
                error_code: -105,
                error_text: "NAME_NOT_RESOLVED".into(),
            }),
            Route::Page(PageEvent::LoadFailed {
                url: "https://x/".into(),
                code: -105,
                text: "NAME_NOT_RESOLVED".into(),
            })
        );
        assert!(
            route(NavigationEvent::LoadError {
                url: "https://x/".into(),
                error_code: -3,
                error_text: "ABORTED".into(),
            })
            .is_empty()
        );
    }

    #[test]
    fn navigation_that_can_change_zoom_rereads_it() {
        assert_eq!(
            route(NavigationEvent::AddressChanged { url: "u".into() }),
            vec![
                Route::Nav(NavInput::Address("u".into())),
                Route::ObserveZoom
            ]
        );
        let loading = |loading| NavigationEvent::LoadingStateChanged {
            loading,
            can_go_back: false,
            can_go_forward: false,
        };
        assert_eq!(route(loading(true)).len(), 1);
        assert_eq!(route(loading(false)).last(), Some(&Route::ObserveZoom));
    }

    #[test]
    fn a_page_drag_carries_its_data() {
        let routed = route_one(NavigationEvent::DragStarted {
            payload: welding::DragPayload {
                link_url: Some("https://l/".into()),
                ..Default::default()
            },
            allowed_operations: welding::DragOperations::COPY,
            x: 3,
            y: 4,
        });
        assert_eq!(
            routed,
            Route::DragStarted(PageEvent::DragStarted {
                data: DragData {
                    link_url: Some("https://l/".into()),
                    ..Default::default()
                },
                allowed: DragOps::COPY,
                x: 3,
                y: 4,
            })
        );
    }

    /// A page that closes itself (`window.close()`) is reported without a
    /// close request, and nothing asks CEF to close it again.
    #[test]
    fn a_page_initiated_close_is_reported_once() {
        let resizing = Resizing {
            wanted: layout(BIG, 1.0),
        };
        for start in [live(), resizing] {
            let (state, effects) = run(start, &[GoneFromCef, CloseRequested, GoneFromCef]);
            assert_eq!((state, effects), (Closed, vec![ReportClosed]), "{start:?}");
        }
    }
}
