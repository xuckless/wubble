//! CEF subprocess entry point (renderer, GPU, utility).
//!
//! Shape from cef-rs `examples/cefsimple/src/bin/cefsimple_helper.rs`
//! (tauri-apps/cef-rs, MIT OR Apache-2.0). welding's `run_subprocess` pins the
//! API version and hands CEF the same app the browser process uses.

use welding::{CefRuntime, CefSandboxMode};

/// Runs in every frame of every renderer (welding change 26): the password
/// autofill content script and the busy probes, one function expression that
/// calls both (a block whose value is that function, so neither script's
/// names reach the page's globals). Only autofill registers a receiver. It
/// lives here because the helper is the renderer; the browser sides are
/// `crates/wubble/src/host/autofill.rs` and `crates/wubble/src/host/busy.rs`.
const CONTEXT_SCRIPT: &str = concat!(
    "{\nconst autofill =\n",
    include_str!("autofill.js"),
    "\n;\nconst probes =\n",
    include_str!("probes.js"),
    "\n;\n(function (send, register) {\n",
    "  try { autofill(send, register); } catch (e) {}\n",
    "  try { probes(send); } catch (e) {}\n",
    "});\n}\n",
);

fn main() {
    #[cfg(target_os = "macos")]
    let _framework = load_framework();
    // Must match `SANDBOX` in wubble-engine/src/app.rs. Duplicated on purpose:
    // the helper stays tiny and cannot depend on wubble-engine (TODO.md, "CEF sandbox").
    let sandbox = CefSandboxMode::UnsandboxedTrustedContent;
    std::process::exit(CefRuntime::run_subprocess_with_script(
        &cef::args::Args::new(),
        sandbox,
        Some(CONTEXT_SCRIPT),
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
