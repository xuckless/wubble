# Build-out batch 16

15 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `list-pages-keyboard-select`: Built-in list pages: keyboard open, arrow keys, selection, delete selection, undo, copy address

Area: todo-a. Needs an engine patch: no.

TODO lines 92 and 133 (Chrome's History and Bookmarks managers). History, Bookmarks, Downloads and Archive rows (`kit/pages/list-row.slint`, `list-table.slint`, models in `wubble_core::pages`, host in `crates/wubble/src/host/pages.rs`) cannot be opened from the keyboard: only a row's Delete is a Tab stop. Up/Down do nothing and a focused row is not scrolled into view. There is no selection, delete selection, undo, copy address or open in a new tab from the keyboard, and a bookmark folder is deleted with its contents by one press. Build: (1) The row itself is focusable (a kit `FocusKeys` on the row, derived, on docs/fidelity/derived-review.md). Enter opens it, Mod+Enter or Shift+Enter opens in a new tab or window, Up/Down/Home/End move focus, and the scroller follows the focused row (the kit scroller's `scroll-to`). (2) Selection as data in core: a `ListSelection` (anchor, set) with a pure transition for click, Shift+click, Mod+click, Shift+arrows and Mod+A. Delete removes the selection. (3) Undo: a deleted batch (or a folder with its contents) shows a toast with Undo (Mod+Z on the page), restored through wubble-store. (4) A row's context menu: Open in new tab, Copy address, Delete. Unit tests in core.

## `video-save-button-states`: Video strip save button: wording and in-progress state

Area: media-downloads. Needs an engine patch: no.

TODO.md review findings: "Video strip button reads 'Save video'; a second click mid-download re-enters the downloader". Chrome has no such control. The rules for this one come from wubble (decisions.md "Save control"). The strip's save state is wubble_core::video::Save (host/videos.rs video_save_now, video_save_state, video_save_redraw), and the click goes to `video_save(x, y)`, which starts the downloader again even while that video's job runs.

What to build: (1) Core (wubble-core/src/video.rs): make `Save` carry the job's state for this tab's video: Idle, Saving{fraction}, Saved, Failed. Derive it from the downloads entry the media job owns, which has one owner, the downloads list. The click maps to a pure effect: Idle → start the downloader; Saving → open the downloads popover at that row, never a second job; Saved → show in folder or open the menu; Failed → retry. Unit tests over the states. (2) Label: the button's accessible label and tooltip read from that state ("Save video", "Saving video, 40%", "Saved, show in folder", "Save failed, retry"), and the glyph shows progress with the kit's progress part. Settle the wording with the owner's decision on wording if one exists; otherwise use these and flag them. (3) video-controls.slint binds the new fields only (no new drawing outside kit parts). All platforms.

## `agent-pages-icons-log-clear`: Agent sessions: no icon-table entries for their pages; the activity log can be cleared

Area: todo-a. Needs an engine patch: no.

TODO line 97. Agent session pages are not history, but their origin and icon are filed in the profile's icon table (`crates/wubble-store/src/favicon.rs`, through the host's favicon path `crates/wubble/src/host` and `wubble_core::favicon`). (1) Skip the favicon store write for tabs an agent session drives and for agent Spaces' tabs (the same test as `Model::closes_into_recent`). Keep icons in memory for the session's own rows. (2) Clearing the activity log: the log is a memory ring (`wubble_core::agent::log::ActivityLog`) plus a store tail (`agent_log`, `STORE_MAX`). Add `ActivityLog::clear` (and per session), a store delete, and a "Clear activity" control on the agent activity page (kit button). Also clear it with "Clear browsing data" when its new checkbox "Agent activity" is ticked (`crates/wubble/src/host/data.rs`). Unit tests for the log's clear.

## `spaces-popup-row-actions`: Spaces popup: rename and delete on a row, aligned to the Space tile's left edge

Area: todo-a. Needs an engine patch: no.

TODO line 113. The Spaces popup (`wubble_core::spacemenu` rows, kit `kit/chrome/spaces-menu.slint`, `spaces-button.slint`) aligns differently from the design (the design puts it at the Space tile's left edge) and offers no rename or delete on a user Space's row (only an agent Space's dustbin). (1) Position: anchor the popup to the Space tile's left edge as in the design's web source (look up `browser/` in the design repo, pull first), with the kit popover's anchor math. (2) Row actions: on hover or keyboard focus, a user Space's row shows Rename (in place: the row's label becomes a kit field; Enter commits, Esc cancels) and Delete (confirm when the Space has tabs, as Chrome asks before closing a window with many tabs). Add `Row` actions in spacemenu.rs and the existing Space commands in `wubble_core::model` (`Command::RenameSpace`/`DeleteSpace` or equivalents). Derived kit pieces go on docs/fidelity/derived-review.md. Unit tests in spacemenu.rs.

## `command-bar-exit-clicks`: Command bar: the shrinking panel no longer takes clicks during its exit

Area: todo-a. Needs an engine patch: no.

TODO line 120. While the command bar's panel shrinks away (176 ms), it still takes pointer input, so a click on the page under it is lost. Check the design's exit in the web source (the design repo's `browser/` command bar CSS: whether it sets `pointer-events:none` on the leaving state). Then make the kit part (`crates/wubble-ui/ui/kit/chrome/command-bar.slint` and its overlay layer) stop taking input as soon as its `open` goes false. Gate the TouchArea and FocusScope with `enabled: root.open` (or the motion's target state, not its current value), and let the click fall through to the page view. Also check the host's layer routing (`crates/wubble/src/host/layers.rs`) so the page gets that click. Kit-lock rules apply; no literals in shell files.

## `space-inline-rename`: In-place rename for a new Space in the sidebar

Area: todo-a. Needs an engine patch: no.

TODO line 121, first half (Chrome names a new tab group in place). Creating a Space gives it a default name that is changed only in Settings. After "New Space", put the sidebar's Space header (`kit/chrome/sidebar.slint`, the Space title part) into an editing state: the title becomes a kit field (`kit/controls/field.slint`) with the text selected. Enter or blur commits through the model's rename command, Esc keeps the default. A double-click on the header renames too. Keep the editing state as data in core (for example `Option<SpaceId>` in the sidebar model with a pure transition), not a Slint boolean. The design has no such state, so this is a derived piece (docs/fidelity/wubble-only.md rules; add it to docs/fidelity/derived-review.md for the owner).

## `keybinding-second-chords`: Shortcuts: both chords editable, rebinding keeps the second chord, Linux alternates where GNOME takes the chord

Area: todo-a. Needs an engine patch: no.

TODO lines 121 (second half) and 131. `wubble_core::keys` (crates/wubble-core/src/keys.rs) holds the default table and user overrides (`keys.overrides`). Rebinding the sidebar shortcut also unbinds its second chord, Mod+Alt+S (once compact mode's), and the Settings shortcut editor shows only the first chord. Chrome lets each command keep several accelerators. (1) Make an override per chord slot: `overrides` maps an action to a list of chords, so editing slot 0 leaves slot 1. Settings' shortcut rows show every chord, each editable and removable, with "Add shortcut" (kit parts in `kit/settings`, host `crates/wubble/src/host/settings.rs`). Migrate stored overrides (old single-chord form → slot 0). (2) Linux: chords the desktop takes (GNOME's Super+…, Ctrl+Alt+Arrow workspace switching, Alt+F7/F8 and others) get a default alternate chord on Linux only. Keep the alternates as data in the default table with an `Os` column, so the binding still exists. Unit tests in keys.rs.

## `start-tiles-kit`: Start page tiles: the kit tile look and its hover spring

Area: todo-a. Needs an engine patch: no.

TODO line 124. The start page's tiles (`crates/wubble-ui/ui/kit/pages/start.slint`, host `crates/wubble/src/host/start.rs`, core `wubble_core::start`) are stand-ins. Port the design's tile from the web source (the design repo's `pages/` start page and its CSS: size, radius, glass, favicon tile, label, and the hover spring's stiffness and damping). Its values come through `node tools/fidelity/extract.mjs` into `kit/gen/*.slint` and `wubble_core::kit_gen` (never hand-edited). Drive the hover spring through the existing controls registry (`wubble_core::controls`, the `Controls` slots pattern used by toggle and slider) if it is a physics spring, or through `kit/motion/*` if it is a CSS transition. The kit file starts with its `// design:` header. Lower the kit lock baseline if stand-in literals go away (`KIT_LOCK_WRITE=1` is a test run, so leave the baseline file for the compile step).

## `settings-reset-fold-search`: Settings: Reset per section, rows folding while searching, searchable Filter lists

Area: todo-a. Needs an engine patch: no.

TODO lines 125 and 136. Against the web design (`pages/settings.tsx`, `settings.css` in the design repo): (1) Reset per section. The kit has `ResetButton` (`kit/settings/reset.slint`), but each section's header has no Reset that restores that section's settings to their defaults. Add it to the section header (`kit/settings/section.slint`), shown while any row in the section differs from its default (`changed` on rows in `wubble_core::pages::settings_page`), and a host action that writes the defaults for that section's keys (`crates/wubble/src/host/settings.rs`). (2) While searching, rows that do not match fold away with the design's fold motion (`kit/motion/fold.slint`), and sections with no match fold too, instead of being dropped from the model at once. (3) The search no longer finds "Your own filter lists": the setting moved to `Section::Hidden`, and the Filter lists section is not searchable. Index the Filter lists section's own rows (and the extensions section's) in the settings search (`wubble_core::pages` search filter) under their visible labels. Unit tests in core.

## `gallery-hover-menu-scenes`: Gallery: scenes for the hover preview card and the sidebar's empty-area menu; a stable page-menu position

Area: todo-a. Needs an engine patch: no.

TODO line 126. `--smoke gallery` (crates/wubble/src/gallery_smoke.rs, `crates/wubble/src/host/gallery.rs`, scenes listed in docs/smokes.md) has no scene for the tab hover preview card (`components/preview-card.slint`, host `host/preview.rs`, core `hovercard.rs`) or for the sidebar's empty-area context menu. The page menu's position also varies from run to run. Add two scenes that set up the state directly (a hovered tab with a snapshot, and the empty-area menu opened at a fixed point). Pin the page menu's anchor in its scene to a fixed point rather than the last pointer position. Follow the scene table's existing pattern and add the scene names to docs/smokes.md's gallery list. Do not run the gallery (owner rule).

## `shell-insert-key-clipboard`: Shell fields: Shift+Insert paste, Ctrl+Insert copy, Shift+Delete cut

Area: todo-a. Needs an engine patch: no.

TODO line 128 (first half; the tests half is out under the owner's no-tests rule). Slint's TextInput has no Shift+Insert, Ctrl+Insert or Shift+Delete shortcuts. Chrome supports them in its omnibox and fields on Linux and Windows. In the key router (`crates/wubble/src/input.rs` `key_route`, where keys that go to a shell field are decided), map Shift+Insert to the paste action (through the host's clipboard path, `crates/wubble/src/clipboard.rs`, which on Wayland goes through the window's worker), Ctrl+Insert to copy and Shift+Delete to cut, for the focused shell field only. Pages get these keys as before. Put the mapping in the keybinding data (`wubble_core::keys` default table, Linux and Windows only, as Chrome does) rather than special-casing it in the router.

## `overlay-dark-hue-fix`: Fix the hue under the command bar in dark overlay mode

Area: todo-a. Needs an engine patch: no.

TODO line 129. `--smoke overlay=dark` fails a hue check under the command bar. It failed before the glass fix too, so the glass fix did not cause it. Look at what the check samples (crates/wubble/src/overlay_smoke.rs), and at what the scene draws under the command bar in dark mode: the glass tint and the backdrop from `crates/wubble/src/backdrop.rs`, `wubble-gfx` `scene/render.rs`, and the dark tokens in `kit/gen/tokens.slint`. Compare with the design's dark command bar (the design repo's `browser/` CSS, the glass and command-bar rules). Find whether the tint takes the light palette's hue or the page's theme colour (`wubble_core::pagetheme`) where the design uses the shell's dark hue. Fix the source of the colour, not the check. Do not run the smoke; reason from code and the design.

## `autofill-chip-field-rect`: Autofill key chip at the focused field, pane-aware in split view

Area: todo-a. Needs an engine patch: no.

TODO line 130. Chrome anchors the password-manager key and dropdown to the focused field. wubble's key chip sits over the page's top left and is not pane-aware in split view (crates/wubble/src/host/autofill.rs; the chip in the autofill menu component `components/autofill-menu.slint`). Get the focused field's rect without an engine patch: the helper's `crates/wubble-helper/src/autofill.js` already reports forms. Add the focused field's `getBoundingClientRect()` (CSS px, top document coordinates, plus the frame offsets when the field is in an iframe), and re-report on focus, scroll and resize, throttled to a frame. Carry it in the autofill report (`wubble_core::autofill` `FormSnapshot` or a new focus event). Place the chip and the menu at that rect in the field's pane: convert with the pane's rect (`Host::pane_rects`) and the page zoom. Unit tests in core for the placement maths.

## `settings-narrow-wrap`: Narrow Settings: slider and select rows wrap to full width

Area: todo-a. Needs an engine patch: no.

TODO line 132. At a content width of 759 px or less, the design's narrow block (`settings.css`:120-130 in the design repo's pages) wraps a row's control below its label at full width. wubble's slider and select rows do not (repro `WUBBLE_SHOT_SIZE=960x640 --smoke settings --shot`, which the owner runs, not you). In the kit settings row (`crates/wubble-ui/ui/kit/settings/*.slint` and the row parts that hold sliders and selects), add the narrow layout. Below the design's breakpoint (from `kit/gen/css.slint`, generated by `tools/fidelity/extract.mjs`; add it there if missing), put the control on its own line at full width with the design's gap. Keep it in kit files only, with the `// design:` header updated to the lines it now covers.

## `focuskeys-repeat`: FocusKeys with native-button false fires only once on a held Enter or Space

Area: todo-a. Needs an engine patch: no.

TODO line 134. `FocusKeys` (crates/wubble-ui/ui/components/focus-ring.slint ~33) with `native-button: false` fires its activation again on key repeat (a held Enter or Space). A native button activates once per press: Space on release, Enter on press without repeat. Slint's `KeyEvent` has a `repeat` field. In `key-pressed`, ignore events with `event.repeat` for Enter and Space. Activate Space on `key-released`, and Enter on its first press, as the platform buttons do. This is a component file. If it moves into the kit, it follows kit rules. Check that every user of `FocusKeys` keeps working (list rows, settings rows, menus).

