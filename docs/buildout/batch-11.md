# Build-out batch 11

15 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `touch-kit-port`: Port the remaining touch-kit pieces: pin, download button, dock, checkbox mask

Area: todo-b. Needs an engine patch: no.

The design's touch kit (`~/Dev/wubble-wt/ref/the-wallpaper-project/src/wubble/pages/touch.tsx`: `Pin` at about line 257, `Download` at 316, `Dock` at 377 with its gliding tooltip, `Checks` at 555 with the check box's mask grow; styles in touch.css) has pieces that the kit does not have yet (crates/wubble-ui/ui/kit/controls has check-box.slint without the mask grow, and no pin, download button or dock).

To build: `git pull` the design clone first. Port each piece into crates/wubble-ui/ui/kit/controls/ as its own file headed `// design: pages/touch.tsx:<lines> @<commit>` (and touch.css lines). Take the values from the generated `ui/kit/gen/*.slint` and `wubble_core::kit_gen` by rerunning `node tools/fidelity/extract.mjs` if new tokens appear. Put each loop's physics in `wubble_core::controls` (the existing registry pattern, decisions "Component physics"), with unit tests. The pieces: Pin (the pin toggle's press and settle), Download (the progress ring, done check and bounce), Dock (an icon row with the tooltip gliding between hovered icons on a spring), and the check box's mask-grow fill. Then use them: Pin in the tab row's pin affordance (kit chrome/tab-row.slint), Download as the sidebar footer's downloads button with live progress from host/downloads.rs, the check box wherever check-box.slint is used. Dock goes in the gallery only unless a shell place is decided. Add gallery scenes (crates/wubble/src/gallery_smoke.rs) and keep the kit lock passing. All platforms.

## `bookmarks-bar`: Bookmarks bar

Area: navigation. Needs an engine patch: no.

Chrome's bookmarks bar is a strip under the toolbar. It shows the 'Bookmarks bar' folder's items as chips with favicons, with folders as drop-down menus and an overflow chevron. Mod+Shift+B toggles it, it shows on the new-tab page by default, items can be dragged in, out and around, and right-clicking a chip gives Open, Edit and Delete. wubble has no bookmarks bar (TODO M2 'Bookmarks: drag reorder, a bookmarks bar'), and no top toolbar by decision ('Layout'). Bookmarks have a single root (crates/wubble-store sql 001 bookmarks.parent_id NULL).

1. Store and core: a special root folder, 'Bookmarks bar' (a well-known folder created on migration, its id in settings). Chrome HTML and profile imports map their bar folder to it. pages::bookmark_tree shows it first.
2. Setting `ui.bookmarks_bar`: Never, New tab only (default) or Always. Action::ToggleBookmarksBar bound to Mod+Shift+B in keys.rs.
3. Placement: the decision rules out a toolbar, so the bar is a slim strip across the top of the content area, above the page card and split-aware. Compose it from kit chrome parts (favs chips, menu for folders, the overflow as a menu) and mark it `// derived:` on docs/fidelity/derived-review.md. The owner must approve the look before merge.
4. A folder chip opens a Menu::Shell of its children (crates/wubble/src/menu.rs), nested. Chips take drops of links and tabs, through the drag machine in wubble_core::drag and dnd and the bookmark-drop path in the 'Drops onto the shell's pages' decision. The page area shrinks by the strip's height (host/panes.rs).

Unit tests for the overflow fit and the visibility rule.

## `privacy-prefs-table`: Privacy preferences table: Do Not Track, preload pages, ad privacy, WebRTC IP

Area: privacy-security. Needs an engine patch: no.

Chrome's privacy settings are profile prefs: "Send a Do Not Track request" (`enable_do_not_track`, off by default), "Preload pages" (`net.network_prediction_options`), "Allow sites to check if you have payment methods saved" (`payments.can_make_payment_enabled`), the Ad privacy toggles (`privacy_sandbox.m1.topics_enabled`, `privacy_sandbox.m1.fledge_enabled`, `privacy_sandbox.m1.ad_measurement_enabled`), and the WebRTC IP handling policy (`webrtc.ip_handling_policy`, with no UI in Chrome; Brave exposes it). wubble sets one pref only (`webkit.webprefs.minimum_font_size`, crates/wubble-engine/src/view.rs:848) and exposes none of these.

Build:
- wubble-core: a data table `privacy::PREFS: &[(setting key, chromium pref, Value mapping)]` with matching SettingDefs in settings.rs (Section::Privacy). Defaults: DNT off (Chrome), preload "standard" (Chrome's default, since no request of wubble's own is added), can-make-payment on, Privacy Sandbox APIs off (Chrome asks for consent; wubble never shows the consent), WebRTC `default`. Unit-test that every row has a SettingDef and a valid mapping.
- Engine: a generic `Engine::set_profile_pref(container, name, value: PrefValue)` that lifts the `set_preference` code out of view.rs to a request-context helper in storage.rs or app.rs. Apply the whole table when each container context comes up (`Profiles::get`) and on each setting change, and keep the min-font-size path on the same helper.
- UI: the rows come from the settings table automatically.

## `color-input-macos`: <input type=color> on macOS (native colour panel)

Area: web-platform-ui. Needs an engine patch: no.

On macOS, Chrome opens <input type=color> in the native NSColorPanel (ColorChooserMac, through WebContentsDelegate::OpenColorChooser). Linux and Windows use Blink's in-page popup, which wubble already draws as a popup widget (PageEvent::PopupShown, crates/wubble-ui/ui/components/popup-layer.slint). AlloyBrowserHostImpl does not override OpenColorChooser, and the default returns nullptr, so on the macOS build a colour input opens nothing.
Build in the repo: (1) On macOS only, add a main-world content-script channel `color` (wubble-helper/src). It catches a trusted click or Enter/Space activation on an enabled input[type=color], calls preventDefault, and posts {value, list suggestions} with a request id to the browser. (2) On the browser side, a pure machine answers once per request and closes on navigation or blur. It shows NSColorPanel through objc2 (crates/wubble already uses objc2 for macOS) with the starting colour, and sends each change back as `value` (the script sets .value and dispatches `input`). When the panel closes, the script dispatches `change`. (3) Leave Linux and Windows alone. (4) docs/macos.md: add colour input to the Mac console hand checks list.

## `audible-probe-frames`: Audible probe across iframes and WebAudio on a stock engine

Area: media-downloads. Needs an engine patch: no.

Until a patched engine ships, wubble's audible probe (crates/wubble-engine/src/media.rs AUDIBLE_PROBE, run every 2 s as a main-frame eval) sees only `<audio>` and `<video>` in the top document. Embedded players (YouTube or Spotify iframes, which are cross-origin OOPIFs), media inside shadow roots and WebAudio games never light the sound badge or protect the tab from sleep.

What to build, without an engine change: (1) Shadow DOM. The probe walks open shadow roots as kit.js `shadowVideos` already does, capped as kit.js caps it. (2) WebAudio. kit.js wraps AudioContext and webkitAudioContext construction and `resume`, counting contexts in the `running` state with a non-zero destination connection. The probe reports `audible` when one is running and was started after a gesture. (3) iframes. The probe asks every frame, not just the main one: same-process frames through CEF frame iteration and `View::run_script_in`, and out-of-process frames through DevTools `Target.setAutoAttach {flatten:true}` and `Runtime.evaluate` with each attached session (the view's cdp() channel, crates/wubble-engine/src/cdp.rs). (4) Keep the ProbeState machine to one probe round in flight per view, and OR the per-frame answers in a pure fold with unit tests. When the engine audible patch exists (marker found), all of this is skipped. Measure by hand that the extra frames do not raise idle cost. Nothing in tests asserts timing. All platforms.

## `import-chromium-family`: Import from Chrome, Chromium, Helium, Brave, Edge, Vivaldi and Arc: bookmarks, history, open tabs

Area: passwords-autofill-sync. Needs an engine patch: no.

Chrome imports bookmarks, history and open tabs from other browsers; wubble-import (crates/wubble-import) reads only Firefox/Zen profiles and Arc's `StorableSidebar.json` (TODO.md: "Import: ... Arc history, Little Arc, Helium/Chromium"). Build in crates/wubble-import: (1) discovery `discover.rs`: a `Browser` table for Chromium-family roots per OS (Linux `~/.config/google-chrome`, `chromium`, `net.imput.helium`, `BraveSoftware/Brave-Browser`, `microsoft-edge`, `vivaldi`; macOS `Library/Application Support/Google/Chrome`, `Chromium`, `net.imput.helium`, `Arc/User Data`, ...; Windows `%LOCALAPPDATA%/Google/Chrome/User Data`, ...), profiles from `Local State` JSON `profile.info_cache` (name, directory); extend `wubble_core::import::Source` with `Chrome, Chromium, Helium, Brave, Edge, Vivaldi` (Arc exists). (2) `chromium/bookmarks.rs`: the `Bookmarks` JSON (`roots.bookmark_bar/other/synced`, `date_added` in microseconds since 1601) to `BookmarkNode`. (3) `chromium/history.rs`: `History` SQLite copied to a temp file under the profile's cache first (as `snapshot_places` does, it is locked while the browser runs): `urls` + `visits` -> `ImportedVisit` (typed from `typed_count`/transition `TYPED`). (4) `chromium/session.rs`: SNSS `Sessions/Session_*` (or legacy `Current Session`), a port of `components/sessions/core/session_service_commands.cc` command ids (BSD-3): windows, tabs, pinned state, tab groups, current navigation URL/title -> `Layout` (one Space per window, a group becomes a folder). (5) Arc: history and tabs through the same readers from its `User Data`. Pure parsers on bytes, fixtures under crates/wubble-import/tests/fixtures, unit tests.

## `ext-context-menus`: Extensions: chrome.contextMenus items in the page and action menus

Area: devtools-extensions-internals. Needs an engine patch: yes.

In Chrome, extension items created with `chrome.contextMenus.create` appear in the page context menu, filtered by `contexts`, `documentUrlPatterns` and `targetUrlPatterns`. More than one item per extension is grouped under the extension's name, radio and checkbox types and separators are honoured, and a click fires `contextMenus.onClicked(info, tab)`. Items with the `action` context appear in the action button's menu. wubble's page menu is its own table (`crates/wubble/src/menu.rs`, `ROWS`), and welding passes CEF's default model (`surface::menu_items`), which for Alloy-style windowless browsers carries no extension items. Patch `56-cef-ext-context-menus.patch`: table functions `menu_items(browser_id, json_params) -> json` and `menu_execute(browser_id, extension_id, item_id, json_params, checked)`. They are built on `extensions::MenuManager` and `ContextMenuMatcher` (`chrome/browser/extensions/context_menu_matcher.cc`, logic as in `RenderViewContextMenu::AppendAllExtensionItems`), and `MenuManager::ExecuteCommand` with a `content::ContextMenuParams` rebuilt from the JSON (link, src, selection, frame url, editable, media type). Plus a delegate notice `menus_changed`. wubble side: `ext_api.rs` wrappers. `wubble-engine::page::ContextMenu` already holds the fields that need serialising. Core/menu: `entries` gains an extension group (between groups 9 and 10) built from the returned items, with a submenu when an extension has more than one, using the kit menu's submenu support or flattened with the extension name as a header. A `Command::Extension { ext, item }` is answered through `menu_execute` and then a `None` CEF command. Add the action context to the ext-popup-shell row menu. Tests: pure entry building from item JSON.

## `homebrew-cask`: Homebrew cask for macOS

Area: platform. Needs an engine patch: no.

Mac users who live in the terminal install Chrome with `brew install --cask google-chrome`. wubble offers only `install.sh` and the DMG (`scripts/release/package-macos.sh`). To build: `scripts/release/macos/wubble.rb`, a cask that downloads `wubble-<V>-macos-<arch>.dmg` from the GitHub release with `sha256` per arch (`on_arm`/`on_intel`), `app "wubble.app"`, `auto_updates true` (wubble updates itself, so brew must not fight it), and a `zap` stanza for `~/Library/Application Support/wubble`, the caches and the preferences plist for `dev.wubble.browser`. Add a small step to `scripts/release/` (or `crates/wubble-release`) that rewrites the cask's version and sha256 lines from the dist/*.sha256 files when a release is packed. Publishing to a tap (`xuckless/homebrew-tap`) is the owner's step. Document it in docs/release.md next to the npm one. With `auto_updates true` the installed app updates through wubble's own updater: check that `updater/install.rs` `classify_mac` treats `/Applications/wubble.app` installed by brew as managed (it should, being the same layout) and say so in the cask's caveats.

## `agent-tab-bfcache-prerender`: Agent session tabs: drop back/forward cache and prerenders when a tab joins a session

Area: todo-a. Needs an engine patch: no.

TODO line 63. A tab moved into an agent session's Space keeps its back/forward cache entries and any prerender made before. Activating either runs no navigation throttle, so the session's site guard is not asked, and only the commit backstop (`agent_backstop`/`agent_take_back` in `crates/wubble/src/host/agent.rs`) takes the tab back, after the page showed. Fix: when a tab joins a session (the Effect that binds a tab to a session in `wubble_core::agent`, executed in `agent_effect`), and for every tab the session opens, (1) send DevTools `Page.resetNavigationHistory`, which evicts the tab's bfcache entries along with its history, and (2) `Page.setPrerenderingAllowed {isAllowed:false}`, which cancels and blocks prerenders for that page. Both go through `View::cdp`; re-send after a renderer swap. Consider also turning speculation rules off in agent containers through the container's request-context preferences if CEF exposes them. Log each drop to the session's activity log. Add a probe step to the existing `--smoke mcp` scenario code only if it is a one-liner; no new test files.

## `film-rim-remaining`: Film rim on the remaining glass controls

Area: todo-b. Needs an engine patch: no.

The design draws a 1.5 px soap-film ring (`.wb-film`, core/wubble.css:33-40 in the design clone, angle `--wb-film-a`) on every glass control while it is touched, hovered, pressed, focused or open. The kit has it on button.slint (glass kind), the menu and select trigger (menu.slint:595), the field, search capsule, toggle, favs, address pill and some overlays (`kit/motion/film.slint`). The design also puts `wb-film` on the settings confirm button (`settings.tsx:366` `.st-confirm`), the start page's site-tile icons (`start.tsx:48`), the settings search field (`settings.tsx:558`), the found-glass tiers and the touch page's buttons. Some of these kit parts lack it: confirm-button.slint, icon-button.slint, the start page's tiles (kit/pages/start.slint) and the menu rows the TODO names.

To build: `git pull` the design clone. Grep the web source for `wb-film` and list each use against the kit component that translates it. For each one missing, add the `film` binding through the existing `kit/motion/film.slint` piece, lit on the same states the CSS uses (`:hover`, `:active`, `:focus-visible`, `.wb-on`). Update each file's `// design:` header with the lines it now translates. Menu rows get it only if the web source has it; if it does not, say so in the report rather than inventing one. Add a gallery shot of each touched surface, and keep the kit lock baseline from growing (crates/wubble-ui/tests/kit_lock.rs). All platforms.

## `bookmarks-html-import-export`: Bookmarks: import and export the Netscape HTML file

Area: navigation. Needs an engine patch: no.

Chrome's bookmark manager has 'Import bookmarks' and 'Export bookmarks', using the Netscape Bookmark File format (bookmarks_DATE.html: <DL><DT><A HREF ADD_DATE ICON>, <H3> folders, the PERSONAL_TOOLBAR_FOLDER attribute for the bar). Every browser reads and writes it. wubble's importers (crates/wubble-import: places for Firefox and Zen, arc, session) do not handle HTML, and there is no export.

1. crates/wubble-import: a new html.rs with `parse_netscape(&[u8]) -> Result<Vec<BookmarkNode>, ImportError>`. It is a tolerant tag scanner, since real files are not valid HTML: case-insensitive tags, entity decoding, no closing </DT>. It reads ADD_DATE in seconds into added_ms, takes javascript: and place: URLs only as kept-but-unopened, or drops them as is_web_url does, and marks the toolbar folder. Port from Firefox's BookmarkHTMLUtils (MPL-2.0) or write it new.
2. wubble-core: a pure `export_netscape(rows: &[BookmarkRow], now_ms) -> String` with Chrome's exact header (<!DOCTYPE NETSCAPE-Bookmark-file-1>, META charset, TITLE and H1 'Bookmarks') and HTML escaping. It has unit tests that round-trip with the parser.
3. Wiring: the bookmarks page header gets 'Import' and 'Export' kit buttons. Import uses the file chooser path the host already has (crates/wubble/src/chooser.rs) and applies the nodes into a new folder 'Imported' through the store's import apply (wubble-store/src/import.rs, import_marks so a file is not imported twice). Export uses a save dialog and writes the file.
4. Add the 'Import bookmarks', 'Export bookmarks' and 'bookmarks html' keywords to PALETTE.

## `global-privacy-control`: Global Privacy Control (Sec-GPC header and navigator.globalPrivacyControl)

Area: privacy-security. Needs an engine patch: no.

GPC sends `Sec-GPC: 1` on every request and exposes `navigator.globalPrivacyControl === true`. Brave and Firefox ship it, and Chrome has it behind a feature. wubble has neither.

Build:
- Add a setting `privacy.gpc` (bool, default off, Section::Privacy) in crates/wubble-core/src/settings.rs.
- First check whether the prebuilt libcef 156 has Chromium's feature: search the CEF binary for the feature name ("GlobalPrivacyControl") with `strings` on the downloaded libcef. If present, add it to the PageFeatures enable list in crates/wubble-engine/src/app.rs (it takes effect at the next start; say so on the row). Otherwise, use the embedder route:
  (a) A request header. Extend welding's request filter (vendor/welding/src, OnBeforeResourceLoad) and `wubble_engine::filter::RequestAction` with a header set. Simplest is an `extra_headers: &'static [(&str,&str)]` returned by a new `ContentFilter::headers(&RequestInfo)` that defaults to none, mark it a wubble change, and have crates/wubble/src/blocking.rs return Sec-GPC when the setting is on (an Arc atomic read on the IO thread).
  (b) The script: a main-world getter `navigator.globalPrivacyControl` installed in every frame at context creation through the helper's page scripts (crates/wubble/src/pagescripts.rs and wubble-helper), only when on. The engine passes the flag with the frame `hello`, like cosmetic.js.
- Write unit tests for the header decision.

## `http-auth-vault`: HTTP sign-in sheet: fill from and save to the password vault

Area: web-platform-ui. Needs an engine patch: no.

Chrome's HTTP auth dialog ("Sign in: <host> requires a username and password") is pre-filled from the password manager. After a successful sign-in Chrome offers to save the credentials, keyed by the scheme, host and realm (an HTML-less PasswordForm with scheme kBasic/kDigest). wubble draws the sheet (crates/wubble/src/prompt.rs Auth and AuthRequest, host/surfaces.rs) but does not consult the vault (wubble_core::autofill, crates/wubble/src/host/autofill.rs).
Build: (1) Key the vault entries for HTTP auth as signon_realm "<scheme>://<host>:<port>/<realm>", Chromium's form, with a unit test in wubble-core autofill. (2) When AuthEvent::Challenged opens the sheet, pre-fill the user and password fields from the one matching entry, or offer the key menu with several. A locked vault fills nothing and shows the existing unlock path. (3) After an answered challenge, if the next response for that URL is not another 401/407 for the same realm (a pure check fed by the next AuthRequested or LoadEnd), show the existing save prompt for that realm. (4) Proxy challenges (is_proxy) are saved under the proxy host. Agent sessions never get a save prompt, as decided. Passwords are never logged (the existing a_password_is_never_printed test extends to this).

## `tab-alert-indicators`: Tab alert indicators: recording, screen capture, PiP

Area: media-downloads. Needs an engine patch: no.

Chrome's tab strip shows one alert indicator per tab, in priority order: media recording (camera/mic, a red dot), desktop/tab capturing, audio playing, audio muted, Bluetooth, USB, HID, serial and VR connected, and picture-in-picture (chrome/browser/ui/tabs/tab_utils.cc GetTabAlertStatesForContents, BSD). The indicators have tooltips ("This tab is using your camera or microphone"). wubble's wubble_core::tabmarks has only the sound badge (Sound::Playing, Sound::Muted). The busy set (wubble_core::busy) already knows Reason::Capture (a live capture track, from the renderer probes) and Reason::Pip, but no tab row shows them.

What to build: (1) wubble-core/src/tabmarks.rs: a `TabAlert` enum with Chrome's priority (Recording, Capturing, Pip, Playing, Muted), derived in `Marks::of` from the busy set, the mute state and the pip owner. Split Capture into camera/mic versus display capture: extend the probes script (crates/wubble-helper/src/probes.js) to report `getDisplayMedia` tracks under a new key, plus a busy::Reason, kept in the one Reason table. Unit tests for the priority. (2) UI: the badge in the extended row and the rail (the kit's tab row parts) draws the right glyph and tint, with a tooltip and an accessible label. Recording uses Palette's alert tint. If the kit has no glyph for it, it is a derived piece and goes on derived-review.md; the badge press does nothing except for sound. (3) Gallery scenes for each state. All platforms.

## `import-chromium-secrets`: Import passwords and cookies from Chromium-family browsers (os_crypt decryption)

Area: passwords-autofill-sync. Needs an engine patch: no.

Chrome's importer brings saved passwords (and Edge/others bring cookies). wubble-import states "Passwords and cookies are out of scope" (crates/wubble-import/src/lib.rs). Build: (1) `os_crypt` decryption, a port of Chromium `components/os_crypt/sync` (BSD-3): Linux `v11` (key = PBKDF2-HMAC-SHA1(password from Secret Service/KWallet item "<Browser> Safe Storage", "saltysalt", 1 iter, 16 bytes), AES-128-CBC, IV 16 spaces) and `v10` (`peanuts` fallback key); macOS (Keychain item "<Browser> Safe Storage", 1003 iterations; the user gets the system's allow prompt); Windows `v10` (AES-256-GCM key from `Local State` `os_crypt.encrypted_key`, DPAPI `CryptUnprotectData`); `v20` app-bound rows are reported as skipped (not decryptable outside the browser). Read the other browser's keyring entry through the existing `keyring` crate (crates/wubble-vault/Cargo.toml) behind a trait for tests; crates pbkdf2, sha1, aes, cbc (RustCrypto, MIT/Apache). (2) `Login Data` SQLite (copied first): `logins` origin_url, username_value, password_value, date_created, times_used, blacklisted_by_user (-> never-save decisions via `site_permissions`) -> `wubble_vault::ImportRow`, applied with `wubble_vault::apply` so duplicates/conflicts are reported. (3) `Cookies` SQLite: decrypt `encrypted_value` (DB meta version >= 24 strips the 32-byte SHA-256 host prefix), map to welding `Cookie` and write with `set_cookie` (vendor/welding/src/cookies.rs) into the default container's request context; skip expired, keep host-only/secure/httponly/samesite. Cookies are a separate, unchecked-by-default choice in the import UI with a note on what they are. Secrets zeroized; nothing logged.

