mod cli;
mod input;
mod nav_smoke;

use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use cli::{Command, Smoke};
use input::{InputState, KeyRoute};
use nav_smoke::{NavSmoke, SmokeAction, SmokeInput};
use slint::ComponentHandle;
use slint::platform::PointerEventButton;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use wubble_engine::app::{
    self, Engine, EngineConfig, EngineError, PumpOutcome, PumpScheduler, PumpTimer, ViewId,
};
use wubble_engine::input::Point;
use wubble_engine::nav::WebEvent;
use wubble_engine::view::{Layout, View, ViewConfig};
use wubble_gfx::pacing::{self, FrameStats};
use wubble_gfx::{FramePresenter, FrameSize, GpuContext};
use wubble_ui::AppWindow;

/// Pumps `--smoke boot` gives CEF before shutting down. A count, not a clock.
const SMOKE_TICKS: usize = 10;

/// Runs of CEF work `--smoke frames` waits for the next frame before it gives
/// up. A count, not a clock: it only stops a smoke that would otherwise hang.
const SMOKE_MAX_IDLE_RUNS: u32 = 2_000;

/// The bundled animated test page.
const SMOKE_PAGE: &str = include_str!("../assets/smoke.html");

/// The page `--smoke nav` navigates to.
const NAV_PAGE: &str = include_str!("../assets/nav.html");

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os()) {
        Ok(command) => command,
        Err(e) => e.exit(),
    };
    match command {
        Command::Run { url } => match run(Mode::Browse { url }, EngineConfig::from_current_exe()) {
            Ok(Outcome { failure: None, .. }) => ExitCode::SUCCESS,
            Ok(Outcome {
                failure: Some(e), ..
            })
            | Err(e) => {
                eprintln!("wubble: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Boot) => match with_smoke_cache(smoke_boot) {
            Ok(()) => {
                println!("smoke: cef boot ok {}", app::version());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: cef boot FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Nav) => match with_smoke_cache(|config| run(Mode::Nav, config)) {
            Ok(Outcome {
                failure: None,
                nav_title: Some(title),
                ..
            }) => {
                println!("smoke: nav ok {title}");
                ExitCode::SUCCESS
            }
            Ok(Outcome {
                failure: Some(e), ..
            })
            | Err(e) => {
                eprintln!("smoke: nav FAILED: {e}");
                ExitCode::FAILURE
            }
            Ok(Outcome { .. }) => {
                eprintln!("smoke: nav FAILED: the window closed first");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Frames { frames, url }) => {
            let mode = Mode::Frames {
                frames: frames.get().into(),
                url,
            };
            match with_smoke_cache(|config| run(mode, config)) {
                Ok(Outcome { stats, failure, .. }) => {
                    println!("{}", stats.summary_line());
                    match failure {
                        None if stats.is_success() => ExitCode::SUCCESS,
                        None => {
                            eprintln!("smoke: frames FAILED: no zero-copy frames");
                            ExitCode::FAILURE
                        }
                        Some(e) => {
                            eprintln!("smoke: frames FAILED: {e}");
                            ExitCode::FAILURE
                        }
                    }
                }
                Err(e) => {
                    eprintln!("smoke: frames FAILED: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Pumps until every browser is closed and CEF has shut down. Bounded by the
/// lifecycle state, not by time: the engine closes its views and reports each
/// one closed once CEF has let go of it.
fn drain(engine: &mut Engine) {
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
}

/// Runs `f` with a fresh per-process CEF cache, so concurrent smokes (lanes,
/// CI jobs, a smoke next to an open browser) never share CEF's singleton
/// lock, and removes it afterwards.
fn with_smoke_cache<T>(
    f: impl FnOnce(Result<EngineConfig, EngineError>) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let cache_dir = std::env::temp_dir().join(format!("wubble-smoke-{}", std::process::id()));
    let config = EngineConfig::from_current_exe().map(|config| EngineConfig {
        cache_dir: cache_dir.clone(),
        ..config
    });
    let result = f(config);
    remove_cache(&cache_dir);
    result
}

/// Called after CEF has shut down (or never started), so nothing holds the directory.
fn remove_cache(cache_dir: &Path) {
    if let Err(e) = std::fs::remove_dir_all(cache_dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("smoke: could not remove {}: {e}", cache_dir.display());
    }
}

fn smoke_boot(config: Result<EngineConfig, EngineError>) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::start(config?)?;
    for _ in 0..SMOKE_TICKS {
        engine.tick();
    }
    drain(&mut engine);
    Ok(())
}

/// How a windowed run ended.
struct Outcome {
    stats: FrameStats,
    /// Why the loop was quit early, if it was.
    failure: Option<Box<dyn std::error::Error>>,
    /// The title `--smoke nav` passed on.
    nav_title: Option<String>,
}

/// What the window is for.
enum Mode {
    /// `url`, or the bundled page if `None`.
    Browse { url: Option<String> },
    /// Quit after `frames` accelerated frames of `url` (bundled page if `None`).
    Frames { frames: u64, url: Option<String> },
    /// `--smoke nav`.
    Nav,
}

/// Where the one web view is.
enum Content {
    /// Opened on the first pump after winit has created the window.
    Waiting {
        url: String,
    },
    Open(ViewId),
}

/// A pump request for the host, from CEF or from the host's own timer.
#[derive(Clone, Copy, Debug)]
enum Wake {
    /// CEF's `OnScheduleMessagePumpWork(delay_ms)`.
    Schedule(i64),
    TimerFired,
}

thread_local! {
    /// The windowed run's host, on the Slint thread. Lives here because CEF's
    /// wake-ups arrive through `invoke_from_event_loop`, whose closures must be
    /// `Send` and so cannot hold it. Empty before and after the Slint loop.
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

/// Runs `f` on the host, if there is one and nothing else is using it.
/// Called on the Slint thread only.
fn with_host(f: impl FnOnce(&mut Host)) {
    HOST.with(|cell| match cell.try_borrow_mut() {
        Ok(mut host) => {
            if let Some(host) = host.as_mut() {
                f(host);
            }
        }
        Err(_) => eprintln!("wubble: host busy, input or pump request dropped"),
    });
}

/// Hands a pump request to the host.
fn deliver(wake: Wake) {
    with_host(|host| host.pump(wake));
}

/// CEF's pump requests, from any thread, onto the Slint loop. After the loop
/// has ended they are dropped: the shutdown drain pumps by itself.
fn pump_scheduler() -> PumpScheduler {
    PumpScheduler::new(|delay_ms| {
        let _ = slint::invoke_from_event_loop(move || deliver(Wake::Schedule(delay_ms)));
    })
}

/// Everything the pump drives. Owned by [`HOST`].
struct Host {
    engine: Engine,
    window: slint::Weak<AppWindow>,
    /// The engine's single-shot pump timer.
    timer: slint::Timer,
    presenter: FramePresenter,
    content: Content,
    stats: FrameStats,
    last_present: Option<Instant>,
    idle_runs: u32,
    /// Accelerated frames after which the loop quits (`--smoke frames`).
    quit_after: Option<u64>,
    /// `--smoke nav`'s progress.
    nav_smoke: Option<NavSmoke>,
    nav_title: Option<String>,
    /// The URL last reloaded after its renderer died, so a page that keeps
    /// crashing (out of memory, say) is reloaded once, not in a loop.
    crash_reloaded: Option<String>,
    input: InputState,
    /// Why the loop was quit early, if it was.
    failure: Option<String>,
}

impl Host {
    fn pump(&mut self, wake: Wake) {
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
        for (_, event) in events {
            self.on_web_event(&window, event);
        }
        self.smoke_step(SmokeInput::Worked);
        if let Err(e) = self.show_frame(&window) {
            self.fail(e);
        }
        if self.quit_after.is_some() {
            self.idle_runs += 1;
            if self.idle_runs > SMOKE_MAX_IDLE_RUNS {
                self.fail(format!(
                    "no new frame after {SMOKE_MAX_IDLE_RUNS} runs of CEF work ({} accelerated so far)",
                    self.stats.accelerated
                ));
            }
        }
    }

    fn show_frame(&mut self, window: &AppWindow) -> Result<(), String> {
        let layout = content_layout(window);
        let id = match &self.content {
            Content::Waiting { url } => {
                // Before winit has created the window there is no monitor to
                // ask; CEF's next pump (at most `MAX_TIMER_DELAY` away) retries.
                let Some(refresh) = window.window().with_winit_window(display_refresh_mhz) else {
                    return Ok(());
                };
                let frame_rate = pacing::frame_rate_for(refresh);
                eprintln!("wubble: CEF frame rate {frame_rate} (display {refresh:?} mHz)");
                let config = ViewConfig {
                    url: url.clone(),
                    layout,
                    frame_rate,
                };
                let id = self.engine.open_view(config).map_err(|e| e.to_string())?;
                self.content = Content::Open(id);
                self.sync_focus(window);
                id
            }
            Content::Open(id) => *id,
        };
        let Some(view) = self.engine.view(id) else {
            return Err("the web view closed unexpectedly".into());
        };
        let new_cpu = self.stats.record_cpu_fallback(view.cpu_fallback_frames());
        if new_cpu > 0 {
            eprintln!("wubble: {new_cpu} CPU fallback frame(s) (OnPaint), not shown");
        }
        view.set_layout(layout);
        let Some(frame) = view.take_frame() else {
            return Ok(());
        };
        let clip = frame.size().cropped_to(layout.size);
        let image = self.presenter.present(frame).map_err(|e| e.to_string())?;
        window.set_web_frame(image);
        window.set_web_frame_clip_width(slint_int(clip.width));
        window.set_web_frame_clip_height(slint_int(clip.height));
        let now = Instant::now();
        self.stats
            .record_accelerated(self.last_present.map(|last| now - last));
        self.last_present = Some(now);
        self.idle_runs = 0;
        if self.quit_after.is_some_and(|n| self.stats.accelerated >= n) {
            quit();
        }
        Ok(())
    }

    /// The one web view, while it is open.
    fn view(&mut self) -> Option<&mut View> {
        match self.content {
            Content::Open(id) => self.engine.view(id),
            Content::Waiting { .. } => None,
        }
    }

    fn on_web_event(&mut self, window: &AppWindow, event: WebEvent) {
        self.smoke_step(SmokeInput::Event(&event));
        match event {
            WebEvent::CursorChanged(cursor) => window.set_web_cursor(input::web_cursor(cursor)),
            WebEvent::UrlChanged(url) => {
                let (host, path) = wubble_core::url::address_parts(&url);
                window.set_address_host(host.into());
                window.set_address_path(path.into());
            }
            // No tabs yet: a later item decides where popups go.
            WebEvent::PopupRequested { url } => eprintln!("wubble: popup not opened: {url}"),
            WebEvent::RendererGone => {
                let Some(url) = self.view().map(|view| view.nav().page().url.clone()) else {
                    return;
                };
                if self.crash_reloaded.as_ref() == Some(&url) {
                    eprintln!("wubble: renderer gone again on {url}, not reloading");
                } else if let Some(view) = self.view() {
                    eprintln!("wubble: renderer gone on {url}, reloading");
                    view.reload();
                    self.crash_reloaded = Some(url);
                }
            }
            WebEvent::TitleChanged(_)
            | WebEvent::LoadingChanged { .. }
            | WebEvent::FaviconChanged(_) => {}
        }
    }

    fn smoke_step(&mut self, input: SmokeInput) {
        let Some(state) = self.nav_smoke.take() else {
            return;
        };
        let want = nav_smoke::title_of(NAV_PAGE).unwrap_or_default();
        let (state, action) = nav_smoke::next(state, input, want);
        self.nav_smoke = Some(state);
        match action {
            Some(SmokeAction::LoadSecondPage) => {
                if let Some(view) = self.view() {
                    view.load(&page_url(NAV_PAGE));
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

    /// Tells the view about keyboard focus: it has it while the window has
    /// focus and the web content is the shell's focused item.
    fn sync_focus(&mut self, window: &AppWindow) {
        let focused = self.input.window_focused && window.get_web_focused();
        if focused != self.input.view_focused {
            self.input.view_focused = focused;
            if let Some(view) = self.view() {
                view.focus(focused);
            }
        }
    }

    /// Raw winit events, before Slint sees them: keys for the page,
    /// modifiers and window focus.
    fn window_event(&mut self, event: &WindowEvent) -> EventResult {
        let Some(window) = self.window.upgrade() else {
            return EventResult::Propagate;
        };
        match event {
            WindowEvent::ModifiersChanged(modifiers) => self.input.keys = modifiers.state(),
            WindowEvent::Focused(focused) => {
                self.input.window_focused = *focused;
                self.sync_focus(&window);
            }
            // Synthetic presses (keys already held when the window gained
            // focus, X11) never reach the page.
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let key = self.input.key_input(event);
                let route = input::route(
                    window.get_web_focused(),
                    &key.unmodified,
                    key.modifiers.keys,
                );
                if route != KeyRoute::Shell
                    && let Some(view) = self.view()
                {
                    view.key(&key);
                }
                if route == KeyRoute::Page {
                    return EventResult::PreventDefault;
                }
            }
            _ => {}
        }
        EventResult::Propagate
    }

    fn pointer_moved(&mut self, point: Point) {
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        if let Some(view) = self.view() {
            view.mouse_move(point, &modifiers, now);
        }
    }

    fn pointer_left(&mut self, point: Point) {
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        if let Some(view) = self.view() {
            view.mouse_leave(point, &modifiers, now);
        }
    }

    fn pointer_button(&mut self, button: PointerEventButton, pressed: bool, point: Point) {
        let Some(button) = input::mouse_button(button) else {
            return;
        };
        self.input.buttons = self.input.buttons.with(button, pressed);
        let (modifiers, now) = (self.input.modifiers(), self.input.now());
        if let Some(view) = self.view() {
            view.mouse_button(button, pressed, point, &modifiers, now);
        }
    }

    fn pointer_cancelled(&mut self) {
        self.input.buttons = Default::default();
        if let Some(view) = self.view() {
            view.capture_lost();
        }
    }

    fn scrolled(&mut self, point: Point, dx: f32, dy: f32) {
        let modifiers = self.input.modifiers();
        if let Some(view) = self.view() {
            view.wheel(point, dx, dy, &modifiers);
        }
    }

    fn fail(&mut self, reason: String) {
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
        quit();
    }
}

fn quit() {
    // Fails only when no event loop runs, and the pump runs inside it.
    let _ = slint::quit_event_loop();
}

/// The window's display refresh rate. Wayland names the current monitor only
/// once the surface has entered an output, so fall back to the fastest
/// connected display: the CEF frame rate is a ceiling, so too high only costs
/// idle callbacks, while too low would cap the frame rate.
fn display_refresh_mhz(window: &slint::winit_030::winit::window::Window) -> Option<u32> {
    window
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .or_else(|| {
            window
                .available_monitors()
                .filter_map(|m| m.refresh_rate_millihertz())
                .max()
        })
}

fn slint_int(pixels: u32) -> i32 {
    i32::try_from(pixels).unwrap_or(i32::MAX)
}

/// The content rectangle in physical pixels and the window's scale factor.
fn content_layout(window: &AppWindow) -> Layout {
    let scale = window.window().scale_factor();
    let physical = |logical: f32| (logical * scale).round().max(0.0) as u32;
    Layout {
        size: FrameSize::new(
            physical(window.get_content_width()),
            physical(window.get_content_height()),
        ),
        scale,
    }
}

/// A bundled page as a `data:` URL, so nothing is fetched.
fn page_url(html: &str) -> String {
    let mut url = String::from("data:text/html;charset=utf-8,");
    wubble_core::url::percent_encode_into(html, &mut url);
    url
}

/// Hands the shell's web-content callbacks and raw window events to the host.
fn connect_input(window: &AppWindow) {
    let point = |x: f32, y: f32| Point { x, y };
    window.on_web_pointer_moved(move |x, y| with_host(|host| host.pointer_moved(point(x, y))));
    window.on_web_pointer_left(move |x, y| with_host(|host| host.pointer_left(point(x, y))));
    window.on_web_pointer_pressed(move |button, x, y| {
        with_host(|host| host.pointer_button(button, true, point(x, y)))
    });
    window.on_web_pointer_released(move |button, x, y| {
        with_host(|host| host.pointer_button(button, false, point(x, y)))
    });
    window.on_web_pointer_cancelled(|| with_host(Host::pointer_cancelled));
    window
        .on_web_scrolled(move |x, y, dx, dy| with_host(|host| host.scrolled(point(x, y), dx, dy)));
    let weak = window.as_weak();
    window.on_web_focus_changed(move |_| {
        if let Some(window) = weak.upgrade() {
            with_host(|host| host.sync_focus(&window));
        }
    });
    window.window().on_winit_window_event(|_, event| {
        let mut result = EventResult::Propagate;
        with_host(|host| result = host.window_event(event));
        result
    });
}

/// Opens the window with one web view on the shared GPU device and runs the
/// Slint loop until the window closes or the smoke ends.
fn run(
    mode: Mode,
    config: Result<EngineConfig, EngineError>,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let gpu = GpuContext::new()?;
    eprintln!("wubble: GPU {}", gpu.describe());
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_30(gpu.slint_configuration())
        .select()?;
    // Slint's platform lives in a thread-local and would drop the last handle
    // to the shared device during thread-local teardown, after wgpu-core's
    // own thread-locals are gone (wgpu-core `snatch` lock tracing, debug
    // builds), which aborts the process. Keep one handle until exit instead.
    std::mem::forget(gpu.clone());

    let bundled = || page_url(SMOKE_PAGE);
    let (url, quit_after, nav_smoke) = match mode {
        Mode::Browse { url } => (url.unwrap_or_else(bundled), None, None),
        Mode::Frames { frames, url } => (url.unwrap_or_else(bundled), Some(frames), None),
        Mode::Nav => (bundled(), None, Some(NavSmoke::default())),
    };
    // The first pump runs once the loop does; from there CEF schedules every
    // pump itself.
    slint::invoke_from_event_loop(|| deliver(Wake::Schedule(0)))?;
    let engine = Engine::start(EngineConfig {
        pump_scheduler: Some(pump_scheduler()),
        ..config?
    })?;
    let window = AppWindow::new()?;
    connect_input(&window);
    HOST.set(Some(Host {
        engine,
        window: window.as_weak(),
        timer: slint::Timer::default(),
        presenter: gpu.presenter(),
        content: Content::Waiting { url },
        stats: FrameStats::default(),
        last_present: None,
        idle_runs: 0,
        quit_after,
        nav_smoke,
        nav_title: None,
        crash_reloaded: None,
        input: InputState::default(),
        failure: None,
    }));

    let result = window.run();
    drop(window);
    let mut host = HOST.take().ok_or("the host went missing")?;
    drop(host.timer);
    drain(&mut host.engine);
    result?;
    Ok(Outcome {
        stats: host.stats.clone(),
        failure: host.failure.take().map(Into::into),
        nav_title: host.nav_title.take(),
    })
}
