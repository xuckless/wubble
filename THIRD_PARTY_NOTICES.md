# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Helper `main` shape (args, execute_process, exit code) | `crates/wubble-helper/src/main.rs` | cef-rs `examples/cefsimple/src/bin/cefsimple_helper.rs`, tauri-apps/cef-rs | MIT OR Apache-2.0 |
| Slint's wgpu device descriptor (`default_device_descriptor`) and `poll_once` | `crates/wubble-gfx/src/import.rs` | Slint 1.19.0, `internal/core/graphics/wgpu_30.rs` (crate i-slint-core), slint-ui/slint | GPL-3.0-only (one of Slint's licence options) |
| Fullscreen-triangle blit shader and pipeline | `crates/wubble-gfx/src/import.rs` (`BLIT_WGSL`, `Blitter`) | wgpu `examples/features/src/mipmap/blit.wgsl` and `mod.rs`, gfx-rs/wgpu | MIT OR Apache-2.0 |
