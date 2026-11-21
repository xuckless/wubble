//! `--smoke spell`: spell check works on a fresh profile, from the dictionary bundled in the app,
//! with no request off the machine.
//!
//! A throwaway profile, no window: a loopback page (`assets/spell.html`) with a textarea.
//! 1. The textarea is focused and a misspelt word is typed into it (an input method commit, the
//!    host's text path).
//! 2. The word is right-clicked until Chromium's menu for it carries `misspelled_word` and
//!    suggestions: what wubble's native context menu shows first (`menu.rs`).
//! 3. After the quit, Chromium's net log is audited (`net_audit.rs`): nothing looked up or
//!    connected off loopback, no dictionary download.
//!
//! Bounded by a count of pump turns, not by time.

use std::error::Error;
use std::path::Path;
use std::sync::PoisonError;

use wubble_engine::app::{Engine, EngineConfig, EngineError, RequestMode};
use wubble_engine::input::Point;
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

use crate::quit_smoke::{Pump, right_click};

/// Typed with a trailing space: Chromium checks a word once it is finished.
const TYPED: &str = "teh ";
const WORD: &str = "teh";
/// A suggestion CEF must offer for [`WORD`].
const EXPECTED: &str = "the";
/// Over the first word of the textarea (`assets/spell.html`: left 20, top 20, 24px type).
const WORD_AT: Point = Point { x: 40.0, y: 36.0 };
/// Turns between tries at the right-click, while the check catches up.
const RETRY_EVERY: u32 = 100;

/// Passes with a line naming the suggestions and the requests the log held.
pub fn run(
    config: Result<EngineConfig, EngineError>,
    dir: &Path,
) -> Result<String, Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    let net_log = dir.join("net-log.json");
    // SAFETY: the smoke's first act, before this process starts any thread of its own; the
    // engine reads it once, in `Engine::start`.
    unsafe {
        std::env::set_var(
            "WUBBLE_PROBE_SWITCHES",
            crate::net_audit::probe_switch(&net_log),
        );
    }
    let port = crate::autofill_smoke::serve()?;
    let gpu = GpuContext::new()?;
    let (mut pump, scheduler) = Pump::new();
    let mut engine = Engine::start(EngineConfig {
        pump_scheduler: Some(scheduler),
        ..config?
    })?;
    engine.set_request_mode(RequestMode::AskHost);
    pump.requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(0);
    let id = engine.open_view(ViewConfig {
        url: format!("http://127.0.0.1:{port}/spell"),
        layout: Layout {
            size: FrameSize::new(800, 600),
            scale: 1.0,
            area: None,
        },
        frame_rate: 30,
        begin_frames: false,
        gpu: gpu.presenter().context().clone(),
        container: wubble_core::space::ContainerId::DEFAULT,
    })?;

    // 1. Ready, focused, typed.
    let (mut ready, mut focused, mut typed) = (false, false, false);
    pump.until(&mut engine, "the word typed", |engine, _, events| {
        for (view, event) in events {
            if let (true, PageEvent::Console { message, .. }) = (view == id, event) {
                match message.as_str() {
                    "spell: ready" => ready = true,
                    "spell: focused" => focused = true,
                    m if m.starts_with("spell: input") => typed = true,
                    _ => {}
                }
            }
        }
        if let Some(view) = engine.view(id) {
            if ready && !focused {
                view.focus(true);
                view.run_script("document.querySelector('textarea').focus()");
            } else if focused && !typed {
                view.ime_commit(TYPED);
            }
        }
        typed
    })?;

    // 2. Right-click the word until its menu says it is misspelt.
    let mut found: Option<Vec<String>> = None;
    pump.until(
        &mut engine,
        "a context menu on the misspelt word",
        |engine, turns, events| {
            for (view, event) in events {
                if let (true, PageEvent::ContextMenuRequested { id: menu, menu: at }) =
                    (view == id, event)
                {
                    if let Some(view) = engine.view(id) {
                        view.answer_context_menu(menu, None);
                    }
                    if at.misspelled_word == WORD {
                        found = Some(at.suggestions);
                    }
                }
            }
            if found.is_none()
                && turns % RETRY_EVERY == RETRY_EVERY - 1
                && let Some(view) = engine.view(id)
            {
                right_click(view, WORD_AT);
            }
            found.is_some()
        },
    )?;
    let suggestions = found.unwrap_or_default();
    if !suggestions.iter().any(|s| s == EXPECTED) {
        return Err(format!("{WORD:?} got suggestions {suggestions:?}, not {EXPECTED:?}").into());
    }

    // 3. Quit, then what the net log says.
    crate::drain(&mut engine);
    let report = crate::net_audit::audit(&std::fs::read_to_string(&net_log)?)?;
    if report.urls == 0 {
        return Err("the net log names no request: nothing was audited".into());
    }
    Ok(format!(
        "{WORD:?} marked, suggestions {suggestions:?}; {}",
        report.line()
    ))
}
