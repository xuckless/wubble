# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Tab lifecycle state model (Active/Frozen/Discarded, discard reasons), no code copied | `crates/wubble-core/src/lifecycle.rs` | Chromium `chrome/browser/resource_coordinator/lifecycle_unit_state.mojom`, `components/performance_manager/freezing/freezing_policy.cc` | BSD-3-Clause |
| Tab unloading rules (never unload audio/PiP/WebRTC tabs), no code copied | `crates/wubble-core/src/lifecycle.rs` | Firefox `browser/components/tabbrowser/TabUnloader.sys.mjs` | MPL-2.0 |
| Spring integrator, rubber band, stretch and easing constants (owner-authored design pack, ported to Rust) | `crates/wubble-core/src/motion.rs` | wubble `ux/pack/project/Components.dc.html:331-338`, `RoomToPlay.dc.html:99-106`, `Tokens.dc.html` Motion | Owner-authored (same repo); originally from "Ultra Reactivate", ownership to confirm |
