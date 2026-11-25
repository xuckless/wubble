# Fidelity audit F: wubble-only surfaces

Status (2026-11-19): the audit that scoped lane L6 and the derived pieces; the surfaces it lists are now rebuilt on kit components ([todo/shelved/review.md](../../todo/shelved/review.md) "Design fidelity"). Line numbers are as of the audit.

Read-only audit, 19 November 2026. It covers every `.slint` file under
`crates/wubble-ui/ui/`, the gallery scenes (`crates/wubble/src/gallery_smoke.rs` `SCENES`,
`host/gallery.rs`), the windows wubble opens (Main, Little, Popup, Pip) and the in-page UI that
wubble injects (`crates/wubble/assets/{reader,hints,kit}.js`). The design reference is the web
version at the design repo (`xuckless/the-wallpaper-project`, `src/wubble`) (HEAD `598f814`, "coloured
frosted glass everywhere").

The owner's rule: wherever wubble has a surface that the web design lacks, the surface is
**derived** from the design's existing primitives and never invented. This file:

1. maps every wubble surface to its design counterpart, or marks it as having none;
2. for each surface with no counterpart, records where it lives, what it shows, how it looks now
   (with the invented values called out), which design primitives it should be built from, and
   which design anatomy it should be a variant of;
3. lists the surfaces that genuinely need a new primitive, which the owner must approve.

Path shorthand: **W** = `crates/wubble-ui/ui/`, **D** =
the design repo's `src/wubble/`. Line numbers are as of this audit.

## Findings that apply to many surfaces

These come up again and again in the per-surface sections. Each is stated once here.

- **G1. The overlays are copied from Ultra Reactivate, but the design now has its own
  primitives.** `W/app-tokens.slint:198-227` says "The design repo has no menu, sheet, toast or
  find bar", so the menu, sheet, toast, find bar and their buttons take their numbers from
  `src/ultra-reactivate/overlays.css` (menu 220 wide, radius 14, rows 40 / radius 9; sheet
  padding 26, title 19/560, body 14.5, buttons 42 high / radius 11, in over 300 ms). The design
  does now have a menu: the touch-kit **Menu** at `D/pages/touch.tsx:458-552` and
  `D/pages/touch.css:133-158`, reused as the settings **Select** at `D/pages/settings.tsx:200-265`.
  Its numbers are:
  - thick glass, 236 wide, padding 6, radius 16, rows 36 high with radius 10, text 13.5;
  - grows out of its trigger: scale 0.95 and blur 4 → 1 over 220 ms out-curve;
  - rows arrive 40 ms after the body, then 30 ms apart, each from 4 px above;
  - leaves in 176 ms with no stagger;
  - one highlight glides between rows (`.tp-menu-hl`: white 9 %, coral 12 % on a danger row,
    160 ms out-curve);
  - shortcuts in `kbd` mono 11.5 haze.

  The design also has **Buttons** (`touch.tsx:440-466`, `touch.css:120-131`): 44 high, radius
  12, 14/600; `pri` is the accent fill with abyss text, `glass` is `.wb-thin` with the film rim,
  `ghost` is mist and turns white 7 % on hover; press 0.97, then the release squash at a third.
  Every wubble overlay should be rebased on these, and the UR values dropped from `Shell`.
- **G2. `SheetButton` primary is not the design's primary.** At `W/components/sheet-button.slint:13`
  the primary button is `Palette.shell` (rim on hover), and the secondary is transparent turning
  `shell` on hover. The design's primary is the **accent fill** (`touch.css:124`). Every sheet,
  prompt, error page and card button inherits this. `PressableButton` hovers to `Palette.shell`
  (`W/components.slint:208`), where the design's `.wb-ib` uses white 7 % (`browser.css:36`).
- **G3. Destructive actions should be holds.** "Destructive means hold" (README Settings;
  `HoldButton` at `D/pages/settings.tsx:300-352`, `settings.css:104-109`). Wubble uses a confirm
  sheet for clearing history (`W/components/pages.slint:200-221`) and one-click bins for history
  rows, bookmarks, passwords, Boosts and Task-manager End.
- **G4. Confirm in place instead of with a notice.** The design confirms a choice in place: the
  trigger's label crossfades to the result (Menu `confirmMs`, `touch.tsx:468-477`;
  **ConfirmButton** at `settings.tsx:354-381`, mint check and one `wb-pulse`). Wubble shows toasts
  instead, and its Copy and Save buttons give no confirmation of their own.
- **G5. Mono is for addresses and numbers.** README "Avoid #17": Geist Mono only for addresses,
  numeric values and code. Several surfaces break this rule in both directions (the status-bubble
  URL is in the UI face; Task-manager numbers and the find counter are not tabular mono).
- **G6. The glass tiers are from before `598f814`.** `W/tokens.slint:107-119` still has thin
  16 / .08, regular 30 / rgba(22,25,33,.42) and thick 44 / rgba(28,31,40,.64), plus a specular
  band and a 22 % top rim. The design's coloured frost (`D/core/tokens.ts:49-57`,
  `wubble.css:22-31`) is now:
  - thin: blur 24, white .09;
  - regular: blur 36, rgb(190 198 216 / .11);
  - thick: blur 48, brightness .55, rgb(70 74 86 / .42);
  - no specular band, a 3.5 % breath of light at the top.

  `Shell.thick-unblurred-fill` (.94, `app-tokens.slint:90`) is a stand-in that every
  over-content overlay uses. This affects every surface below; audit part A/B owns the fix.
- **G7. A "danger" colour is used inconsistently.** The design's danger colour is coral
  (`.tp-mrow.danger`, `.st-hold`). The Boost error uses blush (`boost-panel.slint:136`), the
  bookmark star is amber and the cert lock is coral. Blush is the Pin's colour, and amber is a
  film colour for values (`.wb-n`), not a state.

## Every wubble surface

The Counterpart column has three values:

- **direct**: the design has the same surface;
- **primitive**: the design has no such surface but has the primitive it should be built from;
- **none**: neither.

In the Section column, **§** points to the section below.

### Window, chrome, sidebar

| # | Wubble surface | Wubble file:line | Counterpart | Design | Section |
|---|---|---|---|---|---|
| 1 | Living backdrop and glass scene | `W/app.slint:680-700` | direct | `D/core/gl/`, README Shaders | |
| 2 | Window edges and move area | `W/components/window-frame.slint:11-86` | direct | `browser.css:150-159`, `desktop.tsx:22` | |
| 3 | Window lights | `W/components/window-lights.slint:13,62` | direct | `browser.css:23-31`, `chrome.tsx:83` | |
| 4 | Sidebar: extended, rail, hidden | `W/components/sidebar.slint:51` | direct | `chrome.tsx:93`, `browser.css:11-21`, `room.tsx:298` | |
| 5 | Hidden-sidebar peek | `W/app.slint:1146-1200` | direct | `browser.css:20-21,148`, `chrome.tsx:104,250` | |
| 6 | Sidebar seam (string, pill, grab) | `W/components.slint:487` | direct | `seam.ts`, `browser.css:138-147` | |
| 7 | Address pill and border beam | `W/components/address-pill.slint:15`, `border-beam.slint:20` | direct | `chrome.tsx:32-69`, `browser.css:41-47` | |
| 8 | Address pill extras: blocker shield and count, pop-ups, media, reader book, bookmark star | `W/components/address-pill.slint:32-60,186-257` | primitive | `.wb-ib`, touch **Pin** | §1 |
| 9 | Nav buttons and IconButton | `W/components/nav-buttons.slint:14,50` | direct | `browser.css:33-38` | |
| 10 | Favourites grid | `W/components/fav-grid.slint:15` | direct | `browser.css:49-55`, `chrome.tsx:70` | |
| 11 | Space name and Space switcher | `W/components/sidebar.slint` | direct | `tabs.tsx:140`, `browser.css:91-96` | |
| 12 | Tab row, indicator, split glyph | `W/components.slint:298` | direct | `tabs.tsx:25`, `browser.css:60-80,118` | |
| 13 | Tab row speaker glyph | `W/components.slint:403-420` | primitive | `.wb-tab-split` slot | §2 |
| 14 | New-tab row | `W/components/new-tab-row.slint:11` | direct | `browser.css:81-83` | |
| 15 | Tab folder header | `W/components/folder-row.slint:11` | none | | §3 |
| 16 | Sidebar foot: Spaces, dev tools, settings | `W/components/sidebar.slint:530-600` | direct | `browser.css:85-89` | |
| 17 | Sidebar foot: downloads button | `W/components/sidebar.slint:540-553` | primitive | `.wb-ib`, `wb-pulse` | §8 |
| 18 | Tab drag-out ghost and split drop zone | `W/app.slint:258-266,1339-1366` | primitive | `.wb-tab-drag`, split open | §4 |
| 19 | Command bar | `W/components/command-bar.slint:16` | direct | `command.tsx:22`, `browser.css:161-180` | |
| 20 | Command result row | `W/components/command-row.slint:13` | direct | `browser.css:173-180` | |
| 21 | Web view and page swap (`exit-page`) | `W/app.slint:762-772` | direct | `webview.tsx:118`, `wubble.css:53-58` | |
| 22 | Split panes, seams, focus ring, pane close | `W/components.slint:651` | direct | `split.ts`, `browser.css:108-119` | |
| 23 | Page scroller | `W/components.slint:539` | direct | `scroller.tsx:33` | |
| 24 | DevTools dock (pane on the Source seam) | `crates/wubble/src/host/devtools.rs:1-30`, `W/components.slint:626` | primitive | Source panel `sourcepanel.tsx:44`, `browser.css:182-203` | §5 |
| 25 | Tooltip | `W/components/tooltip.slint:20` | direct | touch Dock tooltip `touch.tsx:368-436`, `touch.css:100-107` | |

### Shared primitives in wubble

| # | Wubble surface | Wubble file:line | Counterpart | Design | Section |
|---|---|---|---|---|---|
| 26 | GlassPanel, film rim, GlowRing | `W/components.slint:70`, `W/components/glow-ring.slint:8,25` | direct | `wubble.css:15-44` (see G6) | |
| 27 | PressableButton | `W/components.slint:178` | direct | `wubble.css:48-49`, touch Buttons (see G2) | |
| 28 | OverlapLayer and Entering | `W/components.slint:267`, `W/components/entering.slint:8` | direct | `wubble.css:51-58` | |
| 29 | Wobble and Squash | `W/components/wobble.slint`, `W/components/squash.slint` | direct | `wubble.css:60-68`, `touch-kit.tsx:56-78` | |
| 30 | FocusRing | `W/components/focus-ring.slint:10` | direct | `wubble.css:13` | |
| 31 | Icons | `W/components/icon.slint:8` | direct | `icons.tsx`, `touch-kit.tsx:128-153` | (see below) |
| 32 | Raised and Chip | `W/components/raised.slint:12,73` | direct | `browser.css:63-64`, `settings.css:25,40`, `wubble.css:26` | |

Icons: some hand-drawn wubble glyphs duplicate glyphs the bucket already has. `book` and `play`
are in `touch-kit.tsx` PATH, and `chevronRight` and `check` are there too. Use the bucket's.

### Settings and its controls

| # | Wubble surface | Wubble file:line | Counterpart | Design | Section |
|---|---|---|---|---|---|
| 33 | Settings page | `W/components/settings.slint:970` | direct | `settings.tsx:411`, `settings.css` | |
| 34 | Toggle | `W/components/controls/toggle.slint:15` | direct | `touch.tsx:147-224`, `touch.css:61-71` | |
| 35 | Slider | `W/components/controls/slider.slint:16` | direct | ElasticSlider `touch.tsx:43` | |
| 36 | Segmented | `W/components/settings.slint:37` | direct | `touch-kit.tsx:195-207`, `settings.css:66-69` | |
| 37 | Swatches | `W/components/settings.slint:700` | direct | `settings.tsx:267`, `settings.css:87-92` | |
| 38 | Kbd and BindingRow | `W/components/settings.slint:611,639` | direct (rows) | `settings.css:96-98` | §6 (capture state) |
| 39 | TextButton | `W/components/settings.slint:315` | direct | `.st-reset`, `settings.css:50-54` | |
| 40 | Field: capsule search and plain field | `W/components/settings.slint:167` | direct (capsule) / primitive (plain) | `.st-search`, `settings.css:11-19` | §6 |
| 41 | Stepper | `W/components/settings.slint:273` | none | | §6 |
| 42 | EntryList | `W/components/settings.slint:370` | none | | §6 |
| 43 | CheckBox | `W/components/check-box.slint:10` | direct, deviates | touch Checks `touch.tsx:554-576`, `touch.css:160-174` | §7 |
| 44 | SheetButton | `W/components/sheet-button.slint:8` | direct, deviates | touch Buttons `touch.tsx:440-466` (see G2) | |

### Overlays over the page

| # | Wubble surface | Wubble file:line | Counterpart | Design | Section |
|---|---|---|---|---|---|
| 45 | Tab-row menu (gallery `tab-row-menu`) | `W/components/context-menu.slint:15` | direct | touch Menu, whose rows are tab actions (`touch.tsx:478-483`) | §9 |
| 46 | Page context menu | `W/components/context-menu.slint:15` | primitive | touch Menu | §9 |
| 47 | Site menu (gallery `site-menu`: blocker, page theme, pop-ups, media) | `ContextMenu` at a pill badge | primitive | touch Menu, settings Select | §9 |
| 48 | Downloads popover | `W/components/downloads-popover.slint:14` | none | | §8 |
| 49 | Download row (popover and page) | `W/components/download-row.slint:11` | none | | §8 |
| 50 | Find bar | `W/components/find-bar.slint:12` | none | | §10 |
| 51 | Permission and external-protocol prompt | `W/components/permission-prompt.slint:18` | none | | §11 |
| 52 | Autofill key chip, login menu, save/update prompt | `W/components/autofill-menu.slint:30` | none | | §12 |
| 53 | Sheet frame (scrim, thick sheet, actions) | `W/components/sheet-frame.slint:15` | none | | §13 |
| 54 | JS dialog and HTTP sign-in sheet | `W/components/dialog-sheet.slint:16` | none | | §13 |
| 55 | Bookmark sheet | `W/components/bookmark-sheet.slint:11` | none | | §14 |
| 56 | Print sheet | `W/components/print-sheet.slint:30` | none | | §15 |
| 57 | Toast or notice (zoom, full screen, SponsorBlock) | `W/components/toast.slint:19` | none | | §16 |
| 58 | Link status bubble | `W/components/status-bubble.slint:16` | none | | §17 |
| 59 | Tab hover preview card | `W/components/preview-card.slint:13` | none | | §18 |
| 60 | Glance (link peek) card | `W/components/glance-card.slint:15` | none | | §19 |
| 61 | Error, certificate and crash panel | `W/components/error-panel.slint:15` | primitive | `.wb-site-missing`, start column | §20 |
| 62 | Picture-in-picture stand-in card | `W/components/pip-card.slint:10` | none | | §21 |
| 63 | Touchpad history swipe chip | `W/components/swipe-chip.slint:10` | none | | §22 |
| 64 | Middle-click autoscroll cursor | `W/components/autoscroll-cursor.slint:11` | none | | §23 |
| 65 | Screenshot region picker and toolbar | `W/components/shot-overlay.slint:13` | none | | §24 |
| 66 | Reader controls | `W/components/reader-controls.slint:15` | none | | §25 |
| 67 | Reader page stylesheet (in-page) | `crates/wubble/assets/reader.js:12-36`, `crates/wubble-core/src/reader.rs:202-208` | none | | §25 |
| 68 | Link-hint labels (in-page) | `crates/wubble/assets/hints.js:63-90`, `crates/wubble/src/host/builtins.rs:925-933` | none | | §26 |
| 69 | Zap element outline (in-page) | `crates/wubble/assets/kit.js:254-256` | none | | §26 |
| 70 | Video strip, scrubber, speed menu, collapsed tab | `W/components/video-controls.slint:22` | none | | §27 |
| 71 | Picture-in-picture controls | `W/components/pip-controls.slint:17` | none | | §28 |
| 72 | Boost editor panel | `W/components/boost-panel.slint:60` | none | | §29 |
| 73 | Engine popup layer (`<select>`, date picker) | `W/components/popup-layer.slint:9` | none | | §30 |
| 74 | Page accessibility layer (never drawn) | `W/components/page-ax.slint:12` | none, no visuals | | §31 |

### Internal pages

| # | Wubble surface | Wubble file:line | Counterpart | Design | Section |
|---|---|---|---|---|---|
| 75 | PageFrame (list pages' shell) | `W/components/settings.slint:905` | primitive | settings layout `settings.css:6-31` | §32 |
| 76 | History page and clear-data sheet | `W/components/pages.slint:102-221` | none | | §32 |
| 77 | Archive page | `W/app.slint:977` (HistoryPage) | none | | §32 |
| 78 | Bookmarks page | `W/components/pages.slint:225` | none | | §32 |
| 79 | Downloads page | `W/components/pages.slint:315` | none | | §32 |
| 80 | Passwords page: locked, unlocked, add sheet | `W/components/pages.slint:348` | none | | §32 |
| 81 | Task manager | `W/components/pages.slint:540` | none | | §32 |

### Windows and engine-drawn pages

| # | Wubble surface | Wubble file:line | Counterpart | Design | Section |
|---|---|---|---|---|---|
| 82 | Little window (a link from another app) | `crates/wubble/src/main.rs:700-706`, `crates/wubble/src/menu.rs:735-768` | none | | §33 |
| 83 | Popup window (`window.open` with a size) | same | none | | §33 |
| 84 | Pip window | `W/app.slint:55-66` | none | | §28 |
| 85 | Page dark mode, PDF viewer | engine and CSS only, no wubble chrome | n/a | | |

### Design surfaces wubble does not have yet

These are outside this part's scope, listed so that nobody invents a replacement:

- **Start page.** `D/pages/start.tsx:16-55`, `start.css`: the living bubble, "Where to next?", the
  search capsule and the site shortcuts. Without a URL, wubble shows the smoke page
  `crates/wubble/assets/page.html`.
- **Settings controls never used.** The Select menu, HoldButton, ConfirmButton, the **Pin**
  (`touch.tsx:244-300`), the stretch **Download** button (`touch.tsx:300-358`) and the **Dock**
  (`touch.tsx:360-436`). The sections below lean on all of them.

---

## Surfaces without a direct counterpart

Each section has the same parts: **Where**, **Shows/states**, **Now** (the current look and
motion, with invented values marked *inv.*), **Compose from** (design primitives, with
citations), **Anatomy** (the design anatomy it should be a variant of), and **Note** where
needed.

### §1 Address pill extras (#8)

- **Where:** `W/components/address-pill.slint:32-60` (properties), `:186-257` (shield badge,
  reader book, star). The media glyph comes just before.
- **Shows/states:**
  - blocker shield: mint while it blocks, haze when the site is allowed, plus a count;
  - pop-ups held back, with a count;
  - a media stream that can be saved;
  - reader available, or reader on (sky);
  - bookmark star: haze, or filled amber when bookmarked.
- **Now:** bare 13 px glyphs at the pill's right end with no hit tile. The colour change is
  immediate, with no motion. Amber for "bookmarked" is *inv.* (G7). The count is mono, which is
  correct (G5).
- **Compose from:**
  - Each glyph is a `.wb-ib` tile (`browser.css:34-38`: 30 px, radius 9, white 7 % on hover;
    `.wb-ib-on` white 10 % for reader on), shrunk to the pill's 36 px height.
  - A glyph that appears wobbles once (`wubble.css:62-68`).
  - The star is the touch **Pin** (`touch.tsx:244-300`, `touch.css:73-80`): the ribbon twangs,
    blush pours in, and the stroke turns blush when on. That gives a bookmark motion that is the
    owner's own.
  - The blocked count rolls like the Pin's count (`.tp-pin-count`, tabular digits).
  - Reader-on takes the accent, as `.st-nav-btn.on svg` does (`settings.css:29`).
- **Anatomy:** the `.wb-cmd-hint` slot of the command row (`browser.css:179`), holding `.wb-ib`
  icon buttons.

### §2 Tab-row speaker glyph (#13)

- **Where:** `W/components.slint:403-420`.
- **Shows/states:** audible or muted. A click toggles mute.
- **Now:** a 22 px slot (`Shell.speaker` *inv.* hand-drawn glyph), haze turning ink on hover.
- **Compose from:** the `.wb-tab-split` glyph slot (`browser.css:118-119`: 22 wide, haze, gives
  way to `.wb-tab-x` on hover), and a wobble on toggle (`wubble.css:62`).
- **Anatomy:** the tab row's trailing glyph slot.

### §3 Tab folder header (#15)

- **Where:** `W/components/folder-row.slint:11-89`.
- **Shows/states:**
  - open or folded (chevron down or right), with the name and tab count;
  - rail: a folder glyph only;
  - hover and press; right-click opens a menu.
- **Now:** `.wb-tab` metrics, which is correct. The chevron swaps paths with no rotation. The
  count is 12 px haze in the UI face. The folded rows disappear with the list springs.
- **Compose from:**
  - The title in `.wb-space-name` type (`browser.css:57`, 12/500 mist), or the tab title type.
  - The chevron rotates as `.tp-chev` does (`touch.css:139-140`, 220 ms out-curve) rather than
    swapping glyphs.
  - Rows fold away like the settings **Fold** (`settings.tsx:173-183`, `settings.css:56-61`:
    in 240, out 190, blur 4, shift 7).
  - The count is a numeric value, so tabular mono (G5).
- **Anatomy:** a `.wb-tab` row acting as a group heading. The settings group heading
  (`.st-group`, `settings.css:39`) is the closest semantic match.

### §4 Tab drag-out ghost and split drop zone (#18)

- **Where:** `W/app.slint:258-266` (properties), `:1339-1366` (drawing).
- **Shows/states:** the title follows the pointer, and the left or right quarter of the content
  is lit as a split target.
- **Now:**
  - Ghost: `Palette.shell`, radius 10, shadow blur 12 at .25 (`Shell.drag-ghost-*` *inv.*).
  - Zone: ink 12 % fill with a 2 px ink 35 % border (*inv.*). That is a bordered container,
    against README "Avoid #8".
- **Compose from:**
  - The ghost is `.wb-tab-drag` (`browser.css:70`: white 10 %, shadow 0 10 24 / .4), with the
    tab's stretch along its travel (stretch spring, README Motion tokens).
  - The zone previews the split as the design opens one: the new page grows from no width beside
    the page it joins (README Split view; `split.ts:22-48`). The held seam line takes the sky
    stroke (`.wb-split-line`, `browser.css:114-115`). No box.
- **Anatomy:** a split pane being opened, with a held seam.

### §5 DevTools dock (#24)

- **Where:** `crates/wubble/src/host/devtools.rs:1-30`. A host-only pane at the right edge on the
  Source seam (`SourceSeam`), drawn as a `SplitView` pane with a close button
  (`W/components.slint:626`).
- **Shows/states:** closed, or open docked with the DevTools frontend's title.
- **Now:** a plain split pane, with the split pane's close button and ring. The design's Source
  panel head is not used.
- **Compose from:** the Source panel (`sourcepanel.tsx:44-111`; `browser.css:182-203`):
  - right 8, radius 14, 320–720 px on the same string, closing when pulled under 200;
  - a 46 px head with the file title and a segmented switch;
  - its close is the panel button (`.rm-tool`, `room.css:14`).
- **Anatomy:** the Source panel. Only the head is wubble's to draw; the body is the DevTools page.

### §6 Settings controls without a counterpart (#38 capture, #40 plain field, #41 Stepper, #42 EntryList)

- **Where:** `W/components/settings.slint:167` (Field), `:273` (Stepper), `:370` (EntryList),
  `:639-700` (BindingRow capture: "Press a shortcut", conflict note).
- **Now:**
  - The plain field is a `Chip` with radius 10 and height 36 (`Shell.field-height` *inv.*), and
    no film rim on focus.
  - Stepper is minus, an 88 px field (`Shell.stepper-field` *inv.*) and plus.
  - EntryList is rows with a remove button and an add field.
  - The capture state only changes the text.
- **Compose from:**
  - **Plain field:** the `.st-search` capsule (`settings.css:11-19`) at the control height:
    `.wb-thin` body, film rim while focused (`wubble.css:35-44`), radius 12 (`Shell.control-radius`)
    rather than a capsule. Hit and clear button as `.st-clear`.
  - **Stepper:** the design sets counted values with the **ElasticSlider** (zoom 50–200 step 10,
    pages kept alive 2–12: `settings.tsx:535-537`). Every stepper (settings, print scale, copies)
    should become a slider; see "new primitives" if typed entry is required.
  - **EntryList:** `.st-card` rows (`settings.css:40-47`). Remove is a hover-revealed `.wb-tab-x`
    (`browser.css:75-78`). New rows enter `wb-in`, removed ones leave `wb-tab-out`
    (`browser.css:71-72`). The add field is the plain field above.
  - **Capture:** while listening, the kbd group lights the film rim (touched sheen,
    `wubble.css:41-43`). A conflict note crossfades in with **Swap** (`touch-kit.tsx:80-107`).
- **Anatomy:** a settings row (`.st-row`) with its control.

### §7 CheckBox (#43, counterpart with deviations)

- **Where:** `W/components/check-box.slint:10-92`. Used by the permission prompt and the print
  sheet.
- **Now:** a 16 px box (`Spacing.s4`), radius 3, a 1 px haze or mint rim, a **mint** fill, label
  12/500 mist. Motion is correct: circle fill, tick 70 ms behind, squash.
- **Design** (`touch.css:160-174`):
  - box 22, radius 7, white 8 % with an inset 20 % top and 10 % rim;
  - fill is the **Space accent** (`--tp-accent`), the tick in abyss;
  - row padding 7/8, radius 8, white 6 % on hover, label 14 ink;
  - the whole row is the target (`.st-ck`, `settings.css:83-85`).
- **Fix:** match these numbers. No new primitive.

### §8 Downloads: popover, row, sidebar button (#17, #48, #49)

- **Where:** `W/components/downloads-popover.slint:14-96`, `W/components/download-row.slint:11-122`,
  `W/components/sidebar.slint:540-553`.
- **Shows/states:**
  - a list (newest first, up to 6 rows of 60 px, then it scrolls);
  - each row is running, paused, interrupted, cancelled, failed or complete;
  - the actions on a row are text buttons: Pause, Resume, Cancel, Retry, Open, Show, and an
    × for remove.
- **Now:**
  - Popover: UR surface (G1): 360 wide, radius 14, opens from its top-left corner.
  - Rows: hover `command-selected` white 8 %, radius 9.
  - Progress: a 3 px rim-coloured track with a sky fill (`Shell.download-track` *inv.*,
    `Palette.rim` track *inv.*).
  - Glyph: mint when done.
  - The buttons are `PressableButton`s with 12/500 text. Hover goes to `Palette.shell`, which is
    *inv.* (G2).
  - The sidebar button gives no feedback when a download starts or finishes.
- **Compose from:**
  - **Body:** the touch **Menu** (G1), grown from its trigger, the sidebar download `.wb-ib`.
    Transform origin at the button; it opens upward, since the button is at the foot
    (`settings.css:79` `.st-menu.up`).
  - **Row:** the command-bar result row (`browser.css:175-179`: 44 high, 18 px icon in mist,
    label 14, hint 12 haze), which is two-line here, so `.st-row` title and note type
    (`settings.css:45-46`).
  - **Progress:** the **HoldButton** fill (`settings.css:105-109`) shows "colour fills from the
    left" as a determinate progress in the design's own language. Use it either as the row's
    underlay at a low alpha, or as the slider rail and fill (`touch.css:50-52`: white 12 %, inset
    top 10 %, accent fill) thinned. An indeterminate state uses the border beam lapping the row
    (README "Border beam … while it's thinking"), or the `.st-spin` spinner (`settings.css:114`).
  - **Actions:** `.wb-ib` icon buttons, with wobble on hit (`touch.tsx:376` DOCK), instead of
    text buttons. "Done" is the **Download** specimen's Swap to the check, plus `wb-pulse` in mint
    (`touch.tsx:300-358`, `touch.css:87`, `wubble.css:66`), on the row and on the sidebar button.
- **Anatomy:** touch Menu body with command-row rows.

### §9 Context menu, tab-row menu, site menu (#45-47)

- **Where:** `W/components/context-menu.slint:15-209`. One component for all three.
- **Shows/states:**
  - rows with an icon, label and shortcut;
  - separators, disabled rows and danger rows;
  - keyboard highlight; opens at the pointer, flips at the edges.
- **Now:**
  - UR values (G1): 220 wide, radius 14, rows 40 / radius 9, text 14, shortcut mono 12 haze.
  - The highlight is a per-row background fade (150 ms), not one gliding tile.
  - Rows stagger with an Overlap rise from below (+7 px). The design's rows drop from 4 px above,
    and the first row is 40 ms late.
  - Separator: a 1 px `Palette.rim` with 4 px each side (`Shell.menu-separator` 9 *inv.*).
- **Compose from:** the touch **Menu** exactly (`touch.tsx:458-552`, `touch.css:141-158`):
  - 236 wide, padding 6, radius 16, rows 36 / radius 10, text 13.5, icon 16 in mist;
  - shortcut `kbd` in mono 11.5 haze; submenu `.tp-mrow-sub` with `chevronRight`;
  - one gliding `.tp-menu-hl`, coral on a danger row;
  - opening: 220 ms / 0.95 / blur 4, origin at the trigger or pointer;
  - rows 40 + 30·i ms; exit 176 ms with no stagger.
  - The site menu's choices with a current value (page theme) are the settings **Select** rows
    with `.st-mcheck` (`settings.css:80-81`).
  - The tab-row menu is literally the specimen's rows (Duplicate, Move to Space, Copy link,
    Close tab, `touch.tsx:478-483`). Copy link should confirm in place (G4).
  - The separator has no design counterpart; use the `.st-group` spacing (a 6 px gap) rather than
    a rule, or ask the owner (see "new primitives").
- **Anatomy:** touch Menu.

### §10 Find bar (#50)

- **Where:** `W/components/find-bar.slint:12-136`, at the content's top right.
- **Shows/states:**
  - query, with "Find in page" as the placeholder;
  - counter ("3/12");
  - previous, next, close.
- **Now:**
  - UR surface: thick glass, radius 14, 360×44 (`Shell.find-*` *inv.*), enters at .95 from the
    top-right corner.
  - The counter is 12 haze in the UI face.
  - Buttons are `PressableButton`s with `Palette.shell` hover (G2).
- **Compose from:**
  - The field is the `.st-search` capsule (`settings.css:11-19`: 40 high, radius 20, 14 px, film
    rim while focused or holding a query) sitting in a thick-glass strip. Or the strip itself is
    the capsule, as the address and the start search are capsules (README "Avoid #1"
    exceptions).
  - The counter is tabular mono (G5) and changes with **Swap** `dir="x"` (`touch-kit.tsx:80-107`).
  - Buttons are `.wb-ib`.
  - Entrance `wb-pop-in` (`wubble.css:55-56`) with origin at the top right.
  - No match: the counter turns coral. No new colour.
- **Anatomy:** a search capsule (`.st-search`) on thick glass, a variant of the command bar head
  (`.wb-cmd-head`, `browser.css:169-172`).

### §11 Permission and external-protocol prompt (#51)

- **Where:** `W/components/permission-prompt.slint:18-111`, under the address pill, 320 wide.
- **Shows/states:**
  - "<site> wants to", one line per permission kind;
  - a "Remember for this site" checkbox, only where something can be remembered;
  - Block and Allow, or Cancel and Open for a link to another app.
- **Now:**
  - UR surface: thick glass, radius 14, padding 16; title 15/600; lines 14 mist.
  - `SheetButton`s (G2), and the deviating CheckBox (§7).
  - Drops in from its top-left corner at .95 over 220 ms.
- **Compose from:**
  - Body: the touch **Menu** body (G1), grown from its trigger, the pill's site glyph.
  - Each permission line is a settings permission row (`settings.tsx` ROWS "Site permissions",
    `:100-101`), showing the kind's icon as an `.st-lead` tile (`settings.css:93`: 36, radius 12,
    white 6 %).
  - "Remember" is a whole-row toggle (`.st-row.tap` with a `.tp-tg-sm` toggle,
    `touch.css:68-71`), or the touch checkbox (§7).
  - Buttons: touch **Buttons** `ghost` for Block and `pri` for Allow.
  - On answering, the trigger (the pill glyph) confirms in place (G4).
- **Anatomy:** touch Menu body holding settings rows.

### §12 Autofill: key chip, login menu, save prompt (#52)

- **Where:** `W/components/autofill-menu.slint:30-182`.
- **Shows/states:**
  - the key chip (shown on a sign-in form);
  - the menu of saved logins, plus "Suggest a strong password";
  - the save or update prompt: Never, Not now, Save/Update.
- **Now:**
  - The chip is an `IconButton` with a hand-drawn key (*inv.* glyph).
  - The menu is UR rows with text only, at 220 wide.
  - The prompt is the permission prompt's frame, with three `SheetButton`s.
- **Compose from:**
  - Chip: a `.wb-ib`. It lights the film rim while its menu is open (`.wb-on`, `wubble.css:41`).
  - Menu: touch **Menu**. Each row is the site favicon, the username as the label and the site in
    `.tp-mrow-sub` haze. "Suggest" gets the `sparkle` glyph (`touch-kit.tsx:152`).
  - Prompt: as §11. Never is a `ghost` button in coral (`.tp-mrow.danger` colour), Not now is
    `ghost`, Save is `pri`.
  - Once saved, the chip confirms in place with a check and `wb-pulse` (G4).
- **Anatomy:** touch Menu (menu); §11 (prompt).

### §13 Sheet frame and JS/HTTP-auth dialog (#53, #54)

- **Where:** `W/components/sheet-frame.slint:15-136`, `W/components/dialog-sheet.slint:16-145`.
  Also used by the clear-history and add-password sheets in `pages.slint`.
- **Shows/states:**
  - alert, confirm, prompt (one field) and beforeunload;
  - sign-in (user name and password);
  - a scrim blocks the page.
- **Now:**
  - UR dialog: min(380, w − 32) wide, padding 26, radius 20, 72 from the top.
  - Title 19/**560** in the UI face; body 14.5 mist.
  - Fields: a `Palette.shell` box, radius 11, 40 high (*inv.*), no focus rim.
  - In over **300** ms from .95 (top origin); out 176.
  - Scrim `rgb(5 6 9 / .32)`, which is the design's.
- **Compose from:**
  - **Placement and body:** the command bar (`browser.css:162-168`: scrim `.wb-scrim` with
    `wb-fade` .2 / .16; panel 72 from the top, radius 20, `wb-pop-in` .22 / `wb-pop-out` .176).
    That is the design's one modal, so the sheet is its sibling.
  - **Title:** the settings section heading (`.st-sec-head h2`, `settings.css:36`: **Fredoka
    19/600**). Same size, but in the display face.
  - **Message:** the `.st-row-note` / `.tp-head p` type (13–14 mist).
  - **Fields:** the plain field (§6). Labels as `.st-group` (13/500 mist, `settings.css:39`).
  - **Buttons:** touch **Buttons**: `ghost` for cancel, `pri` for accept.
  - A destructive accept (beforeunload "Leave") uses `pri` in coral, or hold (G3).
- **Anatomy:** the command bar's modal body, holding settings-row type.

### §14 Bookmark sheet (#55)

- **Where:** `W/components/bookmark-sheet.slint:11-103`.
- **Shows/states:**
  - name;
  - folder chooser: a tree of folder rows, the selected one filled `Shell.track` (*inv.* as
    selection);
  - "Remove bookmark" as coral text;
  - the page's rename, move and new-folder variants.
- **Compose from:**
  - The sheet frame (§13).
  - The folder chooser is the settings **Select** (`settings.tsx:200-265`): a glass trigger
    showing the current folder, and a menu from it with `.st-mcheck` on the chosen row. Depth is
    indented by the tree indent (`Shell.folder-indent`).
  - Remove is a touch **Buttons** `ghost` in coral.
  - The star in the pill is the **Pin** (§1); this sheet is what the Pin's long press or second
    click opens.
- **Anatomy:** a command-bar modal body with a settings Select row.

### §15 Print sheet (#56)

- **Where:** `W/components/print-sheet.slint:30-175`.
- **Shows/states:**
  - destination list (Save as PDF, then printers);
  - pages; layout, paper and margins as Segmented; scale and copies as Steppers;
  - background graphics;
  - Printing… (busy), and an error.
- **Now:**
  - The destination list uses the `Shell.track` selection fill (*inv.*).
  - Labels are 12/500 mist on the left.
  - Steppers (§6) and the CheckBox (§7) are used here.
  - Busy only changes the title.
- **Compose from:**
  - Lay the controls out as `.st-row`s in one `.st-card` (`settings.css:40-47`), not
    label|control lines.
  - Destination: the settings **Select**.
  - Layout, paper, margins: `.st-seg` (already used).
  - Scale: an **ElasticSlider** 10–200 step 10, as the zoom row is (`settings.tsx:535`).
  - Copies: an ElasticSlider 1–99, or see "new primitives".
  - Background: a whole-row toggle.
  - Print/Save: a **ConfirmButton** (`settings.tsx:354-381`: idle "Print", busy "Printing…" with
    `.st-spin`, done with a mint check and a pulse).
  - The error is a `.st-row-note` in coral.
- **Anatomy:** a command-bar modal body holding a settings card.

### §16 Toast / notice (#57)

- **Where:** `W/components/toast.slint:19-119`.
- **Shows/states:**
  - zoom level;
  - "Press Esc to exit full screen";
  - saved frame or screenshot;
  - SponsorBlock "Skipped sponsor: Undo" with an action;
  - a sticky one-time question with a close button.
- **Now:**
  - UR toast: thick glass, radius 14, 46 high, 14/540.
  - Rises 14 px from .95 on a **400 ms spring ease**; out 190.
  - Shown 1.5 s (Chromium's number). Max width 520 (*inv.*).
- **Compose from:**
  - **Zoom:** confirm in place instead (G4). The pill's address text Swaps to "110 %" in mono for
    1.4 s, then Swaps back (Menu `confirmMs` 1400, `touch.tsx:477`; **Swap**).
  - **Saved frame / screenshot:** the trigger button confirms in place (ConfirmButton).
  - **Full-screen hint and SponsorBlock undo:** these need a free-standing notice. Build it from:
    - thick glass and the `wb-pop-in` / `wb-pop-out` entrance (`wubble.css:55-56`), not the UR
      spring;
    - the tooltip's anatomy at a larger size (`.tp-tip`: a capsule is allowed for tooltip-like
      things, README "Avoid #1");
    - the action as a `ghost` button;
    - leaving with `wb-out`.
- **Anatomy:** a tooltip (capsule) on thick glass. See "new primitives": a free-standing notice
  is not in the design.

### §17 Link status bubble (#58)

- **Where:** `W/components/status-bubble.slint:16-62`, at the bottom-left of each pane.
- **Shows/states:** host and rest of the hovered URL. It moves to the right when the pointer comes
  near.
- **Now:**
  - `Palette.deep` .94 with a **1 px `Palette.rim` border** (*inv.*, against README "Avoid #8").
  - Capsule 24 high (*inv.*), 12 px **UI face**, the host at 600 (*inv.*).
- **Compose from:**
  - The address pill's text: `.wb-address-text` (`browser.css:46-47`): **mono** 13 (Type.address),
    the host in ink and the rest in haze. Breaks G5 as it stands.
  - Body: `.wb-thin` glass as a capsule (allowed, README "Avoid #1").
  - Motion: `.tp-tip` in and out (`touch.css:103-106`: .16 / .2 in, .13 / .16 out, rise 4,
    .95) instead of Chromium's 120 / 200 fades. Keep Chromium's 80 / 250 ms show and hide delays
    (behaviour, not look).
  - The move to the right is the tooltip glide (260 ms in-out, `DOCK.glideMs`).
- **Anatomy:** the address pill at tooltip size.

### §18 Tab hover preview card (#59)

- **Where:** `W/components/preview-card.slint:13-70`, beside the sidebar at the hovered row.
- **Shows/states:** title, address (mono 11.5 haze), and the last frame when it has one.
- **Now:**
  - UR tip values: thick glass, 240 wide, padding 10, radius 12, thumbnail radius 8 (*inv.*).
  - Title 13/540 (*inv.* weight).
  - In 180 / out 130 ms.
- **Compose from:**
  - The Room preview card (`room.tsx:150-160`, `room.css:16-25`): media on top with radius
    `r × 0.6`, `strong` 15/600 ink, `span` 13 mist, meta 12 haze. Its shadow is
    0 18 40 / .45 with an inset 22 % top.
  - Concentric radii: card 20, padding 8, media 12 (README tile rule, `browser.css:13-14`).
  - Motion: `.tp-tip` in and out. The first appearance waits for intent and then appears in
    place; while warm, the card glides between rows (`DOCK.intent` 280, `warm` 520, `glideMs`
    260; `touch.tsx:368-375`). Wubble already does this for the tooltip.
- **Anatomy:** Room preview card on thick glass; motion from the dock tooltip.

### §19 Glance / link peek card (#60)

- **Where:** `W/components/glance-card.slint:15-173`.
- **Shows/states:** a scrim; a header with title, "Open in split", "Open as tab" and close; the
  live page below.
- **Now:**
  - Thick card, radius 20, page inset 8 at radius 14 (not concentric: 20 − 8 = 12).
  - Header buttons are `SheetButton`s (G2).
  - .95 / 220 ms from the centre.
- **Compose from:**
  - Placement and modality: the command bar (scrim, `wb-pop-in`), with the origin at the clicked
    link.
  - Header buttons: the touch **Buttons** specimen maps one to one
    (`touch.tsx:451`: `pri` "Open in Space", `glass` "Split view", `ghost` "Cancel"), so
    "Open as tab" is `pri`, "Open in split" is `glass` with the film rim, and close is a `.wb-ib`.
  - The page tile is concentric (radius 12).
  - "Open in split" should animate as the split opens: the peek's page grows into the new pane
    from where it is (README Split view: "A new page opens from no width beside the page it
    joins"). Overlap, not a cut.
- **Anatomy:** a command-bar modal body holding a split pane. The design names the feature
  "Open links in a peek" (`touch.tsx:554`).

### §20 Error, certificate and crash panel (#61)

- **Where:** `W/components/error-panel.slint:15-118`. Gallery `error-page` and `cert-error-page`.
- **Shows/states:**
  - load failed (globe glyph, Reload);
  - certificate refused (coral lock, Go back, Proceed only where allowed);
  - page crashed (Reload).
- **Now:**
  - Covers the content with thick glass at **radius 0**, fading in over 300 ms.
  - A column 440 wide (*inv.*) at a third of the height.
  - 40 px glyph, title 24/600 in the UI face, summary 14.5 mist, detail 12 haze.
  - Left-aligned `SheetButton`s (G2).
- **Compose from:**
  - Ground: the design's missing-page state, `.wb-site-missing` (`browser.css:122`,
    `webview.tsx:38`): the web ground `--wb-web` with centred mist text. The panel replaces the
    page, so it should sit on `Palette.web` inside the web view's rounded clip, not on
    full-bleed glass.
  - Column anatomy: the start page front (`start.css:4-10`, `start.tsx:36-44`): centred column,
    gap 24, the heading in **Fredoka display** `clamp(30px, 4vw, 44px)`, children entering
    `wb-in` at 40 / 80 ms delays.
  - Buttons: `pri` and `ghost` touch Buttons, centred.
  - The certificate's Proceed is a **HoldButton** (G3; the design's "destructive means hold").
  - Detail text, such as an error code, is in mono (G5).
- **Anatomy:** the start-page column on the web ground.

### §21 Picture-in-picture stand-in card (#62)

- **Where:** `W/components/pip-card.slint:10-66`.
- **Shows/states:** "Playing in picture-in-picture", the tab title, and Back to tab.
- **Now:** the error panel's column on `Palette.web`, which is right. It inherits §20's invented
  type and the G2 button.
- **Compose from:** §20's column (Fredoka heading, `wb-in` stagger, a `pri` button). The PiP glyph
  could wobble once on arrival.
- **Anatomy:** the start-page column.

### §22 Touchpad history swipe chip (#63)

- **Where:** `W/components/swipe-chip.slint:10-37`.
- **Shows/states:** the back or forward arrow pulled in from the edge with the fingers, mint once
  a release would navigate.
- **Now:**
  - A round `Chip` (`.wb-thin`) of 42 px (*inv.* size).
  - The opacity ramps with progress; the tint turns mint over 150 ms.
- **Compose from:**
  - The start page's site icon: `.wb-start-site-icon` (`start.css:23-26`: 52 px round, thin
    glass with the film rim, `scale(1.1, .88)` on press).
  - The pull is the scroll ends' rubber band (k 34, `scroller.tsx:12-31`), already used.
  - "Ready" is shown as touch: the film rim lights (`wubble.css:41-43`), instead of a colour
    change.
  - On commit: the arrow wobbles (`wubble.css:62`) and the chip squashes (squash token) as it
    leaves with `wb-out`.
- **Anatomy:** the start page's round site tile.

### §23 Middle-click autoscroll cursor (#64)

- **Where:** `W/components/autoscroll-cursor.slint:11-81`.
- **Shows/states:**
  - a ring at the origin;
  - a dot while still;
  - an arrow stretched toward the scroll while moving.
- **Now:**
  - Film-gradient ring, 22 / 2 px, over `deep` .35 (*inv.*), with an ink .8 centre.
  - The dot is 9 px ink with a film rim and an abyss .6 shadow (*inv.*).
  - The arrow stretches and squashes on the design's stretch spring, which is right.
- **Compose from:**
  - The dot and the arrow body are the slider knob (`touch.css:53`: ink, shadow
    0 6 14 / .5).
  - The ring is the film rim (`wubble.css:35-44`). The swirl is fine here, since the cursor is
    "held".
  - The stretch is the stretch spring with cross axis 1/√s (README Motion tokens).
- **Anatomy:** the slider knob with the film rim. The cursor itself is a new kind of thing; see
  "new primitives".

### §24 Screenshot region picker (#65)

- **Where:** `W/components/shot-overlay.slint:13-156`.
- **Shows/states:**
  - the page dimmed at rest;
  - while dragging: a clear cut-out, a 1 px ink rim and the size label;
  - a toolbar (hint, Visible page, Full page, Cancel), hidden while dragging.
- **Now:**
  - Dim: abyss at **.5** (`Shell.shot-dim` *inv.*).
  - Rim: 1 px ink. The size label is 12 in the UI face.
  - Toolbar: UR thick strip, radius 14, with `SheetButton`s.
- **Compose from:**
  - The dim is the design's scrim (`.wb-scrim` `rgb(5 6 9 / .32)` with `wb-fade`,
    `browser.css:163-165`).
  - The rim is a held line: `.wb-split-line` / the held seam stroke (sky 1.6, `browser.css:114,141`),
    or the film rim, since the region is held.
  - The size label is the slider's value bubble (`.tp-sl-bub b`, `touch.css:55-59`: ink pill with
    abyss 12.5/600, springing in) with the number in tabular mono (G5).
  - The toolbar is the touch **Dock** (`touch.css:92-98`: regular glass, padding 6, radius 18,
    44 px buttons) with its gliding tooltip. Or touch **Buttons** `ghost` / `glass` / `pri`:
    Cancel, Visible, Full.
- **Anatomy:** scrim plus held seam line, with a Dock. The marquee itself is new; see "new
  primitives".

### §25 Reader controls and reader page (#66, #67)

- **Where:**
  - `W/components/reader-controls.slint:15-95`;
  - stylesheet `crates/wubble/assets/reader.js:12-36`;
  - colours `crates/wubble-core/src/reader.rs:202-208`.
- **Shows/states:**
  - text size with minus and plus;
  - column (Narrow, Medium, Wide);
  - font (Serif, Sans);
  - theme (Dark, Light, Sepia);
  - close.
- **Now:**
  - A thick UR strip (radius 14) with three Segmented controls and `IconButton`s.
  - The size is a bare number in the UI face.
  - In the page, the site and byline use `system-ui`, rules are 1 px, and images and `pre` have
    radius 12. Light and sepia colours are reader-only (*inv.*, which is acceptable for content).
- **Compose from:**
  - **Strip:** the touch **Dock** (regular glass, padding 6, radius 18, gaussian swell, one
    gliding tooltip). Or a touch Menu grown from the pill's book glyph (§1), holding settings
    rows.
  - **Theme:** settings **Swatches** (`settings.tsx:267-298`, `settings.css:87-92`: 34 px
    targets, 20 px dots, the ink ring travels). Dark, Light and Sepia are colours, not words.
  - **Size:** an **ElasticSlider** (as Page zoom), or minus/plus `.wb-ib` with the value in mono
    Swapping (G5).
  - **Column and font:** `.st-seg`.
  - **Page:** the site and byline in Geist label type (`tokens.ts:68` label 12/500), and the
    heading in Fredoka display. The reading font stays the user's choice.
- **Anatomy:** touch Dock, or a Menu with settings rows.

### §26 Link hints and zap outline (#68, #69, in-page)

- **Where:** `crates/wubble/assets/hints.js:63-90` (colours from `crates/wubble/src/host/builtins.rs:925-933`),
  `crates/wubble/assets/kit.js:254-256`.
- **Shows/states:**
  - a hint label per link; the typed prefix dimmed to .45;
  - the zap picker outlining the hovered element.
- **Now:**
  - Hints: amber fill, web-colour ink, mono 600 11 px uppercase, radius 5, padding 1/4,
    shadow 0 1 4 / .35 (*inv.*).
  - Zap: a 2 px solid `#7cc4ff` outline. This is *inv.*: it is not the sky token `#7CC8FF`.
- **Compose from:**
  - Hints: the `.tp-tip` anatomy at small size (`touch.css:103`: ink fill, abyss text, 600) with
    `Radius.xs` (6). The typed part goes to haze, not alpha. Amber is a value colour (G7), so drop
    it. Optionally `.tp-tip.show` entrance (fade, rise 4, .95).
  - Zap: the focus ring (`wubble.css:13`: 2 px sky, offset 2), with the token's exact value.
- **Anatomy:** tooltip (hints); focus ring (zap).

### §27 Video strip, scrubber, speed menu, collapsed tab (#70)

- **Where:** `W/components/video-controls.slint:22-244`, at the video's top-right corner.
- **Shows/states:**
  - strip: play/pause; progress with SponsorBlock segments; rate; loop (on or off);
    picture-in-picture; save frame; mute; save video; collapse; close (right-click hides it for
    the site);
  - speed menu;
  - collapsed into a 22 px tab at the pane's right edge, which reopens on hover.
- **Now:**
  - UR thick strip, radius 14, at most 380 wide (*inv.*), sliding 14 px.
  - Progress: a 4 px `Shell.track` rail, an ink 75 % fill, a 10 px ink knob, segment marks at .85
    (*inv.*).
  - Loop-on is the hover fill at alpha .18 (*inv.*).
  - Speed menu: `TextButton`s with a "✓ " text prefix.
  - The collapsed tab is a 22 px thick-glass tile holding a chevron.
- **Compose from:**
  - **Strip:** the touch **Dock** (`touch.tsx:360-436`, `touch.css:89-108`): regular glass,
    padding 6, radius 18, 44 px buttons, neighbours swelling, one gliding tooltip, wobble on hit.
    This is the design's toolbar.
  - **Toggled buttons** (loop, mute): `.wb-ib-on` (`browser.css:38`).
  - **Scrubber:** the **ElasticSlider** (`touch.tsx:43-121`, `touch.css:40-59`): rail white 12 %,
    accent fill, a knob that stretches with speed, a value bubble showing the time in mono while
    held.
  - **Segment marks:** a new variant; see "new primitives".
  - **Speed menu:** touch **Menu** with `.st-mcheck` (the check glyph in the accent), grown from
    the rate button. The rate label Swaps (`dir="x"`).
  - **Collapse:** the hidden-sidebar **peek** behaviour (`browser.css:20-21,148`; peek 60 / 260 /
    340 ms from `chrome.tsx`). The tab is the seam's grab pill (`.wb-pill`, `browser.css:143-146`:
    6×40, mist .8 on hover, accent while held), not a glass tile.
- **Anatomy:** touch Dock plus ElasticSlider; the collapse is the sidebar peek.

### §28 Picture-in-picture controls and window (#71, #84)

- **Where:** `W/components/pip-controls.slint:17-87`; the window at `W/app.slint:55-66` (always on
  top, no chrome, minimum 240×135).
- **Shows/states:** on hover, a strip of back 10 s, play/pause, forward 10 s, back to tab and
  close. Dragging moves the window; a double-click returns.
- **Now:** a UR thick strip (radius 14) at the foot, sliding 14 px in and out with Overlap timings.
- **Compose from:**
  - Strip: the touch **Dock**, as §27.
  - The window's corners and resize are the design window (`desktop.tsx:22`, the window spring
    k 380, c 30, `.wb-edge` strips).
  - Closing could take the design's minimise-to-bubble (`.wb-restore`, `browser.css:159`;
    README "the bubble is … the desktop's minimised window"). That is the owner's call.
- **Anatomy:** touch Dock in a design window.

### §29 Boost editor panel (#72)

- **Where:** `W/components/boost-panel.slint:60-234`, docked at the page's right edge.
- **Shows/states:**
  - name, sites (with an error), CSS (code) with Zap, script (code);
  - run time (At start, When ready);
  - tint swatches and strength;
  - font; enabled toggle; Delete, Done, close.
- **Now:**
  - A floating UR thick panel, fixed at 340 wide (*inv.*), radius 14.
  - Code fields: `Shell.track` boxes, 150 high (*inv.*), with UI-face text at 12.
  - Tint dots: 22 px with a 1 px or 2 px ink ring (*inv.*), and an abyss swatch.
  - The error is in **blush** (G7).
  - Delete is a one-click `TextButton` (G3).
- **Compose from:**
  - **Panel:** the **Source panel** and the Room **Inspector**, which are the design's docked
    right-edge panels (`browser.css:182-203`; `room.tsx:150-296`, `room.css:12-53`):
    - on the seam string (`SourceSeam`, 320–720, closing under 200), the edge bowing while held;
    - the head is `.wb-source-head` (46 high, the title, the `.rm-tool` close).
  - **Code fields:** `.wb-code` (`browser.css:198-203`): mono 12 / 19 line-height, `#C9CEDA` text,
    line numbers `.wb-code-n` and token colours `.wb-c/.wb-s/.wb-n/.wb-k/.wb-t`. Editing is new;
    see "new primitives".
  - **Rows** (name, sites, run time, font, enabled): settings `.st-row`s in an `.st-card`, with a
    whole-row toggle.
  - **Tint:** settings **Swatches** (the travelling ring) plus an **ElasticSlider** for strength.
  - **Zap:** touch Buttons `glass` with the film rim; while zapping it is held: `.wb-on` and the
    border beam lapping it ("while it's thinking").
  - **Done** is `pri`; **Delete** is a **HoldButton**.
  - The error goes in coral `.st-row-note`.
- **Anatomy:** Source panel / Inspector holding a settings card.

### §30 Engine popup layer (#73)

- **Where:** `W/components/popup-layer.slint:9-46`.
- **Shows/states:** the engine's own popup frame (a `<select>` list, a date picker) at the
  engine's rectangle.
- **Now:** a bare image: no radius, no shadow, no entrance.
- **Compose from:** wrap the frame in the touch **Menu** body's motion and shape: clip radius 16,
  thick-tier shadow, `wb-pop-in` with the origin at the `<select>`, and `wb-pop-out`. The content
  stays the engine's (CEF paints it).
- **Anatomy:** touch Menu body. Only the frame is drawn by wubble.

### §31 Page accessibility layer (#74)

- **Where:** `W/components/page-ax.slint:12-78`.
- **Note:** it is never drawn: invisible elements for screen readers. There is no look to derive,
  and it needs no change for fidelity. Listed for completeness.

### §32 List pages: PageFrame, History, Archive, Bookmarks, Downloads, Passwords, Task manager (#75-81)

- **Where:**
  - `W/components/settings.slint:905-957` (PageFrame);
  - `W/components/pages.slint:102` (History, also Archive at `W/app.slint:977`);
  - `:225` (Bookmarks), `:315` (Downloads), `:348` (Passwords), `:540` (Tasks).
  - Gallery scenes: `history`, `bookmarks`, `downloads`, `passwords-locked`, `passwords`,
    `archive`, `task-manager`.
- **Shows/states:**
  - a title and a capsule search beside it (none for downloads and tasks);
  - an action line (Clear browsing data, New folder, Clear finished, the total);
  - rows of icon, title, address, and per-row actions;
  - day headers (history), a tree (bookmarks), a vault card (passwords: locked, unlocked,
    unavailable, notice), columns (tasks: state, memory, CPU, Never sleep, Discard, End);
  - empty notes; Show more; an add-password sheet.
- **Now:**
  - A 720 column (`Shell.settings-width` *inv.*). The title and search share one line.
  - Rows are **bare** on the page at 44 high (`Shell.page-row` *inv.*), radius 10, hover white 7 %.
  - Every row action is always visible: the bin `IconButton`, Rename/Move/Show/Copy
    `TextButton`s.
  - Day headers are `SectionHeader`s. Task numbers are 12 haze in the UI face.
  - History clear is a confirm sheet with a Segmented range (G3).
- **Compose from:** the settings page (`settings.tsx:411-600`, `settings.css:6-130`):
  - **Header:** `.st-pin h1` (Fredoka 40) over the `.st-search` capsule, with `.st-count` under
    it: "N results", Swapping as the query changes (`settings.css:9-21`). At the settings page's
    widths the pinned left column (`.st-grid` 232 | 1fr) can hold the search and filters
    (history ranges, bookmark folders), as the settings nav does.
  - **Rows:** inside `.st-card`s (`settings.css:40-47`: radius 20, padding 6; rows min 60, radius
    14, white 4 % on hover, title 14/500 ink, note 13 mist). The address goes in the note slot in
    mono (`.wb-address-text` treatment, G5).
  - **Group headings** (history days, bookmark folders when searching): `.st-group` (13/500 mist,
    `settings.css:39`).
  - **Search:** rows fold away with **Fold** and matches are marked in the accent
    (`settings.tsx:151-183`, `.st-page mark`). Empty states are `.st-empty` (`settings.css:117-118`:
    15 mist with a button under it).
  - **Row actions:** hover-revealed like `.wb-tab-x` (`browser.css:75-78`: 22 tile, radius 7,
    opacity 0 at .8 scale → 1 on row hover or focus).
    - Delete is in coral and is a hold where it cannot be undone (G3).
    - Rename and Move go in a touch **Menu** from a row trigger.
    - Copy (passwords) is a **ConfirmButton** ("Copied" in mint).
    - Show/Hide crossfades the secret with **Swap**.
  - **Clear browsing data:** the settings Privacy group's checkboxes and **HoldButton**
    (`settings.tsx:102-106`, the design's own clear-data UI), not a sheet.
  - **Bookmarks tree:** folders expand with Fold, and the chevron rotates (`.tp-chev`).
  - **Passwords vault card:** an `.st-card` with an `.st-lead` lock tile; Unlock is `pri`.
  - **Task manager:**
    - memory, CPU and state are numbers, so tabular mono (G5);
    - "Never sleep" is a boolean, so a `.tp-tg-sm` toggle;
    - Discard is a ghost button; End is coral and a hold;
    - the columns are a new layout; see "new primitives".
  - **Add-password sheet:** §13.
- **Anatomy:** the settings page with settings cards and rows.

### §33 Little and Popup windows (#82, #83)

- **Where:**
  - `crates/wubble/src/main.rs:700-706` forces the sidebar to Hidden;
  - `crates/wubble/src/menu.rs:735-768` puts the way out ("Open as Tab", "Open in <Space>") only
    in the page's context menu.
- **Shows/states:**
  - Little: one page and a choice of Space.
  - Popup: one page with a read-only address.
- **Now:** the main shell with the sidebar hidden. Without a peek or a visible affordance, the way
  out is invisible; the read-only address shows only in the peek.
- **Compose from:**
  - The hidden-sidebar **peek** (`browser.css:20-21`, `chrome.tsx:104-115,250`) as the only
    chrome.
  - For Little, a small dock at the top or foot: the touch **Space switcher** (`touch.tsx:578-587`,
    `touch.css:111-118`: icons for Personal, Work, Study, the indicator stretching) plus a `pri`
    "Open in <Space>" button. That is exactly the touch Buttons' "Open in Space" (`touch.tsx:451`).
  - Popup: the `.wb-address` capsule, read-only, in mono.
- **Anatomy:** the hidden-sidebar peek, plus touch Space switcher and Buttons. Which of these is
  shown at rest is an owner decision; see "new primitives".

---

## Needs a new primitive (owner approval)

Everything in the sections above can be composed from existing design primitives, except these.
Each one is the smallest addition, kept in the design's language.

1. **Free-standing notice (toast).** The design confirms in place (G4) and has no notice that
   stands alone. The full-screen hint, SponsorBlock "Skipped: Undo" and its one-time question
   have no trigger to confirm on. Proposal: the tooltip anatomy on thick glass, `wb-pop-in`, with
   one `ghost` action (§16).
2. **Marked scrubber.** An ElasticSlider whose rail carries coloured ranges: SponsorBlock segments
   on the video scrubber (§27). The design's rail has a single fill.
3. **Region marquee.** The screenshot drag-rectangle: a dimmed scrim with a clear cut-out and a
   held rim (§24). The scrim, held line and value bubble exist; the cut-out does not.
4. **Custom pointer.** The autoscroll cursor (§23). It is composed from the knob and the film rim,
   but the design draws no pointers.
5. **Editable code area.** The Boost CSS and script fields (§29). The design's `.wb-code` is
   read-only; caret, selection and editing in that style are new.
6. **Numeric entry.** A typed number with bounds: print copies, and any settings Stepper the
   owner wants kept (§6, §15). The design sets numbers only with the ElasticSlider.
7. **Data columns.** The Task manager's aligned value columns (state, memory, CPU, actions) (§32).
   The settings row has one control slot and no table.
8. **Menu separator.** The context menu's divider (§9). The touch Menu has none; group spacing
   may be enough.
9. **Little/Popup window chrome at rest** (§33). Which of peek, dock and address is visible at
   rest when there is no sidebar is a design decision, even though every part exists.

Also to raise with the owner, as a consistency question rather than a primitive: the tokens that
G1 attributes to Ultra Reactivate's `overlays.css` should be retired in favour of the design's
touch Menu and Buttons. The new primitives above should reference the design's tokens only.

## Counts

- **85** wubble surfaces inventoried (table rows #1–#85).
  - **37 direct counterparts:** #1–7, 9–12, 14, 16, 19–23, 25–40 and 43–45. Of these, #43
    CheckBox and #44 SheetButton deviate noticeably, and #40 Field's plain variant is
    primitive-only.
  - **9 primitive-only** (no design surface, but a design primitive to build from): #8, #13, #17,
    #18, #24, #46, #47, #61, #75.
  - **38 with no counterpart:** #15, #41, #42, #48–60, #62–74, #76–84.
  - **1 n/a:** #85, engine-drawn, no wubble chrome.
- **33 sections** (§1–§33). They cover all 47 rows that are primitive-only or have no
  counterpart, plus 4 direct rows with wubble-only states or deviations (#38, #40, #43, #45).
  Related surfaces share a section.
- **9 items** need a new primitive or an owner decision. All other wubble-only surfaces can be
  derived from the design's primitives.
- **Invented values flagged** (*inv.*): 30+ `Shell.*` tokens. These are the UR-sourced menu,
  sheet, toast, find and preview numbers, and the downloads, video, boost, status, shot, drag and
  page-row sizes. Five more are colour choices: amber bookmark, blush error, `#7cc4ff` zap,
  `Palette.shell` button fills, and the abyss .5 dim.
