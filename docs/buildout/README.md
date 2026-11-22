# The build-out (from 2026-11-22)

The owner's order: every open TODO item and Chrome's functionality, written by agents, **one branch per
feature**, no tests, no smokes, no reviews while writing; the owner then reviews each feature and ships them
one by one. Copy open source first, build only what is left. Engine patches are written into
`patches/chromium` on the feature's branch and **not built** (no Chromium rebuild).

- **254 features in 17 batches.** Batch 1 has 16 (the ones already started on 2026-11-22); the rest 15 each.
  One batch a day (owner, 2026-11-22: "batch them at 15 everyday"). The specs are in `batch-NN.md`;
  `features.json` is the same list as data (key, title, spec, area, engine, batch).
- The list came from eleven scouts comparing Chrome and TODO.md against the code on 2026-11-22 (342 proposals,
  88 duplicates dropped). Batch 2 starts with the drag-and-drop items; after that batches take one feature per
  area in turn.
- A feature lives on `feat/<key>` in the worktree `~/Dev/wubble-wt/feat/<key>/w`. Nothing is merged into `main`.
- Each branch gets one compile-only pass (`cargo.sh check --workspace`, one branch at a time, compile errors
  only), then its engine patch, if it needs one, is written against the checkout in `~/Dev/wubble-wt/cef-build`.
- Running a batch: the workflow script `docs/buildout/batch.js` with the batch's entries of `features.json` as
  its `args` (the lead launches it; see docs/orchestration.md for the memory rules every agent works under).
- After a batch: fill its row below (written / partial / nothing, compiles or not, patch files) from the
  workflow's result.

## Status

| Batch | Day | State |
|---|---|---|
| 1 | 2026-11-22 | running |
| 2 | | waiting |
| 3 | | waiting |
| 4 | | waiting |
| 5 | | waiting |
| 6 | | waiting |
| 7 | | waiting |
| 8 | | waiting |
| 9 | | waiting |
| 10 | | waiting |
| 11 | | waiting |
| 12 | | waiting |
| 13 | | waiting |
| 14 | | waiting |
| 15 | | waiting |
| 16 | | waiting |
| 17 | | waiting |

## Features

| Batch | Key | Feature | Area | Engine patch | State |
|---|---|---|---|---|---|
| 1 | `private-windows` | Private (incognito) windows with an in-memory container and a visible mark | tabs-windows |  | |
| 1 | `multiple-profiles` | Multiple profiles, guest profile and a profile switcher | tabs-windows |  | |
| 1 | `nav-history-capture` | Capture each tab's back/forward entries, and the back button history menu | tabs-windows |  | |
| 1 | `recently-closed-full` | Recently closed windows, folders and Spaces, and a recently-closed list UI | tabs-windows |  | |
| 1 | `startup-home-settings` | Startup, home page and quit settings | tabs-windows |  | |
| 1 | `window-state-restore` | Window state in the session: maximized, fullscreen, display and popup windows | tabs-windows |  | |
| 1 | `tear-off-fidelity` | Tab tear-off: new window at the drop point, drop into a window at a row | tabs-windows |  | |
| 1 | `wayland-tab-drag` | Wayland: drag tabs between windows and live tear-off via the data device | tabs-windows |  | |
| 1 | `tab-groups-parity` | Tab groups parity for folders: colour, new tab in group, move, save, keyboard | tabs-windows |  | |
| 1 | `multi-select-tabs` | Select several tabs (Ctrl/Shift+click) and act on the selection | tabs-windows |  | |
| 1 | `tab-search` | Tab search (Ctrl+Shift+A): all open and recently closed tabs in one list | tabs-windows |  | |
| 1 | `site-sound-tab-menu` | Mute site, and the Chrome tab and link menu rows still missing | tabs-windows |  | |
| 1 | `chrome-shortcut-gaps` | Chrome keyboard shortcuts wubble does not bind yet | tabs-windows |  | |
| 1 | `omni-inline-autocomplete` | Command bar: inline autocomplete and Chrome's omnibox keys | navigation |  | |
| 1 | `omni-calculator-units` | Command bar: local calculator and unit conversion answers | navigation |  | |
| 1 | `search-engines-keywords` | Search engines: more built-ins, custom engines with keywords, management in Settings | navigation |  | |
| 2 | `x11-xdnd` | X11: page drags out to other apps and drops in of links, HTML and text (XDND) | page-interaction |  | |
| 2 | `cross-window-page-drag` | Page drags between wubble windows, and an OS drag coming back in | page-interaction |  | |
| 2 | `drag-custom-types-out` | An HTML draggable's custom dataTransfer types (and DownloadURL) reach other apps | page-interaction |  | |
| 2 | `download-drag-out` | Drag a finished download to another app | media-downloads |  | |
| 2 | `windows-drag-drop` | Windows: drags out of the window and drops in from other apps (OLE) | platform |  | |
| 2 | `agent-html-drag` | Agent `drag` carries an HTML draggable drag through the page's drag and drop | todo-a |  | |
| 2 | `bookmark-drop-position` | Bookmarks: drag reorder with Chrome's drop line between rows | todo-a |  | |
| 2 | `omni-keyword-mode` | Command bar: keyword mode (tab to search) and @tabs/@bookmarks/@history scopes | navigation |  | |
| 2 | `shell-drop-parity` | Drops on the tab list: on a tab navigates it, files open as tabs, link titles kept | page-interaction |  | |
| 2 | `popup-setting-allow-list` | Pop-ups: make the allow list work without a click, plus a default and the redirect notice | privacy-security |  | |
| 2 | `perm-kinds-complete` | Prompt for every permission CEF 156 forwards (protocol handler, window management, file editing, downloads, local network, sensors, ...) | web-platform-ui |  | |
| 2 | `download-danger-warnings` | Dangerous and insecure download warnings (Keep / Discard) | media-downloads |  | |
| 2 | `autofill-context-menu` | Autofill entries in the editable-field context menu (manual fallback) | passwords-autofill-sync |  | |
| 2 | `devtools-pipe-bridge` | Docked DevTools without the remote debugging port (embedder bridge) | devtools-extensions-internals |  | |
| 2 | `windows-frame-path` | Windows: windowless views on the zero-copy D3D frame path | platform |  | |
| 3 | `site-layout-fixes` | wubble.cc: feature section recomposed, readable diagrams and install capsule on phones | todo-a |  | |
| 3 | `agent-ax-lazy-mirror` | Agent sessions: apply hidden tabs' accessibility updates lazily | todo-b |  | |
| 3 | `omni-zero-suggest` | Command bar: suggestions on open (current URL, most visited, copied link) | navigation |  | |
| 3 | `wayland-clipboard-rich` | Wayland: page clipboard carries HTML, images, files and web custom formats | page-interaction |  | |
| 3 | `container-management` | Containers: create, rename, delete, and assign from the UI | privacy-security |  | |
| 3 | `screen-share` | getDisplayMedia: ask to share the screen, then show a sharing bar with Stop | web-platform-ui |  | |
| 3 | `download-multiple-permission` | Automatic (multiple) downloads site permission | media-downloads |  | |
| 3 | `pw-export-csv` | Passwords: export to CSV | passwords-autofill-sync |  | |
| 3 | `devtools-dock-sides` | DevTools dock positions: right, bottom, left, undocked | devtools-extensions-internals |  | |
| 3 | `windows-agent-paths` | Windows: agent app config paths and the MCP manifest dir | platform |  | |
| 3 | `undo-close-split-entries` | Undo close tab and Mod+Shift+T bring back the split and record the tab's navigation entries | todo-a |  | |
| 3 | `content-driven-frame-rate` | Content-driven frame rate in wubble_gfx::pacing | todo-b |  | |
| 3 | `address-paste-and-go` | Address field context menu: Paste and go, Paste and search, copy and edit rows | navigation |  | |
| 3 | `ctx-menu-link-page-frame` | Context menu: the missing link, selection, page and frame rows | page-interaction |  | |
| 3 | `routing-rule-integrity` | Routing rules survive Space/container renames and show broken rules | privacy-security |  | |
| 4 | `media-in-use-indicator` | Camera and microphone in-use indicators on the tab and address pill | web-platform-ui |  | |
| 4 | `download-resume-restart` | Resume interrupted downloads after a restart | media-downloads | yes | |
| 4 | `pw-checkup` | Password Checkup: weak and reused passwords, local; compromised behind an off-by-default setting | passwords-autofill-sync |  | |
| 4 | `devtools-inspect-docked` | Inspect-at-point, Console and element picker in the docked panel | devtools-extensions-internals |  | |
| 4 | `windows-shell-integration` | Windows taskbar and window integration: AppUserModelID, jump list, snap layouts | platform |  | |
| 4 | `nav-history-restore-patch` | Engine patch: restore a tab's back/forward stack (reopen closed tab, session restore) | todo-a | yes | |
| 4 | `macos-opaque-window` | macOS: opaque window where the shell covers it | todo-b |  | |
| 4 | `nav-history-menus` | Back and forward history list (long press or right-click), middle-click to new tab | navigation |  | |
| 4 | `find-bar-parity` | Find bar: match case, survives reload, prefill from selection | page-interaction |  | |
| 4 | `site-settings-page` | Chrome-style Site settings: defaults per permission, per-site detail, add exceptions | privacy-security |  | |
| 4 | `media-device-choice` | Choose which camera and microphone a site gets | web-platform-ui |  | |
| 4 | `download-speed-eta` | Download speed and time remaining in rows | media-downloads |  | |
| 4 | `pw-notes-details` | Passwords: notes, per-login details and Chromium-style edit | passwords-autofill-sync |  | |
| 4 | `remote-debug-inspect-page` | Remote debugging UI: a wubble://inspect targets page | devtools-extensions-internals |  | |
| 4 | `windows-installer` | Windows packaging: MSI, winget manifest and the updater's Windows install | platform |  | |
| 5 | `orb-shell-wiring` | Wire the thinking orb into the shell: agent mark, activity header, approval sheet | todo-a |  | |
| 5 | `fullscreen-page-alone` | Page fullscreen draws the page alone | todo-b |  | |
| 5 | `nav-stop-reload` | Reload button turns into Stop while loading; reload menu with hard reload and empty cache | navigation |  | |
| 5 | `zoom-indicator` | Zoom indicator and zoom controls like Chrome's omnibox zoom bubble | page-interaction |  | |
| 5 | `content-settings` | Non-prompt content settings: JavaScript, images, sound, insecure content, sensors, downloads, JIT | privacy-security |  | |
| 5 | `perm-per-container` | Site permission decisions per container, not per origin | web-platform-ui |  | |
| 5 | `download-auto-open` | Open when done, always open this type, open safe files | media-downloads |  | |
| 5 | `pw-never-saved-manage` | Passwords page: never-saved sites list and Manage passwords entry points | passwords-autofill-sync |  | |
| 5 | `ext-inspect-views` | DevTools for extension popups, service workers and options pages | devtools-extensions-internals |  | |
| 5 | `default-browser` | Default browser: status, a Settings row, and registration on Linux, macOS and Windows | platform |  | |
| 5 | `orb-gravity-warp` | Thinking orb: port the gravity cursor warp | todo-a |  | |
| 5 | `scene-damage-shows-through` | Scene renders mark only what the scene shows through | todo-b |  | |
| 5 | `swipe-nav-macos-windows` | History swipe on macOS and Windows touchpads (and X11) | navigation |  | |
| 5 | `window-print-route` | Page window.print() opens wubble's print sheet on every platform | page-interaction |  | |
| 5 | `page-info-popover` | Page info popover from the lock: connection, permissions, cookies, site settings | privacy-security |  | |
| 6 | `web-notifications` | Web notifications shown by wubble, with click-to-tab | web-platform-ui |  | |
| 6 | `download-location-setting` | Downloads settings section with a folder picker | media-downloads |  | |
| 6 | `pw-os-reauth` | OS re-authentication before revealing, copying, exporting or editing secrets | passwords-autofill-sync |  | |
| 6 | `ext-permission-prompts` | Extensions P4: permission requests answered in wubble's UI | devtools-extensions-internals | yes | |
| 6 | `macos-open-urls` | macOS: open links and files from other apps, Dock reopen and Dock menu | platform |  | |
| 6 | `gallery-settle-spring` | Gallery: shoot a Settings section only after the page's jump has landed | todo-a |  | |
| 6 | `ambience-damage-reach` | Page damage rebuilds the page blur only within the ambience's reach | todo-b |  | |
| 6 | `history-page-site-actions` | History page: row menu (more from this site, remove, delete site), bulk select and delete | navigation |  | |
| 6 | `save-page-complete` | Save page as: Webpage, Complete (HTML plus resources folder) | page-interaction |  | |
| 6 | `cert-error-hardening` | Certificate interstitial: Advanced section, non-overridable errors, per-container exceptions | privacy-security |  | |
| 6 | `geolocation-provider` | A working location provider for navigator.geolocation | web-platform-ui |  | |
| 6 | `download-lifetime` | Downloads outlive their tab; warn on quit with downloads running | media-downloads |  | |
| 6 | `http-auth-passwords` | Save and fill HTTP authentication (Basic/Digest) logins | passwords-autofill-sync |  | |
| 6 | `ext-site-access` | Extensions: site access controls (on click, on specific sites, on all sites) | devtools-extensions-internals | yes | |
| 6 | `macos-menu-bar` | macOS: a native menu bar with every command | platform |  | |
| 7 | `translate-detect-offer` | Page translation: language detection, the offer, rules, state machine and in-page DOM translation | todo-a |  | |
| 7 | `hidden-tabs-free-gpu` | Hidden tabs and idle glass free their GPU targets | todo-b |  | |
| 7 | `error-panel-completeness` | Error panel: full error table, HSTS refusals, details, HTTP error pages, form resubmission | navigation |  | |
| 7 | `touch-input` | Touchscreen input reaches pages as touch (scroll, tap, long-press, pinch) | page-interaction |  | |
| 7 | `client-cert-selection` | Client certificate selection sheet (mutual TLS) | privacy-security |  | |
| 7 | `protocol-handlers` | navigator.registerProtocolHandler and the handlers list | web-platform-ui |  | |
| 7 | `download-button-progress` | Download button progress ring and auto-opening bubble | media-downloads |  | |
| 7 | `address-autofill` | Address, name, email and phone autofill with an Addresses page | passwords-autofill-sync |  | |
| 7 | `ext-popup-shell` | Extensions shell: popup auto-size, tooltip, action menu, pinning, embedded options | devtools-extensions-internals |  | |
| 7 | `macos-services` | macOS: Services and Look Up for page and shell selections | platform |  | |
| 7 | `translate-bergamot-backend` | Page translation: Bergamot helper process and opt-in model downloads | todo-a |  | |
| 7 | `skia-cache-hygiene` | Skia resource cache hygiene in the vendored renderer | todo-b |  | |
| 7 | `offline-auto-reload` | Error panel: auto-reload when the network comes back | navigation |  | |
| 7 | `touch-selection-handles` | Touch text selection handles and the quick menu | page-interaction |  | |
| 7 | `cookie-controls` | Third-party cookie blocking, per-site cookie exceptions, clear on exit | privacy-security |  | |
| 8 | `web-share` | navigator.share: a share sheet | web-platform-ui |  | |
| 8 | `downloads-page-parity` | Downloads page and popover rows: Chrome parity | media-downloads |  | |
| 8 | `card-autofill` | Payment card autofill with a Payment methods page | passwords-autofill-sync |  | |
| 8 | `ext-tabs-getcurrent` | Extensions: tabs.getCurrent and getViews for non-tab views (Bitwarden popup width) | devtools-extensions-internals | yes | |
| 8 | `instance-handoff` | Second start: many URLs, local files, the working directory and the activation token | platform |  | |
| 8 | `translate-selection-reader` | Page translation: translate selection and reader-mode translation | todo-a |  | |
| 8 | `gpu-restart-recovery` | Recover page layout after a GPU process restart | todo-b |  | |
| 8 | `cert-viewer-security-state` | Connection security: certificate viewer, Not secure and mixed-content states in the address pill | navigation |  | |
| 8 | `pen-input` | Pen and stylus input as PointerEvents (pressure, tilt, eraser, hover) | page-interaction |  | |
| 8 | `site-data-viewer` | See and delete all site data per container | privacy-security |  | |
| 8 | `web-app-install` | Install a site as an app (PWA) and open it in an app window | web-platform-ui |  | |
| 8 | `download-os-progress` | Download progress on the Dock, taskbar and launcher | media-downloads |  | |
| 8 | `webauthn-security-keys` | WebAuthn and passkeys through a shim, with roaming security keys (CTAP2 over USB HID) | passwords-autofill-sync |  | |
| 8 | `ext-live-management` | Extensions: install, enable, disable, remove and reload without a restart | devtools-extensions-internals | yes | |
| 8 | `chrome-switches` | Command-line switches Chrome users expect | platform |  | |
| 9 | `ocr-pipeline-text-layer` | Live text in images: OCR job machine, image fetch, cache and selectable text layer | todo-a |  | |
| 9 | `blocker-startup-gate` | Content blocker start-up gate holds the first navigation | todo-b |  | |
| 9 | `internal-pages-chrome-urls` | Internal URLs: chrome:// diagnostics, about: aliases, wubble://about index, wubble://version | navigation |  | |
| 9 | `x11-touchpad-gestures` | X11: touchpad pinch, history swipe and lock keys | page-interaction |  | |
| 9 | `clear-data-dialog` | Clear browsing data dialog: time range by kind, per container | privacy-security |  | |
| 9 | `credential-mgmt-passwords` | Credential Management API for passwords (PasswordCredential) from wubble's vault | web-platform-ui |  | |
| 9 | `download-retry-origin` | Retry and Save As from the originating tab with its referrer | media-downloads |  | |
| 9 | `passkeys-macos-platform` | macOS platform passkeys (iCloud Keychain) via AuthenticationServices | passwords-autofill-sync |  | |
| 9 | `ext-container-access` | Extensions: allow per container and in agent sessions (Chrome's 'Allow in Incognito') | devtools-extensions-internals | yes | |
| 9 | `updater-launch-rollback` | Updater: roll back a build that cannot even load | platform |  | |
| 9 | `ocr-backends` | Live text in images: Vision backend on macOS, PP-OCR over ONNX Runtime on Linux and Windows | todo-a |  | |
| 9 | `blocker-scriptlets-at-start` | Blocker scriptlets run at document start | todo-b |  | |
| 9 | `experiments-flags` | Experiments (flags-like) section: user-chosen Chromium features and switches | navigation |  | |
| 9 | `win-touchpad-gestures` | Windows: precision touchpad pinch-zoom and history swipe | page-interaction |  | |
| 9 | `https-upgrade-mode` | HTTPS-First and HTTPS-Only mode | privacy-security |  | |
| 10 | `keyboard-lock-hint` | Keyboard Lock in fullscreen: system keys reach the page, hold Esc to exit | web-platform-ui |  | |
| 10 | `media-session-complete` | Media Session API and OS controls: seek, position, raise, active session | media-downloads |  | |
| 10 | `passkeys-windows-hello` | Windows Hello passkeys via webauthn.dll | passwords-autofill-sync |  | |
| 10 | `ext-webstore-install` | Chrome Web Store install and updates, behind an off-by-default setting | devtools-extensions-internals |  | |
| 10 | `linux-system-packages` | Linux system packages: .deb, .rpm and an AUR PKGBUILD with the setuid sandbox | platform |  | |
| 10 | `agent-frame-sessions` | MCP tools inside frames: flattened DevTools sessions for out-of-process frames, live layout for same-process frames | todo-a |  | |
| 10 | `ext-msg-locale-names` | Resolve __MSG_ names from extension _locales | todo-b |  | |
| 10 | `reading-list` | Reading list: save for later, read and unread, page and sidebar access | navigation |  | |
| 10 | `secure-dns` | Secure DNS (DNS-over-HTTPS) setting | privacy-security |  | |
| 10 | `eyedropper` | EyeDropper API and the colour picker's eyedropper | web-platform-ui |  | |
| 10 | `media-hub` | Global media controls (Chrome's media hub) | media-downloads |  | |
| 10 | `apple-passwords-helper` | Apple Passwords passthrough (macOS helper protocol client) | passwords-autofill-sync |  | |
| 10 | `ext-commands` | Extensions: keyboard shortcuts (chrome.commands) and the shortcuts page | devtools-extensions-internals | yes | |
| 10 | `flatpak` | Flatpak package with zypak | platform |  | |
| 10 | `ax-mirror-scroll-offset` | Accessibility mirror: correct boxes after the page scrolls (screen-reader pane and MCP) | todo-a |  | |
| 11 | `touch-kit-port` | Port the remaining touch-kit pieces: pin, download button, dock, checkbox mask | todo-b |  | |
| 11 | `bookmarks-bar` | Bookmarks bar | navigation |  | |
| 11 | `privacy-prefs-table` | Privacy preferences table: Do Not Track, preload pages, ad privacy, WebRTC IP | privacy-security |  | |
| 11 | `color-input-macos` | <input type=color> on macOS (native colour panel) | web-platform-ui |  | |
| 11 | `audible-probe-frames` | Audible probe across iframes and WebAudio on a stock engine | media-downloads |  | |
| 11 | `import-chromium-family` | Import from Chrome, Chromium, Helium, Brave, Edge, Vivaldi and Arc: bookmarks, history, open tabs | passwords-autofill-sync |  | |
| 11 | `ext-context-menus` | Extensions: chrome.contextMenus items in the page and action menus | devtools-extensions-internals | yes | |
| 11 | `homebrew-cask` | Homebrew cask for macOS | platform |  | |
| 11 | `agent-tab-bfcache-prerender` | Agent session tabs: drop back/forward cache and prerenders when a tab joins a session | todo-a |  | |
| 11 | `film-rim-remaining` | Film rim on the remaining glass controls | todo-b |  | |
| 11 | `bookmarks-html-import-export` | Bookmarks: import and export the Netscape HTML file | navigation |  | |
| 11 | `global-privacy-control` | Global Privacy Control (Sec-GPC header and navigator.globalPrivacyControl) | privacy-security |  | |
| 11 | `http-auth-vault` | HTTP sign-in sheet: fill from and save to the password vault | web-platform-ui |  | |
| 11 | `tab-alert-indicators` | Tab alert indicators: recording, screen capture, PiP | media-downloads |  | |
| 11 | `import-chromium-secrets` | Import passwords and cookies from Chromium-family browsers (os_crypt decryption) | passwords-autofill-sync |  | |
| 12 | `ext-side-panel` | Extensions: chrome.sidePanel in a wubble side pane | devtools-extensions-internals | yes | |
| 12 | `harness-apps-and-windows` | Agent apps: VS Code, Copilot CLI, Goose and Windsurf on macOS as written formats; Windows paths | todo-a |  | |
| 12 | `kit-lock-baseline-empty` | Empty the kit lock baseline | todo-b |  | |
| 12 | `bookmark-editing-complete` | Bookmark editing: edit URL, open all, bookmark all tabs, folder delete confirmation and undo | navigation |  | |
| 12 | `download-source-marks` | Mark downloaded files with their origin (quarantine / MOTW / xattr) | privacy-security |  | |
| 12 | `pip-page-api` | Page Picture-in-Picture API and auto-PiP through wubble's Pip window | media-downloads |  | |
| 12 | `import-firefox-secrets` | Import passwords and cookies from Firefox and Zen directly (no CSV) | passwords-autofill-sync |  | |
| 12 | `ext-omnibox` | Extensions: chrome.omnibox keyword mode in the command bar | devtools-extensions-internals | yes | |
| 12 | `agent-nested-scroll` | Wheel over a nested scroller scrolls the pane, not the document (agent `scroll` and the user's wheel) | todo-a |  | |
| 12 | `voice-glow-kit` | Voice glow shader in the kit (no microphone) | todo-b |  | |
| 12 | `dangerous-site-interstitial` | Dangerous-site interstitial for blocker badware/malware document blocks | privacy-security |  | |
| 12 | `pip-controls-complete` | Picture-in-Picture window controls parity | media-downloads |  | |
| 12 | `import-zen-arc-gaps` | Import: Zen legacy workspace tables, Little Arc, Arc pinned/favourites details | passwords-autofill-sync |  | |
| 12 | `ext-native-messaging` | Extensions: native messaging hosts found where desktop apps install them | devtools-extensions-internals |  | |
| 12 | `agent-wait-network-idle` | `wait_for load` follows in-page navigations' fetches: network idle through DevTools events | todo-a |  | |
| 13 | `feel-stretch-squash` | Feel pass: stretch and squash on every pressable and moving surface | todo-b |  | |
| 13 | `permission-auto-revoke` | Auto-revoke permissions of unused sites (Safety check) | privacy-security |  | |
| 13 | `document-pip` | Document Picture-in-Picture (CEF patch plus host window) | media-downloads | yes | |
| 13 | `import-app-wiring` | Import UI: pick a browser and profile, choose data, apply, show the summary | passwords-autofill-sync |  | |
| 13 | `ext-tabs-windows-events` | Extensions: the missing tabs/windows events, bounds and windows.create | devtools-extensions-internals | yes | |
| 13 | `agent-listbox-options` | `read_page` lists options, list boxes and multiple selects; `select` can add a choice | todo-a |  | |
| 13 | `feel-liquid-glass-floating` | Feel pass: liquid glass under every floating surface | todo-b |  | |
| 13 | `media-context-menu` | Media context menu: Picture in Picture, Mute, video frame rows | media-downloads |  | |
| 13 | `sync-core` | Sync engine: end-to-end encrypted, to a folder the user picks, off by default | passwords-autofill-sync |  | |
| 13 | `tm-renderer-pids` | Task manager: renderer pids under the sandbox and Chrome's process labels | devtools-extensions-internals | yes | |
| 13 | `upload-path-check-worker` | `upload_file`: one path-check worker instead of a thread per call | todo-a |  | |
| 13 | `feel-ring-and-beam` | Feel pass: glowing ring and border beam on focus, loading and new | todo-b |  | |
| 13 | `hdr-engine-f16` | HDR: CEF patch for F16 shared-texture capture | media-downloads | yes | |
| 13 | `sync-collections` | Sync collections: bookmarks, history, passwords, addresses, settings, open tabs | passwords-autofill-sync |  | |
| 13 | `tm-columns` | Task manager: Chrome's columns, sorting and row actions | devtools-extensions-internals |  | |
| 14 | `evaluate-group-per-call` | `evaluate`: a DevTools object group per call, released on timeout and on session end | todo-a |  | |
| 14 | `vault-unlock-worker` | Vault unlock off the store thread | todo-b |  | |
| 14 | `hdr-shell-path` | HDR: shell import, Skia renderer and surface colour space | media-downloads |  | |
| 14 | `sync-webdav` | Sync transport: WebDAV (Nextcloud, ownCloud, any DAV server) | passwords-autofill-sync |  | |
| 14 | `a11y-screen-reader-detect` | Screen reader detection on macOS and Windows | devtools-extensions-internals |  | |
| 14 | `move-tab-to-space-submenu` | Tab menu: two-level "Move tab to" listing user Spaces; moving out of an agent Space makes an ordinary tab | todo-a |  | |
| 14 | `crash-report-issue` | Opt-in crash reports that open a GitHub issue | todo-b |  | |
| 14 | `widevine-cdm-loader` | Widevine CDM loader and Protected content setting | media-downloads |  | |
| 14 | `sync-settings-ui` | Sync settings: set up, passphrase, destination, data types, devices | passwords-autofill-sync |  | |
| 14 | `a11y-caret-browsing` | Caret browsing (F7) | devtools-extensions-internals |  | |
| 14 | `clear-agent-space-ask` | Clearing an agent Space with tabs asks first: Close anyway or Move tabs | todo-a |  | |
| 14 | `cast-media-router` | Cast a video to a Chromecast or DIAL device (CefMediaRouter) | media-downloads |  | |
| 14 | `send-tab-to-device` | Send tab to your devices, and tabs from other devices, over sync | passwords-autofill-sync |  | |
| 14 | `spell-menu-settings` | Spell check: Add to dictionary, a toggle, a custom word list and context-menu rows | devtools-extensions-internals |  | |
| 14 | `autofill-offer-after-submit` | Autofill: offer the login form a page shows right after a submit | todo-a |  | |
| 15 | `live-caption-local` | Live captions from on-device speech recognition | media-downloads |  | |
| 15 | `spell-languages-download` | Spell check in other languages: opt-in dictionary download and language list | devtools-extensions-internals |  | |
| 15 | `agent-signin-storage-copy` | Agent sign-ins: copy localStorage/IndexedDB, keep CHIPS partition keys, print the copy's report | todo-a |  | |
| 15 | `pdf-print-handler` | Linux print handler: window.print() and the PDF viewer's print | media-downloads |  | |
| 15 | `read-aloud-os-voice` | Read aloud with the OS voice (page, selection, Reader view) | devtools-extensions-internals |  | |
| 15 | `agent-hidden-view-throttle` | Agent views no window shows render at a trickle, at full rate only when asked | todo-a |  | |
| 15 | `pdf-download-setting` | "Download PDFs instead of opening them" site setting | media-downloads |  | |
| 15 | `read-aloud-model-voice` | Read aloud: bundled or opt-in on-device neural voice | devtools-extensions-internals |  | |
| 15 | `agent-airlock-prompts` | Agent airlocks: ask the human for permissions, downloads, external links, popups and off-list navigation | todo-a |  | |
| 15 | `print-sheet-parity` | Print sheet: preview, two-sided, colour, pages per sheet, headers | media-downloads |  | |
| 15 | `reader-toolbar-options` | Reader mode: Chrome's reading-mode controls in a toolbar | devtools-extensions-internals |  | |
| 15 | `agent-write-window` | Agent guardrails: hold non-GET requests after an action (write window) and flag hidden content | todo-a |  | |
| 15 | `media-dash-periods-codec` | Media downloader: multi-period DASH and codec-aware default quality | media-downloads |  | |
| 15 | `agent-clipboard-isolation` | Clipboard isolation for agent tabs on every platform | todo-a |  | |
| 15 | `media-detect-sw-iframe` | Media detection for service-worker answers and embedded YouTube | media-downloads |  | |
| 16 | `list-pages-keyboard-select` | Built-in list pages: keyboard open, arrow keys, selection, delete selection, undo, copy address | todo-a |  | |
| 16 | `video-save-button-states` | Video strip save button: wording and in-progress state | media-downloads |  | |
| 16 | `agent-pages-icons-log-clear` | Agent sessions: no icon-table entries for their pages; the activity log can be cleared | todo-a |  | |
| 16 | `spaces-popup-row-actions` | Spaces popup: rename and delete on a row, aligned to the Space tile's left edge | todo-a |  | |
| 16 | `command-bar-exit-clicks` | Command bar: the shrinking panel no longer takes clicks during its exit | todo-a |  | |
| 16 | `space-inline-rename` | In-place rename for a new Space in the sidebar | todo-a |  | |
| 16 | `keybinding-second-chords` | Shortcuts: both chords editable, rebinding keeps the second chord, Linux alternates where GNOME takes the chord | todo-a |  | |
| 16 | `start-tiles-kit` | Start page tiles: the kit tile look and its hover spring | todo-a |  | |
| 16 | `settings-reset-fold-search` | Settings: Reset per section, rows folding while searching, searchable Filter lists | todo-a |  | |
| 16 | `gallery-hover-menu-scenes` | Gallery: scenes for the hover preview card and the sidebar's empty-area menu; a stable page-menu position | todo-a |  | |
| 16 | `shell-insert-key-clipboard` | Shell fields: Shift+Insert paste, Ctrl+Insert copy, Shift+Delete cut | todo-a |  | |
| 16 | `overlay-dark-hue-fix` | Fix the hue under the command bar in dark overlay mode | todo-a |  | |
| 16 | `autofill-chip-field-rect` | Autofill key chip at the focused field, pane-aware in split view | todo-a |  | |
| 16 | `settings-narrow-wrap` | Narrow Settings: slider and select rows wrap to full width | todo-a |  | |
| 16 | `focuskeys-repeat` | FocusKeys with native-button false fires only once on a held Enter or Space | todo-a |  | |
| 17 | `spaces-button-menu-focus` | A Space button opened by keyboard gives the Spaces menu the keyboard focus | todo-a |  | |
| 17 | `popover-thumb-touch-area` | Popover scroll thumbs no longer cover the rows' right edge | todo-a |  | |
| 17 | `settings-select-fit` | Settings Select fits its longest label (no clipped value) | todo-a |  | |
| 17 | `popover-entrance-partial` | A popover's entrance dirties only its own region, so partial rendering helps | todo-a |  | |
| 17 | `window-compositor-blur` | Transparent window with compositor blur: macOS vibrancy, KDE blur | todo-a |  | |
| 17 | `windows-ci-test-fixes` | Windows CI: make the four failing tests pass or gate them | todo-a |  | |
| 17 | `input-fling-touch` | Input: kinetic wheel fling on Linux and real touch input | todo-a |  | |
| 17 | `cursor-custom-altgr` | Cursors: CSS url() images and zoom cursors; the AltGr modifier flag | todo-a |  | |
| 17 | `ime-caret-underlines` | IME: caret rect before the first composed character, underline styles and replacement range | todo-a |  | |
| 17 | `audible-signal-patch` | Engine patch: a real audible signal per tab (iframes, shadow DOM, WebAudio) | todo-a | yes | |
| 17 | `mac-key-repeat-patch` | Engine patch: macOS key repeats reach the page with repeat true | todo-a | yes | |
| 17 | `web-a11y-subtree` | Web content as a real accessibility subtree in the OS tree | todo-a |  | |
| 17 | `windows-sandbox-bootstrap` | Windows sandbox: wubble as a client DLL behind CEF's bootstrap.exe | todo-a |  | |
