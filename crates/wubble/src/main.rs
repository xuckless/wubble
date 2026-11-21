mod alloc_count;
mod autofill_smoke;
mod ax_smoke;
mod backdrop;
mod bar;
mod bind;
mod block_smoke;
mod blocking;
mod builtin_smoke;
mod chooser;
mod cli;
mod clipboard;
mod codecs_smoke;
mod data_smoke;
mod extensions;
mod extensions_smoke;
mod gallery_smoke;
mod host;
mod ime;
mod input;
mod instance;
mod load_run;
mod load_server;
mod mcp;
mod mcp_smoke;
mod measure;
mod media_keys;
mod media_smoke;
mod memory;
mod menu;
mod nav_smoke;
mod orbs;
mod os_motion;
mod os_scope;
mod overlay_smoke;
mod page_drag;
mod page_error;
mod page_smoke;
mod pages_smoke;
mod pagescripts;
mod panes;
mod parity;
mod partial;
mod passwords_smoke;
mod perform;
mod persist;
mod pixels;
mod profile;
mod prompt;
mod quit_smoke;
mod run_state;
mod screen;
mod screen_reader;
mod session_smoke;
mod settings_smoke;
mod shell;
mod shot;
mod shot_io;
mod slint_backend;
mod split_smoke;
mod sponsor_net;
mod start_smoke;
mod suggest_fx;
mod surface;
mod tabs_smoke;
mod ui_loop;
mod update_smoke;
mod updater;
mod verdict;
mod video_smoke;
mod windows_smoke;
mod wl;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
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
use verdict::Verdict;
use wubble_core::seam::Seam;
use wubble_engine::app::{self, Engine, EngineConfig, EngineError, ProfileKind, PumpScheduler};
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
/// `--smoke overlay`'s page: green on top, red below (`#dark`: dark).
const OVERLAY_PAGE: &str = include_str!("../assets/overlay.html");

/// `--smoke gallery`'s plain white page (`#dark`: near-black), for the glass over it.
pub const GROUND_PAGE: &str = include_str!("../assets/ground.html");

/// The page `--smoke nav` navigates to.
pub const NAV_PAGE: &str = include_str!("../assets/nav.html");

/// The third page of `--smoke tabs`.
pub const TAB3_PAGE: &str = include_str!("../assets/tab3.html");

/// `--smoke tabs`'s freeze check: a page whose title counts its timer's ticks.
pub const TIMERS_PAGE: &str = include_str!("../assets/timers.html");

/// The page `--smoke page` drives.
pub const PAGE_PAGE: &str = include_str!("../assets/page.html");
pub const PARITY_PAGE: &str = include_str!("../assets/parity.html");
pub const BLOCK_PAGE: &str = include_str!("../assets/block.html");
/// `--smoke builtins`' pages: an article (reader, dark mode, link hints), a page 3000 CSS px
/// tall (screenshots) and a one-page PDF with the word "needle" (Chromium's viewer).
pub const ARTICLE_PAGE: &str = include_str!("../assets/article.html");
pub const TALL_PAGE: &str = include_str!("../assets/tall.html");
const DOC_PDF: &[u8] = include_bytes!("../assets/doc.pdf");

/// `--smoke tabs`'s pages, in tab order.
pub const TABS_SMOKE_PAGES: [&str; 3] = [SMOKE_PAGE, NAV_PAGE, TAB3_PAGE];

/// How often hidden tabs are checked against the sleep policy. Coarse: the
/// policy's delays are a minute and more.
const LIFECYCLE_TICK: Duration = Duration::from_secs(5);

/// How often the pages on screen are asked about their video (`host/videos.rs`): the controls
/// follow the video's place and time at this pace.
const MEDIA_POLL_EVERY: Duration = Duration::from_millis(250);

/// How often the video controls' hide clock is checked (`Host::tick_video`): finer than a poll,
/// as they go 250 ms after the pointer leaves.
const VIDEO_TICK_EVERY: Duration = Duration::from_millis(100);

/// How often live tabs are asked whether they play sound (the speaker glyph;
/// CEF has no audible callback, see `wubble_engine::media`).
const AUDIBLE_PROBE_EVERY: Duration = Duration::from_secs(2);

use wubble_core::model::Command as Cmd;
use wubble_core::window::{WindowId, WindowKind};

fn main() -> ExitCode {
    // `--measure startup` counts from here.
    measure::process_started();
    // Before anything touches the OS's application object (winit, CEF).
    app::prepare_process();
    let command = match cli::parse_with_shot(std::env::args_os()) {
        Ok((command, shot)) => {
            shot::request(shot);
            command
        }
        Err(e) => e.exit(),
    };
    // Still one thread: the scope must hold every process CEF starts later.
    os_scope::own_scope(matches!(command, Command::Run { .. }));
    match command {
        Command::Run {
            url,
            profile,
            downloads,
        } => match browse(url, profile, downloads) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("wubble: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Mcp { profile, list } => match mcp_relay(profile, list) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("wubble mcp: {e}");
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
        Command::Smoke(Smoke::Quit) => match with_smoke_dir(|config, _| quit_smoke::run(config)) {
            Ok(devtools) => {
                println!("smoke: quit ok devtools={devtools}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: quit FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Update) => match update_smoke::run() {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: update FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Codecs { wired }) => {
            match with_smoke_dir(|config, dir| codecs_smoke::run(config, dir, wired)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("smoke: codecs FAILED: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Smoke(Smoke::Extensions { installed }) => {
            match with_smoke_dir(|config, dir| extensions_smoke::run(config, dir, installed)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("smoke: extensions FAILED: {e}");
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
        Command::Smoke(Smoke::SessionPhase { phase, profile }) => match smoke_profile(&profile) {
            Ok(()) => {
                with_smoke_dir(|config, _| report_run(Mode::Session(phase), config, &profile))
            }
            Err(e) => {
                eprintln!("smoke: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Passwords) => match smoke_passwords() {
            Ok(()) => {
                println!("smoke: passwords ok");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: passwords FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::PasswordsPhase { phase, profile }) => match smoke_profile(&profile) {
            Ok(()) => {
                with_smoke_dir(|config, _| report_run(Mode::Passwords(phase), config, &profile))
            }
            Err(e) => {
                eprintln!("smoke: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Data) => match smoke_data() {
            Ok(()) => {
                println!("smoke: data ok");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: data FAILED: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::DataPhase { ending, profile }) => match smoke_profile(&profile) {
            Ok(()) => with_smoke_dir(|config, _| report_run(Mode::Data(ending), config, &profile)),
            Err(e) => {
                eprintln!("smoke: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Measure(m) if m.scenario == measure::Scenario::Blocker => {
            println!("{}", blocking::measure());
            ExitCode::SUCCESS
        }
        Command::Measure(m) if m.load.serve_only.is_some() => {
            let port = m.load.serve_only.unwrap_or_default();
            match load_server::LoadServer::start(port, true) {
                Ok(server) => {
                    println!("serving http://127.0.0.1:{}", server.port);
                    // Until killed; the pages post their timings, printed as they arrive.
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    eprintln!("wubble: cannot serve: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Measure(m) => {
            measure::begin(
                m.scenario,
                m.seconds.get(),
                m.warmup.map(|w| Duration::from_secs(w.get().into())),
            );
            let load = (m.scenario == measure::Scenario::Load)
                .then(|| load_server::LoadServer::start(0, false).map(|s| (s, m.load.variant)))
                .transpose();
            let load = match load {
                Ok(load) => load,
                Err(e) => {
                    eprintln!("wubble: cannot serve the load pages: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if let Some((server, variant)) = &load {
                let pages = if m.load.pages.is_empty() {
                    load_server::PAGES.map(String::from).to_vec()
                } else {
                    m.load.pages.clone()
                };
                load_run::begin(
                    server.clone(),
                    &variant.name(),
                    &pages,
                    m.load.runs.get() as usize,
                    m.load.back,
                );
            }
            let mode = Mode::Measure {
                scenario: m.scenario,
                begin_frames: m.begin_frames,
                out: m.out,
                load,
                page: m.page,
            };
            with_smoke_dir(|config, dir| report_run(mode, config, dir))
        }
        Command::Smoke(smoke) => {
            let mode = match smoke {
                Smoke::Frames { frames, url } => Mode::Frames {
                    frames: frames.get().into(),
                    url,
                },
                Smoke::Nav => Mode::Nav,
                Smoke::Tabs => Mode::Tabs,
                Smoke::Windows => Mode::Windows,
                Smoke::Split => Mode::Split,
                Smoke::Page => Mode::Page,
                Smoke::Parity => Mode::Parity,
                Smoke::Block => Mode::Block,
                Smoke::Settings => Mode::Settings,
                Smoke::Start { dir } => {
                    if let Some(dir) = dir {
                        shot::request_dir(dir);
                    }
                    Mode::Start
                }
                Smoke::Pages => Mode::Pages,
                Smoke::Autofill { probe } => Mode::Autofill { probe },
                Smoke::Media => Mode::Media,
                Smoke::Builtins => Mode::Builtins,
                Smoke::Video => Mode::Video,
                Smoke::Boost => Mode::Boost,
                Smoke::Ax => Mode::Ax,
                Smoke::Mcp => Mode::Mcp,
                Smoke::Overlay { dark, dir } => {
                    if let Some(dir) = dir {
                        shot::request_dir(dir);
                    }
                    Mode::Overlay { dark }
                }
                Smoke::Gallery { dir } => {
                    shot::request_dir(dir);
                    Mode::Gallery
                }
                // handled above
                Smoke::Boot
                | Smoke::Update
                | Smoke::Quit
                | Smoke::Codecs { .. }
                | Smoke::Extensions { .. }
                | Smoke::Session
                | Smoke::SessionPhase { .. }
                | Smoke::Passwords
                | Smoke::PasswordsPhase { .. }
                | Smoke::Data
                | Smoke::DataPhase { .. } => {
                    unreachable!("{smoke:?} has its own arm")
                }
            };
            with_smoke_dir(|config, dir| report_run(mode, config, dir))
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
            measure::render_began(now);
            if let Ok(mut glass) = glass.try_borrow_mut() {
                let at = Instant::now();
                if glass.frame(window, now, items) {
                    measure::stage(measure::Metric::Scene, at);
                }
            }
            // The thinking orbs that move (orbs.rs): nothing unless one is on screen and moving.
            if orbs::frame(id, window, now) {
                window.window().request_redraw();
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
pub fn drain(engine: &mut Engine) {
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
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let profile = Profile::resolve(profile).ok_or(EngineError::CacheDir)?;
    // One browser per profile, decided before anything opens the store or starts CEF.
    std::fs::create_dir_all(&profile.dir)?;
    // sockets and manifests that dead browsers left behind
    instance::sweep(&instance::runtime_dir());
    let socket = instance::socket_path(&profile.dir, &instance::runtime_dir());
    let message = url
        .clone()
        .map_or(instance::Message::Focus, instance::Message::Open);
    let listener = match instance::claim(&socket, &message) {
        Ok(instance::Claim::Forwarded) => return Ok(ExitCode::SUCCESS),
        Ok(instance::Claim::Primary(listener)) => Some(listener),
        Err(e) => {
            eprintln!("wubble: running without the single-instance socket: {e}");
            None
        }
    };
    // The one instance, before anything starts: a staged update applies or a failing version
    // rolls back, by starting the other version in this process's place.
    updater::on_launch();
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
    let primary = matches!(
        mode,
        Mode::Browse {
            listener: Some(_),
            ..
        }
    );
    let code = report_run(mode, config, &profile.dir);
    // a clean exit leaves no socket of ours behind (the MCP one went with hosting)
    if primary {
        let _ = instance::remove_stale(&socket);
    }
    updater::relaunch_if_requested();
    Ok(code)
}

/// `wubble mcp`: stdio relayed to the MCP socket of the browser on `profile`, or of the one
/// hosting browser the manifests name (`mcp.rs`); `--list` prints them. It opens nothing else:
/// no store, GPU or CEF.
fn mcp_relay(profile: Option<PathBuf>, list: bool) -> Result<(), Box<dyn std::error::Error>> {
    let live = mcp::hosting_browsers();
    if list {
        for m in &live {
            println!("{}\t{}\t{}\tpid {}", m.profile, m.socket, m.version, m.pid);
        }
        if live.is_empty() {
            eprintln!("wubble mcp: no browser hosts MCP (Settings > Agents (MCP))");
        }
        return Ok(());
    }
    let socket = match profile {
        Some(dir) => {
            let profile = Profile::resolve(Some(dir)).ok_or(EngineError::CacheDir)?;
            instance::path_of(
                instance::Socket::Mcp,
                &profile.dir,
                &instance::runtime_dir(),
            )
        }
        None => {
            use wubble_core::agent::manifest::{Pick, pick, pick_error};
            match pick(live) {
                Pick::One(m) => PathBuf::from(m.socket),
                other => {
                    let why = pick_error(&other).unwrap_or_default();
                    eprintln!("wubble mcp: {why}");
                    return Ok(mcp::answer_offline_with(&why)?);
                }
            }
        }
    };
    Ok(mcp::relay(&socket)?)
}

/// Runs `f` with a fresh per-process directory, removed afterwards: CEF's
/// cache inside it, so concurrent smokes (lanes, CI jobs, a smoke next to an
/// open browser) never share CEF's singleton lock, and it doubles as the
/// smoke's throwaway profile.
fn with_smoke_dir<T>(f: impl FnOnce(Result<EngineConfig, EngineError>, &Path) -> T) -> T {
    let dir = std::env::temp_dir().join(format!("wubble-smoke-{}", std::process::id()));
    let profile = Profile { dir: dir.clone() };
    let config = EngineConfig::from_current_exe().map(|config| EngineConfig {
        cache_dir: profile.cef_cache(),
        profile: ProfileKind::Throwaway,
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
        mark_smoke_profile(&profile)?;
        let no_sandbox = app::no_sandbox_requested(std::env::args_os());
        for phase in ["session-phase=1", "session-phase=2"] {
            let status = std::process::Command::new(&exe)
                .args(session_phase_args(phase, &profile, no_sandbox))
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

/// `--smoke passwords`: the file its phases keep the vault key in, on the temporary profile.
const SMOKE_KEYRING: &str = "smoke-keyring";

/// `--smoke passwords`: its four processes on one temporary profile (`passwords_smoke.rs`).
fn smoke_passwords() -> Result<(), Box<dyn std::error::Error>> {
    use passwords_smoke::VaultPhase;
    let exe = std::env::current_exe()?;
    let profile = std::env::temp_dir().join(format!("wubble-passwords-{}", std::process::id()));
    let result = (|| {
        mark_smoke_profile(&profile)?;
        let no_sandbox = app::no_sandbox_requested(std::env::args_os());
        let key_file = profile.join(SMOKE_KEYRING);
        for phase in [
            VaultPhase::Write,
            VaultPhase::Read,
            VaultPhase::Broken,
            VaultPhase::Lost,
        ] {
            // the key gone from the keyring, the logins still in the database
            if phase == VaultPhase::Lost {
                std::fs::remove_file(&key_file)?;
            }
            let status = std::process::Command::new(&exe)
                .args(session_phase_args(phase.arg(), &profile, no_sandbox))
                .status()?;
            if !status.success() {
                return Err(format!("{} exited with {status}", phase.arg()).into());
            }
        }
        if key_file.exists() {
            return Err("a new key was made over logins sealed with the lost one".into());
        }
        Ok(())
    })();
    remove_cache(&profile);
    result
}

/// The file a parent smoke writes into its temporary profile; a phase run refuses any
/// profile without it, so a phase pointed at a real profile cannot overwrite its session
/// or replace its vault's key.
const SMOKE_PROFILE_MARKER: &str = ".wubble-smoke-profile";

/// Creates `dir` and marks it as a smoke's throwaway profile.
fn mark_smoke_profile(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(SMOKE_PROFILE_MARKER), b"")
}

/// Whether a phase run may use `dir`: only a profile its parent smoke marked.
fn smoke_profile(dir: &Path) -> Result<(), String> {
    if dir.join(SMOKE_PROFILE_MARKER).is_file() {
        Ok(())
    } else {
        Err(format!(
            "{} is not a smoke profile (no {SMOKE_PROFILE_MARKER}); phases run only under --smoke session or --smoke passwords",
            dir.display()
        ))
    }
}

/// One `--smoke session` phase's command line; the parent's `--no-sandbox` goes with it,
/// so a debugging run is unsandboxed in both phases too.
fn session_phase_args(phase: &str, profile: &Path, no_sandbox: bool) -> Vec<std::ffi::OsString> {
    let mut args = vec![
        "--smoke".into(),
        phase.into(),
        "--profile".into(),
        profile.into(),
    ];
    args.extend(no_sandbox.then(|| app::NO_SANDBOX_ARG.into()));
    args
}

/// `--smoke data`: the script run twice on throwaway profiles, once ending in a quit and once in
/// a SIGKILL; after each the database is compared with what the pages showed. The opener a
/// download's Open and Show in folder would start is a stand-in that only logs.
fn smoke_data() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;
    let exe = std::env::current_exe()?;
    let no_sandbox = app::no_sandbox_requested(std::env::args_os());
    for ending in [data_smoke::Ending::Quit, data_smoke::Ending::Kill] {
        let word = if ending == data_smoke::Ending::Quit {
            "quit"
        } else {
            "kill"
        };
        let profile =
            std::env::temp_dir().join(format!("wubble-data-{word}-{}", std::process::id()));
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            // its run (`data-phase`) refuses a profile not marked as a smoke's
            mark_smoke_profile(&profile)?;
            let bin = profile.join("bin");
            std::fs::create_dir_all(&bin)?;
            let stub = bin.join("xdg-open");
            std::fs::write(
                &stub,
                format!(
                    "#!/bin/sh\necho \"$@\" >> '{}'\n",
                    profile.join("opener.log").display()
                ),
            )?;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))?;
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut dirs = vec![bin.clone()];
            dirs.extend(std::env::split_paths(&path));
            let mut args: Vec<std::ffi::OsString> = vec![
                "--smoke".into(),
                format!("data-phase={word}").into(),
                "--profile".into(),
                profile.clone().into(),
            ];
            args.extend(no_sandbox.then(|| app::NO_SANDBOX_ARG.into()));
            let status = std::process::Command::new(&exe)
                .args(args)
                .env("PATH", std::env::join_paths(dirs)?)
                // no session bus: Show in folder would otherwise reach the person's file manager
                .env("DBUS_SESSION_BUS_ADDRESS", data_smoke::NO_BUS)
                .status()?;
            match ending {
                data_smoke::Ending::Quit if !status.success() => {
                    return Err(format!("the {word} run exited with {status}").into());
                }
                data_smoke::Ending::Kill if status.signal() != Some(9) => {
                    return Err(format!("the {word} run was not killed: {status}").into());
                }
                _ => {}
            }
            let shown = std::fs::read_to_string(profile.join(data_smoke::SUMMARY))?;
            let db = Profile {
                dir: profile.clone(),
            }
            .db_path();
            data_smoke::compare(&shown, &db, ending).map_err(Into::into)
        })();
        remove_cache(&profile);
        result?;
        println!("smoke: data: {word} run ok");
    }
    Ok(())
}

/// The logins the smokes' key menus must offer: `--smoke autofill`'s fixtures' (origin,
/// username, password), or the loopback page's one for the smokes that share its server.
fn seed_autofill_logins(store: &StoreClient, logins: Vec<(String, String, String)>) {
    store.vault(move |vault| {
        for (origin, username, password) in logins {
            let Ok(origin) = wubble_vault::Origin::parse(&origin) else {
                continue;
            };
            let login = wubble_vault::NewLogin {
                origin,
                form_action_origin: None,
                username,
                password: zeroize::Zeroizing::new(password),
            };
            if let Err(e) = vault.save(login, 0) {
                eprintln!("smoke: autofill: a login was not seeded: {e}");
            }
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
    /// `--smoke parity` passed every item but the known gaps, which failed as expected.
    parity_passed: Option<Vec<&'static str>>,
    /// `--smoke windows` reached its end.
    windows_passed: bool,
    /// `--smoke settings` reached its end.
    settings_passed: bool,
    /// `--smoke start` reached its end.
    start_passed: bool,
    pages_passed: bool,
    /// `--smoke passwords` (this process's phase) reached its end.
    passwords_passed: bool,
    /// `--smoke data`'s run reached its end.
    data_passed: bool,
    gallery_passed: bool,
    /// `--smoke autofill` reached its end.
    autofill_passed: bool,
    /// `--smoke media` reached its end.
    media_passed: bool,
    /// `--smoke builtins` passed, with what it saw last.
    builtins_passed: Option<String>,
    /// `--smoke video` or `--smoke boost` passed.
    video_passed: Option<String>,
    /// `--smoke ax` passed, with the page's item count.
    ax_passed: Option<usize>,
    /// `--smoke mcp` passed, with its step count.
    mcp_passed: Option<usize>,
    /// `--smoke overlay` reached its end.
    overlay_passed: bool,
    /// `--measure`'s report was printed (and its CSV written, if asked) before CEF shut down,
    /// so a teardown that hangs loses nothing; `Err` when the CSV could not be written.
    measured: Option<Result<(), String>>,
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
    /// `--smoke parity`.
    Parity,
    /// `--smoke block`.
    Block,
    /// `--smoke settings`.
    Settings,
    /// `--smoke start`.
    Start,
    /// `--smoke pages`.
    Pages,
    /// `--smoke gallery` (on the pages smoke's seeded profile).
    Gallery,
    /// `--smoke autofill`, or with `probe` its hand-run look at a real page.
    Autofill { probe: Option<String> },
    /// `--smoke media`.
    Media,
    /// `--smoke builtins`.
    Builtins,
    /// `--smoke video`.
    Video,
    /// `--smoke boost`.
    Boost,
    /// `--smoke ax`.
    Ax,
    /// `--smoke mcp`.
    Mcp,
    /// `--smoke overlay` (`dark`: the dark page).
    Overlay { dark: bool },
    /// One phase of `--smoke session`.
    Session(Phase),
    /// One process of `--smoke passwords`.
    Passwords(passwords_smoke::VaultPhase),
    /// One run of `--smoke data`, ending as given.
    Data(data_smoke::Ending),
    /// `--measure`: the scenario's bundled page, measured until it ends;
    /// CEF paced by external begin frames if `begin_frames`; the samples
    /// also go to `out` as CSV.
    Measure {
        scenario: measure::Scenario,
        begin_frames: bool,
        out: Option<PathBuf>,
        /// `--measure load`: the pages' server and what the engine hooks requests with.
        load: Option<(Arc<load_server::LoadServer>, load_run::Variant)>,
        /// `--measure-page`: the page instead of the scenario's own.
        page: Option<String>,
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
        orbs::install(id, &self.gpu, &window);
        host::gallery_orbs::measure_one(id, &window);
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
        // Little, Popup and Pip windows show their page alone.
        let ui = match kind {
            WindowKind::Main => ui,
            WindowKind::Little | WindowKind::Popup | WindowKind::Pip => persist::UiState {
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
            std::array::from_fn(|_| gpu.presenter()),
            glass,
            panels,
        ))
    }
}

/// Hands a pump request to the host.
pub fn deliver(wake: Wake) {
    measure::count(measure::Count::Pump);
    let at = Instant::now();
    with_host(|host| host.pump(wake));
    measure::pumped(at);
}

/// CEF's pump requests, from any thread, onto the UI thread ([`ui_loop`]). After the loop
/// has ended they find no host and are dropped: the shutdown drain pumps by itself.
fn pump_scheduler() -> PumpScheduler {
    use ui_loop::Post as _;
    PumpScheduler::new(|delay_ms| {
        ui_loop::Current::post(move || deliver(Wake::Schedule(delay_ms)));
    })
}

pub fn quit() {
    if shot::defer(quit) {
        return;
    }
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
    window
        .global::<wubble_ui::Tips>()
        .on_hover(move |id, x, top, bottom, on| {
            with_host_in(win, |host| host.shell_tip(&id, x, (top, bottom), on))
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
        with_host_in(win, |host| {
            result = if host.parity_owns_input(event) {
                EventResult::PreventDefault
            } else {
                host.window_event(event)
            }
        });
        result
    });
}

/// Hands the shell's tab, Space and command-bar callbacks to the host,
/// each one a model command or a bar update.
fn connect_shell(window: &AppWindow, win: WindowId) {
    use wubble_core::model::Command as Cmd;
    let command = move |c: Cmd| with_host_in(win, |host| host.dispatch(c));
    // A tab row picks its tab; a folder header folds the folder away or shows it.
    window.on_tab_activated(move |id| {
        if let Some(t) = shell::tab_id(id) {
            command(Cmd::ActivateTab(t));
        } else if let Some(folder) = shell::folder_of_row(id) {
            with_host_in(win, |host| host.toggle_folder(folder));
        }
    });
    window.on_tab_closed(move |id| {
        if let Some(t) = shell::tab_id(id) {
            command(Cmd::CloseTab(t));
        }
    });
    // The peek's card.
    window.on_glance_closed(move || command(Cmd::CloseGlance(win)));
    window.on_glance_to_tab(move || {
        command(Cmd::PromoteGlance {
            window: win,
            to: wubble_core::model::GlanceTo::Tab,
        })
    });
    window.on_glance_to_split(move || {
        command(Cmd::PromoteGlance {
            window: win,
            to: wubble_core::model::GlanceTo::Split,
        })
    });
    // A row dragged to slot `to` of the flat tab list.
    window.on_tab_moved(move |id, to| {
        if let (Some(tab), Ok(to)) = (shell::tab_id(id), usize::try_from(to)) {
            with_host_in(win, |host| host.move_tab(tab, to));
        }
    });
    let swipe = RefCell::new(host::Swipe::default());
    window.on_space_swipe(move |dx, dy| {
        with_host_in(win, |host| {
            host.space_swipe(&mut swipe.borrow_mut(), dx, dy)
        });
    });
    window.on_space_switched(move |id| {
        if let Some(s) = shell::space_id(id) {
            command(Cmd::SwitchSpace(s));
        }
    });
    window.on_space_cleared(move |id| {
        if let Some(s) = shell::space_id(id) {
            with_host_in(win, |host| host.space_cleared(s));
        }
    });
    // The start page (host/start.rs): its stage's pointer and the front's intents.
    {
        use crate::host::start::StagePointer;
        let bridge = window.global::<wubble_ui::StartBridge>();
        let pointer = move |p| with_host_in(win, |host| host.start_pointer(p));
        bridge.on_stage_down(move |x, y| pointer(StagePointer::Down(x.into(), y.into())));
        bridge.on_stage_move(move |x, y| pointer(StagePointer::Move(x.into(), y.into())));
        bridge.on_stage_up(move || pointer(StagePointer::Up));
        bridge.on_stage_leave(move || pointer(StagePointer::Leave));
        // a tile: its site in the start tab
        bridge.on_site_activated(move |url| with_host_in(win, |host| host.page_opened(&url)));
    }
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
    window.on_tab_hovered(move |id, on, y| with_host_in(win, |host| host.tab_hovered(id, on, y)));
    window.on_tab_context(move |id, x, y| with_host_in(win, |host| host.tab_context(id, x, y)));
    window.on_sidebar_context(move |x, y| with_host_in(win, |host| host.sidebar_context(x, y)));
    window.on_tab_track(move |id, kind, x, y| {
        with_host_in(win, |host| host.tab_track(id, kind, x, y))
    });
    window.on_command_dismissed(move || with_host_in(win, |host| host.bar_closed()));
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
    window.on_extensions_requested(move || with_host_in(win, Host::extensions_requested));
    window.on_extension_pressed(move |id| with_host_in(win, |host| host.extension_pressed(&id)));
    window.on_extensions_manage(move || with_host_in(win, Host::extensions_manage));
    window.on_extensions_dismissed(move || with_host_in(win, Host::extensions_dismissed));
    window.on_extension_prompt_answered(move |accept| {
        with_host_in(win, |host| host.extension_prompt_answered(accept))
    });
    window.on_extension_install_file(move || with_host_in(win, Host::extension_install_file));
    window.on_extension_install_folder(move || with_host_in(win, Host::extension_install_folder));
    window.on_extension_toggled(move |id, on| {
        with_host_in(win, |host| host.extension_toggled(&id, on))
    });
    window.on_extension_removed(move |id| with_host_in(win, |host| host.extension_removed(&id)));
    window.on_extension_options(move |id| with_host_in(win, |host| host.extension_options(&id)));
    window.on_agents_toggled(move |on| with_host_in(win, |host| host.agents_toggled(on)));
    window.on_agents_copy(move |text| with_host_in(win, |host| host.agents_copy(&text)));
    window.on_agent_app_added(move |id| with_host_in(win, |host| host.agent_app_added(&id)));
    window.on_agent_app_removed(move |id| with_host_in(win, |host| host.agent_app_removed(&id)));
    window.on_find_edited(move |q| {
        with_host_in(win, |host| host.find_event(FindEvent::Query(q.into())))
    });
    window.on_task_action(move |tab, pid, action| {
        with_host_in(win, |host| host.task_action(tab, pid, action))
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
    window.on_spaces_menu_requested(move || with_host_in(win, |host| host.spaces_menu_requested()));
    window.on_settings_query_changed(move |q| with_host_in(win, |host| host.settings_query(&q)));
    window.on_setting_changed(move |key, value| {
        with_host_in(win, |host| {
            host.write_setting(&key, &value);
        })
    });
    // the Sidebar row follows the shell's own mode; later, outside whatever changed it
    window.on_sidebar_mode_seen(move || {
        let _ = slint::invoke_from_event_loop(move || {
            with_host_in(win, |host| host.settings_sidebar_seen())
        });
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
    window.on_filter_list_toggled(move |id, on| {
        with_host_in(win, |host| host.filter_list_toggled(&id, on))
    });
    window.on_filter_lists_update(move || with_host_in(win, |host| host.filter_lists_update()));
    window.on_updates_toggled(move |on| with_host_in(win, |host| host.updates_toggled(on)));
    window.on_updates_check(move || with_host_in(win, Host::updates_check));
    window.on_updates_restart(move || with_host_in(win, Host::updates_restart));
}

/// The native pages' intents (history, bookmarks, downloads): each becomes one host call.
fn connect_pages(window: &AppWindow, win: WindowId) {
    window.on_history_query_changed(move |q| with_host_in(win, |host| host.history_query(&q)));
    window.on_history_deleted(move |url| with_host_in(win, |host| host.history_deleted(&url)));
    window.on_history_more(move || with_host_in(win, Host::history_more));
    window.on_history_cleared(move |range| with_host_in(win, |host| host.history_cleared(range)));
    window.on_archive_query_changed(move |q| with_host_in(win, |host| host.archive_query(&q)));
    window.on_archive_deleted(move |id| with_host_in(win, |host| host.archive_deleted(id)));
    window.on_archive_restored(move |id| with_host_in(win, |host| host.archive_restored(id)));
    window.on_archive_more(move || with_host_in(win, Host::archive_more));
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
    // The built-ins' own UI: the reader's glyph and controls, the screenshot overlay.
    window.on_reader_toggled(move || with_host_in(win, Host::reader_toggled));
    window.on_reader_setting(move |key, value| {
        with_host_in(win, |host| host.reader_setting(&key, value))
    });
    window.on_shot_picked(move |ax, ay, bx, by| {
        with_host_in(win, |host| host.shot_picked((ax, ay), (bx, by)))
    });
    window.on_shot_capture(move |full| with_host_in(win, |host| host.shot_overlay_capture(full)));
    window.on_shot_cancelled(move || with_host_in(win, Host::shot_cancelled));
    window.on_bookmark_deleted(move |id| with_host_in(win, |host| host.bookmark_deleted(id)));
    window.on_bookmark_folder_requested(move || with_host_in(win, Host::bookmark_folder_requested));
    window.on_print_sheet_edited(
        move |d, pages, landscape, paper, margins, scale, back, copies| {
            with_host_in(win, |host| {
                host.print_edited(d, &pages, landscape, paper, margins, scale, back, copies)
            })
        },
    );
    window.on_agent_sheet_answered(move |allow, headless| {
        with_host_in(win, |host| host.agent_sheet_answered(allow, headless))
    });
    window.on_agent_sheet_removed(move |site, key| {
        with_host_in(win, |host| host.agent_sheet_removed(site, &key))
    });
    window.on_agent_sheet_site_submitted(move |text| {
        with_host_in(win, |host| host.agent_sheet_site_submitted(&text))
    });
    window.on_agent_sheet_share_toggled(move |on| {
        with_host_in(win, |host| host.agent_sheet_share(on))
    });
    window.on_agent_sheet_share_from_picked(move |i| {
        with_host_in(win, |host| host.agent_sheet_share_from(i))
    });
    window.on_agent_sheet_bound(move |name, i| {
        with_host_in(win, |host| host.agent_sheet_bound(&name, i))
    });
    window.on_agent_sheet_unlock_vault(move || {
        with_host_in(win, |host| host.agent_sheet_unlock_vault())
    });
    window.on_print_sheet_answered(move |accept| {
        with_host_in(win, |host| host.print_answered(accept))
    });
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
    window.on_address_media_requested(move |x, y| {
        with_host_in(win, |host| host.media_menu_requested(x, y))
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
    window.on_login_edited(move |id, site, user, password| {
        with_host_in(win, |host| host.login_edited(&id, &site, &user, &password))
    });
    window
        .on_passwords_import_requested(move || with_host_in(win, Host::passwords_import_requested));
    window.on_passwords_start_over(move || with_host_in(win, Host::passwords_start_over));
    // The video controls, the toast's button and the Boost editor.
    window.on_video_command(move |name, value| {
        with_host_in(win, |host| host.video_command(&name, value))
    });
    window.on_video_save(move |x, y| with_host_in(win, |host| host.video_save(x, y)));
    window.on_video_hovered(move |over| with_host_in(win, |host| host.video_hovered(over)));
    window.on_pip_command(move |name, value| {
        with_host_in(win, |host| host.pip_command(&name, value))
    });
    window.on_ax_action(move |index, name, value| {
        with_host_in(win, |host| host.ax_action(index, &name, &value))
    });
    window.on_toast_acted(move || with_host_in(win, Host::toast_acted));
    window.on_toast_closed(move || with_host_in(win, Host::toast_closed));
    window
        .on_boost_text(move |field, text| with_host_in(win, |host| host.boost_text(&field, &text)));
    window.on_boost_number(move |field, value| {
        with_host_in(win, |host| host.boost_number(&field, value))
    });
    window.on_boost_tint(move |colour| with_host_in(win, |host| host.boost_tint(colour)));
    window.on_boost_act(move |name| with_host_in(win, |host| host.boost_act(&name)));
}

/// Runs `mode`'s window and gives its exit code. The verdict is printed before CEF is torn down
/// (`verdict.rs`).
fn report_run(
    mut mode: Mode,
    config: Result<EngineConfig, EngineError>,
    profile: &Path,
) -> ExitCode {
    match run(&mut mode, config, profile) {
        Ok(code) => code,
        Err(e) => Verdict::of_error(&mode, e).print(),
    }
}

/// [`report_run`]'s window: `Err` only when it could not start or lost its host.
fn run(
    mode: &mut Mode,
    config: Result<EngineConfig, EngineError>,
    profile: &Path,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let listener = match mode {
        Mode::Browse { listener, .. } => listener.take(),
        _ => None,
    };
    // `--smoke autofill`'s pages, and `--smoke block`'s (it needs an http page), on a loopback port.
    let memory_run = matches!(
        mode,
        Mode::Measure {
            scenario: measure::Scenario::Memory,
            ..
        }
    );
    let autofill_port = (matches!(
        mode,
        Mode::Autofill { .. } | Mode::Block | Mode::Parity | Mode::Mcp
    ) || memory_run)
        .then(autofill_smoke::serve)
        .transpose()?;
    // `--smoke autofill`'s fixtures (or its probe), seeded below.
    let mut autofill_plan = match &mode {
        Mode::Autofill { probe: Some(url) } => Some(autofill_smoke::probe(url)),
        Mode::Autofill { probe: None } => autofill_port.map(autofill_smoke::corpus),
        _ => None,
    };
    // `--smoke data`'s pages, cookies and held downloads, on a loopback port.
    let data_server = matches!(mode, Mode::Data(_))
        .then(data_smoke::serve)
        .transpose()?;
    // `--smoke media`'s fixtures, on a loopback port.
    let media_port = matches!(mode, Mode::Media)
        .then(media_smoke::serve)
        .transpose()?;
    // `--smoke video`'s and `--smoke boost`'s pages and fake endpoint, on a loopback port.
    let video_server = matches!(mode, Mode::Video | Mode::Boost)
        .then(video_smoke::serve)
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
        Mode::Pages | Mode::Gallery => {
            let keyring = wubble_vault::FakeKeyRing::default();
            pages_smoke::seed(&db, &keyring)?;
            Box::new(keyring)
        }
        // a key that outlives the process, in a file of the throwaway profile; or, only on a
        // private D-Bus session with a throwaway keyring (`WUBBLE_SMOKE_KEYRING=os`), the OS one
        Mode::Passwords(phase) => {
            let broken = *phase == passwords_smoke::VaultPhase::Broken;
            if !broken && std::env::var("WUBBLE_SMOKE_KEYRING").as_deref() == Ok("os") {
                Box::new(wubble_vault::OsKeyRing)
            } else {
                Box::new(wubble_vault::FileKeyRing::new(
                    profile.join(SMOKE_KEYRING),
                    broken,
                ))
            }
        }
        _ => Box::new(wubble_vault::FakeKeyRing::default()),
    };
    // The database loads, and the GPU device is made (about 150 ms on the Linux box, as long
    // as CEF's start), while CEF starts.
    let (store, loaded) = StoreClient::spawn(db, keyring);
    if let Some(port) = autofill_port.filter(|_| !matches!(mode, Mode::Parity)) {
        let logins = match &autofill_plan {
            Some(plan) => plan.iter().flat_map(|f| f.logins.clone()).collect(),
            None => vec![
                (
                    format!("http://127.0.0.1:{port}"),
                    autofill_smoke::SEEDED_USER.into(),
                    autofill_smoke::SEEDED_PASSWORD.into(),
                ),
                // `--smoke mcp`'s login the sheet binds a session's names to
                (
                    format!("http://{}:{port}", mcp_smoke::SIGNIN_HOST),
                    mcp_smoke::SIGNIN_USER.into(),
                    mcp_smoke::SIGNIN_PASSWORD.into(),
                ),
            ],
        };
        seed_autofill_logins(&store, logins);
    }
    let gpu_init = std::thread::Builder::new()
        .name("wubble-gpu-init".into())
        .spawn(|| GpuContext::new().map_err(|e| e.to_string()))?;
    wubble_ui::window::no_os_window_restoration();
    wubble_ui::window::no_automatic_tabbing();

    let bundled = || page_url(SMOKE_PAGE);
    let none = Checks {
        quit_after: None,
        check_pixels: false,
        nav_smoke: None,
        tabs_smoke: None,
        session_smoke: None,
        page_smoke: None,
        parity: None,
        block_smoke: None,
        split_smoke: None,
        settings_smoke: None,
        start_smoke: None,
        pages_smoke: None,
        passwords_smoke: None,
        data_smoke: None,
        gallery_smoke: None,
        autofill_smoke: None,
        windows_smoke: None,
        media_smoke: None,
        builtin_smoke: None,
        video_smoke: None,
        ax_smoke: None,
        mcp_smoke: None,
        overlay_smoke: None,
        sponsor_endpoint: None,
        begin_frames: false,
    };
    let smoke_page = matches!(mode, Mode::Page);
    let smoke_block = matches!(mode, Mode::Block);
    let measuring = matches!(mode, Mode::Measure { .. });
    let (host_timers, measure_tick) = match &mode {
        Mode::Browse { .. } => (true, measure::TICK),
        Mode::Measure { scenario, .. } => (scenario.host_timers(), scenario.tick_every()),
        _ => (false, measure::TICK),
    };
    let measure_out = match &mode {
        Mode::Measure { out, .. } => out.clone(),
        _ => None,
    };
    // Smokes download into their throwaway profile.
    let downloads = match &mode {
        Mode::Browse { downloads, .. } => downloads.clone(),
        _ => Some(profile.join("downloads")),
    };
    // `url` is opened in a new tab; without one a fresh profile gets the
    // bundled page and a restored session opens nothing new.
    let (url, checks) = match &*mode {
        Mode::Browse { url, .. } => (url.clone(), none),
        Mode::Frames { frames, url } => {
            // Only the bundled page has known colours to check against.
            let check_pixels = url.is_none();
            (
                Some(url.clone().unwrap_or_else(bundled)),
                Checks {
                    quit_after: Some(*frames),
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
        Mode::Parity => (
            autofill_port.map(parity::home),
            Checks {
                parity: autofill_port.map(|port| host::ParityRun::new(parity::home(port))),
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
        Mode::Start => (
            Some(bundled()),
            Checks {
                start_smoke: Some(start_smoke::StartSmoke::default()),
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
        Mode::Gallery => (
            Some(bundled()),
            Checks {
                gallery_smoke: Some(gallery_smoke::Stage::Start),
                ..none
            },
        ),
        Mode::Block => (
            autofill_port.map(|port| format!("http://127.0.0.1:{port}/block")),
            Checks {
                block_smoke: Some(block_smoke::BlockSmoke::default()),
                ..none
            },
        ),
        Mode::Autofill { .. } => {
            let smoke =
                autofill_smoke::AutofillSmoke::new(autofill_plan.take().unwrap_or_default());
            (
                smoke.start_url().map(str::to_owned),
                Checks {
                    autofill_smoke: Some(smoke),
                    ..none
                },
            )
        }
        Mode::Media => (
            media_port.map(|port| format!("http://127.0.0.1:{port}/")),
            Checks {
                media_smoke: Some(media_smoke::MediaSmoke::default()),
                ..none
            },
        ),
        Mode::Builtins => {
            // A file:// PDF: Chromium's viewer, as for a downloaded one.
            // The store thread makes the profile directory too, but not before this runs.
            std::fs::create_dir_all(profile)?;
            let pdf = profile.join("doc.pdf");
            std::fs::write(&pdf, DOC_PDF)?;
            (
                Some(page_url(ARTICLE_PAGE)),
                Checks {
                    builtin_smoke: Some(builtin_smoke::BuiltinSmoke::new(
                        page_url(TALL_PAGE),
                        wubble_core::dnd::file_uri(&pdf),
                    )),
                    ..none
                },
            )
        }
        Mode::Ax => (
            Some(page_url(ax_smoke::PAGE)),
            Checks {
                ax_smoke: Some(ax_smoke::AxSmoke::default()),
                ..none
            },
        ),
        // the owner signed in to the sites a session shares (`mcp_smoke::sign_in`)
        Mode::Mcp => (
            autofill_port
                .map(mcp_smoke::sign_in)
                .or_else(|| Some(bundled())),
            Checks {
                mcp_smoke: Some(Default::default()),
                ..none
            },
        ),
        Mode::Overlay { dark } => (
            Some(page_url(OVERLAY_PAGE) + if *dark { "#dark" } else { "" }),
            Checks {
                overlay_smoke: Some(Default::default()),
                ..none
            },
        ),
        Mode::Video | Mode::Boost => {
            let (port, requests) = video_server.clone().ok_or("no smoke server")?;
            let base = format!("http://127.0.0.1:{port}");
            let (page, smoke) = if matches!(mode, Mode::Video) {
                // its pointer finds the strip's buttons by their element ids
                if !wubble_ui::ELEMENT_IDS {
                    return Err("--smoke video needs element ids: a debug build, \
                        or a release one built with SLINT_EMIT_DEBUG_INFO=1"
                        .into());
                }
                (
                    "video.html",
                    video_smoke::VideoSmoke::video(base.clone(), requests),
                )
            } else {
                (
                    "boost.html",
                    video_smoke::VideoSmoke::boost(base.clone(), requests),
                )
            };
            (
                Some(format!("{base}/{page}")),
                Checks {
                    video_smoke: Some(smoke),
                    sponsor_endpoint: Some(base),
                    ..none
                },
            )
        }
        Mode::Session(phase) => (
            None,
            Checks {
                session_smoke: Some(SessionSmoke::new(*phase)),
                ..none
            },
        ),
        Mode::Passwords(phase) => (
            None,
            Checks {
                passwords_smoke: Some(passwords_smoke::PasswordsSmoke::new(*phase)),
                ..none
            },
        ),
        Mode::Data(ending) => (
            data_server.as_ref().map(|s| s.url("/page")),
            Checks {
                data_smoke: data_server.map(|server| host::DataSmoke::new(server, *ending)),
                ..none
            },
        ),
        Mode::Measure {
            scenario,
            begin_frames,
            page,
            ..
        } => (
            // `--measure load` navigates from here to its own pages; the memory run starts on
            // an empty window and opens its tabs itself.
            (*scenario != measure::Scenario::Memory)
                .then(|| page.clone().unwrap_or_else(|| page_url(scenario.page()))),
            Checks {
                begin_frames: *begin_frames,
                ..none
            },
        ),
    };
    // A database that cannot be opened or loaded is left alone: the
    // browser runs without persistence (the store thread has ended). Read
    // before CEF starts: some settings are CEF start switches.
    let loaded = loaded
        .recv()
        .unwrap_or_else(|_| Err("the store thread ended".into()));
    measure::mark(measure::Stage::StoreLoaded);
    let loaded = loaded.unwrap_or_else(|e| {
        eprintln!("wubble: running without a saved session: {e}");
        Loaded::default()
    });
    // `--smoke page` docks DevTools: the endpoint it needs is off by default, so the smoke's
    // settings turn it on, on a free loopback port.
    let mut loaded = loaded;
    if smoke_page {
        loaded.settings.stored.insert(
            wubble_core::settings::DEVTOOLS_REMOTE_PORT.key.into(),
            page_smoke::free_port()?.to_string(),
        );
    }
    let blocking = blocking::Blocking::new();
    blocking.on_lists_changed(|| {
        let _ = slint::invoke_from_event_loop(|| with_host(Host::filter_lists_changed));
    });
    let media_observer = host::MediaObserver::new();
    // `--measure load --load-variant`: which hooks the engine is given, to price them.
    let (filter, observer, blocks) = match &mode {
        Mode::Measure {
            load: Some((_, variant)),
            ..
        } => {
            let (filter, observer) = variant.hooks();
            (filter, observer, variant.blocks())
        }
        _ => (true, true, true),
    };
    if !blocks {
        // Both: the blocker starts from the first, the host applies every stored setting (the
        // second) once it is up.
        loaded.settings.blocker.enabled = false;
        loaded.settings.stored.insert(
            wubble_core::settings::BLOCKER_ENABLED.key.into(),
            "0".into(),
        );
    }
    let installed = loaded.settings.extensions();
    // Before CEF starts, when no Chromium can still be reading a copy the list dropped.
    let extensions_root = Profile {
        dir: profile.into(),
    }
    .extensions_dir();
    extensions::sweep(&extensions_root, &installed);
    let mut engine = Engine::start(EngineConfig {
        pump_scheduler: Some(pump_scheduler()),
        content_filter: filter
            .then(|| blocking.clone() as Arc<dyn wubble_engine::filter::ContentFilter>),
        response_observer: observer
            .then(|| media_observer.clone() as Arc<dyn wubble_engine::filter::ResponseObserver>),
        features: loaded.settings.page_features(),
        clipboard_bridge: clipboard::bridge_needed(std::env::var_os("WAYLAND_DISPLAY").as_deref()),
        containers: loaded.model.containers.iter().map(|c| c.id).collect(),
        extensions: wubble_core::extensions::load_paths(
            wubble_engine::ext_api::available(),
            &installed,
        ),
        ..config?
    })?;
    measure::mark(measure::Stage::CefStarted);
    // CEF is up: a GPU or backend failure from here shuts it down before returning.
    let gpu = (|| -> Result<GpuContext, Box<dyn std::error::Error>> {
        let gpu = gpu_init
            .join()
            .map_err(|_| "the GPU device thread panicked")??;
        measure::mark(measure::Stage::Gpu);
        eprintln!("wubble: GPU {}", gpu.describe());
        slint_backend::select(
            gpu.slint_configuration(),
            "skia-wgpu",
            wubble_ui::window::attributes,
        )?;
        measure::mark(measure::Stage::SlintBackend);
        Ok(gpu)
    })();
    let gpu = match gpu {
        Ok(gpu) => gpu,
        Err(e) => {
            drain(&mut engine);
            return Err(e);
        }
    };
    // Slint's platform lives in a thread-local and would drop the last handle
    // to the shared device during thread-local teardown, after wgpu-core's
    // own thread-locals are gone (wgpu-core `snatch` lock tracing, debug
    // builds), which aborts the process. Keep one handle until exit instead.
    std::mem::forget(gpu.clone());
    // Pump requests CEF made while it started, before Slint's loop existed, were dropped
    // (`ui_loop`); the first pump runs once the loop does, and from there CEF schedules every
    // pump itself.
    slint::invoke_from_event_loop(|| deliver(Wake::Schedule(0)))?;
    if smoke_block {
        // Only the smoke's own list, through the same filter wiring as the browser's.
        blocking.use_list(block_smoke::BLOCK_LIST);
    } else {
        blocking.start(profile, &loaded.settings.blocker);
    }
    measure::mark(measure::Stage::BlockerStarted);
    let url = url.or_else(|| (loaded.model.tabs.is_empty() && !memory_run).then(bundled));
    if let Some(port) = autofill_port.filter(|_| memory_run) {
        // `{}` is each tab's own site (`*.localhost` is loopback in Chromium).
        memory::begin(format!("http://{{}}.localhost:{port}/memory"));
    }
    let factory = WindowFactory {
        gpu: gpu.clone(),
        glass_scene: loaded.settings.glass_scene(),
    };
    // The focused window first; the host opens the session's others.
    let focused = loaded.model.focused();
    let geometry = loaded.model.window(focused).and_then(|w| w.geometry);
    let first = factory.open(focused, WindowKind::Main, loaded.settings.ui, geometry)?;
    measure::mark(measure::Stage::WindowOpened);
    let window = first.handle();
    shot::attach(&window);
    // `--smoke mcp` presses the approval sheet's buttons: the window at the gallery's size, a
    // person's (a headless compositor opens it at its least size, which the sheet overflows).
    if matches!(mode, Mode::Mcp) {
        let (w, h) = shot::SIZE;
        window.window().set_size(slint::LogicalSize::new(w, h));
    }
    let mut host = Host::new(
        engine,
        first,
        factory,
        loaded,
        store,
        url,
        checks,
        downloads,
        blocking,
        media_observer,
    );
    host.set_extensions_root(extensions_root);
    HOST.set(Some(host));
    // The browser checks for an update as it opens (the setting and the policy permitting) and
    // counts the start as healthy after a while (`update::install`); smokes do neither.
    let healthy = slint::Timer::default();
    if matches!(mode, Mode::Browse { .. }) {
        with_host(Host::updates_opened);
        healthy.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(wubble_core::update::install::HEALTHY_AFTER_SECS),
            || with_host(Host::updates_healthy),
        );
    }
    // Agent sessions over MCP (`mcp.rs`): a private socket in the profile, hosted while the
    // setting is on (`host/hosting.rs`, switched live from Settings).
    let socket = instance::path_of(instance::Socket::Mcp, profile, &instance::runtime_dir());
    let default_profile = app::user_data_dir().is_some_and(|d| d == profile);
    with_host(|host| {
        host.set_mcp(host::McpPlace {
            socket: socket.clone(),
            manifest: mcp::manifest_path(profile),
            profile: profile.to_owned(),
            default_profile,
            exe: std::env::current_exe().unwrap_or_default(),
            home: match mode {
                Mode::Browse { .. } => std::env::home_dir(),
                _ => Some(profile.join("agent-apps-home")),
            },
        })
    });
    // `--smoke mcp`: hosting turned on through the setting, as Settings does, then the client
    if let Some(port) = autofill_port.filter(|_| matches!(mode, Mode::Mcp)) {
        with_host(|host| {
            if let Err(e) = host.mcp_smoke_on() {
                host.fail(format!("hosting on: {e}"));
            }
        });
        mcp_smoke::start(socket, format!("http://127.0.0.1:{port}/mcp"));
    }
    if let Some(listener) = listener {
        instance::listen(listener, |message| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.open_external(message))
            });
        });
    }
    // Smokes drive the lifecycle themselves, never by wall-clock time; `--measure idle` runs
    // the browser's timers.
    let lifecycle = slint::Timer::default();
    let audible = slint::Timer::default();
    let media = slint::Timer::default();
    let video_clock = slint::Timer::default();
    if host_timers {
        video_clock.start(slint::TimerMode::Repeated, VIDEO_TICK_EVERY, || {
            measure::count(measure::Count::VideoTick);
            with_host(Host::tick_video)
        });
        media.start(slint::TimerMode::Repeated, MEDIA_POLL_EVERY, || {
            measure::count(measure::Count::MediaPoll);
            with_host(Host::poll_media)
        });
        lifecycle.start(slint::TimerMode::Repeated, LIFECYCLE_TICK, || {
            measure::count(measure::Count::Lifecycle);
            with_host(|host| host.tick(Instant::now()))
        });
        audible.start(slint::TimerMode::Repeated, AUDIBLE_PROBE_EVERY, || {
            measure::count(measure::Count::AudibleProbe);
            with_host(Host::probe_audible)
        });
    }
    let measure_timer = slint::Timer::default();
    if measuring {
        let weak = window.as_weak();
        measure_timer.start(slint::TimerMode::Repeated, measure_tick, move || {
            if let Some(window) = weak.upgrade() {
                with_host(|host| measure::tick(host, &window));
            }
        });
    }

    window.show()?;
    measure::mark(measure::Stage::WindowShown);
    with_host(Host::open_session_windows);
    measure::mark(measure::Stage::SessionRestored);
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
        .flatten()
        .map(|(report, csv)| {
            println!("{report}");
            // A failed load run exits non-zero, so a script never counts its partial rows.
            let failed = load_run::take_failure();
            let written = match measure_out {
                Some(path) => measure::write_csv(&path, &csv)
                    .map(|()| println!("measure: samples written to {}", path.display()))
                    .map_err(|e| format!("cannot write {}: {e}", path.display())),
                None => Ok(()),
            };
            failed.map_or(written, Err)
        });
    drop(lifecycle);
    drop(audible);
    drop(media);
    drop(window);
    let mut host = HOST.take().ok_or("the host went missing")?;
    host.shutdown(ui);
    drop(host.timer);
    let outcome = Outcome {
        stats: host.stats.clone(),
        failure: host
            .failure
            .take()
            .map(Into::into)
            .or_else(|| result.err().map(Into::into)),
        nav_title: host.nav_title.take(),
        tabs_passed: host.tabs_passed,
        split_passed: host.split_passed,
        parity_passed: host.parity_passed,
        windows_passed: host.windows_passed,
        settings_passed: host.settings_passed,
        start_passed: host.start_passed,
        pages_passed: host.pages_passed,
        passwords_passed: host.passwords_passed,
        data_passed: host.data_passed,
        gallery_passed: host.gallery_passed,
        session_passed: host.session_passed,
        page_zoom: host.page_zoom,
        blocked_seen: host.blocked_seen,
        autofill_passed: host.autofill_passed,
        media_passed: host.media_passed,
        builtins_passed: host.builtins_passed.take(),
        video_passed: host.video_passed.take(),
        ax_passed: host.ax_passed,
        mcp_passed: host.mcp_passed,
        overlay_passed: host.overlay_passed,
        measured,
    };
    let code = Verdict::of(mode, &outcome).print();
    drain(&mut host.engine);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_phases_keep_the_parents_no_sandbox() {
        let profile = Path::new("/tmp/p");
        let sandboxed = session_phase_args("session-phase=1", profile, false);
        assert!(!app::no_sandbox_requested(&sandboxed));
        assert_eq!(sandboxed[..2], ["--smoke", "session-phase=1"]);
        let off = session_phase_args("session-phase=2", profile, true);
        assert!(app::no_sandbox_requested(&off));
        assert_eq!(off[3], "/tmp/p");
    }

    #[test]
    fn smoke_profile_accepts_a_marked_dir() {
        let dir = tempfile::tempdir().unwrap();
        mark_smoke_profile(dir.path()).unwrap();
        assert_eq!(smoke_profile(dir.path()), Ok(()));
    }

    #[test]
    fn smoke_profile_refuses_an_unmarked_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = smoke_profile(dir.path()).unwrap_err();
        assert!(err.contains(&dir.path().display().to_string()), "{err}");
    }
}
