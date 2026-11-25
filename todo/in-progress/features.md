# Features now

Legend: `[ ]` open · `[~]` in progress. Loaded by the owner (2026-11-25): the easiest build-outs, integrated one feature at a time (compile, test, caveman then adversarial review, merge). The first three are written on branches from 11-22 (`feat/site-sound-tab-menu`, `feat/tab-search`, `feat/chrome-shortcut-gaps`): rebase onto main first.

- [~] A pop-up window minimum-size design token (approved by the owner): `Shell.popup-min-width`/`-height`, 100x100 as Blink's least `window.open` size (on derived-review.md); written blind on sprint/features, not compiled

## Mute site, and the Chrome tab and link menu rows still missing (`site-sound-tab-menu`)

Key: `site-sound-tab-menu` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

Chrome's tab context menu offers 'New tab to the right', 'Mute site' or 'Unmute site' (a per-site sound content setting that survives restarts and applies to every tab of the site), 'Move tab to new window', 'Close tabs to the right' and 'Close other tabs'. Its link menu has 'Open link in new window' and 'Open link in incognito window'. wubble's `tabmenu::ROWS` (crates/wubble-core/src/tabmenu.rs) has a per-tab mute (`Run::ToggleMute`), Close others, Close below and the window entries. menu.rs ROWS deliberately leave out 'Open Link in New Window' ('one window'), which is no longer true since Main windows exist. Build: 1) `Row::NewTabBelow`, a start tab placed right under this one (`Command::NewTab` with an index, or a new `Dest`). 2) `Row::MuteSite`, a site sound rule stored like the other per-site settings (a `sound.muted_sites` list in settings.rs, the same shape as `sleep.never_sites`). The host applies it with `CefBrowserHost::SetAudioMuted` on every view of the site on commit, and with CEF's content setting `CEF_CONTENT_SETTING_TYPE_SOUND` through `CefRequestContext::SetContentSetting` per container. Settings > Privacy lists the sites. `tabmarks::Marks` shows muted for them. 3) A link-menu row 'Open Link in New Window' in crates/wubble/src/menu.rs (`Command::NewWindow` with a URL, opening in a new or free Space), and 'Open Link in Private Window' once private-windows lands. Update the menu tests.

## Tab search (Ctrl+Shift+A): all open and recently closed tabs in one list (`tab-search`)

Key: `tab-search` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

Chrome's Tab Search (Ctrl+Shift+A, Cmd+Shift+A) is a popover that lists every open tab of every window, most recent first, with favicons, titles, sites and a close button per row. It fuzzy-filters as you type, Enter switches to the tab and its window, and a 'Recently closed' section follows, with groups shown as groups. wubble's command bar already matches open tabs (crates/wubble/src/bar.rs `open_tabs`, omnibox.rs `Kind::OpenTab`), but only mixed with history and only once you type. Build a tab-search mode of the command bar. `Action::SearchTabs` gets the default chord Mod+Shift+A in keys.rs and the palette title 'Search tabs'. The empty query lists all open tabs across Spaces: the active Space first, then others by `Tab::last_active_ms`, each row showing its Space name. Agent tabs are left out and private tabs show only in their private window. Recently closed entries follow from the store (`Store::recently_closed`, see recently-closed-full). Typing filters with the same matcher omnibox uses, so the rows come from one pure function in wubble-core/src/omnibox.rs with tests. Enter or a click runs `Command::ActivateTab` plus a window focus or Space switch for a tab of another Space (reuse `Target::OpenTab`). A per-row close (kit list row's trailing button, as in the history page rows) closes the tab without leaving the list. Up/Down and Delete work from the keyboard.

## Chrome keyboard shortcuts wubble does not bind yet (`chrome-shortcut-gaps`)

Key: `chrome-shortcut-gaps` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

Compared with Chromium's `chrome/browser/ui/accelerator_table.cc` (already a cited source of keys.rs DEFAULTS), these are missing from crates/wubble-core/src/action.rs `Action`, `FIXED`, `PALETTE` and keys.rs `DEFAULTS`. Ctrl+Shift+PageUp/PageDown moves the tab up or down (`Action::MoveTabUp/Down` via `Command::MoveTab`, staying inside pinned/normal/folder). Cmd+Shift+[ and Cmd+Shift+] select the previous and next tab on macOS; Cmd+Alt+Left/Right stays NextSpace. Ctrl+Shift+Delete (Cmd+Shift+Backspace) is `ClearBrowsingData` (exists, unbound). Ctrl+S is Save Page As (the menu row exists in menu.rs; add `Action::SavePage`). Ctrl+O opens a local file through the file chooser and opens file:// in a new tab. Ctrl+Shift+J and Cmd+Alt+J open DevTools on the Console. Ctrl+Shift+V / Cmd+Shift+Alt+V is paste as plain text (CEF frame `PasteAndMatchStyle`, through the Edit-command path). F7 toggles caret browsing (Chrome pref `settings.a11y.caret_browsing.enabled` via `CefRequestContext::SetPreference`, with Chrome's confirm). Ctrl+E focuses the address with a search prefix. Cmd+M minimizes the window on macOS. Cmd+` and Cmd+Shift+` cycle wubble's windows (`Action::NextWindow` with `Command::FocusWindow`), and Ctrl+Shift+Tab-style window cycling uses Alt+F6-free chords on Linux. In the address and command bar: Alt+Enter opens the result in a new tab, Ctrl+Enter adds www. and .com, Shift+Enter opens in a new window (bar.rs and omnibox.rs submit get a disposition). Keep each row `PageFirst` or `Shell` as Chromium does. Add keymap tests for conflicts and the editor labels.

## Address field context menu: Paste and go, Paste and search, copy and edit rows (`address-paste-and-go`)

Key: `address-paste-and-go` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

Right-clicking Chrome's omnibox gives Undo, Cut, Copy, Paste, 'Paste and go to <url>' or 'Paste and search for "text"', Delete, Select all, and 'Always show full URLs'. In wubble, the command bar field (kit/chrome/command-head.slint) and the address pill (kit/chrome/address-pill.slint) have no menu. Chrome's paste-and-go is missing entirely, and Shift+Insert paste in shell fields is an open TODO.

Add a shell menu for the command field and the pill, built as Menu::Shell entries in crates/wubble/src/menu.rs, like the tab row menu:
- one table of rows
- the paste row's label from classify() over the clipboard text (crates/wubble/src/clipboard.rs): 'Paste and go to <elided url>' for Intent::Url, 'Paste and search for "…"' for Search, disabled when the clipboard is empty
- choosing it runs bar.rs target_command in the bar's mode

The pill's menu adds Copy address (Action::CopyUrl), Edit address (open the bar filled) and Paste and go. Add Action::PasteAndGo to action.rs (FIXED table and PALETTE) so the command bar and keymap can reach it; Chrome has no default chord, so leave it unbound. Add a setting `ui.full_urls`, off by default. When on, the pill shows scheme and full path instead of wubble_core::chrome::address_parts' host and path; feed it from shell.rs. Shift+Insert pastes in CommandHead's key-pressed. Unit tests for the menu labels in menu.rs.

## Reload button turns into Stop while loading; reload menu with hard reload and empty cache (`nav-stop-reload`)

Key: `nav-stop-reload` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

While a page loads, Chrome's reload button turns into an X (Stop), and back into Reload when the load ends. Shift-click or Ctrl-click does a hard reload. With DevTools open, a right-click or long press gives Normal reload, Hard reload and 'Empty cache and hard reload'. wubble has Action::Stop (Escape, Cmd+.), Action::HardReload (ViewOp::HardReload, reload_ignore_cache in crates/wubble-engine/src/view.rs) and an `address-loading` property in app.slint, but NavButtons (crates/wubble-ui/ui/components/nav-buttons.slint) always shows Reload.

Add `in property <bool> loading` to NavButtons, bound from the visible tab's loading state (the same source as address-loading in crates/wubble/src/bind.rs). The fourth button's icon, label and id become Glyph.close / 'Stop' / 'stop' while loading, swapped with a kit crossfade (kit/motion/crossfade.slint). If the kit's icon set has no stop glyph, use close and note it in docs/fidelity/derived-review.md. Add a modifiers argument to `action(...)` so the host maps Shift or Mod on reload to hard-reload. The NavButtons button table stays one table.

Add Action::EmptyCacheHardReload (FIXED table and PALETTE). It sends the CDP call Network.clearBrowserCache through View::cdp (crates/wubble-engine/src/cdp.rs), then ViewOp::HardReload. A right-click on the reload button opens a Menu::Shell (crates/wubble/src/menu.rs) with the three rows; the third is shown only when wubble_core::devtools says DevTools is open for that tab. Unit tests in perform.rs for the plans.

## Error panel: auto-reload when the network comes back (`offline-auto-reload`)

Key: `offline-auto-reload` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

Chrome's NetErrorAutoReloader reloads a page that failed with a network error (ERR_INTERNET_DISCONNECTED, ERR_NAME_NOT_RESOLVED, ERR_NETWORK_CHANGED, ERR_CONNECTION_*, ERR_ADDRESS_UNREACHABLE, timeouts). It retries on an exponential backoff (1 s, 5 s, 30 s, 1 m, 5 m, 10 m), and at once when the system comes back online, but only while the tab is visible. It never retries certificate errors, a POST, or a load the user stopped. wubble's PageError::Load (crates/wubble/src/page_error.rs) stays until the user presses Reload.

Add the retry schedule to the error machine as data: a `retry: Option<Retry { attempt, due_ms }>` inside PageError::Load. New events: Tick(now_ms), Online, Hidden, Shown and UserStopped. A new effect is ErrorEffect::Reload with a flag marking it as automatic. All of it stays pure in transition(), and the host's existing tick drives Tick. The table of retryable codes lives in wubble-core/neterror.rs (a `retryable(code)` fn ported from Chromium's net_error_auto_reloader.cc IsReloadableError, BSD-3, behaviour only).

Online detection makes no request. On Linux, watch NetworkManager's or systemd-networkd's state over D-Bus only if a crate is already in the tree; otherwise rely on the backoff alone. On macOS use SCNetworkReachability or NWPathMonitor. Put each behind one shared trait in a new small module, next to os_scope.rs. Hidden tabs and sleeping or discarded tabs (wubble_core::sleep) do not retry. The panel shows 'Trying again…' while a retry is due. Unit tests for the schedule, the visibility gating and the refusal cases.

## Zoom indicator and zoom controls like Chrome's omnibox zoom bubble (`zoom-indicator`)

Key: `zoom-indicator` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

In Chrome, when a site's zoom differs from the default, a magnifier icon shows in the address bar. Clicking it opens a bubble with the percentage, −, + and Reset. The app menu has a Zoom row (−, percent, +, fullscreen), and Settings > Appearance > Page zoom lists the per-site levels. wubble has per-site zoom (`wubble_core::zoom::ZoomTable`, `SITE_ZOOM`/`ZOOM_DEFAULT` settings, host `user_zoomed` in crates/wubble/src/host/surfaces.rs), a brief toast after Ctrl +/-/0 (components/toast.slint), and a site-zoom list in settings (`settings-site-zoom`). Nothing persistent shows that a page is zoomed, and there is no pointer path to zoom. To build: a `zoomed` percent property for the visible tab (from `PageEvent::ZoomChanged` against the default), shown as a small kit chip in the address pill (a derived kit piece composing existing kit parts, on the derived-review sheet) only when it is not the default. The chip opens a popover with −, the percent, + and Reset, which perform `Action::ZoomOut`/`ZoomIn`/`ZoomReset`. Add a Zoom row to the page menu (`window_page_entries` or the page's own menu in crates/wubble/src/menu.rs). The popover closes on Esc and on outside click as the other popovers do. A pure fn in wubble-core decides whether the chip shows (percent, default), with tests.

## Download speed and time remaining in rows (`download-speed-eta`)

Key: `download-speed-eta` · engine patch: no · state: [~] written blind on sprint/features, not compiled.

Chrome's download rows read "2.1 MB/s · 12 MB of 100 MB, 40 secs left". wubble's DownloadEntry::detail (wubble-core/src/download.rs) shows only the amounts, because welding's DownloadProgress event (vendor/welding/src/handlers.rs on_download_updated) drops CefDownloadItem::current_speed.

What to build: (1) welding: add `bytes_per_second: Option<u64>` to NavigationEvent::DownloadProgress from `item.current_speed()`, as a `wubble change` entry. (2) wubble-engine/src/download.rs and page.rs: carry the speed through DownloadInput::Progress and PageEvent::DownloadProgress, and update their tests. (3) wubble-core/src/download.rs: give DownloadEvent::Progress the speed, store `speed: Option<u64>` on DownloadEntry (not persisted; it is cleared when the download leaves Running), and make detail() return Chromium's format: speed, amount, and the time left from (total - received) / speed. Format the time with a pure port of ui::TimeFormat's short remaining strings ("40 secs left", "3 mins left", "2 hours left", from ui/base/l10n/time_format.cc, BSD) in a new `remaining_text(secs)`, with unit tests. Smooth the speed with an exponential moving average, kept in the entry, so the text does not jitter. (4) Media downloads (host/media.rs jobs) report their own rate the same way, from the bytes the writer has written per tick, so both kinds of row read alike. No UI change: the row already shows `detail`. All platforms.
