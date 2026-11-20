# Design fidelity: generator, motion fixtures, kit lock

The web version of the design (`the-wallpaper-project`, `src/wubble/{browser,core,pages}`) is the
truth for look, motion, effects and widget behaviour. These tools take values from it and measure
wubble against it. Plan and scope: [docs/fidelity/kit-plan.md](../../docs/fidelity/kit-plan.md). The design clone
is `$WUBBLE_DESIGN` (else the default clone path in `lib/design.mjs`); nothing here edits it. Node
22 and `/usr/bin/chromium-browser` are the only requirements (no npm packages of our own; the
clone's own `npm install` provides vite).

## Generator

    node tools/fidelity/extract.mjs                write the generated kit files
    node tools/fidelity/extract.mjs --check        exit 1 if they are stale against the clone
    node tools/fidelity/extract.mjs --report F     where generated values differ from wubble's
    node tools/fidelity/extract.mjs --coverage INVENTORY.md   (the pre-phase design inventory, not kept in the repo)

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

    node --disable-warning=ExperimentalWarning tools/fidelity/sample-orbs.mjs

samples Ultra Reactivate's thinking-orb engine (`vendor/thinking-orbs`: every preset at six times,
two knob settings, the ink ramps) into `crates/wubble-core/tests/fixtures/orb_frames.json`, which
`wubble_core::orb`'s tests replay. The generator copies the engine's TypeScript it translates into
`ts-snapshot/` (with `COMMIT`); `crates/wubble-core/tests/orb_constants.rs` diffs each translated
file's numbers against it, as `shader_constants.rs` does for the GLSL.

## Looking

There is no capture or scoring tool here any more (removed 2026-11-19: a cold run of the web
captures took over an hour on the owner's machine). Review is by eye: `--shot` / `--shot-dir` PNGs
of wubble beside the web design. `curves.mjs` and `controls-curves.mjs` still sample the design's
motion on a fixed 1/120 s clock into the JSON fixtures the unit tests replay.

## Kit lock

`crates/wubble-ui/tests/kit_lock.rs` (in `cargo test`): shell `.slint` files compose kit
components only; kit files cite the design; generated files stay generated. Its baseline
(`kit_lock_baseline.txt`) only shrinks: after clearing violations,
`KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock` lowers it (it refuses when any count
grew). Set `WUBBLE_DESIGN` to also check every `design:` citation exists in the clone.
