# Design fidelity

Moved out of [AGENTS.md](../../AGENTS.md) on 2026-11-23, unchanged. Read it before any UI work: a shell `.slint`
file, a kit component, a gallery scene or a shot.

The UX source is the owner's design repo `xuckless/the-wallpaper-project`, directory `src/wubble`,
cloned beside this repo (see [ux/README.md](../../ux/README.md)). The owner keeps adding widgets there: `git pull` it
before UI work, look there first, and write a widget from scratch only when it has nothing (say so in your report).

Its web version (`browser/`, `core/`, `pages/`) is the truth for look, motion, effects and widget behaviour;
its `slint/` port is a head start only where it is line-for-line with the web. wubble's repo is the truth for
features and wording. `ux/pack/` is an older, superseded snapshot. Plan: [docs/fidelity/kit-plan.md](../fidelity/kit-plan.md);
tools: [tools/fidelity/README.md](../../tools/fidelity/README.md).

- **Compose only kit components.** UI is drawn only by `crates/wubble-ui/ui/kit/**`. A shell `.slint` file
  lays out kit components and binds models: no fills, borders, shadows, gradients, `Path`s, `animate`, font
  settings, raw `Text`, token groups (`Palette.`, `Glass.`, `Motion.`, ...) or literals.
- **Derive, never invent.** Every kit file starts with `// design: <file>:<lines> @<commit>` (the web source
  it translates, shaders included) or `// derived: <kit parts it composes>` (rules:
  [docs/fidelity/wubble-only.md](../fidelity/wubble-only.md)). Derived pieces go on the owner's sheet,
  [docs/fidelity/derived-review.md](../fidelity/derived-review.md); one needing a new primitive waits for the owner.
- **Values are generated.** Colours, lengths, durations, easings, springs, keyframes and icon paths come from
  `ui/kit/gen/*.slint` and `wubble_core::kit_gen`, written by `node tools/fidelity/extract.mjs`; never edit them
  by hand. When the clone moves: pull, rerun (`--check` says what is stale), commit the result.
- **The lock.** `crates/wubble-ui/tests/kit_lock.rs` enforces these rules against `kit_lock_baseline.txt`,
  which only shrinks: lower it with `KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock`; never raise it.
- **Look.** No scoring tool: shoot every surface you touched and the plain window (`--shot`, `--shot-dir`),
  compare each by eye with the web design, attach them to your report. What Slint cannot do:
  [docs/fidelity/deviations.md](../fidelity/deviations.md). The owner's eye is the final judge.
