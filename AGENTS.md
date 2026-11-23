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

`vendor/welding/` is our fork of `welding` (MPL-2.0), outside the workspace members: keep its
upstream style, mark each change `wubble change`, list it in `vendor/welding/WUBBLE_CHANGES.md`;
test with `cargo test -p welding`. `vendor/i-slint-renderer-skia/` is Slint's Skia renderer with partial
rendering on its wgpu surface, under the same rules (`WUBBLE_CHANGES.md` there); tested through wubble-ui's
`partial_render` test. `SLINT_SKIA_PARTIAL_RENDERING=off|verify` switches it off or checks every frame.

## Commands

    cargo check --workspace        # first run downloads prebuilt CEF and Skia (rust-skia binaries)
    cargo build --workspace        # builds wubble-helper too; needed before cargo run
    cargo test --workspace
    cargo run -p wubble -- example.com   # the browser on a page
    cargo run -p wubble -- --smoke boot   # CEF boots and shuts down, no window
    cargo run -p wubble -- --shot-dir DIR --smoke gallery   # one PNG per shell surface; run by hand

Every other smoke, measurement and example, and the gallery's scenes: [docs/smokes.md](docs/smokes.md).
Window smokes never open on the desktop: prefix them with `scripts/headless.sh` (an invisible display with focus,
clipboard and GPU) and wrap them in `timeout`. macOS (app bundle, console loop, job runner):
[docs/macos.md](docs/macos.md). Releases and the updater: [docs/release.md](docs/release.md). Hand-run numbers: [docs/measurements.md](docs/measurements.md).

## Design fidelity

The UX source is the owner's design repo `xuckless/the-wallpaper-project`, directory `src/wubble`,
cloned beside this repo (see [ux/README.md](ux/README.md)). The owner keeps adding widgets there: `git pull` it
before UI work, look there first, and write a widget from scratch only when it has nothing (say so in your report).

Its web version (`browser/`, `core/`, `pages/`) is the truth for look, motion, effects and widget behaviour;
its `slint/` port is a head start only where it is line-for-line with the web. wubble's repo is the truth for
features and wording. `ux/pack/` is an older, superseded snapshot. Plan: [docs/fidelity/kit-plan.md](docs/fidelity/kit-plan.md);
tools: [tools/fidelity/README.md](tools/fidelity/README.md).

- **Compose only kit components.** UI is drawn only by `crates/wubble-ui/ui/kit/**`. A shell `.slint` file
  lays out kit components and binds models: no fills, borders, shadows, gradients, `Path`s, `animate`, font
  settings, raw `Text`, token groups (`Palette.`, `Glass.`, `Motion.`, ...) or literals.
- **Derive, never invent.** Every kit file starts with `// design: <file>:<lines> @<commit>` (the web source
  it translates, shaders included) or `// derived: <kit parts it composes>` (rules:
  [docs/fidelity/wubble-only.md](docs/fidelity/wubble-only.md)). Derived pieces go on the owner's sheet,
  [docs/fidelity/derived-review.md](docs/fidelity/derived-review.md); one needing a new primitive waits for the owner.
- **Values are generated.** Colours, lengths, durations, easings, springs, keyframes and icon paths come from
  `ui/kit/gen/*.slint` and `wubble_core::kit_gen`, written by `node tools/fidelity/extract.mjs`; never edit them
  by hand. When the clone moves: pull, rerun (`--check` says what is stale), commit the result.
- **The lock.** `crates/wubble-ui/tests/kit_lock.rs` enforces these rules against `kit_lock_baseline.txt`,
  which only shrinks: lower it with `KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock`; never raise it.
- **Look.** No scoring tool: shoot every surface you touched and the plain window (`--shot`, `--shot-dir`),
  compare each by eye with the web design, attach them to your report. What Slint cannot do:
  [docs/fidelity/deviations.md](docs/fidelity/deviations.md). The owner's eye is the final judge.

## Code principles

1. **Declarative first.** UI is a function of state: Slint bindings over models. Config, keybindings, policies and feature tables are data.
2. **No implicit state machines.** A lifecycle is one `enum` plus one pure `fn transition(state, event) -> (state, effects)`; no scattered booleans or callback ordering.
3. **Effects at the edge.** Transitions return effects as values; only `wubble-engine` and `wubble-gfx` execute them. `wubble-core` stays pure and unit-tested.
4. **One owner per state.** Each piece of state lives in one place; everything else derives from it.
5. **No duplicated code.** Reuse or extend before writing new. Per-platform code implements one shared trait. Two copies of a constant, token or table are duplication.

## Working rules

- **Copy first.** Look for a crate or a proven implementation (Chromium, CEF, Firefox, Zen, Brave, Servo, Zed) and port it. GPL-3.0-compatible licences only (MIT, BSD, Apache-2.0, MPL-2.0); each copy carries a source comment and a `THIRD_PARTY_NOTICES.md` entry.
- **No performance-dependent tests.** Nothing in `cargo test` or CI asserts timings, fps or throughput; performance is measured by hand on the owner's machines.
- **Sprint mode.** Breadth first to spec, each item reviewed for correctness and simplicity; hardening comes at the end of the build-out.
- Version pins move together: `slint`, `wgpu`, `welding` and `cef` must agree (wgpu 30, CEF 156).
- No CPU readback on the frame path; a fallback `OnPaint` frame is a bug to count and log. 120 Hz budget: 8.33 ms; glass degrades before animation drops a frame.
- Platforms: Linux (lead) and macOS built locally; Windows CI-only.
- New logic in `wubble-core` comes with unit tests. Run the checks above before handing work back.
- Nothing phones home by default: any request wubble itself makes needs a flag that is off by default.
- Keep [TODO.md](TODO.md) current (one line per item; done items are removed, `git log` has them); record decisions in [docs/decisions.md](docs/decisions.md), one line each.
- After any change to glass, the driver or a per-frame path: `--measure idle --seconds 4` must report 0 to 2 `slint_renders`.
- No `pgrep -f` wait loops (they match themselves and never end): run the command under `timeout`, or wait on a file.
- Don't edit `ux/pack/`; don't commit `target/` or machine-specific paths. Commit only when asked; one logical change per commit.

## Research

- [docs/research/renderer-roads.md](docs/research/renderer-roads.md): femtovg fork, Slint's Skia renderer, or our own UI (Skia chosen).
- [docs/research/cef-codec-build.md](docs/research/cef-codec-build.md): building CEF 156 with H.264, AAC and HLS, and pointing wubble at it.
- [docs/research/build-runners.md](docs/research/build-runners.md): where automated CEF builds could run, their cost, codec patents.
- [docs/research/apple-passwords.md](docs/research/apple-passwords.md): Apple Passwords and passkeys in a CEF browser on macOS.
- [docs/research/translate.md](docs/research/translate.md): on-device page translation options.
- [docs/research/ocr.md](docs/research/ocr.md): on-device OCR for live text in images.
- [docs/research/motion-audit.md](docs/research/motion-audit.md): every design motion against wubble (pre-kit audit).
- [docs/research/mcp-server.md](docs/research/mcp-server.md): wubble as an MCP server: Space-bound agent sessions, named secrets, airlocks (plan, after the release).
- [docs/research/agent-widgets.md](docs/research/agent-widgets.md): Ultra Reactivate's thinking orb (ported) and bot mascots (researched): drawing, cost, roads, where they go.
- [docs/research/extensions.md](docs/research/extensions.md): extensions on CEF 156 windowless: what works unpatched (loading, service workers, content scripts), the patches P1 to P4.
- [docs/research/tts.md](docs/research/tts.md): tiny-voca as an on-device voice for a Read aloud feature (shelved; not a screen reader).

## Batches

Roadmap items run as lanes through the owner's user-level `lane-batch` workflow (scope contract, implement,
caveman and adversarial reviews, refix, integrate): how, wubble's arguments and the rules learned, in
[docs/orchestration.md](docs/orchestration.md).
