//! Windowless end-to-end check of the content-filter hook. Run by hand:
//!
//!     cargo run -p wubble-engine --example block_smoke
//!
//! (after `cargo build --workspace`, so `wubble-helper` sits next to the
//! binaries). It opens one windowless view on a temporary `file://` page
//! whose `<script src>` points at a script next to it, with a filter that
//! blocks exactly that script. It passes when the page's `onerror` fires,
//! the script never runs, the filter blocked one request and also saw the
//! main-frame document, and the page script the filter returns on commit
//! ran in the page; it prints `block smoke: ok`. Blocking a file that
//! exists proves the request was cancelled rather than failing on its own.
//! The filter is a stand-in, not wubble-blocker: this checks the engine's
//! hook only (wubble-blocker's decisions, type mapping and counters have
//! unit tests; its `Blocking` skips non-web URLs such as this `file://`
//! page). No window is shown and nothing touches the network. The deadline only
//! bounds a hang; nothing here measures speed.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::filter::{ContentFilter, RequestAction, RequestInfo, RequestKind};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

const PAGE: &str = r#"<!doctype html>
<html><body>
<script src="blocked.js"
  onload="console.log('smoke script loaded')"
  onerror="console.log('smoke script blocked')"></script>
</body></html>"#;

const SCRIPT: &str = "console.log('smoke script ran');";

/// What the filter injects on commit.
const PAGE_SCRIPT: &str = "console.log('smoke page script ran');";

/// How long a hang may last before the smoke gives up.
const DEADLINE: Duration = Duration::from_secs(60);

/// Blocks `blocked.js`, counts what it saw.
#[derive(Default)]
struct Filter {
    blocked: AtomicU32,
    documents: AtomicU32,
    page_scripts: AtomicU32,
}

impl ContentFilter for Filter {
    fn check(&self, request: &RequestInfo<'_>) -> RequestAction {
        if request.kind == RequestKind::Document {
            self.documents.fetch_add(1, Ordering::Relaxed);
        }
        if request.url.ends_with("/blocked.js") && request.kind == RequestKind::Script {
            self.blocked.fetch_add(1, Ordering::Relaxed);
            return RequestAction::Block;
        }
        RequestAction::Allow
    }

    fn page_script(&self, _top_url: &str) -> Option<String> {
        self.page_scripts.fetch_add(1, Ordering::Relaxed);
        Some(PAGE_SCRIPT.into())
    }
}

fn main() -> ExitCode {
    let dir = std::env::temp_dir().join(format!("wubble-block-smoke-{}", std::process::id()));
    let result = run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("block smoke: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("block smoke: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let page = dir.join("page.html");
    std::fs::write(&page, PAGE).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("blocked.js"), SCRIPT).map_err(|e| e.to_string())?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // target/<profile>/examples/block_smoke -> target/<profile>
    let exe_dir = exe
        .parent()
        .and_then(Path::parent)
        .ok_or("no exe dir")?
        .to_path_buf();
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let presenter = gpu.presenter();
    let filter = Arc::new(Filter::default());
    let mut engine = Engine::start(EngineConfig {
        exe_dir,
        cache_dir: dir.join("cache"),
        pump_scheduler: None,
        content_filter: Some(filter.clone()),
        features: Default::default(),
    })
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
        })
        .map_err(|e| e.to_string())?;

    let outcome = drive(&mut engine, &filter);
    let _ = view;
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
    outcome
}

fn drive(engine: &mut Engine, filter: &Filter) -> Result<(), String> {
    let start = Instant::now();
    let (mut script_blocked, mut page_script_ran) = (false, false);
    while !(script_blocked && page_script_ran) {
        if start.elapsed() > DEADLINE {
            return Err(format!(
                "timed out: onerror {script_blocked}, page script {page_script_ran}, \
                 filter blocked {}, page scripts asked {}",
                filter.blocked.load(Ordering::Relaxed),
                filter.page_scripts.load(Ordering::Relaxed)
            ));
        }
        engine.tick();
        for (_, event) in engine.take_page_events() {
            let PageEvent::Console { message, .. } = event else {
                continue;
            };
            match message.as_str() {
                "smoke script blocked" => script_blocked = true,
                "smoke page script ran" => page_script_ran = true,
                "smoke script ran" | "smoke script loaded" => {
                    return Err("the blocked script loaded".into());
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let blocked = filter.blocked.load(Ordering::Relaxed);
    if blocked != 1 {
        return Err(format!("filter blocked {blocked} requests, expected 1"));
    }
    if filter.documents.load(Ordering::Relaxed) == 0 {
        return Err("the main-frame request never reached the filter".into());
    }
    Ok(())
}
