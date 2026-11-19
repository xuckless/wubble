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
tiers, fonts, type roles, motion, the CSS ease-out table, keyframe tables), `gen/text.slint` (the
text roles: the type scale and every font rule of the shell's stylesheets, for KitText), `gen/css.slint` (every typed declaration of the design's
stylesheets, `<selector>--<property>[-part]`, one global per file, the unread ones listed),
`gen/icons.slint` (browser and kit icon tables, Space and settings icons, every inline path, the
mark, app-icon plates) and `crates/wubble-core/src/kit_gen.rs` (the motion constant tables).
Each starts with the design commit and an FNV-1a hash of its body; the kit lock fails on a hand
edit. When the clone moves: `git -C <clone> pull`, run the generator, commit the result with the
lanes' call-site changes.

    node tools/fidelity/shell-map.mjs [--accepted]   propose the generated value for each Shell literal
    node tools/fidelity/curves.mjs                   sample the web's fixed-curve motion

`shell-map.mjs` ranks, for each literal left in `kit/derived.slint`, the generated properties of
equal value by the CSS rule its comment cites and the property kind; it prints, never writes (an
alias is a reviewed hand edit). `curves.mjs` loads `core/wubble.css` with its token variables in
headless Chromium, pauses every animation and transition, steps them on the fixed 1/120 s clock
and writes `tools/fidelity/curves/<motion>.json` (opacity, translate y, scale x/y, rotation,
blur, box-shadow spread and alpha, a fold's height
per frame): the truth `crates/wubble-ui/tests/motion_curves.rs` and wubble-core's motion
tests compare the kit with (>= 0.90 per property, mock time, no wall clock). Rerun it when the
clone moves and commit the JSON.
Inline literals (numbers inside the design's functions, not in a table) are listed in
`literals.mjs` by needle: the exact source text with the value marked `⟦ ⟧`, which must occur
once in its file, so a changed value fails `--check`. They land in `kit_gen.rs` (`pub mod
scene / softbody / minis / living / start`) and, when marked `slint`, in `gen/literals.slint`.

    node --disable-warning=ExperimentalWarning tools/fidelity/sample-start.mjs

samples the design's own `SoftBody` and `Minis` (imported through `lib/ts-loader.mjs`, node's
`stripTypeScriptTypes`), its `living.ts` loop (transcribed) and `wTexture()` on the fixed 1/120 s
clock with a recorded `Math.random`, into `crates/wubble-core/tests/fixtures/start_*.json`, which
`wubble_core::start`'s tests replay. Re-run it when the design's start-page code changes.

## Harness

    tools/fidelity/run.sh [OUT]          # everything; OUT defaults to notes/fidelity/runs/<date>

1. Serves the clone with `vite.config.mjs` (port 5199): a fixed clock and a seeded `Math.random`
   injected at serve time, a frozen-backdrop switch patched into `core/gl/scene.ts` at serve time,
   and the parity pages (`/__parity/manifest.json`, `/__parity/tab/<id>`).
2. `capture.mjs` shoots each scene of `scenes.json` in headless Chromium (WebGL on SwiftShader)
   at DPR 1 and 2. Time moves only by `__wb.step(1000/120)`; CSS animations and transitions are
   paused and set to the same clock each step, so stills settle exactly and a `frames` scene is a
   sequence at exactly 1/120 s per frame. The backdrop is frozen after the settle: the emitters
   stop and the liquid is replaced by wubble's still dye (`crates/wubble-gfx/src/scene/still_dye.txt`,
   the browser window's own backdrop), splatted into the `.wb-window` rect on the desktop canvas
   (`lib/still.mjs`), so both sides show one dye.
3. Builds wubble and runs `--smoke gallery --frozen-backdrop --parity OUT/parity.json` at the web
   window's logical size (`WUBBLE_SHOT_SIZE`) and each DPR (`SLINT_SCALE_FACTOR`). The parity
   profile is the design's model: Spaces Personal/Work/Study with their tints and icons, the four
   favourites, and the design's tabs served as pages with its titles and favicons.
4. `score.mjs` pairs each scene's crops: SSIM on luma (gaussian 11, σ 1.5), mean and p95
   CIEDE2000, into `OUT/scores.csv` and `OUT/scores.md` (worst first), and a contact sheet per
   pair in `OUT/sheets/<dpr>x/` (web | wubble | ΔE heat: black 0, red 5, yellow 10+).

Pieces can run alone: `node tools/fidelity/capture.mjs --out D --only command-bar --dpr 1`,
`node tools/fidelity/score.mjs --web D/web --wubble D/wubble --out D`,
`FIDELITY_SKIP=web tools/fidelity/run.sh D` (reuse the web shots).

### Scenes

`scenes.json` is data: `url`, `steps` (`key`/`ctrl`, `type`, `hover`, `click`, `drag`, `wheel`,
`advance` ms on the fixed clock, `wait` ms of real time, `until` a selector exists (real-time turns, for lazily loaded pages), `eval`), `frame` (what is
shot, `.wb-window` by default), `crops` (selectors on the web side). A crop with no `wubble` rect
is **direct**: the same logical rect is cut from wubble's window, since geometry parity is the
point. A **derived** crop (`"wubble": [x, y, w, h]`) scores a wubble-only surface against the
design part it derives from (top-left aligned). A scene without `wubble` is a web-only reference
(the touch kit, settings controls, Foundations) for the lanes building those widgets. `wubble`
names a scene of `crates/wubble/src/gallery_smoke.rs` `SCENES`; to add a pair, add the gallery
scene there (data over host state), then the web scene here.

### Limits (see kit-plan.md)

- wubble's frames are not on a fixed clock: a gallery burst shoots as fast as snapshots allow
  (about 120 ms apart on the Linux box) and records each frame's time; the scorer compares each
  with the web frame at the same time. Exact-dt motion comparison is the curve tests' job (Slint
  mock time, `wubble-core` machines).
- Both sides show the still dye, from one table (`still_dye.txt`). What still differs: the dye's
  grid (256 texels on the short side of the web's whole desktop canvas, of wubble's window on
  wubble) and the scene clock (frozen at the settle's time on the web, at 0 on wubble: only the
  touched film rim's drift depends on it).
- The 2x window must fit the screen the gallery runs on, so 2x uses `viewportByDpr` (a smaller
  window) on both sides.
- Nothing here runs in `cargo test` or CI. `node --test tools/fidelity/test/*.test.mjs` covers
  the pure parts (geometry, metrics, PNG, CSS reading, the still dye's mapping, the ease table,
  text roles). Scores are reported, not asserted.

## Kit lock

`crates/wubble-ui/tests/kit_lock.rs` (in `cargo test`): shell `.slint` files compose kit
components only; kit files cite the design; generated files stay generated. Its baseline
(`kit_lock_baseline.txt`) only shrinks: after clearing violations,
`KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock` lowers it (it refuses when any count
grew). Set `WUBBLE_DESIGN` to also check every `design:` citation exists in the clone.
