# Derived values for the owner's review (lane L2, ui1/motion)

Status (2026-11-19): the owner's review sheet of pieces with no web design counterpart; awaiting the owner. Lane screenshots it names were kept outside the repo; reshoot with `--smoke gallery`.

The `Shell` values with no equal in the web design (`baseline/gen-diff.md`, 29), as they now
stand in `crates/wubble-ui/ui/kit/derived.slint`'s `// derived:` block: value, where used, and
the design rule each derives from. "was" marks a value L2 changed toward the design.

| Shell | value | used in | derived from |
|---|---|---|---|
| `selection-fill` | `Palette.sky.transparentize(0.65)` | components/command-bar.slint, components/dialog-sheet.slint, components/find-bar.slint, components/settings.slint | the Space accent sky (core/tokens.ts color.sky) at .35: selected text in a field; the design styles no ::selection |
| `check-tick-delay` | `70ms` | components/check-box.slint | pages/touch.tsx:554 CHECKS.tickDelay (generated in kit_gen.rs touch::CHECKS, not in gen/*.slint) |
| `tab-exit-shrink` | `1 - BrowserCss.wb-tab-out-k-to--scale` | components/sidebar.slint | browser/browser.css:72 @keyframes wb-tab-out to scale(.94) |
| `beam-segments` | `10` | components/border-beam.slint | wubble-only: the Slint border beam stand-in (GLASS beam is scene-drawn in the design); strokes in its tail |
| `beam-bloom-alpha` | `0.3` | components/border-beam.slint | wubble-only: the stand-in beam's bloom stroke (the design blurs it in the shader) |
| `thick-unblurred-fill` | `rgba(28, 31, 40, 0.94)` | kit/material/glass.slint | .wb-thick over live content (Glass.over-content .42) is blurred in the design; without the page in the scene the fallback needs body (owner decision 3). Since L1 s2 the scene frosts the page under overlays; this fill is left to the Slint-only glass and a fullscreen page, for every tier above the page since qa1/glass |
| `window-width` | `1280px` | app.slint | wubble-only: the first window's size (the design is a page in a browser window) |
| `window-height` | `800px` | app.slint | wubble-only: as window-width |
| `address-icon-stroke` | `1.9` | components/address-pill.slint | browser/chrome.tsx address lock icon drawn at stroke 1.9 (icons.tsx default 1.7) |
| `drag-ghost-shadow` | `BrowserCss.wb-tab-drag--box-shadow-shadow-1-color` | app.slint | browser/browser.css:70 .wb-tab-drag shadow 0 10px 24px .4 (was .25) |
| `sidebar-open` | `0.5px` | app.slint | technical: a sidebar narrower than half a pixel counts as hidden |
| `no-clip` | `100000px` | components/address-pill.slint, components/fav-grid.slint, kit/material/glass.slint | technical: an unbounded clip range for GlassPanel |
| `peek-in` | `60ms` | app.slint | browser/chrome.tsx:254 setTimeout(setPeek(true), 60) |
| `peek-out` | `260ms` | app.slint | browser/chrome.tsx:256 setTimeout(setPeek(false), 260) |
| `command-bar-width` | `600px` | components/command-bar.slint | browser/browser.css:166 .wb-cmd width: min(600px, calc(100% - 32px)) (not read by the generator) |
| `sheet-title-weight` | `Role.st-sec-head-h2.weight` | components/sheet-frame.slint | wubble-only sheet title, 19 px like .st-sec-head h2 (settings.css:36): its weight 600 (was 560) |
| `sheet-body` | `Role.body.size` | components/error-panel.slint, components/pip-card.slint, components/sheet-frame.slint | wubble-only sheet body text: the type scale body (core/tokens.ts type.body 14; was 14.5) |
| `tip-glide` | `260ms` | components/tooltip.slint | pages/touch.css:101 .tp-tip-pos transition var(--tp-glide) = touch.tsx DOCK.glideMs (kit_gen.rs touch::DOCK) |
| `toast-weight` | `Role.st-row-title.weight` | components/preview-card.slint, components/toast.slint | wubble-only toast text, 14 px like .st-row-title (settings.css:45): its weight 500 (was 540) |
| `toast-shown` | `1500ms` | `wubble_core::overlay::TOAST_SHOWN_MS` (the host times the toast) | wubble-only: how long a toast stays; no design counterpart |
| `boost-code-height` | `150px` | components/boost-panel.slint | wubble-only: the Boost editor's code area (a missing primitive, wubble-only.md) |
| `pip-width` | `400px` | app.slint | wubble-only: picture-in-picture window, 16:9 |
| `pip-height` | `225px` | app.slint | wubble-only: as pip-width |
| `pip-min-height` | `135px` | app.slint | wubble-only: as pip-width (pip-min-width 240 at 16:9) |
| `check-tick` | `px * 3 * 14 / 24` | components/check-box.slint | pages/touch.tsx:570 the tick: stroke 3 of a 24 viewBox drawn at 14 px (was 1.6) |
| `settings-pad-share` | `0.04` | components/settings.slint | pages/settings.css:6 .st-grid padding clamp(16px, 4cqi, 48px) (not read by the generator) |
| `settings-spy` | `140px` | components/settings.slint | pages/settings.tsx:468 the scroll spy line (t <= y + 140) |
| `settings-width` | `720px` | components/settings.slint | wubble-only: the list pages' column (pages.slint); settings itself is .st-grid (1120, settings-max) |

## Motion values switched from borrowed or wubble numbers to design tokens (they change feel)

| Shell | was | now | design rule |
|---|---|---|---|
| `sheet-in` (sheets, error panel) | 300 ms (Ultra Reactivate dialog) | `Motion.dropdown-in` 220 ms | core/tokens.ts motion.dropdown (the design's popover entrance; out was already 176) |
| `sheet-from` | .95 | `Motion.dropdown-from` .95 | same value, now named |
| `toast-in` | 400 ms on ease-spring (UR toast) | `Motion.overlap-in` 240 ms | core/tokens.ts motion.overlap.in (wb-in); the call site's spring easing is the toast lane's (L6) |
| `toast-out` | 190 ms | `Motion.overlap-out` 190 ms | same value, now named |
| `preview-in` / `preview-out` (tab hover card) | 180 / 130 ms (UR tip) | 160 / 130 ms | pages/touch.css:105-106 `.tp-tip` / `.tp-tip.show` opacity |
| `status-in` / `status-out` (link status bubble) | 120 / 200 ms (Chromium) | 160 / 130 ms | as preview: the design's tooltip fade |
| `tip-fade-in/out`, `tip-rise-in/out` | 160/130, 200/160 | the same, now named | pages/touch.css:105-106 |
| `status-move` | 160 ms (Firefox) | unchanged | no design rule for a bubble that dodges the pointer; left for L6 |

## Removed (invented motion, no design counterpart)

- The peek sidebar's steady glowing border (`GlowRing`, `peek-glow`; map C-073/W3): the design's peek has no ring.
- Film rims on the toggle and the slider knob (`FilmRim`): the design's `.tp-tg` and slider knob have no `wb-film`.
- Press (.97) on tab rows, command rows, folder rows and Space tiles (map rows 8, 28, 36, 44): the design never presses rows.
- Fades the design does not have: tab row hover background and title colour (7, 9), favourite background (29),
  folder and download row backgrounds (35, 37), the check box rim colour (59), the swipe chip tint (69).

## Scene constants without a web counterpart (lane L1, s2)

| Scene | value | used in | derived from |
|---|---|---|---|
| `AMBIENCE_STRENGTH` | 0.20 | crates/wubble-gfx/src/scene/render.rs (glass `uAmb`) | owner-requested ambience (2026-11-19, "light from the webpage leaks into the navbar"); no web counterpart; started subtle, owner judges by eye |
| `AMBIENCE_LEVEL` | 1.0 | render.rs (glass `uAmbLevel`) | which level of the page-inclusive pyramid: 1 = its 1/16 level (σ ≈ 44 CSS px, the frost's own), the falloff knob |
| `AMBIENCE_HZ` | 8 | render.rs, pages.rs `page_blur` | update rate of the ambience on a changing page; wubble's cost budget, no web counterpart |
| `calm::IDLE_AFTER` | 2 s (`kit_gen::living::idle_after`) | crates/wubble-core/src/start/calm.rs | the start page's settle (owner, 2026-11-21, "settle after a few seconds"): the web's own idle span (`core/living.ts:101`, its `auto` mode starts after it); the web never settles |
| `calm::SETTLE` | 2 s (the same token) | calm.rs | how long the bubble and liquid take to slow to a stop: no web counterpart, the idle span reused |
| `calm::EASE` | `tokens.motion.ease.inOut` | calm.rs | the settle's curve (time scale `1 - inOut(t / SETTLE)`): starts and ends gently, so the stop has no snap |

## Glass over bright pages: the contrast ceiling (qa1/glass)

The scene's glass over a page now draws the design's CSS tier (`core/wubble.css` `.wb-thin`,
`.wb-regular`, `.wb-thick`: saturate, brightness, fill), as the web draws its glass over web
content, instead of the WebGL frost meant for its dark liquid (which lifts a white page to near
white). That is a translation fix, not a derived value. What is derived:

| Piece | value | used in | derived from |
|---|---|---|---|
| Contrast ceiling | relative luminance 0.099 (sRGB grey about #585858): no glass, any tier, over anything (liquid, page, the sidebar's page ambience, a white Space palette), is brighter | `wubble_core::glass::ceiling`, glass.wgsl `ceiling()` (scene and flat fill) | the kit's own inks at WCAG 2's minimums: `ink` (#F2F4FA) 4.5:1 and `mist` (#A3A9B8) 3:1; mist is the binding one. The design's own `.wb-thick` over pure white (#6F7075, L 0.16) gives ink 4.47:1 and mist 2.09:1 |
| Its knee | half the ceiling (L 0.049) | `wubble_core::glass::knee` | below it the glass is untouched, so the design's glass over its liquid (the sidebar measures L 0.050, at the knee) keeps its look; above it the light eases toward the ceiling, scaled in linear light so the page's hue and saturation stay (darkened, not greyed) |
| Fallback over content | `Shell.thick-unblurred-fill` for every tier of a panel above the page (an overlay layer, or `over-content`) when the scene does not draw its glass | kit/material/glass.slint | owner decision 3, which applied to thick only: a thin or regular fill (white .09, unblurred) left light ink on a white page |

Measured from the gallery's plain-page scenes (`--smoke gallery`, 27-38), text ink against its
local glass, before / after. Light ink (`ink`, `mist`, the tab title): page menu row 1.11 / 6.76;
tab menu row over the page 2.33 / 6.78; command bar row 2.16 / 7.92; find bar query 1.38 / 6.39;
peeking sidebar tab title 1.14 / 4.93, its Space label 1.51 / 3.17. `haze` (a mid grey, readable
on near-white glass as dark-on-light): command bar placeholder 1.36 / 2.95, hint 1.60 / 2.28; the
page menu's disabled row 3.13 / 1.99, the tab menu's shortcut 2.45 / 2.03, the find count 2.42 /
1.89 (these three were legible before only because the glass had gone white). Over the dark page
and the liquid nothing changed (sidebar 4.42 / 9.53 both runs; dark page rows 12-14:1).

Owner decision: `haze` (#7E8496: placeholders, shortcuts, disabled rows, counts) stays at about
2:1 over a white page. Holding it to 3:1 too needs the ceiling at L 0.044 (#3B3B3B), darker than
the design's own sidebar glass over its liquid (L 0.050), so the design's look would darken
everywhere a little; one line in `wubble_core::glass::INKS`.

## Controls kit A (lane L3 press, ui2b/press)

Rows moved out of the `// derived:` block: `check-tick-delay` (now `Tsx.check-tick-delay`, generated
from touch.tsx CHECKS.tickDelay), `tip-glide` (now `Tsx.tip-glide`, DOCK.glideMs), `check-tick`
(the tick is now drawn at the design's 14 px with stroke 3, `Tsx.check-tick-size/-stroke`).
Removed with the controls they styled: `sheet-button` 42, `sheet-button-pad` 16 (Ultra Reactivate),
`check-tick-out`, `check-rim`, the toggle's `toggle-*`, `knob-highlight`, `toggle-design-travel`,
the tooltip's `tip-height/-pad/-radius/-text/-rise/-fade-*/-rise-*` (all now gen tokens).

What wubble needs that the design does not have, derived from its rules:

| Control | derived | rule it follows |
|---|---|---|
| Button (all kinds), IconButton text mode | disabled = opacity .35 | `.wb-ib:disabled` (browser.css:37); `.tp-btn` has no disabled state |
| Button pri / ghost, IconButton, Toggle, CheckBox, HoldButton | the keyboard focus ring (2 px sky, 2 px out) | core/wubble.css `.wb :focus-visible` outline, wubble's FocusRing; the design shows only the glass button's film on focus. Owner: drop the ring on pri/ghost if the film-only look is wanted |
| SheetButton | `primary` = Button pri, the others = Button ghost (44 high, radius 12; was UR 42 / 11) | the design's sheets and dialogs use `.tp-btn` (touch.tsx Buttons: pri "Open in Space", ghost "Cancel") |
| PressableButton text mode (download rows, split pane close) | the ghost Button's look (mist to ink, white .07 on hover, 14/600, radius 12, padding 16) at the caller's height (min `Chrome.controls`) | `.tp-btn-body.ghost`; the 44 px height is Button's only, so rows keep their height |
| PressableButton icon mode (find bar, download row remove) | the icon button's radius 9 and ghost hover | `.wb-ib` |
| components/controls/toggle.slint | the settings call sites get `size: sm` | settings.tsx:499 `<Toggle size="sm">` |
| HoldButton | disabling it while held lets go | the web's disabled button stops receiving the pointer up (it would keep filling); wubble ends the hold |
| HoldButton width | at least 156, else its label (or check + done label) plus 18 a side | `.st-hold` min-width / padding; the web's Swap cell holds both labels for 400 ms, wubble sizes to the current one |
| Kbd | keys as a list (`["Ctrl", "K"]`) | settings.tsx:524 splits on spaces; Slint has no string split |
| Tooltip shortcut | the shortcut after the name, haze, gap 11 / 1.5 | wubble-only (the dock tooltip has no shortcut); kept from the adopted tooltip |

## Controls kit C (lane L3 menu, ui2b/menu)

Generated (no longer typed): `TouchMenu` in `gen/tokens.slint` (touch.tsx `MENU`: first 40, stagger
30, gap 8, blur 4, confirm 1400; settings.tsx `Select`: width 220, gap 8, the view margin 8; the
inline icon sizes 16 / 14 / 15, the chevron's 14 and stroke 2, the search 16 and clear 14).
Ultra Reactivate's `Shell.menu-*` values no longer reach the menu (find bar, boost, pip, reader,
autofill, downloads still read some; their lanes move them).

| Control | derived | rule it follows |
|---|---|---|
| Menu separator | an 8 px (`Space.s2`) row with a 1 px hairline in `Glass.thick.rim-top`, inset by the row padding (10); entering with the rows; skipped by the keys | the design's menu has none (owner approved deriving it): the menu's own spacing scale and the thick tier's top rim |
| Menu disabled rows | label and icon in haze; take the keyboard focus, never lit, never chosen | the design has none; wubble's earlier rule (menu.rs), haze is the design's quiet text |
| Menu rows without an icon | the label starts at the row padding | every design row has an icon |
| Submenus ("Move to Space >") | not built: the host's flat "Move to <Space>" rows stay | the design draws only a chevron (`sub: true`) and opens nothing (touch.tsx:539). Alternative for the owner: a `parent` index on `MenuEntry` and a second Menu grown from the row |
| Page context menu placement | from the pointer: right and below, flipped at the window edge, clamped 8 px inside; grows from the pointer | the design's menus grow from their trigger; a pointer menu has the pointer as its trigger |
| Menu key focus on a pointer open | the page's context menu takes the keys at once (Down lights the first row) | the design's trigger menu leaves the focus on the trigger when opened by pointer; the context menu has no trigger to hold it |
| Sheet | scrim (`Shell.scrim`, 200 / 160) + thick GlassPanel entering as Overlap (240 / 190, 7 px, .98, blur 4) at the top of the content area, title `.st-sec-head h2` (19 / 600), message body mist, buttons right-aligned 8 apart; `Shell.sheet-*` geometry (380 wide, padding 26, radius 20) kept; a sheet taller than the window stops `Shell.sheet-margin` above its bottom and its fields scroll between the message and the buttons (BSidePanel's Flickable and `.wb-thumb` Thumb, shown on hover), so the buttons stay in reach | kit-plan 2.4 `Sheet` (derived: thick Glass + Overlap + Button row); was Ultra Reactivate's 300 ms .95 grow |

## Controls kit B (lane L3 slide, ui2b/slide)

Generated (no longer typed): `TouchCss.tp-sl-knob-focus--ring` (3), `tp-sl-bub--bottom-offset` (21),
`tp-sl-bub--origin-y` (1.3), `tp-sl-bub-b--origin-y` (1.4), `tp-sl-wrap--max-width` (300) and
`kit_gen::controls_b` (`SEG_SLOP` 3, `SEAM_KEY_STEP` 16, `SEAM_KEY_KICK` 220: literals in the TSX loops),
read from the rules by `extract.mjs` (it throws when a rule changes).

| Control | derived | rule it follows |
|---|---|---|
| ElasticSlider, Segmented disabled | opacity .35 | `.wb-ib:disabled` (browser.css:37); the design's slider and segmented control have no disabled state |
| components/controls/slider.slint (settings rows, boost strength) | `compact` (`.st-slider`), step a hundredth of the range, every value reported while held (`set`), max 1 by default | the old wrapper's API and resolution; the design reports live (`onChange` while held) |
| print sheet copies (`Stepper` there) | not replaced: a typed numeric entry is a new primitive (kit-plan 2.3, wubble-only "numeric entry"); the zoom and pages settings use `Stepper` today where the design has `ElasticSlider` (`settings.tsx:535-537`): the settings lane's switch | |
| Segmented `narrow` | an input: the caller says when its container is under 340 px | `@container (max-width: 340px)` (touch.css:118); Slint has no container query |
| Seam string ends | the rest string runs from the page frame's top radius to its bottom radius (Radius.md in) | `seam.clip(a, lw, lh, 14, ...)` (chrome.tsx:189); split seams from the view's radius |
| Split / DevTools pills | rest at mid height until the host passes `gy` (SplitSeam's new `bend`, `gy`, `held` default to 0 / mid / false) | webview.tsx:125, chrome.tsx:143 |

## Chrome (lane L4, ui3/chrome), first part

| Item | derived | rule it follows |
|---|---|---|
| Tab list Up / Down over folder headers | a folder header row is skipped (not a stop); clamped at both ends, no wrap (`wubble_core::tabs::step_focus`) | `tabs.tsx:125` `clamp(i ± 1, 0, n - 1)`; the design has no folders |
| Tab list roving focus | the selected row takes the focus when the selection moves while a tab row holds it (`follow-focus`) | `tabs.tsx:125` focuses `children[j + 1]` |
| Dragged folder rows | no `.wb-tab-drag` look (only tab rows carry it) | `.wb-tab-drag` is a tab-row class |
| Split pane close | no button on split panes; the docked DevTools pane keeps its close button (wubble only) | `webview.tsx` has no pane close; close stays on the tab menu, the command bar and Ctrl+W |
| Held split line | sky 1.6 at opacity 1 while the string bends; no fade-out (the path is gone on release) | `.wb-split-held .wb-split-line` (browser.css:114-115) |

## Overlays A (lane L6a, ui4/ova)

Kit files in `crates/wubble-ui/ui/kit/overlay/a/`; the nine shell files compose them (kit lock: 76
baseline rows cleared). The design (598f814) has no toast, status bubble, find bar, downloads list,
progress bar, prompt or error page: every piece below is derived. Screenshots (gallery, 1x and 2x):
(screenshots kept outside the repo) (`NN-name-1x.png`, `-2x.png`).

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Popover (`a-popover.slint`) | the Menu body without rows, scrim or keys: thick glass, radius 16, from .95 / blur 4 at its anchor (the downloads popover passes the anchor's point inside the clamped panel), 220 ease-out in, 176 ease-in-out out, arrowless; `present` from the tween | touch.tsx Menu, touch.css `.tp-menu` | 11, 14, 16, 18 |
| Prompt (`a-prompt.slint`) | Popover + `heading` title + body (mist) + request lines led by `.st-lead` tiles (one shield glyph for every line: the host sends plain strings) + kit CheckBox + Button row (ghost..., one pri), padding 16, gap 8; "Never" ghost in coral; no keyboard focus of its own | Sheet button rule, `.st-lead`, `.st-row-title` | 14, 15, 18 |
| Menu list (`a-menu-list.slint`) | MenuRow + MenuHighlight with a local hover index, pointer only (no Menu machine, scrim or focus: it shows over a page form); the key glyph on every row | `.tp-mrow`, `.tp-menu-hl` | 16, 17 |
| Capsule bar (`a-capsule-bar.slint`) | the search capsule's thin glass, 40 / r20, film ring, icon and input; count in mono at `.st-count`'s 12.5, coral only when the host says a finished search found nothing (`FindBar::none`, not while a keystroke waits); kit IconButtons; grows from its top right as the Popover. Thin glass over a bright page washes the count and icon out: `tier` is one line to switch to thick | `.st-search`, `.wb-mono` | 13 |
| Download row (`a-download-row.slint`) | menu row anatomy, 60 high (`Shell.download-row`), name `.tp-mrow`, detail `.st-row-note` in sub haze, `.tp-menu-hl` under it on hover, kit IconButtons (pause, play, close, reload, arrow, folder, trash) | `.tp-mrow`, `.wb-ib` | 11, 12 |
| Progress track (`a-progress.slint`) | the slider rail (white .12, 1 px inset top light, round ends) and accent fill, 4 px (`Space.s1`, not the rail's 8), unknown size: full at .35 | `.tp-sl-rail`, `.tp-sl-fill` | (running downloads: `--smoke pages`) |
| Toast (`a-toast.slint`) | thick glass capsule (radius = height / 2), 46 least height, 16 side padding, `.st-row-title` ink, Overlap in from below / out up; the action a 30-high ghost base button (a 44-high Button would make the capsule 68) | `wb-in` / `wb-out` | 20 |
| Status bubble (`a-status-bubble.slint`) | glass capsule 24 high (thick with the unblurred over-content body, owner decision 3: the thin fill, white .09 without blur, vanished on light pages and left the host's ink on white; ui7/list), the capsule is the bubble's box and the text row the capsule's (host and rest both elide inside it), `.wb-address-text` (UI face 13, host ink, rest haze; wubble-only.md said mono, the CSS says UI face), `.tp-tip` in / out; no scene key (Slint frost only: a `GlassKeys` entry is model.slint's) | `.wb-address-text`, `.tp-tip` | (live page: `--smoke nav` hover) |
| Preview card (`a-preview-card.slint`) | thick glass r12, pad 10, title 13 / 500, address mono 11.5 haze, frame r8; `.tp-tip` motion | `.tp-tip`, `.st-row-title` | (live tabs) |
| Glance card (`a-glance-card.slint`) | scrim + thick card (radius lg, page md, 8 px frame), title `.st-row-title`, Buttons glass "Open in split" / pri "Open as tab", IconButton close; Popover entrance | Sheet, `.tp-btn` | (`--smoke overlay`) |
| Error page (`a-error-panel.slint`) | thick glass over the content, column 440 a third down, globe mist / lock coral 40, `title` role, body mist, detail mono haze in a kit Fold under a ghost "Details" / "Hide details" on its own line above the row (open for a certificate error), Button pri + ghost (the row never holds more than two, so it fits the column), column by Overlap. Mist and haze on the scene's bright glass read poorly over a pink page | settings roles, `.st-fold` | 21, 22 |

Refix (ui4/ova): shots in `ui4-ova-shots-refix/` (22: Details above the row, row inside the column).
Open: ghost Button and CheckBox hover fills render opaque (kit/controls/button.slint `mix()` and
check-box.slint scale `.alpha`, an int 0..255, by t without /255): forbidden files for this lane,
raised with the lead; hover shots 12, 15, 17 stay wrong until the controls lane fixes it.

## Chrome, second half (lane L4, ui5/chrome)

Kit files in `crates/wubble-ui/ui/kit/chrome/`. Shots: (screenshots kept outside the repo) (gallery on the
parity manifest, 1288x816, frozen backdrop), compared by eye with (screenshots kept outside the repo).

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Address pill chips (`address-pill.slint` `Chip`) | wubble's pop-ups (amber expand + count), media (mint download), blocker (mint shield + count, haze when the site is allowed), reader (book: haze, ink on hover, sky while on), bookmark star (haze, amber filled when bookmarked) at the pill's right end: the address icon's 13 px / 1.9 stroke, the pill's 8 px gap, counts in the address face (mono 13) mist, half a gap between a glyph and its count | `.wb-address` gap and `.wb-address-icon` | address/01, 10 |
| Address pill lock vs bubble | the design shows the lock for every site and the bubble for its own pages; wubble shows the lock for https sites only (an http or file page has no lock to show) | chrome.tsx `secure` | address/01 |
| Address of a `data:` page | the design has no data: pages; wubble's bundled pages read `data` alone, never their body (`wubble_core::chrome::address_parts`) | chrome.tsx `addressOf` (own pages: name only) | base/01 vs address/01 |
| Favourites active tile (`favs.slint`) | wubble's favourites are its pinned tabs; the active one's tile takes the selected tab's fill (`Shell.selected-fill`, `.wb-tab-ind` white .12) and its icon ink, the others mist | `.wb-tab-ind`, `.wb-tab-on` | favs/01 |
| Favourite wave | the host waves the scene glass only on a left press over a favourite (`AppWindow.fav-hovered`), as `Fav` onPointerDown; presses elsewhere on the shell no longer wave (the design has no other wave in the window) | chrome.tsx:77 | (live) |

## Settings page (lane L5a, ui4/settings)

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Field (`kit/controls/field.slint`) | a plain one-line field: the search capsule's thin glass (`wb-thin`) at the control height 36 and radius 12, the capsule's padding (14), 8 gap, 16 px mist icon when given, 14 px input, haze placeholder; the film rim while focused; no clear button (that is the search's); numeric right-aligned, secret masked; `capsule: true` is the SearchCapsule itself | wubble-only.md section 6 ("the `.st-search` capsule at the control height, radius 12, film rim while focused") | (screenshots kept outside the repo) (Add, Default), `22-settings-3.png`, `24-settings-x1.png` (number fields) |
| Number field (Stepper above a range of 500) | the same Field, numeric, 88 wide, Enter applies, clamped | the design has no stepper; a slider over thousands of steps is unusable | (screenshots kept outside the repo) |
| Reset per row | the section head's `.st-reset` button in the row's controls, entering as the design's (popover in) when the row changes | the host resets per key; the section-level loop is a later step | (live: change any setting) |
| Rail footer with downloads | wubble's downloads button joins the rail's column under the Spaces (10 apart), so the column starts one button higher than the web's | `.wb-side-bottom-rail` gap 10 | spaces/02 |
| Footer buttons (`side-bottom.slint`) | the design's `[Spaces] ... [Source][Settings]`; wubble's Source is the page's developer tools (same `code` glyph, place and `.wb-ib` 36/12 look), and wubble's downloads button comes first while there are downloads, in the same look; developer tools give way when the row has no room for three (F12 still opens them) | `.wb-side-bottom > .wb-ib`, `margin-left: auto` | footer/01, 02 |

## List pages (lane L5 list, ui6/pages)

Kit files in `crates/wubble-ui/ui/kit/pages/`; `components/pages.slint` and `components/download-row.slint`
compose them only (kit lock: 63 baseline rows cleared). The design has no list page, row actions,
tree, table, progress bar or plain field: all derived from the settings page (wubble-only.md §32).
Screenshots: (screenshots kept outside the repo) (1x) and `ui6-pages/2x/` (2x): 24 history, 25 bookmarks,
26 downloads, 27 passwords-locked, 28 passwords, 29 archive, 30 task-manager, 31 history-hover.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| PageFrame (`page-frame.slint`) | the settings pinned column stacked into one 720 column: `.st-pin h1` (40, 4 in, 20 under), kit SearchCapsule full width, `.st-count` line; the task manager takes `.st-grid`'s 1120; PageScroller is a copy of settings.slint's private one until L5a imports this | `.st-pin`, `.st-search`, `.st-count` | 24-30 |
| ListCard / ListGroup / ListEmpty / ListToolbar / ListNote / ListFoot (`list-card.slint`) | `.st-card` (white .035, r20, pad 6, top light); history's flat lines draw it in slices (only the first and last of a day round); `.st-group` day headers; `.st-empty` 15 mist; the action line spaced as `.st-count`, buttons ghost at its end; a card paragraph in `.st-row-note` under the row text | settings.css:39-40,117-118 | 24, 27 |
| ListRow (`list-row.slint`) | `.st-row` (min 60, 11 14, gap 16, r14, `.tap` hover white .04 / .15 s) led by an `.st-lead` tile (favicon 16 or mist icon), `.st-row-title` over `.st-row-note` (addresses in `.st-path .wb-mono`); entrance on the Overlap stagger (capped at row 12); click opens, middle-click opens in a tab | settings.css:41-47 | 24, 31 |
| RowAction / RowTextAction | kit IconButton (or the 30-high ghost base button for Rename / Move) revealed as `.wb-tab-x` (0 at .8 → 1, .15 / .2 s ease-out) on the row's hover or its own keyboard focus; delete in coral | browser.css:75-78 | 31 |
| RowChevron | the design's disclosure `.tp-chev` (down, up while open, .22 s) at a folder row's end, in place of a tree's right/down chevron | touch.css:139-140 | 25 |
| ListProgress (`list-progress.slint`) | the slider rail (white .12) and accent fill at `Shell.download-track` 3 px under the row text, 220 ms ease-out; unknown size full at .35; no inset light at 3 px | touch.css:50-52 | (running downloads) |
| Table (`list-table.slint`) | column heads in `.st-group`, numbers in `.st-path .wb-mono` right-aligned, controls centred in `Shell.task-*` columns; End column as wide as the HoldButton (156) | settings.css:39,101 | 30 |
| ListField (`list-field.slint`) | the search capsule's body without icon and clear button, `secret` masked (the plain field the design lacks: wubble-only 6, 13) | settings.css:12-14 | (add-password sheet) |
| Passwords row | Show/Hide IconButton (eye, always shown), Copy ConfirmButton ("Copied"), Delete HoldButton ("Deleted"): the row's controls stay visible (a hold and a confirm cannot hide mid-run) | wubble-only §32, G3, G4 | 28 |

## Overlays B (lane L6b, ui6/ovb)

Kit files in `crates/wubble-ui/ui/kit/overlay/b/`; the eleven shell files compose them (kit lock:
77 baseline rows cleared, none of the eleven files has a row left). The design (598f814) has no
sheet body, video strip, picture-in-picture, reader, Boost editor, marquee, swipe chip or
autoscroll pointer: every piece below is derived. Screenshots: gallery (scene glass, Skia):
(screenshots kept outside the repo) (`16-dialog-sheet`, `17-dialog-prompt`, `18-bookmark-sheet`,
`19-print-sheet`; `gallery-2x/` the same at 2x); live-page surfaces drawn by the shell offscreen
from properties (Slint fallback glass, no scene; `crates/wubble-ui/examples/overlay_b_shots.rs`,
`cargo run -p wubble-ui --example overlay_b_shots -- DIR`): (screenshots kept outside the repo); the real strip over a video:
(screenshots kept outside the repo) (`--smoke video --shot`).

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Sheet field (`b-field.slint` BField) | the search capsule's body without icon or clear: thin glass, 40 high, film ring while focused, 14 input ink / haze placeholder, 14 in at both ends; radius 12 (the button's, a sheet field is not a capsule); Escape consumed (`escaped`) | `.st-search`, `.tp-btn-body` radius | gallery 17, 18, 19 |
| Sheet group (`b-field.slint` BGroup) | `.st-group` label (13 / 500 mist) 8 in, 8 above its control; built only with a label | `.st-group` | gallery 18 ("Folder") |
| Dialog sheet body | the fields are built only when the dialog has them (`if`): the 32 px of spacing around four hidden fields is gone; the prompt's and the password's labels are groups | wubble-only §13 | gallery 16, 17 |
| Settings card and rows (`b-sheet-parts.slint` BCard, BRow, BStackRow, BNote) | `.st-card` (white .035, top light, r20, pad 6) of `.st-row`s (60 least, pad 11 14, gap 16, title 14 / 500); BStackRow: the control under the title (the row's wrap, the sheet is 380 wide); `wide`: the control takes the room left (a field); BNote `.st-row-note`, coral when `danger` | settings.css:40-47 | gallery 19, live 06 |
| Flat chooser (`b-sheet-parts.slint` BChoiceList, BChoiceRow) | the Select's open menu laid flat: `.tp-mrow` rows with the `.st-mcheck` column, one `.tp-menu-hl` gliding under the pointer (`jump` from none), on a card or bare; tree indent `Shell.folder-indent` per depth. Stands in for the Select: the Select takes `[MenuEntry]`, these lists are host items (`[FolderChoiceItem]`, `[string]`) Slint cannot map | `.tp-menu`, `.st-mcheck` | gallery 18, 19, live 02 |
| Remove bookmark (`BDangerButton`) | ghost Button in coral (rest and hover) | `.tp-dock-btn.danger`, `.tp-mrow.danger` | (a bookmarked page) |
| Print sheet | destinations on the chooser, then one card: Pages (wide field), Layout / Paper / Margins (`.st-seg` compact), Scale (ElasticSlider compact 10..200 step 10, across the row), Copies (ElasticSlider compact 1..99, printers only: recorded, a number field would be the alternative), Background graphics (CheckBox); the error a coral note; built at the first opening (its controls take registry slots, which the host wires after the window) | wubble-only §15 | gallery 19 |
| Dock (`b-dock.slint` BDock) | `.tp-dock` body (pad 6, gap 2, r18) in **thin** glass (the spec's capsule; the touch page's dock is regular) with 30 px `.wb-ib` tiles (not the 44 px dock tiles), Overlap in / out as a transition, `present` from its progress, its controls inside its touch area; no gliding tooltip (the shell has one tooltip), no neighbour swell | touch.css:92-98 | live 01, 05, 07; video-strip.png |
| Value button and value (`BDockText`, `BDockValue`) | `.wb-ib` body around a number in mono 13 / 500, swapped across (`.tp-sw-x`) | G5, `.tp-sl-label output` | live 01, 05 |
| Collapsed strip tab (`BDockTab`) | a thin-glass tile as high as the strip, r9, mist chevron, sliding in from the pane edge on the Overlap curve (wubble-only suggests the seam pill: kept a tile, the pill is 6 px wide to hit) | Overlap along x | live 03 |
| Scrubber (`b-scrubber.slint`) | the compact ElasticSlider across the strip's free room (240 preferred, less when narrow), its 44 px track centred and clipped to the strip (the value label and the held bubble are cut: no room); 0..1000 for the progress; SponsorBlock marks under the rail in their colours (a new variant) | `.tp-sl`, wubble-only §27 | live 01, video-strip.png |
| Strip save button (`b-dock.slint` BDockSave) | the media downloader's entry on the video strip, before the collapse chevron: the strip's `.wb-ib` IconButton with the download glyph (the address pill's chip and the sidebar foot use it); saved: the check (the touch Download specimen swaps "Download" to "Saved" and its icon to the check, `touch.tsx:300-358`; here without the Swap crossfade, IconButton draws one icon); saving: the downloads list's progress track (`a-progress.slint`, 4 px, unknown size full at .35) across the tile's foot, `Space.s2` in from its sides, `Space.s1 / 2` up from its foot, under the glyph; failed: the download glyph again. Always there while the downloader is on (2026-11-21, owner: "it never shows on the overlay"): pressing it saves or says why not (protected, live, YouTube's lookup off, nothing found), and on a narrow video the scrubber (which shrinks) gives way to it, as it does to every tile. Over an embedded frame the page kit cannot reach (a cross-origin player) the strip is the save control and the close and collapse tiles only, and shows only when the downloader lists something for the page. The design's Download button (156 x 52, accent, labelled) is a page control with no progress look, not a strip tile | `.wb-ib`, touch.tsx:300-358, `.tp-sl-rail` / `.tp-sl-fill` | `--smoke video --shot` (saved, hovered); overlay_b_shots 13-17 |
| Speed menu (overlays A's `APopover`) | the popover (the Menu's body: thick, r16, from .95 at the trigger's end, 220 / 176), cut to its corners, its own hover area, holding the flat chooser's rows (each keeps the `speed` id the video smoke clicks) | `.tp-menu` | live 02 |
| PiP card (`b-pip-card.slint`) | the error page's column: pip glyph mist 40, `title` 24 / 600 ink, tab title body mist, pri Button; rows Overlap 30 ms apart; one wobble on arrival | wubble-only §20, §21 | live 04 |
| Reader controls | the Dock: minus / value / plus, three `.st-seg` compact (themes stay words: their colours are the reader's own, not the design's, so no Swatches) | wubble-only §25 | live 05 |
| Side panel (`b-panel.slint` BSidePanel) | the Source panel: r14, head 46 (title in the heading role, `.wb-ib` close), a scrolling column with the `.wb-thumb`; thick glass for its fill (the scene blurs under it); Overlap shifted across from the right | browser.css:183-203 | live 06 |
| Code area (`b-panel.slint` BCode) | `.wb-code` mono 12 on a 19 line, #C9CEDA, six lines, in the field's thin glass with the film ring; no line numbers (Slint cannot count a text's lines) and no token colours | `.wb-code` | live 06 |
| Boost editor | card (On toggle sm, Script runs `.st-seg`), Name / Sites / Font fields, the pattern hint as a note (coral with the error), CSS and script code areas, Zap a glass Button, a Tint toggle row (off is the toggle's: the kit Swatches have no "none"), the Swatches and the strength ElasticSlider (1 to 100) shown only while the tint is on, Delete a HoldButton, Done pri | wubble-only §29 | live 06 |
| Marquee (`b-marquee.slint`) | the scrim (.32, `wb-fade`) cut round the region, the held line (sky 1.6) outside it, the size in the slider's value bubble (ink pill, mono 12.5 / 600 abyss, springing in from .6 and 8 px) under its bottom right | browser.css:114,163; touch.css:55-59 | live 07, 08 |
| Screenshot toolbar | the Dock: the hint in the menu row type (mist), IconButtons eye (Visible page), expand (Full page), close (Cancel) | wubble-only §24 | live 07 |
| Swipe chip (`b-swipe-chip.slint`) | the start page's site tile (52 round, thin glass); ready = the film rim lit, the press squash (1.1, .88 on the tile's spring) and one arrow wobble, instead of a mint arrow; no `wb-out` exit (app.slint builds it only while the swipe runs) | start.css:23-28, wubble.css:41-43,62 | live 09, 10 |
| Autoscroll pointer (`b-autoscroll.slint`) | the film rim (lit) as the anchor ring round an ink centre; the slider knob (ink, its shadow scaled) as the still dot; the chevron Icon in mist as the arrow, drawn to fill the stretched box | touch.css:53, wubble.css:35-44 | live 11, 12 |
| Keyed row buttons (`kit/pages/row-keyed.slint`) | the kit HoldButton and ConfirmButton unchanged, made anew when the row they sit in shows another item (passwords Copy / Delete, task manager End); nothing drawn | wubble-only §32 | ui6-pages/28-passwords.png, 30-task-manager.png |
| Command row hint (`command-row.slint`) | wubble's results carry a subtitle and a shortcut; the design's row has one hint: the shortcut when there is one (the design's actions put theirs there, `'Ctrl U'`), else the subtitle (the design's sites put their address there), 12 haze, at most 45 % | `.wb-cmd-hint` | command/07, 08 |
| Command bar head (`command-head.slint`) | the design's head without its microphone and "Show code" buttons (wubble has no voice and no Source panel); a new small kit part, not the settings page's SearchCapsule (the web head has no capsule, ring or clear button) | `.wb-cmd-head`, `.wb-cmd-in` | command/07 |
| Command bar empty line | wubble's wording "Nothing matches. Press Enter to search." (the engine is a setting, not "the web") in the design's `.wb-cmd-empty` look | `.wb-cmd-empty` | (no results) |
| Command bar rows | at most 7 rows (`CMD.rows`), as the design slices; wubble's old list scrolled past them | command.tsx `slice(0, CMD.rows)` | command/07 |

Overlays B, seen while shooting, not the lane's to fix (kit Button, `kit/controls/button.slint`
`PressableButton.mix`): Slint's `color.alpha` is an int 0..255, so `b.with-alpha(b.alpha * t)`
clamps to opaque: a hovered IconButton or ghost Button from a clear fill turns solid white
((screenshots kept outside the repo), the frame button; `video-strip.png`). Fix:
`b.alpha / 255 * t` and `a.alpha / 255 * (1 - t)`.

## List editor (ui7/list)

`kit/controls/list-editor.slint`: macOS's add / remove list ((screenshots kept outside the repo))
in wubble's look, composed of kit parts only; its interaction is `wubble_core::controls::list_editor`.
Shots: (screenshots kept outside the repo) (settings-lists, settings-spaces, settings-sites).

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Well | the settings card's material (white .035, inset 1 px top highlight) at the row radius 14, so it nests concentric in a card row (card 20, padding 6) | `.st-card`, `.st-row` radius | settings-lists |
| Header | the column titles in the settings group label (13 / 500 mist), 32 high (the section head's height), over a 1 px hairline in the menu rule's colour (thick rim-top) | `.st-group`, `.st-sec-head` height, MenuRule | settings-lists |
| Rows | the menu's rows: 36 high, padding 0 10, gap 10, first column 13.5 ink, the others in the row note (13 mist); a leading 16 px icon (`.tp-mrow svg`) in the row's tint; equal columns, elided | `.tp-mrow`, `.st-row-note` | settings-spaces |
| Selection | the menu's ONE highlight (white .09, radius 10, inset 6) gliding between rows (.16 s); hover is the tap row's white .04 at the same inset and radius; no alternate tint (the design has no zebra) | `.tp-menu-hl`, `.st-row.tap:hover` | settings-lists |
| Swatch column | the row's colour as the swatch dot (20, radius 10) before its name | `.st-sw i` | settings-spaces |
| Footer | a hairline, then "+" and "−" as `.wb-ib` icon buttons (30, radius 9) 4 apart with a 1 px hairline between them (8 inset top and bottom); "−" at .35 with nothing selected; a second "−" asked in coral beside them (Spaces) | `.wb-ib`, MenuRule, `.st-row-note` coral | settings-spaces |
| Draft row | one kit Field per edited column (thin glass, radius 12, 36 high) in the row's place; Enter submits, Escape drops; a refused draft keeps its fields and folds its note in coral under the rows (`.st-fold`) | Field, `.st-fold`, NoteTone.conflict | (live) |
| Empty | the row note (13 mist) in a 36 row | `.st-row-note` | settings-sites (none) |

## Follow-ups (ui8/follow)

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Start page address field (`kit/pages/start.slint`, `controls/search-capsule.slint`) | the settings page's SearchCapsule (thin glass, film ring while focused or holding text, the clear button with text) at `.wb-start-search`'s geometry and type (54, r27, padding 0 20, gap 12, 18 px icon, 15 px; placeholder mist, typed text ink), its `<kbd>` shortcut (mono 12 haze) in the clear button's place while empty; Up / Down added to the capsule as `step` (off unless asked) | `.st-search`, `.wb-start-search` | ui8-shots/start/01-rest.png |
| Start page results (`kit/pages/start.slint`) | the command bar's own panel and list (kit CommandPanel + CommandList, split out of command-bar.slint so both share them): thick glass r20, padding 6 8 8, the bar's rows and gliding selection, as wide as the capsule, `TouchMenu.gap` (8) under it as the Menu hangs under its trigger, growing from the capsule's centre on `wb-pop-in` / `wb-pop-out`; shown while the bar is open in the page with a query. Drawn by Slint (`over-content` fill), not the scene's blurred glass: the page's own glass (capsule, tiles) is Slint's too | `.wb-cmd`, `.wb-cmd-list`, `.tp-menu` gap | ui8-shots/start/04-results.png |
| Rail close badge (`kit/chrome/rail-badge.slint`, once `rail-close.slint`, used by `tab-row.slint` on the rail) | the window's close light (`.wb-light`: 12 round, white .18, coral `nth-child(1)` on its own hover with its dark glyph, 8 px close at 2.4) centred on the tab icon's top-left corner; the glyph in `.wb-tab-x:hover`'s ink at rest (mist was unreadable at 8 px on white .18); it fades in (.15 s) and grows from .8 (.2 s ease-out) while the row or the badge is hovered, as `.wb-tab-x` does on a full row; "Close tab" to assistive tech; a click closes, the rest of the icon still picks | `.wb-light`, `.wb-tab-x` | ui8-shots/gallery/NN-sidebar-rail-hover.png |
| Rail sound badge (`rail-badge.slint`, in `tab-row.slint` on the rail) | the same circle, speaker or muted speaker (the extended row's glyphs, `Glyph.speaker`, `Glyph.speaker-muted`) at 8 px, centred on the icon's top-right corner, shown (not hover-revealed) while the tab plays sound or is muted, so a muted quiet tab can be unmuted; the window's amber light (`nth-child(2)`) on its own hover, the circle growing and the row's other badges as the close badge; a press mutes or unmutes ("Mute tab" / "Unmute tab" to assistive tech) | `.wb-light`, `.wb-tab-x` | gallery rail-sound, rail-sound-hover, rail-sound-badge-hover |
| Space switcher "+" (removed by ui8/nav: the "^" and the Spaces popup replace it; New Space is the popup's sticky row) | ~~one more `.wb-space` tile after the Spaces~~ | | |
| Menu rows without an icon (`controls/menu.slint` MenuRow `icon-column`) | in a menu where another row has an icon, an icon-less row keeps the icon's 16 px column empty (the web's `.tp-mrow` rows all carry one, so their labels line up); a menu with no icons starts its labels at the padding as before | `.tp-mrow svg` | ui8-shots/gallery/NN-tab-row-menu.png |

## QA layout fixes (qa1/layout)

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Menu width (`controls/menu.slint` Menu `fit-width`, PointMenu) | a menu at a point (tab row, site, page, sidebar) is the design's 236 unless a row needs more; then it grows to its widest row whole, up to `Shell.menu-max` (the prompt surface's 320, `.ur-o-surface`), past which a row is cut on one line with an ellipsis; a host inside a label is cut in its middle past 20 characters (`menu::menu_host`); the Select and the touch menu button keep their fixed widths | `.tp-menu` width, `.tp-mrow` one line, `.ur-o-surface` | gallery NN-long-menu-rows, NN-site-popover-long-host; tests/layout_shot.rs menu-* |

## Settings links and the sidebar footer (ui8/nav)

Owner, 2026-11-19: "Remove dev tools from the navbar (it stays on the right click) we would add
the extensions button over there. Replace the + near the Home icon with a ^ and that opens up
popup with all the saved spaces and 'sessions (mcp plan)' and a sticky new session inside that
popup"; the footer shows only the current Space's icon and the "^"; the pages (history,
archive, ...) get "Links in the settings nav ... and when searched, shows a redirect link". The
design has no Spaces menu and no page links in its settings nav.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Current Space tile (`kit/chrome/space-switcher.slint` SpaceCurrent) | the switcher keeps one tile, the window's Space: its glyph in ink on the capsule (`.wb-spaces-ind`, the Space's colour at .2, the colour moving over .4 s), which now rests under it and never travels (the capsule's follow spring and stretch are gone with the other tiles); a click wobbles it and opens the Spaces popup (it switches nothing); "Current Space" to assistive tech | `.wb-space`, `.wb-spaces-ind` | notes/ui8/after/01-sidebar-extended.png, 02-sidebar-rail.png |
| "^" (`kit/chrome/spaces-button.slint` SpacesButton) | one more `.wb-space` tile 2 after it: the touch kit's menu chevron (`chevron`) at the Space glyph's 17 px in the tile's ink, turned 180 (pointing up) at rest and turning back down over .22 s ease-out while the popup is open (the trigger's `.tp-chev.up`, mirrored because this menu opens upward); a click's wobble; a button "Spaces", expanded while open; no tooltip (no action names it) | `.wb-space`, `.tp-chev`, `.tp-chev.up` | 01-sidebar-extended.png, 12-spaces-popup.png |
| Footer buttons (`kit/chrome/side-bottom.slint`) | the design's Source button (wubble's developer tools) is gone; downloads (while there are any) then Settings at the far end, the row needing no give-way rule any more; the extensions button will take the free place before downloads | `.wb-side-bottom`, `.wb-ib` | 01, 02 |
| Spaces popup (`kit/chrome/spaces-menu.slint` SpacesMenu) | the touch kit's menu (thick glass r16, padding 6, 236 wide, the body growing from .95 blurred over 220 / 176 ms, rows 36 entering 40 + 30·i ms behind) opening above its trigger as the settings Select does near the page's end (`.st-menu.up`: 8 above, from the switcher's left edge, growing from the "^"'s centre; on the rail above the current tile, since the "^" is under it); the Spaces in order as the Select's rows (`.st-mcheck` check column, the current Space checked in its own colour), each with its Space glyph and name; a rule between groups (none yet: a second group, the MCP plan's sessions, is data the shell already draws); at most 7 rows show (the command bar's `CMD.rows`), the rest scroll under the design's thumb (`.wb-thumb`, on hover); then, outside the scroller, the menu's rule and the sticky "New Space" row (plus glyph, its shortcut in the kbd column); one highlight in each part (the menu's `.tp-menu-hl`), the kit menu's keys and machine (Up/Down wrap over both parts, Enter/Space, Escape, Tab), the focused row scrolled into view; a press outside closes it; one overlay among the others (`wubble_core::overlay::Kind::Spaces`) | `.tp-menu`, `.tp-mrow`, `.tp-menu-hl`, `.st-mcheck`, `.st-menu.up`, `CMD.rows`, `.wb-thumb`, MenuRule | 12-spaces-popup.png, 13-spaces-popup-rail.png, 66-spaces-popup-many.png (pointer on a row) |
| Settings nav pages group (`kit/settings/nav.slint` NavGroup, NavRow `leads`) | under the sections a second group: its heading the settings group label (`.st-group`: 13 / 500 mist, 22 above, 8 below, 8 in), "Pages"; one row per built-in page (History, Bookmarks, Downloads, Passwords, Archive, Task manager; `wubble_core::pages::PAGE_LINKS`) in the section rows' look with the page's own icon (kit `Glyph.page`) and the menu's "leads on" chevron at its end (`.tp-mrow-sub`: 14 px haze); never current, never disabled by a search; a click runs the page's own action (as its shortcut and the command bar) | `.st-group`, `.st-nav-btn`, `.tp-mrow-sub` | 42-settings.png |
| Settings search redirect rows (`kit/settings/section.slint` PageLinkRow) | a search that names a page (its title or keywords) shows, above the settings it leaves, a "Pages" section head and a card of tap rows (`.st-row.tap`): the page's icon on the `.st-lead` tile in the Space accent, its title and one-line note (`.st-row-text`), the "leads on" chevron at the end; a click opens the page; the count line reads "N settings, M pages" ("M pages" alone, "No matches" only when neither) | `.st-sec-head`, `.st-card`, `.st-row.tap`, `.st-lead`, `.tp-mrow-sub`, `.st-count` | 50-settings-search-pages.png |
| A tab dragged onto the "^" (`wubble_core::tabdrag`, SpacesMenu `drop-line`) | owner, 2026-11-20: "when drag on top of the ^ the popup opens and place the tab into the desired space": a dragged tab resting on the switcher (the "^" or the current Space's tile) for the drag's hover delay (`tabdrag::SPACE_HOVER`, the one the Space icons used) opens the popup through the overlays; the row under the pointer takes the menu's one highlight (`.tp-menu-hl`, as a hovered row); a release there moves the tab to that Space, on New Space into a new Space; the current Space's row and the panel's rule take nothing (the drag cancels as elsewhere); off the popup for the same delay, or when the drag ends, it closes | `.tp-menu-hl`, the drag's dwell | notes/ui8/final gallery NN-spaces-popup-drag.png |
| Archive's page icon (kit `Glyph.page`) | the design's stacked trays (`pages/settings.tsx:59`, its Spaces section glyph), so History keeps the clock alone (owner: "Find separate icons"); the generated set has no box or archive glyph; candidates sheet notes/ui8/icons.png | settings.tsx `spaces` | 42-settings.png |

## Extensions popover and Settings section (xm/ext)

Owner, 2026-11-19: "Remove dev tools from the navbar ... we would add the extensions button over there". The design has no extensions UI (the clone's `src/wubble` has none), so every piece is derived from the kit; none needed a new primitive.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Extensions button (`kit/chrome/side-bottom.slint`) | the footer's `.wb-ib` before the downloads button, the puzzle glyph (`Derived.puzzle`, a jigsaw piece drawn on the 24 grid at the icons' 1.7 stroke), on (`.wb-ib-on`) while the popover or a popup is open; shown once an extension is installed; no tooltip (no `Action` names it yet) | `.wb-ib`, `.wb-ib-on` | extensions-popover.png, extensions-popover-rail.png |
| Extensions popover (`components/extensions-popover.slint`, `kit/overlay/a/a-extension-row.slint`) | the downloads popover (Popover growing from its anchor, rows on the Overlap stagger) with the menu row's anatomy at 36: the action's icon at 16 (the extension's picture, else the puzzle in haze), its title in the row role, its badge as a pill at the end in the colours the extension set (Chromium's grey and white when none); an action off on the page is dimmed (.5) and takes no press; a last row "Manage extensions" (settings glyph) opens Settings; a press outside or Escape closes it; one overlay among the others (`wubble_core::overlay::Kind::Extensions`) | `.tp-menu`, `.tp-mrow`, `.tp-menu-hl`, the downloads popover | extensions-popover.png |
| Hover list (`kit/controls/hover-list.slint`) | the downloads and extensions popovers' list, moved into the kit unchanged: a Flickable whose `.wb-thumb` shows while the pointer is over it and the rows overflow (its hover the list's own, no surface's) | Flickable, Thumb | downloads-popover.png, extensions-popover.png |
| Action popup (`kit/overlay/a/a-popup-card.slint`) | the Popover round the page, the page in the glance card's frame (8 round it, radius md) sized as the page asks between 25 x 25 and 800 x 600; replaces the list under the same anchor; no header | glance card, Popover | not shot (needs a live extension) |
| Install prompt (`components/extension-sheet.slint`) | the kit's Sheet (as the JS dialog): "Add “name”?", the version, then "It can:" and one bullet per permission line, Cancel and "Add extension"; modal (`Kind::ExtensionSheet`) | Sheet | extension-prompt.png |
| Settings section "Extensions" (`components/extensions-section.slint`) | a Card of Rows: per extension its name over "Version, first permission line, and N more", an Options ghost button when it has an options page, a remove IconButton (trash) and the sm Toggle (the row taps it too); then the "Add an extension" row with "Install from file" and "Install from folder"; an amber "Restart to apply" row when the list differs from what this run loaded; its nav row (puzzle glyph) between Filter lists and Updates | `.st-card`, `.st-row`, `.st-row-ctl`, the Filter lists section | settings-extensions.png |
| Settings section "Agents (MCP)" (`components/agents-section.slint`) | a Card of Rows: the hosting switch (sm Toggle, the row taps it too) over its status line; "Agent apps" over what turning it on adds wubble to (or, while on, where it is); a row per agent app found, its file and state in the note (amber when the owner took wubble out of it) and a ghost Remove, Add or "Copy entry" button; "Relay command" and "Server entry (any app)" with Copy buttons; "Agent activity" with a ghost Open; a closing note on what being findable means; its nav row (agent glyph) between Extensions and Updates | `.st-card`, `.st-row`, `.st-row-ctl`, the Extensions section | settings-agents.png |

## Agent sessions (mcp5, xm/mcp)

The design has no agent surface. Each piece is composed of kit parts; the owner's eye is the judge.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Approval sheet (`components/agent-sheet.slint`, kit `Sheet` `enter-accepts`) | the print sheet's anatomy: the derived Sheet over a settings card (`.st-card`) holding, as stacked rows (`BStackRow`: the title over the control), Secrets as the kit list editor (one "Name" column, "None asked for" when empty, remove only: a name is only what a client asks for), Sites as the kit list editor (one "Site" column, "Any site" when empty; add, edit in place and remove, a refused draft keeps its coral note), both as Settings' list settings draw them; then the Window row (the compact Segmented, Windowed / Headless, starting from what the client asked); a note that no secret is bound to a login yet, and a note on how to stop; Allow primary, Deny ghost. Enter never approves (`enter-accepts: false`: the sheet may appear while the user types); Escape and Deny refuse; the window's, one at a time, in the one overlay stack | `.st-card`, `.st-row`, the list editor, the print sheet | gallery agent-sheet |
| Agent mark (`kit/chrome/agent-mark.slint`, in `tab-row.slint` and `SpaceName`) | the tab row's close tile (`.wb-tab-x`: 22, radius 7, white .1 on hover) holding the agent glyph (`Glyph.agent`) in the sky, coral on hover where a click stops every agent session (Stop All Agents); a button "Stop all agents" to assistive tech; not shown on the compact rail (icon-only rows; Stop All Agents in the command bar and the mark on the Space header still work); after the speaker on a driven tab's row, at the end of the Space header of the session's Space; no steady glow (a new primitive: waits for the owner) | `.wb-tab-x`, Palette sky / coral | gallery agent-tabs |
| Request-more sheet (`components/agent-sheet.slint`, `view.more`) | the approval sheet again for `request_more`: "Let the agent use more?", a message that only what is new is listed and nothing changes without Allow; Secrets and Sites as on the approval sheet holding only the new names and sites ("None asked for" when a list is empty); no Window row (the session's mode stays) | the approval sheet | gallery agent-sheet-more |
| Agent Space dustbin (`kit/chrome/spaces-menu.slint`, on an agent Space's row) | the kit IconButton (`.wb-ib`, 30 square, white .07 under it on hover) at the row's right, the label cut before it, the generated set having no bin: `Glyph.trash` (`Derived.trash`, already the download row's remove), in the rows' icon ink at rest (`.tp-mrow svg`) and the menu's danger ink on hover (`.tp-mrow.danger`); enters with its row (the row's tween). A press clears that agent Space (a running session stops, a Space it left is deleted), the popup stays; no keyboard stop (Enter switches to the row's Space), "Clear <Space>" to assistive tech. A user's Space row has none | `.wb-ib`, `.tp-mrow svg`, `.tp-mrow.danger`, the download row's remove | gallery agent-spaces-popup, agent-spaces-popup-bin (pointer on it) |
| Activity page (`AgentLogPage`, `wubble://agent`) | the list page's own parts: PageFrame, ListCard, ListRow (a check or close glyph on the lead tile by outcome, the tool and page as title, the session, age and outcome as note, not tappable), the empty note; no search | list pages (section 32) | gallery agent-activity |
| Page icon (`Glyph.page`) | the generated `zap` glyph for `wubble://agent` (the set has no agent glyph) | `Glyph.page` | gallery agent-activity |

## Agent widgets (xm/orbs)

The design tunes the thinking orb at 64, 32 and 20 and shows it on its library page only (`alive-orbs.tsx`). These pieces are wubble's.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Orb at 16 and 24 (icon sizes) | the 20 design scaled to the side (dots and strokes scale with it) | ThinkingOrb `tuned: s20` | gallery agent-orbs |
| Orb at 160 (large) | the 64 design scaled to the side | ThinkingOrb `tuned: s64` | gallery agent-orbs-large |
| Agent widgets specimen (`kit/specimen/agent-widgets.slint`) | columns on scene glass (GlassPanel thick), the library's stage (`.al` `--al-stage`), its ink as a paper ground, and the bare page; state captions in the label role, `--al-muted` on dark | library.css `.al`, `.al-stage` | gallery agent-orbs, agent-orbs-dark-page |
| Page icon (`Glyph.page`) | the agent glyph for `wubble://agent` | `Glyph.page` | gallery agent-activity |
| Agent glyph (`Glyph.agent`, `controls/icon.slint`) | **needs the owner's glyph.** The design's only assistant glyph is `sparkle` (`pages/touch-kit.tsx:152`, `M12 4v4M12 16v4M4 12h4M16 12h4`: four ticks, unused by the design itself), generated into `KitIcons.sparkle`; `Glyph.agent` aliases it, in this one place, for the agent Space's tile (footer, Spaces popup, Settings list: `Glyph.space("agent")`, from `space::glyph_id`), the agent badge, the Space header mark and the activity page. Replace the one line when the owner has a glyph; the old `zap` (a bolt of our own) is gone | `Glyph.space`, `Glyph.page` | gallery agent-tabs, agent-spaces-popup |
| Agent badge (`kit/chrome/rail-badge.slint` with `inert`, used by `tab-row.slint`) | the rail close badge's circle (12 round, white .18, 8 px glyph at 2.4 in `.wb-tab-x:hover`'s ink) holding the agent glyph, centred on the favicon's bottom-right corner, always shown while the tab is an agent's, on the extended row and the rail. An indicator only: no pointer input, no hover light, no accessible element; a press on it picks the tab like the rest of the favicon. Stopping stays on the extended row's agent mark, the Space header mark and the command bar (the rail has no row mark) | `.wb-light`, `.wb-tab-x` | gallery agent-tabs, agent-tabs-rail |

## Settings side column (xm/settings-nav)

In the design the pinned column (`.st-pin`) rides the one scroll view, so a column taller than the window can never be scrolled to its end by the pointer over it. wubble's window is often shorter than the section list plus its "Pages" group (owner, 2026-11-20).

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Section list scroller (`NavScroller` in `kit/settings/page.slint`, `NavScroll` global) | The title, the search and its count stay pinned (`.st-pin`: 24 below the top once the page scrolls, as before); the section list and the Pages group under them scroll on the kit scroller (`WubbleScroller`: the same wheel physics, rubber band and `.wb-thumb`) on their own offset, when the window is shorter than the list. The column ends 24 above the window's foot (`Shell.settings-pin`). The wheel goes to the column under the pointer; over the list when it fits, or over the title and search, it scrolls the page as before. Hovering moves no focus and picks nothing. When the current section changes (the page scrolled, or a pick) its row is brought into view with the least travel. | `.st-pin`, `.st-nav`, `.wb-thumb` | gallery settings-nav-end |

## Page drag and drop (pf/dnd)

Chrome draws a page's drag with the image Blink made for it (a grey label for a link, the picture or the selection at 75 %, the page's own `setDragImage`); wubble draws that image as it is. These pieces are wubble's: the design repo has no drag image and no drop target.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Drag ghost (`kit/chrome/drag-ghost.slint`) | Chromium's drag image at the pointer, its hotspot under it, untouched; without one, the tab drag's ghost row (shell fill, radius sm, `.wb-tab-drag`'s shadow) holding the title in `wb-tab` and under it the URL in `wb-cmd-hint` mist, as Chromium's label orders them; a dragged tab's ghost is the same piece with the title alone (moved out of app.slint) | `.wb-tab-drag`, the tab row's text | gallery drag-ghost-label, drag-ghost-image; `--smoke dnd --shot-dir` (Chromium's own images) |
| Drop mark (`kit/chrome/drop-mark.slint`) | the split drop zone's box (ink .12 fill, ink .35 rim of 2, radius md; moved out of app.slint) over the address pill, the New tab row, the open command bar's field, the bookmarks page and a bookmarks folder row (`ListRow` `drop-target`); its rim alone as a 2 px line in the tab list's gap; a Space row is lit as the tab drag lights it | the split drop zone | gallery drag-ghost-tab-list, drag-ghost-bar |

## Page layer hole (page-layer stage 2)

The new primitive the owner approved on 2026-11-23. The web design has no counterpart: a page there is the browser's own. It has no look: what shows is the page itself, from its OS layer under the shell (`--page-layer=window`, macOS).

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Page hole (`kit/material/page-hole.slint`, `PageHole` and its `PageHoleSource` global; composed by `kit/chrome/split-view.slint` as `pane-hole`) | the pane's frame image replaced by an image the vendored renderer clears with (`BlendMode::Clear`, Skia change 3): the pane's fill and the window's ground, glass and scene under it removed, in the pane's own clip (`Radius.md`, none in page fullscreen), the status bubble and the close button still drawn over it; the page's layer behind it has the same corner radius from the same generated token. Drawn only while the pane's page is on its layer (`SplitPane.hole`) | `SplitView`'s pane (`webview.tsx` `.wb-view`), `Radius.md` | none: the hole is transparent in a shot (`--shot` and the gallery hold the page on the shell, `Need::Pixels`); judged on the Mac with `--page-layer=window` |

## Private windows (feat/private-windows)

Chrome marks an incognito window with a dark frame and an "Incognito" label; the design has no private browsing. These pieces are wubble's, for the owner's eye.

| Piece | derived | rule it follows | shot |
|---|---|---|---|
| Private chip (`PrivateChip` in `kit/chrome/private-mark.slint`, in `SpaceName`) | a private window's Space header shows its name ("Private") as a pill in place of the plain name: the header's own type (`wb-space-name`, 12 / 500) in lilac, on lilac at the Space capsule's .2 (`.wb-spaces-ind` opacity), 18 high (the header's height), the header's 10 padding each side, radius half its height; inert, named to assistive tech with "This window is private: nothing of it is saved". The private Space's colour is the lilac swatch | `.wb-space-name`, `.wb-spaces-ind`, Palette lilac | not yet shot (no gallery scene) |
| Private tint (`PrivateTint` in `kit/chrome/private-mark.slint`, in `app.slint` over the glass) | the window's ground and glass darkened by abyss at .35 (`Shell.private-tint`), under the sidebar's content and the page card, with the window's radius; not drawn in page fullscreen. The window title reads "wubble (Private)" | Palette abyss, Chrome's dark incognito frame | not yet shot (no gallery scene) |
