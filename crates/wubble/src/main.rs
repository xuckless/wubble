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

fn start_engine() -> Result<Engine, EngineError> {
    Engine::start(EngineConfig::from_current_exe()?)
}

/// Pumps until every browser is closed and CEF has shut down. Bounded by the
/// lifecycle state, not by time.
fn drain(engine: &mut Engine) {
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
}

fn smoke_boot() -> Result<(), EngineError> {
    let mut engine = start_engine()?;
    for _ in 0..SMOKE_TICKS {
        engine.tick();
    }
    drain(&mut engine);
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Rc::new(RefCell::new(start_engine()?));
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
