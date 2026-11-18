//! Windowless end-to-end check of the engine's page surface. Run by hand:
//!
//!     cargo run -p wubble-engine --example engine_smoke
//!
//! (after `cargo build --workspace`, so `wubble-helper` sits next to the
//! binaries). It opens one windowless view on a temporary `file://` page that
//! calls `alert`, `confirm` and `prompt`, asks for geolocation, starts a
//! download and has a `<select>`; it answers each request, opens the select
//! and a context menu with synthetic clicks, and exits with
//! `engine smoke: ok` or the step that failed. No window is shown. The
//! deadline only bounds a hang; nothing here measures speed.
//!
//! With `-- --default-now` the engine answers every request itself with its
//! default (`RequestMode::DefaultNow`, what a host without prompt UI gets):
//! the page must see cancelled dialogs, a denied permission and a cancelled
//! download, and nothing may hang.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use winit::keyboard::ModifiersState;
use wubble_engine::app::{Engine, EngineConfig, RequestMode, ViewId};
use wubble_engine::input::{Buttons, Modifiers, MouseButton, Point};
use wubble_engine::page::{JsDialogKind, PageEvent, PermissionKind};
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

const PAGE: &str = r#"<!doctype html>
<html><body style="margin:0">
<select id="s" style="position:absolute;left:0;top:0;width:200px;height:40px">
  <option>one</option><option>two</option><option>three</option>
</select>
<script>
window.onload = () => {
  alert("smoke alert");
  const c = confirm("smoke confirm");
  const p = prompt("smoke prompt", "default");
  console.log(`smoke dialogs c=${c} p=${p}`);
  navigator.geolocation.getCurrentPosition(
    () => console.log("smoke geo granted"),
    () => {
      console.log("smoke geo denied");
      const a = document.createElement("a");
      a.href = "data:text/plain,engine%20smoke";
      a.download = "smoke.txt";
      document.body.appendChild(a);
      a.click();
    });
};
</script>
</body></html>"#;

/// The steps in order; each is passed when its event arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Alert,
    Confirm,
    Prompt,
    DialogResults,
    Geolocation,
    GeoDenied,
    DownloadPath,
    DownloadFinished,
    /// `--default-now`: the download was declined.
    DownloadDeclined,
    Popup,
    ContextMenu,
    Done,
}

/// How long a hang may last before the smoke gives up.
const DEADLINE: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    let dir = std::env::temp_dir().join(format!("wubble-engine-smoke-{}", std::process::id()));
    let mode = if std::env::args().any(|a| a == "--default-now") {
        RequestMode::DefaultNow
    } else {
        RequestMode::AskHost
    };
    let result = run(&dir, mode);
    let _ = std::fs::remove_dir_all(&dir);
    match result {
        Ok(()) => {
            println!("engine smoke: ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("engine smoke: FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(dir: &Path, mode: RequestMode) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let page = dir.join("smoke.html");
    std::fs::write(&page, PAGE).map_err(|e| e.to_string())?;
    let download_to = dir.join("downloaded.txt");

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // target/<profile>/examples/engine_smoke -> target/<profile>
    let exe_dir = exe
        .parent()
        .and_then(Path::parent)
        .ok_or("no exe dir")?
        .to_path_buf();
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let presenter = gpu.presenter();
    let mut engine = Engine::start(EngineConfig {
        exe_dir,
        cache_dir: dir.join("cache"),
        pump_scheduler: None,
    })
    .map_err(|e| e.to_string())?;
    engine.set_request_mode(mode);
    let view = engine
        .open_view(ViewConfig {
            url: format!("file://{}", page.display()),
            layout: Layout {
                size: FrameSize::new(800, 600),
                scale: 1.0,
            },
            frame_rate: 30,
            gpu: presenter.context().clone(),
        })
        .map_err(|e| e.to_string())?;

    let outcome = drive(&mut engine, view, &download_to, mode);
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
    outcome
}

fn drive(
    engine: &mut Engine,
    view: ViewId,
    download_to: &Path,
    mode: RequestMode,
) -> Result<(), String> {
    let start = Instant::now();
    let default_now = mode == RequestMode::DefaultNow;
    // DefaultNow reports no requests, only what the page does with the
    // default answers.
    let mut step = if default_now {
        Step::DialogResults
    } else {
        Step::Alert
    };
    let mut clicked_select = false;
    while step != Step::Done {
        if start.elapsed() > DEADLINE {
            return Err(format!("no event for step {step:?}"));
        }
        engine.tick();
        for (id, event) in engine.take_page_events() {
            if id != view {
                continue;
            }
            step = handle(engine, view, step, event, download_to, default_now)?;
        }
        if step == Step::Popup && !clicked_select {
            clicked_select = true;
            click(engine, view, MouseButton::Left, Point { x: 20.0, y: 20.0 });
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn handle(
    engine: &mut Engine,
    view_id: ViewId,
    step: Step,
    event: PageEvent,
    download_to: &Path,
    default_now: bool,
) -> Result<Step, String> {
    let view = engine.view(view_id).ok_or("view gone")?;
    let next = match (step, event) {
        (
            Step::Alert,
            PageEvent::DialogRequested {
                id,
                kind: JsDialogKind::Alert,
                ..
            },
        ) => answer(view.answer_dialog(id, true, ""), Step::Confirm)?,
        (
            Step::Confirm,
            PageEvent::DialogRequested {
                id,
                kind: JsDialogKind::Confirm,
                ..
            },
        ) => answer(view.answer_dialog(id, true, ""), Step::Prompt)?,
        (
            Step::Prompt,
            PageEvent::DialogRequested {
                id,
                kind: JsDialogKind::Prompt,
                default_text,
                ..
            },
        ) => {
            if default_text != "default" {
                return Err(format!("prompt default {default_text:?}"));
            }
            answer(view.answer_dialog(id, true, "smoke"), Step::DialogResults)?
        }
        (Step::DialogResults, PageEvent::Console { message, .. })
            if message.starts_with("smoke dialogs") =>
        {
            let expected = if default_now {
                "smoke dialogs c=false p=null"
            } else {
                "smoke dialogs c=true p=smoke"
            };
            if message != expected {
                return Err(format!("dialog answers reached the page as {message:?}"));
            }
            if default_now {
                Step::GeoDenied
            } else {
                Step::Geolocation
            }
        }
        (
            Step::Geolocation,
            PageEvent::PermissionRequested {
                id, permissions, ..
            },
        ) => {
            if !permissions.contains(&PermissionKind::Geolocation) {
                return Err(format!("asked for {permissions:?}"));
            }
            answer(view.answer_permission(id, &[]), Step::GeoDenied)?
        }
        (Step::GeoDenied, PageEvent::Console { message, .. })
            if message.starts_with("smoke geo") =>
        {
            if message != "smoke geo denied" {
                return Err(format!("geolocation: {message}"));
            }
            if default_now {
                Step::DownloadDeclined
            } else {
                Step::DownloadPath
            }
        }
        (Step::DownloadDeclined, PageEvent::DownloadCancelled(_)) => Step::Done,
        (
            Step::DownloadPath,
            PageEvent::DownloadRequested {
                id, suggested_name, ..
            },
        ) => {
            if suggested_name != "smoke.txt" {
                return Err(format!("suggested name {suggested_name:?}"));
            }
            answer(
                view.answer_download_path(id, Some(download_to)),
                Step::DownloadFinished,
            )?
        }
        (Step::DownloadFinished, PageEvent::DownloadFinished { path, .. }) => {
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if text != "engine smoke" {
                return Err(format!("downloaded {text:?}"));
            }
            Step::Popup
        }
        (Step::DownloadFinished, PageEvent::DownloadFailed { error, .. }) => {
            return Err(format!("download failed: {error}"));
        }
        (Step::Popup, PageEvent::PopupShown(rect)) => {
            if rect.width == 0 || rect.height == 0 {
                return Err(format!("popup at {rect:?}"));
            }
            // Close the dropdown, then right-click below it.
            click(
                engine,
                view_id,
                MouseButton::Left,
                Point { x: 400.0, y: 500.0 },
            );
            click(
                engine,
                view_id,
                MouseButton::Right,
                Point { x: 400.0, y: 500.0 },
            );
            Step::ContextMenu
        }
        (Step::ContextMenu, PageEvent::ContextMenuRequested { id, menu }) => {
            if menu.items.is_empty() {
                return Err("context menu has no items".into());
            }
            answer(view.answer_context_menu(id, None), Step::Done)?
        }
        (step, event) if event.request().is_some() => {
            return Err(format!("unexpected request at {step:?}: {event:?}"));
        }
        (step, _) => step,
    };
    Ok(next)
}

fn answer(ok: bool, next: Step) -> Result<Step, String> {
    if ok {
        Ok(next)
    } else {
        Err(format!("answer before {next:?} was refused"))
    }
}

fn click(engine: &mut Engine, view: ViewId, button: MouseButton, point: Point) {
    let Some(view) = engine.view(view) else {
        return;
    };
    let now = Duration::ZERO;
    let up = Modifiers {
        keys: ModifiersState::empty(),
        buttons: Buttons::default(),
    };
    let down = Modifiers {
        buttons: Buttons::default().with(button, true),
        ..up
    };
    view.focus(true);
    view.mouse_move(point, &up, now);
    view.mouse_button(button, true, point, &down, now);
    view.mouse_button(button, false, point, &up, now);
}
