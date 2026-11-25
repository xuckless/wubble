# Shell polish

Shelved by the owner (2026-11-23): "shell polish can also be shelved".

Legend: `[ ]` open. Picked up only when the owner says so; how items move: [todo/README.md](../README.md).

The review findings these came from: [review.md](review.md).

## Build-out features (2026-11-22)

## Spaces popup: rename and delete on a row, aligned to the Space tile's left edge (`spaces-popup-row-actions`)

Key: `spaces-popup-row-actions` · engine patch: no · state: not written.

TODO line 113. The Spaces popup (`wubble_core::spacemenu` rows, kit `kit/chrome/spaces-menu.slint`, `spaces-button.slint`) aligns differently from the design (the design puts it at the Space tile's left edge) and offers no rename or delete on a user Space's row (only an agent Space's dustbin). (1) Position: anchor the popup to the Space tile's left edge as in the design's web source (look up `browser/` in the design repo, pull first), with the kit popover's anchor math. (2) Row actions: on hover or keyboard focus, a user Space's row shows Rename (in place: the row's label becomes a kit field; Enter commits, Esc cancels) and Delete (confirm when the Space has tabs, as Chrome asks before closing a window with many tabs). Add `Row` actions in spacemenu.rs and the existing Space commands in `wubble_core::model` (`Command::RenameSpace`/`DeleteSpace` or equivalents). Derived kit pieces go on docs/fidelity/derived-review.md. Unit tests in spacemenu.rs.

## A Space button opened by keyboard gives the Spaces menu the keyboard focus (`spaces-button-menu-focus`)

Key: `spaces-button-menu-focus` · engine patch: no · state: not written.

TODO line 137. Opening the Spaces popup from the Space button with the keyboard (Enter or Space on `kit/chrome/spaces-button.slint`) leaves the focus on the button. Chrome's menus take the keyboard on open, with the first or current item highlighted. When the popup opens by keyboard, move focus into the menu (`kit/chrome/spaces-menu.slint`), with the current Space's row focused. Up/Down move between rows, Enter opens, Esc closes and returns focus to the button. Record how it was opened (keyboard or pointer) as data in the overlay state (`wubble_core::overlay` or `spacemenu.rs`) so a pointer open does not show a focus ring. Kit rules apply.

## In-place rename for a new Space in the sidebar (`space-inline-rename`)

Key: `space-inline-rename` · engine patch: no · state: not written.

TODO line 121, first half (Chrome names a new tab group in place). Creating a Space gives it a default name that is changed only in Settings. After "New Space", put the sidebar's Space header (`kit/chrome/sidebar.slint`, the Space title part) into an editing state: the title becomes a kit field (`kit/controls/field.slint`) with the text selected. Enter or blur commits through the model's rename command, Esc keeps the default. A double-click on the header renames too. Keep the editing state as data in core (for example `Option<SpaceId>` in the sidebar model with a pure transition), not a Slint boolean. The design has no such state, so this is a derived piece (docs/fidelity/wubble-only.md rules; add it to docs/fidelity/derived-review.md for the owner).

## Command bar: the shrinking panel no longer takes clicks during its exit (`command-bar-exit-clicks`)

Key: `command-bar-exit-clicks` · engine patch: no · state: not written.

TODO line 120. While the command bar's panel shrinks away (176 ms), it still takes pointer input, so a click on the page under it is lost. Check the design's exit in the web source (the design repo's `browser/` command bar CSS: whether it sets `pointer-events:none` on the leaving state). Then make the kit part (`crates/wubble-ui/ui/kit/chrome/command-bar.slint` and its overlay layer) stop taking input as soon as its `open` goes false. Gate the TouchArea and FocusScope with `enabled: root.open` (or the motion's target state, not its current value), and let the click fall through to the page view. Also check the host's layer routing (`crates/wubble/src/host/layers.rs`) so the page gets that click. Kit-lock rules apply; no literals in shell files.

## Shortcuts: both chords editable, rebinding keeps the second chord, Linux alternates where GNOME takes the chord (`keybinding-second-chords`)

Key: `keybinding-second-chords` · engine patch: no · state: not written.

TODO lines 121 (second half) and 131. `wubble_core::keys` (crates/wubble-core/src/keys.rs) holds the default table and user overrides (`keys.overrides`). Rebinding the sidebar shortcut also unbinds its second chord, Mod+Alt+S (once compact mode's), and the Settings shortcut editor shows only the first chord. Chrome lets each command keep several accelerators. (1) Make an override per chord slot: `overrides` maps an action to a list of chords, so editing slot 0 leaves slot 1. Settings' shortcut rows show every chord, each editable and removable, with "Add shortcut" (kit parts in `kit/settings`, host `crates/wubble/src/host/settings.rs`). Migrate stored overrides (old single-chord form → slot 0). (2) Linux: chords the desktop takes (GNOME's Super+…, Ctrl+Alt+Arrow workspace switching, Alt+F7/F8 and others) get a default alternate chord on Linux only. Keep the alternates as data in the default table with an `Os` column, so the binding still exists. Unit tests in keys.rs.

## Start page tiles: the kit tile look and its hover spring (`start-tiles-kit`)

Key: `start-tiles-kit` · engine patch: no · state: not written.

TODO line 124. The start page's tiles (`crates/wubble-ui/ui/kit/pages/start.slint`, host `crates/wubble/src/host/start.rs`, core `wubble_core::start`) are stand-ins. Port the design's tile from the web source (the design repo's `pages/` start page and its CSS: size, radius, glass, favicon tile, label, and the hover spring's stiffness and damping). Its values come through `node tools/fidelity/extract.mjs` into `kit/gen/*.slint` and `wubble_core::kit_gen` (never hand-edited). Drive the hover spring through the existing controls registry (`wubble_core::controls`, the `Controls` slots pattern used by toggle and slider) if it is a physics spring, or through `kit/motion/*` if it is a CSS transition. The kit file starts with its `// design:` header. Lower the kit lock baseline if stand-in literals go away (`KIT_LOCK_WRITE=1` is a test run, so leave the baseline file for the compile step).

## Settings: Reset per section, rows folding while searching, searchable Filter lists (`settings-reset-fold-search`)

Key: `settings-reset-fold-search` · engine patch: no · state: not written.

TODO lines 125 and 136. Against the web design (`pages/settings.tsx`, `settings.css` in the design repo): (1) Reset per section. The kit has `ResetButton` (`kit/settings/reset.slint`), but each section's header has no Reset that restores that section's settings to their defaults. Add it to the section header (`kit/settings/section.slint`), shown while any row in the section differs from its default (`changed` on rows in `wubble_core::pages::settings_page`), and a host action that writes the defaults for that section's keys (`crates/wubble/src/host/settings.rs`). (2) While searching, rows that do not match fold away with the design's fold motion (`kit/motion/fold.slint`), and sections with no match fold too, instead of being dropped from the model at once. (3) The search no longer finds "Your own filter lists": the setting moved to `Section::Hidden`, and the Filter lists section is not searchable. Index the Filter lists section's own rows (and the extensions section's) in the settings search (`wubble_core::pages` search filter) under their visible labels. Unit tests in core.

## Narrow Settings: slider and select rows wrap to full width (`settings-narrow-wrap`)

Key: `settings-narrow-wrap` · engine patch: no · state: not written.

TODO line 132. At a content width of 759 px or less, the design's narrow block (`settings.css`:120-130 in the design repo's pages) wraps a row's control below its label at full width. wubble's slider and select rows do not (repro `WUBBLE_SHOT_SIZE=960x640 --smoke settings --shot`, which the owner runs, not you). In the kit settings row (`crates/wubble-ui/ui/kit/settings/*.slint` and the row parts that hold sliders and selects), add the narrow layout. Below the design's breakpoint (from `kit/gen/css.slint`, generated by `tools/fidelity/extract.mjs`; add it there if missing), put the control on its own line at full width with the design's gap. Keep it in kit files only, with the `// design:` header updated to the lines it now covers.

## Settings Select fits its longest label (no clipped value) (`settings-select-fit`)

Key: `settings-select-fit` · engine patch: no · state: not written.

TODO line 139. The Settings Select (`crates/wubble-ui/ui/kit/controls/select.slint`) clips a long value at its fixed width: "Ask on first sea" for the search engine with none chosen. Follow the design's select (`pages/settings.css` / select source in the design repo). Either size the closed control to its widest option label (measure every option's text width in Slint through a hidden `Text` per option, or pass the max width from the model), capped at the row's available width, or elide with an ellipsis where the design elides. Prefer the design's behaviour. Keep it in the kit. The select's list (drawn by Slint, no blur) is a separate owner item; leave it.

## Gallery: shoot a Settings section only after the page's jump has landed (`gallery-settle-spring`)

Key: `gallery-settle-spring` · engine patch: no · state: not written.

TODO line 45. In `--smoke gallery`, a Settings section picked far from where the page sits is shot mid-jump: the settings page scrolls with a spring, and `settings-extensions` on the patched engine shows Filter lists passing. Fix it in the gallery driver (`crates/wubble/src/gallery_smoke.rs`, `crates/wubble/src/host/gallery.rs`). Either wait for the page's scroll spring (the settings page's scroller motion in `wubble_core::scroller` / `motion`) to report it is at rest before the shot, or jump with no animation in the gallery (a `jump_instant` path on the settings nav that sets the scroller's position and velocity directly). Prefer waiting on the spring's settled state, with a bounded wait (for example 2 s) and a log line if it does not settle. No timing assertions.

## Gallery: scenes for the hover preview card and the sidebar's empty-area menu; a stable page-menu position (`gallery-hover-menu-scenes`)

Key: `gallery-hover-menu-scenes` · engine patch: no · state: not written.

TODO line 126. `--smoke gallery` (crates/wubble/src/gallery_smoke.rs, `crates/wubble/src/host/gallery.rs`, scenes listed in docs/smokes.md) has no scene for the tab hover preview card (`components/preview-card.slint`, host `host/preview.rs`, core `hovercard.rs`) or for the sidebar's empty-area context menu. The page menu's position also varies from run to run. Add two scenes that set up the state directly (a hovered tab with a snapshot, and the empty-area menu opened at a fixed point). Pin the page menu's anchor in its scene to a fixed point rather than the last pointer position. Follow the scene table's existing pattern and add the scene names to docs/smokes.md's gallery list. Do not run the gallery (owner rule).

## Shell fields: Shift+Insert paste, Ctrl+Insert copy, Shift+Delete cut (`shell-insert-key-clipboard`)

Key: `shell-insert-key-clipboard` · engine patch: no · state: not written.

TODO line 128 (first half; the tests half is out under the owner's no-tests rule). Slint's TextInput has no Shift+Insert, Ctrl+Insert or Shift+Delete shortcuts. Chrome supports them in its omnibox and fields on Linux and Windows. In the key router (`crates/wubble/src/input.rs` `key_route`, where keys that go to a shell field are decided), map Shift+Insert to the paste action (through the host's clipboard path, `crates/wubble/src/clipboard.rs`, which on Wayland goes through the window's worker), Ctrl+Insert to copy and Shift+Delete to cut, for the focused shell field only. Pages get these keys as before. Put the mapping in the keybinding data (`wubble_core::keys` default table, Linux and Windows only, as Chrome does) rather than special-casing it in the router.

## Fix the hue under the command bar in dark overlay mode (`overlay-dark-hue-fix`)

Key: `overlay-dark-hue-fix` · engine patch: no · state: not written.

TODO line 129. `--smoke overlay=dark` fails a hue check under the command bar. It failed before the glass fix too, so the glass fix did not cause it. Look at what the check samples (crates/wubble/src/overlay_smoke.rs), and at what the scene draws under the command bar in dark mode: the glass tint and the backdrop from `crates/wubble/src/backdrop.rs`, `wubble-gfx` `scene/render.rs`, and the dark tokens in `kit/gen/tokens.slint`. Compare with the design's dark command bar (the design repo's `browser/` CSS, the glass and command-bar rules). Find whether the tint takes the light palette's hue or the page's theme colour (`wubble_core::pagetheme`) where the design uses the shell's dark hue. Fix the source of the colour, not the check. Do not run the smoke; reason from code and the design.

## FocusKeys with native-button false fires only once on a held Enter or Space (`focuskeys-repeat`)

Key: `focuskeys-repeat` · engine patch: no · state: not written.

TODO line 134. `FocusKeys` (crates/wubble-ui/ui/components/focus-ring.slint ~33) with `native-button: false` fires its activation again on key repeat (a held Enter or Space). A native button activates once per press: Space on release, Enter on press without repeat. Slint's `KeyEvent` has a `repeat` field. In `key-pressed`, ignore events with `event.repeat` for Enter and Space. Activate Space on `key-released`, and Enter on its first press, as the platform buttons do. This is a component file. If it moves into the kit, it follows kit rules. Check that every user of `FocusKeys` keeps working (list rows, settings rows, menus).

## Popover scroll thumbs no longer cover the rows' right edge (`popover-thumb-touch-area`)

Key: `popover-thumb-touch-area` · engine patch: no · state: not written.

TODO line 138. In popovers (downloads, extensions, menus), the scroll thumb's touch area (`crates/wubble-ui/ui/kit/controls/scroller.slint`) covers the rows' right edge, so clicks there grab the thumb instead of the row. Follow the design's scrollbar (the design repo's CSS for the popover scroll thumb: width, inset, hit area). Limit the thumb's TouchArea to the thumb's drawn rect plus the design's padding. Show the track's hit area only while the thumb is visible (hovered or scrolling), and let row clicks through elsewhere. Kit-only change, with the `// design:` header updated.

## Built-in list pages: keyboard open, arrow keys, selection, delete selection, undo, copy address (`list-pages-keyboard-select`)

Key: `list-pages-keyboard-select` · engine patch: no · state: not written.

TODO lines 92 and 133 (Chrome's History and Bookmarks managers). History, Bookmarks, Downloads and Archive rows (`kit/pages/list-row.slint`, `list-table.slint`, models in `wubble_core::pages`, host in `crates/wubble/src/host/pages.rs`) cannot be opened from the keyboard: only a row's Delete is a Tab stop. Up/Down do nothing and a focused row is not scrolled into view. There is no selection, delete selection, undo, copy address or open in a new tab from the keyboard, and a bookmark folder is deleted with its contents by one press. Build: (1) The row itself is focusable (a kit `FocusKeys` on the row, derived, on docs/fidelity/derived-review.md). Enter opens it, Mod+Enter or Shift+Enter opens in a new tab or window, Up/Down/Home/End move focus, and the scroller follows the focused row (the kit scroller's `scroll-to`). (2) Selection as data in core: a `ListSelection` (anchor, set) with a pure transition for click, Shift+click, Mod+click, Shift+arrows and Mod+A. Delete removes the selection. (3) Undo: a deleted batch (or a folder with its contents) shows a toast with Undo (Mod+Z on the page), restored through wubble-store. (4) A row's context menu: Open in new tab, Copy address, Delete. Unit tests in core.

## Empty the kit lock baseline (`kit-lock-baseline-empty`)

Key: `kit-lock-baseline-empty` · engine patch: no · state: not written.

The kit lock (crates/wubble-ui/tests/kit_lock.rs) still records about 100 violations in crates/wubble-ui/tests/kit_lock_baseline.txt. 60 are literals in `kit/derived.slint`; the rest are in shell files that the rules say must hold no visuals: components/raised.slint (9), components/folder-row.slint (9), components/border-beam.slint (5), app.slint (5), components/sidebar.slint (3), components/focus-ring.slint (3), kit/chrome/tab-row.slint (2) and kit/chrome/split-view.slint (2). TODO Design fidelity asks to empty it and to alias the value-equal `Shell` literals.

To build: (1) For each `derived.slint` literal, find the generated token with the same value in `ui/kit/gen/*.slint` and use it. Where none exists, add a named alias in one place. If it comes from the design, add it to the extractor (`tools/fidelity/extract.mjs`, then rerun it); never edit the gen files by hand. A truly wubble-only value becomes a single named constant recorded in docs/fidelity/derived-review.md. (2) Move the visuals in components/raised.slint, folder-row.slint, border-beam.slint and focus-ring.slint into kit components (a `// derived:` header naming the kit parts they compose) and make the shell files only lay them out. (3) Do the same for app.slint's five, and replace the `Shell` literals that equal a token. (4) Lower the baseline with `KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock` (the compile or test step) until it is empty, and leave the rule in force. Gallery shots before and after must match by eye. All platforms.

