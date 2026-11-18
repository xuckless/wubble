# AGENTS.md

## What this is

wubble is a Chromium-based browser with a Slint shell, GPL-3.0-only. Chromium
renders web content off-screen; Slint composites it as GPU textures, so the whole
UI is one scene. Roadmap, decisions and risks live in [TODO.md](TODO.md) — read
its "Decided" table before proposing architecture.

## Layout

| Crate | Role | May depend on |
|---|---|---|
| `wubble-core` | Tabs, Spaces, sleep policy, suggestions. Pure logic | nothing heavy (no CEF, wgpu, Slint) |
| `wubble-gfx` | Native texture handle → `wgpu::Texture` → `slint::Image` | wgpu, welding, slint |
| `wubble-engine` | The only crate that names CEF types | core, gfx, cef |
| `wubble-ui` | Slint shell (`ui/*.slint`) | slint |
| `wubble` | Binary: wires the above | all |
| `wubble-helper` | CEF subprocess entry point; stays tiny | cef, welding (for its shared CEF app) |
| `wubble-sysmem` | OS readers: memory pressure, per-process memory and CPU | core, sysinfo (and the OS APIs) |

`vendor/welding/` is our fork of the `welding` crate (MPL-2.0), outside the
workspace members: keep its upstream style, mark each change `wubble change`
and list it in `vendor/welding/WUBBLE_CHANGES.md`; test with `cargo test -p welding`.

UX source of truth: the owner's design repo, cloned at
`~/Dev/wubble-wt/ref/the-wallpaper-project/src/wubble` (see `ux/README.md`). Match it, and
copy from its `slint/` port (tokens, physics, components, wgpu renderer) before writing UI
code. `ux/pack/` is an older, superseded snapshot.
The owner keeps adding widgets and behaviours to that repo: it is the bucket UI work is
adopted from. `git pull` the clone before UI work, look there first, and write a widget
from scratch only when the bucket has nothing for it (then say so in your report).

## Commands

    cargo check --workspace        # first run downloads prebuilt CEF
    cargo build --workspace        # builds wubble-helper too; needed before cargo run
    cargo test -p wubble-core
    cargo run -p wubble
    cargo run -p wubble -- --smoke boot   # CEF boots and shuts down, no window
    cargo run -p wubble -- --smoke frames=300   # window shows 300 zero-copy frames, exits; run by hand only
    cargo run -p wubble -- --smoke nav   # window navigates between two bundled pages, exits; run by hand only
    cargo run -p wubble -- --smoke tabs   # window opens, switches, sleeps, restores and closes tabs, exits; run by hand only
    cargo run -p wubble -- --smoke split   # window splits two bundled pages, checks each pane, unsplits, exits; run by hand only
    cargo run -p wubble -- --smoke page   # window drives a select popup, an alert, a context menu, find and zoom, exits; run by hand only
    cargo run -p wubble -- --smoke session   # two tabs saved, restarted and restored on a temp profile; two windows, exits; run by hand only
    cargo run -p wubble -- --smoke settings   # window opens wubble://settings, flips a setting through its toggle, reads the store back, exits; run by hand only
    cargo run -p wubble -- --smoke autofill   # loopback sign-in page, fake keyring: fill from the key menu, submit, save prompt, vault row; exits; run by hand only
    cargo run -p wubble-engine --example block_smoke   # windowless: the request-filter hook cancels a script and runs the commit script (a stand-in filter, not wubble-blocker), exits; run by hand only
    cargo run -p wubble -- --smoke pages   # opens history, bookmarks, downloads, passwords on a seeded profile, exits; run by hand only
    cargo run -p wubble -- example.com   # the browser on a page (the bundled one without a URL)
    cargo run -p wubble -- --profile DIR   # the browser on another profile (database + CEF cache)
    cargo run -p wubble -- --downloads DIR   # downloads go to DIR instead of the OS downloads folder

macOS runs CEF only from an app bundle (`ssh pixl-mac`, plain cargo there; rsync the
worktree without `target/` and `.git`; clippy needs `rustup component add clippy` once):

    cargo build --workspace && cargo run -p wubble-bundle   # writes target/debug/bundle/wubble.app
    target/debug/bundle/wubble.app/Contents/MacOS/wubble --smoke boot   # any wubble args; `open` fails over ssh
    cargo build --workspace --examples && cargo run -p wubble-bundle -- --exe target/debug/examples/frame_smoke
    target/debug/bundle/wubble.app/Contents/MacOS/wubble   # windowless zero-copy and colour check; re-bundle without --exe after
    cargo run -p wubble-engine --example frame_smoke   # the same check on Linux

Window smokes need a GUI session: over ssh they hang creating the window, so always
wrap them in `timeout`. Console smoke loop: log in at the Mac console as `pixlbot`
(fast user switching; the toolchain and `~/wubble-b6` are that user's), open Terminal,
`cd ~/wubble-b6 && export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH`, build the bundle
(above), then:

    for s in boot nav tabs session frames=120 split page block; do echo "== $s"; timeout 120 target/debug/bundle/wubble.app/Contents/MacOS/wubble --smoke $s > /tmp/wubble-$s.log 2>&1; echo "exit $?"; grep -E '^smoke|panicked|^wubble:' /tmp/wubble-$s.log; done; pgrep -fl MacOS/wubble

Each section ends `exit 0` with its `smoke:` lines (`frames=120`: `smoke: pixels ok` and
`smoke: glass ok`); `exit 124` is a hang killed by `timeout`; any other exit is a crash
(full output in `/tmp/wubble-<smoke>.log`). A `panicked` line about handling an event
while another is handled is the cue for a dispatch2 main-queue pump. `pgrep` prints nothing.

## Code principles

1. **Declarative first.** UI is a function of state: Slint bindings over models, no imperative widget poking. Config, keybindings, policies and feature tables are data.
2. **No implicit state machines.** Anything with a lifecycle is one `enum` of states plus one pure `fn transition(state, event) -> (state, effects)`. No behaviour may depend on combinations of scattered booleans or on callback ordering.
3. **Effects at the edge.** Transitions return effects as values; only `wubble-engine` and `wubble-gfx` execute them. `wubble-core` stays pure and unit-tested.
4. **One owner per state.** Each piece of state lives in one place; everything else derives from it.
5. **No duplicated code.** Reuse or extend an existing function, type or Slint component before writing a new one. Per-platform code implements one shared trait. Two copies of the same constant, token or table count as duplication.

## Working rules

- **Copy first.** Before writing anything non-trivial, look for an existing crate or a proven implementation in another browser/project (Chromium, CEF, Firefox, Zen, Brave, Servo, Zed) and use or port it. Licence must be GPL-3.0-compatible (MIT, BSD, Apache-2.0, MPL-2.0); nothing proprietary. Copied or ported code carries a source comment and an entry in `THIRD_PARTY_NOTICES.md`.
- **No performance-dependent tests.** Nothing in `cargo test` or CI may assert timings, fps or throughput. Performance is measured by hand on the owner's Linux machine or the owner's macOS machine.
- **Sprint mode.** Build breadth first to spec, each item reviewed for correctness and simplicity; a hardening pass over the whole product comes at the end of the build-out.
- Version pins move together: `slint`, `wgpu`, `welding` and `cef` must agree (wgpu 30, CEF 156). Do not bump one alone.
- No CPU readback on the frame path. A fallback `OnPaint` frame is a bug to count and log.
- 120 Hz budget: 8.33 ms per frame. Glass quality degrades before animation drops a frame.
- Platforms: Linux (lead) and macOS are built locally; Windows is CI-only.
- New logic in `wubble-core` comes with unit tests. Run the commands above before handing work back.
- Nothing phones home by default. Any new network request made by wubble itself needs a flag that is off by default.
- Keep `TODO.md` current: tick items you finish, record new decisions in its "Decided" table.
- Don't edit `ux/pack/`; don't commit `target/` or machine-specific paths.
- Commit only when asked; one logical change per commit.

## Orchestrating a batch

`.claude/workflows/lane-batch.js` runs a batch of roadmap items in parallel lanes:
(research) → implement → correctness review + simplicity review → refix, then merges
the lanes into one integration branch and runs the full check round.

- A lane is a git worktree under `~/Dev/wubble-wt/` on its own branch; items in a lane run in order, lanes run in parallel. Put items that touch the same files in the same lane.
- The lead creates the integration branch and the lane worktrees, launches the workflow, then reviews the integration diff before merging to `main` and pushing.
- Each stage uses the cheapest model that can do it: Sonnet medium by default, Opus medium for items marked `hard`, per-stage overrides (including Haiku) via the item's `models`.
- Research runs for every item unless it sets `research: false`, and is copy-first: it looks for code to reuse before designing anything.
- Lanes build through `~/Dev/wubble-wt/cargo.sh`: one build at a time (a lock), each worktree with its own target directory, so a lane never picks up another lane's crates.
