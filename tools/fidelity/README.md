# Design fidelity: generator, harness, scorer

The web version of the design (`the-wallpaper-project`, `src/wubble/{browser,core,pages}`) is the
truth for look, motion, effects and widget behaviour. These tools take values from it and measure
wubble against it. Plan and scope: `~/Dev/wubble-wt/notes/fidelity/kit-plan.md`. The design clone
is `$WUBBLE_DESIGN` or `~/Dev/wubble-wt/ref/the-wallpaper-project`; nothing here edits it. Node
22 and `/usr/bin/chromium-browser` are the only requirements (no npm packages of our own; the
clone's own `npm install` provides vite).

## Generator

    node tools/fidelity/extract.mjs                write the generated kit files
    node tools/fidelity/extract.mjs --check        exit 1 if they are stale against the clone
    node tools/fidelity/extract.mjs --report F     where generated values differ from wubble's
    node tools/fidelity/extract.mjs --coverage ~/Dev/wubble-wt/notes/fidelity/inventory.md

Writes `crates/wubble-ui/ui/kit/gen/tokens.slint` (palette, Spaces, radius, space, chrome, glass
tiers, fonts, type roles, motion), `gen/css.slint` (every typed declaration of the design's
stylesheets, `<selector>--<property>[-part]`, one global per file, the unread ones listed),
`gen/icons.slint` (browser and kit icon tables, Space and settings icons, every inline path, the
mark, app-icon plates) and `crates/wubble-core/src/kit_gen.rs` (the motion constant tables).
Each starts with the design commit and an FNV-1a hash of its body; the kit lock fails on a hand
edit. When the clone moves: `git -C <clone> pull`, run the generator, commit the result with the
lanes' call-site changes.

## Harness

    tools/fidelity/run.sh [OUT]          # everything; OUT defaults to notes/fidelity/runs/<date>

1. Serves the clone with `vite.config.mjs` (port 5199): a fixed clock and a seeded `Math.random`
   injected at serve time, a frozen-backdrop switch patched into `core/gl/scene.ts` at serve time,
   and the parity pages (`/__parity/manifest.json`, `/__parity/tab/<id>`). Skipped when every web
   shot and the parity manifest are in the cache.
2. `capture.mjs` shoots each scene of `scenes.json`, and each specimen of `specimens.json`, in
   headless Chromium at DPR 1 and 2, in parallel browsers (`--jobs`, default 2: the box is shared by lanes; 6 is the measured sweet spot alone), through the
   web cache. Time moves only by `__wb.step(1000/120)`; CSS animations and transitions are paused
   and set to the same clock each step, so stills settle exactly and a `frames` scene is a
   sequence at exactly 1/120 s per frame. The backdrop is frozen after the settle.
3. Builds wubble and runs `--smoke gallery --frozen-backdrop --parity OUT/parity.json` at the web
   window's logical size (`WUBBLE_SHOT_SIZE`) and each DPR (`SLINT_SCALE_FACTOR`). The parity
   profile is the design's model: Spaces Personal/Work/Study with their tints and icons, the four
   favourites, and the design's tabs served as pages with its titles and favicons.
4. `score.mjs` pairs each scene's crops: SSIM on luma (gaussian 11, σ 1.5), mean and p95
   CIEDE2000, into `OUT/scores.csv` and `OUT/scores.md` (gating pairs worst first, then the
   informational ones), and a contact sheet per pair in `OUT/sheets/<dpr>x/` (web | wubble | ΔE
   heat: black 0, red 5, yellow 10+; masked regions hatched grey).
5. `report.mjs` writes `OUT/report.html`, the page the owner signs off on (below).

Pieces can run alone: `node tools/fidelity/capture.mjs --out D --only command-bar,spec-toggle-* --dpr 1`,
`node tools/fidelity/score.mjs --web D/web --wubble D/wubble --out D`,
`node tools/fidelity/report.mjs D --baseline-name baseline`,
`FIDELITY_SKIP=web tools/fidelity/run.sh D` (reuse the web shots).
Environment of `run.sh`: `FIDELITY_JOBS`, `FIDELITY_CACHE` (`off` to bypass), `FIDELITY_BASELINE`,
`FIDELITY_WUBBLE_RECTS`, `FIDELITY_DPR`, `FIDELITY_SKIP`, `FIDELITY_PORT`, `WUBBLE_DESIGN`.

### Browser, renderer, speed

Each browser has its own profile and an ephemeral debugging port (`--remote-debugging-port=0`,
read back from `DevToolsActivePort`), in its own process group, removed on exit: runs of several
lanes at once never attach to each other's browser (L0's fixed port 9333 did). A scene's `gl`
picks WebGL: `swiftshader` (the default) or `gpu` (ANGLE on Vulkan). On this box the GPU is
deterministic and pixel-identical to SwiftShader on pages that draw no WebGL (touch kit 0.000
mean ΔE, menu 0.002), but renders the liquid wrong (glass blown out white, 15 mean ΔE on
`sidebar-extended`), so only `/wubble/pages/` scenes (the touch kit and its specimens) may
say `gpu`; a test enforces it. Without a GPU, `gpu` falls back to SwiftShader.

Scenes run in parallel: each worker owns a browser per renderer and navigates fresh per task;
page clocks are per page, so the result does not depend on the job count (measured below). A
crashed browser is relaunched and the task tried once more (a crash retry, not a timing one);
a missing selector fails its scene only.

Measured on the Linux box (2026-11-19, shared with other lanes, load 35 to 75 on 28 cores):
L0's serial capture of its 34 tasks took 4483 s. Touch-kit specimens (GPU, 74 tasks at 1x):
103 s at `--jobs 1`, 32 s at `--jobs 6`; 46 tasks at `--jobs 2`: 49 s cold, 0.3 s warm (all from
the cache). Jobs 1 against jobs 6: rects and curves byte-identical, 1185 of 1345 PNGs identical,
the rest within 0.12 mean ΔE (sequence frames). SwiftShader scenes (the shell, settings and
shell specimens) cost 120 to 330 s each under that load even in parallel: a full cold run is
still long (not measured to the end, to spare the shared box) and is paid once per design commit
or harness change; settings specimens settle the liquid 100 ms only (it never shows inside
them). Settings specimens on the GPU differ from SwiftShader by 0.6 to 1.0 mean ΔE, so they stay
on SwiftShader.

### Web cache

`~/Dev/wubble-wt/notes/fidelity/web-cache/` (`--cache DIR`, `--no-cache`, `--prune`). A task
(scene x DPR) is keyed by sha1 of: the clone's HEAD (plus a hash of its diff and untracked
names when `src/wubble` is dirty), the harness files that drive the browser (`capture.mjs`,
`lib/cdp.mjs`, `lib/specimen.mjs`, `lib/curve.mjs`, `vite.config.mjs`) and `chromium --version`,
the scene's canonical JSON with the globals it inherits (not `wubble`, `surface`, `gate`, `why`:
the scorer's), the DPR and the renderer. `<cache>/<key>/` holds the task's files; a hit is a
copy. Editing one scene re-shoots that scene; a design pull or a harness edit re-shoots all.
`--probe-cache` exits 0 when every task hits; `--prune` keeps the newest key per task and the
last two runs' keys. `<cache>/specimens-index.json` is the last run's specimen index with each
row's cache entry.

### Scenes

`scenes.json` is data: `url`, `steps` (below), `frame` (what is shot, `.wb-window` by default),
`crops` (selectors on the web side; the first match inside the frame is taken). A crop with no
`wubble` rect is **direct**: the same logical rect is cut from wubble's window, since geometry
parity is the point. A **derived** crop (`"wubble": [x, y, w, h]`) scores a wubble-only surface
against the design part it derives from (top-left aligned); `"wubble": "@<element id>"` takes
that rect from a wubble rects file (`score.mjs --wubble-rects F`, below). A scene without
`wubble` is a web-only reference for the lanes building those widgets. `wubble` names a scene of
`crates/wubble/src/gallery_smoke.rs` `SCENES`; to add a pair, add the gallery scene there (data
over host state), then the web scene here.

Also per scene or crop: `surface` (the report's group), `gl`, `gate: false` with a `why` (an
informational pair: reported, not counted against the bar), `mask` (crop only: regions excluded
from SSIM and ΔE, each `{ "web": sel, "pad": px, "toRight": px }` or `{ "rect": [x, y, w, h] }`
relative to the crop; masked pixels of wubble's side take the web's value before the SSIM
windows run, so the mask's edge adds no difference, and means and p95 count unmasked pixels
only; the CSV records the masked share). Masks and gates are declared with a `why`, listed in
the report: a mask can hide a defect, so keep them to text whose content differs by the owner's
rule.

Steps: `key` (with `ctrl`/`shift`/`alt`), `type`, `hover`, `click`, `press` (button down, held),
`release`, `move` (`{ to: sel, dx, dy, fx }` or `{ dx, dy }` from the pointer, one clock frame per
move segment), `drag` (`{ from, by | to, hold, steps }`), `wheel`, `focus` (a Tab key first, then
`focus()`, so `:focus-visible` matches as for keyboard focus), `disable` (only where the design
styles `:disabled`), `holdFor` ms (clock time with the button down), `reveal` (wheel a scroller
until an element is inside it), `advance` ms on the fixed clock, `wait` ms of real time, `until`
a selector exists (real-time turns, up to 20 s, for lazily loaded pages), `eval`.

Fairness: the address pill masks its URL text (the design shows `wubble://start`, wubble the
served URL). The whole-page settings pairs are informational: wubble's settings has its own
sections and wording by the owner's rule. `settings-anatomy` scores the parts instead: nav item
(plain and active), section header, search capsule and a row with each control kind, each
against the element with id `Settings::<part>` in wubble's settings (row titles and notes
masked). Until the settings lane exports those rects (`--wubble-rects`), the anatomy pairs read
"no wubble rect", not a score.

### Specimens

`specimens.json` lists every control the design defines, on the page that defines it (touch kit
`_harness.html?page=touch` on the GPU, settings and the shell `/wubble/` on SwiftShader), with its
`root`, named `elements`, `states` and `sequences` (format in its `about`). `lib/specimen.mjs`
expands it into scenes `spec-<control>--<state>` (a tight crop: root plus the state's `clip`
selectors, plus 8 px) and `spec-<control>--seq-<name>` (frames on the 1/120 s clock with a
curve). Covered: Button pri/glass/ghost, IconButton (rest, hover, press, focus, on, disabled),
Toggle lg/sm (on, off, hover, press, focus, mid-drag; flip), ElasticSlider (rest, hover, focus,
held with the value bubble, dragging, past both ends, at min; press, drag, release spring),
Segmented with icons and accent and the settings one (each position, hover, press on the
indicator, focus, drag stretch; moves), CheckBox (off, on, hover, press, focus; fill, clear),
Swatches (rest, hover, selected, focus; ring glide), Menu (closed, trigger hover/press/focus,
open, highlight on each row kind: shortcut, submenu, long shortcut, destructive, keyboard open;
open with the row stagger, highlight glide, close), Select (closed, hover, open, highlight, open
upward; open), HoldButton (rest, hover, press, 25/50/75 %, done, disabled; hold, drain),
ConfirmButton (idle, hover, press, busy, done; run), SearchCapsule (empty, hover, focused, query,
clear hover; focus, type), Kbd, Scroller thumb (hidden, shown, hover, dragging; show and fade),
Seam + Pill (rest, hover, held, focus; grab), Tooltip (hidden, over icons, destructive; appear,
glide).

Per specimen: `<dpr>x/<scene>.png`, `<scene>.json` (the shot's rect), `<scene>.rects.json` (each
element's `rect` with transforms, `layout` without, and drawn `opacity`, CSS px relative to the
root), and for sequences `<scene>/fNNN.png` and `<scene>/curve.json` (per frame: rect, layout,
opacity, transform matrix, `scale`, `translate`, clip-path, background, width, height of each
tracked element). `specimens-index.json` lists them with the design source line (`@source`
anchors, or `path#text`). `capture.mjs --check-selectors` runs every scene's steps without
shooting and lists selectors that no longer match (an element counts as missing only when it is
absent in every state of its control, since a closed menu has no rows).

A controls lane scores its own kit gallery scene against a state by adding `"wubble":
"<gallery scene>"` to that state.

### Geometry

    node tools/fidelity/geometry-diff.mjs --web WEB.rects.json --wubble WUBBLE.json [--tol 1] [--use layout|rect] [--json]
    node tools/fidelity/geometry-diff.mjs --dir-web RUN/web/1x --dir-wubble target/fidelity

Per element dx, dy, dw, dh (wubble minus web, px), exit 1 when one is over `--tol` (1 px, the
plan's bar) or an element is missing. wubble's file, written by the controls lane's own test
(`init_integration_test_with_mock_time`, then per element
`ElementHandle::find_by_element_id(&ui, "Gallery::knob")`, `absolute_position()` minus the root's,
`size()`, `computed_opacity()`):

    { "specimen": "toggle-lg/on", "scale": 1, "root": "Gallery::toggle-lg",
      "elements": { "track": { "x": 0, "y": 0, "w": 92, "h": 52, "opacity": 1 }, "knob": { ... } } }

Names are the specimen's `elements` keys (the lane maps its element ids to them); `scale` other
than 1 divides the numbers back to logical px. `--use layout` (default) compares the web's
transform-free box, which is what Slint's position and `size()` mean; `rect` compares the drawn
box (transformed states, informational). Curves (`curve.json`) are read by the lanes' curve tests
directly (value comparison at fixed dt, kit-plan 5.3).

The rects file for anatomy crops (`score.mjs --wubble-rects`) is the same idea for a whole
gallery scene: `{ "scene": "settings", "elements": { "Settings::row-toggle": { "x", "y", "w", "h" } } }`
in logical px of wubble's window.

### Report

    node tools/fidelity/report.mjs RUN [--baseline DIR | --baseline-name NAME] [--out FILE] [--inline]

One static HTML page (no network, no fonts, no scripts) from `RUN/scores.csv` and its sheets:
the counts of gating and informational pairs that pass (SSIM >= 0.90 and mean ΔE <= 3), the
gating pairs that lost more than 0.02 SSIM since the baseline, then one section per surface
(scenes.json `surface`), surfaces ordered by their worst gating pair and pairs worst first, each
with its PASS/FAIL (or info) chip, SSIM, mean/p95 ΔE, the change since the baseline, the
web | wubble | ΔE sheet (eight frames of a motion pair), note, mask share and why. Sheets are
linked relative to the page; `--inline` embeds them (one file to send). `run.sh` writes it
against `FIDELITY_BASELINE` (default the L0 baseline, `notes/fidelity/baseline`).

### Limits (see kit-plan.md)

- wubble's frames are not on a fixed clock: a gallery burst shoots as fast as snapshots allow
  (about 120 ms apart on the Linux box) and records each frame's time; the scorer compares each
  with the web frame at the same time. Exact-dt motion comparison is the curve tests' job (Slint
  mock time, `wubble-core` machines, the specimens' `curve.json`).
- The liquid is chaotic and the two sides do not share it: both freeze theirs, but the frozen
  states differ until lane L1 lands a shared frozen dye. Glass crops score the frost over
  different backdrops until then.
- The 2x window must fit the screen the gallery runs on, so 2x uses `viewportByDpr` (a smaller
  window) on both sides.
- Nothing here runs in `cargo test` or CI. `node --test tools/fidelity/test/*.test.mjs` covers
  the pure parts (geometry, metrics and masks, PNG, CSS reading, cache keys, specimen expansion,
  DOM readers, rects comparison, scores CSV, report). Scores are reported, not asserted.

## Kit lock

`crates/wubble-ui/tests/kit_lock.rs` (in `cargo test`): shell `.slint` files compose kit
components only; kit files cite the design; generated files stay generated. Its baseline
(`kit_lock_baseline.txt`) only shrinks: after clearing violations,
`KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock` lowers it (it refuses when any count
grew). Set `WUBBLE_DESIGN` to also check every `design:` citation exists in the clone.
