mod backdrop;
mod bar;
mod bind;
mod chooser;
mod cli;
mod host;
mod ime;
mod input;
mod menu;
mod nav_smoke;
mod page_error;
mod page_smoke;
mod perform;
mod persist;
mod pixels;
mod profile;
mod prompt;
mod screen;
mod session_smoke;
mod shell;
mod surface;
mod tabs_smoke;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use backdrop::Glass;
use cli::{Command, Phase, Smoke};
use host::{Checks, Host};
use nav_smoke::NavSmoke;
use persist::{Loaded, StoreClient};
use profile::Profile;
use session_smoke::SessionSmoke;
use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use tabs_smoke::TabsSmoke;
use wubble_core::seam::Seam;
use wubble_engine::app::{self, Engine, EngineConfig, EngineError, PumpScheduler};
use wubble_engine::input::Point;
use wubble_engine::view::Layout;
use wubble_gfx::pacing::FrameStats;
use wubble_gfx::{FrameSize, GpuContext};
use wubble_ui::AppWindow;
use wubble_ui::glass::GlassRegistry;

/// Pumps `--smoke boot` gives CEF before shutting down. A count, not a clock.
const SMOKE_TICKS: usize = 10;

/// Runs of CEF work `--smoke frames` waits for the next frame before it gives
/// up. A count, not a clock: it only stops a smoke that would otherwise hang.
pub const SMOKE_MAX_IDLE_RUNS: u32 = 2_000;

/// The bundled animated test page.
const SMOKE_PAGE: &str = include_str!("../assets/smoke.html");

/// The page `--smoke nav` navigates to.
pub const NAV_PAGE: &str = include_str!("../assets/nav.html");

/// The third page of `--smoke tabs`.
pub const TAB3_PAGE: &str = include_str!("../assets/tab3.html");

/// The page `--smoke page` drives.
pub const PAGE_PAGE: &str = include_str!("../assets/page.html");

/// `--smoke tabs`'s pages, in tab order.
pub const TABS_SMOKE_PAGES: [&str; 3] = [SMOKE_PAGE, NAV_PAGE, TAB3_PAGE];

/// How often hidden tabs are checked against the sleep policy. Coarse: the
/// policy's delays are a minute and more.
const LIFECYCLE_TICK: Duration = Duration::from_secs(5);

/// How often live tabs are asked whether they play sound (the speaker glyph;
/// CEF has no audible callback, see `wubble_engine::media`).
const AUDIBLE_PROBE_EVERY: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os()) {
        Ok(command) => command,
        Err(e) => e.exit(),
    };
    match command {
        Command::Run {
            url,
            profile,
            downloads,
        } => match browse(url, profile, downloads) {
            Ok(Outcome { failure: None, .. }) => ExitCode::SUCCESS,
            Ok(Outcome {
                failure: Some(e), ..
            })
            | Err(e) => {
                eprintln!("wubble: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Boot) => match with_smoke_dir(|config, _| smoke_boot(config)) {
            Ok(()) => {
                println!("smoke: cef boot ok {}", app::version());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: cef boot FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Nav) => {
            match with_smoke_dir(|config, dir| run(Mode::Nav, config, dir)) {
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
            }
        }
        Command::Smoke(Smoke::Tabs) => {
            match with_smoke_dir(|config, dir| run(Mode::Tabs, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    tabs_passed: true,
                    ..
                }) => {
                    println!("smoke: tabs ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: tabs FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: tabs FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Page) => {
            match with_smoke_dir(|config, dir| run(Mode::Page, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    page_zoom: Some(zoom),
                    ..
                }) => {
                    println!("smoke: page ok zoom={zoom}%");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: page FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: page FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Session) => match smoke_session() {
            Ok(()) => {
                println!("smoke: session ok");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: session FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::SessionPhase { phase, profile }) => {
            let outcome = with_smoke_dir(|config, _| run(Mode::Session(phase), config, &profile));
            match outcome {
                Ok(Outcome {
                    failure: None,
                    session_passed: true,
                    ..
                }) => {
                    println!("smoke: session phase {phase:?} ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: session phase {phase:?} FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: session phase {phase:?} FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Frames { frames, url }) => {
            let mode = Mode::Frames {
                frames: frames.get().into(),
                url,
            };
            match with_smoke_dir(|config, dir| run(mode, config, dir)) {
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

/// Starts the spring-driven shell (sidebar seam from the saved `ui`, tab rows, Space
/// indicator) and hands the scene its frames from the same, single rendering notifier.
fn install_shell_driver(
    window: &AppWindow,
    ui: persist::UiState,
    glass: &Rc<RefCell<Glass>>,
) -> Result<Rc<RefCell<GlassRegistry>>, Box<dyn std::error::Error>> {
    let seam = Seam::restored(ui.sidebar_width.into(), ui.mode);
    let glass = Rc::clone(glass);
    let panels = wubble_ui::driver::install(window, seam, move |window, now, items| {
        if let Ok(mut glass) = glass.try_borrow_mut() {
            glass.frame(window, now, items);
        }
    })?;
    Ok(panels)
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

/// The browser on its profile: `--profile`, else the per-user one, which
/// also holds CEF's cache.
fn browse(
    url: Option<String>,
    profile: Option<PathBuf>,
    downloads: Option<PathBuf>,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let profile = Profile::resolve(profile).ok_or(EngineError::CacheDir)?;
    let config = EngineConfig::from_current_exe().map(|config| EngineConfig {
        cache_dir: profile.cef_cache(),
        ..config
    });
    let downloads = profile::downloads_dir(downloads);
    run(Mode::Browse { url, downloads }, config, &profile.dir)
}

/// Runs `f` with a fresh per-process directory, removed afterwards: CEF's
/// cache inside it, so concurrent smokes (lanes, CI jobs, a smoke next to an
/// open browser) never share CEF's singleton lock, and it doubles as the
/// smoke's throwaway profile.
fn with_smoke_dir<T>(
    f: impl FnOnce(Result<EngineConfig, EngineError>, &Path) -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("wubble-smoke-{}", std::process::id()));
    let profile = Profile { dir: dir.clone() };
    let config = EngineConfig::from_current_exe().map(|config| EngineConfig {
        cache_dir: profile.cef_cache(),
        ..config
    });
    let result = f(config, &dir);
    remove_cache(&dir);
    result
}

/// `--smoke session`: both phases, each its own process (CEF starts once per
/// process), on one temporary profile.
fn smoke_session() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let profile = std::env::temp_dir().join(format!("wubble-session-{}", std::process::id()));
    let result = (|| {
        for phase in ["session-phase=1", "session-phase=2"] {
            let status = std::process::Command::new(&exe)
                .args(["--smoke", phase, "--profile"])
                .arg(&profile)
                .status()?;
            if !status.success() {
                return Err(format!("{phase} exited with {status}").into());
            }
        }
        Ok(())
    })();
    remove_cache(&profile);
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
    /// `--smoke tabs` reached its end.
    tabs_passed: bool,
    /// The `--smoke session` phase reached its end.
    session_passed: bool,
    /// The zoom `--smoke page` passed with.
    page_zoom: Option<u32>,
}

/// What the window is for.
enum Mode {
    /// `url`, or the bundled page if `None`; downloads go to `downloads`.
    Browse {
        url: Option<String>,
        downloads: Option<PathBuf>,
    },
    /// Quit after `frames` accelerated frames of `url` (bundled page if `None`).
    Frames { frames: u64, url: Option<String> },
    /// `--smoke nav`.
    Nav,
    /// `--smoke tabs`.
    Tabs,
    /// `--smoke page`.
    Page,
    /// One phase of `--smoke session`.
    Session(Phase),
}

/// A pump request for the host, from CEF or from the host's own timer.
#[derive(Clone, Copy, Debug)]
pub enum Wake {
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
pub fn deliver(wake: Wake) {
    with_host(|host| host.pump(wake));
}

/// CEF's pump requests, from any thread, onto the Slint loop. After the loop
/// has ended they are dropped: the shutdown drain pumps by itself.
fn pump_scheduler() -> PumpScheduler {
    PumpScheduler::new(|delay_ms| {
        let _ = slint::invoke_from_event_loop(move || deliver(Wake::Schedule(delay_ms)));
    })
}

pub fn quit() {
    // Fails only when no event loop runs, and the pump runs inside it.
    let _ = slint::quit_event_loop();
}

/// The window's display refresh rate. Wayland names the current monitor only
/// once the surface has entered an output, so fall back to the fastest
/// connected display: the CEF frame rate is a ceiling, so too high only costs
/// idle callbacks, while too low would cap the frame rate.
pub fn display_refresh_mhz(window: &slint::winit_030::winit::window::Window) -> Option<u32> {
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

pub fn slint_int(pixels: u32) -> i32 {
    i32::try_from(pixels).unwrap_or(i32::MAX)
}

/// The content rectangle in physical pixels and the window's scale factor.
pub fn content_layout(window: &AppWindow) -> Layout {
    let scale = window.window().scale_factor();
    Layout {
        size: FrameSize::new(
            physical(window.get_content_width(), scale),
            physical(window.get_content_height(), scale),
        ),
        scale,
    }
}

/// A logical length in whole physical pixels.
pub fn physical(logical: f32, scale: f32) -> u32 {
    (logical * scale).round().max(0.0) as u32
}

/// A bundled page as a `data:` URL, so nothing is fetched.
pub fn page_url(html: &str) -> String {
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

/// Hands the shell's tab, Space and command-bar callbacks to the host,
/// each one a model command or a bar update.
fn connect_shell(window: &AppWindow) {
    use wubble_core::model::Command as Cmd;
    let command = |c: Cmd| with_host(|host| host.dispatch(c));
    window.on_tab_activated(move |id| {
        if let Some(t) = shell::tab_id(id) {
            command(Cmd::ActivateTab(t));
        }
    });
    window.on_tab_closed(move |id| {
        if let Some(t) = shell::tab_id(id) {
            command(Cmd::CloseTab(t));
        }
    });
    // A row dragged to slot `to` of the flat tab list.
    window.on_tab_moved(|id, to| {
        if let (Some(tab), Ok(to)) = (shell::tab_id(id), usize::try_from(to)) {
            with_host(|host| host.move_tab(tab, to));
        }
    });
    window.on_space_switched(move |id| {
        if let Some(s) = shell::space_id(id) {
            command(Cmd::SwitchSpace(s));
        }
    });
    let weak = window.as_weak();
    window.on_new_tab_requested(move || {
        if let Some(window) = weak.upgrade() {
            with_host(|host| host.bar_for_new_tab(&window));
        }
    });
    let weak = window.as_weak();
    window.on_address_focus_requested(move || {
        if let Some(window) = weak.upgrade() {
            with_host(|host| host.bar_for_address(&window));
        }
    });
    let weak = window.as_weak();
    window.on_command_query_changed(move |query| {
        if let Some(window) = weak.upgrade() {
            with_host(|host| host.bar_query(&window, &query));
        }
    });
    window.on_command_result_chosen(|index| with_host(|host| host.bar_chosen(index)));
    window.on_tab_mute_toggled(|id| {
        if let Some(tab) = shell::tab_id(id) {
            with_host(|host| host.toggle_mute(tab));
        }
    });
}

/// Hands the page's own UI (menu, dialog sheet, find bar) to the host.
fn connect_surfaces(window: &AppWindow) {
    use surface::FindEvent;
    window.on_menu_chosen(|i| {
        if let Ok(i) = usize::try_from(i) {
            with_host(|host| host.menu_chosen(i));
        }
    });
    window.on_menu_dismissed(|| {
        with_host(|host| {
            host.menu_dismissed();
        })
    });
    window.on_dialog_answered(|accept, text, secret| {
        with_host(|host| host.dialog_answered(accept, &text, &secret))
    });
    window.on_permission_answered(|allow, remember| {
        with_host(|host| host.permission_answered(allow, remember))
    });
    window.on_error_primary(|| with_host(Host::error_primary));
    window.on_error_secondary(|| with_host(Host::error_secondary));
    window.on_download_action(|id, action| {
        with_host(|host| host.download_action(i64::from(id), action))
    });
    window.on_find_edited(|q| with_host(|host| host.find_event(FindEvent::Query(q.into()))));
    window.on_find_step(|forward| {
        let event = if forward {
            FindEvent::Next
        } else {
            FindEvent::Prev
        };
        with_host(|host| host.find_event(event));
    });
    window.on_find_closed(|| with_host(|host| host.find_event(FindEvent::Close)));
}

/// Opens the window on the session stored in `profile` (one web view, the
/// visible tab's, on the shared GPU device) and runs the Slint loop until
/// the window closes or the smoke ends, then saves the session.
fn run(
    mode: Mode,
    config: Result<EngineConfig, EngineError>,
    profile: &Path,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    // The database loads while the GPU and CEF start.
    let (store, loaded) = StoreClient::spawn(
        Profile {
            dir: profile.into(),
        }
        .db_path(),
    );
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
    let none = Checks {
        quit_after: None,
        check_pixels: false,
        nav_smoke: None,
        tabs_smoke: None,
        session_smoke: None,
        page_smoke: None,
    };
    let browsing = matches!(mode, Mode::Browse { .. });
    // Smokes download into their throwaway profile.
    let downloads = match &mode {
        Mode::Browse { downloads, .. } => downloads.clone(),
        _ => Some(profile.join("downloads")),
    };
    // `url` is opened in a new tab; without one a fresh profile gets the
    // bundled page and a restored session opens nothing new.
    let (url, checks) = match mode {
        Mode::Browse { url, .. } => (url, none),
        Mode::Frames { frames, url } => {
            // Only the bundled page has known colours to check against.
            let check_pixels = url.is_none();
            (
                Some(url.unwrap_or_else(bundled)),
                Checks {
                    quit_after: Some(frames),
                    check_pixels,
                    ..none
                },
            )
        }
        Mode::Nav => (
            Some(bundled()),
            Checks {
                nav_smoke: Some(NavSmoke::default()),
                ..none
            },
        ),
        Mode::Tabs => (
            Some(bundled()),
            Checks {
                tabs_smoke: Some(TabsSmoke::default()),
                ..none
            },
        ),
        Mode::Page => (
            Some(page_url(PAGE_PAGE)),
            Checks {
                page_smoke: Some(page_smoke::PageSmoke::default()),
                ..none
            },
        ),
        Mode::Session(phase) => (
            None,
            Checks {
                session_smoke: Some(SessionSmoke::new(phase)),
                ..none
            },
        ),
    };
    // The first pump runs once the loop does; from there CEF schedules every
    // pump itself.
    slint::invoke_from_event_loop(|| deliver(Wake::Schedule(0)))?;
    let engine = Engine::start(EngineConfig {
        pump_scheduler: Some(pump_scheduler()),
        ..config?
    })?;
    // A database that cannot be opened or loaded is left alone: the
    // browser runs without persistence (the store thread has ended).
    let loaded = loaded
        .recv()
        .unwrap_or_else(|_| Err("the store thread ended".into()));
    let loaded = loaded.unwrap_or_else(|e| {
        eprintln!("wubble: running without a saved session: {e}");
        Loaded::default()
    });
    let url = url.or_else(|| loaded.model.tabs.is_empty().then(bundled));
    let window = AppWindow::new()?;
    let glass = Rc::new(RefCell::new(backdrop::choose(&gpu, &window)));
    connect_input(&window);
    connect_shell(&window);
    connect_surfaces(&window);
    wubble_ui::window::wire(&window);
    let panels = install_shell_driver(&window, loaded.settings.ui, &glass)?;
    let host = Host::new(
        engine,
        &window,
        [gpu.presenter(), gpu.presenter()],
        glass,
        panels,
        loaded,
        store,
        url,
        checks,
        downloads,
    );
    HOST.set(Some(host));
    // Smokes drive the lifecycle themselves, never by wall-clock time.
    let lifecycle = slint::Timer::default();
    let audible = slint::Timer::default();
    if browsing {
        lifecycle.start(slint::TimerMode::Repeated, LIFECYCLE_TICK, || {
            with_host(|host| host.tick(Instant::now()))
        });
        audible.start(slint::TimerMode::Repeated, AUDIBLE_PROBE_EVERY, || {
            with_host(Host::probe_audible)
        });
    }

    let result = window.run();
    // Read while the window still exists.
    let ui = host::ui_state(&window);
    drop(lifecycle);
    drop(audible);
    drop(window);
    let mut host = HOST.take().ok_or("the host went missing")?;
    host.shutdown(ui);
    drop(host.timer);
    drain(&mut host.engine);
    result?;
    Ok(Outcome {
        stats: host.stats.clone(),
        failure: host.failure.take().map(Into::into),
        nav_title: host.nav_title.take(),
        tabs_passed: host.tabs_passed,
        session_passed: host.session_passed,
        page_zoom: host.page_zoom,
    })
}
