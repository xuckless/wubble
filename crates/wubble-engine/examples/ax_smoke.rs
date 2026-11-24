//! Windowless check of the page accessibility path. Run by hand:
//!
//!     cargo run -p wubble-engine --example ax_smoke [-- --dump FILE] [--scale 2]
//!
//! (after `cargo build --workspace`, so `wubble-helper` sits next to the binaries). It opens
//! one windowless view on a temporary `file://` page with headings, links, a button and a form,
//! turns CEF's accessibility on (`View::set_accessibility`, TreeOnly mode), feeds every tree
//! change the engine reports (`PageEvent::AxTree`, welding change 50) to `wubble_ax::PageTree`
//! and prints the mapped page, one line per item of `PageTree::flatten`. It passes when the
//! headings (with their levels), the links, the button, the text field and the checkbox are
//! there with their names, and prints `ax smoke: ok`. `--dump FILE` writes the JSON of every
//! tree change, one per line (the fixture in `wubble-ax/src/fixtures` came from it). No window
//! is shown and nothing touches the network. `--scale` sets the view's device scale (default 1):
//! the printed bounds stay CSS pixels at any scale. It waits for tree changes, counted; the deadline
//! only bounds a hang and nothing here measures speed.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_ax::{AxValue, PageTree, value};
use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

const PAGE: &str = r#"<!doctype html>
<html lang="en"><head><title>ax smoke</title></head><body>
<h1>Welcome</h1>
<p>Read the <a href="https://example.invalid/docs">docs</a> first.</p>
<h2>Sign in</h2>
<form>
  <label for="email">Email</label>
  <input id="email" type="text" value="a@b" placeholder="you@example">
  <label><input type="checkbox" checked> Remember me</label>
  <button type="button">Send</button>
</form>
<a href="https://example.invalid/help">Help</a>
</body></html>"#;

/// How long a hang may last before the smoke gives up.
const DEADLINE: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let arg = |name: &str| std::env::args().skip_while(|a| a != name).nth(1);
    let dump = arg("--dump").map(PathBuf::from);
    let scale = arg("--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let dir = std::env::temp_dir().join(format!("wubble-ax-smoke-{}", std::process::id()));
    let result = run(&dir, dump.as_deref(), scale);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("ax smoke: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ax smoke: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path, dump: Option<&Path>, scale: f32) -> Result<(), String> {
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
                size: FrameSize::new(800, 600),
                scale,
                area: None,
            },
            frame_rate: 30,
            begin_frames: false,
            gpu: presenter.context().clone(),
            container: wubble_core::space::ContainerId::DEFAULT,
            storage: wubble_core::space::Storage::Disk,
        })
        .map_err(|e| e.to_string())?;
    if let Some(v) = engine.view(view) {
        v.set_accessibility(true);
    }

    let outcome = drive(&mut engine, dump);
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
    outcome
}

/// What the page must show, as `PageTree::flatten` lines (role, then name).
const EXPECTED: &[&str] = &[
    "heading 1 \"Welcome\"",
    "link \"docs\"",
    "heading 2 \"Sign in\"",
    "textField \"Email\"",
    "checkBox \"Remember me\"",
    "button \"Send\"",
    "link \"Help\"",
];

fn drive(engine: &mut Engine, dump: Option<&Path>) -> Result<(), String> {
    let start = Instant::now();
    let mut tree = PageTree::default();
    let mut raw: Vec<String> = vec![];
    let (mut trees, mut locations) = (0usize, 0usize);
    loop {
        if start.elapsed() > DEADLINE {
            print_items(&tree);
            if let Some(path) = dump {
                let _ = std::fs::write(path, raw.join("\n") + "\n");
            }
            return Err(format!(
                "timed out after {trees} tree and {locations} location changes"
            ));
        }
        engine.tick();
        for (_, event) in engine.take_page_events() {
            match event {
                PageEvent::AxTree(json) => {
                    trees += 1;
                    let v = AxValue::from_json(&json).ok_or("a tree change that is not JSON")?;
                    tree.apply(&value::parse_updates(&v));
                    raw.push(json);
                }
                PageEvent::AxLocation(json) => {
                    locations += 1;
                    let v =
                        AxValue::from_json(&json).ok_or("a location change that is not JSON")?;
                    tree.move_nodes(&value::parse_locations(&v));
                    raw.push(json);
                }
                _ => {}
            }
        }
        let lines: Vec<String> = tree.flatten(usize::MAX).iter().map(|i| i.line()).collect();
        if EXPECTED
            .iter()
            .all(|e| lines.iter().any(|l| l.starts_with(e)))
        {
            println!("ax smoke: {trees} tree and {locations} location changes");
            print_items(&tree);
            if let Some(path) = dump {
                std::fs::write(path, raw.join("\n") + "\n").map_err(|e| e.to_string())?;
                println!("ax smoke: dumped to {}", path.display());
            }
            return Ok(());
        }
    }
}

fn print_items(tree: &PageTree) {
    for item in tree.flatten(usize::MAX) {
        println!("  {}", item.line());
    }
}
