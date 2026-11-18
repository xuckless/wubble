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
use std::time::Duration;

use cef::{ImplBrowser, ImplBrowserHost, ImplFrame};
use welding::{
    CefRuntime, CefSurfaceEvent, CefSurfaceProducer, HostWgpuContext, NavigationEvent,
    PlatformCefProducer, WeldError,
};
use winit::keyboard::PhysicalKey;
use wubble_gfx::{FrameSize, NativeFrame};

use crate::app::{EngineError, RequestMode};
use crate::download::{self, Download, DownloadEffect, DownloadInput};
use crate::drag::{self, DragSource, DragSourceInput};
use crate::find::{self, Find, FindEffect, FindInput};
use crate::input::{
    self, CefKey, ClickTracker, KeyId, KeyInput, Modifiers, MouseButton, Point, WheelRemainder,
};
use crate::media::{self, Fullscreen, FullscreenInput, MediaEffect};
use crate::nav::{self, NavEffect, NavInput, NavState, WebEvent};
use crate::page::{
    ContextMenu, DownloadId, DragData, DragOps, EditCommand, EditFlags, LogLevel, PageEvent,
    PermissionKind, Rect, RequestId,
};
use crate::popup::{self, PopupWidget};
use crate::zoom::{self, Zoom};

/// Physical size and device scale of the rectangle a view fills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub size: FrameSize,
    pub scale: f32,
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
            (Resizing { wanted: new }, effects)
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
    /// The device frames are shown on. On Linux welding copies each frame
    /// onto it inside CEF's paint callback, before CEF reuses the buffer.
    pub gpu: HostWgpuContext,
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
    zoom: Zoom,
    drag: DragSource,
}

/// The second frame stream: the popup widget's newest frame and where to
/// draw it (physical pixels, relative to the view; may leave the view).
pub struct PopupLayer {
    pub frame: NativeFrame,
    pub rect: Rect,
}

/// Where a drag over the view is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// The drag entered, carrying what is dragged.
    Enter(DragData),
    Over,
    Leave,
    Drop,
}

impl View {
    pub(crate) fn open(runtime: &CefRuntime, config: ViewConfig) -> Result<Self, EngineError> {
        let layout = config.layout.clamped();
        let producer = new_producer(
            runtime,
            config.url,
            layout,
            (config.frame_rate, config.begin_frames),
            &config.gpu,
        )?;
        let browser_id = producer.browser_id();
        Ok(Self {
            producer,
            browser_id,
            state: ViewState::Live { layout },
            visible: true,
            nav: NavState::default(),
            events: Vec::new(),
            host: None,
            clicks: ClickTracker::default(),
            wheel: WheelRemainder::default(),
            held_keys: Vec::new(),
            page: Vec::new(),
            popup: PopupWidget::default(),
            downloads: HashMap::new(),
            find: Find::default(),
            fullscreen: Fullscreen::default(),
            muted: false,
            zoom: Zoom::default(),
            drag: DragSource::default(),
        })
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
        self.apply_nav(NavInput::Back);
    }

    /// Ignored when there is nothing to go forward to.
    pub fn forward(&mut self) {
        self.apply_nav(NavInput::Forward);
    }

    pub fn reload(&mut self) {
        self.apply_nav(NavInput::Reload);
    }

    /// Reloads without the cache (CEF `ReloadIgnoreCache`).
    pub fn hard_reload(&mut self) {
        self.apply_nav(NavInput::HardReload);
    }

    /// Ignored unless loading.
    pub fn stop(&mut self) {
        self.apply_nav(NavInput::Stop);
    }

    /// `now` is any monotonic time (it only times double clicks).
    pub fn mouse_move(&mut self, point: Point, modifiers: &Modifiers, now: Duration) {
        self.clicks.moved(now, point);
        let event = input::mouse_event(point, modifiers);
        self.send(|host| host.send_mouse_move_event(Some(&event), 0));
    }

    pub fn mouse_leave(&mut self, point: Point, modifiers: &Modifiers, now: Duration) {
        self.clicks.moved(now, point);
        let event = input::mouse_event(point, modifiers);
        self.send(|host| host.send_mouse_move_event(Some(&event), 1));
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
        let event = input::mouse_event(point, modifiers);
        let button = input::cef_button(button);
        self.send(|host| {
            host.send_mouse_click_event(Some(&event), button, (!pressed).into(), count)
        });
    }

    /// Deltas in logical pixels, positive = up / left.
    pub fn wheel(&mut self, point: Point, dx: f32, dy: f32, modifiers: &Modifiers) {
        let (dx, dy, flags) = input::wheel_deltas(dx, dy, modifiers, &mut self.wheel);
        let event = cef::MouseEvent {
            modifiers: flags,
            ..input::mouse_event(point, modifiers)
        };
        self.send(|host| host.send_mouse_wheel_event(Some(&event), dx, dy));
    }

    pub fn key(&mut self, key: &KeyInput) {
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
    /// opened with `begin_frames`.
    pub fn send_begin_frame(&mut self) {
        self.send(|host| host.send_external_begin_frame());
    }

    /// CEF's frame-rate ceiling (`SetWindowlessFrameRate`), for a window
    /// that moved to a display with another refresh rate.
    pub fn set_frame_rate(&mut self, frame_rate: u32) {
        self.send(|host| host.set_windowless_frame_rate(frame_rate as i32));
    }

    /// Follows the host's content rectangle; unchanged layouts cost nothing.
    pub fn set_layout(&mut self, layout: Layout) {
        self.apply(ViewEvent::Layout(layout));
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
        let effects = self.apply(ViewEvent::Frame(frame.size()));
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
                } => self.events.push(WebEvent::KeyUnhandled(KeyId::reported(
                    windows_key_code,
                    modifiers,
                ))),
                _ => {}
            }
        }
        if let Some(shape) = self.producer.poll_cursor_shape() {
            self.apply_nav(NavInput::Cursor(shape.into()));
        }
        let placement = self.producer.popup_rect().map(Rect::from);
        let (next, event) = popup::transition(self.popup, placement);
        self.popup = next;
        self.page.extend(event);
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
            Route::Nav(input) => {
                if input == NavInput::RendererTerminated {
                    eprintln!("wubble-engine: view {}: renderer gone", self.browser_id);
                }
                self.apply_nav(input);
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
    /// answers when the view closes).
    fn decline(&mut self, request: &PageEvent) {
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

    pub fn zoom_reset(&mut self) {
        self.zoom_step(welding::ZoomCommand::Reset);
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
        let (kind, payload) = match phase {
            DragPhase::Enter(data) => (welding::DragEventKind::Enter, Some(data.into())),
            DragPhase::Over => (welding::DragEventKind::Over, None),
            DragPhase::Leave => (welding::DragEventKind::Leave, None),
            DragPhase::Drop => (welding::DragEventKind::Drop, None),
        };
        let (x, y) = self.physical(point);
        let event = welding::DragInput {
            kind,
            payload,
            x,
            y,
            modifiers: weld_modifiers(modifiers),
            allowed_operations: welding::DragOperations(allowed.0),
        };
        if let Err(e) = self.producer.send_drag_input(event) {
            eprintln!("wubble-engine: view {}: drag: {e}", self.browser_id);
        }
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
#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    Nav(NavInput),
    Page(PageEvent),
    Download(DownloadId, DownloadInput),
    Find(FindInput),
    Fullscreen(FullscreenInput),
    DragStarted(PageEvent),
    /// Zoom is per host and may change on navigation without a callback.
    ObserveZoom,
}

/// Translates one welding event. Pure; events nothing uses map to nothing.
fn route(event: NavigationEvent) -> Vec<Route> {
    use NavigationEvent as E;
    let one = |r| vec![r];
    match event {
        E::AddressChanged { url } => vec![Route::Nav(NavInput::Address(url)), Route::ObserveZoom],
        E::TitleChanged { title } => one(Route::Nav(NavInput::Title(title))),
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
        E::NewWindowRequested { url, .. } => one(Route::Nav(NavInput::Popup(url))),
        E::ContentProcessTerminated { .. } => one(Route::Nav(NavInput::RendererTerminated)),
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
            media,
            has_image_contents,
            editable,
            edit_flags,
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
                selection_text,
                editable: editable || targets.contains(&welding::ContextMenuTarget::Editable),
                edit: EditFlags(edit_flags),
                misspelled_word,
                suggestions,
                items: items.into_iter().map(Into::into).collect(),
            }),
        })),
        E::ContextMenuDismissed => one(Route::Page(PageEvent::ContextMenuDismissed)),
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
            suggested_filename,
            total_bytes_expected,
        } => one(Route::Download(
            DownloadId(download),
            DownloadInput::Offered {
                request: RequestId(id),
                url,
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
                // cef_download_interrupt_reason_t.
                error: format!("interrupted ({reason})"),
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
        _ => vec![],
    }
}

/// welding's modifier set for drags.
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

#[cfg(not(windows))]
fn new_producer(
    runtime: &CefRuntime,
    url: String,
    layout: Layout,
    (frame_rate, begin_frames): (u32, bool),
    gpu: &HostWgpuContext,
) -> Result<PlatformCefProducer, EngineError> {
    let config = welding::PlatformCefConfig {
        surface: welding::CefSurfaceConfig {
            initial_url: url,
            initial_size: dpi_size(layout.size),
            scale_factor: layout.scale,
            windowless_frame_rate: frame_rate,
            external_begin_frame: begin_frames,
            // Every request is held for the engine, which answers it with
            // the host's answer or (RequestMode::DefaultNow) its default.
            handle_permission_requests: true,
            handle_auth_challenges: true,
            ask_download_path: true,
            ..welding::CefSurfaceConfig::default()
        },
    };
    #[cfg(target_os = "linux")]
    let producer = PlatformCefProducer::new(runtime, config, gpu);
    // welding's macOS producer takes no device: it imports at present time.
    #[cfg(not(target_os = "linux"))]
    let producer = {
        let _ = gpu;
        PlatformCefProducer::new(runtime, config)
    };
    producer.map_err(|e| EngineError::View(e.to_string()))
}

/// welding's Windows producer also needs the host's `HostWgpuContext`, and
/// wubble-gfx has no Windows import path yet (D3D11, M0 Windows item).
#[cfg(windows)]
fn new_producer(
    _runtime: &CefRuntime,
    _url: String,
    _layout: Layout,
    _pacing: (u32, bool),
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
        super::Layout { size, scale }
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
        ];
        for ((w, h), scale, (fw, fh)) in cases {
            let layout = layout(FrameSize::new(w, h), scale);
            assert_eq!(layout.frame_size(), FrameSize::new(fw, fh), "{layout:?}");
        }
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

    fn route_one(event: NavigationEvent) -> Route {
        let mut routed = route(event);
        assert_eq!(routed.len(), 1, "{routed:?}");
        routed.remove(0)
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
            media: welding::ContextMenuMedia::Image,
            has_image_contents: true,
            editable: false,
            edit_flags: EditFlags::CAN_PASTE,
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
        assert_eq!(menu.media, crate::page::MediaKind::Image);
        assert!(menu.editable, "the Editable target counts as editable");
        assert!(menu.edit.allows(EditFlags::CAN_PASTE));
        assert_eq!(menu.suggestions, vec!["the".to_string()]);
        assert_eq!(menu.items.len(), 1);
        assert_eq!(menu.items[0].command, 114);
    }

    #[test]
    fn stateful_events_route_to_their_machines() {
        assert_eq!(
            route_one(NavigationEvent::DownloadRequested {
                id: 2,
                download: 11,
                url: "u".into(),
                suggested_filename: "f".into(),
                total_bytes_expected: None,
            }),
            Route::Download(
                DownloadId(11),
                DownloadInput::Offered {
                    request: RequestId(2),
                    url: "u".into(),
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
                    error: "interrupted (20)".into()
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
