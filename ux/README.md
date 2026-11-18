# UX pack

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
