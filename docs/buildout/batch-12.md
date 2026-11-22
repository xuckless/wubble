# Build-out batch 12

15 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `ext-side-panel`: Extensions: chrome.sidePanel in a wubble side pane

Area: devtools-extensions-internals. Needs an engine patch: yes.

Chrome shows an extension's `side_panel.default_path` page in the browser's side panel. `sidePanel.setOptions({tabId, path, enabled})` sets a per-tab page, `sidePanel.open({windowId|tabId})` opens it on a user gesture, `setPanelBehavior({openPanelOnActionClick})` makes the action button open it, and the side panel menu lists every extension with a panel. wubble has none. Bitwarden's default `sidepanel-disabled.html` is never shown (docs/research/extensions.md). wubble side (no patch needed for the default path): `parse_manifest` reads `side_panel.default_path`. A pane like the DevTools panel (`host/devtools.rs`, the `SourceSeam` right-edge pane with a new key, `SIDE_PANEL_TAB`) hosts a windowless view of `chrome-extension://<id>/<path>` in the visible tab's container. The lifecycle is a pure `extensions::side_panel` enum (Closed, Opening{ext}, Open{ext, tab}) with a transition, one per window, with tests, and the panel follows the visible tab, as `dock_follows_tab` does. Open it from the footer popover row menu and from the `_execute_side_panel` command. Patch `57-cef-side-panel.patch`: delegate `side_panel_open(extension_id, browser_id or window_id)`, called from `SidePanelOpenFunction` (`chrome/browser/extensions/api/side_panel/side_panel_api.cc`) when no `Browser` exists. A table function `side_panel_options(extension_id, browser_id) -> {path, enabled, open_on_action}` reads `SidePanelService`. A delegate notice on `OnPanelOptionsChanged` lets the pane change page. `action_click` consults `open_on_action` first. Mark the view via ext-tabs-getcurrent's `mark_view(kind = side panel)`.

## `harness-apps-and-windows`: Agent apps: VS Code, Copilot CLI, Goose and Windsurf on macOS as written formats; Windows paths

Area: todo-a. Needs an engine patch: no.

TODO lines 65-66. `crates/wubble-core/src/agent/harness.rs` lists VS Code, Copilot CLI and Goose with `format: None` (found, never written), Windsurf on Linux only, and paths for Linux and macOS only (`files`/`detect` keyed by `Os`). Add from each app's documentation: VS Code user `mcp.json` with `{"servers": {name: {"type":"stdio","command","args"}}}` (path `["servers"]`, `Entry::Stdio`); Copilot CLI `~/.copilot/mcp-config.json` `mcpServers` with `{"type":"local","command","args","tools":["*"]}` (a new `Entry` variant); Goose `~/.config/goose/config.yaml` `extensions.<name>: {type: stdio, cmd, args, enabled: true, name, timeout}`. Goose needs a new `Format::GooseYaml`, edited keeping the rest of the file byte for byte (a yaml-edit-preserving crate, or a narrow line-based edit of the `extensions:` block that refuses odd shapes); Windsurf on macOS is `~/.codeium/windsurf/mcp_config.json`. Windows: add `(Os::Windows, ...)` entries for every app (Claude desktop under `AppData/Roaming/Claude`, VS Code `AppData/Roaming/Code/User/mcp.json`, home-relative dotfiles as on Linux). The manifest's runtime dir (`crates/wubble-core/src/agent/manifest.rs`, `crates/wubble/src/instance.rs` `private_root`) uses `%LOCALAPPDATA%\wubble\mcp`. Extend the existing harness unit tests for the new formats (core logic).

## `kit-lock-baseline-empty`: Empty the kit lock baseline

Area: todo-b. Needs an engine patch: no.

The kit lock (crates/wubble-ui/tests/kit_lock.rs) still records about 100 violations in crates/wubble-ui/tests/kit_lock_baseline.txt. 60 are literals in `kit/derived.slint`; the rest are in shell files that the rules say must hold no visuals: components/raised.slint (9), components/folder-row.slint (9), components/border-beam.slint (5), app.slint (5), components/sidebar.slint (3), components/focus-ring.slint (3), kit/chrome/tab-row.slint (2) and kit/chrome/split-view.slint (2). TODO Design fidelity asks to empty it and to alias the value-equal `Shell` literals.

To build: (1) For each `derived.slint` literal, find the generated token with the same value in `ui/kit/gen/*.slint` and use it. Where none exists, add a named alias in one place. If it comes from the design, add it to the extractor (`tools/fidelity/extract.mjs`, then rerun it); never edit the gen files by hand. A truly wubble-only value becomes a single named constant recorded in docs/fidelity/derived-review.md. (2) Move the visuals in components/raised.slint, folder-row.slint, border-beam.slint and focus-ring.slint into kit components (a `// derived:` header naming the kit parts they compose) and make the shell files only lay them out. (3) Do the same for app.slint's five, and replace the `Shell` literals that equal a token. (4) Lower the baseline with `KIT_LOCK_WRITE=1 cargo test -p wubble-ui --test kit_lock` (the compile or test step) until it is empty, and leave the rule in force. Gallery shots before and after must match by eye. All platforms.

## `bookmark-editing-complete`: Bookmark editing: edit URL, open all, bookmark all tabs, folder delete confirmation and undo

Area: navigation. Needs an engine patch: no.

Chrome's bookmark editor and manager do the following, and wubble is missing them:
- 'Edit…' changes both the name and the URL. BookmarkEdit in crates/wubble-core/src/pages.rs has Rename (title only), Move, Delete, Add and AddFolder.
- 'Open all (N)', 'Open all in new window' and 'Open all in Incognito', with a confirmation over 15 tabs.
- 'Bookmark all tabs…' (Mod+Shift+D) makes a folder of the window's tabs.
- Deleting a folder is undoable ('Undo' toast). TODO notes that wubble deletes a folder with its contents in one press.
- 'Copy' and 'Paste' of bookmarks, and 'Show in folder' from search results.

Build:
1. BookmarkEdit::Edit { id, title, url } and a store fn (crates/wubble-store/src/bookmarks.rs update_bookmark). The bookmark sheet (components/bookmark-sheet.slint, host/pages.rs sheet_view and sheet_edits) gets a URL field for the Edit kind, validated through omnibox::classify, which must be Intent::Url.
2. Action::BookmarkAllTabs (FIXED, PALETTE, keys.rs Mod+Shift+D). It opens the sheet in a new 'folder of tabs' kind and writes AddFolder plus Adds as one store transaction.
3. A row menu on bookmark and folder rows (Menu::Shell): Open in new tab, Open all (N) for folders, Edit, Move, Copy address, Delete. Open all opens Command::NewTab for each URL. Over 15 asks through the existing dialog-sheet.
4. Undo for deletes: a pure undo slot in wubble-core holding the deleted subtree (BookmarkRow list). A kit toast (components/toast.slint) with 'Undo' re-inserts it at the same parent and positions. The slot clears on the next edit.

Unit tests for subtree capture and restore, and for the sheet kinds.

## `download-source-marks`: Mark downloaded files with their origin (quarantine / MOTW / xattr)

Area: privacy-security. Needs an engine patch: no.

Chrome annotates every finished download. On macOS it sets the `com.apple.quarantine` xattr (with the agent name and the origin URL, so Gatekeeper checks the file) and `kMDItemWhereFroms`. On Windows it writes the Mark-of-the-Web `Zone.Identifier` ADS (ZoneId=3, ReferrerUrl, HostUrl) through IAttachmentExecute. On Linux it sets `user.xdg.origin.url` and `user.xdg.referrer.url`. With CEF and wubble's download handling it is not known that these are written, and the media downloader (wubble-ffmpeg muxes) writes files itself without them.

Build:
- Add a pure `wubble_core::download::source_marks(url, referrer, platform) -> Vec<Mark>` that strips credentials from URLs, as Chromium's `quarantine` component does (components/services/quarantine, BSD-3-Clause, with a source comment). Unit-test it.
- Add one shared trait `Quarantine` with a per-platform implementation (AGENTS.md: per-platform code implements one shared trait) in crates/wubble (or wubble-sysmem-style OS code). Linux uses `xattr` setxattr (the `xattr` crate, MIT/Apache). macOS uses setxattr `com.apple.quarantine` with the `0081;<hex time>;wubble;<uuid>` format plus the WhereFroms plist. Windows writes `<path>:Zone.Identifier`. It runs only when the mark is missing, so a CEF-written mark is kept.
- Call it from crates/wubble/src/host/downloads.rs on the Complete transition and from the media downloader's finish.
- A failure is logged, never fatal.

## `pip-page-api`: Page Picture-in-Picture API and auto-PiP through wubble's Pip window

Area: media-downloads. Needs an engine patch: no.

In Chrome, `HTMLVideoElement.requestPictureInPicture()` opens the PiP window and resolves with a PictureInPictureWindow (width, height, onresize). `document.pictureInPictureElement`, `exitPictureInPicture()` and the enterpictureinpicture and leavepictureinpicture events all work. The media session action `enterpictureinpicture` lets video calls auto-enter PiP on a tab switch. In wubble's windowless CEF the API rejects. wubble's own PiP (wubble_core::pip, host/videos.rs pip_event, kit.js `k.pip`) is started only from the strip, the setting PIP_ON_TAB_SWITCH, or the shell.

What to build: (1) kit.js: replace `HTMLVideoElement.prototype.requestPictureInPicture`, `document.exitPictureInPicture`, `pictureInPictureEnabled` and the `pictureInPictureElement` getter, plus the events. A request with a gesture (navigator.userActivation.isActive) records the element and returns a pending promise. The next snapshot reports `pipAsk`. The host feeds it into the pip machine as a new `PipInput::PageAsked(tab)`, and later answers with a kit call `k.pipDone(ok, w, h)` that resolves or rejects and fires the events. `disablePictureInPicture` refuses. (2) wubble-core/src/pip.rs: the new input and the effect `AnswerPage`, added to the transition table with unit tests. (3) Auto-PiP: a page that registered an `enterpictureinpicture` media session handler (kit.js already keeps handlers) is popped out on a tab switch even with PIP_ON_TAB_SWITCH off, as Chrome does for video calls. Its handler is called instead of the element fullscreen. All platforms.

## `import-firefox-secrets`: Import passwords and cookies from Firefox and Zen directly (no CSV)

Area: passwords-autofill-sync. Needs an engine patch: no.

Chrome and Firefox import saved logins straight from another profile; wubble only accepts a CSV the user exports by hand (TODO.md "Zen/Arc password import (CSV only)", crates/wubble-vault/src/import.rs). Build in crates/wubble-import: (1) `firefox/logins.rs`: parse `logins.json` (hostname, httpRealm, formSubmitURL, encryptedUsername, encryptedPassword, timeCreated, timesUsed) and decrypt with the key from `key4.db` (SQLite `metaData` password-check entry and `nssPrivate` a11): ASN.1 DER (crate `der`, Apache/MIT) PBES2 with PBKDF2-HMAC-SHA256 + AES-256-CBC for modern profiles and PKCS#12 3DES (SHA-1 PBE) for older ones, then the per-entry 3DES-CBC/AES-CBC decrypt. Implement from NSS's published formats (NSS is MPL-2.0; any copied code credited in THIRD_PARTY_NOTICES.md; do not copy GPL firefox_decrypt). A primary password set in Firefox is detected via the password-check entry and asked for in a wubble sheet; wrong password -> retry. Rows -> `wubble_vault::ImportRow`, applied with `wubble_vault::apply`; Firefox's `disabledSavingOrigins` in `permissions.sqlite` (`password-saving` type) -> never-save decisions. (2) `firefox/cookies.rs`: `cookies.sqlite` `moz_cookies` (plaintext) mapped by `originAttributes` userContextId to the imported containers (crates/wubble-import/src/containers.rs) where wubble has the container, else the default one; written through welding `set_cookie`. Both read from copied files (locked DB). Zen uses the same code. Unit tests with fixture key4.db/logins.json generated from a throwaway profile with a known password.

## `ext-omnibox`: Extensions: chrome.omnibox keyword mode in the command bar

Area: devtools-extensions-internals. Needs an engine patch: yes.

In Chrome, an extension declares `omnibox.keyword`. Typing the keyword followed by Tab or Space enters keyword mode ("<Extension name> |"). `onInputStarted`, `onInputChanged(text, suggest)`, `onInputEntered(text, disposition)`, `onInputCancelled` and `onDeleteSuggestion` fire, and suggestions with description markup (`<match>`, `<dim>`, `<url>`) plus `setDefaultSuggestion` appear as results. wubble's command bar (`wubble_core::omnibox`, `suggest.rs`, `crates/wubble/src/bar.rs`) has no extension source. Core (pure, tested): `parse_manifest` reads `omnibox.keyword`. `omnibox::classify` recognises `<keyword><space>` against the enabled extensions' keywords and yields a `Mode::ExtensionKeyword { ext, text }`. A small state machine `omnibox::ext_session` (Idle, Started, Waiting(req id), Shown) turns typing into effects: Start, Changed(text, req), Entered(text, disposition), Cancelled. Description markup is parsed into styled runs for the kit result row (`ExtensionSuggestion` mapped onto an existing `CommandKind`). Patch `58-src-omnibox.patch`: table functions `omnibox_started(profile, ext)`, `omnibox_changed(profile, ext, text, req)`, `omnibox_entered(profile, ext, text, disposition)`, `omnibox_cancelled(profile, ext)` and `omnibox_delete(profile, ext, content)`, calling `ExtensionOmniboxEventRouter::On*` (`chrome/browser/extensions/api/omnibox/omnibox_api.cc`). A delegate `omnibox_suggestions(ext, req, json)` is fed from `OmniboxAPI`'s suggestions-ready path and `OmniboxSetDefaultSuggestionFunction`. Entered with disposition current tab, new foreground tab or new background tab maps to the extension's own handling, since the extension navigates itself.

## `agent-nested-scroll`: Wheel over a nested scroller scrolls the pane, not the document (agent `scroll` and the user's wheel)

Area: todo-a. Needs an engine patch: no.

TODO line 67. `scroll` with a direction over an `overflow:auto` pane sends `View::wheel` at the pane's point (`agent_wheel` in `crates/wubble/src/host/agent.rs`, 5 ticks of 20 ms). The event reaches the pane, but in most runs the document scrolls. The likely cause is Chromium's wheel scroll latching and hit-test: wubble sends no mouse move to the point first, so the latched scroll target and hover state come from the last pointer position, and the phase (`wheel_deltas` in `crates/wubble-engine/src/input.rs`) may start a new latch sequence on the document. Fix: before the first tick, send a mouse move to the point and fence it (`Fence::Pointer`). Then send the ticks as one gesture with Chromium's phase flags (began, changed, ended) where CEF exposes them through `MouseEvent` modifiers (`EVENTFLAG_PRECISION_SCROLLING_DELTA`), or move the agent path to DevTools `Input.dispatchMouseEvent {type:'mouseWheel'}` at the point. That event is hit-tested at its own coordinates and scrolls the innermost scroller as a user's would. Check that `View::wheel` for the user's own wheel moves the pointer first too (`crates/wubble/src/input.rs`). Update the `scroll` tool text in `crates/wubble-mcp/src/tools.rs`.

## `voice-glow-kit`: Voice glow shader in the kit (no microphone)

Area: todo-b. Needs an engine patch: no.

The design has a voice glow: low/mid/high bands with attack and release plus a flow phase (`core/voice.ts` `VOICE` constants and `VoiceBands` in the design clone), feeding uniforms of the glass shader and the listening bubble. In wubble the shader is ported, but `glass.wgsl` sends the voice uniforms as zero (crates/wubble-gfx/src/scene/mod.rs around lines 67-68), and nothing drives them. Decisions "No voice yet": the shader goes into the kit, with no microphone.

To build: (1) Port `VoiceBands` as a pure state machine in wubble-core (`voice.rs`, with a `// design: core/voice.ts:<lines> @<commit>` comment): `fn step(bands, input: Levels, dt) -> Bands` with the design's attack 18/s, release 4.5/s and flow 0.6 + 3.2·level rad/s. Generate the constants through `tools/fidelity/extract.mjs` into `wubble_core::kit_gen`, not by hand. Add the synthetic speech-like envelope the design uses for demos. Unit tests. (2) Add a scene input for the bands in wubble-gfx scene state (state.rs/render.rs) that feeds the existing voice uniforms in glass.wgsl, and the bubble if the design's listening bubble shares them. Effects run only while something sets non-zero bands, so idle cost stays zero (scene run state). (3) Add a kit-facing property, for example a `voice` struct on the glass layer, and a gallery scene driven by the synthetic envelope (crates/wubble/src/gallery_smoke.rs). No microphone, and no permission prompt. All platforms.

## `dangerous-site-interstitial`: Dangerous-site interstitial for blocker badware/malware document blocks

Area: privacy-security. Needs an engine patch: no.

Chrome shows a red full-page "Dangerous site" warning with "Back to safety" and, under Details, "visit this unsafe site". uBlock Origin's strict blocking does the same for `$doc` and badware rules. wubble's blocker ships the uBO badware list (crates/wubble-blocker/src/catalog.rs `ublock-badware`). A blocked main-frame document ends as ERR_BLOCKED_BY_CLIENT in the generic error panel (crates/wubble-core/src/neterror.rs -20), with no way through and no explanation.

Build:
- wubble-blocker: decisions for Document requests report which list matched (`Decision` in crates/wubble-blocker/src/decision.rs gets a `list: Option<&'static str>` or a `Category {Badware, Privacy, Ads}`). Unit-test it.
- crates/wubble/src/blocking.rs keeps the last document block per browser_id, holding the URL, list and rule.
- crates/wubble/src/page_error.rs: when the load error is -20 and a document block is recorded for that URL, the panel becomes `PageError::Blocked { url, category, rule }`. Its text is Chrome's register ("Dangerous site" for badware, "Page blocked" for others), with "Go back" (the default) and "Proceed anyway" under Advanced.
- Proceed adds a session exception (per container and host) that the filter consults for Document requests only, then reloads. The site's Shields toggle (`blocker.exceptions`) stays the permanent way.
- UI: reuse kit a-error-panel with a danger tint if the kit has one, else a derived piece on derived-review.md.
- No network lookups: Safe Browsing stays out.

## `pip-controls-complete`: Picture-in-Picture window controls parity

Area: media-downloads. Needs an engine patch: no.

Chrome's video PiP window has play/pause and back and forward 10 s. It adds previous and next track when the page has those media session handlers, plus mute and unmute, a progress bar with the current and total time, Skip Ad (skipad), the video-call buttons (togglemicrophone, togglecamera, hangup), back to tab and close. The window can be resized from its edges, and its size and position are remembered. In wubble, components/pip-controls.slint has back 10 s, play, forward 10 s, return and close only (host/videos.rs pip_command: play, seek-by, return, close). Resize and placement memory are not handled.

What to build: (1) Host: feed the Pip window's controls from the same kit `meta` and `v` snapshot the strip uses, which already carries t, dur, muted and the next/prev flags. Extend kit.js `meta` with flags for skipad, togglemicrophone, togglecamera and hangup handlers. (2) pip-controls.slint: add the conditional buttons and a seek track. Reuse the video strip's track from video-controls.slint, and its kit parts, BDock and IconButton. Commands go out as new pip_command names: mute, next, prev, skipad, mic, camera, hangup, seek-to; route the media-session ones to `kit.act`. (3) Window: resizable from its edges, keeping the video's aspect ratio. Remember its size and position in settings (Section::Hidden) and restore them next time, clamped to the current display (wubble_core::window owns the Pip window's state). (4) Keys while the PiP window is focused: Space toggles, the arrows seek, M mutes. The design has no PiP (wubble-only.md §28), so add the new pieces to derived-review.md. All platforms.

## `import-zen-arc-gaps`: Import: Zen legacy workspace tables, Little Arc, Arc pinned/favourites details

Area: passwords-autofill-sync. Needs an engine patch: no.

TODO.md lists gaps in the existing importers: "Zen legacy tables, Arc history, Little Arc" (Arc history moves to `import-chromium-family`). Build in crates/wubble-import: (1) Zen legacy storage: before `zen-sessions.jsonlz4`, Zen kept workspaces and pins in `places.sqlite` tables (`zen_workspaces`, `zen_pins`, `zen_workspace_*`, depending on version) and in `sessionstore.jsonlz4` window extra data; extend crates/wubble-import/src/places.rs/session.rs to read those tables when present and map them to `ImportedSpace`/`ImportedFolder`/`ImportedTab` (essentials -> `Placement::Essential`, pinned -> `Pinned`, workspace container -> `container`), preferring the newest source and noting the fallback in `Imported::warnings`. Use Zen's own source (MPL-2.0) for the schema and credit it. (2) Little Arc: Arc keeps Little Arc windows' tabs in its sidebar data (`StorableSidebar.json` containers of type little-arc / `StorableWindows.json`); crates/wubble-import/src/arc.rs maps them to a Space named "Little Arc" (or into the active Space's unpinned tabs). (3) Arc's favourites (top apps), split-view items and folder colours: map what `wubble_core` has (favourites -> pinned/essential, splits -> wubble splits via `split_panes` if the store's merge supports it, else one tab each with a warning). Fixtures and unit tests for each format version. Verifying the macOS Zen and Windows Arc paths on real machines is hand work, not part of this.

## `ext-native-messaging`: Extensions: native messaging hosts found where desktop apps install them

Area: devtools-extensions-internals. Needs an engine patch: no.

In Chrome, `runtime.connectNative`/`sendNativeMessage` find host manifests in the user-level dirs (`~/.config/google-chrome/NativeMessagingHosts`, `~/.config/chromium/NativeMessagingHosts`, macOS `~/Library/Application Support/Google/Chrome/NativeMessagingHosts`) and the system dirs, and Windows uses the registry. Password-manager desktop apps (Bitwarden, 1Password, KeePassXC) write Chrome's and Chromium's dirs. In wubble, CEF overrides `DIR_USER_DATA`, so Chromium looks only under `<cache root>/NativeMessagingHosts` and the system Chromium paths (docs/research/extensions.md table row "nativeMessaging": "wubble's user dir needs them copied or linked (later)"). To build: (1) Core, pure: `extensions::native_hosts::plan(found: &[(source_dir, file_name, json)], installed_ids, existing_in_ours) -> Vec<HostOp>` keeps only valid manifests (`name`, `path`, `type: stdio`, and `allowed_origins` naming an installed extension id, or any `chrome-extension://` origin when the user allows it). It copies changed ones, removes ones whose source vanished and that wubble wrote (tracked by a marker field or a side list), and never overwrites a user's own file. Tests. (2) Engine: `crates/wubble-engine/src/extensions.rs` gains `sync_native_hosts(root)`, run before CEF starts next to the dictionary install, scanning the per-platform source dirs (Linux: google-chrome, chromium, BraveSoftware, microsoft-edge, vivaldi; macOS equivalents). On Windows, no copy is needed, since Chromium reads `HKCU\Software\Google\Chrome\NativeMessagingHosts` only for Chrome branding: add our own registry key mirroring entries. (3) Settings: a Bool `extensions.native_hosts_import` (default 1, local only, no network) and a list of imported hosts in the extensions section.

## `agent-wait-network-idle`: `wait_for load` follows in-page navigations' fetches: network idle through DevTools events

Area: todo-a. Needs an engine patch: no.

TODO line 68. `Wait::Loaded` (crates/wubble/src/host/agent.rs) waits for no load running and a tree. A history-API navigation's fetches are not followed. Add Playwright's `networkidle`: in `crates/wubble-engine/src/cdp.rs`, a pure `NetIdle` reader (like `NavRequests`) that counts in-flight requests from `Network.requestWillBeSent`, `Network.loadingFinished`, `Network.loadingFailed` and `Network.webSocketCreated/Closed`, excluding data: URLs and long-polling event streams after 5 s, and records the time it last went to zero. Enable the Network domain only on agent-driven views (`Network.enable` with small buffer sizes, sent with the session's other setup) so ordinary tabs pay nothing. Expose `View::net_idle_since() -> Option<Instant>`. Add `Wait::Idle { tab }` (0 in flight for 500 ms) and a `network` (or `idle`) value for `wait_for` in `crates/wubble-mcp/src/tools.rs`. Make `load` wait for idle after a same-document navigation when the URL changed during the call. Update the tool description that documents the gap.

