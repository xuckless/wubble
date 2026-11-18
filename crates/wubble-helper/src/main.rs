//! CEF subprocess entry point (renderer, GPU, utility).
//!
//! Shape from cef-rs `examples/cefsimple/src/bin/cefsimple_helper.rs`
//! (tauri-apps/cef-rs, MIT OR Apache-2.0). welding's `run_subprocess` pins the
//! API version and hands CEF the same app the browser process uses.

use welding::{CefRuntime, CefSandboxMode};

fn main() {
    // Must match `SANDBOX` in wubble-engine/src/app.rs. Duplicated on purpose:
    // the helper stays tiny and cannot depend on wubble-engine (TODO.md, "CEF sandbox").
    let sandbox = CefSandboxMode::UnsandboxedTrustedContent;
    std::process::exit(CefRuntime::run_subprocess(&cef::args::Args::new(), sandbox));
}
