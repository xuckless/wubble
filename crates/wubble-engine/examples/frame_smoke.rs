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
//! its colour and no frame took the CPU `OnPaint` fallback (`cpu_fallback=0`);
//! when a 10-bit video shows (`10-bit video ok`, not black); and on Linux when
//! every renderer of this process runs under seccomp-bpf (`sandbox ok`, read from `/proc`).
//! The readback is the check, never on the frame path. Needs no window, so it works over ssh.
//! The deadline only bounds a hang (of the page or of CEF's shutdown);
//! nothing here measures speed.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_engine::app::{Engine, EngineConfig, ViewId};
use wubble_engine::view::{Layout, ViewConfig};

use wubble_gfx::{FramePresenter, FrameSize, GpuContext, swatch};

/// The bundled smoke page, whose swatches `wubble_gfx::swatch` checks.
const PAGE: &str = include_str!("../../wubble/assets/smoke.html");

/// A 10-bit AV1 clip (three PQ greys side by side, from the HDR probe), filling the view.
const VIDEO_10BIT: &[u8] = include_bytes!("hdr_probe/pq-hdr.mp4");
const VIDEO_PAGE: &str = "<!doctype html><body style='margin:0;background:#000'>\
    <video src='pq-hdr.mp4' muted loop autoplay playsinline \
    style='position:fixed;inset:0;width:100%;height:100%;object-fit:fill'></video>";

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
    let video_page = dir.join("video.html");
    std::fs::write(&video_page, VIDEO_PAGE).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("pq-hdr.mp4"), VIDEO_10BIT).map_err(|e| e.to_string())?;

    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    println!("frame smoke: gpu {}", gpu.describe());
    let mut presenter = gpu.presenter();
    let mut engine =
        Engine::start(EngineConfig::for_example(dir.join("cache")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let open = |engine: &mut Engine, url: String| {
        engine
            .open_view(ViewConfig {
                url,
                layout: Layout {
                    size: FrameSize::new(400, 300),
                    scale: 1.0,
                    area: None,
                },
                frame_rate: 30,
                begin_frames: false,
                gpu: presenter.context().clone(),
            })
            .map_err(|e| e.to_string())
    };
    let swatches = open(&mut engine, format!("file://{}", page.display()))?;
    let video = open(&mut engine, format!("file://{}", video_page.display()))?;

    let outcome = (|| {
        // The first frames may be blank while the page loads.
        until(
            &mut engine,
            &mut presenter,
            swatches,
            "swatches",
            |presenter| swatch::check(|x, y| presenter.probe(x, y).map(|[r, g, b, _]| [r, g, b])),
        )?;
        let fallback = engine
            .view(swatches)
            .ok_or("view gone")?
            .cpu_fallback_frames();
        println!("frame smoke: swatches ok cpu_fallback={fallback:?}");
        if fallback != Some(0) {
            return Err(format!("CPU fallback frames: {fallback:?}"));
        }
        // The clip's middle third is a PQ 0.752 grey (SDR white once tone-mapped);
        // black was the bug (TODO.md M1, `disable-gpu-memory-buffer-video-frames`).
        until(
            &mut engine,
            &mut presenter,
            video,
            "10-bit video",
            |presenter| match presenter.probe(200, 150) {
                Some([r, g, b, _]) if r.max(g).max(b) >= 64 => Ok(()),
                other => Err(format!("centre pixel {other:?}")),
            },
        )?;
        println!("frame smoke: 10-bit video ok");
        #[cfg(target_os = "linux")]
        {
            let renderers = sandboxed_processes()?;
            println!("frame smoke: sandbox ok renderers={renderers} gpu=seccomp");
        }
        Ok(())
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

/// Presents `id`'s frames until `check` passes on one; the last failure if none does
/// before [`DEADLINE`] (which only bounds a hang).
fn until(
    engine: &mut Engine,
    presenter: &mut FramePresenter,
    id: ViewId,
    what: &str,
    check: impl Fn(&FramePresenter) -> Result<(), String>,
) -> Result<(), String> {
    let start = Instant::now();
    let mut last = Err("no frame".to_string());
    while start.elapsed() < DEADLINE {
        engine.tick();
        std::thread::sleep(Duration::from_millis(2));
        let Some(frame) = engine.view(id).ok_or("view gone")?.take_frame() else {
            continue;
        };
        presenter.present(frame).map_err(|e| e.to_string())?;
        last = check(presenter);
        if last.is_ok() {
            return Ok(());
        }
    }
    Err(format!("{what} never matched: {}", last.unwrap_err()))
}

/// How many sandboxed renderers this process has, once each and the GPU process are in
/// seccomp filter mode (`Seccomp: 2` in `/proc/<pid>/status`); an error names the first
/// that is not, or says none was found. Read from the process tree, as renderers are not dumpable and their
/// command lines are the zygote's: Chromium's sandboxed zygote is a process in a new user
/// namespace (its init) with the zygote under it, and every process the zygote forks, two
/// levels into that namespace, is a renderer.
#[cfg(target_os = "linux")]
fn sandboxed_processes() -> Result<usize, String> {
    use std::collections::HashMap;
    struct Proc {
        parent: u32,
        userns: std::path::PathBuf,
        seccomp: Option<u32>,
    }
    let read = |pid: u32| {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let field = |name: &str| {
            status
                .lines()
                .find_map(|l| l.strip_prefix(name)?.trim().parse::<u32>().ok())
        };
        Some(Proc {
            parent: field("PPid:")?,
            userns: std::fs::read_link(format!("/proc/{pid}/ns/user")).ok()?,
            seccomp: field("Seccomp:"),
        })
    };
    let procs: HashMap<u32, Proc> = std::fs::read_dir("/proc")
        .map_err(|e| e.to_string())?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter_map(|pid| Some((pid, read(pid)?)))
        .collect();
    let me = std::process::id();
    let my_ns = &procs.get(&me).ok_or("this process is not in /proc")?.userns;
    let inside = |pid: &u32| procs.get(pid).is_some_and(|p| &p.userns != my_ns);
    let ours = |mut pid: u32| loop {
        match procs.get(&pid).map(|p| p.parent) {
            Some(parent) if parent == me => return true,
            Some(parent) if parent > 1 => pid = parent,
            _ => return false,
        }
    };
    let renderers: Vec<(&u32, &Proc)> = procs
        .iter()
        .filter(|(pid, p)| inside(pid) && inside(&p.parent) && ours(**pid))
        .filter(|(_, p)| {
            procs
                .get(&p.parent)
                .is_some_and(|zygote| inside(&zygote.parent))
        })
        .collect();
    if renderers.is_empty() {
        return Err("no sandboxed renderer of this process found in /proc".into());
    }
    if let Some((pid, p)) = renderers.iter().find(|(_, p)| p.seccomp != Some(2)) {
        return Err(format!(
            "renderer {pid} is not under seccomp-bpf (Seccomp: {:?})",
            p.seccomp
        ));
    }
    // Forked by the unsandboxed zygote, so its command line is the zygote's: known by
    // its Viz compositor thread instead.
    let gpu = procs
        .iter()
        .find(|(pid, _)| ours(**pid) && has_thread(**pid, "VizCompositorTh"))
        .ok_or("no GPU process (with a VizCompositorTh thread) of this process found in /proc")?;
    if gpu.1.seccomp != Some(2) {
        return Err(format!(
            "GPU process {} is not under seccomp-bpf (Seccomp: {:?})",
            gpu.0, gpu.1.seccomp
        ));
    }
    Ok(renderers.len())
}

/// Whether one of `pid`'s threads is named `name` (`/proc/<pid>/task/*/comm`, which Chromium
/// sets for its named threads and anyone may read).
#[cfg(target_os = "linux")]
fn has_thread(pid: u32, name: &str) -> bool {
    std::fs::read_dir(format!("/proc/{pid}/task")).is_ok_and(|tasks| {
        tasks.filter_map(Result::ok).any(|task| {
            std::fs::read_to_string(task.path().join("comm")).is_ok_and(|c| c.trim() == name)
        })
    })
}
