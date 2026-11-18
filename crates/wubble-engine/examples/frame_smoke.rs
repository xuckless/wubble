//! Windowless check of the zero-copy frame path: CEF paints a page of colour
//! swatches, the frame is presented on wubble-gfx's device and read back at
//! each swatch. Run by hand:
//!
//!     cargo run -p wubble-engine --example frame_smoke      # Linux
//!
//! On macOS it must run inside the app bundle, which holds the framework and
//! the helper apps:
//!
//!     cargo build --workspace --examples
//!     cargo run -p wubble-bundle -- --exe target/debug/examples/frame_smoke
//!     target/debug/bundle/wubble.app/Contents/MacOS/wubble
//!
//! Prints `frame smoke: ok` when every swatch of the bundled smoke page has
//! its colour and no frame took the CPU `OnPaint` fallback (`cpu_fallback=0`).
//! The readback is the check, never on the frame path. Needs no window, so it works over ssh.
//! The deadline only bounds a hang (of the page or of CEF's shutdown);
//! nothing here measures speed.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext, swatch};

/// The bundled smoke page, whose swatches `wubble_gfx::swatch` checks.
const PAGE: &str = include_str!("../../wubble/assets/smoke.html");

/// How long a hang may last before the smoke gives up.
const DEADLINE: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let dir = std::env::temp_dir().join(format!("wubble-frame-smoke-{}", std::process::id()));
    let result = run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("frame smoke: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("frame smoke: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let page = dir.join("smoke.html");
    std::fs::write(&page, PAGE).map_err(|e| e.to_string())?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // macOS: wubble.app/Contents/MacOS, as the bundled wubble.
    // Elsewhere: target/<profile>/examples/frame_smoke -> target/<profile>.
    let exe_dir = if cfg!(target_os = "macos") {
        exe.parent()
    } else {
        exe.parent().and_then(Path::parent)
    }
    .ok_or("no exe dir")?
    .to_path_buf();
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    println!("frame smoke: gpu {}", gpu.describe());
    let mut presenter = gpu.presenter();
    let mut engine = Engine::start(EngineConfig {
        exe_dir,
        cache_dir: dir.join("cache"),
        pump_scheduler: None,
        content_filter: None,
        features: Default::default(),
        profile: wubble_engine::app::ProfileKind::Throwaway,
    })
    .map_err(|e| e.to_string())?;
    let id = engine
        .open_view(ViewConfig {
            url: format!("file://{}", page.display()),
            layout: Layout {
                size: FrameSize::new(400, 300),
                scale: 1.0,
                area: None,
            },
            frame_rate: 30,
            begin_frames: false,
            gpu: presenter.context().clone(),
        })
        .map_err(|e| e.to_string())?;

    let outcome = (|| {
        let start = Instant::now();
        let mut last = Err("no frame".to_string());
        while start.elapsed() < DEADLINE {
            engine.tick();
            std::thread::sleep(Duration::from_millis(2));
            let view = engine.view(id).ok_or("view gone")?;
            let Some(frame) = view.take_frame() else {
                continue;
            };
            presenter.present(frame).map_err(|e| e.to_string())?;
            // The first frames may be blank while the page loads.
            last = swatch::check(|x, y| presenter.probe(x, y).map(|[r, g, b, _]| [r, g, b]));
            if last.is_ok() {
                let fallback = view.cpu_fallback_frames();
                println!("frame smoke: swatches ok cpu_fallback={fallback:?}");
                return match fallback {
                    Some(0) => Ok(()),
                    other => Err(format!("CPU fallback frames: {other:?}")),
                };
            }
        }
        Err(format!("swatches never matched: {}", last.unwrap_err()))
    })();
    engine.request_shutdown();
    let stop = Instant::now();
    while !engine.is_stopped() {
        if stop.elapsed() >= DEADLINE {
            // Dropping a half-stopped CEF may block too: leave it to the
            // process exit, so the smoke always ends.
            std::mem::forget(engine);
            return Err(format!("CEF did not shut down ({outcome:?})"));
        }
        engine.tick();
        std::thread::sleep(Duration::from_millis(2));
    }
    outcome
}
