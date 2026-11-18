mod autofill_smoke;
mod backdrop;
mod bar;
mod bind;
mod block_smoke;
mod blocking;
mod chooser;
mod cli;
mod host;
mod ime;
mod input;
mod instance;
mod measure;
mod menu;
mod nav_smoke;
mod page_error;
mod page_smoke;
mod pages_smoke;
mod panes;
mod perform;
mod persist;
mod pixels;
mod profile;
mod prompt;
mod screen;
mod session_smoke;
mod settings_smoke;
mod shell;
mod split_smoke;
mod surface;
mod tabs_smoke;
mod windows_smoke;

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
use split_smoke::SplitSmoke;
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
pub const BLOCK_PAGE: &str = include_str!("../assets/block.html");

/// `--smoke tabs`'s pages, in tab order.
pub const TABS_SMOKE_PAGES: [&str; 3] = [SMOKE_PAGE, NAV_PAGE, TAB3_PAGE];

/// How often hidden tabs are checked against the sleep policy. Coarse: the
/// policy's delays are a minute and more.
const LIFECYCLE_TICK: Duration = Duration::from_secs(5);

/// How often live tabs are asked whether they play sound (the speaker glyph;
/// CEF has no audible callback, see `wubble_engine::media`).
const AUDIBLE_PROBE_EVERY: Duration = Duration::from_secs(2);

use wubble_core::model::Command as Cmd;
use wubble_core::window::{WindowId, WindowKind};

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
        Command::Smoke(Smoke::Windows) => {
            match with_smoke_dir(|config, dir| run(Mode::Windows, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    windows_passed: true,
                    ..
                }) => {
                    println!("smoke: windows ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: windows FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: windows FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Split) => {
            match with_smoke_dir(|config, dir| run(Mode::Split, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    split_passed: true,
                    ..
                }) => {
                    println!("smoke: split ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: split FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: split FAILED: the window closed first");
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
        Command::Smoke(Smoke::Block) => {
            match with_smoke_dir(|config, dir| run(Mode::Block, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    blocked_seen: Some(blocked),
                    ..
                }) => {
                    println!("smoke: block ok");
                    log::debug!("block smoke counted {blocked}");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: block FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: block FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Settings) => {
            match with_smoke_dir(|config, dir| run(Mode::Settings, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    settings_passed: true,
                    ..
                }) => {
                    println!("smoke: settings ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: settings FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: settings FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Pages) => {
            match with_smoke_dir(|config, dir| run(Mode::Pages, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    pages_passed: true,
                    ..
                }) => {
                    println!("smoke: pages ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: pages FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: pages FAILED: the window closed first");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Autofill) => {
            match with_smoke_dir(|config, dir| run(Mode::Autofill, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    autofill_passed: true,
                    ..
                }) => {
                    println!("smoke: autofill ok");
                    ExitCode::SUCCESS
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("smoke: autofill FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("smoke: autofill FAILED: the window closed first");
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
        Command::Measure(m) => {
            measure::begin(m.scenario, m.seconds.get());
            let mode = Mode::Measure {
                scenario: m.scenario,
                begin_frames: m.begin_frames,
            };
            match with_smoke_dir(|config, dir| run(mode, config, dir)) {
                Ok(Outcome {
                    failure: None,
                    measured: Some((report, csv)),
                    ..
                }) => {
                    println!("{report}");
                    match m.out {
                        Some(path) => match measure::write_csv(&path, &csv) {
                            Ok(()) => {
                                println!("measure: samples written to {}", path.display());
                                ExitCode::SUCCESS
                            }
                            Err(e) => {
                                eprintln!("measure: cannot write {}: {e}", path.display());
                                ExitCode::FAILURE
                            }
                        },
                        None => ExitCode::SUCCESS,
                    }
                }
                Ok(Outcome {
                    failure: Some(e), ..
                })
                | Err(e) => {
                    eprintln!("measure: FAILED: {e}");
                    ExitCode::FAILURE
                }
                Ok(Outcome { .. }) => {
                    eprintln!("measure: FAILED: nothing was recorded");
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
    id: WindowId,
    ui: persist::UiState,
    glass: &Rc<RefCell<Glass>>,
) -> Result<Rc<RefCell<GlassRegistry>>, Box<dyn std::error::Error>> {
    let seam = Seam::restored(ui.sidebar_width.into(), ui.mode);
    let glass = Rc::clone(glass);
    let panels = wubble_ui::driver::install(
        window,
        seam,
        move |window, now, items| {
            if let Ok(mut glass) = glass.try_borrow_mut() {
                glass.frame(window, now, items);
            }
            // The panes (split strings, sizes, stretch), stepped with the rest of the shell.
            // Inside rendering the host may be busy (a pump renders); then this frame skips
            // them and asks for the next.
            if panes_frame(id, window, now) {
                window.window().request_redraw();
            }
        },
        || {
            measure::rendered();
            // Queued: the render may run while the host is busy (snapshots).
            let _ = slint::invoke_from_event_loop(|| {
                with_host(|host| host.pace(wubble_gfx::pacing::PaceEvent::Vsync))
            });
        },
    )?;
    Ok(panels)
}

/// Pumps until every browser is closed and CEF has shut down. Bounded by the
/// lifecycle state, not by time: the engine closes its views and reports each
/// one closed once CEF has let go of it.
fn drain(engine: &mut Engine) {
    trace("shutdown requested");
    engine.request_shutdown();
    let mut ticks = 0u64;
    while !engine.is_stopped() {
        engine.tick();
        ticks += 1;
        // A shutdown that never ends shows as a count that keeps growing.
        if ticks.is_power_of_two() && ticks >= 1 << 10 {
            trace(&format!("still shutting down after {ticks} ticks"));
        }
    }
    trace(&format!("stopped after {ticks} ticks"));
}

/// `WUBBLE_TRACE=1`: stage lines on stderr with the time since the first one, for finding where
/// a run that hangs (once, 300 s, right after a rebuild: never reproduced) is stuck. Take
/// `eu-stack -p <pid>` of the process and its children next to the last line. Off by default.
fn trace(stage: &str) {
    static START: std::sync::OnceLock<Option<Instant>> = std::sync::OnceLock::new();
    let start = START.get_or_init(|| std::env::var_os("WUBBLE_TRACE").map(|_| Instant::now()));
    if let Some(start) = start {
        eprintln!(
            "wubble: trace +{}ms pid {}: {stage}",
            start.elapsed().as_millis(),
            std::process::id()
        );
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
    // One browser per profile, decided before anything opens the store or starts CEF.
    std::fs::create_dir_all(&profile.dir)?;
    let socket = instance::socket_path(&profile.dir, &instance::runtime_dir());
    let message = url
        .clone()
        .map_or(instance::Message::Focus, instance::Message::Open);
    let listener = match instance::claim(&socket, &message) {
        Ok(instance::Claim::Forwarded) => return Ok(Outcome::default()),
        Ok(instance::Claim::Primary(listener)) => Some(listener),
        Err(e) => {
            eprintln!("wubble: running without the single-instance socket: {e}");
            None
        }
    };
    let config = EngineConfig::from_current_exe().map(|config| EngineConfig {
        cache_dir: profile.cef_cache(),
        ..config
    });
    let downloads = profile::downloads_dir(downloads);
    let mode = Mode::Browse {
        url,
        downloads,
        listener,
    };
    run(mode, config, &profile.dir)
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

/// `--smoke autofill`: the login the key menu must offer on the smoke's page.
fn seed_autofill_login(store: &StoreClient, port: u16) {
    let origin = format!("http://127.0.0.1:{port}");
    store.vault(move |vault| {
        let Ok(origin) = wubble_vault::Origin::parse(&origin) else {
            return;
        };
        let login = wubble_vault::NewLogin {
            origin,
            form_action_origin: None,
            username: autofill_smoke::SEEDED_USER.into(),
            password: zeroize::Zeroizing::new(autofill_smoke::SEEDED_PASSWORD.into()),
        };
        if let Err(e) = vault.save(login, 0) {
            eprintln!("smoke: autofill: the login was not seeded: {e}");
        }
    });
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
    let config = config?;
    trace(&format!(
        "starting CEF from {}, cache {}",
        config.exe_dir.display(),
        config.cache_dir.display()
    ));
    let mut engine = Engine::start(config)?;
    trace("CEF started");
    for _ in 0..SMOKE_TICKS {
        engine.tick();
    }
    trace("ticked");
    drain(&mut engine);
    Ok(())
}

/// How a windowed run ended.
#[derive(Default)]
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
    /// The blocked count `--smoke block` passed with.
    blocked_seen: Option<u32>,
    /// `--smoke split` reached its end.
    split_passed: bool,
    /// `--smoke windows` reached its end.
    windows_passed: bool,
    /// `--smoke settings` reached its end.
    settings_passed: bool,
    pages_passed: bool,
    /// `--smoke autofill` reached its end.
    autofill_passed: bool,
    /// `--measure`'s report and CSV.
    measured: Option<(String, String)>,
}

/// What the window is for.
enum Mode {
    /// `url`, or the bundled page if `None`; downloads go to `downloads`.
    Browse {
        url: Option<String>,
        downloads: Option<PathBuf>,
        /// Later starts on the same profile (`instance.rs`).
        listener: Option<interprocess::local_socket::Listener>,
    },
    /// Quit after `frames` accelerated frames of `url` (bundled page if `None`).
    Frames { frames: u64, url: Option<String> },
    /// `--smoke nav`.
    Nav,
    /// `--smoke tabs`.
    Tabs,
    /// `--smoke split`.
    Split,
    /// `--smoke windows`.
    Windows,
    /// `--smoke page`.
    Page,
    /// `--smoke block`.
    Block,
    /// `--smoke settings`.
    Settings,
    /// `--smoke pages`.
    Pages,
    /// `--smoke autofill`.
    Autofill,
    /// One phase of `--smoke session`.
    Session(Phase),
    /// `--measure`: the scenario's bundled page, measured until it ends;
    /// CEF paced by external begin frames if `begin_frames`.
    Measure {
        scenario: measure::Scenario,
        begin_frames: bool,
    },
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

/// Runs `f` on the host, if there is one and nothing else is using it, in the focused window.
/// Called on the Slint thread only.
fn with_host(f: impl FnOnce(&mut Host)) {
    HOST.with(|cell| match cell.try_borrow_mut() {
        Ok(mut host) => {
            if let Some(host) = host.as_mut() {
                host.enter_focused();
                f(host);
            }
        }
        Err(_) => eprintln!("wubble: host busy, input or pump request dropped"),
    });
}

/// Runs `f` on the host in window `id` (one of its callbacks), if that window is still open.
pub(crate) fn with_host_in(id: WindowId, f: impl FnOnce(&mut Host)) {
    HOST.with(|cell| match cell.try_borrow_mut() {
        Ok(mut host) => {
            if let Some(host) = host.as_mut()
                && host.enter(id)
            {
                f(host);
            }
        }
        Err(_) => eprintln!("wubble: host busy, input or pump request dropped"),
    });
}

/// The panes' frame of window `id` from its rendering notifier: whether another frame is
/// needed. A busy host (rendering inside a pump) skips this one and asks for the next; no host
/// needs none.
fn panes_frame(id: WindowId, window: &AppWindow, now: Instant) -> bool {
    HOST.with(|cell| match cell.try_borrow_mut() {
        Ok(mut host) => host
            .as_mut()
            .is_some_and(|host| host.enter(id) && host.panes_frame(window, now)),
        Err(_) => true,
    })
}

/// What a new window is made from: the shared GPU device and the glass setting.
pub(crate) struct WindowFactory {
    pub gpu: GpuContext,
    pub glass_scene: bool,
}

impl WindowFactory {
    /// A platform window for model window `id`: the shell, its glass and spring driver, every
    /// callback wired to the host in that window. Not shown yet.
    pub(crate) fn open(
        &self,
        id: WindowId,
        kind: WindowKind,
        ui: persist::UiState,
        geometry: Option<wubble_core::window::Geometry>,
    ) -> Result<host::WindowHost, Box<dyn std::error::Error>> {
        let window = AppWindow::new()?;
        window.set_window_kind(kind.index());
        if let Some(g) = geometry {
            host::place_window(&window, g);
        }
        let glass = Rc::new(RefCell::new(backdrop::choose(
            &self.gpu,
            &window,
            self.glass_scene,
        )));
        connect_input(&window, id);
        connect_shell(&window, id);
        connect_surfaces(&window, id);
        wubble_ui::window::wire(&window);
        // The shell's close button and the window manager's close the window, not the app:
        // the model says when the last Main window is gone (`Effect::LastWindowClosed`).
        window.on_window_close_requested(move || {
            with_host_in(id, |host| host.dispatch(Cmd::CloseWindow(id)))
        });
        window.window().on_close_requested(move || {
            with_host_in(id, |host| host.dispatch(Cmd::CloseWindow(id)));
            slint::CloseRequestResponse::KeepWindowShown
        });
        // Little and Popup windows show their page alone.
        let ui = match kind {
            WindowKind::Main => ui,
            WindowKind::Little | WindowKind::Popup => persist::UiState {
                mode: wubble_core::seam::SidebarMode::Hidden,
                ..ui
            },
        };
        let panels = install_shell_driver(&window, id, ui, &glass)?;
        let gpu = &self.gpu;
        Ok(host::WindowHost::new(
            id,
            kind,
            window,
            [
                gpu.presenter(),
                gpu.presenter(),
                gpu.presenter(),
                gpu.presenter(),
            ],
            glass,
            panels,
        ))
    }
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
        area: None,
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
fn connect_input(window: &AppWindow, win: WindowId) {
    let point = |x: f32, y: f32| Point { x, y };
    window.on_web_pointer_moved(move |pane, x, y| {
        with_host_in(win, |host| host.pointer_moved(pane, point(x, y)))
    });
    window.on_web_pointer_left(move |pane, x, y| {
        with_host_in(win, |host| host.pointer_left(pane, point(x, y)))
    });
    window.on_web_pointer_pressed(move |pane, button, x, y| {
        with_host_in(win, |host| {
            host.pointer_button(pane, button, true, point(x, y))
        })
    });
    window.on_web_pointer_released(move |pane, button, x, y| {
        with_host_in(win, |host| {
            host.pointer_button(pane, button, false, point(x, y))
        })
    });
    window.on_web_pointer_cancelled(move |pane| {
        with_host_in(win, |host| host.pointer_cancelled(pane))
    });
    window.on_web_scrolled(move |pane, x, y, dx, dy| {
        with_host_in(win, |host| host.scrolled(pane, point(x, y), dx, dy))
    });
    // The split's seams and the panes' close buttons.
    window.on_split_grab(move |j, x, y| with_host_in(win, |host| host.split_grab(j, x, y)));
    window.on_split_drag(move |x, y| with_host_in(win, |host| host.split_drag(x, y)));
    window.on_split_release(move || with_host_in(win, Host::split_release));
    window.on_split_even(move || with_host_in(win, Host::split_even));
    window.on_pane_close(move |id| with_host_in(win, |host| host.pane_close(id)));
    let weak = window.as_weak();
    window.on_web_focus_changed(move |_| {
        if let Some(window) = weak.upgrade() {
            with_host_in(win, |host| host.sync_focus(&window));
        }
    });
    window.window().on_winit_window_event(move |_, event| {
        let mut result = EventResult::Propagate;
        with_host_in(win, |host| result = host.window_event(event));
        result
    });
}

/// Hands the shell's tab, Space and command-bar callbacks to the host,
/// each one a model command or a bar update.
fn connect_shell(window: &AppWindow, win: WindowId) {
    use wubble_core::model::Command as Cmd;
    let command = move |c: Cmd| with_host_in(win, |host| host.dispatch(c));
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
    window.on_tab_moved(move |id, to| {
        if let (Some(tab), Ok(to)) = (shell::tab_id(id), usize::try_from(to)) {
            with_host_in(win, |host| host.move_tab(tab, to));
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
            with_host_in(win, |host| host.bar_for_new_tab(&window));
        }
    });
    let weak = window.as_weak();
    window.on_address_focus_requested(move || {
        if let Some(window) = weak.upgrade() {
            with_host_in(win, |host| host.bar_for_address(&window));
        }
    });
    let weak = window.as_weak();
    window.on_command_query_changed(move |query| {
        if let Some(window) = weak.upgrade() {
            with_host_in(win, |host| host.bar_query(&window, &query));
        }
    });
    window.on_command_result_chosen(move |index| with_host_in(win, |host| host.bar_chosen(index)));
    window.on_tab_context(move |id, x, y| with_host_in(win, |host| host.tab_context(id, x, y)));
    window.on_tab_track(move |id, kind, x, y| {
        with_host_in(win, |host| host.tab_track(id, kind, x, y))
    });
    let weak = window.as_weak();
    window.on_beside_requested(move || {
        if let Some(window) = weak.upgrade() {
            with_host_in(win, |host| host.bar_for_beside(&window));
        }
    });
    window.on_tab_mute_toggled(move |id| {
        if let Some(tab) = shell::tab_id(id) {
            with_host_in(win, |host| host.toggle_mute(tab));
        }
    });
    // The sidebar's buttons name their action; it runs like its shortcut.
    window.on_shell_action(move |id| match wubble_core::action::Action::from_id(&id) {
        Some(action) => with_host_in(win, |host| host.perform(action)),
        None => eprintln!("wubble: the shell asked for an unknown action {id:?}"),
    });
}

/// Hands the page's own UI (menu, dialog sheet, find bar) to the host.
fn connect_surfaces(window: &AppWindow, win: WindowId) {
    use surface::FindEvent;
    window.on_menu_chosen(move |i| {
        if let Ok(i) = usize::try_from(i) {
            with_host_in(win, |host| host.menu_chosen(i));
        }
    });
    window.on_menu_dismissed(move || {
        with_host_in(win, |host| {
            host.menu_dismissed();
        })
    });
    window.on_dialog_answered(move |accept, text, secret| {
        with_host_in(win, |host| host.dialog_answered(accept, &text, &secret))
    });
    window.on_permission_answered(move |allow, remember| {
        with_host_in(win, |host| host.permission_answered(allow, remember))
    });
    window.on_error_primary(move || with_host_in(win, Host::error_primary));
    window.on_error_secondary(move || with_host_in(win, Host::error_secondary));
    window.on_download_action(move |id, action| {
        with_host_in(win, |host| host.download_action(i64::from(id), action))
    });
    window.on_find_edited(move |q| {
        with_host_in(win, |host| host.find_event(FindEvent::Query(q.into())))
    });
    window.on_find_step(move |forward| {
        let event = if forward {
            FindEvent::Next
        } else {
            FindEvent::Prev
        };
        with_host_in(win, |host| host.find_event(event));
    });
    window.on_find_closed(move || with_host_in(win, |host| host.find_event(FindEvent::Close)));
    connect_settings(window, win);
    connect_pages(window, win);
    window.on_autofill_menu(move |i| with_host_in(win, |host| host.autofill_menu(i)));
    window.on_autofill_answer(move |i| with_host_in(win, |host| host.autofill_answer(i)));
}

/// Opens the window on the session stored in `profile` (one web view, the
/// visible tab's, on the shared GPU device) and runs the Slint loop until
/// the window closes or the smoke ends, then saves the session.
/// The settings page's intents: each becomes one host call.
fn connect_settings(window: &AppWindow, win: WindowId) {
    use wubble_core::action::Action;
    use wubble_core::keys::CaptureEvent;
    window.on_settings_requested(move || {
        with_host_in(win, |host| host.perform(Action::OpenSettings))
    });
    window.on_settings_query_changed(move |q| with_host_in(win, |host| host.settings_query(&q)));
    window.on_setting_changed(move |key, value| {
        with_host_in(win, |host| {
            host.write_setting(&key, &value);
        })
    });
    window.on_setting_reset(move |key| with_host_in(win, |host| host.setting_reset(&key)));
    window.on_setting_entry_removed(move |key, entry| {
        with_host_in(win, |host| host.setting_entry_removed(&key, &entry))
    });
    window.on_setting_entry_added(move |key, entry| {
        with_host_in(win, |host| host.setting_entry_added(&key, &entry))
    });
    window.on_binding_record(move |id| with_host_in(win, |host| host.binding_record(&id)));
    window.on_binding_reset(move |id| with_host_in(win, |host| host.binding_reset(&id)));
    window.on_capture_confirm(move || {
        with_host_in(win, |host| host.capture_event(CaptureEvent::Confirm))
    });
    window.on_capture_cancel(move || {
        with_host_in(win, |host| host.capture_event(CaptureEvent::Cancel))
    });
    window
        .on_space_renamed(move |id, name| with_host_in(win, |host| host.space_renamed(id, &name)));
    window.on_space_tinted(move |id, tint| with_host_in(win, |host| host.space_tinted(id, tint)));
    window.on_space_icon_chosen(move |id, icon| {
        with_host_in(win, |host| host.space_icon_chosen(id, &icon))
    });
    window.on_site_zoom_removed(move |h| with_host_in(win, |host| host.site_zoom_removed(&h)));
    window.on_site_permission_removed(move |i| {
        with_host_in(win, |host| host.site_permission_removed(&i))
    });
}

/// The native pages' intents (history, bookmarks, downloads): each becomes one host call.
fn connect_pages(window: &AppWindow, win: WindowId) {
    window.on_history_query_changed(move |q| with_host_in(win, |host| host.history_query(&q)));
    window.on_history_deleted(move |url| with_host_in(win, |host| host.history_deleted(&url)));
    window.on_history_more(move || with_host_in(win, Host::history_more));
    window.on_history_cleared(move |range| with_host_in(win, |host| host.history_cleared(range)));
    window.on_page_opened(move |url| with_host_in(win, |host| host.page_opened(&url)));
    window.on_page_opened_new_tab(move |url| {
        with_host_in(win, |host| host.page_opened_new_tab(&url))
    });
    window.on_bookmarks_query_changed(move |q| with_host_in(win, |host| host.bookmarks_query(&q)));
    window.on_bookmark_toggled(move |id| with_host_in(win, |host| host.bookmark_toggled(id)));
    window.on_bookmark_rename_requested(move |id| {
        with_host_in(win, |host| host.bookmark_rename_requested(id))
    });
    window.on_bookmark_move_requested(move |id| {
        with_host_in(win, |host| host.bookmark_move_requested(id))
    });
    window.on_bookmark_deleted(move |id| with_host_in(win, |host| host.bookmark_deleted(id)));
    window.on_bookmark_folder_requested(move || with_host_in(win, Host::bookmark_folder_requested));
    window.on_bookmark_sheet_answered(move |accept, remove, name, folder| {
        with_host_in(win, |host| {
            host.bookmark_sheet_answered(accept, remove, &name, folder)
        })
    });
    window.on_address_bookmark_requested(move || with_host_in(win, Host::bookmark_page));
    window.on_address_blocker_requested(move |x, y| {
        with_host_in(win, |host| host.blocker_menu_requested(x, y))
    });
    window.on_address_popups_requested(move |x, y| {
        with_host_in(win, |host| host.popups_menu_requested(x, y))
    });
    window.on_downloads_cleared(move || with_host_in(win, Host::downloads_cleared));
    window.on_passwords_query_changed(move |q| with_host_in(win, |host| host.passwords_query(&q)));
    window.on_passwords_unlock(move || with_host_in(win, Host::passwords_unlock));
    window.on_login_reveal(move |id| with_host_in(win, |host| host.login_reveal(&id)));
    window.on_login_hide(move || with_host_in(win, Host::login_hide));
    window.on_login_copy(move |id| with_host_in(win, |host| host.login_copy(&id)));
    window.on_login_deleted(move |id| with_host_in(win, |host| host.login_deleted(&id)));
    window.on_login_added(move |site, user, password| {
        with_host_in(win, |host| host.login_added(&site, &user, &password))
    });
    window
        .on_passwords_import_requested(move || with_host_in(win, Host::passwords_import_requested));
}

fn run(
    mut mode: Mode,
    config: Result<EngineConfig, EngineError>,
    profile: &Path,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let listener = match &mut mode {
        Mode::Browse { listener, .. } => listener.take(),
        _ => None,
    };
    // `--smoke autofill`'s pages, on a loopback port.
    let autofill_port = matches!(mode, Mode::Autofill)
        .then(autofill_smoke::serve)
        .transpose()?;
    let db = Profile {
        dir: profile.into(),
    }
    .db_path();
    // The browser keeps the vault's key in the OS keyring; smokes run on
    // throwaway profiles and never touch it. The pages smoke fills the
    // profile first, its vault key going into the fake keyring the store
    // thread then gets.
    let keyring: Box<dyn wubble_vault::KeyRing + Send> = match mode {
        Mode::Browse { .. } => Box::new(wubble_vault::OsKeyRing),
        Mode::Pages => {
            let keyring = wubble_vault::FakeKeyRing::default();
            pages_smoke::seed(&db, &keyring)?;
            Box::new(keyring)
        }
        _ => Box::new(wubble_vault::FakeKeyRing::default()),
    };
    // The database loads while the GPU and CEF start.
    let (store, loaded) = StoreClient::spawn(db, keyring);
    if let Some(port) = autofill_port {
        seed_autofill_login(&store, port);
    }
    let gpu = GpuContext::new()?;
    eprintln!("wubble: GPU {}", gpu.describe());
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_30(gpu.slint_configuration())
        .with_winit_window_attributes_hook(wubble_ui::window::attributes)
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
        block_smoke: None,
        split_smoke: None,
        settings_smoke: None,
        pages_smoke: None,
        autofill_smoke: None,
        windows_smoke: None,
        begin_frames: false,
    };
    let browsing = matches!(mode, Mode::Browse { .. });
    let smoke_block = matches!(mode, Mode::Block);
    let measuring = matches!(mode, Mode::Measure { .. });
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
        Mode::Split => (
            Some(bundled()),
            Checks {
                split_smoke: Some(SplitSmoke::default()),
                ..none
            },
        ),
        Mode::Windows => (
            Some(bundled()),
            Checks {
                windows_smoke: Some(windows_smoke::WindowsSmoke::default()),
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
        Mode::Settings => (
            Some(bundled()),
            Checks {
                settings_smoke: Some(settings_smoke::SettingsSmoke::default()),
                ..none
            },
        ),
        Mode::Pages => (
            Some(bundled()),
            Checks {
                pages_smoke: Some(pages_smoke::PagesSmoke::default()),
                ..none
            },
        ),
        Mode::Block => (
            Some(page_url(BLOCK_PAGE)),
            Checks {
                block_smoke: Some(block_smoke::BlockSmoke::default()),
                ..none
            },
        ),
        Mode::Autofill => (
            autofill_port.map(|port| format!("http://127.0.0.1:{port}/")),
            Checks {
                autofill_smoke: Some(autofill_smoke::AutofillSmoke::default()),
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
        Mode::Measure {
            scenario,
            begin_frames,
        } => (
            Some(page_url(scenario.page())),
            Checks {
                begin_frames,
                ..none
            },
        ),
    };
    // The first pump runs once the loop does; from there CEF schedules every
    // pump itself.
    slint::invoke_from_event_loop(|| deliver(Wake::Schedule(0)))?;
    // A database that cannot be opened or loaded is left alone: the
    // browser runs without persistence (the store thread has ended). Read
    // before CEF starts: some settings are CEF start switches.
    let loaded = loaded
        .recv()
        .unwrap_or_else(|_| Err("the store thread ended".into()));
    let loaded = loaded.unwrap_or_else(|e| {
        eprintln!("wubble: running without a saved session: {e}");
        Loaded::default()
    });
    let blocking = blocking::Blocking::new();
    let engine = Engine::start(EngineConfig {
        pump_scheduler: Some(pump_scheduler()),
        content_filter: Some(blocking.clone()),
        features: loaded.settings.page_features(),
        ..config?
    })?;
    if smoke_block {
        // Only the smoke's own list, through the same filter wiring as the browser's.
        blocking.use_list(block_smoke::BLOCK_LIST);
    } else {
        blocking.start(profile, &loaded.settings.blocker);
    }
    let url = url.or_else(|| loaded.model.tabs.is_empty().then(bundled));
    let factory = WindowFactory {
        gpu: gpu.clone(),
        glass_scene: loaded.settings.glass_scene(),
    };
    // The focused window first; the host opens the session's others.
    let focused = loaded.model.focused();
    let geometry = loaded.model.window(focused).and_then(|w| w.geometry);
    let first = factory.open(focused, WindowKind::Main, loaded.settings.ui, geometry)?;
    let window = first.handle();
    let host = Host::new(
        engine, first, factory, loaded, store, url, checks, downloads, blocking,
    );
    HOST.set(Some(host));
    if let Some(listener) = listener {
        instance::listen(listener, |message| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.open_external(message))
            });
        });
    }
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
    let measure_timer = slint::Timer::default();
    if measuring {
        let weak = window.as_weak();
        measure_timer.start(slint::TimerMode::Repeated, measure::TICK, move || {
            if let Some(window) = weak.upgrade() {
                with_host(|host| measure::tick(host, &window));
            }
        });
    }

    window.show()?;
    with_host(Host::open_session_windows);
    let result = slint::run_event_loop();
    // Read while the windows still exist.
    let ui = HOST.with_borrow_mut(|host| {
        host.as_mut().map(|host| {
            host.record_geometry();
            host.ui_state()
        })
    });
    let ui = ui.unwrap_or_else(|| host::ui_state(&window));
    drop(measure_timer);
    let measured = measuring
        .then(|| measure::finish(&gpu.describe()))
        .flatten();
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
        split_passed: host.split_passed,
        windows_passed: host.windows_passed,
        settings_passed: host.settings_passed,
        pages_passed: host.pages_passed,
        session_passed: host.session_passed,
        page_zoom: host.page_zoom,
        blocked_seen: host.blocked_seen,
        autofill_passed: host.autofill_passed,
        measured,
    })
}
