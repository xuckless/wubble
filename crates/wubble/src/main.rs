mod cli;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use cli::{Command, Smoke};
use slint::ComponentHandle;
use wubble_engine::app::{self, Engine, EngineConfig, EngineError};
use wubble_ui::AppWindow;

/// How often the Slint loop pumps CEF. Interim: replaced by compositor-driven
/// ticks in the M0 "Frame pacing" item.
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// Pumps `--smoke boot` gives CEF before shutting down. A count, not a clock.
const SMOKE_TICKS: usize = 10;

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os()) {
        Ok(command) => command,
        Err(e) => e.exit(),
    };
    match command {
        Command::Run => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("wubble: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Smoke(Smoke::Boot) => match smoke_boot() {
            Ok(()) => {
                println!("smoke: cef boot ok {}", app::version());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("smoke: cef boot FAILED: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Pumps until every browser is closed and CEF has shut down. Bounded by the
/// lifecycle state, not by time: it relies on the engine reporting every
/// browser closed (see `Effect::CloseAllBrowsers` in wubble-engine).
fn drain(engine: &mut Engine) {
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
}

/// Boots with a fresh per-process cache so concurrent smokes (lanes, CI jobs,
/// a smoke next to an open browser) never share CEF's singleton lock.
fn smoke_boot() -> Result<(), EngineError> {
    let cache_dir = std::env::temp_dir().join(format!("wubble-smoke-{}", std::process::id()));
    let config = EngineConfig {
        cache_dir: cache_dir.clone(),
        ..EngineConfig::from_current_exe()?
    };
    let result = Engine::start(config).map(|mut engine| {
        for _ in 0..SMOKE_TICKS {
            engine.tick();
        }
        drain(&mut engine);
    });
    // CEF has shut down (or never started), so nothing holds the directory.
    if let Err(e) = std::fs::remove_dir_all(&cache_dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("smoke: could not remove {}: {e}", cache_dir.display());
    }
    result
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Rc::new(RefCell::new(Engine::start(
        EngineConfig::from_current_exe()?,
    )?));
    let window = AppWindow::new()?;

    let pump = slint::Timer::default();
    let ticked = Rc::clone(&engine);
    pump.start(slint::TimerMode::Repeated, PUMP_INTERVAL, move || {
        ticked.borrow_mut().tick();
    });

    let result = window.run();
    drop(pump);
    drop(window);
    drain(&mut engine.borrow_mut());
    Ok(result?)
}
