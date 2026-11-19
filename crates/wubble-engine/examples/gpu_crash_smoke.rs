//! Windowless check that pages keep painting after the GPU process dies. Run by hand:
//!
//!     cargo build --workspace
//!     cargo run -p wubble-engine --example gpu_crash_smoke
//!
//! One view shows a page that changes every animation frame; a second view loads
//! `chrome://gpucrash` (Chromium's debug URL, which kills the GPU process). The smoke waits,
//! by counts of what arrives, for the GPU process's id to change (`wubble_sysmem::Meter`, as
//! the browser's watch does: CEF reports no GPU-process crash), asks the page for a fresh frame
//! (`View::repaint`, what the browser does on a restart), and then for more zero-copy frames
//! from the page. It prints `gpu crash smoke: ok` when frames resumed and none took the CPU
//! `OnPaint` fallback. The deadline only bounds a hang; nothing here measures speed.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_core::gpu::{self, Change};
use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};
use wubble_sysmem::Meter;

/// A page that paints every frame.
const PAGE: &str = r#"<!doctype html><html><body style="margin:0">
<canvas id="c" width="200" height="100"></canvas>
<script>
const g = document.getElementById("c").getContext("2d");
let n = 0;
(function draw() { n++; g.fillStyle = `hsl(${n % 360} 80% 50%)`; g.fillRect(0, 0, 200, 100);
  requestAnimationFrame(draw); })();
</script></body></html>"#;

/// Frames the page must paint before the crash and again after it.
const FRAMES: u32 = 30;

/// How long a hang may last before the smoke gives up.
const DEADLINE: Duration = Duration::from_secs(90);

fn main() -> ExitCode {
    let dir = std::env::temp_dir().join(format!("wubble-gpu-crash-{}", std::process::id()));
    let result = run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("gpu crash smoke: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gpu crash smoke: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, PartialEq)]
enum Stage {
    /// Frames before the crash, and the GPU process's id.
    Before,
    /// `chrome://gpucrash` loaded; waiting for another GPU process.
    Crashing,
    /// Frames after the restart.
    After,
    Done,
}

fn run(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let page = dir.join("paint.html");
    std::fs::write(&page, PAGE).map_err(|e| e.to_string())?;
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let mut presenter = gpu.presenter();
    let mut engine =
        Engine::start(EngineConfig::for_example(dir.join("cache")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let layout = Layout {
        size: FrameSize::new(200, 100),
        scale: 1.0,
        area: None,
    };
    let context = presenter.context().clone();
    let open = |engine: &mut Engine, url: String| {
        engine.open_view(ViewConfig {
            url,
            layout,
            frame_rate: 30,
            begin_frames: false,
            gpu: context.clone(),
        })
    };
    let painter =
        open(&mut engine, format!("file://{}", page.display())).map_err(|e| e.to_string())?;

    let mut meter = Meter::default();
    let outcome = (|| {
        let start = Instant::now();
        let (mut stage, mut frames, mut last_pid, mut seen) = (Stage::Before, 0u32, None, false);
        let mut crasher = None;
        let mut polls = 0u32;
        while stage != Stage::Done {
            if start.elapsed() > DEADLINE {
                return Err(format!("stuck at {stage:?} after {frames} frames"));
            }
            engine.tick();
            let _ = engine.take_page_events();
            std::thread::sleep(Duration::from_millis(2));
            let view = engine.view(painter).ok_or("painter gone")?;
            if let Some(frame) = view.take_frame() {
                presenter.present(frame).map_err(|e| e.to_string())?;
                frames += 1;
            }
            // The process list is read every 50th turn only (a scan of every process).
            polls += 1;
            if !polls.is_multiple_of(50) {
                continue;
            }
            let pid = meter.gpu_pid();
            let change = gpu::watch(last_pid, pid, seen);
            seen |= pid.is_some();
            last_pid = pid;
            match (&stage, change) {
                (Stage::Before, _) if frames >= FRAMES && pid.is_some() => {
                    println!("gpu crash smoke: {frames} frames, GPU pid {pid:?}; crashing it");
                    crasher = Some(
                        open(&mut engine, "chrome://gpucrash".into()).map_err(|e| e.to_string())?,
                    );
                    stage = Stage::Crashing;
                }
                (Stage::Crashing, Change::Gone { was }) => {
                    println!("gpu crash smoke: GPU process {was} gone");
                }
                (Stage::Crashing, Change::Restarted { was, now }) => {
                    println!("gpu crash smoke: GPU process restarted ({was:?} -> {now})");
                    engine.view(painter).ok_or("painter gone")?.repaint();
                    frames = 0;
                    stage = Stage::After;
                }
                (Stage::After, _) if frames >= FRAMES => stage = Stage::Done,
                _ => {}
            }
        }
        let _ = crasher;
        let fallback = engine.view(painter).and_then(|v| v.cpu_fallback_frames());
        println!("gpu crash smoke: {frames} frames after the restart, cpu_fallback={fallback:?}");
        match fallback {
            Some(0) => Ok(()),
            other => Err(format!("CPU fallback frames: {other:?}")),
        }
    })();
    engine.request_shutdown();
    let stop = Instant::now();
    while !engine.is_stopped() {
        if stop.elapsed() >= DEADLINE {
            std::mem::forget(engine);
            return Err(format!("CEF did not shut down ({outcome:?})"));
        }
        engine.tick();
    }
    outcome
}
