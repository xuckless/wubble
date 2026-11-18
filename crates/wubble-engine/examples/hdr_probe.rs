//! HDR probe: what does CEF's windowless mode report to a page, and what
//! pixel values arrive for HDR content? Run by hand, no window opens:
//!
//!     ~/Dev/wubble-wt/cargo.sh run -p wubble-engine --example hdr_probe
//!     WUBBLE_PROBE_SWITCHES='force-color-profile=hdr10' ... (see `app::probe_switches`)
//!
//! The page (`hdr_probe/page.html`) logs what media queries, CSS and canvas
//! say about the display and decodes an HDR10 (PQ, BT.2020) AVIF image and AV1
//! video, 10-bit and an 8-bit control (all generated for this probe with
//! ffmpeg/libaom, no third-party content; the 8-bit clip is the 10-bit one
//! re-encoded with its tags kept:
//! `ffmpeg -i pq-hdr.mp4 -vf format=yuv420p -c:v libaom-av1 -cpu-used 8 -crf 10
//! -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc
//! -color_range tv -an pq-hdr-8bit.mp4`). Once it has logged, the example reads back 8-bit pixels of the
//! frame wubble-gfx presented, at the swatch centres, and prints them. A
//! readback for the research only, never the frame path. Nothing is asserted:
//! the numbers are the finding (see notes/b6/h1-hdr.md).

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

const PAGE: &str = include_str!("hdr_probe/page.html");
const AVIF: &[u8] = include_bytes!("hdr_probe/pq-hdr.avif");
const VIDEO: &[u8] = include_bytes!("hdr_probe/pq-hdr.mp4");
const VIDEO_8BIT: &[u8] = include_bytes!("hdr_probe/pq-hdr-8bit.mp4");
const SDR_VIDEO: &[u8] = include_bytes!("hdr_probe/sdr.mp4");

/// Only bounds a hang.
const DEADLINE: Duration = Duration::from_secs(60);

/// (name, x, y) of the probed pixels in the 400x400 frame.
const PROBES: &[(&str, u32, u32)] = &[
    ("css rec2100-pq 0.581 (203 nit)", 30, 30),
    ("css rec2100-pq 0.752 (1000 nit)", 80, 30),
    ("css rec2100-pq 0.903 (4000 nit)", 130, 30),
    ("css srgb-linear 4 (4x SDR white)", 180, 30),
    ("css srgb 1 (SDR white)", 230, 30),
    ("css srgb 0.5", 280, 30),
    ("canvas rec2100-pq f16 0.752", 330, 30),
    ("avif PQ 0.581", 58, 118),
    ("avif PQ 0.752", 154, 118),
    ("avif PQ 0.903", 250, 118),
    ("10-bit video PQ 0.581", 37, 225),
    ("10-bit video PQ 0.752", 90, 225),
    ("10-bit video PQ 0.903", 143, 225),
    ("8-bit video PQ 0.581", 37, 325),
    ("8-bit video PQ 0.752", 90, 325),
    ("8-bit video PQ 0.903", 143, 325),
    ("css rec2020 green (outside sRGB)", 370, 90),
    ("css display-p3 red", 370, 140),
    ("sdr video grey 0.5", 227, 225),
    ("sdr video grey 0.75", 280, 225),
    ("sdr video grey 1.0", 333, 225),
];

fn main() -> ExitCode {
    let dir = std::env::temp_dir().join(format!("wubble-hdr-probe-{}", std::process::id()));
    let result = run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hdr probe: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let page = dir.join("page.html");
    std::fs::write(&page, PAGE).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pq-hdr.avif"), AVIF).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pq-hdr.mp4"), VIDEO).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pq-hdr-8bit.mp4"), VIDEO_8BIT).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("sdr.mp4"), SDR_VIDEO).map_err(|e| e.to_string())?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe
        .parent()
        .and_then(Path::parent)
        .ok_or("no exe dir")?
        .to_path_buf();
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    println!("hdr probe: gpu {}", gpu.describe());
    println!(
        "hdr probe: switches {:?}",
        std::env::var("WUBBLE_PROBE_SWITCHES").ok()
    );
    let mut presenter = gpu.presenter();
    let mut engine = Engine::start(EngineConfig {
        exe_dir,
        cache_dir: dir.join("cache"),
        pump_scheduler: None,
        content_filter: None,
        response_observer: None,
        features: Default::default(),
    })
    .map_err(|e| e.to_string())?;
    let id = engine
        .open_view(ViewConfig {
            url: format!("file://{}", page.display()),
            layout: Layout {
                size: FrameSize::new(400, 400),
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
        let mut done = false;
        let mut frames_after_done = 0;
        while start.elapsed() < DEADLINE {
            engine.tick();
            for (view, event) in engine.take_page_events() {
                if let (true, PageEvent::Console { message, .. }) = (view == id, event) {
                    if message == "HDRPROBE done" {
                        done = true;
                    } else if message.starts_with("HDRPROBE ") {
                        println!("{message}");
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
            let view = engine.view(id).ok_or("view gone")?;
            let Some(frame) = view.take_frame() else {
                continue;
            };
            presenter.present(frame).map_err(|e| e.to_string())?;
            if done {
                frames_after_done += 1;
                // Frame count, not time: let the video show a few frames.
                if frames_after_done == 60 {
                    for (name, x, y) in PROBES {
                        println!("pixel {name:<36} {:?}", presenter.probe(*x, *y));
                    }
                    println!("hdr probe: cpu_fallback={:?}", view.cpu_fallback_frames());
                    return Ok(());
                }
            }
        }
        Err("no result before the deadline".to_string())
    })();
    engine.request_shutdown();
    let stop = Instant::now();
    while !engine.is_stopped() {
        if stop.elapsed() >= DEADLINE {
            std::mem::forget(engine);
            return Err(format!("CEF did not shut down ({outcome:?})"));
        }
        engine.tick();
        std::thread::sleep(Duration::from_millis(2));
    }
    outcome
}
