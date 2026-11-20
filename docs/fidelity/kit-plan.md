# Locked widget kit: plan, lanes, comparison harness

Status (2026-11-19): phases 0 to 3 are merged (see [TODO.md](../../TODO.md) "Design fidelity"). The capture and
scoring harness (section 5, the gate in phase 4) was removed on 2026-11-19 (commit 187d7a4): review is by
screenshots and the owner's eye. Harness passages below are historical. Pre-phase audits it cites
(`inventory.md`, `map.md`) and run results were scratch notes and are not kept in the repo.

Inputs: `inventory.md` (798 design rows), `map.md` (verdicts), [wubble-only.md](wubble-only.md) (85 wubble surfaces,
38 with no design counterpart). Design truth: the web version in
the design repo (`xuckless/the-wallpaper-project`, `src/wubble`) at `598f814`. The design's `slint/` port is a
source of already-translated code where it is line-for-line with the web (8 of 9 shaders, physics,
`softbody.rs`, `minis.rs`, `wfield.rs`, `voice.rs`, `bubble.wgsl`, `pop.wgsl`), never a source of
values where it lags (seam, scroller thumb, Overlap re-entry, glass fallback).

## 1. Principles of the kit

1. **One kit, named after the design.** Each kit component carries the design's name and a header
   `// design: <file>:<lines> @598f814` pointing at the web source it translates. A kit component
   with no design source is a **derived** component and must name the design primitives it composes
   (`// derived: Menu(touch.tsx:458) + Button.ghost(touch.tsx:440)`); a derived component that needs
   a new primitive waits for the owner (list in `wubble-only.md` "Needs a new primitive").
2. **Values are generated, not typed.** Tokens, icon paths, spring constants and keyframe tables are
   extracted from the web sources by a script (`tools/fidelity/extract.mjs`, node, no deps beyond the
   design's own) into generated files: `ui/kit/gen/tokens.slint`, `ui/kit/gen/icons.slint`,
   `wubble-core/src/kit_gen.rs`. A test checks the generated files carry the pinned design commit and
   that `tools/fidelity/extract.mjs --check` (run by hand when the clone moves) reports no drift. No
   agent edits a generated file by hand.
3. **Shell files compose; only the kit draws.** Shell `.slint` files (everything outside `ui/kit/`)
   may use layouts, `Text` through `KitText`, kit components and model bindings. They may not paint
   (fills, borders, shadows, gradients, paths), animate, or name a colour/duration/easing.
4. **Shaders are translations.** Every WGSL file opens with the GLSL block it translates
   (`core/gl/glsl.ts:<lines>`, `scene.ts:<lines>`) and keeps the GLSL's statement order, names and
   constants so a reader can diff them side by side. Deviations are marked `// wubble change:` with a
   reason (e.g. WGSL `select` argument order, wgpu coordinate flip).
5. **Motion lives in two places only:** springs/bands/stretch/beam/scroller/seam in
   `wubble-core/src/motion*` (already a 1:1 port, keep), and fixed-curve tweens (`wb-in`, `wb-out`,
   press, squash, wobble, crossfade, fold, pulse) as kit components in `ui/kit/motion/`. Shell files
   never write `animate`.

## 2. Kit component list

Names are the design's. "Becomes" names the wubble file that turns into it or is deleted.

### 2.1 Generated assets (`ui/kit/gen/`)

| Kit asset | Design source | Becomes / replaces |
|---|---|---|
| `tokens.slint`: Palette (colour, spaces' accents and dyes), Radius, Space, Chrome geometry, Glass tiers (598f814 frost: fill, blur, saturate, brightness, rims, top light, shadows), Type roles, Motion (durations, easings, springs, bands, squash/wobble keyframes) | `core/tokens.ts`, `core/wubble.css`, `browser/browser.css` literals, `pages/*.css` literals, `core/physics.ts` | `ui/tokens.slint` (port copy, stale glass) and the design-valued half of `ui/app-tokens.slint`; the `Shell.*` half (126 wubble-only properties) moves to `kit/derived.slint`, each re-expressed as a design token or an arithmetic of design tokens |
| `icons.slint`: one `Icon { name }` table, stroke 1.7 default, size per call site rule | `browser/icons.tsx` (P table), `pages/touch-kit.tsx:128` (PATH), `SPACE_ICONS`, found-icons | `components/icon.slint` and the hand-drawn paths (speaker, book, play, tab close at `components.slint:461`) |
| `mark.slint`: logo Mark (dark/light/outline/solid, size rules: dot drops < 48, single colour at 16) | `pages/found-ui.tsx:71-176`, `browser/icons.tsx:48` | asset SVGs only today; adds start-page and About marks |
| `wubble-core/src/kit_gen.rs`: K springs, BAND_K, DT_CAP, REST, SQUASH/WOBBLE keyframes, SLIDER, TOGGLE_SIZE, SEG, VOICE, beam curve constants | `core/physics.ts`, `pages/touch.tsx:27,146`, `pages/touch-kit.tsx:60,195`, `core/voice.ts:7` | hand-copied constants in `wubble-core/src/motion.rs`, `controls.rs`, `beam.rs` (values already equal; they become `use kit_gen::*`) |
| fonts | `@fontsource-variable/geist`, `geist-mono`, `fredoka` | `ui/fonts` (same families; verify the wght axis renders 500/600 under Skia) |

### 2.2 Materials and motion (`ui/kit/material/`, `ui/kit/motion/`)

| Kit component | Design | Becomes / replaces |
|---|---|---|
| `Glass { tier: thin/regular/thick, shadow-strength }` reports its rect to the scene (`GlassLayer`), Slint fallback = frost tokens | `.wb-thin/.wb-regular/.wb-thick` (`core/wubble.css`), `browser/layer.ts`, `GLASS` | `GlassPanel` in `components.slint`; `raised.slint` `Chip` |
| `Film` (soft sheen ring 1.5 px, on hover/press/drag/focus only) | `.wb-film` | `glow-ring.slint` (the steady peek glow is wubble's: delete), Toggle/Slider FilmRim additions (delete) |
| `Press` (0.97, 120 ms ease-out, spring back) only where design uses `wb-press` | `.wb-press` | the blanket .97 on tab rows, command rows, Space tiles, folder rows (delete there) |
| `Squash`, `Wobble` (keyframe tables from gen) | `SQUASH`, `.wb-wob-a/b` | `squash.slint`, `wobble.slint` (keep, feed from gen) |
| `Overlap` / `Swap` / `Presence` (in 240 / out 190, shift 7, blur 4 → see limits, rows 30 ms apart, lead 40/60) | `.wb-in/.wb-out`, `touch-kit.tsx:80,109` | `entering.slint`, the port's `OverlapLayer` (fix re-entry −5 → +7) |
| `Crossfade` (label swaps) | `Swap` | new |
| `Fold` (240/190 height fold for settings search) | `settings.tsx` fold | new |
| `Pulse` | `wb-pulse` | new |
| `FocusRing` | design focus ring (accent) | `focus-ring.slint` (keep) |
| `Beam` (scene-drawn; Slint stand-in only where the scene cannot reach) | `GLASS` BEAM, `chrome.tsx:31-58`, `command.tsx:66-81` | `border-beam.slint` (stand-in, keep until overlay scene exists) |
| `KitText { role: display/title/heading/body/label/caption/mono }` | `core/tokens.ts type`, `.wb-display`, `.wb-mono` | every raw `Text` font-size/weight in shell files |

### 2.3 Controls (`ui/kit/controls/`)

| Kit component | Design | Becomes / replaces |
|---|---|---|
| `Button { kind: pri / glass / ghost }` 44 / r12 / 14·600, pri = Space accent fill + abyss text | `touch.tsx:440-466`, `touch.css .tp-btn*` | `sheet-button.slint` `SheetButton`, text mode of `PressableButton`, settings `TextButton` (as general button) |
| `IconButton { on }` | `.wb-ib`, `.wb-ib-on` | `PressableButton` icon mode, `nav-buttons.slint` buttons |
| `Toggle { size: lg/sm }` | `touch.tsx:146-240` | `controls/toggle.slint` (fix sm units: band 13→ per-size, slop, full stretch 1500) |
| `ElasticSlider` (value bubble, keyboard, step snap, live value) | `touch.tsx:27-140` | `controls/slider.slint` |
| `Segmented` (stretch indicator, keyboard, per-item accent, icons) | `touch-kit.tsx:195-260` | `controls/segmented.slint`, video speed TextButtons |
| `CheckBox` 22 / r7 glass, accent fill, drawn tick | `.tp-ck` | `check-box.slint` (16 px mint: replace) |
| `Swatches` (travelling, stretching ring) | settings swatches | settings `Swatches`, boost panel swatch tokens |
| `Menu`, `MenuRow`, `MenuHighlight` (236, r16, rows 36/r10/pad 10, 13.5, one gliding highlight, rows from −4 at 40+30·i, grows from trigger, `confirmMs`) | `touch.tsx:458-552`, `.tp-menu/.tp-mrow/.tp-menu-hl` | `context-menu.slint` and the menu bodies of autofill, downloads popover, permission prompt, tab-row and site menus |
| `Select` (trigger-anchored Menu, opens upward near the edge) | `settings.tsx` Select | Segmented used for many-option settings (search engine) |
| `HoldButton`, `ConfirmButton` | `settings.tsx` | the history clear sheet, one-click bins |
| `SearchCapsule` (`wb-thin wb-film`, ring on focus/query, animated clear) | `.st-search` | settings `Field` capsule; the command bar head |
| `Kbd` (one cap per key) | settings/command hints | settings `Kbd` |
| `Scroller` + `Thumb` (6 / hover 9, ink .38, .12 in / .3 ease-in out) | `browser/scroller.tsx`, `.wb-thumb` | `WubbleScroller` thumb, `page-scroll.slint` |
| `Seam` + `Pill` (white .1 1.25 string at rest, sky 1.6 held, 20 px strip, 6×40 pill, separator role + arrows) | `browser/seam.ts`, `.wb-seam/.wb-pill`, `browser/split.ts` | port `Seam` in `components.slint:487`; split and DevTools seams |
| `Tooltip` | dock tooltip | `tooltip.slint` (adopted, keep) |
| `Swap`-driven `Stepper`? | none (design uses ElasticSlider for numbers) | `Stepper` deleted where a slider fits; print copies needs a "typed numeric entry" primitive (owner) |

### 2.4 Chrome and pages (`ui/kit/chrome/`, `ui/kit/pages/`)

| Kit component | Design | Becomes |
|---|---|---|
| `WindowLights` (glyphs hidden until group hover) | `.wb-lights/.wb-light` | `window-lights.slint` |
| `AddressPill` (+ lock, beam) | `chrome.tsx`, `.wb-address*` | `address-pill.slint` |
| `Favs` | `.wb-favs/.wb-fav` | `fav-grid.slint` |
| `SpaceSwitcher` + capsule | `tabs.tsx:140` | sidebar footer parts |
| `TabRow`, `TabList`, `TabIndicator`, `NewTabRow` (drag fill white .1, shadow 0 10 24 .4, close icon 12/2 in mist, no press) | `tabs.tsx`, `.wb-tab*` | parts of `sidebar.slint`, `components.slint`, `new-tab-row.slint` |
| `Sidebar { mode: extended/rail/hidden/peek }` | `chrome.tsx`, `.wb-side*` | `sidebar.slint` |
| `CommandBar`, `CommandRow` (search icon head, hover selects, empty line, clamp keys, 60 ms lead, per-mode placeholder, mic) | `command.tsx`, `.wb-cmd*` | `command-bar.slint`, `command-row.slint` |
| `PageCard` / `SplitPanes` / `SplitRing` | `webview.tsx`, `.wb-view/.wb-split*` | `SplitView` in `components.slint` (pane "×" removed) |
| `SettingsPage` parts: `SettingsNav` (indicator on follow spring + stretch), `SettingsSection`, `SettingsRow { control }`, `ResetButton` (popover entrance), `NothingMatches` | `pages/settings.tsx`, `settings.css` | `components/settings.slint` |
| `StartPage` (bubble, search capsule, site tiles, stagger) | `pages/start.tsx`, `start.css`, `core/living.ts` | new (wubble has none) |
| `PageFrame` (list pages: history, bookmarks, downloads, passwords, archive, task manager) | derived: settings layout + Scroller + rows | `pages.slint` |
| `Sheet` (derived: thick Glass + Overlap + Button row) | derived | `sheet-frame.slint`, `dialog-sheet.slint`, bookmark/print sheets |

### 2.5 Fate of existing wubble UI files

- **Become kit (moved, then rebuilt to design):** `components.slint` (split into kit files first, see lane 0), `icon.slint`, `focus-ring.slint`, `squash.slint`, `wobble.slint`, `entering.slint`, `tooltip.slint`, `controls/*`, `check-box.slint`, `sheet-button.slint`, `context-menu.slint`, `window-lights.slint`, `address-pill.slint`, `nav-buttons.slint`, `new-tab-row.slint`, `fav-grid.slint`, `command-row.slint`, `raised.slint`, `border-beam.slint` (stand-in).
- **Deleted:** `glow-ring.slint` (steady peek glow is not in the design), the FilmRim additions on Toggle/Slider, `Stepper` where a slider fits, the text-mode `PressableButton`, `ui/tokens.slint` + design half of `app-tokens.slint` (replaced by generated tokens).
- **Stay shell (compose only):** `app.slint`, `sidebar.slint`, `command-bar.slint`, `settings.slint`, `pages.slint` and all wubble-only surfaces (`downloads-popover`, `download-row`, `find-bar`, `permission-prompt`, `autofill-menu`, `bookmark-sheet`, `print-sheet`, `dialog-sheet`, `sheet-frame`, `toast`, `status-bubble`, `preview-card`, `glance-card`, `pip-*`, `swipe-chip`, `autoscroll-cursor`, `shot-overlay`, `reader-controls`, `video-controls`, `boost-panel`, `error-panel`, `folder-row`, `popup-layer`, `page-ax`, `page-scroll`, `window-frame`, `model.slint`): rebuilt as compositions of kit components per `wubble-only.md`.

## 3. The lock (tests that take agency away)

Extend `crates/wubble-ui/tests/tokens_only.rs` into `tests/kit_lock.rs` (pure string scans, no timing):

1. **Kit boundary.** `ui/kit/**` is the only place that may contain: `background:` / `border-color:` /
   `border-width:` / `drop-shadow-*` / `@linear-gradient` / `@radial-gradient` / `Path` /
   `commands:` / `animate` / `easing` / `font-size:` / `font-weight:` / `opacity:` animations /
   `Palette.` / `Glass.` / `Motion.` / `Type.` references, and any literal with `px`, `ms`, `s`, `#hex`,
   `rgb(`, `hsl(` (the current scanner, with `components.slint`'s exemption removed). In shell files
   only `0px`, layout `spacing:`/`padding:` from `Space.*`, and `Chrome.*` geometry are allowed.
2. **Text through KitText.** A shell `Text {` element is a violation; use `KitText { role }`.
3. **Provenance headers.** Every file under `ui/kit/` (except `gen/`) starts with `// design:` or
   `// derived:`; each `design:` path must exist in the pinned clone when `WUBBLE_DESIGN` points at
   one (skipped otherwise, so CI without the clone still passes). `derived:` must cite at least one
   kit component.
4. **Generated files untouched.** `ui/kit/gen/*` and `kit_gen.rs` start with
   `// generated by tools/fidelity/extract.mjs from <commit>`; a hash of the body is recorded in the
   header and checked.
5. **Shaders.** Each `*.wgsl` in `wubble-gfx/src/scene/shaders/` starts with `// translates:`
   `<design file:lines>`; a constants test extracts every float literal from the cited GLSL block
   (read from the clone when present, else from a checked-in snapshot `tools/fidelity/glsl-snapshot/`)
   and from the WGSL, and fails on any value present in one and not the other unless listed beside a
   `// wubble change:` line.
6. **Ratchet.** The lock lands first with a baseline file (`tests/kit_lock_baseline.txt`, every current
   violation). The test fails on any violation not in the baseline and on any baseline line that no
   longer occurs (so the baseline only shrinks). Lanes delete lines as they migrate; the phase ends at
   an empty baseline.
7. **Existing tests stay:** `no_focus_borders`, `no_letter_tiles`, `tokens_match_physics` (pointed at
   the generated tokens).

## 4. Where things live

| What | Where |
|---|---|
| Generated tokens, icons, marks | `crates/wubble-ui/ui/kit/gen/` |
| Kit Slint components | `crates/wubble-ui/ui/kit/{material,motion,controls,chrome,pages}/` |
| Derived (wubble-only) tokens | `crates/wubble-ui/ui/kit/derived.slint`, each line an expression of design tokens with a comment naming the anatomy it derives from |
| Translated shaders | `crates/wubble-gfx/src/scene/shaders/*.wgsl` (existing), plus `bubble.wgsl`, `pop.wgsl` from the port (line-for-line with the web) |
| Pass setup (targets, blur levels, uniforms) | `crates/wubble-gfx/src/scene/render.rs` |
| Physics and motion machines | `crates/wubble-core/src/motion*`, `controls.rs`, `seam.rs`, `split.rs`, `scroller.rs`, `beam.rs`, new `softbody.rs`, `minis.rs`, `voice.rs` (ported from the design port, which is 1:1 with the web) |
| Generated motion constants | `crates/wubble-core/src/kit_gen.rs` |
| Generator and helpers | `tools/fidelity/` (`extract.mjs`, curve and icon tools); the capture harness and scorer were removed (187d7a4) |

## 5. Comparison harness (historical)

Removed on 2026-11-19 (commit 187d7a4); kept as the record of how the early lanes were scored.

### 5.1 Web side (verified today)

- `node` 22.23 and `/usr/bin/chromium-browser` 157 are installed (no google-chrome/chromium).
  `npm install` in the design repo succeeded; `npx vite --port 5199` serves the design at
  `http://127.0.0.1:5199/wubble/` (dev harnesses: `/wubble/pages/_found.html` (Foundations page),
  `/wubble/pages/_harness.html?page=touch|room&accent=`).
- Headless capture works with `--headless=new --use-angle=swiftshader --enable-unsafe-swiftshader`
  (WebGL on SwiftShader). A dependency-free CDP driver ((kept outside the repo), uses Node's
  built-in `WebSocket`) navigated the design, clicked the "Touch everything" tab, took stills and a
  screencast. Samples: (kept outside the repo), `web-01-touch.png`,
  `web-foundations-harness.png`.
- Findings from the prototype: the screencast gives only ~10 fps under SwiftShader, and the
  Foundations page's sections enter on intersection (a full-page shot catches tiles mid-entrance).
  So:
  - **Deterministic clock.** `Page.addScriptToEvaluateOnNewDocument` installs a fake
    `performance.now`, `requestAnimationFrame`, `Date.now`, `setTimeout` and a seeded
    `Math.random` (the liquid seeds and beam hue use it). The driver advances the clock by exactly
    1/120 s and captures after each step, so a frame sequence is exact regardless of render speed.
  - **Settle before stills** (advance the clock 2 s; scroll each section into view, advance again).
  - **Frozen backdrop option.** The fluid is chaotic; for glass scoring both sides render over the
    same fixed dye texture. On the web this is a harness-side vite plugin (`transform` hook in the
    harness's own `vite.config.mjs`, root = the clone) that replaces the dye initialiser and stops
    the emitters at serve time; the clone is never edited. On wubble it needs a debug flag
    (`--scene-backdrop PNG`, frozen dye) — a small engine addition, behind a smoke-only flag.
  - **State drivers.** Scenes are data (`scenes.json`): URL, clicks/keys/pointer path (CDP
    `Input.dispatch*`), clock advance, crop selector. Crops come from `getBoundingClientRect` of a
    selector (e.g. `.wb-window`, `.tp-menu`), so each crop is exact.
- Size and scale: viewport 1360×880 at DPR 1 and DPR 2 (`Emulation.setDeviceMetricsOverride`). The
  design draws the browser as a window on a desktop with a ~36 px margin; the harness crops
  `.wb-window` (≈1288×728 at the default size) and wubble shoots at that window size
  (`WUBBLE_SHOT_SIZE=1288x728`).

- **Renderer, parallelism, cache (ui2a harness, 2026-11-19).** GPU headless works on the Linux box
  (ANGLE on Vulkan, NVIDIA): deterministic, and pixel-identical to SwiftShader on pages with no
  WebGL (touch kit, specimens), but it renders the liquid wrong (glass blown out white, mean ΔE 15
  on `sidebar-extended`). So scenes say `gl`: `gpu` only on `/wubble/pages/` scenes, SwiftShader
  for anything that loads the shell. The ~35 min run is cut by parallel browsers (`--jobs`, one
  browser per renderer per worker, ephemeral debugging ports) and a web cache keyed by design
  state, harness files, Chromium version, scene, DPR and renderer
  ((kept outside the repo)): a run re-shoots only what changed. The stepper now starts an
  animation on the clock when it first sees it and syncs before every shot (L0 drew a transition
  just started by an input at its real-time progress, so a sequence's first frames differed run
  to run); jobs 1 against jobs 6: rects and curves byte-identical, pixels within 0.12 mean ΔE.
  Measured under the shared box's load: L0 serial 4483 s for 34 tasks; GPU specimens 74 tasks
  32 s at 6 jobs, a warm cached run 0.3 s; SwiftShader scenes 120 to 330 s each, so a cold full
  run stays long (paid once per design commit); not measured end to end.

### 5.2 wubble side

- Existing: `--smoke gallery --shot-dir DIR` (26 scenes, 1360×880, `WUBBLE_SHOT_SIZE`,
  `SLINT_SCALE_FACTOR`), `--shot PNG` for live-page surfaces, `WUBBLE_SHOT_SCROLL`. Last gallery run:
  kept outside the repo (18 Nov).
- Needed:
  - **Content parity profile.** A seeded profile mirroring the design's model (`browser/model.ts:40-41`:
    Spaces Personal/Work, tabs "Start page", "Skink: a modeless code …", "Soap bubble - Wikipedia",
    "Touch everything", the site snapshots in `sites/*.webp` served from loopback with their favicons),
    so text and favicons match. Today's gallery shows a "wubble smoke" data: page; a trial SSIM on a
    tab-row crop is meaningless while the content differs
    ((kept outside the repo)).
  - **Pointer and hover in scenes.** Gallery scenes gain an optional pointer position / held state so
    hover, press and focus states can be shot (the web side hovers through CDP).
  - **Deterministic frame sequences.** Slint's testing backend has mock time
    (`init_integration_test_with_mock_time`); a `--smoke gallery --frames N --dt 1/120` mode stepping
    the driver's clock and the scene's time by a fixed dt gives the matching sequence.
  - **Frozen backdrop flag** (above).

### 5.3 Scoring

- **Per pair:** align by the scene's crop rect (same logical rect both sides), then
  - SSIM on luma (gaussian window 11, σ 1.5), target ≥ 0.90;
  - mean and 95th-percentile CIEDE2000 ΔE in Lab, target mean ≤ 3, p95 ≤ 8 (catches tint shifts
    SSIM ignores, e.g. the frost colour);
  - edge/geometry: bounding boxes of key elements must match within 1 logical px (see next point).
- **Geometry parity as a real test.** The web harness exports `getBoundingClientRect` of every named
  element per scene to `tools/fidelity/geometry/<scene>.json`; a `wubble-ui` test (Slint testing
  backend, `ElementHandle` by accessible id, no timing) lays out the same scene and compares rects.
  Deterministic, runs in `cargo test`.
- **Motion curves.** The web clock-stepped run records sampled properties (transform, opacity, width,
  beam head/strength) per frame into JSON; the wubble side samples the same properties from core
  machines (unit tests) and from Slint under mock time. Score = RMS error over the normalised curve
  and peak/settle-time differences; target ≥ 0.90 similarity (1 − normalised RMS). These are value
  comparisons at fixed dt, not timing assertions, so they are allowed in tests.
- **Tools.** ImageMagick 7 is installed (`magick compare -metric SSIM` works; trial run done);
  Python has no scikit-image/OpenCV. Proposed: a small Rust bin `tools/fidelity/score` using
  `image` + `image-compare` (MIT) for SSIM and a hand-written CIEDE2000, writing `scores.csv` and an
  HTML contact sheet (web | wubble | diff heat map). Alternative without building: a venv with
  scikit-image.
- **Tracking.** `scores.csv` per run (scene, crop, ssim, ΔE mean, ΔE p95, geometry max delta, curve
  score); the phase gate is every scene ≥ 0.90 SSIM and ΔE mean ≤ 3, or an owner-signed exception.

- **As built (ui2a, 2026-11-19).** Geometry is a node tool, not a `cargo test` of its own:
  `tools/fidelity/geometry-diff.mjs` compares the web's per-specimen `rects.json` (layout box and
  drawn box per named element) with a JSON each controls lane writes from its own Slint test
  (`ElementHandle::find_by_element_id`, position relative to the specimen root, `size()`,
  `computed_opacity()`), 1 px tolerance, exit 1 on a miss; the lane's test may call it. Curves are
  the specimens' `curve.json` (per 1/120 s frame: rects, opacity, transform, scale, translate,
  clip-path, background). Pairs are **gating** unless declared `gate: false` with a `why`
  (whole-page settings pairs: wubble's own sections and wording); the gate reads "every gating
  pair". Crops may mask text whose content differs by the owner's rule (the address URL); the
  report lists every mask and its share. Settings is scored by anatomy crops (nav item, section
  header, search, a row per control kind) against wubble element ids `Settings::<part>`. The
  sign-off page is `report.html` (`tools/fidelity/report.mjs`): surfaces worst first, pass/fail,
  change since a named run.

### 5.4 What cannot be scored (owner's eye)

- Motion feel at 120 Hz (springiness under the hand, drag follow, throw, rubber-band), frame pacing.
- The living liquid's character over minutes (speed, calm, colour drift), since it is chaotic.
- Glass over live, changing web pages (frost, legibility), beam aesthetics in motion.
- Bubble facing, pop, distortion wave look in motion.
- Anything on macOS (Retina, Metal), and HiDPI text rendering versus Chromium's.

## 6. Foreseeable Slint/Skia limits and fallbacks

| Limit | Where it bites | Fallback / proposal |
|---|---|---|
| No backdrop blur under Slint items; the scene never sees page pixels | Menus, command bar, peek sidebar, sheets over the page: the web blurs the page 24–48 px with saturate 1.7 and thick brightness .55 | **Proposal (owner decision):** composite the CEF page texture inside the scene (it is already a zero-copy wgpu texture), so overlay glass blurs it through the same pyramid; Slint draws content only, with transparent fills over scene glass. Interim fallback: opaque-ish frost fill (today 0.94) — scores below 0.90 over bright pages. |
| ~~No per-item filter blur~~ (L2: stale) | `wb-in/wb-out` 4 px blur, tab exit 3 px blur | Slint 1.19 `backdrop-blur` on a transparent Rectangle inside an item whose opacity is below 1 blurs only that item's pixels (Skia only; `i-slint-compiler-1.19.0/builtin_elements.rs` docs, `i-slint-renderer-skia-1.19.0/itemrenderer.rs` `draw_backdrop_blur`). L2 uses it in `OverlapLook` (Overlap, Entering, Crossfade, Fold): blur 4 px x (1 - e), hidden at rest. **L2-1, owner flag:** not yet checked by eye in a live run (the testing backend does not render); cost is a re-read of the pixels under the item for 190 to 240 ms, measure by hand at 120 Hz. Fallback if it misbehaves: drop the blur rectangle (opacity + shift only). |
| ~~`drop-shadow` has no spread and no inset~~ (L2: stale); **L2-2: a drop shadow follows its fill's alpha** | Glass fallback shadow (`.wb-thin/regular/thick` 0 8/14/20 22/36/50 at .22/.28/.35 under fills of alpha .09/.11/.42) | `inner-shadow-*` and `drop-shadow-spread` exist (Skia): the fallback's rims are inner shadows (spread 1 px all round, offset-y 1 px on top). Skia draws a translucent source's shadow from its alpha (`render_drop_shadow_image` uses the fill unless opaque; docs: "the fill and border alpha determine the shadow's shape and strength"), so the Flat-tier shadow is fainter than the CSS; the scene draws the real one where it is under the item. Pulse draws its spread ring as a border for the same reason. |
| ~~No conic gradient~~ (L2: stale); **L2-3: no `from` angle, stops not reorderable** | Film sheen ring's 4 s swirl | `@conic-gradient` exists (Slint 1.19 `parser/expressions.rs`); `Film` draws the design's ring at its starting angle (210deg, stops rewritten from 0deg). The swirl (`wb-film-swirl`, the start angle animated to 570deg) would need stops that move and wrap; not run. Stroke colour along a path still cannot vary (border beam stand-in). |
| **L2-4: no global animation scale** | Reduced motion (`core/wubble.css:72-74`: every animation and transition 1 ms) | Slint's `SLINT_SLOW_ANIMATIONS` only slows, its `MotionPreference` reaches only `Flickable` and is not public: every `animate` duration goes through `Calm.ms(d)` (`kit/motion/calm.slint`: the design's 1 ms when reduced), the one switch; the host writes `Calm.reduced` from the setting and the OS (Linux portal, macOS NSWorkspace, Windows SPI_GETCLIENTAREAANIMATION). |
| **L2-5: change handlers run on the event loop's turn** | Crossfade swaps its label in `changed text` | In a test, `slint::platform::update_timers_and_animations()` runs them (as the event loop does before a frame); the swap starts one frame after the change. |
| **Design bug (598f814), owner flag** | `.wb-out`, `.wb-tab-closing`, `.wb-cmd-out` | They ease on `var(--wb-ease-in-out)`, but `tokenCss()` defines `--wb-ease-inOut`: the declarations are invalid at computed-value time and those exits do not animate in the browser. The kit follows the intent (tokens.ts `overlap.out` on `ease.inOut`); `tools/fidelity/curves.mjs` defines the missing name to sample it. |
| No `mix-blend-mode`, `backdrop-filter: brightness/saturate` | Frost colour lift ×2.7, thick dimming | In the glass shader (web GL path is the truth anyway). |
| Native window owned by the compositor | Window move lag/throw/stretch, rubber-band resize, minimise to bubble, open spring | Decided compositor-native in TODO.md; matching needs a client-side-decorated, self-moved window (Wayland can't move itself) — owner decision; otherwise these 8 rules are documented exceptions. |
| Variable fonts under Skia | Geist/Fredoka weights 500/600 | Verify wght axis applies; else ship static instances of the needed weights. |
| Text rasterisation | Skia + FreeType vs Chromium (same rasteriser family) | Expected close; score text-heavy crops with ΔE and allow a lower SSIM band for small text. |
| Per-pixel refraction on text | The web does not refract DOM text either (DOM content over GL material) | No issue; only the backdrop distorts (press wave). |
| Frame-rate-dependent forcing | Web splats per frame (tuned at 60 Hz); wubble runs 120/15 Hz | Convert splat strength to per-second at the web's 60 Hz equivalent (decision). |
| Liquid seeding | `Math.random` vs xorshift | Seeded PRNG both sides for harness runs; production may stay random. |

## 7. Work breakdown (lanes)

Phase 0 is serial and blocks the rest; phases 1–3 run lanes in parallel with disjoint files.
Every lane: research is copy-first from the web source (the port only where it is line-for-line),
correctness + simplicity review, then the lock baseline shrinks by the lines the lane cleared.

### Phase 0 — foundations (one lane, the lead)

**L0 Harness, generator, lock, split.**
Owns: `tools/fidelity/**`, `crates/wubble-ui/ui/kit/gen/**`, `crates/wubble-core/src/kit_gen.rs`,
`crates/wubble-ui/tests/kit_lock.rs` + baseline, the gallery scene additions
(`crates/wubble/src/gallery_smoke.rs`, `host/gallery.rs`: parity profile, pointer, frames, frozen
backdrop flag). Also a **move-only** commit splitting `components.slint` and `app-tokens.slint` into
`ui/kit/*` files (no behaviour change), so later lanes own separate files.
Acceptance: `extract.mjs` reproduces every A-row value; web capture of all scenes in `scenes.json`
(the 26 gallery surfaces that have a design counterpart + touch/settings/foundations specimens) at
DPR 1 and 2; wubble gallery on the parity profile; first `scores.csv` baseline; lock test green with
its baseline; `cargo test --workspace` green.

### Phase 1 — material, scene, motion primitives (parallel)

**L1 Scene glass and liquid.** Owns `crates/wubble-gfx/src/scene/**`.
Glass to 598f814 frost (`frost()`, b3 level σ5, reads b2+b3 at .4/.7/1, rim bend `t²·bezel·.35`, no
dispersion/specular, soft edge, grain, tint `(tint·.55+.1)` at 7 %, pointer light σ .45 gain .03,
shadows mix(.22,.38) per surface: window 1, address 0, favs .3), σ ÷ scale on lower quality tiers,
beam added into the glass, per-second splats, Space dyes from the design's three-colour tables, DPR
cap 2, frozen-backdrop flag. Then the bubble/W field/facing/pop passes from the port (line-for-line
with the web) and the voice glow driver (behind the mic decision).
Acceptance: shader constants test green with zero unexplained differences; glass crops ≥ 0.90 SSIM,
ΔE ≤ 3 over the frozen backdrop; `--smoke frames=300` (pixels ok, glass ok); `gpu_crash_smoke`.

**L2 Materials and motion.** Owns `ui/kit/material/**`, `ui/kit/motion/**`, `ui/kit/gen` consumers,
`crates/wubble-core/src/motion*`, `ui/kit/derived.slint`.
Glass fallback tokens (598f814 fills, shadows .22/.28/.35, rims .05/.06), Film only on touch, Press
only where `wb-press`, Overlap (+7 re-entry, 40/30·i, 190/120 exits), Crossfade, Fold, Pulse,
reduced motion = every duration to ~0 (and read OS prefs on macOS/Windows), KitText roles.
Acceptance: curve tests vs web curves ≥ 0.90; lock baseline lines for material/motion cleared.

**L8 Icons, logo, fonts** (small; may ride with L2). Owns `ui/kit/gen/icons.slint` consumers,
`mark.slint`, app icon assets (n=5 squircle, plate gradient radius 1, current edge lights).
Acceptance: icon sheet render vs web icon sheet ≥ 0.95 SSIM per glyph; app icon PNG diff.

### Phase 2 — controls (one or two lanes)

**L3 Controls kit.** Owns `ui/kit/controls/**`, `crates/wubble-core/src/controls.rs`.
Button pri/glass/ghost, IconButton on-state, Toggle sm units, ElasticSlider (bubble, keys, step,
live), Segmented (keys, icons, accent), CheckBox, Swatches ring, Menu (gliding highlight, grow from
trigger, confirm), Select, HoldButton, ConfirmButton, SearchCapsule, Kbd, Scroller thumb, Seam + Pill
(+ keyboard), Tooltip.
Acceptance: a `wubble://touch`-equivalent kit specimen gallery scene per control and state (rest,
hover, press, focus, disabled, selected) vs `_harness.html?page=touch` crops ≥ 0.90; geometry test
green; curve tests for toggle/slider/segmented/menu.

### Phase 3 — composition (parallel; each lane composes only kit components)

**L4 Chrome.** Owns `ui/kit/chrome/**`, `app.slint`, `sidebar.slint`, `command-bar.slint`,
`window-frame.slint`, chrome parts moved out of `components.slint` in L0.
Sidebar modes, seam, tab rows (drag look, close icon, no press, Up/Down), address pill (lock, beam),
lights, favs, Space switcher (haze at rest), command bar gaps, split panes/ring/seams (no "×").
Acceptance: gallery scenes 01–05 and split/peek shots ≥ 0.90; smokes `tabs`, `split`, `windows`, `nav`.

**L5 Settings and list pages.** Owns `components/settings.slint`, `pages.slint`, `ui/kit/pages/**`,
`crates/wubble-ui/src/settings.rs`, `pages.rs`.
Nav indicator on follow+stretch, search (every word, notes, keywords, highlight, fold, "Nothing
matches"), Select, Reset per section with entrance, HoldButton clear, narrow layout < 760, PageFrame.
Acceptance: settings scenes 16–19 and 20–26 ≥ 0.90 (list pages scored against the derived anatomy's
web parts); smokes `settings`, `pages`.

**L6 Derived overlays (wubble-only).** Owns every wubble-only component file listed in 2.5 except
those in L4/L5. Rebuild each per `wubble-only.md` on Menu/Button/Sheet/Glass/Overlap/KitText.
Acceptance: lock baseline empty for these files; gallery scenes 06–15 scored against the design part
they derive from (menu crops vs `.tp-menu`, buttons vs `.tp-btn`); owner eye pass on the derived
anatomy; smokes `page`, `autofill`, `media`, `video`, `boost`, `builtins`.

**L7 Start page, bubble, voice.** Owns `ui/kit/pages/start*.slint`, new `wubble-core` softbody /
minis / voice, host wiring for the start page. Depends on L1's bubble pass (the pass itself is L1's,
since `render.rs` is L1's file).
Acceptance: start scene vs `/wubble/` start ≥ 0.90 (frozen backdrop, clock-stepped bubble at rest);
owner eye on poke/drag/pop.

### Phase 4 — gate (historical: now the owner's sign-off on screenshots and by eye)

Full harness run at DPR 1 and 2 on Linux and the Mac (bundle), `scores.csv` all ≥ 0.90 or
owner-signed exceptions, lock baseline empty, the console smoke loop green, owner eye pass on motion.

### Collision rules

- `render.rs` and every WGSL file: L1 only. `components.slint` ceases to exist after L0's split.
- Tokens: generated (L0); derived tokens: L2 owns `derived.slint`; other lanes add derived values in
  a `// derived:` block at the top of their own kit file and L2 hoists them after merge.
- `app.slint`: L4 only; L5/L6/L7 expose components and L4 places them (or the lead does in the
  integration branch).

## 8. Open questions for the owner (answered 2026-11-19, see the end of this section)

1. Adopt `598f814` coloured-frost glass wholesale (tokens + shader) now? (All parts assume yes.)
2. Menus, buttons, checkbox, sheets: move from Ultra Reactivate's values to the touch kit's (Menu
   236/r16/36, Button 44/r12, CheckBox 22/r7)? The design now has them; wubble chose UR when it
   thought the design had none.
3. Overlay glass over the live page: composite the page in the scene so menus/command bar/peek blur
   the page like the web (architecture change), or accept the opaque fallback below 0.90?
4. Window physics and minimise-to-bubble: stay compositor-native (TODO.md decision; 8 motion rules
   become exceptions) or client-side?
5. Do design specimen/doc surfaces count toward the gate (Foundations page, Room, touch specimens:
   Pin, Download, Dock; Source panel), or only the shell plus the kit specimens?
6. Start page: build it (living bubble, search capsule, site tiles) as wubble's new-tab page?
7. Voice: ship the mic (command-bar voice + glow), off by default? Privacy/permission call.
8. Liquid forcing: per-second at the web's 60 Hz look, or keep per-frame?
9. The nine "needs a new primitive" items in `wubble-only.md` (notice/toast, ranged scrubber,
   marquee, custom pointer, code area, numeric entry, data columns, menu separator, Little/Popup
   window rest look): approve derivations or add them to the design repo first?
10. Room seam's mass term (string wobbles on Ctrl S toggle) for the sidebar seam: the room and
    `browser/seam.ts` disagree — which is truth?
11. Scoring thresholds: SSIM ≥ 0.90 plus mean ΔE ≤ 3 per crop, at DPR 1 and 2 — acceptable as the
    gate's definition of "1:0.90"?

### Owner decisions (2026-11-19)

Recorded as Decided rows in wubble's TODO.md ("Design fidelity" rows).

1. Yes: 598f814 coloured-frost glass adopted wholesale (tokens and shader).
2. Yes: menus, buttons and the checkbox take the touch kit's values, not Ultra Reactivate's.
3. Yes: the scene composites the page so overlay glass (menus, command bar, peek sidebar)
   blurs the live page. This overturns the old "glass never blurs web content" rule.
4. Window physics and minimise-to-bubble stay with the OS compositor: not ported.
5. Specimen and doc pages (Foundations, Room, Source panel, Pin/Download/Dock) are reference only:
   their widgets and effects are scored through real surfaces and the pages are not shipped.
   `scenes.json` may still capture them as references for isolated widgets.
6. The design's start page becomes wubble's new-tab page.
7. No voice feature in this phase: the voice glow shader goes into the kit, no microphone.
8. **Not yet decided** (liquid stirring per second vs per frame): left as is.
9. The nine missing kit pieces and the 38 wubble-only surfaces are derived from the design's
   rules and shown to the owner on one review sheet.
10. The split seam's truth is `browser/seam.ts`, not the Room page's.
11. The bar before a surface reaches the owner: SSIM >= 0.90 and mean ΔE <= 3 on stills at 1x
    and 2x, plus frame-by-frame motion on the fixed clock; the owner's eye is the final judge.
12. (Owner override of the web design, lane L1) The sidebar has no moving liquid: still
    coloured-frost translucent glass with a slight ambience, light from the page's edge colours
    leaking in (sampled from the page texture the scene composites). So the harness's frozen
    backdrop is the normal state for sidebar scenes: sidebar pairs are scored against the web
    with its backdrop frozen, and the ambience has no web counterpart (owner's eye).

## 9. Phase 0 as built (L0, 2026-11-19): where reality differed

Branch `ui/kit0`. Commits: gallery fidelity hooks; the move-only split; the generator; the lock;
the harness and docs. Baseline scores (`scores.csv`, `scores.md`,
`sheets/`, the generator's `gen-diff.md`, the split proof) were kept outside the repo.

**Generator** (`tools/fidelity/extract.mjs`, node only, no dependency beyond the clone's).
- `core/tokens.ts` is imported directly (node's type stripping); `core/physics.ts` cannot be (a
  class with parameter properties), so it and every other constant table are read as text, their
  object literal evaluated with the token scope. TSX tables (icons) are read by regex.
- Generated files: `ui/kit/gen/tokens.slint`, `gen/icons.slint` (browser `P`, kit `PATH`, Space
  icons, settings section icons, every inline `name="M…"`/`d="M…"` path of browser/ and pages/,
  the `Mark` geometry and the app-icon plates: no separate `mark.slint`), **and `gen/css.slint`,
  not in the plan**: every typed declaration of the seven stylesheets as
  `<Global>.<selector>--<property>[-part]` (2500 values; shorthands, shadows, transitions,
  transforms, filters and gradients split into parts; 116 declarations with numbers that do not
  read as a typed value are listed in comments; keyword-only declarations such as `position`
  are layout and counted). This is how "browser.css/pages literals" reach the kit.
- `wubble-core/src/kit_gen.rs`: `physics` (BAND_K, K, DT_CAP, REST, STRETCH defaults), `touch`,
  `touch_kit`, `settings`, `seam`, `split`, `tabs`, `scroller`, `window`, `command`, `voice`
  tables, `tokens` (radius, space, chrome, motion, type) and CSS keyframe tables (squash,
  wobble, button squash). Modules are `#[rustfmt::skip]`.
- Header: `// generated by tools/fidelity/extract.mjs from 598f814 …` and `// body-fnv1a64:`.
- A-row coverage is by **value**, not by line: the inventory's line numbers for `core/wubble.css`
  do not match the file at 598f814 (it is 75 lines; rows cite :95-:163). 247 of 249 A-rows have
  every value generated; the two left are A-223's clear-space 15.2 (computed in code, not a
  literal) and A-244's "1024" (caption prose).
- Differences against wubble today (`baseline/gen-diff.md`): every same-named Rust constant is
  equal (K, BAND_K, DT_CAP, REST, SLIDER, TOGGLE, SEG, SEAM, SPLIT, TABS, SCROLL); 74 generated
  constants have no same-named copy in wubble-core; `ui/tokens.slint` differs in the glass tiers,
  rims, specular band and thickOverContent (the stale 6c840da glass); of `Shell`'s literals,
  most have an equal value somewhere in the generated kit, 29 have none (wubble values to derive
  or get signed off). No call site was switched.

**Lock** (`crates/wubble-ui/tests/kit_lock.rs`, replaces `tokens_only.rs`).
- The baseline counts per (file, rule, token), not per line, so moving code does not churn it;
  1421 violations at the start (shell: token groups 560, font 206, paint 138, text 80, animate
  64, primitive 8; kit literals 356; shader headers 9).
- Stricter than planned: kit files are scanned for literals too (`kit-literal`), so values must
  come from `gen/`; `derived.slint`'s literals count until L2 re-expresses them.
- `ui/tokens.slint` (the port copy) is exempt as legacy until L2 deletes it.
- Shaders: only the `// translates:` header is ratcheted; the GLSL constants test (item 5) is
  L1's, with the shader work.
- `KIT_LOCK_WRITE=1` lowers the baseline and refuses when any count grew; the first baseline is
  written only when the file is absent.

**Split** (move only).
- `components.slint` → `kit/material/glass.slint` (GlassPanel), `kit/motion/overlap.slint`,
  `kit/controls/{button,seam,scroller}.slint`, `kit/chrome/{tab-row,split-view,space-icon}.slint`.
- `app-tokens.slint` moved **whole** to `kit/derived.slint` (one `Shell` global; splitting it
  into design and derived halves would rename hundreds of references and is L2's work when it
  re-expresses them).
- The other "become kit" files of 2.5 (icon, focus-ring, squash, controls/*, check-box, ...) were
  **not** moved: in `kit/` their violations would leave the shell rules' ratchet; each lane moves
  its files when it rebuilds them.
- Proof: the Slint compiler's generated Rust (`app.rs`, 17 MB) is byte-identical before and after;
  gallery shots on `--frozen-backdrop` differ only within the run-to-run noise of two runs of the
  same build (`baseline/split-proof.txt`).

**Harness.**
- The design window at the 1360x880 viewport is 1288x816 (not 1288x728).
- 2x: wubble's window must fit the screen (2560x1440 on the Linux box), so 2x uses a 1280x760
  viewport on both sides (window 1208x696): `scenes.json` `viewportByDpr`.
- Web clock: injected at serve time (vite `transformIndexHtml`); CSS animations and transitions
  are paused and set to the same clock each step through the Web Animations API. Stills settle
  2 s plus 0.6 s on that clock. About 25 to 60 s per scene under SwiftShader.
- Frozen backdrop: web, a serve-time patch of `core/gl/scene.ts` (emitters and fluid step off
  after the settle); wubble, `--frozen-backdrop` holds the liquid's pace at 0 (host,
  `backdrop.rs`; no engine change). The two frozen liquids are not the same picture, and
  wubble's frozen state is not bit-identical run to run (small differences). A shared frozen
  dye (`--scene-backdrop`) is L1's.
- Parity profile: the harness vite serves `/__parity/manifest.json` (Spaces, tints, icons, the
  four favourites as pinned tabs, the design's tabs) and `/__parity/tab/<id>` pages with the
  design's titles and favicons; wubble's `--parity FILE` seeds it before the walk. Page tabs
  (Start page, Touch everything, ...) are served pages, so wubble shows their favicons as images
  where the design draws stroke icons; Touch/Room pages embed the design's page.
- wubble frames are **not** on a fixed clock: a gallery burst snapshots as fast as it can (about
  120 ms apart) and records each frame's time; the scorer pairs it with the web frame at that time.
  Exact-dt comparison of motion is the curve tests' (Slint mock time), not built here.
- Not built in L0: the geometry parity `cargo test` (5.3; the web side writes every crop's rect
  per scene, which that test can read) and the motion curve JSON. They belong with L3/L4.
- Scorer in node (PNG, SSIM, CIEDE2000 hand written, tested against Sharma's data), no Rust bin.
- Derived pairs (wubble menus against `.tp-menu`) are compared top-left at the common size.

**Harness follow-up (ui2a, 2026-11-19).** Branch `ui2a/harness`, `tools/fidelity` only (research note
not kept).
- Speed: ephemeral debugging ports and per-browser profiles (L0's fixed 9333 let a stale or
  concurrent run attach to another's browser), parallel browsers (`--jobs`, default 6), the web
  cache (above), `run.sh` skips vite when every shot and the parity manifest are cached. GPU only
  for specimen pages (5.1).
- Specimens: `specimens.json` (20 controls, 140 scenes: stills per state, 1/120 s sequences with
  curves) expanded into ordinary scenes by `lib/specimen.mjs`; new capture steps `press`,
  `release`, `move`, `focus` (keyboard modality), `disable`, `holdFor`, `reveal`, `drag` with
  `to`/`hold`; `specimens-index.json` with the design source line of each.
- Geometry: `geometry-diff.mjs` (5.3). No Rust; the lanes export their side.
- Fairness: crop masks (address URL), `gate: false` for whole-page settings pairs,
  `settings-anatomy` derived pairs resolved through `score.mjs --wubble-rects`; scores.csv gains
  `gate,masked,why`.
- Report: `report.mjs` (static, no network), written by every `run.sh` against the L0 baseline.

