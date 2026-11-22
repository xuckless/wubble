# Build-out batch 9

15 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `ocr-pipeline-text-layer`: Live text in images: OCR job machine, image fetch, cache and selectable text layer

Area: todo-a. Needs an engine patch: no.

TODO line 57, docs/research/ocr.md sections 4-5 (Chrome's "Select text in image"/Lens-less, Safari Live Text). (1) A pure `wubble_core::ocr`: `OcrState` (Idle, Fetching, Recognising, Ready, Failed) with `transition` and effects, plus an LRU keyed by blake3(bytes), engine version and languages, unit-tested. The cache is memory only for private profiles. (2) Pixels: the original image bytes through the engine's own fetch on the tab's request context (`crates/wubble-engine/src/fetch.rs`), with DevTools `Page.captureScreenshot` with a clip for canvas, video frames and CSS backgrounds (`View::cdp`). No frame-texture readback. (3) Presentation: a page script makes an injected closed-shadow-root layer of transparent `<span>`s positioned over the image. It follows the image's box (ResizeObserver, getBoundingClientRect), and takes pointer events only over text, so native selection, copy and find work. (4) Trigger: a context-menu row "Select text in image" on img, canvas or video (`crates/wubble/src/menu.rs`), later a hover trigger behind an off-by-default setting. Recognition goes through an `OcrBackend` trait that `ocr-backends` implements. Nothing touches the network.

## `blocker-startup-gate`: Content blocker start-up gate holds the first navigation

Area: todo-b. Needs an engine patch: no.

With no compiled-engine cache (first run, or after a list change), requests go through unfiltered for several seconds until the lists compile. Chrome extensions such as uBO hold requests at start-up. wubble cannot wait on CEF's IO thread, so the hold belongs at the host. The swap point is `Blocking::swap` around crates/wubble/src/blocking.rs:477-481 (`measure::Stage::BlockerReady`).

To build: a pure lifecycle in wubble-core (for example `blocker_gate.rs`: `enum Gate { Waiting { held: Vec<Held> }, Ready }`, `fn transition(Gate, GateEvent) -> (Gate, Vec<GateEffect>)`). The events are Navigate { view, url } (a main-frame load the host is about to start: restored active tab, typed address, start URL, `wubble <url>` from the instance socket), BlockerReady, BlockerOff, ListsFailed and Deadline. The effects are Load(view, url) and ShowWaiting(view). It is Ready from the start when the cache loaded or the blocker is off (store `BlockerEnabled`). While Waiting, the host creates the view but delays `load_url`, and the pane shows the existing loading state. The gate is bounded by events: blocker ready, compile failure, or a deadline (for example 10 s) that loads anyway and logs it. Page-initiated navigations inside already-loaded pages are not held. Wire it into crates/wubble/src/host.rs where views get their first URL, and send BlockerReady from blocking.rs through the existing UI-thread channel. Unit tests for the transition. All platforms.

## `internal-pages-chrome-urls`: Internal URLs: chrome:// diagnostics, about: aliases, wubble://about index, wubble://version

Area: navigation. Needs an engine patch: no.

In Chrome you can type chrome://version, gpu, net-internals, net-export, credits, media-internals, webrtc-internals, process-internals, accessibility, histograms, serviceworker-internals, indexeddb-internals, inspect, sandbox and so on. The about:X form maps to chrome://X, and chrome://about lists them all. The decision 'Internal surfaces' already allows Chromium diagnostics as web pages. But omnibox classify (crates/wubble-core/src/omnibox.rs, HIER_SCHEMES) does not take 'chrome', so 'chrome://gpu' becomes a search, and about:version is passed raw.

1. Add 'chrome' to the accepted schemes. Add a pure fn in wubble-core/pages.rs, `resolve_internal(url) -> Resolved { Native(InternalPage) | Chromium(String) | Refused }`, from one data table. Aliases go to wubble's native pages: chrome://settings, history, bookmarks, downloads, newtab, password-manager and about:settings and so on map to the wubble:// InternalPage. The WebUI hosts CEF 156 serves for alloy-style browsers pass through. Pages CEF lacks or that wubble replaces are refused (flags is covered by a separate feature).
2. Add InternalPage::About (wubble://about), a native list of every wubble:// page and the allowed chrome:// hosts, built from PAGE_LINKS and the table with kit list rows (kit/pages).
3. Map wubble://version and wubble://credits to chrome://version and chrome://credits, since CEF fills them with CEF and Chromium data. Settings' About section links them.
4. Agent-session tabs keep refusing chrome: (agent may_go and the 'Agent tabs stay on the web' decision); add a check that resolve_internal is never consulted for them.

The command bar suggests matching internal pages as Action-like rows. Unit tests for the resolver table.

## `x11-touchpad-gestures`: X11: touchpad pinch, history swipe and lock keys

Area: page-interaction. Needs an engine patch: no.

Chrome on X11 (with XInput 2.4 gesture events, X server 21.1 and later) pinch-zooms the page on a touchpad pinch and goes back or forward on a two-finger sideways overscroll. wubble does both on Wayland through the worker (`zwp_pointer_gestures_v1`, the axis-source finger swipe in crates/wubble/src/wl/wayland.rs feeding `AuxEvent::Pinch`/`AuxEvent::Swipe` into host/gesture.rs and host/pinch.rs). Under X11 nothing reports them (`Aux` is `wl::Null`), so a pinch arrives as Ctrl+wheel steps (`zoom.js`, a page-zoom step, not Chrome's visual-viewport pinch), and there is no history swipe. Lock keys (`AuxEvent::Locks`) are also unreported there. To build: in the X11 worker introduced by x11-xdnd (crates/wubble/src/wl/x11.rs, x11rb), select XI_GesturePinchBegin/Update/End and XI_GestureSwipe* on each window and map them to `wubble_core::gesture::PinchInput`. Build the history swipe from XI2 smooth-scroll valuators of a touchpad device with no momentum, matching what wayland.rs does with axis frames, and deliver `AuxEvent::Swipe`. Read lock key state from XKB state notify. If the worker exists without x11-xdnd, it must be the same one owner. Pure mapping functions get unit tests in wubble-core gesture.rs.

## `clear-data-dialog`: Clear browsing data dialog: time range by kind, per container

Area: privacy-security. Needs an engine patch: no.

Chrome's Clear browsing data has a time range (Last hour, Last 24 hours, Last 7 days, Last 4 weeks, All time) and checkboxes: Browsing history, Download history, Cookies and other site data, Cached images and files, Passwords, Autofill form data, Site settings, and Hosted app data. In wubble, `Action::ClearBrowsingData` opens a menu (crates/wubble/src/menu.rs `clear_entries`) that deletes cookies, cache and auth of containers at all time, and forgets one site. History has its own range (`pages::ClearRange`, three ranges).

Build:
- In crates/wubble-core/src/pages.rs, extend `ClearRange` with LastWeek and Last4Weeks. Add a pure `ClearPlan { range, kinds: BTreeSet<ClearKind>, containers }` and `fn effects(plan, now) -> Vec<ClearEffect>`, with unit tests.
- Effects: history and recently-closed through the store's existing range delete; downloads list rows in range (wubble-store downloads.rs); passwords created in range (wubble-store logins.rs); site_permissions and content settings with `decided_at` in range; cert exceptions; cookies created in range through a cookie visitor that deletes when `creation >= from` (`ImplCookieVisitor` with `delete_cookie=1`, in crates/wubble-engine/src/storage.rs); site storage per origin visited in range (Storage.clearDataForOrigin through a page of the container, the site-data-viewer helper); and the HTTP cache. CEF has no ranged cache clear, so the row says "all time" for cache when the range is not All. Use `storage::clear`'s ClearHttpCache.
- UI: a sheet (overlay `Kind::Dialog`) with a range select, kit check boxes, a container choice (This container / All), and "Delete data". It replaces the menu, and the History page's clear opens it preset.

## `credential-mgmt-passwords`: Credential Management API for passwords (PasswordCredential) from wubble's vault

Area: web-platform-ui. Needs an engine patch: no.

In Chrome, navigator.credentials.get({password: true, mediation}) returns a saved PasswordCredential from the password manager (with an account chooser, and auto sign-in for 'silent'/'optional'). credentials.store(new PasswordCredential(form)) offers to save, and preventSilentAccess() turns auto sign-in off. wubble has its own vault and autofill (wubble_core::autofill, crates/wubble/src/host/autofill.rs, wubble-helper/src/autofill.js), but these calls resolve null, because Chromium's password manager is not wired to CEF.
Build: (1) Add a password-only path to the content script, or a sibling channel `credentials`. It wraps credentials.get/store/preventSilentAccess when `password` is asked, and leaves publicKey and federated requests alone. (2) On the browser side, answer get from the vault entries for the frame's real origin, using the same matching rules autofill uses. With more than one entry, or mediation 'required', show the existing key menu or a kit account chooser. Silent and optional mediation auto sign in only if the user has not called preventSilentAccess for that origin (a per-origin flag stored with the vault). store() goes to the existing save prompt. (3) Agent sessions follow decisions.md "Agent secrets bind on the sheet": no credentials are handed to an agent tab except bound secrets. (4) Unit-test the pure decision (entries, mediation, silent flag → answer, chooser or null) in wubble-core autofill.

## `download-retry-origin`: Retry and Save As from the originating tab with its referrer

Area: media-downloads. Needs an engine patch: no.

TODO.md: "Downloads: Retry restarts from any tab of the download's container, not the tab it came from (a login-gated file can come back as a sign-in page)". Chrome retries with the original request's referrer and site context. wubble's host/downloads.rs `apply_download` StartAgain calls `view_for_download(container)`, which takes the shown tab or any live tab of the container. It then runs `View::start_download(&url)`, which sends no referrer.

What to build: (1) DownloadEntry (wubble-core/src/download.rs) gains `tab: Option<TabId>` (the tab that started it) and `referrer: String` (the page's committed URL when it started), stored through a migration in crates/wubble-store/sql and wubble-store/src/downloads.rs. (2) Core: a pure `fn retry_view(entry, live: &[(TabId, ContainerId, agent: bool)], shown) -> Option<TabId>` that prefers the origin tab, then a tab of the same container on the referrer's site, then any tab of the container, never an agent-driven tab. Unit tests. (3) Engine: `View::start_download_with_referrer(url, referrer)`. CEF's StartDownload has no referrer, so use a CefURLRequest-free road: DevTools `Page.navigate` of a hidden frame is wrong, so use `Network.setExtraHTTPHeaders` around the start, or `Browser.setDownloadBehavior` plus `Runtime.evaluate` of an `<a download>` click in the origin page, which carries the page's referrer policy. Choose the one that keeps cookies in the container. (4) A retried download keeps its list position, as Chrome does, instead of appearing as a new top row. All platforms.

## `passkeys-macos-platform`: macOS platform passkeys (iCloud Keychain) via AuthenticationServices

Area: passwords-autofill-sync. Needs an engine patch: no.

Chrome on macOS creates and uses passkeys stored in iCloud Keychain (and Chrome's own profile passkeys). Plug a platform authenticator into the `webauthn-security-keys` trait using public API only: `ASAuthorizationPlatformPublicKeyCredentialProvider` with `ASPublicKeyCredentialClientData` (macOS 13.5+, browser variant that takes the origin, per docs/research/apple-passwords.md section 3), via objc2-authentication-services, driven from the main thread with `ASAuthorizationController` and a presentation anchor of wubble's NSWindow (crates/wubble/src/mac_app.rs-adjacent code in crates/wubble-engine/src/mac_app.rs). Registration and assertion map the shim's options to `createCredentialRegistrationRequest`/`createCredentialAssertionRequest` with clientData built from the frame's real origin; conditional mediation uses `performAutoFillAssistedRequests` where available, surfaced as rows in the autofill dropdown ("Passkey for <site>"). Needs the `com.apple.developer.web-browser.public-key-credential` entitlement and a Developer ID provisioning profile (owner files the request; TODO "Apple Passwords / passkeys entitlement request"): add the entitlement plist entry and the bundling step (docs/macos.md app bundle) behind a build feature `mac-passkeys`, and fail closed (falls back to security keys only) when the entitlement is missing at runtime. `isUserVerifyingPlatformAuthenticatorAvailable` answers true only then. Setting row under the Autofill and passwords section. macOS only.

## `ext-container-access`: Extensions: allow per container and in agent sessions (Chrome's 'Allow in Incognito')

Area: devtools-extensions-internals. Needs an engine patch: yes.

Chrome loads extensions in incognito only when the user ticks "Allow in Incognito" on the details page (`util::SetIsIncognitoEnabled`). wubble has no private windows. Its isolated contexts are containers, including the ones agent sessions create, and every extension loads in every container (decision "Extensions on"). So a password manager or content script runs inside agent-session tabs too. To build, as the analogue: (1) Core: `Extension` gains `containers: ContainerAccess { All, DefaultOnly, Listed(Vec<ContainerId>) }`, plus `in_agent_sessions: bool` (default false, as Chrome's incognito default is off). Serialise both in `to_json`/`from_json`. A pure `extensions::enabled_in(ext, container, is_agent) -> bool` gets tests. (2) Engine: patch (extends ext-live-management's table) `set_enabled_in_profile(profile_path, ext_id, bool)`, implemented with `ExtensionRegistrar::DisableExtension` and a wubble disable reason on that profile only. wubble calls it when a container's request context is created (`wubble-engine/src/app.rs`, where containers get their profile path) and when the setting changes. Without the patch, an agent container's request context could be made with `--disable-extensions`-equivalent... it cannot, so without the patch the toggle is hidden (`ext_api::available()`). (3) Projection: `extensions/projection.rs::project` leaves out tabs of containers where the extension is off. (4) UI: on each Settings extension row, a kit toggle "Allow in agent sessions" and a kit select "Containers".

## `updater-launch-rollback`: Updater: roll back a build that cannot even load

Area: platform. Needs an engine patch: no.

TODO.md M7 "Release gaps: ... rollback misses a build that cannot load". The rollback in `wubble_core::update::install` (`on_start`, `LaunchRecord`, `on_healthy`) runs inside the new `wubble` binary (`updater::on_launch` in `main.rs` `browse`). If that binary cannot start at all (a missing or incompatible `libcef.so` through RUNPATH, a dyld failure in the app, a bad CPU feature), it never runs its own rollback, and the user is stuck. To build: a launcher that does not link CEF. On Linux, `~/.local/bin/wubble` and the desktop entry's `Exec` point at a tiny `wubble-launch` (a new small crate, or a mode of the release tool that links nothing heavy) placed in `<data>/wubble/`. It reads `updates/launch.json`, execs `current/wubble` with the same args, and if the child exits within a few seconds with a loader failure (exit 127, a signal before `on_launch` wrote its start mark) while a trial is open, it applies `on_switch_failed`/`RollBack` itself (switch `current`, mark the version bad) and execs the previous one. The trial rule stays in `wubble_core::update::install` as pure functions with tests; add a `LoaderFailed` event. On macOS the app bundle's main executable is the stub: either a launcher binary in `Contents/MacOS` that loads the real one, or `Contents/Helpers`. Make the bundling change in `wubble-bundle` and `package-macos.sh`. `install.sh` and `package-linux.sh` install the launcher. Docs: docs/release.md "Updating and rollback".

## `ocr-backends`: Live text in images: Vision backend on macOS, PP-OCR over ONNX Runtime on Linux and Windows

Area: todo-a. Needs an engine patch: no.

TODO line 57, docs/research/ocr.md section 5. Implement the `OcrBackend` trait (from `ocr-pipeline-text-layer`) twice, in a new crate (for example `crates/wubble-ocr`, or a `wubble-ocr` helper process for crash isolation, as the research recommends). macOS: Apple Vision (`VNRecognizeTextRequest`, accurate level, languages from `supportedRecognitionLanguages`) through objc2-vision, returning line text and quads. Linux and Windows: PP-OCRv5 mobile detection, angle classifier and recognition over ONNX Runtime (`ort` crate, MIT/Apache). Either use oar-ocr after confirming its licence, or write a small in-repo pipeline (resize, DB post-processing, CTC decode). Bundle the models (about 25-30 MB) in the release packaging scripts (`scripts/release/`), with a THIRD_PARTY_NOTICES entry per model traced to Paddle's Apache-2.0 release. Keep `auto-download` features off. Run on a low-priority 1-2 thread pool. Windows: the same ONNX backend.

## `blocker-scriptlets-at-start`: Blocker scriptlets run at document start

Area: todo-b. Needs an engine patch: no.

uBO injects scriptlets (`##+js(...)`) at document_start, before any page script runs. In wubble the frame's `cosmetic.js` sends `hello` when its context is created; the browser answers, and only then runs the scriptlets with `run_script_in` (crates/wubble-engine/src/view.rs `answer_cosmetic`, around line 1099). By then the page's own scripts have often run, so many scriptlets (set-constant, abort-on-property-read, json-prune, no-fetch-if) come too late. That is the "uBO scriptlet coverage" gap.

To build: when a view's main frame is about to navigate (`OnBeforeBrowse`/nav commit start in crates/wubble-engine/src/nav.rs or view.rs), ask the filter for the target URL's scriptlets (`filter.frame_cosmetic(top, url).script`, which comes from `Cosmetic::scriptlets` in crates/wubble-blocker/src/blocker.rs). Register them through the view's DevTools channel with `Page.addScriptToEvaluateOnNewDocument` (with `runImmediately` where supported), and remove the previous identifier with `Page.removeScriptToEvaluateOnNewDocument`. Same-site subframes are covered by the same registration. For cross-site subframes, keep the current hello path as a fallback, and skip it when the document-start copy has already run (the scriptlet wrapper sets a guard global inside its closure scope, in the page world). Respect per-site off (`SiteOverrides`) and agent tabs. Add a page fixture to the block smoke with a set-constant rule that is checked by an inline script running before load. All platforms.

## `experiments-flags`: Experiments (flags-like) section: user-chosen Chromium features and switches

Area: navigation. Needs an engine patch: no.

chrome://flags lets the user turn Chromium features on and off and asks for a relaunch. CEF does not serve a working flags page for wubble's browsers. wubble builds its command line from one table, PageFeatures::knobs in crates/wubble-engine/src/app.rs (Knob::Enable, Disable, Switch, merged into a single enable-features and disable-features).

1. Add a curated data table in wubble-core (new experiments.rs): id, Chromium feature or switch, description, platforms, default, and whether it needs a relaunch. Start with entries that prebuilt CEF 156 honours and that users ask for: ElasticOverscroll, SmoothScrolling (disable-smooth-scrolling), ParallelDownloading, OverlayScrollbar, ForceDarkMode for web contents (careful: wubble has page themes), WebGPU on Linux (enable-unsafe-webgpu), Vulkan, ignore-gpu-blocklist, enable-features=BackForwardCache variants. Check each name is a string in libcef, as the existing knobs comment does.
2. Add settings `experiments.enabled` (list of ids) and `experiments.custom` (a list-editor of raw `--switch[=value]` and `+Feature` or `-Feature` entries, validated by a pure fn that refuses switches wubble owns: remote-debugging, user-data-dir, the sandbox ones).
3. In app.rs, merge them into the one enable/disable list and the switches, with the table's knobs winning over a conflicting user entry, and log what was applied.
4. Settings section 'Experiments' at wubble://flags (alias chrome://flags), built from settings::ALL rows with toggles and a relaunch toast. Off by default, and a 'Reset all' button.

Unit tests for validation and merging.

## `win-touchpad-gestures`: Windows: precision touchpad pinch-zoom and history swipe

Area: page-interaction. Needs an engine patch: no.

Chrome on Windows uses DirectManipulation for precision touchpads. A pinch zooms the visual viewport smoothly (not page-zoom steps), and a two-finger horizontal overscroll navigates back or forward with the arrow bubble. In wubble on Windows, winit has no `PinchGesture`, and a precision touchpad pinch arrives as Ctrl+wheel. `zoom.js` and the engine pinch code treat it as a page-zoom step (`Action::ZoomIn/Out`) or a Touch-road pinch per wheel, and no history swipe is reported (host/gesture.rs). To build: in the Windows `Aux` worker (crates/wubble/src/wl/windows.rs, shared with win-ole-dnd; one owner), create a DirectManipulation manager and viewport per HWND as Chromium's `ui/base/win/direct_manipulation` / `content/browser/renderer_host/direct_manipulation_helper_win.cc` does (BSD-3, notice entry). Feed pointer hit-test contacts (`DM_POINTERHITTEST`) into it, and map viewport transform updates to `AuxEvent::Pinch(PinchInput)` (scale) and to `AuxEvent::Swipe` (horizontal pan with the page not consuming it), with the end on the ready state. Ctrl+wheel from a real mouse keeps the page-zoom step. A pure translation from DM transform deltas to gesture inputs goes in wubble-core with tests. Use the `windows` crate (MIT/Apache).

## `https-upgrade-mode`: HTTPS-First and HTTPS-Only mode

Area: privacy-security. Needs an engine patch: no.

Chrome's "Always use secure connections" setting tries https for every http navigation. If https fails, it shows an interstitial ("Connection is not secure" … "Continue to site") and remembers the choice per site for a while. Chrome also upgrades automatically with a silent fallback (HTTPS-Upgrades). wubble only upgrades typed scheme-less public hosts (decision "Typed local addresses") and has no mode.

Build:
- Add a setting `privacy.https_mode`: off|upgrade|only. The default is "upgrade", to match Chrome's automatic upgrades; it makes no new request to a new party. Put it in crates/wubble-core/src/settings.rs.
- Add a pure module `wubble_core::https`: `plan(url, mode, exempt) -> Upgrade|Keep` (exempt: IP literals, localhost, single labels, .local, non-default ports, the per-container allow list) and a lifecycle enum per navigation (Upgraded → Loaded | Failed → Fallback or Warn) with `transition`. Unit-test it.
- Engine path: main-frame Document requests go through the request filter (crates/wubble-engine/src/filter.rs `RequestAction::Redirect`). The host's ContentFilter (crates/wubble/src/blocking.rs) consults an Arc snapshot of the mode and the allow list and redirects http→https, recording the upgrade per browser_id.
- On a load or certificate error of an upgraded navigation: in upgrade mode, reload the http URL marked exempt for that navigation. In only mode, show a new `PageError::HttpsOnly { url }` in crates/wubble/src/page_error.rs with "Go back" and "Continue to site". Continue adds the host to the container's allow list (session plus 15 days, as Chrome does) and loads http.
- Set Chromium's own prefs `https_only_mode_enabled`/`https_upgrades` false so the two do not both act.

