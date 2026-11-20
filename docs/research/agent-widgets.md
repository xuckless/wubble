# Agent widgets: the thinking orb and the agentic mascots

Question: How do Ultra Reactivate's thinking orb and its bot mascots draw, what do they cost, and how does each come into wubble's kit?
Status: Orb ported (kit `ThinkingOrb`, gallery scenes); mascots researched, not ported (section 3.3).

Date: 2026-11-20. Sources: the owner's design repo `xuckless/the-wallpaper-project` @ 598f814,
`src/ultra-reactivate/vendor/thinking-orbs` and `vendor/bot-avatars`, both from Libraries.dev
(github.com/Jakubantalik/Libraries.dev, MIT, (c) 2026 Jakub Antalik), and the library page that shows
them (`src/ultra-reactivate/pages/alive-orbs.tsx`, `alive-bots.tsx`, `library.css`). Every number below
is read from those files; line references are to them.

## 1. The thinking orb

### 1.1 What it draws

A small sphere of dots (and, in one mode, lines) that moves while an agent works. Each frame is pure
geometry: the engine returns a list of dots `{x, y, z, r, white, a}` and lines
`{x1, y1, x2, y2, w, white, a}` in CSS px of the tuned size, sorted far to near (`engine/core.ts`
`finalizeFrame`). The component paints them on a 2D canvas: lines stroked with butt caps, then dots
as filled arcs, source-over, colour `rgba(g, g, g, a)` with `g = white × 255` on a dark theme and
`(1 − white) × 255` on light (`core.ts` `inkColor`, `paintFrame`), or a tint ramp when `color` is set.

### 1.2 States, modes, presets

Nine states map to eight modes (`presets.ts`):

| state | mode | label | dots at 64 / 32 / 20 |
|---|---|---|---|
| working | orbits | Working… | 516 / 100 / 39 |
| searching | globe | Searching… | 204 / 86 / 54 |
| solving | rubik | Solving… | 138 / 62 / 30 |
| listening | wave | Listening… | 134 / 62 / 42 |
| connecting | web | Connecting… | 48 + 81 lines / 17 + 6 / 9 |
| weaving | braid | Weaving… | 153 / 64 / 35 |
| composing | ribbon | Composing… | 566 / 258 / 208 |
| breathing | ring (ribbon face-on) | Thinking… | 484 / 184 / 120 |
| shaping | morph | Shaping… | 24 / 20 / 18 |

Counts are at t = 0.6 (they vary a little with t in orbits and web). Three tuned sizes, 64, 32 and
20 px (`types.ts` `OrbSize`), each with its own preset: a speed, count and dot-size multipliers and
extra mode options (`PRESETS`, 27 rows). `scaleCounts` scales counts by the knob (by its square root
for grid pairs such as lat rings × lon density), `scaleRadii` scales radii (`profiles.ts`).

### 1.3 Inputs

`ThinkingOrb.tsx` props: `state`, `size` (64/32/20), `theme` (dark/light), `speed`, `paused`,
`color` (tint), `dots` and `dotSize` knobs (each at least 0.1), `opts` (raw mode options), `frame`
(a fixed t), `gravity` (a cursor warp, section 1.6).

### 1.4 Motion constants

- One shared clock: `t = performance.now() / 1000 × speed` (preset speed × the `speed` prop), so
  every orb on a page is in phase.
- Reduced motion shows one still frame at t = 0.6; `paused` shows the frame at the current t.
- Canvas pixel ratio `min(2, devicePixelRatio)`.
- Morph holds each shape `HOLD` and blends `MORPH` seconds; every mode's constants are in its file
  (`engine/*.ts`) and now in `wubble_core::orb` (section 3.1).
- The web pauses an orb off screen (IntersectionObserver) and stops its rAF.

### 1.5 Cost model (web)

Per frame per orb: O(n) engine work for n marks (n above; the rubik mode also runs its move list per
dot, 14 moves at 64), a sort, then n canvas path fills on the CPU raster. A 64 px composing orb is
about 570 arcs a frame; at 60 fps that is about 34 000 arcs a second per orb. Off screen: nothing.

### 1.6 Gravity

`gravity` warps the dots toward the cursor through a sprite (`ThinkingOrb.tsx`, `gravity.ts`).
Pointer driven, used on the library page only; not ported (deviations).

## 2. The mascots (BotAvatar)

### 2.1 What it draws

A small 3D-looking creature (clover, flower, star, ghost and others, `types.ts`) with eyes, drawn
entirely with Canvas 2D on the CPU: no WebGL, no `ctx.filter`, no `shadowBlur`. The 3D look comes
from about 17 per-slice affine transforms of the body silhouette (`draw.ts`), not from meshes. Only
the worn items (`wear.ts`: hats, glasses, headphones, bow tie) are meshes (`mesh.ts`), painted as
gradient strips.

### 2.2 The rig (engine.ts `Sim`)

- Pose: yaw, pitch, roll, x, y, squash (`sx`, `sy`), per-eye blink, gaze (`lookX`, `lookY`), breath,
  laugh, whirl, and blend weights over three states: default, working, sleeping (`REST`, 264-268).
- Randomness: a mulberry32 stream seeded per instance (164-173); the order of draws is behaviour.
- `update(dt)` with dt clamped to 0.05 s: state blend (`SWITCH_TO` 1.2 / 0.7 / 1.4 s), idle gaze
  between corners (35° yaw, 14° pitch, held 2.6-4.4 s), spring wanderers on the head (ω = 1.6 rate,
  ζ = 0.9), pointer follow, blinks (0.17 s, 22% double), eye darts, idle flips (a jump of height 26
  over 0.68 s with squash and rise pulses, about 1.41 s in all), working hops, sleeping nods.
- The springs are explicit Euler, so the motion is not frame-rate independent at large dt.

### 2.3 Shadings

`shading`: `flat`, `crisp`, `smooth` (the slice fills with gradients), `plastic`, and the default
`fabric`. Plastic and fabric (`plastic.ts`, 3961 lines) need:

- one-off CPU bakes per shape: a distance transform, Poisson sampling, ambient occlusion and, for
  fabric, a fur bake of thousands of hair strokes;
- per frame while the light or view moves (every frame of a turn): a 64 × 64 matcap and an N × N
  texel shading loop (N 64 to 128) written with `putImageData`;
- conic gradients, pattern fills under a transform, Path2D from SVG strings, dashed strokes and
  about ten `globalCompositeOperation` modes (multiply, source-atop, destination-in/out, lighter...);
- a `getImageData` readback during spins (`measureMiss`).

### 2.4 Cost model (web)

Flat and crisp: about 17 path fills plus the eyes per frame. Fabric: a bake of tens of ms per shape
and size, then per frame tens of thousands of texel shades plus a readback during a turn. The
library page shows four at 44 px (`alive-bots.tsx`), fabric, dark.

## 3. Porting roads

### 3.1 Orb: engine in core, painter on the GPU, the orb's own texture (done)

- **Engine:** `wubble_core::orb` translates `engine/*.ts` and `presets.ts` line for line (each file
  cites its source; `tests/orb_constants.rs` diffs every number against the snapshot in
  `tools/fidelity/ts-snapshot/`). The preset tables, labels, `HOLD`, `MORPH` and the sizes are
  generated (`extract.mjs`, `wubble_core::kit_gen::orb`, `OrbGen`). `tests.rs` checks all 27
  presets at six times each against frames the design's own engine computed
  (`tools/fidelity/sample-orbs.mjs`, fixture `orb_frames.json`): identical to 1e-9.
- **Life:** one enum, `Life {Off, Still, Reduced, Running}`, and a pure `transition` returning
  `Paint` and `Animate` effects: off screen draws nothing, paused and reduced motion draw once, only
  `Running` asks for frames.
- **Painter:** `wubble-gfx/src/orb.rs` and `orb.wgsl`: every mark one instanced quad; the fragment
  counts 4 × 4 sample points per pixel (Skia's path AA grid) and blends premultiplied in paint order
  into a float target; a resolve pass writes the orb's own small texture (a ring of two, as
  `import.rs`), shown 1:1 by the kit's `ThinkingOrb` image. Reason: hundreds of marks a frame; a
  Slint `Path` per dot would rebuild hundreds of paths every frame on the CPU, and Skia's renderer
  redraws the whole window anyway, so a texture is the cheap unit.
- **Host:** `crates/wubble/src/orbs.rs` keeps each window's orbs, steps their life, and in the
  shell driver's frame hook paints only the running ones at the shared clock, asking for the next
  frame only while one runs.

### 3.2 What is the same and what is not

Same: every frame's geometry and ink, the presets, labels, shared clock, reduced-motion frame, dpr
cap. Different (deviations.md): no gravity; coverage from 16 samples instead of Skia's analytic
scanline AA (edges can differ by one sample, about 6% alpha); the canvas stores 8-bit premultiplied
colour while the painter blends in float, so faint overlapping dots are slightly smoother.

### 3.3 Mascots: not ported in this pass

The flat, crisp and smooth shadings would port (the rig to core, about 17 transformed slice fills
per frame as Slint paths or the same mark painter). The default fabric look, which is what the
design shows, needs the bakes, per-texel shading, composite modes and the readback of section 2.3:
a CPU texel loop per frame is against the frame budget and the readback is a CPU readback on the
frame path (AGENTS.md). Doing it properly means a new GPU port of `plastic.ts` (bakes as compute or
precomputed textures, shading in a fragment shader), larger than this pass. Porting only flat would
show a different look than the design's, which the brief forbids. Road when it is picked up: rig in
`wubble_core::mascot` (deterministic, test against the design's Sim seeded), bakes as one-off GPU
passes into the mascot's texture, shading per frame in WGSL, wear meshes as triangle strips.

## 4. Cost in wubble (measured by hand, docs/measurements.md)

- Idle page, no orb: `--measure idle --seconds 4` reports 0 slint_renders.
- An orb at rest or paused: painted once, then no frames (gallery `agent-orbs-paused`: 180 shown,
  0 moving). An orb off screen has no cell and no texture.
- One 64 px orb moving (`WUBBLE_MEASURE_ORB=<state>`, release, 60 Hz headless): 50 to 60
  slint_renders a second; the browser's main thread 8 to 9 % of a core against 1 % idle. The
  cost is the whole-window redraw a moving orb asks for, not the orb's own pass.

## 5. Proposal: where they go

- **Mascots as the agent session's identity.** Each MCP session (`wubble_core::agent`) gets a mascot
  picked from its session id, shown in the agent activity log's rows and on the approval sheet. State
  map: `AwaitingApproval` (asking) → default, gaze at the user; `Running` with calls in flight →
  working (hops); `Running` idle (waiting) → default; `Stopped` → sleeping.
- **Thinking orb places**, each driven by one state:
  1. the agent mark on a driven tab's row and its Space header (16 px, `Running` with a call in
     flight, else still);
  2. the activity page's header (32 px, any session running);
  3. the approval sheet's title (20 px, `AwaitingApproval`: listening);
  4. a tab's favicon while the page loads (16 px, the loading state: working), if the owner wants it.
- The specimen sizes 16, 24 and 160 are derived (derived-review.md); the wiring is one TODO line.
