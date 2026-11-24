# Agent widgets

Shelved by the owner (2026-11-23): "agent widgets can get shelved for now. Thinking orb I'll build it in free time".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

Research: [docs/research/agent-widgets.md](../../docs/research/agent-widgets.md). The orb's features belong to the feel pass too ([feel-pass.md](feel-pass.md)).

## From the old roadmap
- [ ] Wire the thinking orb (in the kit, gallery only so far): the agent mark while a call is in flight, the activity page header, the approval sheet; after the owner reads the proposal in `docs/research/agent-widgets.md` section 5
- [ ] The mascots are researched, not ported: their fabric and plastic shading is CPU-baked with per-frame readbacks in the web version, so a faithful port needs a GPU rewrite (`docs/research/agent-widgets.md` section 3.3); owner decides whether to do it
- [ ] Thinking orb: the `gravity` cursor warp is not ported; one moving 64 px orb costs a whole-window redraw per frame (about 8 to 9 % of a core at 60 Hz, hand run): tied to the shell's redraw work in [01-optimization.md](../upcoming/01-optimization.md)
- [ ] The agent glyph: the design's `sparkle` stands in (one alias, `Glyph.agent`) and reads as a crosshair at badge size; the owner's glyph, a mascot or the orb, then one swap

## Build-out features (2026-11-22)

## Wire the thinking orb into the shell: agent mark, activity header, approval sheet (`orb-shell-wiring`)

Key: `orb-shell-wiring` · engine patch: no · state: not written.

TODO line 40. The orb is ported (`wubble_core::orb`: `Orb`, `Life`, `transition`; GPU painter `wubble_gfx::orb`; per-window stage in `crates/wubble/src/orbs.rs`) but is shown only by the gallery specimen (`kit/specimen/agent-widgets.slint`). Wire it into the places in docs/research/agent-widgets.md section 5: (1) the agent mark on a driven tab's row and on its Space header, 16 px, animated while that session (`wubble_core::agent` Session) has a call in flight, else a still frame; (2) the agent activity page's header, 32 px, while any session runs; (3) the approval sheet's title, 20 px, in the listening preset while the session is `AwaitingApproval`. Each place is a kit component property (an `Image` slot already in `kit/chrome/agent-mark.slint`, the agent sheet and the activity page kit parts): add an `orb` image input to those kit parts (derived, add a line to docs/fidelity/derived-review.md). orbs.rs gains cells keyed by place (tab id, page, sheet), created and dropped as models change, driven by the session state through `LifeEvent`. Only moving orbs redraw (orbs.rs `frame`). Reduced motion shows the still frame. Keep the existing kit lock rules (no drawing in shell .slint files). The owner's go on the proposal is still open; build it so each place can be switched off in one table.

## Thinking orb: port the gravity cursor warp (`orb-gravity-warp`)

Key: `orb-gravity-warp` · engine patch: no · state: not written.

TODO line 42. The web ThinkingOrb has a `gravity` prop (`ThinkingOrb.tsx`, `gravity.ts` in the design repo `xuckless/the-wallpaper-project` src/wubble): it warps the dots toward the pointer through a sprite. wubble's port (`crates/wubble-core/src/orb/*.rs`, painter `crates/wubble-gfx/src/orb.rs`, cells in `crates/wubble/src/orbs.rs`) leaves it out (listed in docs/fidelity/deviations.md). Port it: a pure `gravity` module in wubble-core/orb that takes the pointer position relative to the orb (logical px, or None) and returns each mark's offset, with the design's falloff and constants exactly (put the constants through `tools/fidelity/extract.mjs` into kit_gen if they are design values). Add a `pointer` input to `Orb`/`Life`, so a pointer moving over an orb with gravity on makes `Life` say it moves (it redraws only while the warp changes, then settles). Feed the pointer from the specimen's TouchArea (`kit/specimen/agent-widgets.slint`) through `orbs.rs`. Add unit tests for the pure warp in wubble-core (new core logic needs them under AGENTS.md; they are no performance tests). Remove the deviation line.

