# Build-out batch 5

15 features, one branch each (`feat/<key>`, worktree `~/Dev/wubble-wt/feat/<key>/w`). Status is kept in [README.md](README.md).

## `orb-shell-wiring`: Wire the thinking orb into the shell: agent mark, activity header, approval sheet

Area: todo-a. Needs an engine patch: no.

TODO line 40. The orb is ported (`wubble_core::orb`: `Orb`, `Life`, `transition`; GPU painter `wubble_gfx::orb`; per-window stage in `crates/wubble/src/orbs.rs`) but is shown only by the gallery specimen (`kit/specimen/agent-widgets.slint`). Wire it into the places in docs/research/agent-widgets.md section 5: (1) the agent mark on a driven tab's row and on its Space header, 16 px, animated while that session (`wubble_core::agent` Session) has a call in flight, else a still frame; (2) the agent activity page's header, 32 px, while any session runs; (3) the approval sheet's title, 20 px, in the listening preset while the session is `AwaitingApproval`. Each place is a kit component property (an `Image` slot already in `kit/chrome/agent-mark.slint`, the agent sheet and the activity page kit parts): add an `orb` image input to those kit parts (derived, add a line to docs/fidelity/derived-review.md). orbs.rs gains cells keyed by place (tab id, page, sheet), created and dropped as models change, driven by the session state through `LifeEvent`. Only moving orbs redraw (orbs.rs `frame`). Reduced motion shows the still frame. Keep the existing kit lock rules (no drawing in shell .slint files). The owner's go on the proposal is still open; build it so each place can be switched off in one table.

## `fullscreen-page-alone`: Page fullscreen draws the page alone

Area: todo-b. Needs an engine patch: no.

In Chrome, fullscreen video is the page's compositor frame alone. In wubble, page fullscreen already hides the sidebar, frame and seam, and the scene does not step (decisions "Page fullscreen", "Scene follows the run state"). Slint still walks the shell's item tree, the scene's textures and over-layer images stay allocated and bound, and each page frame goes through the full render (TODO Optimization 1d).

To build: in crates/wubble-ui/ui/app.slint, put every shell subtree (sidebar, bars, overlays that are not the page's own: find, prompts and video controls stay) behind `if !root.page-fullscreen` conditionals. The shell file only lays out kit components, so no new visuals are needed. Then only the page `Image` and the page-owned surfaces remain. In crates/wubble/src/backdrop.rs, when `get_page_fullscreen()` turns on, release the scene's output and over-layer images (`set_scene`/`set_over_*` to an empty image) and the page pyramid P (`Renderer::set_pages(vec![])` path in crates/wubble-gfx/src/scene/render.rs). Rebuild them when fullscreen ends, the same way a resize does. In host/panes.rs, a fullscreen pane always takes the in-place damage path (`FramePresenter::present_damaged`), so only the video's rect is marked dirty. Count `fullscreen_frames` in `--measure` so it can be compared with Chromium. All platforms.

## `nav-stop-reload`: Reload button turns into Stop while loading; reload menu with hard reload and empty cache

Area: navigation. Needs an engine patch: no.

While a page loads, Chrome's reload button turns into an X (Stop), and back into Reload when the load ends. Shift-click or Ctrl-click does a hard reload. With DevTools open, a right-click or long press gives Normal reload, Hard reload and 'Empty cache and hard reload'. wubble has Action::Stop (Escape, Cmd+.), Action::HardReload (ViewOp::HardReload, reload_ignore_cache in crates/wubble-engine/src/view.rs) and an `address-loading` property in app.slint, but NavButtons (crates/wubble-ui/ui/components/nav-buttons.slint) always shows Reload.

Add `in property <bool> loading` to NavButtons, bound from the visible tab's loading state (the same source as address-loading in crates/wubble/src/bind.rs). The fourth button's icon, label and id become Glyph.close / 'Stop' / 'stop' while loading, swapped with a kit crossfade (kit/motion/crossfade.slint). If the kit's icon set has no stop glyph, use close and note it in docs/fidelity/derived-review.md. Add a modifiers argument to `action(...)` so the host maps Shift or Mod on reload to hard-reload. The NavButtons button table stays one table.

Add Action::EmptyCacheHardReload (FIXED table and PALETTE). It sends the CDP call Network.clearBrowserCache through View::cdp (crates/wubble-engine/src/cdp.rs), then ViewOp::HardReload. A right-click on the reload button opens a Menu::Shell (crates/wubble/src/menu.rs) with the three rows; the third is shown only when wubble_core::devtools says DevTools is open for that tab. Unit tests in perform.rs for the plans.

## `zoom-indicator`: Zoom indicator and zoom controls like Chrome's omnibox zoom bubble

Area: page-interaction. Needs an engine patch: no.

In Chrome, when a site's zoom differs from the default, a magnifier icon shows in the address bar. Clicking it opens a bubble with the percentage, −, + and Reset. The app menu has a Zoom row (−, percent, +, fullscreen), and Settings > Appearance > Page zoom lists the per-site levels. wubble has per-site zoom (`wubble_core::zoom::ZoomTable`, `SITE_ZOOM`/`ZOOM_DEFAULT` settings, host `user_zoomed` in crates/wubble/src/host/surfaces.rs), a brief toast after Ctrl +/-/0 (components/toast.slint), and a site-zoom list in settings (`settings-site-zoom`). Nothing persistent shows that a page is zoomed, and there is no pointer path to zoom. To build: a `zoomed` percent property for the visible tab (from `PageEvent::ZoomChanged` against the default), shown as a small kit chip in the address pill (a derived kit piece composing existing kit parts, on the derived-review sheet) only when it is not the default. The chip opens a popover with −, the percent, + and Reset, which perform `Action::ZoomOut`/`ZoomIn`/`ZoomReset`. Add a Zoom row to the page menu (`window_page_entries` or the page's own menu in crates/wubble/src/menu.rs). The popover closes on Esc and on outside click as the other popovers do. A pure fn in wubble-core decides whether the chip shows (percent, default), with tests.

## `content-settings`: Non-prompt content settings: JavaScript, images, sound, insecure content, sensors, downloads, JIT

Area: privacy-security. Needs an engine patch: no.

Chrome has content settings that are not prompts: JavaScript, Images, Sound, Insecure content (mixed script), Motion sensors, Automatic downloads, Protocol handlers, V8 optimizer (JIT), Intrusive ads, and Auto picture-in-picture. Each has a default and per-site exceptions. wubble has none of them, and the `Permission` enum in crates/wubble-core/src/site.rs covers only prompt kinds.

Build:
- Add `site::ContentSetting` in wubble-core: an enum plus a data table of (stored name, label, Chrome default, CEF `cef_content_setting_types_t`). Exceptions are stored per container and origin, in a new store table or in site_permissions with new kind names.
- Engine: add `Engine::set_content_setting(container, origin: Option<&str>, kind, value)` in crates/wubble-engine/src/app.rs. It calls `cef::RequestContext::set_content_setting(requesting_url, top_level_url, type, value)`, which exists in cef 156.6 bindings, on the container's context from `Profiles::get`. Replay the whole table when each context comes up and on every change.
- Sound: if CEF's SOUND setting does not mute, the host mutes the tab's browser (`set_audio_muted`) when its site is blocked. Put that in a pure function in wubble-core media.rs (`muted = user_mute || site_sound_blocked`).
- Insecure content: the Allow value maps to CEF_CONTENT_SETTING_TYPE_MIXEDSCRIPT for the site.
- UI: rows in the site-settings-page lists and the page-info popover.
- Write unit tests for the table, name round-trips and the mute rule.

## `perm-per-container`: Site permission decisions per container, not per origin

Area: web-platform-ui. Needs an engine patch: no.

TODO.md: "Site permission decisions are per origin, not per container (a camera allowed in one container is allowed in all)". The containers decision (docs/decisions.md) says containers share nothing: one CEF request context each, with its own permission store. wubble's own table (wubble_core::site::SitePermissions, a BTreeMap keyed by (origin, Permission)) is global, and crates/wubble/src/prompt.rs `PermissionRequest::verdict` consults it with no container. Chrome profiles keep separate content settings.
Build: (1) Key the table as (container id, origin, kind). Container ids come from wubble_core::space. Make `decide`, `set`, `forget` and `iter` take the container. Unit tests cover isolation and the external-link keys (wubble_core::external::key stays per scheme and site within a container). (2) Carry the tab's container on PermissionRequest; host/surfaces.rs builds it from the tab's Space/container. (3) Persist: add a container column to the site-permission store in crates/wubble/src/persist.rs, migrating existing rows to every existing container. Copying preserves the previous behaviour; the migration and the decision go in the report. Add a persistence test. (4) Settings Sites shows the container beside each row when more than one exists, and filters to the active Space's container by default. (5) Deleting a container drops its rows. The SavePasswords Block rows stay per container too.

## `download-auto-open`: Open when done, always open this type, open safe files

Area: media-downloads. Needs an engine patch: no.

Chrome lets a running download be marked "Open when done". On a finished one it offers "Always open files of this type", stored per extension (download.extensions_to_open) and refused for dangerous types. The design's Settings (ref/the-wallpaper-project/src/wubble/pages/settings.tsx, `openSafe`) also has "Open safe files after downloading". wubble opens a file only on a click of Open (host/downloads.rs download_action DownloadAction::Open).

What to build: (1) wubble-core/src/download.rs: an `open_when_done: bool` on DownloadEntry, a pure `fn opens_on_finish(entry, always_types, open_safe) -> bool` (never for a dangerous type, which comes from the danger table if that feature has landed, else from a small built-in list of executable extensions), and a `DownloadEffect::OpenFile` emitted on the Finished transition when it returns true. Write unit tests. (2) Settings: a `downloads.open_types` list setting (Section::Hidden, edited from the row menu) and a `downloads.open_safe` Bool row, default off, in wubble-core/src/settings.rs, with SETTINGS-table tests. (3) Host: run OpenFile with `opener::open`. Persist open_when_done in a store migration, so it survives a restart for a resumed item. (4) UI: DownloadAction gets `open-when-done` and `always-open-type`. A small row menu on the popover's ADownloadRow and the page's DownloadRow lists Open when done (checked), Always open files of this type, Copy link and Show in folder. Compose it from the kit menu (kit/controls/menu.slint) and put it on derived-review.md. All platforms.

## `pw-never-saved-manage`: Passwords page: never-saved sites list and Manage passwords entry points

Area: passwords-autofill-sync. Needs an engine patch: no.

Chrome lists sites where you chose "Never" in Password Manager settings ("Declined sites and apps") with a remove button, and its autofill dropdown ends with "Manage passwords...". wubble stores "Never" as `Permission::SavePasswords` = `Decision::Block` in `site_permissions` (crates/wubble/src/host/autofill.rs near line 288, crates/wubble-store/src/permissions.rs) and shows it only inside Settings' generic Sites list; the chip menu (`MenuRow` in crates/wubble-core/src/autofill.rs) has only `Login` and `Generate`. Build: (1) a "Never saved" ListCard section on `PasswordsPage` (crates/wubble-ui/ui/components/pages.slint) fed from `site_permissions` rows of `SavePasswords`/Block, each with a Remove action that clears the decision (existing site-permission revoke path), listed even while the vault is locked since origins are not secret; host in crates/wubble/src/host/pages.rs. (2) `MenuRow::Manage` appended to `rows()` (label "Manage passwords") opening `InternalPage::Passwords` with the page's search prefilled with the site, implemented as a new `Effect::OpenManager { origin }` executed in crates/wubble/src/host/autofill.rs. (3) A "Delete all passwords" hold button (kit `KeyedHoldButton`, as `start-over`) as Chrome's "Delete all data". (4) The `wubble://passwords` page accepts a `?q=` site filter from the omnibox. Unit tests for the new menu row and effect in autofill.rs, and for the never-saved list builder in wubble-core/src/pages.rs. Gallery scene for the page with the new section.

## `ext-inspect-views`: DevTools for extension popups, service workers and options pages

Area: devtools-extensions-internals. Needs an engine patch: no.

In Chrome, the extension card offers "Inspect views: service worker, popup.html", and a right-click on the action offers "Inspect popup", which keeps the popup open while DevTools is attached. wubble has neither. The popup is a windowless view (`crates/wubble/src/host/ext_ui.rs::open_popup`, pane `POPUP_TAB`) that closes on an outside press, and the service worker has no view. To build: (1) In the Settings extensions list and the footer popover row, add an "Inspect" item (kit menu rows) for an extension whose `ext_api::available()` is true. (2) Popup: open CEF's detached DevTools on the popup's view (`View::open_devtools_at(None)`), or the bridge's undocked window. While DevTools is attached, the panel lifecycle (`wubble_core::extensions::shell::panel_transition`) gets a `PanelEvent::Pinned` state in which an outside press or Escape does not close the popup, as in Chrome. (3) Service worker: find its target by `Target.getTargets` (type `service_worker`, url `chrome-extension://<id>/`) over a browser-target session (`Target.attachToBrowserTarget` from any view's DevTools channel), then attach the devtools-pipe-bridge frontend with `Target.attachToTarget {flatten: true}` and route by `sessionId`. If CEF refuses the browser target to its page client, fall back to `devtools.remote_port`'s `/json/list` and say so in the row. (4) Options pages are tabs already, so F12 works there. Add pure tests for the Pinned transition in `extensions/shell.rs`.

## `default-browser`: Default browser: status, a Settings row, and registration on Linux, macOS and Windows

Area: platform. Needs an engine patch: no.

Chrome shows "Default browser" in Settings with "Make default" and the current status. wubble has no code for this (TODO.md M7 "Default-browser registration, URL handlers"). The Linux desktop entry (`assets/icons/linux/wubble.desktop`) lists only text/html, xhtml, http and https. To build: (1) `wubble_core::default_browser`: an enum `Status {Unknown, Checking, Default, NotDefault, Unsupported, Failed}` and a pure `transition(state, Event::{Check, MakeDefault, Answered(..)}) -> (state, Vec<Effect::{Probe, Register}>)`, with unit tests. (2) A trait `DefaultBrowser {fn is_default(&self) -> Result<bool>; fn make_default(&self) -> Result<()>}` in `crates/wubble/src/default_browser.rs`, run on a worker thread, with one impl per OS. Linux: `xdg-settings check/set default-web-browser wubble.desktop`, plus `xdg-mime default` for the scheme handlers. Unsupported inside Flatpak (Chrome's flatpak says so too). macOS: compare `NSWorkspace URLForApplicationToOpenURL:` for `https:` against bundle id `dev.wubble.browser`, and set with `setDefaultApplicationAtURL:toOpenURLsWithScheme:completionHandler:` for http and https (macOS shows its own confirmation). Windows: write the HKCU `Software\Clients\StartMenuInternet\wubble` Capabilities, `RegisteredApplications` and a `wubbleHTML`/`wubbleURL` ProgId, then open `ms-settings:defaultapps?registeredAppUser=wubble` (Windows 10+ forbids setting it silently). (3) Settings General row: status text and a "Make default" button, built from kit parts like the Updates group's "Check now" (`ui/components/settings.slint`:604, `UpdatesView`). (4) desktop entry: Chrome's MimeType set (image/*, svg, pdf, webp, xml) and `Actions=new-window;` with `[Desktop Action new-window] Exec=wubble --new-window`. No request leaves the machine.

## `orb-gravity-warp`: Thinking orb: port the gravity cursor warp

Area: todo-a. Needs an engine patch: no.

TODO line 42. The web ThinkingOrb has a `gravity` prop (`ThinkingOrb.tsx`, `gravity.ts` in the design repo `xuckless/the-wallpaper-project` src/wubble): it warps the dots toward the pointer through a sprite. wubble's port (`crates/wubble-core/src/orb/*.rs`, painter `crates/wubble-gfx/src/orb.rs`, cells in `crates/wubble/src/orbs.rs`) leaves it out (listed in docs/fidelity/deviations.md). Port it: a pure `gravity` module in wubble-core/orb that takes the pointer position relative to the orb (logical px, or None) and returns each mark's offset, with the design's falloff and constants exactly (put the constants through `tools/fidelity/extract.mjs` into kit_gen if they are design values). Add a `pointer` input to `Orb`/`Life`, so a pointer moving over an orb with gravity on makes `Life` say it moves (it redraws only while the warp changes, then settles). Feed the pointer from the specimen's TouchArea (`kit/specimen/agent-widgets.slint`) through `orbs.rs`. Add unit tests for the pure warp in wubble-core (new core logic needs them under AGENTS.md; they are no performance tests). Remove the deviation line.

## `scene-damage-shows-through`: Scene renders mark only what the scene shows through

Area: todo-b. Needs an engine patch: no.

A scene render (ambience, about 8 to 10 per second during video and scroll) currently returns `Damage::Whole` unless only the start page's card changed (crates/wubble-gfx/src/scene/start.rs `Damage`; crates/wubble/src/backdrop.rs `frame` calls `partial::mark_window`). Slint then redraws the whole window, including the page card, which covers the scene.

To build: extend `Damage` with a region variant, for example `Rects(SmallVec<[Rect; 4]>)`, and make `Renderer::before_rendering` (scene/render.rs) compute it. When only the window-glass ambience was rebuilt, the damage is the window's glass surfaces where they do not lie under an opaque page card. Use `pages::by_layer`, `rect_of` and the `PageRect`s already held; the visible scene is the sidebar, the gaps around the card and any transparent pane (split, bowed edge, start). When over-layer glass was redrawn, the damage is those surfaces' rects. Use Whole only for a palette change, a resize, new textures or a backdrop that moved under everything. Add `Damage::and` merging for the new variant. In backdrop.rs, call `partial::mark_rect` once per rect; Slint's dirty region merges them. Pure unit tests in start.rs and pages.rs for which rects come out, and a `partial_render` test in wubble-ui for a scene frame that does not redraw the page. All platforms.

## `swipe-nav-macos-windows`: History swipe on macOS and Windows touchpads (and X11)

Area: navigation. Needs an engine patch: no.

In Chrome, two fingers sideways go back or forward on every platform. On macOS that is the NSEvent scroll phases (it is the 'Swipe between pages' gesture), with the arrow bubble following the fingers. wubble's history swipe (crates/wubble-core/src/gesture.rs SwipeInput, crates/wubble/src/host/gesture.rs, chip in components/swipe-chip.slint) is fed only from Wayland through crates/wubble/src/wl. host/gesture.rs says 'Nothing reports a history swipe elsewhere yet (macOS's would be winit's PanGesture)'.

macOS: feed SwipeInput from winit's WindowEvent::PanGesture (phase Started, Moved, Ended), or from MouseWheel with TouchPhase and PixelDelta when the delta is mainly horizontal. Respect the system 'Swipe between pages' preference: read NSEvent.isSwipeTrackingFromScrollEventsEnabled through objc2 in a platform impl of one shared trait, alongside os_motion.rs and os_scope.rs. Phases end the gesture as the Wayland path does, and the PageScrolled give-way stays.

Windows and X11: precision touchpads arrive as horizontal MouseWheel PixelDelta with no phases. Add a pure idle-timeout end (no delta for 150 ms counts as release) to gesture.rs's transition as a new SwipeInput::Idle, with the timer driven by the host's tick.

Add unit tests for the idle end and the phase mapping in gesture.rs. Keep it to one path: the platform code turns OS events into SwipeInput only, and the rule stays in wubble-core.

## `window-print-route`: Page window.print() opens wubble's print sheet on every platform

Area: page-interaction. Needs an engine patch: no.

In Chrome, `window.print()` (and a print button inside a page) opens the same print preview as Ctrl+P. It fires `beforeprint`/`afterprint` and blocks the script until the dialog closes. wubble's Ctrl+P and menu Print open the sheet (`wubble_core::print::Sheet`, crates/wubble/src/host/print.rs, PrintToPDF plus `lp`), but welding's `print()` errors on Linux ("requires an embedder-owned print handler", vendor/welding/src/linux_cef/mod.rs). No CefPrintHandler is registered, so a page's `window.print()` does nothing on Linux. On macOS and Windows it would reach Chromium's native dialog in a windowless view, which is not wubble's sheet. To build: register a CefPrintHandler in welding (a `wubble change`, listed in WUBBLE_CHANGES.md; cef 156 has `on_print_start`, `on_print_settings`, `on_print_dialog`, `on_print_job`, `get_pdf_paper_size`). On Linux, `on_print_dialog` holds the callback and emits a new `PageEvent::PrintRequested { tab }` that opens the sheet through `Host::print`. Submitting answers the held callback with cancel, since wubble renders its own PDF, so the page's afterprint fires. On macOS and Windows, a helper content script replaces `window.print` with a call to the same event (keeping beforeprint/afterprint ordering), so every platform shows the one sheet. Add a `print::Event` for a page-started open and test it in crates/wubble-core/src/print.rs.

## `page-info-popover`: Page info popover from the lock: connection, permissions, cookies, site settings

Area: privacy-security. Needs an engine patch: no.

In Chrome, clicking the omnibox lock opens Page Info. It shows whether the connection is secure, with a sub-page that has "Certificate is valid". It lists the site's permissions that are not at their default, each with a toggle or dropdown, plus "Reset permissions". It shows cookies and site data in use ("N cookies in use", Manage), a toggle for third-party cookies on this site, "Turn on warnings" when a certificate exception exists, and links to Site settings. In wubble, the lock in kit/chrome/address-pill.slint is not clickable and nothing like this exists.

Build:
- Add an overlay kind `PageInfo` (tab-scoped, exclusive, Escape closes) in crates/wubble-core/src/overlay.rs.
- Add a pure view model `wubble_core::pageinfo` built from the tab's `SecurityLevel`/`SecurityInfo` (security-state feature), the container's SitePermissions and content settings, cookie counts and cert exceptions. Its rows are an enum, and its actions return `Command`s: SetPermission, ResetPermissions, ForgetCertException, OpenSiteSettings, ShowCertificate, ClearSiteData. Unit-test it.
- UI: a popover composed from kit components (popover/card/rows/toggle/select), new file crates/wubble-ui/ui/components/page-info.slint, opened by the pill's lock (`clicked` on the lock element).
- Host: crates/wubble/src/host/ (new pageinfo.rs) fills the model. The cookie count comes from the container's cookie manager visitor filtered by the site's domain (vendor/welding/src/cookies.rs CookieJar). Revoking a cert exception also calls `RequestContext::clear_certificate_exceptions`.

