# wubble.cc

The landing page: plain HTML, CSS and a little JS, no build step. It works from `file://` and from
GitHub Pages at the domain root (`CNAME` holds `wubble.cc`, `.nojekyll` turns Jekyll off). Every URL
is relative, and the page makes no request to anyone but its own host: no fonts, scripts or
analytics from elsewhere, and no GitHub API call.

## Preview

    python3 -m http.server 8000 --bind 127.0.0.1 --directory site
    # then open http://127.0.0.1:8000/, or open site/index.html straight from disk

## A release

The version lives in one place: `VERSION` at the top of `assets/js/site.js`. The download links
(`.../releases/download/v<VERSION>/wubble-<VERSION>-linux-x86_64.tar.xz` and
`-macos-aarch64.dmg`) and every version shown on the page are built from it. Without JS the links
fall back to `https://github.com/xuckless/wubble/releases/latest`. A release bumps that one line;
if the asset names change, `ASSETS` beside it.

## Files

| File | What |
|---|---|
| `index.html` | The page: hero, the app window, Features, How it works, Install, footer |
| `assets/css/site.css` | All styles |
| `assets/js/site.js` | Version and links, OS detection, copy, the download button's stretch, section entrances |
| `assets/js/hero.js` | Mounts the living bubble where the mark is laid out; skipped under `prefers-reduced-motion` or without WebGL2; paused off-screen |
| `assets/js/living.js` | The design's living bubble, bundled (below) |
| `assets/img/start-*`, `settings-*`, `command-bar-*`, `agent-*` | Shot from the app: `--shot-dir DIR --smoke gallery` at `SLINT_SCALE_FACTOR=2` (scenes 77, 58, 56, 93), resized to WebP and JPEG |
| `assets/img/*.svg` (diagrams) | Copies of `docs/readme/` |
| `assets/img/liquid.webp` | A still of the liquid (the design's fluid, 1100 frames, no bubble): the hero's backdrop before the shader starts, without WebGL2 and under reduced motion |
| `assets/img/social-preview.png` | Copy of `docs/readme/social-preview.png` (Open Graph) |
| `assets/img/favicon.svg`, `favicon-32.png`, `apple-touch-icon.png` | The mark (small-size variant for the favicon) |
| `assets/fonts/` | Fredoka, Geist, Geist Mono (SIL OFL 1.1, latin subsets from `@fontsource-variable` 5.3.0, licences beside them) |

## Where the design values come from

The owner's design repo `xuckless/the-wallpaper-project`, `src/wubble`, at `598f814`. Dark only, as
the design is ("light mode comes later", its README).

| On the page | Design source |
|---|---|
| Colours, radii, spacing, fonts, easings | `core/tokens.ts` lines 7-15 (colour), 19-23 (the Personal Space's accent and dye), 26-27 (radius, space), 59-63 (font), 71-77 (motion) |
| Glass (`.thin`, `.regular`), the film rim, press, Overlap keyframes | `core/wubble.css` lines 15-31 (glass-css), 33-44 (film-rim), 46-49 (press), 51-58 (overlap) |
| Hero layout, the command capsule | `pages/start.css` lines 1-20 (start-layout, `.wb-start-search`), `pages/start.tsx` (the bubble at 10% of the shorter side) |
| The living bubble and liquid | `core/living.ts`, `core/gl/scene.ts` (backdrop, blur, glass, bubble and pop shaders), `core/gl/fluid.ts`, `core/gl/glsl.ts`, `core/gl/util.ts`, `core/softbody.ts`, `core/minis.ts`, `core/physics.ts`: bundled unchanged into `assets/js/living.js` |
| Download button: shape and stretch | `pages/touch.css` line 83 (`.tp-dl`), `pages/touch.tsx` lines 300-355 (touch-download, `DL` springs), `core/physics.ts` (spring step, rubber band) |
| "Copied" tip | `pages/touch.css` line 103 (`.tp-tip`) |
| Section heads, plates, entrances | `pages/foundations.css` lines 22-34 (`.fd-sec`, `.fd-head`, `fd-in`), 52 (`.fd-plate`); `pages/found-ui.tsx` lines 34-56 (found-section: 30 ms apart, `-12%` margin) |
| Keys row and its card | `pages/settings.css` lines 40 (`.st-card`), 98 (`kbd`) |
| Nav (concentric segment) | `pages/touch.css` `.tp-seg` (radius 14, padding 4, items 10) |
| The mark | `pages/found-ui.tsx` lines 71-83 (`LOGO`), `browser/icons.tsx` line 48 (`Mark`, small sizes) |
| Icons | `pages/touch-kit.tsx` lines 128-153 (`download`, `duplicate`, `check`) |

Made here, not in the design: the bottom fade of the hero's liquid into the page, the two-column
feature layout, and the light tint on the detected platform's card (the Space colour at 7%, as the
glass shader mixes its tint).

### Rebuilding `living.js`

From the design repo (its `node_modules` has rolldown), with an entry file outside the repo:

    // entry.ts
    import { mountLiving } from '<design repo>/src/wubble/core/living';
    (window as any).wubbleLiving = mountLiving;

    ./node_modules/.bin/rolldown entry.ts --format iife -o living.js

then put the header comment back on top.
