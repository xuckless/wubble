# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| Tab lifecycle state model (Active/Frozen/Discarded, discard reasons), no code copied | `crates/wubble-core/src/lifecycle.rs` | Chromium `chrome/browser/resource_coordinator/lifecycle_unit_state.mojom`, `components/performance_manager/freezing/freezing_policy.cc` | BSD-3-Clause |
| Tab unloading rules (never unload audio/PiP/WebRTC tabs), no code copied | `crates/wubble-core/src/lifecycle.rs` | Firefox `browser/components/tabbrowser/TabUnloader.sys.mjs` | MPL-2.0 |
