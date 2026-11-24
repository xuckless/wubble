# AGENTS.md

## What this is

wubble is a Chromium-based browser with a Slint shell, GPL-3.0-only. Chromium renders web content
off-screen; Slint composites it as GPU textures, so the whole UI is one scene. Roadmap and risks:
[TODO.md](TODO.md). Decisions: [docs/decisions.md](docs/decisions.md), read before proposing architecture.

## Layout

| Crate | Role | May depend on |
|---|---|---|
| `wubble-core` | Tabs, Spaces, sleep policy, suggestions. Pure logic | nothing heavy (no CEF, wgpu, Slint) |
| `wubble-gfx` | Native texture handle → `wgpu::Texture` → `slint::Image` | wgpu, welding, slint |
| `wubble-engine` | The only crate that names CEF types | core, gfx, cef |
| `wubble-ui` | Slint shell (`ui/*.slint`) | slint |
| `wubble-mcp` | MCP protocol for agent sessions: framing, tool table, parsed calls (`AgentCall`). Pure, no I/O | core |
| `wubble-ax` | A page's accessibility tree (CEF's values) mapped to AccessKit. Pure | accesskit |
| `wubble` | Binary: wires the above | all |
| `wubble-helper` | CEF subprocess entry point; stays tiny | cef, welding (for its shared CEF app) |
| `wubble-sysmem` | OS readers: memory pressure, per-process memory and CPU | core, sysinfo (and the OS APIs) |
| `wubble-release` | Release tool: update key, signing, the update manifest (CI builds only this) | core |

`vendor/welding/` and `vendor/i-slint-renderer-skia/` are our forks, with their own rules:
[docs/agents/vendor.md](docs/agents/vendor.md).

## Commands

    cargo check --workspace        # first run downloads prebuilt CEF and Skia (rust-skia binaries)
    cargo build --workspace        # builds wubble-helper too; needed before cargo run
    cargo test --workspace
    cargo run -p wubble -- example.com   # the browser on a page
    cargo run -p wubble -- --smoke boot   # CEF boots and shuts down, no window
    cargo run -p wubble -- --shot-dir DIR --smoke gallery   # one PNG per shell surface; run by hand

Every other smoke, measurement and example, and the gallery's scenes: [docs/smokes.md](docs/smokes.md).
Window smokes never open on the desktop: prefix them with `scripts/headless.sh` (an invisible display with focus,
clipboard and GPU) and wrap them in `timeout`.

## Code principles

1. **Declarative first.** UI is a function of state: Slint bindings over models. Config, keybindings, policies and feature tables are data.
2. **No implicit state machines.** A lifecycle is one `enum` plus one pure `fn transition(state, event) -> (state, effects)`; no scattered booleans or callback ordering.
3. **Effects at the edge.** Transitions return effects as values; only `wubble-engine` and `wubble-gfx` execute them. `wubble-core` stays pure and unit-tested.
4. **One owner per state.** Each piece of state lives in one place; everything else derives from it.
5. **No duplicated code.** Reuse or extend before writing new. Per-platform code implements one shared trait. Two copies of a constant, token or table are duplication.
6. **Surfaces are store rows.** Every shell surface is a row in `wubble_core::overlay::ROWS`; its visibility is decided by the store, never by Slint alone (no `has-hover` feeding `shown`, no shell `Timer`, no Slint-written `*-open`); `tests/surface_lock.rs` holds the count down.

## Working rules

- **Copy first.** Look for a crate or a proven implementation (Chromium, CEF, Firefox, Zen, Brave, Servo, Zed) and port it. GPL-3.0-compatible licences only (MIT, BSD, Apache-2.0, MPL-2.0); each copy carries a source comment and a `THIRD_PARTY_NOTICES.md` entry.
- **No performance-dependent tests.** Nothing in `cargo test` or CI asserts timings, fps or throughput; performance is measured by hand on the owner's machines.
- **Sprint mode.** Breadth first to spec, each item reviewed for correctness and simplicity; hardening comes at the end of the build-out.
- **UI is drawn only by kit components** (`crates/wubble-ui/ui/kit/**`): every UI task reads and follows
  [docs/agents/design-fidelity.md](docs/agents/design-fidelity.md) first (the design source, derive never invent,
  generated values, the kit lock, shots compared by eye).
- Version pins move together: `slint`, `wgpu`, `welding` and `cef` must agree (wgpu 30, CEF 156).
- No CPU readback on the frame path; a fallback `OnPaint` frame is a bug to count and log. 120 Hz budget: 8.33 ms; glass degrades before animation drops a frame.
- Platforms: Linux (lead) and macOS, built locally and on CI; Windows parked (its code kept, neither built nor
  released; TODO.md "Windows (parked)").
- New logic in `wubble-core` comes with unit tests. Run the checks above before handing work back.
- Nothing phones home by default: any request wubble itself makes needs a flag that is off by default.
- Keep [TODO.md](TODO.md) current (one line per item; done items are removed, `git log` has them); record decisions in [docs/decisions.md](docs/decisions.md), one line each.
- After any change to glass, the driver or a per-frame path: `--measure idle --seconds 4` must report 0 to 2 `slint_renders`.
- No `pgrep -f` wait loops (they match themselves and never end): run the command under `timeout`, or wait on a file.
- Don't edit `ux/pack/`; don't commit `target/` or machine-specific paths. Commit only when asked; one logical change per commit.

## Read when the task touches it

- [docs/agents/design-fidelity.md](docs/agents/design-fidelity.md): any UI work (shell `.slint`, kit, gallery, shots): the design repo, the kit rules, the lock, the look.
- [docs/agents/vendor.md](docs/agents/vendor.md): any change under `vendor/` (welding, the Skia renderer).
- [docs/agents/research.md](docs/agents/research.md): the research documents, one line each; read the one for your area before designing in it.
- [docs/smokes.md](docs/smokes.md): every smoke, measurement and example, the gallery's scenes.
- [docs/macos.md](docs/macos.md): macOS (app bundle, console loop, job runner).
- [docs/release.md](docs/release.md): releases and the updater.
- [docs/measurements.md](docs/measurements.md): hand-run numbers.
- [docs/orchestration.md](docs/orchestration.md): batches. Roadmap items run as lanes through the owner's user-level
  `lane-batch` workflow (scope contract, implement, caveman and adversarial reviews, refix, integrate): how, wubble's
  arguments and the rules learned.
