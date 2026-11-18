mod cli;

use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use cli::{Command, Smoke};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_engine::app::{
    self, Engine, EngineConfig, EngineError, PumpOutcome, PumpScheduler, PumpTimer, ViewId,
};
use wubble_engine::view::{Layout, ViewConfig};
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

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os()) {
        Ok(command) => command,
        Err(e) => e.exit(),
    };
    match command {
        Command::Run => match run(Mode::Browse, EngineConfig::from_current_exe()) {
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
        Command::Smoke(Smoke::Frames { frames, url }) => {
            let mode = Mode::Smoke {
                frames: frames.get().into(),
                url,
            };
            match with_smoke_cache(|config| run(mode, config)) {
                Ok(Outcome { stats, failure }) => {
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
}

/// What the window is for.
enum Mode {
    Browse,
    /// Quit after `frames` accelerated frames of `url` (bundled page if `None`).
    Smoke {
        frames: u64,
        url: Option<String>,
    },
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

/// Hands a pump request to the host. Called on the Slint thread only.
fn deliver(wake: Wake) {
    HOST.with_borrow_mut(|host| {
        if let Some(host) = host {
            host.pump(wake);
        }
    });
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
    /// Why the loop was quit early, if it was.
    failure: Option<String>,
}

impl Host {
    fn pump(&mut self, wake: Wake) {
        let outcome = match wake {
            Wake::Schedule(delay_ms) => self.engine.schedule_pump(delay_ms),
            Wake::TimerFired => self.engine.pump_timer_fired(),
        };
        let PumpOutcome { timer, worked } = outcome;
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

/// The bundled test page as a `data:` URL, so nothing is fetched.
fn smoke_page_url() -> String {
    let mut url = String::from("data:text/html;charset=utf-8,");
    wubble_core::url::percent_encode_into(SMOKE_PAGE, &mut url);
    url
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

    let (url, quit_after) = match mode {
        Mode::Browse => (smoke_page_url(), None),
        Mode::Smoke { frames, url } => (url.unwrap_or_else(smoke_page_url), Some(frames)),
    };
    // The first pump runs once the loop does; from there CEF schedules every
    // pump itself.
    slint::invoke_from_event_loop(|| deliver(Wake::Schedule(0)))?;
    let engine = Engine::start(EngineConfig {
        pump_scheduler: Some(pump_scheduler()),
        ..config?
    })?;
    let window = AppWindow::new()?;
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
    })
}
