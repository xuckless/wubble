# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Message-pump re-entrancy guard (`PumpGuard`) | `crates/wubble-engine/src/app.rs` | CEF `tests/shared/browser/main_message_loop_external_pump.cc` (`PerformMessageLoopWork`), chromiumembedded/cef | BSD-3-Clause |
| Helper `main` shape (args, execute_process, exit code) | `crates/wubble-helper/src/main.rs` | cef-rs `examples/cefsimple/src/bin/cefsimple_helper.rs`, tauri-apps/cef-rs | MIT OR Apache-2.0 |
