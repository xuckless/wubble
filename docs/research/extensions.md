# Extensions on wubble's CEF 156: what works unpatched, what needs a patch

Question: With CEF 156's Chrome runtime and windowless (off-screen) browsers, which parts of Chrome's extension system
work without patching Chromium or CEF, how wubble loads unpacked and .crx extensions, and which gaps need a patch?
Status: Loading (unpacked and .crx), Manifest V3 service workers, runtime messaging and content scripts work and are
smoked (`--smoke extensions`); tabs/windows/action/permission prompts need patches P1 to P4 (section 4).

Date: 2026-11-20. Sources: the CEF checkout `~/Dev/wubble-wt/cef-build/chromium_git/cef` (HEAD 6028375a4, CEF 156.0.30),
Chromium `.../chromium/src` (156.0.8078.96), cef-rs `cef-156.6.0+156.0.30`, samuelmaddock/electron-browser-shell @3817c42,
runs of `--smoke extensions` / `extensions-none` on the own CEF build (`CEF_PATH`, Linux, NVIDIA, headless display).
Tags: **[V]** read in the cited source or seen in a run; **[I]** inferred, to be confirmed before relying on it.

## 1. The answer in one table

| Feature | Unpatched on CEF 156 windowless | Why / what it needs |
|---|---|---|
| Load an unpacked directory | **Yes**: `--load-extension=a,b` at start [V, smoke] | `extension_service.cc:375-470`; refused only under `GOOGLE_CHROME_BRANDING`, Enhanced Safe Browsing or the `command_line` install-type blocklist policy; our build is unbranded. Restart to change. |
| Load a .crx | **Yes**, by unpacking it ourselves and writing the .crx's key into `manifest.json`, then as unpacked [V, smoke] | Chromium's `CrxInstaller` (location `kInternal`, the Web Store's way) is C++ only: reaching it would need a CEF API (a patch). |
| MV3 service worker, `runtime.sendMessage`/`onMessage` | **Yes** [V, smoke: the worker answers the content script] | profile-level, not tied to a tab |
| Content scripts (`document_start`) in wubble's views | **Yes** [V, smoke] | CEF attaches Chrome's tab helpers to Alloy-style (windowless) browsers: `browser_platform_delegate_alloy.cc:206` |
| `storage`, `alarms` | Expected yes [I] | profile-level APIs |
| `scripting.executeScript`, `tabs.get/update/sendMessage` by `tabId` | Partly [I] | CEF's minimal shim: `chrome_extension_util.cc` `cef::GetAlloyTabById`, hooked into `ExtensionTabUtil::GetTabById` (`chrome_browser_extensions.patch`) |
| `tabs.query`, `tabs.on*` events, `windows.*` | **No** [V source] | `TabsQueryFunction::BuildTabList` walks only `Browser` windows' tab lists (`tabs_api.cc` ~1956); `TabsEventRouter` observes `TabListInterface` only. **P1**. **Patched** (section 7): `tabs.query`, `get`, `update`, `remove`, `sendMessage`, `onCreated/onUpdated/onActivated/onRemoved`, `windows.getCurrent/getAll/getLastFocused/get`, `onFocusChanged` [V, smoke on `cef-ext`] |
| `tabs.create`, `windows.create`, `runtime.openOptionsPage`, `clients.openWindow` | **No**: a native Chrome-style window with no client, which blocks shutdown [V source] | `chrome_browser_delegate.cc:628-650` (`cef_params == nullptr`); only hook `CefBrowserProcessHandler::GetDefaultClient` (welding does not implement it). **P2**. **Patched**: `tabs.create`, `openOptionsPage` and `clients.openWindow` open a wubble tab; `windows.create` is refused [V, smoke: `tabs.create`] |
| Action button, badge, `action.onClicked`, the popup, `activeTab` | **No UI** [V source] | state exists, nothing can click it or read it. **P3**; the popup page itself can be a windowless view at `chrome-extension://<id>/<default_popup>` (but `tabs.query` inside it hits P1). **Patched** for the engine (click with `activeTab`, `onClicked` or the popup URL, badge/title/icon state, change notices) [V, smoke]; no button UI yet |
| Options page | Yes when wubble opens `chrome-extension://<id>/<options_page>` in a tab [I] | the extension's own `openOptionsPage` hits P2 |
| `webRequest` (observe), `declarativeNetRequest` | Expected yes [I] | `ChromeContentBrowserClientCef::WillCreateURLLoaderFactory` calls Chrome's (where the web request proxy goes in) before CEF's interceptor (`chrome_content_browser_client_cef.cc:504-563`) |
| `nativeMessaging` | Yes, with host manifests in `<cache>/NativeMessagingHosts` or `/etc/chromium/native-messaging-hosts` (Linux), `/Library/Application Support/Chromium/NativeMessagingHosts` (macOS) [V source] | `chrome_paths.cc:454-485`; CEF overrides `DIR_USER_DATA` (`resource_util.cc:130`). Password managers' desktop apps write Chrome's/Chromium's dirs, so wubble's user dir needs them copied or linked (later). |
| `permissions.request`, install/uninstall prompts | Likely no [I] | prompts need a `Browser` window. **P4**, or answer in wubble's UI |
| `contextMenus` | Unknown [I] | check later |
| Web Store install, updates | **Off by design** | decision "Extension updates"; section 3 |

## 2. How wubble does it (this lane)

- **No CEF extension API.** CEF removed `CefExtension`/`CefRequestContext::LoadExtension` with the Alloy bootstrap (CEF
  17a1fd62e "Remove extension tests (see #3685)", a461a8972 "Delete Alloy bootstrap (fixes #3685)") [V]. Chrome's own
  extension system is the only one, driven by switches at `cef::initialize`.
- **On only when installed.** `wubble_engine::app::extension_switches`: an empty list keeps `disable-extensions` (the
  extension system's ~60 MiB PSS never start); a non-empty one gives `load-extension`,
  `disable-component-extensions-with-background-pages` and `disable-default-apps` (section 3).
- **The list** is `wubble_core::extensions::Installed` (id, name, version, enabled, source, granted permissions) in the
  setting `extensions.installed`; install/enable/disable/remove is one pure `transition` returning `Save` and
  `Refused` effects, run by `crates/wubble/src/extensions.rs`. Chromium does not keep command-line extensions
  between runs (`installed_loader.cc:365-367`) [V], so the list is the one owner. Every change applies at the next
  start (`restart_needed` compares the enabled entries with the list at start, so a new copy in the same directory
  counts). Nothing is deleted while running, since the running Chromium keeps reading a removed or replaced copy
  (popup, options, a service worker restarting): at start, before CEF, `sweep` deletes the copies the list no
  longer names (`unused_copies`), refused and half-unpacked ones included.
- **Ids** follow Chromium exactly: with a manifest `key`, SHA-256 of the decoded key; without, SHA-256 of the
  directory's absolute, symlink-free path (`id_util.cc`, `extension.cc` `ComputeExtensionID`; ported with the upstream
  test vectors). The smoke checks that Chromium gives both test copies the ids wubble-core computes [V].
- **.crx**: `parse_crx` reads the CRX3 header (port of electron-chrome-web-store `parseCrx`, CRX2 dropped), the zip is
  unpacked with CEF's own `CefZipReader` (works before `cef::initialize` once the framework is loaded, which on macOS
  welding's `CefRuntime::prepare_api` does first; unit-tested on Linux and Windows) into a new
  `<profile>/extensions/<id>/<version>` (`<version>-2`, ... when that copy exists), refusing entries that would escape (zip slip), and `"key"` is written into the
  manifest so the copy keeps the .crx's id. As in electron-chrome-web-store, the signature is not verified: the declared
  id must match a key in the file. RSA verification can go into `parse_crx` later.
- **Per container.** Every Chrome profile reads the global command line in `ExtensionService::Init` [V source], and each
  container is its own profile (`storage.rs`). The smoke sees each service worker **twice** with one view open [V], most
  likely once in CEF's global profile and once in the view's container [I]: memory per extension scales with the
  profiles in use, and an extension's storage is per container (a password manager unlocks per container) [I]. A
  per-container choice needs a patch.
- **Process counting on Linux**: zygote-forked renderers keep the zygote's command line in `/proc/<pid>/cmdline`, so
  `--extension-process` (`chrome_content_browser_client_extensions_part.cc:908`) is not visible there [V];
  `wubble_sysmem::label` names such renderers "Extension" where the OS shows it (macOS, Windows) [I]. The smoke counts
  extension service workers from the DevTools endpoint's `/json/list` instead.

## 3. Nothing phones home: what was measured

A net log (`WUBBLE_PROBE_SWITCHES="log-net-log=FILE"`) of `--smoke extensions` and of `--smoke extensions-none`,
grepped for every non-loopback URL [V]:

- With extensions on and only `load-extension` + the component switch, Chromium **installed an extension from the Web
  Store**: `clients2.google.com/service/update2/crx?...id=gphhapmejobijbbhgpjhcjognlahblep...installedby=external` and the
  .crx download from `clients2.googleusercontent.com`. Source: Linux's external extension providers
  (`external_provider_impl.cc`, `DIR_STANDALONE_EXTERNAL_EXTENSIONS` = `/usr/share/chromium/extensions/*.json`), which
  Fedora fills with the GNOME browser connector. `disable-default-apps` makes `CreateExternalProviders` return before
  every external source (standalone JSON, `DIR_EXTERNAL_EXTENSIONS`, the per-user dir, preinstalled apps, the Drive
  migration, external components); with it the extensions-on log has **no host the extensions-off log lacks** [V].
- Command-line extensions are never auto-updated (`Manifest::IsAutoUpdateableLocation`, `manifest.h:72-77`;
  `extension_updater.cc:422`) [V], so no update check is made for anything wubble loads.
- **Pre-existing, with or without extensions** (not this lane's to fix, recorded in the roadmap, todo/): the component updater
  (`update.googleapis.com/service/update2/json`), network time (`clients2.google.com/time/1/current`), a component diff
  from `edgedl.me.gvt1.com`, the spellcheck dictionary (`redirector.gvt1.com/edgedl/chrome/dict/en-us-10-2.bdic`) and
  `www.google.com/async/folae` (an omnibox/AI Mode prefetch [I]) are requested on a throwaway profile in both modes.
  Switched off in the privacy pass (`CHROMIUM_SWITCHES`, `QUIET_FEATURES` in `wubble-engine/src/app.rs`): a 7 minute capture on a private
  network namespace then shows no DNS query or connection; the translate ranker model (`www.gstatic.com`) and the on-device model's
  manifest component (an `update.googleapis.com` request registered by `chrome_browser_main_extra_parts_optimization_guide.cc`
  whatever `disable-component-update` says) were found there too.

## 4. Gaps that need a Chromium or CEF patch

| Patch | What | Where |
|---|---|---|
| **P1** | `tabs.query`, tab events and `windows.*` see wubble's views: include Alloy browsers in `TabsQueryFunction::BuildTabList`, `TabsEventRouter` and `WindowControllerList`, plus a CEF API through which wubble gives each view its window id, active flag and index | `chrome/browser/extensions/api/tabs/tabs_api.cc`, `tabs_event_router.cc`, `window_controller_list.cc`; CEF `chrome_extension_util.cc` |
| **P2** | Tabs and windows an extension opens: a CEF callback when Chrome code creates a `Browser` without CEF params, so wubble opens a tab of its own instead (partial unpatched mitigation: `GetDefaultClient` in welding, close and reopen in a wubble tab) | `libcef/browser/chrome/chrome_browser_delegate.cc`, `include/cef_browser_process_handler.h` |
| **P3** | The action button and popup: a CEF API over `ExtensionActionAPI::DispatchExtensionActionClicked`, the action observers (icon, badge, title), the popup host and `ActiveTabPermissionGranter` | `chrome/browser/extensions/extension_action_runner.cc`, `api/extension_action/` |
| **P4** | Permission prompts (`permissions.request`, install prompts): a CEF callback to answer them in wubble's UI | `chrome/browser/extensions/extension_install_prompt.cc`, `api/permissions/` |

Password managers (Bitwarden, 1Password) need at least P1 (the active tab) and P3 (the popup); their first-run welcome
tab hits P2, which today opens an unmanaged native window that blocks shutdown [I], so until P2 the list is for
test and developer extensions, and the UI should say so.

## 5. How other embedders do it

- **Electron** (MIT): `session.loadExtension` loads unpacked only, with a partial `chrome.tabs`; no action UI of its own
  [I]. samuelmaddock/electron-browser-shell's `electron-chrome-extensions` (MIT) fills `tabs`/`windows`/`action` in JS
  on the host's own tab model: the model for P1/P3's wubble side. Its `electron-chrome-web-store` unpacks .crx files as
  wubble does now (ported).
- **CefSharp / CEF Alloy** used `CefRequestContext::LoadExtension` with a minimal API set; gone since CEF #3685 [V].
- **Chrome-runtime CEF apps** get Chrome's extension system as is, but CEF's own windowless (Alloy-style) browsers only
  the minimal shim of section 1; windowed Chrome-style browsers would get Chrome's UI, which wubble does not use.

## 6. Risks

- Path-made ids change when an unpacked directory moves (Chromium's rule); its data under the old id is orphaned.
  Removing an extension leaves its per-id data in each container (`Local Extension Settings/<id>`, IndexedDB, the SW
  registration): clearing it is a follow-up.
- No install prompt: whatever the manifest asks for is granted at load; the install UI must show the permissions first.
- Windows ids for path-loaded extensions use the UTF-16, upper-case drive letter rule (ported, untested on Windows).
- `disable-default-apps` and `disable-component-extensions-with-background-pages` apply only while an extension is
  installed; `--smoke builtins` (the PDF viewer) and `--smoke page` (DevTools) pass with the list empty.

## 7. The patch queue (P1 to P3)

wubble's patches live in `patches/chromium/` ([patches/README.md](../../patches/README.md)), applied by
`scripts/build-cef.sh WORK patch` after CEF's hook, and built beside the CEF in use (`CEF_INSTALL`; the lane's
build is `~/Dev/wubble-wt/cef-ext`). No translated CEF API: cef-dll-sys ships pregenerated bindings for API
15600, so the patches export one plain C function, `cef_wubble_ext_v1` (libcef.lst exports every `cef_*`), which
`crates/wubble-engine/src/ext_api.rs` looks up at run time. A stock CEF lacks it and wubble behaves as before.

| API | On the patched CEF | How |
|---|---|---|
| `windows.getCurrent`, `getAll`, `getLastFocused`, `get` | wubble's Main windows (`normal`) and Little/Popup windows (`popup`); Pip left out; bounds 0 | one `WindowController` per (wubble window, loaded profile), ported from Chromium's `AppWindowController`/`AppBaseWindow`; `getAll`/`getLastFocused` fallbacks in `tabs_api.cc` |
| `tabs.query` | wubble's live tabs, index/active/pinned from wubble's model | `BuildTabList` also walks wubble's controllers (the same window and tab rules); `CreateTabObject` asks the registry |
| `tabs.get`, `sendMessage`, `update {url, muted}`, `scripting` by tab | yes (CEF's own shim) with the window id and index | `SessionTabHelper::SetWindowID` per tab |
| `tabs.update {active}`, `tabs.remove` | wubble activates / closes the tab; `remove` answers when the page is gone | delegate `activate_tab` / `close_tab` |
| `tabs.create`, `runtime.openOptionsPage`, `clients.openWindow` | wubble opens a tab last in the Space of the window asked for (else the active Space), in the asking extension's container, shown when active, and answers with it (Chrome's new-tab page: wubble's start page, answered with no tab object; the global profile's requests are refused) | delegate `open_tab`, answered by `tab_opened` (0 refused, -1 a wubble page) |
| `windows.create` | refused: "windows.create is not supported in wubble" | would make an unmanaged `Browser` window |
| `onCreated`, `onUpdated`, `onActivated`, `onRemoved`, `windows.onFocusChanged` | yes | `TabsEventRouter`'s own entries and dispatch, reached as its friend |
| `onMoved`, `onAttached`, `onDetached`, `onReplaced`, `windows.onCreated/onRemoved` | no | Chromium ignores controllers without a `Browser` for window events (`windows_event_router.cc:203-206, 227-230`); moves are not diffed yet |
| `action` click, badge, title, icon, popup URL, changes | engine API (`Engine::action_click`, `action_state`, `ExtRequest::ActionChanged`); no buttons yet | `ExtensionActionRunner::RunAction(ext, true)`, `ExtensionAction` getters, `ExtensionActionDispatcher` observer |

The model mapping (decision "Extension tabs model"): `wubble_core::extensions::project` lists each Main window's
Space tabs (pinned, then items, folders flattened), puts tabs of Spaces no window shows at the end of the focused
Main window (never active), leaves sleeping tabs out (Chrome would keep the tab and send `onReplaced` on wake: a
follow-up), and lists every window in every container's profile, each profile's controller holding only its own
tabs (indices are the window's, so a profile may see gaps).

## 8. Private windows

Date: 2026-11-23, read in the CEF checkout (`libcef/browser/chrome/chrome_browser_context.cc`) and the patch queue; not run.
A private window's container is opened with no directory (`SharedProfile::new(runtime, None)`): CEF then makes a
**unique** off-the-record profile (`Profile::OTRProfileID::CreateUniqueForCEF`) whose parent is CEF's primary user
profile (the global one, `GetPrimaryUserProfile`), and destroys it when the context shuts down [V]. It is not Chrome's
primary incognito profile.

Each extension has "Allow in Private Windows" (`wubble_core::extensions::Extension::private`, stored in
`extensions.installed`, off by default, also offered as a box in the install prompt). `wubble_core::extensions::reaches`
is the one rule: enabled, and in a private window only when allowed.

| What | Enforced by | State |
|---|---|---|
| Action button, its popover list, the popup, a click on the page (`activeTab`) | the host (`host/ext_ui.rs`: `reaches` on the window) | per extension, live |
| Footer button in a private window | the host (`footer_button(.., private)`) | shown only with an allowed extension installed |
| Tabs an extension opens (`tabs.create`, options page) | the host (`ext_tabs.rs` `open_command`, `Model::extension_tab`) | never in a private window, whoever asks |
| `tabs.remove`, `tabs.update {active}` on a private tab | the host (`ext_tabs.rs` `serve`) | refused for every extension: `ExtRequest` does not name the asking extension |
| `tabs.query`, `windows.*`, tab events (the projection) | the host leaves private windows out of the one projection (`extensions::project`); the patch's registry makes controllers per *regular* profile only, and lists a profile's own tabs only (`TabsIn(window, profile)`) [V] | no extension sees private tabs or windows, allowed or not |
| Content scripts, `webRequest`/`declarativeNetRequest`, events in the off-the-record profile | Chromium: an extension acts in an off-the-record profile only when `extensions.settings.<id>.incognito` is set in its original profile's prefs (`util::IsIncognitoEnabled`, off for unpacked and command-line extensions) [I] | off for every extension: nothing sets that pref |

So today the host alone makes "not allowed" hold everywhere it can see, and Chromium's default makes it hold in the
engine [I: to confirm with a content script on a private tab]. "Allowed" reaches only the host-side surfaces (the action
button and its click). Making it reach the engine needs **P5**, a patch: (a) a `cef_wubble_ext_v1` entry that calls
`extensions::util::SetIsIncognitoEnabled(id, primary_profile, on)` for each extension at start and on change (setting
the whole `extensions.settings` dictionary through `CefRequestContext::SetPreference` from the host would race
Chromium's own writes, so not that); (b) the registry (`10-cef-tabs-windows.patch`) making controllers for
off-the-record profiles too, so that `BuildTabList` and the events, which already filter on
`include_incognito_information()`, show private tabs to allowed extensions only; then (c) the projection carrying
private windows and `ExtRequest` naming its extension so `ext_tabs.rs` can apply `reaches`. Whether Chromium runs an
incognito-enabled extension in a *non-primary* off-the-record profile at all is [I], to check first.

## Acceptance

Hand check, 2026-11-20, Linux, the patched CEF (`CEF_PATH=~/Dev/wubble-wt/cef-ext`), `scripts/headless.sh` under
`timeout`, a throwaway profile per extension. Extensions downloaded by hand from their GitHub releases into a scratch
directory outside the repo (never committed, never fetched by wubble): Bitwarden `oss-dist-chrome-2026.11.0.zip`
(bitwarden/clients, tag `browser-v2026.11.0`) and uBlock Origin Lite `uBOLite_2026.1116.1650.chromium.zip`
(uBlockOrigin/uBOL-home), each unzipped and installed as a folder.

How: a throwaway driver (not committed: no new feature code) replayed a script of steps on the host's own code paths,
each step a call the Settings and footer controls make: `extension_picked(folder)` (Settings "Install from folder",
minus the native file dialog), the prompt's "Add extension" (`extension_prompt_answered`), a restart (a second run on
the same profile: extensions load at start), the footer button (`extensions_requested`), a press on the extension's row
(`extension_pressed`), synthetic Slint pointer events and the host's key path (`smoke_keys`, as `--smoke start` types),
and a window PNG per step. The page's and the extension's own state came from the DevTools endpoint
(`remote_debugging_port`) read by a small Node script: `/json/list`, `Runtime.evaluate` in the service worker, the popup
and the page, and their console (`Runtime.enable` replays the console, and announces each isolated world). Pages: a
loopback login form (`python3 -m http.server`, `127.0.0.1`). No Bitwarden account exists here: the check stops at the
sign-in screen (the email field takes typing; no login was attempted). Screenshots and the driver's patch:
`~/Dev/wubble-wt/notes/xm/acc/` (`p1-prompt.png`, `p2-panel.png`, `p8-typed.png`, `u2-popup.png`, `u3-dashboard.png`,
`driver-and-fix.patch`).

| Check | Bitwarden 2026.11.0 | uBlock Origin Lite 2026.1116.1650 |
|---|---|---|
| Installs (Settings path, prompt, restart) | yes; the prompt lists its permissions (all sites, clipboard, history, block content, notifications); the name shows as `__MSG_extName__` (gap 1) | yes; same, same name gap |
| Service worker starts | yes: `background.js` is a target; console: WASM SDK loaded, state migrations run, "Autofill monitoring disabled; the active account is logged out" (expected without an account); `tabs.create` works: it opened its getting-started page as a tab | yes: `js/background.js` is a target; storage round trip ok; 6 static rulesets enabled |
| Footer button shows its action | yes: the popover lists "Bitwarden" with the shield icon (after the fix below) | yes: "uBlock Origin Lite" with its icon |
| Popup opens, renders, takes input | opens at once but 57 px wide (gap 2); with the page's width set by hand it renders the intro carousel, then the Log in screen, and the email field takes typed text (`email=hello`) | yes: 273 x 438, the filtering-mode slider takes a click (optimal to basic), "Open the dashboard" opens `dashboard.html` as a tab; no text field to type in |
| Content script runs on the loopback login form | yes: an isolated world "Bitwarden Password Manager" is attached to the page. No inline menu or autofill icon appears: Bitwarden turns autofill off while no account is logged in (its own warning above). Not verifiable here | yes: an isolated world "uBlock Origin Lite" is attached; its four registered content scripts (`scriptlets-isolated/main`, `prevent-popup`, `css-specific`) exist; a dynamic `declarativeNetRequest` block rule stopped a `fetch` on the page (`ERR_BLOCKED_BY_CLIENT`) |

API probe from Bitwarden's service worker (`Runtime.evaluate`): `storage.local`, `alarms`, `idle.queryState`,
`offscreen.hasDocument`, `contextMenus.create`, `notifications.getAll`, `sidePanel.getOptions`, `tabs.query` (both
tabs, with urls), `windows.getAll` (one `normal` focused window), `scripting.executeScript` on the loopback tab all
answer. `chrome.privacy` is absent until its optional permission is granted (as in Chrome).

### Gaps

1. **Extension names with `__MSG_..._` placeholders show raw** in the install prompt, the Settings list and the
   popover title (`manifest.name` is `__MSG_extName__` for both; `wubble_core::extensions::Extension::name` says "shown
   raw for now"). Resolve it from `_locales/<default_locale>/messages.json` (then the UI language) when the manifest is read.
2. **The action popup is laid out far too narrow for Bitwarden (57 px).** Root cause, read in the extension's code:
   `chrome.tabs.getCurrent()` called from the popup returns a tab object (id, `windowId: -1`, the popup's own URL)
   where Chrome returns `undefined`, because the popup is a windowless view that carries a `SessionTabHelper` id
   (`TabsGetCurrentFunction::Run`, `tabs_api.cc:1937`, only tests `GetTabId(contents) >= 0`). Bitwarden's
   `BrowserApi.isInTab()` then finds a "tab" view (`extension.getViews({tabId, type: 'tab'})` is 1 for that id),
   decides it is not in a popup and skips `document.body.style.width = 380px` (`inPopup && !isInTab`); it adds the
   `body-full` class instead. The page's max-content width is then its text's, 57 px. Any extension that tests
   `tabs.getCurrent()` to tell a popup from a tab behaves so. Fix: a patch so `tabs.getCurrent` answers nothing (and
   `getViews` does not count the view as a `tab`) for a view that is not one of wubble's tabs (the popup and the
   DevTools panel); needs a CEF rebuild. Seen: `chrome.tabs.getCurrent()` in the popup gives
   `{id: 1814074672, windowId: -1, active: false}`; `chrome.extension.getViews({type:'tab'}).length` is 1.
3. **`chrome.permissions.request` has no window** (P4): from Bitwarden's popup with a user gesture,
   `chrome.permissions.request({permissions: ['privacy']})` rejects with "Could not find an active window." Bitwarden's
   first-run "Make Bitwarden your browser's default password manager" step calls it (`Continue`); a user can press
   Skip. Optional host permissions of other extensions fail the same way. Not tried: the answer in wubble's UI.
4. **Inline menu not verified**: it needs a Bitwarden account (stopped at sign-in, as agreed). A test account's
   vault on a loopback form is the next hand check.
5. **The popup's first page is `#/default-password-manager-prompt`**, not the sign-in screen, because of gap 3 and
   `chrome.privacy` being undefined; a user presses Skip and Log in. Same cause, listed so nobody reads it as a bug.

Fixed in this pass (found by the check): the footer popover showed the generic puzzle piece for every extension whose
manifest names its icon with the default-icon map. The engine reports the chosen file as an absolute path inside the
extension's directory (`action_state.icon_path`); `action_icon` joined it to the directory again and the read failed.
`icon_file` in `crates/wubble/src/host/ext_ui.rs` now strips the directory, with a unit test.

Not covered: the Web Store, updates, `nativeMessaging` (Bitwarden's desktop-app unlock), `contextMenus` clicks,
the side panel (Bitwarden's `sidepanel-disabled.html` is its default), `userScripts` (uBOL's optional use), the
cosmetic filter on a page that has an advert, and macOS and Windows (the patch queue is Linux only).
