# Motion / interaction / VFX audit: design repo vs wubble

Question: Which motions, interactions and effects of the design does wubble match, differ on, or lack (as of fb/motion, 2026-11-18)?
Status: Historical: the audit drove the motion lane; the kit's motion primitives (design-fidelity phase) since replaced most rows.

Design: the design repo (`xuckless/the-wallpaper-project`, `src/wubble`) @ 21838b8 (paths below are relative to it).
wubble: branch `fb/motion`, base `4a7bb5e`. Paths relative to `crates/`.

Scope (the owner's words): copy the design's feel (motion, effects, interaction behaviour,
components), not its content or product. Rows for the design's demo-only specimens (start-page
bubble, minis, pop, voice glow, Room, dock magnification) are listed so the inventory is complete,
marked n/a where wubble has no such surface.

Status key: **OK** present and matching · **DIFF** present but different (how) · **MISSING** ·
**n/a** no such surface in wubble. "Before" is `4a7bb5e`; "Now" is this branch.

## Counts

93 rows (counted from the tables below; "other" = wubble-only rules and mixed cells such as the
beam's hue or the loading hold).

| | Before | Now |
|---|---|---|
| OK (incl. documented approximations) | 35 | 52 |
| DIFF | 20 | 11 |
| MISSING | 17 | 9 |
| n/a | 10 | 9 |
| other | 11 | 12 |

## 1. The border beam (the owner's "ring of glow when changing tab / tab loading")

What the design draws: `browser/chrome.tsx:31-58` (`@source address-loading`). A beam of light
(head plus smootherstep tail) laps the **address bar** (thin glass, r 12), not the page card. It
runs whenever the address signature changes: `sig = tab id : history position : loads : location`
(tab switch, navigation, back/forward, reload), never on the first render, never under reduced
motion. One lap is 1500 ms: head `0.08 + e·1.35` with `e = 1 − (1 − k)³`; strength
`min(1, 12.5k) · fade²`, `fade = 1 − smootherstep((k − .35)/.65)` (bright in 120 ms, fading from
35% of the lap); tail `0.35 + 0.3f` of the perimeter; hue `Math.random()`. Drawn by the glass
shader as stroke + inner glow + outer bloom in the cyclic brand sheen (`core/gl/glsl.ts BEAM`,
port `slint/shaders/glass.wgsl` uBeam). README feedback: "the loading streak should fade out
gently → the address beam fades over 1.5 s". The command bar has its own "thinking" beam:
`browser/command.tsx:66-81`, 1000 ms per edit, head `0.1 + 0.9k`, tail 0.3, strength
`min(1,5k)·max(0, 1 − max(0, k − .5)/.5)`, hue 0.2. The split view's focused pane has a static
1.5 px accent ring (`browser.css:113`, `webview.tsx:123`), not a glow.

| Item | Design | Before | Now |
|---|---|---|---|
| Beam trigger: tab switch | `chrome.tsx:37-39` sig | **MISSING**: only `address-loading`'s rising edge lapped (`wubble/src/backdrop.rs:250`) | **OK**: `wubble_core::beam::Watch` → `Event::Shown` (`wubble-core/src/beam.rs`), stepped in `wubble-ui/src/driver.rs step_beam` |
| Beam trigger: navigation / loading | sig (loads, at) | **MISSING in practice**: `address-loading` was never set by the host (only tests set it), so no beam ever ran | **OK**: host sets it from the visible view's `Phase::Loading` (`wubble/src/bind.rs render_history`, `host.rs after_change`); `Event::LoadStarted` |
| Beam while loading longer than a lap | (design pages load within one lap) | one lap, then nothing | **wubble rule**: the lap holds at k = 0.35 (full strength, head running on at the curve's speed there) while loading |
| Beam at load end | fade over the lap's last 65% | n/a | **OK**: `Loading → Lapping` continues the curve from 0.35: the design's 975 ms fade, continuous |
| Beam curve | `chrome.tsx:44-53` | **OK** (`lap_beam_at`, gfx) | **OK**, moved to `wubble_core::beam::shape_at` (one owner); gfx takes the shape per frame (`Scene::set_beam_shape`) |
| Beam hue | `Math.random()` | scene Rng | golden-ratio sequence per lap (`beam::hue`), deterministic |
| Beam place | address bar glass | **OK** (address pill's glass item) | **OK** |
| Beam reduced motion | none under `reducedMotion()` | n/a (no reduced motion) | **OK** (b8/shell): the driver sends `Event::Reduce` instead of `Shown` / `LoadStarted` while reduced |
| Slint fallback (no scene, peek over the page) | (web: CSS only) | **DIFF**: static film ring + linear sheen sweep, not the beam's shape or timing | **OK (approx.)**: `BorderBeam` (`wubble-ui/ui/components/border-beam.slint`) traces the same shape round the pill: 10 tail strokes at the shader's `(1 − behind/len)^2.2`, the same cyclic sheen, plus a 5 px bloom stroke; flat (no blur) |
| Command bar thinking beam | `command.tsx:66-81` | **MISSING** | **OK**: `beam::Thinking`, driver watches `command-query`; `BorderBeam` on the command bar (Slint: the bar is over the page) |
| Split focus ring | `browser.css:113` 1.5 px accent | **OK** (static) | **OK** |

## 2. Press, hover, film rim, sheen, pointer light

| Item | Design (file:line, parameters) | Before | Now |
|---|---|---|---|
| Press 0.97 on pressables | `core/wubble.css:44-45` `.wb-press`: 0.97 in 120 ms ease-out, back 200 ms ease-out | **OK** on PressableButton and everything inheriting it, TabRow, Space tiles, fav tiles, NewTabRow, CommandRow (`components.slint:212`, …) | **OK** |
| Address press | `browser.css:44` 0.985, .2 s ease-out | **OK** (`address-pill.slint`, Shell.address-press) | **OK** |
| Window lights press | `browser.css:30` scale(.8, 1.15), .3 s ease-spring; hover colours | **OK** (`window-lights.slint`) | **OK** |
| Fav hover | `browser.css:55` icon 1.1, .36 s ease-spring | **OK** (`fav-grid.slint:74`) | **OK** |
| Hover fades (rows, icon buttons) | `.wb-ib` .15 s; `.wb-tab:hover` | **OK** (150 ms easeOut) | **OK** |
| Tab close button reveal | `browser.css:75-78` opacity 0→1 .15 s, scale .8→1 .2 s ease-out, 22 px tile r 7, white 10% on hover | **DIFF**: instant `visible` flip, no tile | **OK** (`components.slint` TabRow; tokens `tab-close-radius`, `tab-close-hover`) |
| Film rim (soap-film ring) on hover/press/drag/focus | `core/wubble.css:30-40` 1.5 px conic Sky→Lilac→Blush→Mint at .8, in .2 s, out .15 s, swirl 4 s; GL: `scene.ts:253` uFilm | **OK** in the scene (binary film per surface, as the design's GL path); **OK (approx.)** in Slint (linear sweep, 200/150 ms) | **OK** |
| Film rim coverage | glass the hand touches (address, favs, window while dragged) | address, favs | same |
| Sheen ring on the peek panel | (design: none; peek is plain thick glass) | wubble-only steady GlowRing while peeking | unchanged (consistent with "sheen only where touched": the pointer is on it) |
| Pointer light / specular on glass | `scene.ts:630-633` light lags `1 − e^(−18dt)` | **OK** (`wubble-gfx/src/scene/state.rs advance`) | **OK** |
| Pointer pushes the liquid | `scene.ts:592-600` | **OK** (`fluid_ops`) | **OK** |
| Press wave through glass | `chrome.tsx:77` `scene.wave(surface, x, y, 90, 0.7)` on press | **OK** (front-most glass) | **OK** |
| Tap ripple through the liquid | `desktop.tsx:358-368` bare desktop only, `ripple(x, y, 240, 0.6)` | **DIFF**: every left press anywhere (also over the page and on glass) rippled at 260 / 1.0 | **OK**: glass → wave only; bare backdrop → ripple 240 / 0.6; nothing over the page (`state.rs tap`, `host.rs window_event`, `input::over_content`) |
| Release squash on buttons | `touch.tsx` Buttons: squash token at 1/3 (1.08×.88 → .97×1.04 → 1.01×.99, 500 ms) | **MISSING** | **OK** (b8/shell): `squash.slint` on `PressableButton` (so `SheetButton`, `IconButton`) and `TextButton`, a third of the token |
| Cursor feedback | `cursor: pointer / text / col-resize / grab` | **OK** (Slint mouse-cursor where set) | **OK** |

## 3. Overlap: menus, sheets, popovers, toasts, page switches

Design: `core/wubble.css:49-54` `wb-in` (240 ms ease-out from 7 px below at .98, blur 4) and
`wb-out` (190 ms in-out, 5 px up); dropdowns `tokens.ts:257` in 220 / out 176 / from .95, rows
30 ms apart (`touch.tsx` Menu, `command.tsx`).

| Item | Design | Before | Now |
|---|---|---|---|
| Command bar open/close from the invocation point | `command.tsx:117-122`, `browser.css:166-168` 220 / 176 | **OK** (`command-bar.slint`, real exit window) | **OK** |
| Command rows stagger | `browser.css:176` wb-in, `60 + 30i` ms | **OK (approx.)** OverlapLayer 30i | same |
| Command bar scrim | `browser.css:163-164` .2 s / .16 s | **OK** | **OK** |
| Context menu | menu: dropdown 220 / 176 in-out, from the trigger, rows 30 ms | **DIFF**: 200 / 160 easeIn (Ultra Reactivate), **exit never seen** (`visible: open`) | **OK**: dropdown tokens, exit drawn until faded on the last open menu's rows (latched), scrim and rows inert while leaving |
| Downloads popover | as menu | **DIFF**: same two problems | **OK** |
| Find bar | as menu | **DIFF**: 200 ms both ways, exit never seen | **OK** |
| Permission prompt | as menu | **DIFF**: 200/160 easeIn, exit never seen | **OK** (request latched while leaving; buttons answer only while open) |
| Autofill menu / save prompt | as menu | **DIFF** 200/160 easeIn; save prompt exit never seen | **OK** (b8/shell): the save prompt keeps its view while it leaves |
| Dialog / sign-in / bookmark sheets | (design has no sheet; Ultra Reactivate `.ur-o-dialog` 300 ms) | **DIFF**: exit never seen (`visible: open`) | **OK** (b8/shell): `SheetFrame` stays drawn while it fades; the dialog and bookmark sheets keep their view; the pages' sheets keep their own fields |
| Error panel | (none in design) | opacity 300 ms, exit unseen | **OK** (b8/shell): exit seen, the panel keeps its view |
| Toast | (none in design; wubble follows Ultra Reactivate: rise 14 px, 400 ms spring, out 190) | **OK** (exit visible) | opacity-in now on the dropdown token (220 ms) |
| Page switch (tab change) | `webview.tsx:72-85,118` shown view `wb-in`, leaving view `wb-out` 190 ms underneath | **MISSING**: frames swap in one frame | **OK** (b8/shell): the outgoing page's last frame leaves under the incoming one (190 ms in-out, 5 px up), one pane only, not when reduced |
| OverlapLayer re-entry offset | `wb-in` from +7 px | **DIFF** (port note: re-entry starts at −5 px) | unchanged (port's documented approximation); the new `Entering` uses +7 exactly |
| Settings row fold / reset reveal / clear button | `pages/settings.css:18-19,52-60` | wubble settings has none of these controls | n/a |

## 4. Tab list

| Item | Design (`browser/tabs.tsx`, `browser.css`) | Before | Now |
|---|---|---|---|
| Row springs at slot·pitch (k 300, c 26) | `tabs.tsx:52-63` | **OK** (`wubble-core/src/tabs.rs`) | **OK** |
| Drag reorder: 6 px start, 1:1, band 18 past ends, lift 1.025 (k 500 c 30), others make room | `tabs.tsx:90-114` | **OK** | **OK** |
| Active indicator: follow (300/15) + stretch (+35% at 1500 px/s, k 700 c 24), cross axis 1/√s, trailing-edge origin | `tabs.tsx:64-73` | **OK** (`sidebar.slint` indicator) | **OK** |
| Row insert | `browser.css:66` `.wb-tab` runs `wb-in` (240 ms, 7 px, .98) | **MISSING**: rows appear in place (the OverlapLayer's `rows-shown` was never flipped) | **OK**: `RowAnim::enter`, `tabs.rs`; `row-enter` model; `Entering` (`ui/components/entering.slint`) |
| Row remove | `browser.css:71-72` `wb-tab-out` 190 ms in-out (fade, blur 3, .94) before the row leaves | **MISSING**: removed at once | **OK (approx.)**: the driver keeps what the row showed and draws it leaving where it was (`tabs::Leaving`, `leaving-rows`), 190 ms in-out to clear at .94; no blur. The design delays the close; wubble's host has already closed it, so the other rows close the gap meanwhile |
| Space switch: whole list re-enters | `chrome.tsx:289` `TabList key={sp.id}` | **MISSING** | **OK** (every row is new, so every row enters) |
| Middle click closes | `tabs.tsx:91` | **OK** | **OK** |

## 5. Spaces

| Item | Design | Before | Now |
|---|---|---|---|
| Capsule follows (300/15) and stretches between icons, pitch 38 | `tabs.tsx:139-169` | **OK** (`tabs.rs SpaceSwitcher`) | **OK** |
| Capsule colour fade .4 s in-out, alpha .2 | `browser.css:94` | **OK** (`sidebar.slint:465`) | **OK** |
| Icon wobble on pick (−14° → 9° → −4°, 550 ms) | `tabs.tsx:163-165`, `wubble.css:58` | **OK** (`wobble.slint`) | **OK** |
| Space name enters | `chrome.tsx:286` span keyed by Space, `wb-in` | **MISSING** | **OK** (`space-enter`, driver) |
| Backdrop dye follows the Space, ~600 ms crossfade (`1 − e^(−5dt)`) | `scene.ts:627-629`, `desktop.tsx:282-286` | **OK** (`state.rs advance`, `backdrop.rs set_palette(dye(tint))`) | **OK** |
| Segmented Space switcher (grab and fling the capsule, masked labels) | `touch-kit.tsx:360-490` | n/a in the sidebar (icons, not a segmented control) | n/a |

## 6. Sidebar, seam, split, window

| Item | Design | Before | Now |
|---|---|---|---|
| Three modes on one width spring (release 260/24), contents a beat behind (210/21) | `seam.ts:59-80` | **OK** (`wubble-core/src/seam.rs`) | **OK** |
| Seam held 150/20, band 34, bow (bend k 70, twang 520/9), snap to rail/hidden | `seam.ts:15-23` | **OK** | **OK** |
| Sections ride the bowed string | `chrome.tsx:180-183` | **OK** (`bow.rs`, Ride) | **OK** |
| Seam line: white 10% 1.25 px at rest, sky 1.6 px while held | `browser.css:140-141` | **DIFF**: invisible at rest, sky hairline 0.6 on hover, bowed path 1 px sky .6 | unchanged (static look; left to fb/look) |
| Seam grab pill (6×40; hover .8 at .75; held sky, 1, ease-spring) | `browser.css:143-146` | **MISSING** | **MISSING** |
| Peek in from the left edge: 60 ms intent, `wb-peek-in` 340 ms ease-out from −100%−16 px at .2 | `chrome.tsx:250-257`, `browser.css:20-21` | **OK** | **OK** |
| Peek out | design: drops at once after 260 ms | wubble animates out 340 ms (wubble-only, harmless) | same |
| Split: pane opens from zero width, closes by merging seams, strings as the sidebar's | `split.ts` | **OK** (`wubble-core/src/split.rs`, parity) | **OK** |
| Split held line fade (.2 s in, .15 s out) | `browser.css:114-115` | **DIFF**: drawn while bowed, no fade | unchanged |
| Pane resize stretch | (wubble rule from `desktop.tsx WIN`) | wubble-only (`stretch.rs`) | same |
| Window move/resize spring, throw, stretch 3.5% | `desktop.tsx:122-133` | n/a: the OS moves the native window | n/a |
| Window open (from y+24 at .96, alpha 0) / minimise into a bubble / restore | `desktop.tsx:239,253-281` | **MISSING** (native window; the content could fade in on launch) | **MISSING** |
| Rounded corners spring to 0 on maximise (k 300 c 30) | `desktop.tsx:190` | **DIFF**: instant | unchanged |

## 7. Scrolling

| Item | Design (`browser/scroller.tsx`) | Before | Now |
|---|---|---|---|
| Wheel on a follow spring (520/46), rubber band k 34·3, release 420/38, momentum budget 160 ms, fling e^(−3.2t) | `scroller.tsx:162-173` | **OK** for the sidebar tab list (`wubble-core/src/scroller.rs`) | **OK** |
| Native pages (settings, history, bookmarks, downloads, passwords) | the design's pages all sit in a `Scroller` | **DIFF**: wheel clamped 1:1, no spring, no overscroll, no touch, passive thumb (`settings.slint PageFrame`) | **OK**: `PageScroll` global (`ui/components/page-scroll.slint`) on a second core `Scroller` in the driver |
| `glide` spring (170/26) for programmatic jumps | `scroller.tsx:165` | **MISSING** in core | **MISSING** (nothing in wubble jumps to a section yet) |
| Thumb: 6 px, .38, widens to 9 on hover, squashes at the ends | `browser.css:127-130`, `scroller.tsx:202-211` | **DIFF**: 4 px .55 (the port's), no hover width; squash computed by core `render` | unchanged (static look) |
| Command bar results, downloads popover | design: no scrolling lists there | std ScrollView / Flickable | unchanged |
| Web pages | Chromium scrolls them | n/a | n/a |

## 8. Controls (Touch everything)

| Item | Design (`pages/touch.tsx`, `touch.css`) | Before | Now |
|---|---|---|---|
| Toggle: follow spring, stretch, wall squash scaled by impact, fill blooms out of the knob, hold widens 14%, drag 1:1 band 26, fling decides | `touch.tsx:155-235` | **DIFF**: 240 ms ease-spring tween on x, colour fades | **OK** (b7: `controls::Toggle`) |
| Segmented: follow + stretch, grab and fling, masked labels | `touch-kit.tsx:371-490` | **DIFF**: 240 ms ease-spring slide | **OK** (b8/shell): `controls::Segmented` (follow k300 c15, width on the stretch preset, grab and fling .1 s, band 26, wall 6), the dark labels clipped to the indicator |
| Elastic slider: 1:1 with band 26, track press catch-up 900/48, knob swells 1.1, stretch, value bubble | `touch.tsx:28-120` | **DIFF**: no motion at all | **OK** (b7: `controls::Slider`) |
| Checkbox: circular fill 240 / 190, tick 70 ms behind, press squeeze 1.12×.9, squash on fill | `touch.tsx:545-575`, `touch.css:160-174` | **DIFF**: colour fade only | **OK** (b8/shell): the box squashes at full strength as it fills |
| Gliding tooltip (intent 280 ms, warm 520 ms, glide 260 ms in-out) | `touch.tsx:360-430` | **MISSING** (no tooltips at all) | **MISSING** (a product surface; see Remaining) |
| Dock magnification | `touch.tsx:360-380` | n/a (no dock) | n/a |
| Pin ribbon twang | `touch.tsx:240-300` | n/a (no pin control) | n/a |
| Download button stretch | `touch.tsx:302-355` | n/a (wubble's download rows have no such button) | n/a |
| Download row progress | (none) | width 200 ms | width 220 ms (dropdown-in token) |
| Hold-to-clear (1.1 s fill, stretch, pulse) | `settings.tsx:303-350` | **MISSING** (clear data is a sheet) | **MISSING** |
| Confirm-in-place + pulse | `settings.tsx:353-370`, `wubble.css:62` | **MISSING** | **MISSING** |
| Menu highlight glides between rows (.16 s) | `touch.css:154-158` | **DIFF** (context menu: per-row hover fade); command bar **OK** (`command-sel-ms` 180) | unchanged |

## 9. Living backdrop and glass

| Item | Design | Before | Now |
|---|---|---|---|
| Stable-fluids backdrop, slow emitters, dark ground | `core/gl/fluid.ts`, `scene.ts:646-660` | **OK** (`wubble-gfx` scene, port) | **OK** |
| Blur pyramid, glass tiers, refraction, dispersion, 1 px rim, specular | `scene.ts`, `glass.wgsl` | **OK** | **OK** |
| Quality degrades before motion drops a frame | (wubble rule) | wubble-only governor | same |
| Bubble, minis, pop, voice glow, facing | `core/softbody.ts`, `minis.ts`, `living.ts` | n/a (no start page bubble yet; TODO "Glass backdrop") | n/a |

## 10. Global

| Item | Design | Before | Now |
|---|---|---|---|
| Reduced motion / Calmer motion | `wubble.css:69-71` (1 ms), `touch-kit.tsx:196-229` (springs snap), `chrome.tsx:56` (no beam) | **MISSING** | **OK** (b8/shell): setting `ui.reduce_motion` + the OS on auto (Linux portal; macOS / Windows not read yet), `motion::reduced` + Slint `Calm` |
| Wobble keyframes on icon hits | `wubble.css:58-63` | **OK** (`wobble.slint`, favs and Space tiles) | **OK** |
| Squash keyframes (`wb-sq`) | `wubble.css:60-61` | **MISSING** | **OK** (b8/shell): `squash.slint`, a text test pins it to `motion::SQUASH_KEYS` |
| Snapshot reveal (top-down mask .7 s) | `browser.css:132-136` | n/a (real pages paint) | n/a |
| Loop rests at REST 0.02, dt cap 32 ms | `physics.ts:181-209` | **OK** (`motion.rs`) | **OK** |

## wubble-only motion (not in the design)

- Peek panel's steady film ring and its 340 ms exit (§6): consistent with touch sheen; kept.
- Pane resize stretch and stale-frame fade (`wubble-core/src/stretch.rs`): owner rule "Resize feel"; kept.
- Toast (Ultra Reactivate's rise and spring): kept; its in-fade now uses the design's dropdown time.
- Sheets' 300 ms grow-in (Ultra Reactivate): kept.
- Error panel 300 ms fade: kept.
- Glass quality governor: kept.

## Remaining, and why

- Outgoing page during a tab switch (`wb-out` under the new page): needs the old pane's last
  frame kept for 190 ms (host/panes), like the resize stretch's stale frame.
- Sheets', error panel's and the autofill save prompt's exits: each needs its view latched while
  it leaves (done for the context menu and permission prompt).
- Toggle / segmented / slider physics (spring, wall squash, band, fling): a per-control Rust
  state; today they are Slint tweens on the design's CSS-fallback curve.
- Button release squash, checkbox fill squash, `wb-sq`: need a keyframe driver like `Wobble`.
- Tooltips and their glide: no tooltips exist in wubble; adding them is a product decision.
- Reduced motion: no source (setting or OS); the beam is ready for it.
- Seam pill and the seam's rest/held stroke: static look (fb/look's area), listed here only.
- Window open/minimise motion: the native window; at most a content fade on launch.
