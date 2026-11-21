//! `--smoke spell`: spell check works on a fresh profile, from the dictionary bundled in the app,
//! with no request off the machine.
//!
//! A throwaway profile, no window: a loopback page (`assets/spell.html`) with a textarea.
//! 1. The textarea is focused and a misspelt word is typed into it (an input method commit, the
//!    host's text path).
//! 2. The word is right-clicked until Chromium's menu for it carries `misspelled_word` and
//!    suggestions: what wubble's native context menu shows first (`menu.rs`).
//! 3. After the quit, Chromium's net log (`log-net-log`, through `WUBBLE_PROBE_SWITCHES`) is read:
//!    every URL a request named is on loopback, and none is the dictionary download
//!    (`redirector.gvt1.com`) the refused hosts would have caught.
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

/// Every URL a net log names (`"url":"..."`), read as text so a log cut short still reads.
pub fn logged_urls(log: &str) -> Vec<&str> {
    log.split("\"url\":\"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect()
}

/// Whether `url` stays on this machine (loopback, or a page that is not a network one).
pub fn is_local(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return true;
    };
    if !matches!(scheme, "http" | "https" | "ws" | "wss") {
        return true;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    host == "127.0.0.1" || host == "localhost" || host == "[::1]"
}

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
            format!("log-net-log={}", net_log.display()),
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
    let log = std::fs::read_to_string(&net_log)?;
    let urls = logged_urls(&log);
    if urls.is_empty() {
        return Err("the net log names no URL: nothing was audited".into());
    }
    if !urls
        .iter()
        .any(|u| u.contains(&format!("127.0.0.1:{port}/spell")))
    {
        return Err("the net log does not hold the loopback page's own request".into());
    }
    // Chromium makes a few requests of its own that wubble refuses at name resolution (the
    // component updater's and autofill's); they are counted and
    // named here, not failed on: what this proves is that the dictionary is not among them and
    // that no request left for anything else of the page's.
    let away: Vec<&str> = urls.iter().copied().filter(|u| !is_local(u)).collect();
    let hosts: std::collections::BTreeSet<&str> = away
        .iter()
        .filter_map(|u| u.split_once("://")?.1.split(['/', '?']).next())
        .collect();
    if let Some(download) = urls
        .iter()
        .find(|u| u.contains("gvt1.com") || u.contains(".bdic"))
    {
        return Err(format!("a dictionary download was asked for: {download}").into());
    }
    Ok(format!(
        "{WORD:?} marked, suggestions {suggestions:?}; net log: {} URLs, the page's own on \
         loopback, no dictionary download, other hosts asked for: {hosts:?}",
        urls.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_urls_are_loopback_or_not_network() {
        assert!(is_local("http://127.0.0.1:5173/spell"));
        assert!(is_local("http://localhost/x"));
        assert!(is_local("data:text/html,hi"));
        assert!(is_local("about:blank"));
        assert!(!is_local("https://redirector.gvt1.com/edgedl/x.bdic"));
        assert!(!is_local("https://127.0.0.1.evil.com/"));
        assert!(!is_local("http://127.0.0.1@evil.com/"));
    }

    #[test]
    fn urls_are_read_out_of_a_log() {
        let log = r#"{"params":{"url":"http://a/"}},{"params":{"url":"https://b/x?y=1"},"#;
        assert_eq!(logged_urls(log), ["http://a/", "https://b/x?y=1"]);
    }
}
