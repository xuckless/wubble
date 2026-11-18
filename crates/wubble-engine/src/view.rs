//! One windowless browser: frames, resize, HiDPI scale, close, navigation
//! and input.
//!
//! The lifecycle is the pure [`transition`], navigation the pure
//! [`nav::transition`]; [`View`] wraps welding's `PlatformCefProducer` (which
//! owns the CEF render handler and the dmabuf custody) and runs the effects.
//! Input goes straight to the view's CEF `BrowserHost`, translated by
//! [`crate::input`]. Views are owned by [`crate::app::Engine`], which closes
//! them on shutdown and reports each one closed to its lifecycle.

use std::time::Duration;

use cef::{ImplBrowser, ImplBrowserHost};
use welding::{
    CefRuntime, CefSurfaceEvent, CefSurfaceProducer, NavigationEvent, PlatformCefProducer,
};
use winit::keyboard::PhysicalKey;
use wubble_gfx::{FrameSize, NativeFrame};

use crate::app::EngineError;
use crate::input::{
    self, CefKey, ClickTracker, KeyInput, Modifiers, MouseButton, Point, WheelRemainder,
};
use crate::nav::{self, NavEffect, NavInput, NavState, WebEvent};

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
        // Frames at a stale size are still shown (stretched) rather than
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
#[derive(Clone, Debug)]
pub struct ViewConfig {
    pub url: String,
    pub layout: Layout,
    /// CEF `windowless_frame_rate`; see `wubble_gfx::pacing::frame_rate_for`.
    pub frame_rate: u32,
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
}

impl View {
    pub(crate) fn open(runtime: &CefRuntime, config: ViewConfig) -> Result<Self, EngineError> {
        let layout = config.layout.clamped();
        let producer = new_producer(runtime, config.url, layout, config.frame_rate)?;
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
    /// here so the queue does not grow.
    pub(crate) fn drain_events(&mut self) -> (Vec<WebEvent>, bool) {
        let mut closed = false;
        while let Some(event) = self.producer.poll_web_event() {
            match event {
                CefSurfaceEvent::BrowserClosed => {
                    closed |= self
                        .apply(ViewEvent::GoneFromCef)
                        .contains(&ViewEffect::ReportClosed);
                }
                CefSurfaceEvent::Navigation(event) => {
                    if let Some(input) = self.nav_input(event) {
                        self.apply_nav(input);
                    }
                }
                _ => {}
            }
        }
        if let Some(shape) = self.producer.poll_cursor_shape() {
            self.apply_nav(NavInput::Cursor(shape.into()));
        }
        (std::mem::take(&mut self.events), closed)
    }

    /// The navigation input a welding event stands for, if any.
    fn nav_input(&self, event: NavigationEvent) -> Option<NavInput> {
        match event {
            NavigationEvent::AddressChanged { url } => Some(NavInput::Address(url)),
            NavigationEvent::TitleChanged { title } => Some(NavInput::Title(title)),
            NavigationEvent::LoadingStateChanged {
                loading,
                can_go_back,
                can_go_forward,
            } => Some(NavInput::Loading {
                loading,
                can_go_back,
                can_go_forward,
            }),
            NavigationEvent::FaviconUrlsChanged { urls } => Some(NavInput::Favicon(
                urls.into_iter().next().unwrap_or_default(),
            )),
            NavigationEvent::NewWindowRequested { url, .. } => Some(NavInput::Popup(url)),
            NavigationEvent::ContentProcessTerminated { status, .. } => {
                eprintln!(
                    "wubble-engine: view {}: renderer gone ({status:?})",
                    self.browser_id
                );
                Some(NavInput::RendererTerminated)
            }
            // ERR_ABORTED (-3) is a navigation replaced by another one.
            NavigationEvent::LoadError {
                url,
                error_code,
                error_text,
            } if error_code != -3 => {
                eprintln!(
                    "wubble-engine: view {}: {url} failed: {error_text} ({error_code})",
                    self.browser_id
                );
                None
            }
            _ => None,
        }
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

#[cfg(not(windows))]
fn new_producer(
    runtime: &CefRuntime,
    url: String,
    layout: Layout,
    frame_rate: u32,
) -> Result<PlatformCefProducer, EngineError> {
    PlatformCefProducer::new(
        runtime,
        welding::PlatformCefConfig {
            surface: welding::CefSurfaceConfig {
                initial_url: url,
                initial_size: dpi_size(layout.size),
                scale_factor: layout.scale,
                windowless_frame_rate: frame_rate,
                ..welding::CefSurfaceConfig::default()
            },
        },
    )
    .map_err(|e| EngineError::View(e.to_string()))
}

/// welding's Windows producer also needs the host's `HostWgpuContext`, and
/// wubble-gfx has no Windows import path yet (D3D11, M0 Windows item).
#[cfg(windows)]
fn new_producer(
    _runtime: &CefRuntime,
    _url: String,
    _layout: Layout,
    _frame_rate: u32,
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
