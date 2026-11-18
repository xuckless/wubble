# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Helper `main` shape (args, execute_process, exit code) | `crates/wubble-helper/src/main.rs` | cef-rs `examples/cefsimple/src/bin/cefsimple_helper.rs`, tauri-apps/cef-rs | MIT OR Apache-2.0 |
| Slint's wgpu device descriptor (`default_device_descriptor`) and `poll_once` | `crates/wubble-gfx/src/import.rs` | Slint 1.19.0, `internal/core/graphics/wgpu_30.rs` (crate i-slint-core), slint-ui/slint | GPL-3.0-only (one of Slint's licence options) |
| Fullscreen-triangle blit shader and pipeline | `crates/wubble-gfx/src/import.rs` (`FULLSCREEN_VS_WGSL`, `BLIT_FS_WGSL`, `Blitter`; the vertex stage and pipeline shape are reused by `glass/pass.rs`) | wgpu `examples/features/src/mipmap/blit.wgsl` and `mod.rs`, gfx-rs/wgpu | MIT OR Apache-2.0 |
| Tab lifecycle state model (Active/Frozen/Discarded, discard reasons), no code copied | `crates/wubble-core/src/lifecycle.rs` | Chromium `chrome/browser/resource_coordinator/lifecycle_unit_state.mojom`, `components/performance_manager/freezing/freezing_policy.cc` | BSD-3-Clause |
| Tab unloading rules (never unload audio/PiP/WebRTC tabs), no code copied | `crates/wubble-core/src/lifecycle.rs` | Firefox `browser/components/tabbrowser/TabUnloader.sys.mjs` | MPL-2.0 |
| Spring integrator, rubber band, stretch and easing constants (owner-authored design pack, ported to Rust) | `crates/wubble-core/src/motion.rs` | wubble `ux/pack/project/Components.dc.html:331-338`, `RoomToPlay.dc.html:99-106`, `Tokens.dc.html` Motion | Owner-authored (same repo); originally from "Ultra Reactivate", ownership to confirm |
| Tab selection after close, pinned-prefix index clamping, opener forgetting, no pinned tab in a group, no code copied | `crates/wubble-core/src/model.rs` | Chromium `chrome/browser/ui/tabs/tab_strip_model.cc` (`DetermineNewSelectedIndex`, `ConstrainInsertionIndex`) | BSD-3-Clause |
| Workspace/container data model (default container per workspace, container deletion reset, delete closes tabs, remembered selected tab), no code copied | `crates/wubble-core/src/model.rs` | Zen `src/zen/spaces/ZenSpaceManager.mjs` | MPL-2.0 |
| Fredoka variable font (`Fredoka[wdth,wght].ttf`, renamed `Fredoka-Variable.ttf`, unmodified) | `crates/wubble-ui/ui/fonts/` (licence: `OFL-Fredoka.txt`) | google/fonts `ofl/fredoka` @ a60a77e14f28abd4ef243a1b5dfc48df0cec5205 | SIL OFL 1.1 |
| Geist variable font (`Geist[wght].ttf`, renamed `Geist-Variable.ttf`, unmodified) | `crates/wubble-ui/ui/fonts/` (licence: `OFL-Geist.txt`) | vercel/geist-font `fonts/Geist/variable` @ v1.9.2 | SIL OFL 1.1 |
| Geist Mono variable font (`GeistMono[wght].ttf`, renamed `GeistMono-Variable.ttf`, unmodified) | `crates/wubble-ui/ui/fonts/` (licence: `OFL-Geist.txt`) | vercel/geist-font `fonts/GeistMono/variable` @ v1.9.2 | SIL OFL 1.1 |
| Key handler (Up/Down/Escape) and scroll-into-view logic, adapted in Slint | `crates/wubble-ui/ui/components/command-bar.slint` | slint-ui/slint 1.19.0 (i-slint-compiler) `widgets/common/combobox-base.slint` (popup-key-handler), `listview.slint` (bring-into-view) | GPL-3.0-only (Slint) |
| History schema design (urls/visits columns, typed/visit counts); no code copied | `crates/wubble-store/sql/001_init.sql`, `src/history.rs` | Chromium `components/history/core/browser/url_database.cc`, `visit_database.cc` | BSD-3-Clause |
| History/bookmarks schema design and frecency recency buckets (100/70/50/30/10); no code copied | `crates/wubble-store/sql/001_init.sql`, `src/history.rs` | Firefox Places `toolkit/components/places/nsPlacesTables.h` | MPL-2.0 |
| Resource-type to filter-type table | `crates/wubble-blocker/src/types.rs` | brave-core `components/brave_shields/content/browser/ad_block_request_type_util.cc` (`ToAdBlockRequestType`) | MPL-2.0 |
| Per-frame hook idiom (`set_rendering_notifier`, set properties in `BeforeRendering`, then `request_redraw`) | `crates/wubble-ui/src/driver.rs` | slint-ui/slint v1.19.0 `examples/wgpu_texture/lib.rs` | MIT |
| Sidebar seam and compact-mode behaviour (held spring toward rubber-banded pointer, release spring toward clamp) | `crates/wubble-core/src/shell.rs` | wubble `ux/pack/project/RoomToPlay.dc.html:91,99-106,118-132,148-157` | Owner-authored (same repo); originally from "Ultra Reactivate", ownership to confirm |
| Dual-Kawase down (5-tap) and up (8-tap) blur shaders, `halfpixel` convention | `crates/wubble-gfx/src/glass/kawase.wgsl` (`fs_down`, `fs_up`) | KWin `src/plugins/blur/shaders/downsample.frag`, `upsample.frag`, `src/plugins/blur/blur.cpp` (KDE/kwin) | GPL-2.0-or-later |
| WGSL port of the dual-Kawase shaders, `kawase_level_size`, uniform layout test | `crates/wubble-gfx/src/glass/kawase.wgsl`, `glass/plan.rs` | backdrop-blur-wgpu 0.2.0 `src/shaders/downsample.wgsl`, `upsample.wgsl`, `src/lib.rs`, `src/uniforms.rs` (abdu-benayad/backdrop-blur) | MIT OR Apache-2.0 |
| Dual-Kawase offset/expand table per iteration (`KWIN_OFFSETS`) | `crates/wubble-gfx/src/glass/plan.rs` | KWin `src/plugins/blur/blur.cpp` `initBlurStrengthValues` (KDE/kwin) | GPL-2.0-or-later |
| Saturate colour matrix (published constants, no code copied) | `crates/wubble-gfx/src/glass/plan.rs` (`saturation_matrix`) | W3C Filter Effects Module Level 1, `feColorMatrix type="saturate"` | W3C specification (numbers only) |
