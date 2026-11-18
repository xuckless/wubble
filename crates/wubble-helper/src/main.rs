//! CEF subprocess entry point (renderer, GPU, utility).
//!
//! Shape from cef-rs `examples/cefsimple/src/bin/cefsimple_helper.rs`
//! (tauri-apps/cef-rs, MIT OR Apache-2.0). welding's `run_subprocess` pins the
//! API version and hands CEF the same app the browser process uses.

use welding::{CefRuntime, CefSandboxMode};

/// The content scripts, each on its own channel of welding's one context
/// script (change 26). The browser side splits the channel off in
/// wubble-engine `page::FrameChannel`; the names must match its table.
/// `autofill.js`: password autofill (browser side `crates/wubble/src/autofill.rs`).
const AUTOFILL: (&str, &str) = ("autofill", include_str!("autofill.js"));
/// `zoom.js`: Ctrl+wheel zoom, when the page leaves the wheel alone.
const ZOOM: (&str, &str) = ("zoom", include_str!("zoom.js"));
/// `clipboard.js`: the page clipboard through the window, only when the browser
/// passed [`CLIPBOARD_SWITCH`] (browser side `crates/wubble/src/clipboard.rs`).
const CLIPBOARD: (&str, &str) = ("clipboard", include_str!("clipboard.js"));
/// Must match `CLIPBOARD_SWITCH` in wubble-engine/src/app.rs. Duplicated on purpose,
/// as `sandbox` below: the helper cannot depend on wubble-engine.
const CLIPBOARD_SWITCH: &str = "wubble-clipboard-bridge";

/// One context script running `scripts`, each called with its own channel:
/// `send(text)` posts `name\ntext`, and a browser message `name\ntext` reaches
/// the function that channel registered. The dispatcher captures what it uses
/// before page scripts run.
fn compose(scripts: &[(&str, &str)]) -> String {
    let parts: String = scripts
        .iter()
        .map(|(name, script)| format!("[{name:?}, {}],\n", script.trim().trim_end_matches(';')))
        .collect();
    format!(
        "(function (send, register) {{\n\
         'use strict';\n\
         const handlers = new Map();\n\
         const get = Map.prototype.get, set = Map.prototype.set, apply = Reflect.apply;\n\
         const indexOf = String.prototype.indexOf, slice = String.prototype.slice;\n\
         register((text) => {{\n\
           const at = apply(indexOf, text, ['\\n']);\n\
           const fn = at < 0 ? undefined : apply(get, handlers, [apply(slice, text, [0, at])]);\n\
           if (fn) fn(apply(slice, text, [at + 1]));\n\
         }});\n\
         for (const [name, script] of [\n{parts}]) {{\n\
           try {{ script((text) => send(name + '\\n' + text), (fn) => apply(set, handlers, [name, fn])); }} catch (e) {{}}\n\
         }}\n\
         }})"
    )
}

fn main() {
    #[cfg(target_os = "macos")]
    let _framework = load_framework();
    // Must match `SANDBOX` in wubble-engine/src/app.rs. Duplicated on purpose:
    // the helper stays tiny and cannot depend on wubble-engine (TODO.md, "CEF sandbox").
    let sandbox = CefSandboxMode::UnsandboxedTrustedContent;
    let clipboard = std::env::args().any(|a| a.strip_prefix("--") == Some(CLIPBOARD_SWITCH));
    let scripts: Vec<_> = [AUTOFILL, ZOOM]
        .into_iter()
        .chain(clipboard.then_some(CLIPBOARD))
        .collect();
    // Built once per process and kept for its life.
    let script: &'static str = Box::leak(compose(&scripts).into_boxed_str());
    std::process::exit(CefRuntime::run_subprocess_with_script(
        &cef::args::Args::new(),
        sandbox,
        Some(script),
    ));
}

/// macOS helpers live in `wubble.app/Contents/Frameworks/wubble Helper*.app`
/// and must load the CEF framework before any CEF call; the loader keeps it
/// loaded while it lives. Linux links libcef directly and needs nothing.
///
/// From cef-rs `cef/src/library_loader.rs` and its use in the cefsimple helper
/// (tauri-apps/cef-rs, MIT OR Apache-2.0), as wgpu-weld's
/// `demo-weld-mac/src/helper.rs` does (MPL-2.0).
#[cfg(target_os = "macos")]
fn load_framework() -> cef::library_loader::LibraryLoader {
    let exe = std::env::current_exe().expect("the helper knows its own path");
    let loader = cef::library_loader::LibraryLoader::new(&exe, true);
    assert!(loader.load(), "cannot load the CEF framework");
    loader
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_script_is_one_function_expression_on_its_channel() {
        let script = compose(&[AUTOFILL, ZOOM, CLIPBOARD]);
        assert!(script.starts_with("(function (send, register) {"));
        assert!(script.trim_end().ends_with("})"));
        assert!(script.contains("[\"autofill\", "));
        assert!(script.contains("[\"clipboard\", "));
        assert!(script.contains("[\"zoom\", "));
        // No script ends in a statement terminator inside the array.
        assert!(!script.contains("});,"));
    }

    #[test]
    fn the_clipboard_runs_only_when_asked() {
        assert_eq!(CLIPBOARD_SWITCH, "wubble-clipboard-bridge");
        assert!(!compose(&[AUTOFILL]).contains("\"clipboard\""));
    }
}
