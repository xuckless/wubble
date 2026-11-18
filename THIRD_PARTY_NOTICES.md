# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Tab lifecycle state model (Active/Frozen/Discarded, discard reasons), no code copied | `crates/wubble-core/src/lifecycle.rs` | Chromium `chrome/browser/resource_coordinator/lifecycle_unit_state.mojom`, `components/performance_manager/freezing/freezing_policy.cc` | BSD-3-Clause |
| Tab unloading rules (never unload audio/PiP/WebRTC tabs), no code copied | `crates/wubble-core/src/lifecycle.rs` | Firefox `browser/components/tabbrowser/TabUnloader.sys.mjs` | MPL-2.0 |
| Spring integrator, rubber band, stretch and easing constants (owner-authored design pack, ported to Rust) | `crates/wubble-core/src/motion.rs` | wubble `ux/pack/project/Components.dc.html:331-338`, `RoomToPlay.dc.html:99-106`, `Tokens.dc.html` Motion | Owner-authored (same repo); originally from "Ultra Reactivate", ownership to confirm |
| Tab selection after close, pinned-prefix index clamping, opener forgetting, no pinned tab in a group, no code copied | `crates/wubble-core/src/model.rs` | Chromium `chrome/browser/ui/tabs/tab_strip_model.cc` (`DetermineNewSelectedIndex`, `ConstrainInsertionIndex`) | BSD-3-Clause |
| Workspace/container data model (default container per workspace, container deletion reset, delete closes tabs, remembered selected tab), no code copied | `crates/wubble-core/src/model.rs` | Zen `src/zen/spaces/ZenSpaceManager.mjs` | MPL-2.0 |
