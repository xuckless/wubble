# Build-out batch 1: what the agents report (2026-11-22)

Written without tests, smokes or reviews; each branch then got one compile-only pass. Nothing here has been run. The text is each agent's own report.

## `chrome-shortcut-gaps` (`feat/chrome-shortcut-gaps`): written, 3 commits, 17 files changed, 736 insertions(+), 38 deletions(-)

### Written

All of the spec is written, in 3 commits on feat/chrome-shortcut-gaps (worktree /home/xuckl/Dev/wubble-wt/feat/chrome-shortcut-gaps/w). Per the brief, nothing was built, checked or tested, and I added no new tests.

What the keys do now:
- **Move tab up/down:** Ctrl+Shift+PageUp/PageDown (PageFirst) move the visible tab one place. It stays inside its pinned row, its folder or the unpinned list, and stops at the end. New `Model::tab_shift`, run through `Command::MoveTab`.
- **macOS tab keys:** Cmd+Shift+] and Cmd+Shift+[ select the next and previous tab. They are bound as "Cmd+}" and "Cmd+{" (Shell), because Shift turns the key into that character. Cmd+Alt+Left/Right stay NextSpace/PrevSpace.
- **Clear browsing data:** Ctrl+Shift+Delete, and Cmd+Shift+Backspace on macOS (PageFirst).
- **Save page:** Mod+S runs `Action::SavePage`, which uses the existing Save As download path. It does nothing on wubble:// pages.
- **Open file:** Mod+O opens the OS dialog (multiple files allowed) and opens each file as a file:// tab. New `wubble_core::url::file_url`.
- **Console:** Ctrl+Shift+J, and Cmd+Alt+J on macOS, open DevTools on the Console. This is a new `DockEvent::Console`; `Dock::Resolving` gained a `console` flag, and the docked frontend URL gets `&panel=console`. If the panel is already docked it reopens on the Console. CEF's separate DevTools window cannot be told a panel, so there it opens on the default panel and an open one stays as it is.
- **Paste as plain text:** Ctrl+Shift+V, and Cmd+Alt+Shift+V on macOS (PageFirst, counts as an edit command), through the existing `EditCommand::PasteMatchStyle`.
- **Caret browsing (F7):** a small pure transition in the new `wubble_core::caret`. The state is Chrome's own two profile preferences, read and written by new engine methods `View::caret_browsing` and `View::set_caret_browsing`. Turning it off is immediate. Turning it on shows Chrome's question as a shell menu (the same way clear-data confirms): "Turn On" or "Turn On, Don't Ask Again"; dismissing it changes nothing. If Chromium has no such preference, it logs and does nothing.
- **Focus search:** Ctrl+E, and Cmd+Alt+F on macOS. The bar opens on the visible tab with an empty field, and everything typed is treated as a search (the omnibox `?` prefix, applied by the host; the field does not show a `?`).
- **Minimize:** Cmd+M on macOS, via Slint's `set_minimized`.
- **Window cycling:** Cmd+` and Cmd+Shift+` (bound as "Cmd+~") on macOS, and the same keys on Ctrl on Linux and Windows. They go round the windows in the order they were opened, skip the picture-in-picture window, focus the model window and raise the platform window. New `Model::window_step`.
- **Command bar Enter:** Alt+Enter (also Cmd+Enter on macOS) opens in a new tab, Shift+Enter in a new window, and Ctrl+Enter on the first row adds www. and .com to a bare name. Pure helpers are in omnibox (`disposition`, `with_desired_tld`, `as_search`); bar.rs gained `target_command_as` and `Choice::NewWindow`. `target_command` behaves as before. Shift+Enter dispatches a new window and then the tab, so if the new window starts with its own start tab, both appear.

Each new row is Shell or PageFirst following Chromium's reserved-command list. The 11 new actions are in `Action`, `FIXED` and the keymap defaults; 10 of them (all but focus search) also got command-bar titles in `PALETTE`, which the editor uses as labels.

Main files: crates/wubble-core/src/{action.rs, keys.rs, model.rs, model/windows.rs, omnibox.rs, url.rs, caret.rs (new), devtools.rs}, crates/wubble-engine/src/view.rs, crates/wubble/src/{perform.rs, bar.rs, menu.rs, host.rs, host/shortcuts.rs (new), host/devtools.rs, host/surfaces.rs}.

Left out: the keymap tests the spec asked for, because the brief forbids writing tests. I did edit existing tests so they agree with the new table and types: action counts 86 to 97, the edit-action list, Ctrl+S now expected to be Save Page, ClearBrowsingData taken off the unbound list, Minimize unbound outside macOS, and the `Dock::Resolving` shape. No kit or .slint changes. I added no settings rows, because the caret state stays in Chromium's own preferences rather than being stored twice.

### Notes for the review

TODO.md lines I would have added:
- chrome-shortcut-gaps: compile, then run cargo test -p wubble-core and -p wubble. Existing tests were changed to match (action counts 97, the edit-action list, the Ctrl+S save test, the unbound list per platform, the `Dock::Resolving { console }` shape); new keymap tests for conflicts and editor labels are still owed.
- chrome-shortcut-gaps: check by hand with `scripts/headless.sh` that F7 reaches Blink under CEF's Chrome runtime, that Ctrl+Shift+J lands on the Console when DevTools is docked, and that Slint's TextInput fires `accepted` for Alt, Shift and Ctrl+Enter (it should, from the i-slint-core 1.19 source).
- chrome-shortcut-gaps: Ctrl+E keeps no visible `?` in the field. A kit function to put the caret at the end (`set-selection-offsets`) would let the field show `?` as Chrome does; that needs the owner.
- chrome-shortcut-gaps: caret browsing is per Chromium profile, which in wubble means per container. F7 writes it through every live view, so a container with no open view keeps its old value until it is toggled there.

docs/decisions.md lines I would have added:
- Caret browsing state is Chrome's own profile preferences (`settings.a11y.caret_browsing.enabled`, `settings.a11y.caretbrowsing.show_dialog`); wubble stores no setting for it, and F7's confirmation is a shell menu, as clear browsing data is.
- Window cycling is Cmd+` / Cmd+Shift+` on macOS, Ctrl+` / Ctrl+Shift+` on Linux and Windows, in the order windows were opened (macOS-style), skipping the picture-in-picture window.
- The command bar's held keys decide where a row opens (`omnibox::disposition`): Alt (Cmd on macOS) a new tab, Shift a new window; Ctrl+Enter on the typed name adds www. and .com.

Settings added: none. THIRD_PARTY_NOTICES: no new entry; no code was copied, and keys.rs already cites Chromium's `accelerator_table.cc` (BSD-3-Clause).

For review:
- The keybinding editor label for PasteAndMatchStyle comes from its palette title, "Paste as plain text". Action ids are kebab-case, e.g. `paste-and-match-style`, `move-tab-up`, `toggle-caret-browsing`, `next-window`.
- The new palette rows (Save page as, Open file, Move tab up/down, Console in developer tools, Caret browsing, Next/Previous window, Paste as plain text, Minimize window) may rank above searches for queries made of those words.
- Ctrl+Shift+PageUp/PageDown is PageFirst, so a page that handles the key keeps it, as in Chromium.

### Compile pass

Compiles now: `cargo.sh check --workspace` finished with exit OK (Finished dev profile) on round 2 of the 4 allowed. Only dead_code and other warnings remain, for example `bar.rs:185 target_command` is now unused.

Round 1 failed in wubble-store with 3 errors: `BookmarkEdit::SaveFolder` not covered in bookmarks.rs:357, and a missing `tint` field on `Folder` in session.rs:116 and import.rs:209. Neither `SaveFolder` nor `Folder.tint` exists in this branch's source. The cause was a stale cache, not a code bug. The shared target dir is `.targets/w` (named by worktree basename), so other branches' wubble-core artifacts got reused. I touched every .rs, .slint and Cargo.toml under crates/ to force a rebuild, and round 2 was clean.

I made no source edits and `git status` is clean, so there was no "compile fixes" commit to make. The branch head is still dfe3dd9.

Heads-up for the lead: every feature worktree is named `w`, so they all share one target dir, and mtime fingerprints can mislead. If another branch shows impossible errors, touch the sources.

TODO.md and decisions.md notes: none from the compile step.

audit.sh output: slice: 3.3 GB of 20 GB; free: 24 GB available; /tmp: 186M used

## `multi-select-tabs` (`feat/multi-select-tabs`): written, 2 commits, 20 files changed, 986 insertions(+), 37 deletions(-)

### Written

I wrote multi-select-tabs on branch feat/multi-select-tabs, worktree ~/Dev/wubble-wt/feat/multi-select-tabs/w, in 2 commits. Nothing was built, checked or run, as ordered, so it has not been compiled.

Core (wubble-core):
- New crates/wubble-core/src/tabselect.rs: one `Selection` enum (`Single`, or `Many { space, pinned, anchor, tabs }`) and one pure `transition(selection, input, &Model) -> (selection, effects)`. Inputs are `Click { tab, mods }`, `Sync { space }` and `Clear`; the only effect is `Activate(tab)`. Clicks follow Chromium's ToggleSelectionAt, ExtendSelectionTo and AddSelectionFromAnchorTo (behaviour only, BSD-3; a row is in THIRD_PARTY_NOTICES.md). Pinned and normal tabs, and tabs of different Spaces, never share a selection. A range skips tabs in collapsed folders and peek tabs. `Sync` drops the selection on a Space switch or when a tab outside it becomes active, and drops closed or moved tabs; under two left, it falls back to `Single`. Helpers: `acting(tab)`, `contains`, `tabs`, `Mods::from_bits` (1 = Ctrl/Cmd, 2 = Shift).
- model.rs gains `PinTabs`, `UnpinTabs` (order kept), `MoveTabs { tabs, to }` (each lands right after the previous, with an index fix-up), `FolderWithTabs`, `DuplicateTabs` and `MoveTabsToWindow { tabs, to }` (in model/windows.rs: one window for all of them, pinned tabs re-pinned, the previously active tab shown). It also gains `pub fn place_before(anchor, tab)`.
- tabmenu.rs gains `Run::ReloadTabs`, `MuteTabs`, `SplitPair` and `WindowEntriesOf`, plus `selection_entries(model, tab, selection, facts)`. Its rows: Reload Tabs, Pin/Unpin Tabs, Duplicate Tabs, Copy Links, New Folder with N Tabs, Move Tabs to <folder>, Move Tabs to <Space>, Open in Split View (exactly two), Move Tabs to New Window / Window: X, Mute/Unmute Tabs, Close Other Tabs, Close Tabs Below, Close N Tabs. `entries()` keeps its signature for a single tab.

Host and wiring (crates/wubble):
- New host/tab_select.rs: `tab_pressed`, `tab_picked` (reads the press's modifiers), `clear_tab_selection` (Escape), `sync_selection` (runs on every window render), `selection_marks`, drag-group helpers, `move_group_to_space` and `split_pair`.
- WindowHost holds the selection and the pending press; Host has `drag_tabs`.
- main.rs: `tab-activated` now goes through `host.tab_picked`, and `tab-track` carries the modifiers.
- host.rs `move_tab` regroups a dragged selection around the dropped row with `MoveTabs`.
- tab_drag.rs carries the group onto a Space, a new Space or a window (new or existing). Two selected tabs dropped on an edge make a split of the two; with more than two, only the dragged tab splits. The drag ghost reads "N Tabs".
- The tab context menu (panes.rs) uses `menu::selection_tab_entries`. Its mute label says Unmute only when every selected tab is muted.
- surfaces.rs runs ReloadTabs, MuteTabs (new `set_muted`) and SplitPair.
- shell::TabRow and bind.rs carry a new `picked` flag.

UI:
- `TabItem.picked` added in components/model.slint.
- kit/chrome/tab-row.slint: `track` gets a modifiers argument (Ctrl or Meta = 1, Shift = 2). A selected row keeps the existing hover background, and the accessibility selected state includes `picked`.
- kit/chrome/favs.slint: a new `press(id, mods)` callback, and a selected tile keeps the hover look.
- sidebar.slint forwards both; a favourite's press arrives as `tab-track` kind 4.
- No new drawing, tokens or literals, so nothing goes on the derived-review sheet. I did not pull or check the design repo for a multi-select widget.

Left out, and why:
- No unit tests, because the brief forbids writing tests; the spec asked for tests in tabselect.
- No "Bookmark Tabs" row: it needs a store-side "bookmark all into a folder" path.
- Favourites cannot be dragged as a group, because favourites have no drag today.
- Only the pressed row lifts during an in-list drag; the other selected rows join it when it is dropped.
- tabdrag.rs itself is unchanged. Its effects stay single-tab and Copy, and the host expands them with the group it recorded at press time.
- The spec says a tab-row Shift+click opens a peek; I found no such handler in tab-row.slint, so Shift+click on a row now only selects a range.

### Notes for the review

TODO.md lines I would have added:
- Multi-select tabs: unit tests for tabselect::transition (toggle, range, Ctrl+Shift range, cross-section reset, sync on space switch / close / activation), for tabmenu::selection_entries, and for Model MoveTabs, FolderWithTabs, UnpinTabs order and MoveTabsToWindow.
- Multi-select tabs: "Bookmark Tabs" row (bookmark every selected tab into a new folder) once the store has a bulk path.
- Multi-select tabs: lift and animate the whole selection during an in-list drag (only the pressed row lifts now); drag a selection of favourites.
- Multi-select tabs: chrome.tabs.highlight / the extension API could set the window's selection.

docs/decisions.md lines I would have added:
- Tab multi-selection is per window (WindowHost.selection), one tabselect::Selection and one pure transition; never mixes pinned and unpinned tabs or Spaces; a range skips collapsed folders.
- A dragged selection is expanded by the host from the group recorded at press time; tabdrag stays single-tab and Copy.
- Two selected tabs dropped on a split edge make a split of the two; with more than two, only the dragged tab splits.
- The tab menu's selection labels use the repo's Title Case ("Close 3 Tabs", "Pin Tabs", "Move Tabs to New Window", "New Folder with 3 Tabs", "Mute Tabs"), not the spec's lower case.

Settings added: none.

For the owner's review:
- None of this has been compiled.
- Ctrl/Cmd is read as `control || meta` in Slint, so it works whichever way Slint maps Cmd on macOS. On Linux, Super+click also toggles.
- Escape clears the selection only after open overlays have taken their Escape.
- A plain click on a selected row clears the selection on release, as in Chrome; a press-and-drag keeps it.
- `tab_entries` and `tabmenu::entries` keep their signatures, so the existing tests still compile; the selection-aware versions are new functions next to them.

### Compile pass

feat/multi-select-tabs compiles now (workspace check finished clean).

Rounds: 2 check rounds.
- Round 1 failed with one error: E0004 non-exhaustive `CommandMode::Tabs` in crates/wubble-ui/src/driver.rs:762. The branch source has no `CommandMode::Tabs` (chrome.rs has only Here, New and Split). I read this as stale cached build output in the shared target dir, not a code error.
- I touched crates/wubble-core/src/lib.rs and chrome.rs to force a rebuild. Round 2 then passed with no errors.

There were no source fixes, so there are no changes to commit and I made no "compile fixes" commit. `git status` is clean. No warnings were fixed.

Audit output: `slice: 3.7 GB of 20 GB; free: 23 GB available; /tmp: 186M used`

Note for the lead: another branch's wubble-core build, which has `CommandMode::Tabs`, seems to leave stale output in the shared target dir. Other branches may hit the same bogus error. Touching the wubble-core source files fixes it.

Nothing for TODO.md or docs/decisions.md from this step.

## `multiple-profiles` (`feat/multiple-profiles`): written, 4 commits, 24 files changed, 1845 insertions(+), 21 deletions(-)

### Written

I wrote multiple profiles, the Guest profile and the profile switcher on branch feat/multiple-profiles (worktree ~/Dev/wubble-wt/feat/multiple-profiles/w), in 3 commits. As the brief ordered, nothing was built, checked or run, so none of this has been compiled. I checked every type and function I used against the real code by reading it.

Core: crates/wubble-core/src/profiles.rs is pure, with no I/O. It holds:
- The profile list kept in `<data>/profiles.json`: id, name, colour (a swatch name), last used, the last profile used, and "Show on startup". The parser is tolerant, the default profile is always present and first, and ids are checked so they are safe as folder names.
- Folder paths: the default profile stays at today's data folder, so nothing migrates; new profiles go in `<data>/Profiles/<id>` (Chrome-style "Profile N", default names "Person N"); Guest is `<data>/Guest`.
- `transition` for changes (Add, Rename, Tint, Remove, Opened, ShowOnStartup), returning effects (Save, DeleteDir, Open).
- `start()`, which picks the profile for a launch: `--guest`, `--profile DIR` (named when it matches a listed profile), `--profile-directory NAME` (by folder name or by name, and created if missing, as Chrome does), or else the last used profile, with the picker on top when "Show on startup" is on and there is more than one profile.
- `picker_transition` for the picker sheet. If you choose another profile from the picker shown at startup, this browser quits, since Chrome shows only the picker there.
- `window_title`, `mark` and `rows`.

Other core changes:
- `Action::SwitchProfile`: id "switch-profile", bound to Mod+Shift+M in `keys::DEFAULTS`, plus a "Switch profile" command-bar row in `PALETTE`. I updated the action-count asserts from 86 to 87.
- `overlay::Kind::ProfileSheet`: belongs to the window, modal, closes when you leave the tab.
- `settings::Section::Profiles`.

Binary:
- crates/wubble/src/profiles.rs reads and saves the list (written to a temp file, then renamed over the old one), carries out effects, and opens a profile: it sends `focus` over the instance socket if that profile is already running (new `instance::send`), otherwise it starts `current_exe --profile DIR` or `--guest`, with a thread that waits on the child. It deletes a profile's folder only under `Profiles/`, never the default, and never while that profile is running. It also removes the Guest folder at the end of its run and again at every start (`sweep_guest`).
- cli.rs gains `--profile-directory` and `--guest`, both conflicting with `--profile`, smokes and measurements.
- main.rs gains `choose_profile`, which records the launch and sets the process-wide `profiles::Here`, plus the callback wiring and the startup hook.
- host/profiles.rs holds the picker (opened through the overlay stack), the Settings handlers, and the window title, footer mark and picker views for each window. Deleting is refused for the default profile, this window's profile, or any profile that is running.
- `Plan::SwitchProfile` in perform.rs; the overlay stack and close handling in layers.rs; the Settings render calls the profiles render.

UI:
- New derived kit piece kit/chrome/profile-mark.slint: the footer icon button with the Space switcher's colour capsule and the name's first letter.
- components/profile-sheet.slint: the picker, a kit Sheet of settings rows (each profile, Guest, Add profile).
- components/profiles-section.slint: laid out like the Spaces section (list editor, rename in place, colour swatches, delete asked twice, Open, Guest, a "Show on startup" switch, a refusal note).
- settings.slint: a Profiles section and nav row between Agents (MCP) and Updates, with the section index arithmetic updated.
- side-bottom.slint and sidebar.slint: the mark leads the footer buttons.
- app.slint: properties and callbacks, the sheet, and a `window-title` property feeding `title`.
- model.slint: ProfileItem, ProfileMarkView, ProfilePickerView, ProfilesView, and `GlassKeys.profile-sheet` = 1000093.
- docs/fidelity/derived-review.md: a new Profiles table with the three derived pieces.

Linux, macOS and Windows share the same code: the existing `user_data_dir` per-platform trait, the instance socket (named pipe on Windows), and `std::process` for starting a profile.

Left out:
- No tests (the brief overrode the spec's "with unit tests"), no screenshots, and no kit_lock or baseline run.
- No person glyph: the nav row uses the bubble glyph and the mark draws the initial; a real glyph waits for the owner.
- The picker opened at startup is drawn over the last used profile's window, which is already running, rather than as a separate window.

### Notes for the review

TODO.md lines I would have added:
- Profiles: unit tests for wubble_core::profiles (parse/repair, transition, start, picker_transition, window_title, mark), skipped on the no-tests order.
- Profiles: shoot the picker sheet, the Profiles section and the footer mark with two profiles and with Guest (gallery scenes); owner review on derived-review.md "Profiles".
- Profiles: a person glyph from the owner, for the nav row (it borrows Glyph.bubble now) and Chrome's avatar.
- Profiles: Chrome's "Delete" also closes that profile's open windows. wubble refuses while the profile is running; a `close` instance message could do it instead.
- Profiles: per-profile avatar pictures and Chrome's profile-picker window when there is no window at all (today the picker opens over the last used profile's window).
- Profiles: the gallery and kit_lock tests have not been run against the new slint files; tests asserting the settings nav order may need the Profiles row.

docs/decisions.md lines I would have added:
- Profiles: one wubble process per profile (CEF's root_cache_path is process-wide); the list lives in `<data>/profiles.json` beside every profile, not in a profile's database; the default profile stays at the per-user data folder (no migration), others go in `<data>/Profiles/<id>`, Guest in `<data>/Guest`.
- Profiles: "Show on startup" is stored in profiles.json, not the settings table, because it must be read before a profile is chosen; Section::Profiles has no table rows.
- Profiles: opening a profile sends `focus` to its running instance, else starts `current_exe --profile DIR` (or `--guest`); `--no-sandbox` carries over.
- Profiles: the picker shown at startup is drawn over the last used profile; choosing another profile starts it and quits this one.
- Profiles: the window title is "wubble - <name>" only with more than one profile or on Guest (as Chromium appends it).

No settings-table rows were added. Settings > Profiles holds add, rename, colour, delete (asked twice by the list editor), Open, Guest and a "Show on startup" switch. New CLI flags: `--profile-directory NAME`, `--guest`. New key: Mod+Shift+M. New command-bar row: "Switch profile". New glass key: GlassKeys.profile-sheet = 1000093. Nothing makes a network request.

For review:
- Untested logic I would check first: the settings section index shift (profiles-at = table+6-gap, updates-at +7, about-at +8).
- Merge conflicts expected with the other batch-01 lanes on action.rs (FIXED and PALETTE counts), keys.rs DEFAULTS, overlay.rs Kind, model.slint GlassKeys and app.slint.
- Shell .slint files add no literals or token groups; the kit file takes every value from gen/.

### Compile pass

feat/multiple-profiles compiles now: `cargo.sh check --workspace` finished with no errors, in 2 check rounds.

Round 1 failed in the wubble-ui build script on a Slint error in `crates/wubble-ui/ui/kit/chrome/profile-mark.slint`: "The property 'x' cannot be set for elements placed in this layout". `ProfileMark` inherits `PressableButton`, whose children go into a `HorizontalLayout`, so the child Rectangle's explicit `x`/`y` were rejected. I wrapped the colour capsule and the initial text in one Rectangle, so they are positioned inside it and not by the layout. Round 2 compiled cleanly.

The fix is committed on feat/multiple-profiles as "feat/multiple-profiles: compile fixes". Nothing else was run, and nothing is still failing.

Audit output: `slice: 3.4 GB of 20 GB; free: 25 GB available; /tmp: 188M used`

Nothing for TODO.md or docs/decisions.md.

## `nav-history-capture` (`feat/nav-history-capture`): written, 5 commits, 25 files changed, 1088 insertions(+), 27 deletions(-)

### Written

I wrote nav-history-capture on branch feat/nav-history-capture (worktree ~/Dev/wubble-wt/feat/nav-history-capture/w) in 4 commits. Following the standing rules, nothing was built, checked, tested or run, so the code has never been compiled. I checked each CEF, turso and Slint call against the real sources by hand. The 2nd and 3rd commits do not compile on their own: `StoreClient::save` gets its new argument in commit 3, but host.rs only passes it in commit 4.

What it does:
- **Core** (crates/wubble-core/src/navstack.rs, pure): the stack type `NavStack` (entries of url and title, plus the current index), capped at 50 entries kept around the current one. The back/forward menu lists at most 12 entries, nearest first, and labels each by its title, or its URL when it has none (Chrome's rule). `Histories` is the host's table keyed by tab. `Hold` with `fn hold(state, event) -> (state, effects)` is the press-and-hold machine: a 500 ms press opens the menu, and the click from that release is ignored.
- **Engine**: crates/wubble-engine/src/history.rs reads the stack with `CefBrowserHost::GetNavigationEntries` and a visitor, so no CDP round trip is needed. view.rs reads it after each address change, title change and load end, and reports changes as a new `PageEvent::HistoryChanged(NavStack)`. It also adds `View::history()`, `refresh_history()` (a fresh read) and `go_to_offset(n)`. A step of 1 uses back/forward; longer jumps go through CDP `Page.getNavigationHistory` then `Page.navigateToHistoryEntry` (new `cdp::HistoryJumps`). No welding change was needed.
- **Store**: migration 17 (sql/017_tab_history.sql) adds two tables. `tab_history` (tab_id, idx, url, title, current) is cleared with the tabs in `write_model` and rewritten by the new `Store::save_session_with_history`; `Store::tab_history()` reads it back. `recently_closed_history` (closed_id, idx, url, title, current) holds closed tabs' stacks: `ClosedTab` gains `entries` and `index`, which push, pop and list carry.
- **Persist**: `StoreClient::save` now takes the table and writes only the stacks of tabs in the saved model, which leaves out agent Space tabs and peeks. Stored stacks are loaded at start (`Loaded::tab_history`), so sleeping restored tabs keep theirs.
- **Host** (crates/wubble/src/host/nav_history.rs): one table, `Host::nav_history`, outside `Model`. It is filled from `HistoryChanged`, skips agent-driven tabs, and drops closed tabs. Every page with a view is re-read before closing commands, and a tab is re-read before its view closes, which covers discard. Closed-tab rows carry the stack.
- **Menu**: right-click or a 500 ms hold on back or forward opens a shell menu (`menu::history_entries`, the same machine as `tab_entries`). It lists entries as `Command::HistoryGo(offset)`, then a separator and "Show Full History" (`Action::OpenHistory`, wubble://history). Ctrl/Cmd-click or middle-click opens the next entry that way in a background tab (`Command::BackgroundTab`).
- **UI**: in the shell file crates/wubble-ui/ui/components/nav-buttons.slint, back and forward now report history-clicked, history-pressed and history-menu. sidebar.slint and app.slint pass these up, and main.rs binds them. A plain click now goes through the hold machine and then `perform(Back/Forward)`.

Left out:
- **The restore half**: putting a stack back into a page after restart or reopen-closed-tab. CEF 156 has no API to load entries into a browser's history. Until then, reopen and session restore still load the URL only. The saved stack stays in the database, but a restored tab's stored stack is replaced by the page's own one-entry stack once it loads.
- **Duplicate does not copy the stack.**
- **Ctrl/middle-click opens only the URL**: Chrome duplicates the tab with its history and goes back in the copy; wubble opens the entry's URL in a background tab.
- **Chrome's "chapter stops"** (entries from other sites) are not in the menu.
- **Ctrl/middle-click on a menu row** does nothing special.
- **Private tabs**: main has no private Spaces yet, so only agent tabs are skipped.

### Notes for the review

TODO.md lines I would have added:
- History capture is done; the restore half is still open. Putting saved stacks back into a page (session restore, reopen closed tab, duplicate) needs a CEF change: an API to restore navigation entries (Chromium `NavigationController::Restore` / `content::NavigationEntry::Create` from `sessions::SerializedNavigationEntry`). The stacks are already saved in `tab_history` and `recently_closed_history`.
- Skip private tabs in `Host::nav_history_changed` and in the session save once private-windows lands (agent tabs are skipped today).
- Back/forward menu: Chrome's chapter stops, and middle-click on a menu row to open in a new tab.

docs/decisions.md lines I would have added:
- Back/forward stacks are read with `CefBrowserHost::GetNavigationEntries`, which runs synchronously on CEF's UI thread, not CDP `Page.getNavigationHistory`. A read before a close or discard therefore needs no round trip. Jumps of more than one step use CDP `Page.navigateToHistoryEntry`.
- Each tab keeps at most 50 entries (Chromium's `gMaxPersistNavigationCount`) in total around the current entry. Chromium keeps 50 on each side.
- The history table is one host-side owner (`Host::nav_history`, `wubble_core::navstack::Histories`), not in `Model`. It is saved in the session transaction and `write_model` clears it with the tabs.

Things to know when reviewing:
- Merge conflict risk: schema migration number 17 (crates/wubble-store/src/migrate.rs). Any other batch-01 branch that adds a migration will clash and must renumber.
- Kit change: the base button crates/wubble-ui/ui/kit/controls/button.slint (`PressableButton`) gains `callback middle-clicked()` and `out property <bool> modified-press` (Ctrl or Cmd held on the last left press; reset on keyboard activation). This is behaviour only, with no new look or values, so I added nothing to derived-review.md.
- The hold delay (500 ms) lives in Rust (`navstack::HOLD_MS`), not in Slint, so the kit lock's literal rule is untouched.
- The `StoreClient::save` signature changed, and `Request::Save` gained a field. I updated the existing test helpers and test calls in persist.rs, wubble-store closed.rs and history.rs so they still compile. No new tests were written.
- Nothing in the feature sends a network request.
- No settings were added.
- No code was copied, so there is no THIRD_PARTY_NOTICES entry. Chromium and CEF are cited only for constants and policy.

### Compile pass

Compiles now: `cargo check --workspace` finished clean. It took 2 check rounds.

Round 1 had one error: E0599 in crates/wubble/src/host/nav_history.rs:141, `window.window()` on `AppWindow`, because the `slint::ComponentHandle` trait was not imported. I added `use slint::ComponentHandle;`. Round 2 passed. I did not look at warnings.

I committed the fix on feat/nav-history-capture as "feat/nav-history-capture: compile fixes". Nothing is still failing.

Audit output (~/Dev/wubble-wt/audit.sh): `slice: 3.3 GB of 20 GB; free: 24 GB available; /tmp: 188M used`

Nothing for TODO.md or docs/decisions.md beyond the author's report.

## `omni-calculator-units` (`feat/omni-calculator-units`): written, 2 commits, 8 files changed, 813 insertions(+), 4 deletions(-)

### Written

The feature is written to spec on branch feat/omni-calculator-units (worktree ~/Dev/wubble-wt/feat/omni-calculator-units/w), in two commits. Nothing was built, checked or run, as the brief ordered. I reused the uncommitted work from an earlier attempt and checked it against the real code before committing: Glyph.duplicate exists in kit/controls/icon.slint, Host::copy and Host::toast(&AppWindow, &str) are in host/surfaces.rs, and the only exhaustive matches on Kind and Target are in bar.rs, which now covers the new variants.

1ac7521 core:
- New file crates/wubble-core/src/answer.rs, pure and written from scratch (no copied code, so no THIRD_PARTY_NOTICES entry).
  - Calculator: a small parser with + - * / ^ (right-associative, so -2^2 is -4), % as remainder between two numbers, a trailing % as percent ('15% of 80' is 12, '100 + 10%' is 110, as Google does), unary minus, implicit multiplication (2pi, 3(4+1)), and an unclosed '(' at the end is accepted. Constants pi, π and e; functions sqrt, sin, cos, tan, log (base 10), ln, abs, round. It counts operators: a bare number or constant gives no answer, and division by zero or a result that is not finite gives no row.
  - Units: the table is data (UNITS, base = (value + offset) * factor, exact SI factors) for length, mass, volume, temperature, speed, area, data size and time. The forms are '<n> <unit> in|to <unit>', with °, 'degree(s)' and multi-word names handled, and '5 in in cm' parses. No currency.
  - format_number keeps 12 significant digits for math and 6 for units, trims trailing zeros, and uses e-notation outside 1e-6..1e15.
  - Unit tests cover parsing, precedence, percent, division by zero and junk, a bare number, each unit family, formatting, and unit names being unique and lower case.
- crates/wubble-core/src/lib.rs: registers the module.
- crates/wubble-core/src/omnibox.rs: adds Kind::Answer and Target::Copy(String). rank() puts the answer first for math and second, under the search intent, for unit forms. The row's subtitle is "Copy". There is a test for the ranking.

a47219a UI and wiring:
- crates/wubble-ui/ui/components/model.slint: adds CommandKind.answer.
- crates/wubble-ui/ui/kit/chrome/command-row.slint: an answer row uses the existing duplicate glyph, because the kit has no calculator or equals glyph.
- crates/wubble/src/bar.rs:
  - Maps Kind::Answer to CommandKind::Answer.
  - Adds Choice::Copy.
  - On first run, the engine-choice rows go below a math answer, just as they go below the "open" row.
- crates/wubble/src/host.rs: Choice::Copy copies the result through Host::copy (the clipboard.rs path) and shows the toast "Copied <result>".
- docs/fidelity/derived-review.md: a row for the derived answer row.

Left out:
- Currency, as the spec says (it would need a network rate source).
- No shots were taken, since the brief forbids running anything.

### Notes for the review

Lines I would have added:
- TODO.md: "Command bar answers: owner to review the answer row's glyph (duplicate for now; a calculator/equals glyph would be a new kit icon) and shoot command/answer in the gallery."
- docs/decisions.md: "Command-bar calculator and unit answers are computed on the device (wubble_core::answer). Chrome's server answers and currency (rates need a network source) are left out."
- docs/decisions.md: "'%' between two numbers is the remainder; a trailing '%' is a percent, and 'a + b%' changes a by b percent, as Google's calculator does."

Settings: none were added (the answers are local, so no setting is needed).

For the owner's review:
- Kit lock: no shell file draws anything. The only kit change is a glyph branch in command-row.slint, and docs/fidelity/derived-review.md has a new row for the derived answer row (no shot yet).
- Data units are case-insensitive: 'mb' means megabyte, and bits are written out as mbit or megabit.
- Months and years use the Gregorian average of 365.2425 days.
- rustfmt was not run, so the long UNITS lines in crates/wubble-core/src/answer.rs may get reflowed when the compile step formats.

### Compile pass

Compiles now: yes. `cargo.sh check --workspace` finished clean on round 1 of 4 (about 1m 27s, no errors), so I made no fixes and no commit ("feat/omni-calculator-units: compile fixes" was not needed). The worktree is clean.

Memory before the check: 14 GB available, which is above the 8 GB threshold.

Still failing: nothing.

audit.sh output: `slice: 17.4 GB of 20 GB; free: 25 GB available; /tmp: 186M used`

Nothing to add for TODO.md or docs/decisions.md beyond the author's report.

## `omni-inline-autocomplete` (`feat/omni-inline-autocomplete`): written, 2 commits, 10 files changed, 552 insertions(+), 26 deletions(-)

### Written

I wrote omni-inline-autocomplete on branch feat/omni-inline-autocomplete, worktree ~/Dev/wubble-wt/feat/omni-inline-autocomplete/w, in 2 commits. Per the brief, nothing was built, checked or run, and no new tests were written.

Commit 752ee07, wubble-core (crates/wubble-core/src/omnibox.rs, pure). This ports Chromium's behaviour only, no code copied (BSD-3):
- `inline_completion(query, &[HistoryEntry], &[Bookmark]) -> Option<String>` returns the suffix. `inline_match` returns the suffix plus the URL Enter opens.
- Which pages complete:
  - A history page whose URL starts with the query, tried as the full URL, without its scheme, and without its scheme and `www.` (ASCII case-insensitive).
  - It must have been typed once for a bare host, twice for a deeper page (HistoryURLProvider's `CanPromoteMatchForInlineAutocomplete`), or visited 10 times or more (a stand-in for HistoryQuickProvider's score bar).
  - Any bookmark also qualifies.
  - If the query ends inside the host of a deeper page, it completes to the host root ("git" gives "github.com/").
- Order among several: typed pages first, then bare hosts, more typed visits, more visits, newest, shortest.
- Nothing is offered for a query with any whitespace (a trailing space included) or a `?` prefix.
- `just_deleted(before, after)` is Chrome's no-completion-after-a-deletion test, worked out from the two texts alone.
- `with_desired_tld` ports `AddDesiredTLD` for Ctrl+Enter: a name without a known TLD becomes `www.<name>.com`.
- `Candidates` has a new field, `prevent_inline`. When a completion exists, `rank` puts the completed URL as the first row, drops the duplicate history or bookmark row, and the "action named by the query" jump no longer applies.

Commit 022583e, wiring and UI:
- crates/wubble/src/bar.rs:
  - New `BarMode::NewWindow`, `Choice::NewWindow(url)`, and `BarInputs.prevent_inline`.
  - `EnterMods` and `enter_kind(mods, Platform)`: on macOS the Control key is Slint's `meta`, so Ctrl+Enter works there too; Alt or Option gives a new tab, Shift a new window.
  - `chosen_mode`, `with_tld_target`, `bar_inline`, and `removable_url` (history rows only).
  - On first run the engine-choice rows go after the leading "open" rows.
- crates/wubble/src/host.rs:
  - `Bar.prevent_inline`, set on every edit from `just_deleted`. It is always on for the start page's in-page capsule, which cannot show a completion, and on open (Chrome does not complete on focus).
  - `bar_inline_accepted`, `bar_chosen_with(index, mods)`, and `bar_row_removed(index)`, which calls `history_deleted` (HistoryClear::Url), drops the row locally, bumps the generation and re-ranks.
  - `choose_in`: a new window is a NewTab followed by MoveTabToWindow{New}.
  - `render_bar` also writes the inline view.
- crates/wubble/src/bind.rs: `render_inline`. crates/wubble/src/main.rs: three new callbacks.
- Slint:
  - components/model.slint: new struct `CommandInline {base, suffix, from, to, serial}`.
  - CommandHead:
    - `inline` property; a `changed` handler writes base + suffix and selects the suffix with `set-selection-offsets`, but only while the field still holds the host's text.
    - Right or End accepts the completion; typing over it or Backspace drops it.
    - Tab / Shift+Tab (Backtab) step through rows.
    - Enter with modifiers raises `opened`.
    - Shift+Delete asks `removing() -> bool`.
  - CommandBar: removes a row only if it is a history row; adds `result-opened`, `row-removed`, `inline-accepted`, `inline`.
  - app.slint: the matching `command-*` callbacks and `command-inline` property, forwarded only (no drawing, no literals).
- THIRD_PARTY_NOTICES.md: one behaviour-only Chromium row.

Left out:
- New unit tests: the spec asked for them in omnibox.rs, but the brief forbids writing tests. Three existing tests that rely on rows the completion now removes (omnibox `tab_beats_equal_class_history_and_dedupes`, bar `bookmarks_are_candidates`, `history_rows_open_their_url`) now pass `prevent_inline: true` so they keep checking what they did. The other `BarInputs` literals in bar.rs's tests pass `prevent_inline: false`.
- The start page's in-page capsule gets neither the completion nor the new keys.
- Nothing has been compiled. The Slint parts most worth checking at the compile step: `changed inline` on a struct property, the `<=>` to child `in` properties (the same pattern as `command-results`), and the `removing() -> bool` callback with return statements.

### Notes for the review

TODO.md lines I would have added:
- Command bar inline autocomplete: once it builds, check by hand that typing "git" shows "github.com/" selected, and that Right/End, Backspace, Enter, Alt/Shift/Ctrl+Enter, Tab and Shift+Delete behave as in Chrome (headless shot of the bar with a completion).
- Inline autocomplete for the start page's in-page capsule (search-capsule.slint has no completion yet; the host keeps it off there).
- Unit tests for omnibox::inline_match, just_deleted and with_desired_tld, and for bar::enter_kind (the spec asks for them; this lane was told to write none).

docs/decisions.md lines I would have added:
- Inline autocomplete follows HistoryURLProvider: a bare host needs 1 typed visit, a deeper page 2; 10 visits stand in for HistoryQuickProvider's score; bookmarks always qualify. A query that ends inside a deeper page's host completes to the host root.
- "Just deleted" is judged from the before and after texts (one cut run, or the same text), not from caret positions.
- Shift+Enter opens the page as a new tab that is then torn out to a new Main window (wubble windows show Spaces).
- Enter's modifiers are passed to Rust as raw booleans, and bar::enter_kind maps them per platform (Slint's `control` is Command on macOS, so Ctrl+Enter reads `meta` there).

Settings: none added. No network requests. No engine change.

For review:
- With a completion, the first row is the completed page (Kind::Search with Target::Url, titled with the URL) and the typed intent is second; the history or bookmark row for the same URL is not repeated.
- The field is updated only while its text still equals the typed base, or base plus the completion shown before; take-focus clears any old completion so select-all on open survives.
- A Shift+Delete removal bumps the bar generation, so a store reply already in flight that still holds the row is dropped.

### Compile pass

Compiles now: yes. The check passed on round 1 of 4 (`cargo.sh check --workspace`, finished in 46s). I made no fixes. The tree is clean, so there was nothing to commit and the "compile fixes" commit was not created. Nothing is still failing.

Audit output: `slice: 4.6 GB of 20 GB; free: 24 GB available; /tmp: 186M used`

Free memory at start was 23 GB available, above the 8 GB threshold.

For TODO.md and decisions.md: nothing from me. I didn't run any tests or smokes, so the Slint concerns the author flagged (`changed inline` on a struct property, `<=>` to child `in` properties, and the `removing() -> bool` callback) are compile-verified only.

## `private-windows` (`feat/private-windows`): written, 10 commits, 34 files changed, 721 insertions(+), 85 deletions(-)

### Written

Private windows are written on feat/private-windows (worktree ~/Dev/wubble-wt/feat/private-windows/w), in 10 commits. Nothing was compiled, checked, tested or shot, as the brief said. Nothing of the engine needed patching. An earlier attempt had left uncommitted core work in the tree. I reviewed it, kept it and finished the feature on top of it.

What it does:
- **Core model** (wubble-core `space.rs`, `model.rs`, `model/windows.rs`, `spacemenu.rs`):
  - New fields `Space::private`, `Container::ephemeral` and `PRIVATE_NAME`, plus `Command::NewPrivateWindow { url }`.
  - The first private window creates one ephemeral container; every private window shares it. Each private window gets a new private Space with the lilac swatch, showing its URL or the command bar.
  - `SwitchSpace` is refused in a private window and never reaches a private Space. Next/previous Space, the nth-Space keys and the Spaces popup skip private Spaces; a private window's popup lists no Spaces.
  - `set_active_space` never turns a private window to another Space. It uses the last normal window instead, or opens a new one.
  - New tabs, pop-ups and links from a private tab stay in its Space and container. A normal tab can never be put in the ephemeral container.
  - `MoveTab` and `MoveToWindow` refuse to cross between private and normal. Dragging a private tab out to a new window makes a new private window.
  - Closing a private window closes its tabs and deletes its Space. When the last private window goes, the container goes too (`Effect::ContainerDeleted`).
  - `DeleteSpace`, `SetSpaceContainer`, `ReopenInContainer`, `DeleteContainer` and `MarkAgentSpace` refuse private targets.
  - Agent `OpenInSpace`, extension tabs and Little windows never land in a private Space; with no Space given they go to the last normal window (`Model::normal_space`).
  - `for_session` drops private windows; if only private windows are open, a normal window stands in. `closes_into_recent`, `archive::due` and routing `resolve` skip private Spaces. The extension projection leaves private windows out.
  - `Model::check` now also checks the privacy invariants.
- **Engine** (`storage.rs`, `app.rs`): `Profiles` keeps a set of ephemeral containers. `get` opens those with `SharedProfile::new(runtime, None)`, which is an off-the-record profile under CEF's Chrome runtime. `forget_container` clears its cookies but never writes a tombstone (`storage::forget`). New public `Engine::set_ephemeral` and `Engine::is_ephemeral`.
- **Host** (crates/wubble):
  - `set_ephemeral` is called before a private tab's first view opens, and before Clear Browsing Data.
  - Private tabs write nothing to the store: no history visits, favicons, titles, archive entries, per-site zoom, Allow-popups choices or download rows. Their downloads stay in the list for this run only and leave it when the container goes; the files stay.
  - In a private window, links ignore routing rules. Agent sign-in sources skip private Spaces. Extensions cannot open, close or activate private tabs.
  - The command bar's open-tab rows stay on their own side: a private window lists only its own tabs, and no other window lists a private one. Settings' Spaces list hides private Spaces.
  - The tab menu's Move to Space rows and the Move to Window rows never cross the private line; neither do the Little and Popup windows' "Open as Tab" rows.
  - wubble-store `write_model` also leaves out every private part, because import calls it with the live model.
- **Keys and menus**: `Action::NewPrivateWindow` (id `new-private-window`) on Mod+Shift+N, a "New private window" row in the command bar, and "Open Link in Private Window" in menu.rs ROWS (not offered inside a private window; `NavFlags::private`).
- **UI**: new derived kit file `kit/chrome/private-mark.slint` with:
  - `PrivateChip`: in a private window the Space header shows the Space's name, "Private", as a lilac pill instead of plain text.
  - `PrivateTint`: abyss at 0.35 over the window's ground and glass, using the new `Shell.private-tint` value in derived.slint.
  - `SpaceItem.private` and `SpaceRow.private` carry the flag, and the window title becomes "wubble (Private)". Both pieces are on docs/fidelity/derived-review.md.

Left out or limited:
- Undo close tab (Ctrl+Shift+T) does not work inside a private window, because `closes_into_recent` excludes private tabs as the spec asks. Chrome keeps that within the session.
- Because `SwitchSpace` is refused while a private window has focus, a reopened closed tab or an agent approval that switches Space does nothing until a normal window is focused.
- An Allow-popups choice made in a private window is not stored, but it still applies to normal windows until restart.
- Save-password "Never" (autofill) is still stored from private windows.
- Dragging the last tab out of a private window leaves that window empty instead of closing it.
- No gallery scene or shots: nothing was run.

### Notes for the review

TODO.md lines I would have added:
- Private windows: a gallery scene (`private-window`) and shots of the Private chip and tint for the owner's eye (derived-review.md, Private windows).
- Private windows: Undo close tab inside a private window (a session-only closed list that is never stored), as Chrome's incognito keeps one.
- Private windows: keep Allow-popups and other site choices made in a private window in a private-only table instead of the shared one, and stop storing save-password "Never" from them.
- Private windows: close a private window whose last tab was dragged out (Chrome closes an empty window).
- Private windows: let a reopened tab or an agent approval switch Space while a private window has focus, by focusing the last normal window instead of refusing.
- Private windows: Chrome's "N incognito downloads will be cancelled" prompt when the last private window closes with downloads running.

docs/decisions.md lines I would have added:
- Private windows: one ephemeral container (`Container::ephemeral`) shared by every private window, opened with no storage directory (CEF Chrome runtime: an off-the-record profile); it goes with the last private window, and no tombstone is ever written for it.
- A private window is a Main window locked to its own private Space: no Space switching, its tabs never cross to a normal Space or window, and its Space is deleted when it closes.
- Nothing of a private window is stored: the session, history, favicons, titles, recently closed, archive, zoom, popup permission and download rows. Routing rules, extensions, agent sign-in sources and MCP `OpenInSpace` never reach it.
- Private window mark: the Space name "Private" as a lilac chip in the Space header, abyss at 0.35 over the window's ground and glass, and the window title "wubble (Private)".

Settings added: none. Mod+Shift+N is a new default binding.

For the owner's review:
- Nothing was compiled, so the first build may turn up small type errors.
- Existing menu.rs tests were updated for the new link-menu row: the expected labels and indices in `a_link_offers_new_tab_and_copy` and `a_linked_image_lists_both_groups`, plus `private: false` in a NavFlags literal. No new tests were written.
- `FIXED` is now 69 entries and `PALETTE` 32.
- The earlier attempt's uncommitted core work is in the first commit (1f23c2a).
- No code was copied, so there is no THIRD_PARTY_NOTICES entry; the behaviour follows Chrome's incognito (`IDC_CONTENT_CONTEXT_OPENLINKOFFTHERECORD`, the primary off-the-record profile).
- The `--measure idle` check was not run. The tint is a static Rectangle, drawn only in private windows.

### Compile pass

feat/private-windows compiles now. `cargo.sh check --workspace` finished clean on round 1 of 4, so it needed no fixes. Nothing is failing. The working tree was clean, so there was nothing to commit and no "compile fixes" commit exists.

Audit output: slice: 2.6 GB of 20 GB; free: 24 GB available; /tmp: 186M used

## `recently-closed-full` (`feat/recently-closed-full`): written, 3 commits, 23 files changed, 1536 insertions(+), 201 deletions(-)

### Written

The recently-closed list now holds whole windows, folders and Spaces as well as tabs, and the history page and the command bar list it. Nothing was built, checked or tested (the brief said not to), so this has never been compiled. It is 3 commits on feat/recently-closed-full in ~/Dev/wubble-wt/feat/recently-closed-full/w.

**Core (wubble-core):**
- `crates/wubble-core/src/closed.rs`: a list entry is now `Entry::{Tab, Window, Folder, Space}`. Window, folder and Space entries keep their tabs (address, title, container, and whether each was pinned, top-level or in a folder). Pure builders make entries from the model (`window_of`, `space_of`, `folder_of`); agent Spaces and peeks are left out. Entries also give each row its title, note, search match and favicon address.
- The undo machine names an entry by key plus close time. It gained `Event::Take(id)` (a row reopened), `Event::Remove` (a row deleted) and the effects `Take`, `Delete`, `RestoreWindow`, `RestoreFolder` and `RestoreSpace`. Its existing tests were adapted to the new types; no new tests.
- New model commands in `crates/wubble-core/src/model/reopen.rs`:
  - `ReopenWindow` opens a Main window on its Space at its old geometry. If the Space is gone it is rebuilt from the entry; if another window already shows it, that window is focused.
  - `ReopenFolder` puts the folder back last in its Space (the active Space if that one is gone) and shows its first tab.
  - `ReopenSpace` rebuilds the Space and switches to it.
  - Rebuilt tabs come back asleep, and only the shown one loads, as Chrome's lazy restore does.
- `CloseFolder` closes a folder's tabs and removes the folder. The folder menu's Close row now uses it, and its existing tabmenu test assertion was updated.

**Store (wubble-store):**
- Migration 17 (`sql/017_recently_closed_entries.sql`) adds `kind`, name, tint, icon and window size/position columns to `recently_closed`, plus a new `recently_closed_tabs` table (deleted with its entry). Existing rows read as tab entries.
- `push_closed(&Entry)`, `recently_closed()` (entries with their row ids), `pop_closed()`, and new `take_closed(id)` and `delete_closed(id)`. The 25-entry cap is unchanged.
- Deleting history by address, site or time range also reaches grouped entries' tabs, and drops a grouped entry left with no tabs. Favicon pruning counts those tabs.

**Shell (crates/wubble):**
- `host.rs` `dispatch` records one entry when a Main window (not the last), a folder or a Space closes as a whole, and its tabs get no entries of their own. The window's current geometry is captured just before it closes.
- Mod+Shift+T and Mod+Z reopen any kind of entry; tab reopen still goes through the old path.
- `persist.rs` has the new store requests (`TakeClosed`, `DeleteClosed`, `ListClosed`, which returns icons too).
- `host/recent.rs` caches the list, lists it again after any change, and draws the rows.
- History page: a "Recently closed" block above the visits, built only from existing kit parts (ListGroup, ListCard, ListRow, RowAction). Each row reopens its entry or deletes it, and the page's search filters it. New Slint types `ClosedLineItem` and `ClosedKind`, mapped by `wubble_ui::pages::closed_items`.
- Command bar (`bar.rs` `with_closed`): on an empty query it lists all entries newest first; on a typed query it adds up to 3 matches. It is off in the "open beside" bar. Picking a row reopens that entry, and the rows show favicons.

**Left out:**
- Deleting a single row from the command bar: the bar has no per-row delete or key yet (omni-inline-autocomplete adds Shift+Delete), so rows are deleted from the history page only.
- Per-tab back/forward history in entries: nothing captures it yet.
- Private tabs: wubble has no private tabs or windows yet; agent tabs and agent Spaces are kept off the list.
- No new setting, no engine change.
- I took no screenshots of the new history block or bar rows (the brief forbade runs), so neither has been checked by eye against the design.

### Notes for the review

**TODO.md lines I would have added:**
- Recently closed: a per-row delete in the command bar, once the bar has Shift+Delete (omni-inline-autocomplete).
- Recently closed: keep each tab's back/forward history in entries once navigation history is captured.
- Recently closed: keep private tabs off the list once private-windows lands (the check goes in `Host::keeps_closed`).
- Recently closed: shoot the history page's "Recently closed" block and the bar's rows, and compare them with the design.

**docs/decisions.md lines I would have added:**
- The recently-closed list follows Chromium's TabRestoreService. A Main window, folder or Space that closes as a whole is one entry, and its tabs get no entries of their own. Reopening rebuilds it with sleeping tabs, and only the shown tab loads.
- A closed Main window keeps its Space, so its entry reopens a window on that Space. Only when the Space is gone too is it rebuilt from the entry. If another window already shows the Space, that window is focused instead.
- Mod+Z undoes window, folder and Space closes of this session as well as tab closes.

**For review:**
- Migration number 17 may collide with other branches that add a migration; renumber when merging.
- The folder menu's Close now sends `Command::CloseFolder(folder)` instead of `CloseTabs(..)`. Its tabs close and the folder is removed, where before an empty folder was left. The tab-groups-parity lane relies on this.
- `Command::NewWindow` is untouched; restored windows go through the new `Command::ReopenWindow`.
- The new history-page block composes existing kit parts only. No new kit piece, no derived-review entry, and no new kit_lock violations by my reading of its rules.
- No code was copied (Chromium behaviour only), so there is no THIRD_PARTY_NOTICES entry.
- No settings were added and nothing makes a network request.
- I updated existing tests only where the type changes would otherwise not compile: core `closed.rs` tests, store `closed.rs` and `history.rs` tests, `persist.rs` tests, and the tabmenu assertion.

### Compile pass

Compiles now: yes. `cargo check --workspace` passed on round 1 of 4 with no errors. I made no fixes, so there is no "compile fixes" commit; the working tree is clean.

Still failing: nothing.

Audit output (`~/Dev/wubble-wt/audit.sh`): "slice: 3.3 GB of 20 GB; free: 25 GB available; /tmp: 188M used"

Nothing to add to TODO.md or docs/decisions.md beyond the feature author's report.

## `search-engines-keywords` (`feat/search-engines-keywords`): written, 2 commits, 25 files changed, 1830 insertions(+), 186 deletions(-)

### Written

I wrote the whole feature to the spec on feat/search-engines-keywords (worktree ~/Dev/wubble-wt/feat/search-engines-keywords/w) in 2 commits. Nothing was built, checked or tested, as the brief ordered, so compile errors are possible. The second commit is needed before the workspace compiles: commit c3b5f55 removes SearchEngine, which crates/wubble still uses until bfc776c.

What it does:
- **Built-ins:** `omnibox::ENGINES` now has 9 engines. Bing, Brave Search, Ecosia, Startpage, Qwant, Kagi and Wikipedia are new, and the shipped ids `google` and `duckduckgo` are unchanged. Each engine has a keyword (its domain, as Chrome does it). The templates come from Chromium's prepopulated_engines.json with Chrome's partner codes left out. Startpage, Kagi and Wikipedia come from each site's own OpenSearch file instead; I did not confirm that Chromium lists them.
- **SearchEngine enum gone:** a new pure module `crates/wubble-core/src/search.rs` replaces it.
  - `EngineRef` and `Engines::resolve`/`from_stored` give one owner for the default and the active and found engines. The default can be any built-in or custom engine, stored as `custom:<keyword>` for a custom one.
  - A pure `check`/`check_template` validates a custom engine: exactly one `%s` or `{q}`, http(s) with a host, a one-word unique keyword.
  - Keyword search (`keyword_search`), plus `default_after_edit` and `without_keyword`.
  - The discovery machine (`Discovery`, `transition`, `DiscoveryEvent`/`DiscoveryEffect`), a small tolerant reader for the OpenSearch file (`parse_osdd`), and `generate_keyword` (page host without www.).
  - Unit tests cover the template checks, engine resolution, keywords, the file reader and the discovery machine.
- **New settings:**
  - `search.custom`: a list in General, columns Name/Keyword/Address.
  - `search.discovered`: a list in General, shown remove-only, inactive.
  - `search.discover_sites`: a toggle in Privacy, off by default.
  - All three are added to the store's typed table in the same order. The store codec for the default is now `Option<String>`.
- **Saving edits:** `apply::put_entry` checks engine entries whole, with a new refusal `Refusal::Engine(Problem)`. The changes are applied through `Applied::SearchEngines(Engines)` and `Applied::DiscoverSites(bool)`.
- **Address bar:** `omnibox::rank` takes `&Engines`. Typing `keyword query` gives that engine's results row, and suggestions in that mode come from that engine. Before the first-run choice, the bar offers all 9 engines, and keyword rows still work.
- **Suggestions:** `SuggestSettings` now holds `engines`. Live suggestions stay behind `suggest.live` and are only fetched for an engine that has a suggest URL; with no default chosen, nothing is fetched.
- **Other callers moved to the resolved engines:** bar.rs, menu.rs (`NavFlags.search` is now `Option<String>`, no longer Copy), the menu's "Search … for", dropped text, and the settings smoke. The start page capsule is the bar's field and needed no separate change.
- **Settings page:** the engine Select lists the built-in and custom engines. The read-only built-in list sits under it, using the kit ListEditor with removes off. There is a custom list editor and a remove-only found list. In the shell layout this is two new `SettingItem` fields, `fixed` and `kept`, bound to the existing ListEditor `adds`/`editable`/`removes` properties; no new kit part.
- **Edits that move the default:** in the host, editing the keyword of the default custom engine moves the default with it. Removing it resets the default to "ask on first search". Adding a custom engine drops a found engine with the same keyword.
- **Discovery (`crates/wubble/src/host/site_search.rs`):** only when `search.discover_sites` is on. After a page loads, a page script (`pagescripts::OPENSEARCH_LINK`) reads `<link rel=search type=application/opensearchdescription+xml>`. The file is fetched with `wubble_engine::fetch::Fetcher` in the tab's session, without cookies, capped at 64 KiB, and stopped when the setting is turned off or at shutdown. The engine found is added to `search.discovered`. Found engines are never made the default and never searched by keyword.
- **Tests and notices:** existing tests were updated to the new API and engine count (omnibox, apply, settings, pages, suggest, store, wubble-ui settings/settings_select, bar, menu, suggest_fx, settings_smoke). THIRD_PARTY_NOTICES.md has a new row.

Left out:
- Editing a custom engine in the list editor drops any suggest URL it had, because the editor has only 3 cells. Custom engines typed by hand never had one.
- Found engines have no one-click "activate". The user adds one to their own engines, and that removes it from the found list.
- Discovery does not skip agent or ephemeral containers.
- Keyword mode needs keyword plus space plus a query; there is no Tab-to-search.

### Notes for the review

TODO.md lines I would have added:
- Search engines: a one-click "Use" on a found site search (copy it to search.custom), as Chrome's inactive-shortcut Activate does.
- Search engines: keep a custom engine's suggestions URL when it is edited in the 3-column list editor.
- Search engines: Tab-to-search hint in the bar once a typed word matches a keyword; keyword prefix matching (Chrome's "bin" then Tab).
- Search engines: shoot the Settings General section (Select plus the built-in list under it, custom and found lists) and put the built-in list's layout on docs/fidelity/derived-review.md if the owner wants it reviewed. It composes the kit ListEditor only.
- Site search discovery: skip agent and ephemeral containers.

docs/decisions.md lines I would have added:
- Search engines are data. Built-ins are in omnibox::ENGINES (ids never renamed). The user's own are search.custom entries `name | keyword | address` (one `%s` or `{q}`, http(s), unique keyword). The default is any active engine, stored as its id or `custom:<keyword>`. search::Engines, resolved from the settings, is the one owner. The SearchEngine enum is gone.
- Built-in keywords are the engine's domain (Chrome's), and `keyword query` searches that engine.
- OpenSearch discovery is off by default (search.discover_sites). Found engines are inactive (search.discovered), never the default, never searched by keyword. The file is fetched without cookies, once per run per file, and only for a keyword no engine has.

Settings added: search.custom (List, General, "[]"), search.discovered (List, General, "[]", shown remove-only), search.discover_sites (Bool, Privacy, "0").

For the owner to review:
- Nothing was compiled or run.
- The engine row now draws a fixed ListEditor under its Select, through the new SettingItem.fixed and SettingItem.kept fields in model.slint and settings.slint.
- The settings smoke and the settings_select test now expect Down from DuckDuckGo to land on Bing (FINAL_ENGINE = "bing").
- The wubble-ui settings scenario test now expects 3 `SettingRow::entries`.
- Live suggestions with a keyword query go to that keyword's engine even before a default is chosen, but only when suggest.live is on.

### Compile pass

feat/search-engines-keywords compiles now: `cargo.sh check --workspace` finished clean in 51.7 s on round 1 of 4, so no errors remain. I made no fixes and no commit ("feat/search-engines-keywords: compile fixes" was not created), and the worktree is clean. Available memory was 25 GB before the run.

Audit output: `slice: 3.2 GB of 20 GB; free: 24 GB available; /tmp: 188M used`

For TODO.md and docs/decisions.md I have nothing beyond the feature author's report.

## `site-sound-tab-menu` (`feat/site-sound-tab-menu`): written, 1 commits, 21 files changed, 544 insertions(+), 38 deletions(-)

### Written

I wrote all three parts of the spec on branch feat/site-sound-tab-menu (worktree ~/Dev/wubble-wt/feat/site-sound-tab-menu/w). It is one commit, a620774, on top of the core work an earlier attempt had left uncommitted (that work is now in the same commit). Nothing was built, checked or tested, as the brief ordered.

1) Mute Site
- crates/wubble-core/src/sitesound.rs holds the logic. A tab's mute is one enum (`Mute`: Unmuted, ByUser, BySite) with one pure `transition`, ported as rules from Chromium's `SoundContentSettingObserver`: a site rule never undoes the user's own mute, and an allowed site unmutes only a tab the rule muted. The file also has `is_muted`, `toggle` (take off the entry covering the site, or add the host), `web_host`, `diff` and `origins`.
- The setting is `sound.muted_sites` in settings.rs, in Section::Privacy, the same shape as `sleep.never_sites`. It is also in `SITE_LISTS`, in `apply::Applied::MutedSites` and in the store's `SoundMutedSites`.
- The engine gets `View::set_sound_blocked(urls, blocked)` (crates/wubble-engine/src/view.rs). It calls `CefRequestContext::SetContentSetting(SOUND, BLOCK or DEFAULT)` on the view's context, which is its container's profile. Muting itself uses the existing `set_muted` (SetAudioMuted).
- The host (crates/wubble/src/host/surfaces.rs):
  - `restore_site_sound` runs on every commit, next to `restore_zoom`: it writes the site's content setting, then runs `MuteEvent::Site`.
  - `set_muted_sites` runs when the setting changes: it writes the added and removed sites through every live view and re-applies every committed page.
  - `toggle_mute_site` is the menu row; it writes the stored setting, so the change shows up in Settings > Privacy.
  - `toggle_mute`, the sound badge and the mute key now go through `MuteEvent::Toggled`. When the view reports a mute that came from elsewhere, the stored reason is brought back in line with it.
- `Audio` gains `why: Mute`. `TabRow` gains `site_muted`, and `tabmarks::Marks::of` takes it, so a sleeping tab of a muted site shows the muted badge.

2) New Tab Below (Chrome's "New tab to the right")
- New `tabmenu::Row::NewTabBelow` at the top of the menu, then a divider.
- It runs a new `model::Command::NewTabBelow { tab, url }`. The start tab goes at the context tab's index + 1, inside its folder if it has one; under a pinned tab it goes first among the unpinned tabs. It uses the Space's default container, has no opener, and is shown.

3) Open Link in New Window
- A new link-menu row in crates/wubble/src/menu.rs runs `menu::Command::OpenInNewWindow`, which becomes the new `model::Command::OpenInNewWindow { url, opener }` (model/windows.rs). That opens a new Main window on a free or new Space; the tab keeps the opener's container, as a link does.
- `external_target` in host.rs also covers the new command, so links another app handles still go there.
- The doc comment that said this row was left out for "one window" is updated.

Other changes:
- `tabmenu::Facts` gains `site_muted`; all its call sites are updated.
- The menu tests and the tabmenu tests are updated to the new rows. I added no new tests beyond those updates and the sitesound tests the earlier attempt had written.
- New THIRD_PARTY_NOTICES.md row: Chromium behaviour, no code copied, BSD-3-Clause.
- No .slint changes. The "plus" and "mute" icons are existing kit icons.

Left out:
- "Open Link in Private Window": private windows have not landed.
- Chrome's plural "Mute sites" when several tabs are selected: wubble has no multi-select.

### Notes for the review

TODO.md lines I would have added:
- Open Link in Private Window (link menu): add once private windows land; menu.rs ROWS doc names it as left out.
- Site mute: `restore_site_sound` runs on every UrlChanged, same-document navigations included. Chrome skips those (`!IsSameDocument()`), so a muted site's tab the user unmuted is muted again after a pushState or fragment change. Fix: feed only cross-document commits.
- Site mute: when a site is removed while a container has no open view, that container's stale SOUND=BLOCK exception stays until the site's next commit there (`restore_site_sound` rewrites it to DEFAULT).

docs/decisions.md lines I would have added:
- `sound.muted_sites` (wubble's setting) is the source of truth for Mute Site. CEF's SOUND content setting is written to match it on each commit and on each list change, so Chromium's own observer agrees; the host also mutes open pages directly with SetAudioMuted.
- A tab's mute reason (`sitesound::Mute`: ByUser or BySite) lives with the view's sound state (`shell::Audio.why`) and goes when the view goes. A site rule never undoes the user's own mute, as in Chrome.
- The tab menu row is named "New Tab Below", not Chrome's "New Tab to the Right", because the sidebar is vertical.

Settings added: `sound.muted_sites` (List, default [], Section::Privacy, label "Muted sites", no restart).

For review:
- The work was never compiled. The places most likely to fail are:
  - the `if let … && let …` chain in `restore_site_sound`;
  - the `cef::ContentSettingTypes::SOUND` and `ContentSettingValues::BLOCK/DEFAULT` constants and the `ImplRequestContext::set_content_setting` call in `view.rs` (checked against the cef-156.6.0 bindings);
  - `open_in_new_window` in `model/windows.rs` calling the private `place_new_tab` from its parent module.
- `Model::NewTabBelow` and `OpenInNewWindow` have no new model tests: the brief said no tests.
- Nothing makes a network request.

### Compile pass

Compiles: yes. `cargo.sh check --workspace` finished cleanly in 1 round (1m 13s), with no errors. I made no fixes and no commit, because the working tree is clean and there was nothing to commit. The branch stays at a620774.

Still failing: nothing.

Audit output (~/Dev/wubble-wt/audit.sh):
slice: 15.2 GB of 20 GB; free: 24 GB available; /tmp: 186M used

Free memory before the check was 25 GB available.

For TODO.md and decisions.md, nothing to add from the compile step.

## `startup-home-settings` (`feat/startup-home-settings`): written, 3 commits, 20 files changed, 757 insertions(+), 24 deletions(-)

### Written

I wrote the whole feature on branch feat/startup-home-settings in three commits (worktree ~/Dev/wubble-wt/feat/startup-home-settings/w). Nothing was built, checked or run, as the brief ordered, so it is unverified against the compiler.

**Commit 04e8c18: core, store and UI**
- crates/wubble-core/src/settings.rs: five new setting rows.
  - `startup.mode`: Last session (default) / Start page / Specific pages, labelled "When wubble opens".
  - `startup.pages`: a list editor of addresses ("Address" column). It only shows while Specific pages is picked; the `SHOWN_WHEN` table drives this, applied in pages.rs.
  - `home.url`: text; empty means the start page.
  - `quit.warn`: on by default only on macOS.
  - `session.exited_cleanly`: hidden.
  - A `SELECT_ROWS` table with `select_labels()` makes startup.mode a Select with proper labels in wubble-ui/src/settings.rs. Settings search finds those labels.
- crates/wubble-core/src/apply.rs: the startup.pages list editor stores each entry as the address it loads, and refuses anything that isn't an address (new `Refusal::NotAPage`).
- crates/wubble-core/src/startup.rs (new):
  - `StartupMode`.
  - `open(model, mode, pages, opening)`, a pure function. It keeps Spaces, pinned tabs and folders with their tabs, closes the loose tabs, and opens a start tab in each Main window. With Specific pages, the focused window gets those pages instead, the first one shown. A URL given on the command line takes the focused window.
  - The Restore pages offer: `enum Offer { Idle, Asking, Kept }` plus a pure `transition`.
  - `home_url()`.
- crates/wubble-core/src/quit.rs (new): `QuitGuard` with a pure `transition`. A second press within 2 s quits, and so does holding the key (key repeats count as presses). Presses closer than 100 ms count as one press.
- New actions `Action::GoHome` and `Action::OpenLastSession`; GoHome is bound to Alt+Home (Linux/Windows) and Cmd+Shift+H (macOS). Palette rows "Home" and "Open pages from last session".
- crates/wubble-store/src/settings.rs: typed settings for the new rows, plus `Store::begin_run` (reads the flag, then marks the run unclean) and `Store::end_run` (marks it clean).
- I updated three existing tests so they match the new rows and actions (action counts 86 to 88, the unbound-keys list, the Select rows in the settings UI tests). I added no tests.

**Commit 0c70fc2: engine (macOS only)**
- crates/wubble-engine/src/mac_app.rs and app.rs: new `set_quit_key_handler`. A quit that arrives while a key press is the current event runs this handler, so Cmd+Q can ask for a second press. The menu's Quit, the Dock and logout still quit at once, which is how Chrome decides.

**Commit c28e886: wiring**
- persist.rs: the clean-exit flag is read and lowered at load, and raised on the store's orderly shutdown. `Loaded` gained `clean_exit` and `startup`.
- main.rs: in normal browsing runs only (smokes keep their sessions), `host::open_session` applies the startup mode before the host gets the model. The tabs it leaves out are pushed onto the recently-closed list.
- After the windows open, `Host::startup_opened` shows the existing kit toast (`toast_with`, sticky) "Restore pages? wubble did not quit properly" with a Restore button. Closing it, or another notice replacing it, leaves the command-bar row as the way back. Restore reopens the tabs in their Spaces and containers, shows the one that was active, and closes the start tabs still on the start page.
- host/startup.rs (new): Home loads the home page in the visible tab, or opens a new tab when there is none. `Plan::Quit` now goes through the guard and shows "Press <chord> again or hold it to quit".

**Left out**
- No new widget: the banner is the existing kit toast and the settings rows use existing controls, so I did not pull the design repo and took no screenshots.
- No Windows-specific code was needed; everything runs through shared paths.

### Notes for the review

**Lines I would have added to TODO.md**
- startup-home-settings: run `cargo check --workspace`, then try each mode by hand: Start page, Specific pages, and a killed run showing "Restore pages?".
- macOS: check that Cmd+Q with quit.warn on warns first and quits on a second press or a hold, and that the Dock's Quit still quits at once.
- Restoring pages closes the start tabs it opened, and those go onto the recently-closed list as "Start" entries. They should be skipped there.

**Lines I would have added to docs/decisions.md**
- Startup modes follow Chrome's "On startup". Start page and Specific pages keep Spaces, pinned tabs and folders with their tabs. Only the loose tabs are dropped, onto recently closed and into the Restore offer.
- "Restore pages?" uses the kit toast with a button, not a new banner widget. It shows only after an unclean exit and only when the mode is not Last session; otherwise the command-bar row "Open pages from last session" is the way back.
- Clean exit is the `session.exited_cleanly` row: lowered at load, raised when the store shuts down in order (like Chrome's exited_cleanly pref).
- quit.warn guards only the quit keys. On macOS a `terminate:` arriving during a key press counts as the keys; the menu, the Dock and logout quit at once.

**Settings added**
- `startup.mode` (choice, default last-session)
- `startup.pages` (list, default [])
- `home.url` (text, default empty)
- `quit.warn` (bool, default 1 on macOS, 0 elsewhere)
- `session.exited_cleanly` (bool, hidden, default 1)

**For the owner's review**
- Nothing was built, so expect possible compile errors. The riskiest spots:
  - the `SELECT_ROWS` const coercion and `select_labels` lifetimes in settings.rs;
  - the `row_text` label lookup in pages.rs;
  - the `currentEvent()` / `r#type()` calls inside `terminate:` in mac_app.rs (copied from how wl/macos.rs uses them).
- The startup modes only apply in normal browsing runs; smokes are untouched.
- To place pages in their Spaces, both the startup step and Restore reuse `Command::ExtensionTab { space, container, show }` (shown only when asked).
- Tests: I wrote none. I edited three existing ones so they don't break on the new rows and actions: action counts 86 to 88, the unbound-keys list, and the Select rows in wubble-ui's settings tests.

### Compile pass

The branch compiles now. `cargo check --workspace` finished with no errors on round 4. It took 4 check rounds, the maximum.

- **Rounds 1 to 3:** wubble-ui failed with two E0425 errors, "cannot find function `select_labels` in module `settings`", at `crates/wubble-ui/src/settings.rs` lines 33 and 90. The function exists in `crates/wubble-core/src/settings.rs`, and `pages.rs` in wubble-core already calls it. The shared target directory held a stale wubble-core artifact from a different branch, whose worktree is also named `w`.
- **Round 4:** I ran `touch crates/wubble-core/src/*.rs`, which forced wubble-core to rebuild. After that every crate checked cleanly.
- **Code changes:** I made no source fixes. I tried one edit to `settings.rs`, and it was reverted with `git checkout`. I made no "feat/startup-home-settings: compile fixes" commit because there is nothing to commit, and `git status` is clean.
- **Still failing:** nothing.
- **Cargo wrapper hazard:** `cargo.sh` names the target directory `.targets/<basename of the worktree root>`. Every feature worktree is called `w`, so all branches share `.targets/w`. That can cause stale-artifact errors like this one in any branch. Touching the wubble-core sources before a check avoids it. The better fix is keying the directory on the branch name. That is a wrapper change, so I didn't make it.

Audit output:
```
slice: 2.7 GB of 20 GB; free: 24 GB available; /tmp: 188M used
```

I have nothing to add for TODO.md or docs/decisions.md.

## `tab-groups-parity` (`feat/tab-groups-parity`): partial, 6 commits, 29 files changed, 856 insertions(+), 149 deletions(-)

### Written

I wrote tab-groups-parity on branch feat/tab-groups-parity in 5 commits (worktree ~/Dev/wubble-wt/feat/tab-groups-parity/w). As ordered, nothing was built, checked or run. I checked the types, fields and visibility I used against the source by hand. No new tests were written; I edited two existing assertions to match the changed table and menu.

What it does:
1. **Folder colour.** `space::Folder` now has `tint: Option<Rgb>`, picked from the Space swatches (`space::swatches()`; an earlier attempt's duplicate `folder_tints` is removed). There is a new `Command::SetFolderTint`. Every `Folder` literal now sets the field (model.rs, store session.rs and import.rs, extensions/projection.rs). The colour is saved: wubble-store/sql/017_folder_tints.sql adds `tab_folders.tint` as `#rrggbb` (NULL means no colour), and migrate.rs registers it as version 17. session.rs reads and writes it in `write_folders`.
2. **New tab in folder.** `Command::NewTabInFolder { folder, url }` puts a tab last in the folder, opens a collapsed folder, and shows the tab. The folder menu and the action pass the start page address.
3. **Move folder.** `Command::MoveFolder { folder, to: FolderTo::Space(id) | FolderTo::NewWindow }` is one command. For a Space it runs `move_tab` on each tab, then gathers them back into the same folder at the end of the target Space, in order. For a new window it reuses `move_tab_to_window(first, WindowTarget::New)`, moves the rest into that window's Space, and puts the first tab back at the head of the folder.
4. **Reopen a saved folder.** `Command::OpenFolder { name, tint, urls }` makes a new folder in the active Space with those tabs and shows the first.
5. **Folder menu** (wubble-core/src/tabmenu.rs `FOLDER_ROWS`, order taken from Chromium's tab group editor):
   - Rename, New Tab in Folder, Collapse
   - Colour rows: "No Colour" and one per swatch, with the current one checked
   - "Move Folder to <Space>" for each other non-agent Space, then "Move Folder to New Window"
   - A slot for the host's saved-folder entries (`Run::SavedFolderEntries`)
   - Ungroup, Close
   - The tab menu's existing `Row::Folders` rows are unchanged.
   - crates/wubble/src/menu.rs `folder_entries` now takes the saved folders and fills the slot: "Save Folder", or "Update Saved Folder" and "Forget Saved Folder" when one of that name exists, then "Open Saved Folder <name>" for each saved folder the Space has no folder of. Dividers are disabled and never doubled or at the ends. `tab_entries` skips the new slot.
6. **Save folder.** wubble-core pages.rs adds `BookmarkEdit::SaveFolder`, `SAVED_FOLDERS` ("Saved Folders") and `saved_folders(rows)`. wubble-store bookmarks.rs adds `save_tab_folder`: in one transaction it creates the top-level Saved Folders folder if missing and replaces a saved folder of the same name. In the host: `saved_folders`, `save_folder` (web tabs only) and `forget_saved_folder` (host/pages.rs), plus new menu commands `SaveFolder` and `ForgetSavedFolder` (menu.rs, surfaces.rs).
7. **Command bar.** bar.rs adds `RowTarget::SavedFolder`, `saved_folder_rows` (matches on the folder name) and `with_saved_folders`, which puts these rows after the first row. `render_bar` in host.rs uses them.
8. **Keyboard.** Four new actions, in the command bar, with no default keys (Chromium binds none for groups): add-tab-to-new-folder, new-tab-in-folder, toggle-folder, close-folder. They are added to `FIXED` (now 72), `PALETTE` (now 35), the unbound list in keys.rs, and perform.rs. `Model::folder_of(tab)` is new.
9. **UI.**
   - The folder header drawing moved into a new kit component, kit/chrome/folder-row.slint `FolderHeader` (a `derived:` file). components/folder-row.slint is now just `FolderRow inherits FolderHeader`, and its 9 baseline lines are removed from kit_lock_baseline.txt (the baseline only shrank).
   - A coloured folder shows its chevron in the colour plus a dot after the count.
   - Each tab in a coloured folder gets a colour rail in the indent before it (a "wubble change" in kit/chrome/tab-row.slint).
   - `TabItem` gains `tinted` and `tint`; shell.rs `TabRow.tint` feeds them through bind.rs.
   - New Shell values `folder-rail`, `folder-rail-inset` and `folder-dot` sit in derived.slint's derived block, built from existing tokens, so there are no new literals.
   - docs/fidelity/derived-review.md has a new "Folder colours" section, and THIRD_PARTY_NOTICES.md a Chromium entry (behaviour only, no code copied).

Left out:
- **Closed folder on the recently-closed list.** That needs the store's entry enum, which the recently-closed-full lane is building in the same files. Close Folder still closes its tabs one by one, each going on the list as today.
- **Saved folder colour.** A bookmark folder has nowhere to keep it, so a reopened folder has no colour.
- **Swatch circles in the menu.** `MenuEntry` cannot draw a swatch, so the colours are text rows with a check mark. Real swatch circles would need a menu swatch part, which waits for the owner.
- **The design source.** The design clone is not on this machine; the folder widget has no design source anyway ("written new"), and this is noted in the review sheet.
- **Shots.** None were taken, since nothing could be run.
- **Some menu icons.** Several icon names ("plus", "bookmark", "folder", "chevron-down") are not in `Glyph.named`, so those rows draw no icon, as before.

### Notes for the review

For TODO.md (I would have added):
- Folders: a closed folder as one entry on the recently-closed list (after recently-closed-full lands its entry enum; FolderRow::Close then records it).
- Folders: keep a saved folder's colour (the bookmark has no field for it; reopened folders come back with no colour).
- Folder menu: real swatch circles instead of text colour rows (needs a swatch part in the kit menu, for the owner).
- Unit tests for the new commands (SetFolderTint, NewTabInFolder, MoveFolder to a Space and to a new window, OpenFolder) in model/tests.rs; tabmenu tests for the new folder rows; tests for pages::saved_folders and Store::save_tab_folder; bar::saved_folder_rows. None were written, per the order.
- Shoot the sidebar with a coloured folder (gallery scene to add) and compare it by eye.
- Folder collapse is still not saved with the session (Chrome keeps it).

For docs/decisions.md:
- Folder colours are the Space swatches (kit_gen::spaces::SWATCHES, 6 colours), not Chrome's 9 group colours: one palette in the shell.
- Saved folders are bookmark folders under a top-level "Saved Folders" folder; saving one of the same name replaces it; they reopen from the folder menu and the command bar.
- MoveFolder to a new window reuses MoveTabToWindow(New): the window shows the first free Space, or a new one, as a torn-out tab does.
- Folder actions (add-tab-to-new-folder, new-tab-in-folder, toggle-folder, close-folder) have no default keys, as in Chromium.

Settings: none added. Nothing makes a network request.

Schema: migration 17 (017_folder_tints.sql) adds tab_folders.tint TEXT. Other lanes in this batch may also take 017, so it may need renumbering at merge.

Likely merge conflicts: action.rs FIXED and PALETTE counts and keys.rs unbound list (chrome-shortcut-gaps); bar.rs and host.rs render_bar (omni-inline-autocomplete, tab-search, recently-closed-full); migrate.rs.

For the owner's review: the moved kit component kit/chrome/folder-row.slint, the TabRow rail change in kit/chrome/tab-row.slint, the three new Shell values in derived.slint, and the derived-review.md section "Folder colours". No shots were taken, because nothing could be run.

### Compile pass

feat/tab-groups-parity compiles: `cargo.sh check --workspace` finishes with no errors. It took 3 check rounds. The rounds were:
- Round 1 stopped on a Slint error.
- Round 2 showed 19 Rust errors. They came from a stale wubble-core build, not from the code.
- Round 3 (after the touch) was clean, and a repeat run finished in 0.24s.

Fixes made, committed as 28bd38c "feat/tab-groups-parity: compile fixes":
1. **Slint error.** In `crates/wubble-ui/ui/kit/derived.slint`, `folder-rail` read `BrowserCss.wb-focus-visible--outline-width`. That property lives in the `WubbleCss` global in `gen/css.slint`. I added `WubbleCss` to the import on line 9 and switched `folder-rail` to it.
2. **Stale build.** The 19 errors were all in the `wubble` crate: missing `SavedFolder`, `Action::NewTabInFolder`, `Model::folder_of`, `Folder.tint`, and a `prevent_inline` field that appears nowhere in this source. The sources already contain all the missing items, so cargo was reusing a wubble-core artifact built from another branch. `cargo.sh` names the target dir `.targets/<basename of the worktree>`, so every feature worktree called `w` shares `.targets/w`. Cargo's workspace-member hashes don't include the absolute path, so builds from different worktrees collide in that directory. I ran `touch` on the `.rs`, `Cargo.toml` and `build.rs` files under `crates/` in this worktree, which forced a rebuild, and the errors disappeared. No source change was needed for this.

Other lanes are probably hitting the same problem. If any other worktree named `w` shows impossible "missing item" errors after a compile fix, `touch` its sources. A proper fix would be to put a unique per-worktree name in the `CARGO_TARGET_DIR` that `cargo.sh` builds.

Audit output (`~/Dev/wubble-wt/audit.sh`):
slice: 3.6 GB of 20 GB; free: 24 GB available; /tmp: 186M used

For TODO.md and docs/decisions.md, I have nothing beyond the author's report. I touched neither file.

## `tab-search` (`feat/tab-search`): written, 2 commits, 21 files changed, 557 insertions(+), 52 deletions(-)

### Written

Tab search is written and committed on feat/tab-search in two commits (d0de957 and d23530f) in /home/xuckl/Dev/wubble-wt/feat/tab-search/w. Nothing was built, checked or run, as the brief ordered, so it has not been compiled.

What it does: Mod+Shift+A (Action::SearchTabs, palette title "Search tabs", id "search-tabs", Shell route, all platforms) opens the command bar in a new tabs mode. With an empty query it lists every open tab: the active Space first, then by last time on screen, and a tab on screen counts as now. Each row shows its Space name and site. The recently closed tabs follow, newest first, each reading "Recently closed · site" with a clock glyph. Typing filters both with the omnibox matcher; the Space name counts as the tab's group. Agent Spaces, agent-driven tabs and peeks are left out. Tab search asks for no history and no engine suggestions.

Choosing a row:
- An open tab: Command::ActivateTab, which already switches Space or focuses the other window.
- A closed entry: that exact entry is taken off the store's list and restored at its place if this run closed it, through the new closed::Event::Taken.

An open tab's row ends in the list row's close button; Shift+Delete (or Delete in an empty field) closes the selected one. The bar stays open, the closed list is fetched again and the rows re-render. In this mode the 7-row list slides with the selection so every row is reachable.

Main files:
- crates/wubble-core/src/omnibox.rs: one pure function `tab_search` with types SearchTab, SearchClosed and RECENTLY_CLOSED. It reuses `query_terms` (taken out of `rank` by the earlier attempt) and `match_class`.
- crates/wubble-core/src/action.rs, keys.rs, chrome.rs (CommandMode::Tabs, Placement::for_mode: tab search is always the overlay), closed.rs (Event::Taken).
- crates/wubble-store/src/closed.rs (take_closed) and crates/wubble/src/persist.rs (ListClosed and TakeClosed requests, ClosedList).
- crates/wubble/src/bar.rs (BarMode::Tabs, search_tabs, tab_search_rows, Choice::ReopenClosed), host.rs, main.rs, perform.rs (Plan::SearchTabs), bind.rs.
- Slint: components/model.slint (CommandKind closed-tab, CommandResult.closable, CommandMode tabs) and kit/chrome/command-row, command-list, command-bar and command-head; app.slint (request-tab-search, tab-search-requested, command-result-closed); crates/wubble-ui/src/driver.rs.

Left out:
- New tests, per the owner's no-tests order. I only raised the counts in the existing action tests (86 to 87) and added `closable: false` to the CommandResult literal in crates/wubble-ui/src/lib.rs so the existing tests still compile.
- Private tabs showing only in their private window: main has no private windows yet (that is the parallel `private-windows` feature), so nothing filters them.
- Recently closed groups shown as groups: the store keeps tabs only; folders and Spaces come with `recently-closed-full`.
- Section headers: "Recently closed" is the row's hint, not a header row, because the kit list has no header row.
- Screenshots were not taken, since running is forbidden.

### Notes for the review

TODO.md lines I would have added:
- Tab search: hide private tabs outside their private window once `private-windows` lands (filter in bar::search_tabs through its `left_out` argument).
- Tab search: show closed folders and Spaces as groups once `recently-closed-full` stores them.
- Tab search: shoot the tabs-mode bar (rows, close button, clock glyph, scrolled list) and put it on the derived-review sheet for the owner's eye.
- Tab search: the rows do not refresh while the bar is open if tabs change elsewhere (they refresh on typing or on a close from the list).

docs/decisions.md lines I would have added:
- Tab search is a mode of the command bar (CommandMode::Tabs, always the overlay), not a separate popover; its rows come from one pure function, omnibox::tab_search.
- Tab search's Delete is Shift+Delete, or Delete in an empty field (Chromium's omnibox removes a suggestion on Shift+Delete), so plain Delete still edits the query.
- A closed entry is named by its (url, closed_at_ms) pair everywhere: store, closed machine and omnibox Target::ClosedTab.

Settings added: none (no new setting is needed, and tab search makes no network request).

For the owner's review:
- docs/fidelity/derived-review.md has a new "Tab search" row (marked "to shoot").
- Kit changes: CommandRow imports RowAction from kit/pages/list-row.slint; CommandList gains `scrolls` (off by default, so the normal bar and the start page behave as before), `reach`, a sliding `first`, `close-requested` and `close-current`; CommandHead gains `removed` on Delete; CommandBar gains `result-closed` and the "Search tabs" placeholder.
- CommandResult has a new field, `closable`. Any other Rust literal of it outside the ones I found would need `closable: false`; the chrome_kit test uses `..Default::default()`, so it is fine.
- The KeyEvent `modifiers.shift` and `changed current =>` uses in Slint need a compile check, as does the rest, since nothing was built.
- Expect merge conflicts with `recently-closed-full` (both touch the store's recently-closed code and persist.rs), with `chrome-shortcut-gaps` (keys.rs and the action counts) and with `omni-inline-autocomplete` (omnibox.rs and the command bar kit).

### Compile pass

feat/tab-search now compiles: `cargo.sh check --workspace` finished clean on round 2 (2 rounds used).

- Round 1 failed with 3 errors in crates/wubble-ui/src/driver.rs: no `Tabs` variant on `chrome::CommandMode`, and no `Placement::for_mode`. Both exist in crates/wubble-core/src/chrome.rs. The shared target directory had a stale wubble-core build, so wubble-ui compiled against the old one.
- I ran `touch` on crates/wubble-core/src/lib.rs and chrome.rs, which forces a rebuild without changing content. Round 2 passed (`Finished dev profile in 1m 02s`).
- No source changes were needed, so I made no "feat/tab-search: compile fixes" commit. `git status` is clean.
- Nothing is still failing.

Audit output: `slice: 5.7 GB of 20 GB; free: 23 GB available; /tmp: 186M used`

I wrote no TODO.md or decisions.md notes, since nothing changed.

## `tear-off-fidelity` (`feat/tear-off-fidelity`): written, 2 commits, 15 files changed, 665 insertions(+), 159 deletions(-)

### Written

Tab tear-off is written on feat/tear-off-fidelity in ~/Dev/wubble-wt/feat/tear-off-fidelity/w, in 2 commits (11cf56e, 60ea8f5). As ordered, nothing was built, checked or tested, so none of this has been compiled. An earlier attempt had left uncommitted changes in the worktree (WindowTarget::NEW, the Group enum, the MoveGroupToWindow command); I kept them and finished the work on top.

What it does now:
- **New window at the drop point.** A tab dropped outside every window opens a window at the source window's size, moved by how far the pointer travelled since the press. The new window lands where the source would be if it had been dragged, so the grabbed spot stays near the pointer. On Wayland the position is unknown and the compositor places the window.
- **Drop at a row in another window.** Dropping onto another window's tab list puts the tab at the gap the pointer is over. While it hovers there, that window's rows spring apart to show the gap. Dropping elsewhere on that window puts the tab last, as before.
- **Whole folders and splits.** A folder header can now be dragged out, and it carries the whole folder (same name and fold state, tabs in order) to another window or a new one. Dragging the row of a tab that is in a split moves the whole split, with its seams kept.

Main files:
- **crates/wubble-core/src/window.rs**: `WindowTarget` is now `New { geometry }`, `Existing(WindowId)` or `At(Dest)`. `WindowTarget::NEW` means "where the platform puts it". `Geometry::torn_off(offset, scale)` computes the new window's place.
- **crates/wubble-core/src/model/windows.rs**: new helpers `target_window`, `land`, `move_out`, `move_group_to_window`, `item_index` and `nth`. `move_tab_to_window` opens the new window at its geometry and honours a drop place. A group move is refused if the group is unknown, empty, already in the target's Space, or has a tab in a Little or Popup window.
- **crates/wubble-core/src/model.rs**: the `Group` enum (`Folder` or `Split`) and `Command::MoveGroupToWindow`.
- **crates/wubble-core/src/tabdrag.rs**:
  - What is dragged is now `Held::Tab` or `Held::Folder`.
  - The drag state remembers the press point and the current pointer.
  - New drop target: `WindowRow { window, gap }`. `Rects.windows` is now a list of other windows, each with its tab list's rows; `gap()` is the pure hit-test.
  - The effects that replace the old `MoveToWindow { to }` are `TearOff { held, offset }` and `MoveToWindow { held, window, gap }`.
  - A dragged folder drops only onto windows; Spaces and split edges ignore it.
- **crates/wubble-core/src/tabs.rs**: `TabRows::set_gap`, marked as a wubble change. The rows from the gap down move one row lower while a tab from another window hovers.
- **crates/wubble/src/host/tab_drag.rs**:
  - Folder headers start a drag.
  - The other windows are measured by their inner positions, with their tab lists and row counts.
  - Each other window is told whether to show the gap.
  - A drop becomes `MoveTabToWindow`, or `MoveGroupToWindow` for a split or a folder.
  - The ghost shows a folder's name.
- **crates/wubble/src/shell.rs**: the row-walking code was taken out of `move_dest` into `list_rows`, so `move_dest` and the new `list_dest(model, space, tab, gap)` share it.
- **crates/wubble-ui**: in app.slint, the properties `tab-drop-on` and `tab-drop-gap`, with change handlers. driver.rs passes them to `set_gap`. folder-row.slint gets a `track` callback copied from TabRow, wired in sidebar.slint. Nothing new is drawn.
- **Small edits to existing code**: drag.rs and the existing tabdrag tests were changed only so they compile against the new types; menu.rs and perform.rs use `WindowTarget::NEW`.

Left out, and why:
- **No new unit tests.** The spec asked for tests in tabdrag.rs and model/tests.rs, but the brief said not to write any.
- **Wayland.** That is the separate `wayland-tab-drag` item.
- **Dropping into another window's pinned area.** That area is not measured, so a drop there lands last in the normal list.
- **Exact row placement in the new window.** It is placed by the pointer's travel only. If the tab ends up at a different row in the new window, it will not sit exactly under the pointer.
- **Dragging the only tab of a window.** It still opens a new window and leaves the source empty; Chrome would move the window itself.
- **The split pane's content area.** Only the pane's row in the list drags the split, not the pane itself.

### Notes for the review

TODO.md lines I would have added:
- Tear-off: the new window is placed by the pointer's travel only; also offset it by the grabbed row's top in the new window's list (pinned rows and a non-empty free Space shift it).
- Tear-off: dropping on another window's pinned grid lands last in the normal list; measure the pinned area and give it a Dest::Pinned.
- Tear-off: dragging a window's only tab should move the window itself (Chrome), not open a new one.
- Tear-off: unit tests (the brief forbade them) for tabdrag (gap(), WindowRow hits, TearOff offset, a held folder ignoring Space and split drops), Geometry::torn_off, shell::list_dest, TabRows::set_gap, and model MoveTabToWindow At / MoveGroupToWindow for a folder and a split.

docs/decisions.md lines I would have added:
- A tab dropped outside every window opens at the source window's size, moved by the drag's travel (Chrome's tear-off); on Wayland the compositor places it.
- A drop on another window's tab list goes in at the row gap (rounded by half rows); while a tab hovers, that window's rows open the gap (TabRows::set_gap via `tab-drop-on` / `tab-drop-gap`).
- A folder's header drags the whole folder to another window or a new one, as a new folder with the same name and fold; a split pane's row drags the whole split, seams kept. Neither drops into a Space or a split.

Settings added: none.

For review:
- Nothing was compiled; the brief said not to build.
- Kit lock: the new slint lines contain no px, ms or hex literals, no paint and no tokens. The new `track` callback in components/folder-row.slint follows TabRow's pattern. Nothing new is drawn, so nothing goes on derived-review.md.
- No code was copied from elsewhere; Chrome was the behaviour reference only, so no THIRD_PARTY_NOTICES entry.
- No network requests are added.
- No scratch files were created.

### Compile pass

Compiles now: yes. `cargo check --workspace` finished clean on the first round (1 round of 4), with no errors in the output tail.
Fixes: none were needed, so I made no "compile fixes" commit. The worktree is clean, with nothing uncommitted.
Still failing: nothing.
Memory before the check: 20 GB available, above the 8 GB threshold.
Audit output (~/Dev/wubble-wt/audit.sh): "slice: 7.3 GB of 20 GB; free: 24 GB available; /tmp: 186M used"
Report items for TODO.md and docs/decisions.md: none beyond the author's own report.

## `wayland-tab-drag` (`feat/wayland-tab-drag`): written, 2 commits, 9 files changed, 961 insertions(+), 36 deletions(-)

### Written

The wayland-tab-drag feature is written and committed on feat/wayland-tab-drag in ~/Dev/wubble-wt/feat/wayland-tab-drag/w (commits 860e091 core, 1c94475 wayland and host). Per the brief, nothing was built, checked or run, and no tests were written.

What a tab drag does now:
- **Leaving the window.** On Wayland, a drag whose pointer goes outside the window becomes an OS drag. The tabdrag machine gets an `Os` state with these inputs: LeftWindow, OsRefused, Detached, OsDropped and OsEnded.
- **Drag data.** The drag offers a single type, `application/x-wubble-tab`. Its data is `wubble-tab <instance hex> <tab id>`. The instance is a random 64-bit mark made once per process from `RandomState`; it is never stored or sent anywhere. New module: crates/wubble-core/src/tabdnd.rs.
- **The OS drag itself.** The Wayland worker (crates/wubble/src/wl/wayland.rs) starts the `wl_data_source` from the row's press serial. Without xdg_toplevel_drag, the icon is the tab's row, cut from a single `take_snapshot` of the window when the drag starts.
- **Live tear-off.** If the compositor advertises `xdg_toplevel_drag_manager_v1`, the drag object is created before `start_drag`. The host then moves the tab into a new window right away. That window is attached at the grab offset from the Slint thread, either straight after it opens or on its first winit event, before its first frame maps it. The drag object is destroyed once the drop is performed or the drag is cancelled.
- **Hovering and dropping on a wubble window.** Every wubble window receives the drag as a data-device drop. A Main window shows the existing kit drop-mark line at the slot under the pointer. Over the list, that is the nearest gap; elsewhere in the window, the end of the list.
  - On drop, the window reads the data on a thread. It takes the tab only if the data names this process and the tab being dragged: `MoveTabToWindow{Existing}`, then `MoveTab` to `shell::move_dest` at that slot, then the live window is closed if there was one.
  - Offers from another process or profile are refused when they enter, and again if the data check fails.
- **Ending anywhere else.**
  - Without a live window: releasing outside every wubble window opens a new window, as today; Esc leaves the tab where it was.
  - With a live window: it stays wherever the drag ended, Esc included, as the spec asks.
  - If the OS refuses to start the drag, it continues inside the window and a release opens a new window as before.

Files: crates/wubble-core/src/tabdrag.rs, crates/wubble/src/wl/mod.rs (new AuxEvent variants and Aux methods with defaults, so macOS and Null need no changes), crates/wubble/src/host/tab_drag.rs (the host side and the `OsTab` state), crates/wubble/src/host/drag.rs (event routing; `drop_mark` made `pub(super)`; attach hook in `add_aux_window`), crates/wubble/src/host.rs (one new field), THIRD_PARTY_NOTICES.md (one row).

What is not done:
- **The gap is the existing drop-mark line, not a gap opened by the `tabs::TabRows` row physics.** TabRows has no API for a row coming in from outside, and tear-off-fidelity is changing that code. This differs from the spec's "feed motion into the tabdrag machine of the target window".
- **The drop row uses `MoveTabToWindow` followed by `MoveTab`,** because the `Dest` on `MoveTabToWindow` belongs to tear-off-fidelity and is not on main yet.
- **The live window opens at the platform's default size,** not the source window's size, since `WindowTarget::New{geometry}` is also tear-off-fidelity's.
- **Dragging a window's only tab does not drag the whole window** (Chrome does). In wubble an emptied Space shows the start page rather than closing the window.
- **No setting was added;** the feature does not make any network request.
- **X11, macOS and Windows are unchanged:** they already report window positions and never enter the OS state.

### Notes for the review

TODO.md lines I would have added:
- Wayland tab drag: open a gap for an incoming tab in `tabs::TabRows` (rows make room on hover); for now the drop-mark line shows the slot (host/tab_drag.rs `os_tab_over`).
- Wayland live tear-off: open the new window at the source window's size once tear-off-fidelity's `WindowTarget::New{geometry}` lands (host/tab_drag.rs `start_os_tab_drag`).
- After the tear-off-fidelity merge: replace the `MoveTabToWindow` + `MoveTab` pair in `Effect::DropInWindow` with its `Dest`.
- Wayland tab drag: try on GNOME (no toplevel drag, icon road), KWin, Mutter 46+ and Sway (live road), and check that attaching from the first winit event happens before Slint's first frame maps the window.

docs/decisions.md lines I would have added:
- On Wayland a tab drag that leaves its window becomes a wl_data_device drag of `application/x-wubble-tab` carrying a per-process random mark and the TabId; only the same process takes the drop.
- Wayland tab drag endings: Esc with no live window leaves the tab in place; a release over no wubble window opens a new window; under xdg_toplevel_drag the travelling window stays wherever the drag ends, Esc included. A release on the desktop and one on another app cannot be told apart on Wayland (both are a cancel after `dnd_drop_performed`), so both count as "elsewhere".
- The Wayland tab drag icon is the row cut from one `Window::take_snapshot` at drag start: a single readback, never on the frame path.

Settings added: none.

For the owner's review:
- **Merge conflicts.** tabdrag.rs (new `Drag::Os`, Inputs and Effects) and host/tab_drag.rs will conflict with tear-off-fidelity; both branches touch them.
- **Blocking wait.** `start_tab_drag` blocks the Slint thread for up to 500 ms waiting for the worker to answer. This is needed so the toplevel drag exists before the new window is attached. It is a local channel round with no compositor round trip.
- **winit API used.** The live window's xdg_toplevel comes from winit 0.30's `WindowExtWayland::xdg_toplevel()`, wrapped as a foreign proxy the same way the worker already wraps the window's `wl_surface`.
- **New dependency features.** The protocol bindings come from sctk 0.20's re-exported wayland-protocols, which already enables staging, so no Cargo changes were needed.
- **Visual check.** No shots were taken (the brief forbids runs). The only shell visual is the existing drop-mark line and no kit or .slint file changed, so the kit lock is unaffected.

### Compile pass

Compiles now: yes. Rounds: 1 (the first `cargo check --workspace` finished with no errors). Still failing: nothing. No fixes were needed, so I made no "compile fixes" commit. The worktree is clean (`git status` is empty).

Memory was fine before the check: 21 GB available.

Check output: `Finished dev profile [optimized + debuginfo] target(s) in 1m 04s`.

Audit output:
slice: 3.1 GB of 20 GB; free: 23 GB available; /tmp: 188M used

Nothing to add for TODO.md or docs/decisions.md beyond the feature author's report.

## `window-state-restore` (`feat/window-state-restore`): written, 3 commits, 9 files changed, 519 insertions(+), 66 deletions(-)

### Written

The window-state-restore feature is written in 3 commits on branch feat/window-state-restore, in the worktree ~/Dev/wubble-wt/feat/window-state-restore/w. Nothing was built, checked or run, as the brief ordered, so whether it compiles is unknown. Uncommitted work from an earlier attempt (the core and store changes) was already in the worktree. I checked it against the rest of the code and committed it.

1. Core (crates/wubble-core/src/window.rs, model/windows.rs):
   - A new `WindowState` (Normal, Maximized, Fullscreen, Minimized) and `Geometry.state` / `Geometry.monitor`. `Geometry` is no longer `Copy`, and its callers now clone or borrow.
   - `Geometry::observed` keeps the last normal size and position while a window is maximized, fullscreen or minimized, as Chrome keeps restored bounds.
   - `Geometry::restoring` brings the focused window back normal instead of minimized.
   - `Geometry::cascaded` opens Mod+N windows at the main window's size, 10 px down and right, maximized if that window is.
   - `Rect`, `Display`, `display_for` and `visible_on` port Chrome's `WindowSizer::AdjustBoundsToBeVisibleOnDisplay` and `gfx::Rect::AdjustToFit`: at least 30 px of the window stays on a display, never above it, and on macOS the window is fitted inside.
   - `resize_window` stores what `observed` returns.
2. Store (crates/wubble-store/sql/017_window_state.sql, migrate.rs, session.rs):
   - Migration v17 adds the `state`, `monitor` and `tab_id` columns to `windows`.
   - Popup windows are saved as rows carrying their tab. On load, a Popup row whose tab is not placed in its Space, or is already on screen, is dropped and the tab stays an ordinary tab.
   - If no Main window is left after loading, one is inserted below the Popups. Popup tabs come back with a renderer (Active).
   - When a Space's only tabs are in windows, its saved active tab is the first one not in a kept Popup.
   - Little and Pip windows are still not kept, as before.
3. Host (crates/wubble/src/host.rs, main.rs):
   - `window_geometry` reads fullscreen and maximized from Slint, minimized and the monitor name from winit (`current_monitor().name()`).
   - `place_window` sets size, position and maximized or fullscreen before the window shows.
   - A new `settle_window` runs once the native window exists. It moves a normal window that is off every monitor back onto one; this does nothing on Wayland, which reports no positions. It moves a fullscreen window to its saved monitor by name (`Fullscreen::Borderless(Some(m))`) and minimizes a minimized one.
   - `open_session_windows` posts `settle_session_windows` to the event loop. That settles every window and then raises the last focused one, restoring the focus order. Windows opened later are settled right after they show.
   - `record_geometry` compares after `observed`, and `dispatch` runs it before `Command::NewWindow` so the cascade starts from the window's current size.
   - At startup, main.rs opens the last focused Main window first (`main_window()`), so it gets the saved sidebar; any Popup window that was focused is then raised on top.
4. THIRD_PARTY_NOTICES.md has a new row for the Chromium window_sizer.cc / rect.cc / display_finder.cc port.

Left out:
- No new tests, per the owner's no-tests order, although the spec asks for store round-trip and clamp tests. Only existing tests were adjusted so they still compile with the new `Geometry` fields.
- The saved work area is not stored, because winit gives no work area, so Chrome's "resize to the new work area" step is skipped and the clamp uses the full monitor area.
- No settings and no UI were needed.

### Notes for the review

TODO.md lines I would have added:
- Window state restore: unit tests for the store round trip (state, monitor, Popup rows) and for `visible_on` / `display_for` (the spec asks for them; skipped under the no-tests order).
- Window state restore: the clamp uses the full monitor area, not the work area (winit has none); add per-platform work areas if windows land under docks or panels.
- Window state restore: check by hand that Slint's set_maximized / set_fullscreen before show applies on X11, Wayland, macOS and Windows; that the first window's native window exists by the time settle_session_windows runs; and that the Mod+N cascade lands right on every platform.

docs/decisions.md lines I would have added:
- The session keeps each Main and Popup window's restored bounds, show state (normal, maximized, fullscreen, minimized) and monitor name. Little and Pip windows are not kept. (Chrome's GetRestoredBounds and GetRestoredState.)
- The focused window never comes back minimized. The last focused Main window opens first and the last focused window of any kind is raised after the others.
- A window off every monitor is brought back by Chrome's AdjustBoundsToBeVisibleOnDisplay (30 px left visible), after it shows. On macOS it is fitted inside the display. On Wayland only a fullscreen window's output can be chosen.

For the owner's review:
- Settings added: none. Schema migration: v17 (017_window_state.sql). If another branch also adds a v17 migration, one of them has to be renumbered when merging.
- Geometry lost `Copy`; every caller in the workspace was updated: host.rs, main.rs, model/windows.rs, the store session code and existing tests.
- Nothing was built or checked. The likeliest compile risks are the winit 0.30 API calls in host.rs `settle_window` and `window_geometry` (MonitorHandle::name/position/size, request_inner_size, Fullscreen::Borderless) and the let-chain style.
- No scratch files were created.

### Compile pass

Compiles now: yes. `cargo check --workspace` passed on the first round (finished in 41.75s), so I made no fixes and no commit ("feat/window-state-restore: compile fixes" was not needed). Rounds used: 1 of 4. Nothing is still failing. The working tree was clean afterwards.

Memory before the build: 25 GB available.

audit.sh output:
/home/xuckl/Dev/wubble-wt/audit.sh: line 5: /proc/574144/cmdline: No such file or directory
slice: 17.2 GB of 20 GB; free: 24 GB available; /tmp: 186M used

Nothing to add for TODO.md or docs/decisions.md from this step.

