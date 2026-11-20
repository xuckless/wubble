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
| `tabs.query`, `tabs.on*` events, `windows.*` | **No** [V source] | `TabsQueryFunction::BuildTabList` walks only `Browser` windows' tab lists (`tabs_api.cc` ~1956); `TabsEventRouter` observes `TabListInterface` only. **P1** |
| `tabs.create`, `windows.create`, `runtime.openOptionsPage`, `clients.openWindow` | **No**: a native Chrome-style window with no client, which blocks shutdown [V source] | `chrome_browser_delegate.cc:628-650` (`cef_params == nullptr`); only hook `CefBrowserProcessHandler::GetDefaultClient` (welding does not implement it). **P2** |
| Action button, badge, `action.onClicked`, the popup, `activeTab` | **No UI** [V source] | state exists, nothing can click it or read it. **P3**; the popup page itself can be a windowless view at `chrome-extension://<id>/<default_popup>` (but `tabs.query` inside it hits P1) |
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
  setting `extensions.installed`; install/enable/disable/remove is one pure `transition` returning `Save`, `DeleteDir`
  and `Refused` effects, run by `crates/wubble/src/extensions.rs`. Chromium does not keep command-line extensions
  between runs (`installed_loader.cc:365-367`) [V], so the list is the one owner. Every change applies at the next
  start (`restart_needed` is derived from the start-time paths).
- **Ids** follow Chromium exactly: with a manifest `key`, SHA-256 of the decoded key; without, SHA-256 of the
  directory's absolute, symlink-free path (`id_util.cc`, `extension.cc` `ComputeExtensionID`; ported with the upstream
  test vectors). The smoke checks that Chromium gives both test copies the ids wubble-core computes [V].
- **.crx**: `parse_crx` reads the CRX3 header (port of electron-chrome-web-store `parseCrx`, CRX2 dropped), the zip is
  unpacked with CEF's own `CefZipReader` (works before `cef::initialize`, unit-tested) into
  `<profile>/extensions/<id>/<version>`, refusing entries that would escape (zip slip), and `"key"` is written into the
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
- **Pre-existing, with or without extensions** (not this lane's to fix, recorded in TODO.md): the component updater
  (`update.googleapis.com/service/update2/json`), network time (`clients2.google.com/time/1/current`), a component diff
  from `edgedl.me.gvt1.com`, the spellcheck dictionary (`redirector.gvt1.com/edgedl/chrome/dict/en-us-10-2.bdic`) and
  `www.google.com/async/folae` (an omnibox/AI Mode prefetch [I]) are requested on a throwaway profile in both modes.

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
