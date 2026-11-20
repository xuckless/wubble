# UX source

**Source of truth:** the owner's design repo `xuckless/the-wallpaper-project`, directory
`src/wubble` (private). Clone it beside this repo (the tools find it through `$WUBBLE_DESIGN`, see
[tools/fidelity/README.md](../tools/fidelity/README.md)). Read its
`README.md` (philosophy, motion tokens, shaders, structure) and `slint/README.md` (the
Slint/Rust/WGSL port and what is 1:1). wubble's UX is built to match it; copy from its
`slint/` port rather than re-deriving. It is the owner's own code (as is Ultra Reactivate,
which it draws on), so there is no licence obstacle to copying.

Ignore its marketing copy. It does not contain every wubble feature: build the missing
ones in the same style.

## Superseded: canvas snapshot

`pack/project/` is a read-only snapshot of the "Wubble" design canvas
(https://claude.ai/artifact/EeKe5MU4HQFq42QppMfKPU). The canvas is the source of
truth for *style*; re-pull the snapshot when it changes, don't edit it here.

**Guideline, not spec.** Take colour, glass, type and motion feel from it. Do not
treat its layouts as final: it was drawn without knowledge of the full feature
set, so layout and feature placement are decided in wubble itself.

| Board | Contents |
|---|---|
| `Main.dc.html` | Brand: logo, colour (dark first), type |
| `Tokens.dc.html` | Glass tiers, radius, spacing, motion constants, 120 Hz budget |
| `Browser.dc.html` | Browser window: sidebar, toolbar, command bar, stretchy seam |
| `Components.dc.html` | Interactive controls and their spring behaviour |
| `RoomToPlay.dc.html` | Inspector with the rubber-string seam |
| `Icons.dc.html` | App icons per platform |

The spring/seam maths lives in each board's `<script type="text/x-dc">` block and
is the reference when porting motion to Rust.
