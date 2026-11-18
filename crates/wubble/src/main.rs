mod cli;
mod pixels;

use std::cell::RefCell;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cli::{Command, Smoke};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use wubble_engine::app::{self, Engine, EngineConfig, EngineError, ViewId};
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::pacing::{self, FrameStats};
use wubble_gfx::{FramePresenter, FrameSize, GpuContext};
use wubble_ui::AppWindow;

/// How often the Slint loop pumps CEF. Interim: replaced by compositor-driven
/// ticks in the M0 "Frame pacing" item.
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// Pumps `--smoke boot` gives CEF before shutting down. A count, not a clock.
const SMOKE_TICKS: usize = 10;

/// Pumps `--smoke frames` waits for the next frame before it gives up. A
/// count, not a clock: it only stops a smoke that would otherwise hang.
const SMOKE_MAX_IDLE_TICKS: u32 = 2_000;

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
                    eprintln!("smoke: {}", pacing::CPU_FALLBACK_UNOBSERVED);
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
    /// Opened on the first pump, once the window and its monitor exist.
    Waiting {
        url: String,
    },
    Open(ViewId),
}

/// Everything the pump timer drives. Owned by the timer closure via `Rc`.
struct Host {
    engine: Engine,
    presenter: FramePresenter,
    content: Content,
    stats: FrameStats,
    last_present: Option<Instant>,
    idle_ticks: u32,
    /// Accelerated frames after which the loop quits (`--smoke frames`).
    quit_after: Option<u64>,
    /// Snapshot the window before quitting and require the content area to
    /// show the page. Only for the bundled page: its colours are known.
    check_pixels: bool,
    /// Why the loop was quit early, if it was.
    failure: Option<String>,
}

impl Host {
    fn pump(&mut self, window: &AppWindow) {
        self.engine.tick();
        if let Err(e) = self.show_frame(window) {
            self.fail(e);
        }
        if self.quit_after.is_some() {
            self.idle_ticks += 1;
            if self.idle_ticks > SMOKE_MAX_IDLE_TICKS {
                self.fail(format!(
                    "no new frame after {SMOKE_MAX_IDLE_TICKS} pumps ({} accelerated so far)",
                    self.stats.accelerated
                ));
            }
        }
    }

    fn show_frame(&mut self, window: &AppWindow) -> Result<(), String> {
        let layout = content_layout(window);
        let id = match &self.content {
            Content::Waiting { url } => {
                let refresh = window
                    .window()
                    .with_winit_window(display_refresh_mhz)
                    .flatten();
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
        view.set_layout(layout);
        let Some(frame) = view.take_frame() else {
            return Ok(());
        };
        let frame_size = frame.size();
        let clip = frame_size.cropped_to(layout.size);
        let image = self.presenter.present(frame).map_err(|e| e.to_string())?;
        window.set_web_frame(image);
        window.set_web_frame_clip_width(slint_int(clip.width));
        window.set_web_frame_clip_height(slint_int(clip.height));
        let now = Instant::now();
        self.stats
            .record_accelerated(self.last_present.map(|last| now - last));
        self.last_present = Some(now);
        self.idle_ticks = 0;
        if self.quit_after.is_some_and(|n| self.stats.accelerated >= n) {
            eprintln!(
                "smoke: content {}x{} @{} frame {}x{} clip {}x{}",
                layout.size.width,
                layout.size.height,
                layout.scale,
                frame_size.width,
                frame_size.height,
                clip.width,
                clip.height
            );
            if self.check_pixels {
                let sample = check_pixels(window)?;
                println!(
                    "smoke: pixels ok distinct={} mean_luma={}",
                    sample.distinct, sample.mean_luma
                );
            }
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
    Layout {
        size: FrameSize::new(
            physical(window.get_content_width(), scale),
            physical(window.get_content_height(), scale),
        ),
        scale,
    }
}

/// Snapshots the window and checks that the content rectangle shows the page.
fn check_pixels(window: &AppWindow) -> Result<pixels::Sample, String> {
    let snapshot = window
        .window()
        .take_snapshot()
        .map_err(|e| format!("cannot snapshot the window: {e}"))?;
    let scale = window.window().scale_factor();
    let region = pixels::Region {
        x: physical(window.get_content_x(), scale),
        y: physical(window.get_content_y(), scale),
        width: physical(window.get_content_width(), scale),
        height: physical(window.get_content_height(), scale),
    };
    pixels::check(&snapshot, region)
}

/// A logical length in whole physical pixels.
fn physical(logical: f32, scale: f32) -> u32 {
    (logical * scale).round().max(0.0) as u32
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

    let (url, quit_after, check_pixels) = match mode {
        Mode::Browse => (smoke_page_url(), None, false),
        Mode::Smoke { frames, url } => {
            let bundled = url.is_none();
            (url.unwrap_or_else(smoke_page_url), Some(frames), bundled)
        }
    };
    let engine = Engine::start(config?)?;
    let host = Rc::new(RefCell::new(Host {
        engine,
        presenter: gpu.presenter(),
        content: Content::Waiting { url },
        stats: FrameStats::default(),
        last_present: None,
        idle_ticks: 0,
        quit_after,
        check_pixels,
        failure: None,
    }));
    let window = AppWindow::new()?;

    let pump = slint::Timer::default();
    let pumped = Rc::clone(&host);
    let weak = window.as_weak();
    pump.start(slint::TimerMode::Repeated, PUMP_INTERVAL, move || {
        if let Some(window) = weak.upgrade() {
            pumped.borrow_mut().pump(&window);
        }
    });

    let result = window.run();
    drop(pump);
    drop(window);
    let mut host = host.borrow_mut();
    drain(&mut host.engine);
    result?;
    Ok(Outcome {
        stats: host.stats.clone(),
        failure: host.failure.take().map(Into::into),
    })
}
