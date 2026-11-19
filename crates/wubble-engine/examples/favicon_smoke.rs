//! Windowless end-to-end check of favicon images and DevTools. Run by hand:
//!
//!     cargo run -p wubble-engine --example favicon_smoke
//!
//! (after `cargo build --workspace`). It opens one windowless view on a
//! temporary `file://` page whose `<link rel=icon>` is a `data:` PNG, and
//! passes when a `PageEvent::FaviconImage` of at most 64x64 arrives. With
//! `devtools` as the first argument it then opens DevTools in a native
//! window, waits for `has_devtools`, closes it and exits (to see whether
//! `ShowDevTools` survives windowless CEF). The deadline only bounds a hang;
//! nothing here measures speed. No network.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

const PAGE: &str = r#"<!doctype html>
<html><head><link rel="icon" href="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="></head>
<body>favicon smoke</body></html>"#;

const DEADLINE: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let devtools = std::env::args().nth(1).as_deref() == Some("devtools");
    let dir = std::env::temp_dir().join(format!("wubble-favicon-smoke-{}", std::process::id()));
    let result = run(&dir, devtools);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("favicon smoke: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("favicon smoke: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path, devtools: bool) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let page = dir.join("page.html");
    std::fs::write(&page, PAGE).map_err(|e| e.to_string())?;
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let presenter = gpu.presenter();
    let mut engine =
        Engine::start(EngineConfig::for_example(dir.join("cache")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let view = engine
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
            container: wubble_core::space::ContainerId::DEFAULT,
        })
        .map_err(|e| e.to_string())?;

    let outcome = drive(&mut engine, view, devtools);
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
    outcome
}

fn drive(
    engine: &mut Engine,
    view: wubble_engine::app::ViewId,
    devtools: bool,
) -> Result<(), String> {
    let start = Instant::now();
    let mut icon = false;
    loop {
        if start.elapsed() > DEADLINE {
            return Err(format!("timed out: icon {icon}"));
        }
        engine.tick();
        for (_, event) in engine.take_page_events() {
            if let PageEvent::FaviconImage(img) = event {
                if img.width > 64
                    || img.height > 64
                    || img.rgba.len() != (img.width * img.height * 4) as usize
                {
                    return Err(format!("bad icon {}x{}", img.width, img.height));
                }
                println!(
                    "favicon: {}x{} {}",
                    img.width,
                    img.height,
                    &img.url[..24.min(img.url.len())]
                );
                icon = true;
                if devtools {
                    engine.view(view).ok_or("view gone")?.open_devtools();
                }
            }
        }
        if icon && !devtools {
            return Ok(());
        }
        if icon && devtools {
            let v = engine.view(view).ok_or("view gone")?;
            if v.has_devtools() {
                println!("devtools: open");
                v.close_devtools();
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
