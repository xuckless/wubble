# 06 Privacy and security

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

## Containers, routing and the privacy tools (hardening pass, open)
- [ ] No way to create, rename or delete a container (only agent sessions make one): the routing rules' Container column can name only Default or a live agent container
- [ ] Routing rules: renaming or deleting a Space (or container) a rule names silently turns the rule off, with no mark in the list; Space names are not unique; a Space named with `,` or `->` cannot be named by a rule; the row label does not say typed addresses and page links are not routed
- [ ] Pill and counter: no check drives the shell by pointer for the blocker menu, the Sites list revoke or the Spaces list in a running window (only the engine side is driven, `notes/hp/privtools` of the pass)
- [ ] Pop-ups: an allowed site's pop-up opened without a click has no `window.opener` (the factory cannot see site permissions; pass it the decision if sign-ins need it); after a click a cross-origin frame can navigate the top page (Chrome's rule with a gesture)

## The built-in content blocker

## Build-out features (2026-11-22)

## Pop-ups: make the allow list work without a click, plus a default and the redirect notice (`popup-setting-allow-list`)

Key: `popup-setting-allow-list` · engine patch: no · state: not written.
Note: 0.6.1 shipped a working pop-up blocker: check what is left before writing.

Chrome: "Pop-ups and redirects" has a default (Don't allow, or Sites can send) and per-site Allow/Block exceptions. An allowed site opens script pop-ups with no gesture, and a blocked one shows the address bar's blocked-pop-up icon. In wubble, `wubble_core::popup::decide` (crates/wubble-core/src/popup.rs) and the `Permission::Popups` rows in `site.rs` already exist. But Chromium refuses a gesture-less `window.open` in `CanCreateWindow` before the host hears of it, so the no-gesture rows never run and the pill (`popups-blocked` chip in kit/chrome/address-pill.slint) never appears. This is the open TODO item.

Build: when each container's request context comes up (`Profiles::get` in crates/wubble-engine/src/storage.rs / app.rs), call `cef::RequestContext::set_content_setting(None, None, CEF_CONTENT_SETTING_TYPE_POPUPS, CEF_CONTENT_SETTING_VALUE_ALLOW)`. That puts every request through welding's popup path (vendor/welding/src/popup.rs, opened.rs) and lets wubble's table decide. Expose it as `Engine::set_content_setting(container, ...)`. Add the setting `popups.default` (Block|Allow, default Block) to crates/wubble-core/src/settings.rs, under Section::Privacy, and pass it as a new `default: Decision` argument to `popup::decide`. Keep the menu rows `Command::OpenBlockedPopups` / `AllowPopups` in crates/wubble/src/menu.rs, and add a "Manage" row that opens Settings at Sites. Redirects: Blink already blocks gesture-less cross-origin framebusting. When a load is cancelled that way (a console message or OnBeforeBrowse with no gesture), count it in the same pill as a blocked redirect with an "Allow" row that sets `Popups` Allow. Unit-test the new `decide` default rows in wubble-core.

## Containers: create, rename, delete, and assign from the UI (`container-management`)

Key: `container-management` · engine patch: no · state: not written.

wubble's containers are Firefox-style cookie jars (decision "Containers" and "Web storage per container" in docs/decisions.md). `wubble_core::model` already has `Command::NewContainer`, `DeleteContainer`, `SetSpaceContainer` and `ReopenInContainer` (crates/wubble-core/src/model.rs:186-200), and `Engine::forget_container` deletes a container's storage. But only agent sessions and the media smoke call them, so no user can make one (TODO "No way to create, rename or delete a container").

Build:
- Add `Command::RenameContainer { id, name }` to model.rs. Names are trimmed, non-empty and unique case-insensitively, and Default cannot be renamed or deleted. Agent containers are never listed or editable. Write unit tests in model/tests.rs.
- Add a Settings section "Containers" in crates/wubble-ui/ui/components/settings.slint, made only from kit components: a ListEditor with the columns Name, Used by (the Spaces that use it), and adds/rename/remove. Delete asks first through the existing dialog-sheet ("Delete Work? Its cookies, site data and cache are deleted"). Spaces that used it fall back to Default, which model.rs already does.
- Add a Space menu row "Container ▸" listing containers plus "New container…" (crates/wubble-core/src/spacemenu.rs), dispatching SetSpaceContainer.
- Add a tab menu row "Reopen in container ▸" (crates/wubble-core/src/tabmenu.rs) that dispatches ReopenInContainer.
- Wire it in crates/wubble/src/host/settings.rs and menu.rs. Persist through crates/wubble/src/persist.rs, which already stores containers.
- The routing rules' Container column (`routing.rules`) can then name user containers.

## Routing rules survive Space/container renames and show broken rules (`routing-rule-integrity`)

Key: `routing-rule-integrity` · engine patch: no · state: not written.

Open TODO under Privacy and containers. `routing.rules` (crates/wubble-core/src/routing.rs, Settings ListEditor) names Spaces and containers by display name. Renaming or deleting one silently turns the rule off. Space names are not unique, and a name containing `,` or `->` cannot be written in a rule.

Build:
- Store rules by id (SpaceId and ContainerId) and keep names only for display. Change the serialized form in settings.rs (kind List) to `pattern -> space:<id> [container:<id>]`. Add a migration from the old name form that resolves names once at load and keeps the original text for rows it cannot resolve.
- Add a pure `routing::resolve(rules, model) -> Vec<RuleState>` with the states Active, SpaceGone and ContainerGone. The Settings list shows a broken rule with a mark and the text "Space deleted" (a kit ListEditor row state; if no kit piece fits, a derived piece goes on docs/fidelity/derived-review.md).
- Make Space names unique: `model::Command::RenameSpace` / NewSpace append " 2" etc., like Chrome profile names.
- Change the row label to say that typed addresses and page links are not routed.
- Write unit tests in routing.rs and model tests.

## Chrome-style Site settings: defaults per permission, per-site detail, add exceptions (`site-settings-page`)

Key: `site-settings-page` · engine patch: no · state: not written.

Chrome's chrome://settings/content lists every permission and content type with a default ("Sites can ask" / "Don't allow") and Allowed/Blocked lists. Each site has a page with a dropdown per kind (Ask/Allow/Block), Reset permissions, and Delete data. In wubble, Settings > Sites (crates/wubble-ui/ui/components/settings.slint, about line 1003) shows only a read-only ListEditor of remembered decisions (Site, Permission, Decision) with remove, and there are no defaults or adds.

Build:
- In wubble-core site.rs, add a default per Permission, stored as settings keys `site.default.<as_str>` (Ask|Block, plus Allow for kinds Chrome allows by default), in crates/wubble-core/src/settings.rs. `SitePermissions::decide` takes the defaults (no row → the default; Ask stays Ask).
- Add a pure view model `site::SiteSettingsView` (per container: kinds → default, allowed origins, blocked origins), with unit tests.
- Shell: Sites becomes a list of kinds. Each row opens a sub-panel with the default dropdown and the Allowed/Blocked ListEditors with `adds` enabled (origin input validated by `site::origin_of`). Add a Site detail panel (one origin, container chooser) with a dropdown per kind, "Reset permissions", and "Delete site data" (reuses `ClearData::Site`). Compose only kit components.
- Host: crates/wubble/src/host/settings.rs writes through the store (wubble-store permissions.rs).
- The command bar action "Site settings" (crates/wubble-core/src/action.rs) opens the detail panel for the visible tab's origin.
Depends on permissions-per-container for the container key.

## Non-prompt content settings: JavaScript, images, sound, insecure content, sensors, downloads, JIT (`content-settings`)

Key: `content-settings` · engine patch: no · state: not written.

Chrome has content settings that are not prompts: JavaScript, Images, Sound, Insecure content (mixed script), Motion sensors, Automatic downloads, Protocol handlers, V8 optimizer (JIT), Intrusive ads, and Auto picture-in-picture. Each has a default and per-site exceptions. wubble has none of them, and the `Permission` enum in crates/wubble-core/src/site.rs covers only prompt kinds.

Build:
- Add `site::ContentSetting` in wubble-core: an enum plus a data table of (stored name, label, Chrome default, CEF `cef_content_setting_types_t`). Exceptions are stored per container and origin, in a new store table or in site_permissions with new kind names.
- Engine: add `Engine::set_content_setting(container, origin: Option<&str>, kind, value)` in crates/wubble-engine/src/app.rs. It calls `cef::RequestContext::set_content_setting(requesting_url, top_level_url, type, value)`, which exists in cef 156.6 bindings, on the container's context from `Profiles::get`. Replay the whole table when each context comes up and on every change.
- Sound: if CEF's SOUND setting does not mute, the host mutes the tab's browser (`set_audio_muted`) when its site is blocked. Put that in a pure function in wubble-core media.rs (`muted = user_mute || site_sound_blocked`).
- Insecure content: the Allow value maps to CEF_CONTENT_SETTING_TYPE_MIXEDSCRIPT for the site.
- UI: rows in the site-settings-page lists and the page-info popover.
- Write unit tests for the table, name round-trips and the mute rule.

## Page info popover from the lock: connection, permissions, cookies, site settings (`page-info-popover`)

Key: `page-info-popover` · engine patch: no · state: not written.

In Chrome, clicking the omnibox lock opens Page Info. It shows whether the connection is secure, with a sub-page that has "Certificate is valid". It lists the site's permissions that are not at their default, each with a toggle or dropdown, plus "Reset permissions". It shows cookies and site data in use ("N cookies in use", Manage), a toggle for third-party cookies on this site, "Turn on warnings" when a certificate exception exists, and links to Site settings. In wubble, the lock in kit/chrome/address-pill.slint is not clickable and nothing like this exists.

Build:
- Add an overlay kind `PageInfo` (tab-scoped, exclusive, Escape closes) in crates/wubble-core/src/overlay.rs.
- Add a pure view model `wubble_core::pageinfo` built from the tab's `SecurityLevel`/`SecurityInfo` (security-state feature), the container's SitePermissions and content settings, cookie counts and cert exceptions. Its rows are an enum, and its actions return `Command`s: SetPermission, ResetPermissions, ForgetCertException, OpenSiteSettings, ShowCertificate, ClearSiteData. Unit-test it.
- UI: a popover composed from kit components (popover/card/rows/toggle/select), new file crates/wubble-ui/ui/components/page-info.slint, opened by the pill's lock (`clicked` on the lock element).
- Host: crates/wubble/src/host/ (new pageinfo.rs) fills the model. The cookie count comes from the container's cookie manager visitor filtered by the site's domain (vendor/welding/src/cookies.rs CookieJar). Revoking a cert exception also calls `RequestContext::clear_certificate_exceptions`.

## Certificate interstitial: Advanced section, non-overridable errors, per-container exceptions (`cert-error-hardening`)

Key: `cert-error-hardening` · engine patch: no · state: not written.

Chrome's SSL interstitial hides Proceed behind "Advanced", which also shows the error code and a short explanation. It never allows proceeding for revoked certificates, pinned or HSTS hosts, or blocked interception. Exceptions are per profile and are cleared by "Turn on warnings" and by clear browsing data. wubble has the panel (crates/wubble/src/page_error.rs `PageError::Cert`, crates/wubble-core/src/neterror.rs `CertExceptions`, session-only, per host and code), with Proceed offered for every certificate code.

Build:
- In neterror.rs, add `fn overridable(code) -> bool` (false for ERR_CERT_REVOKED -206, ERR_CERT_KNOWN_INTERCEPTION_BLOCKED -217, ERR_SSL_PINNED_KEY_NOT_IN_CERT_CHAIN -150, ERR_CERT_SYMANTEC_LEGACY, and similar), following Chromium's `ssl_error_handler`/`ssl_blocking_page` (BSD-3-Clause). When the URL's host is an HSTS host (if CEF gives no HSTS bit, treat `CefSSLInfo` cert_status with STRICT_ENFORCEMENT as non-overridable), the panel shows no Proceed. Write unit tests.
- PageError::Cert gains `advanced: bool`, so the panel transition has an Expand step. The kit error panel shows the code name and a "View certificate" link (certificate-viewer feature).
- Exceptions are keyed by container (shared with permissions-per-container), are cleared by the clear-data dialog and page info, and `RequestContext::clear_certificate_exceptions` is called on a revoke so Chromium's cached allow goes too.

## Client certificate selection sheet (mutual TLS) (`client-cert-selection`)

Key: `client-cert-selection` · engine patch: no · state: not written.

Chrome shows "Select a certificate" when a server asks for a client certificate. It lists the subject and issuer of each matching certificate with OK and Cancel, and it auto-selects per the AUTO_SELECT_CERTIFICATE policy or setting. wubble does not handle `CefRequestHandler::OnSelectClientCertificate`, which exists in cef 156.6 bindings, so CEF's default runs and no choice is offered.

Build:
- welding: implement `on_select_client_certificate` in vendor/welding/src/handlers.rs. It returns 1 and holds the `CefSelectClientCertificateCallback` in the held-request table (vendor/welding/src/held.rs, like certificate and auth answers). It emits `NavigationEvent::ClientCertificate { id, host, port, certs: Vec<CertSummary { subject, issuer, not_after, der }> }` and adds `answer_client_certificate(id, Option<index>)` (a wubble change, listed in WUBBLE_CHANGES.md).
- wubble-engine page.rs / view.rs: add `PageEvent::ClientCertificate` and `View::answer_client_certificate`.
- crates/wubble/src/prompt.rs: a new `ClientCertPrompt` enum with a pure transition answering exactly once (the same shape as `Auth`). Remember the choice per (container, host:port) for the session. Agent tabs cancel with no prompt (fail closed, logged).
- UI: a Dialog-kind sheet (overlay `Kind::Dialog`) composed from kit list rows plus sheet buttons.
- Write unit tests for the transition.

## Third-party cookie blocking, per-site cookie exceptions, clear on exit (`cookie-controls`)

Key: `cookie-controls` · engine patch: no · state: not written.

Chrome's "Third-party cookies" settings offer Allow third-party cookies, Block in Incognito, and Block third-party cookies. There are per-site exceptions: allowed to use third-party cookies (user bypass, with a page-info toggle), Always clear cookies when windows are closed, and Block. A global "Clear cookies and site data when you close all windows" option exists too. wubble has none of these, and Chromium's defaults apply. The blocker's cookie-notice lists are something else.

Build:
- Add settings in crates/wubble-core/src/settings.rs (Section::Privacy): `cookies.third_party` (allow|block, default block, matching Chrome's current Incognito-only default and the privacy brief; the owner can flip it; note it in the report) and `cookies.clear_on_exit` (bool, default false).
- Apply them to every container's request context with `set_preference` (the pattern in crates/wubble-engine/src/view.rs:848, moved to an Engine-level `set_profile_pref(container, name, value)`): `profile.cookie_controls_mode` (0/1/2) and `profile.block_third_party_cookies`. Re-apply when each context comes up.
- Per-site exceptions: a `wubble_core::site::CookieRule {Allow, AllowThirdParty, SessionOnly, Block}` per (container, site), stored in the store, applied through `set_content_setting(url, top_url, CEF_CONTENT_SETTING_TYPE_COOKIES, …)`. For third-party allows, use a primary of `*` and a secondary of the site. Unit-test the mapping.
- Clear on exit: at quit (`crates/wubble/src/quit` / run_state.rs, before engine shutdown), delete cookies per site, globally or for SessionOnly sites, through the cookie manager and `Storage.clearDataForOrigin`. A crash is covered by doing the same at the next start before the first navigation.
- UI: rows in Settings Privacy, plus a toggle in page info.

## See and delete all site data per container (`site-data-viewer`)

Key: `site-data-viewer` · engine patch: no · state: not written.

Chrome's "See all site data and permissions" lists every site that stored something (cookies, local storage, IndexedDB, cache storage, service workers), grouped by registrable domain, with its size. You can delete one site or all of them, and expand a site to see its cookies. wubble can only forget the visible tab's origin (`View::forget_site`, CDP `Storage.clearDataForOrigin`, crates/wubble-engine/src/view.rs:1496) or clear whole containers.

Build:
- Engine: add `Engine::list_cookies(container, done)`, which visits every cookie through the container's cookie manager (reuse welding's CookieJar visitor, vendor/welding/src/cookies.rs, or storage.rs's visitor pattern). Add `Engine::site_usage(container, origins, done)` through CDP `Storage.getUsageAndQuota` and `Storage.getCookies`. These need a page of the container: use a live tab, else open a hidden about:blank view in that container (the media downloader's "live page of the same container" rule) and close it after. Add `Engine::forget_site(container, origin)` the same way, plus `delete_cookies(url)` for every scheme.
- wubble-core: add a pure `sitedata::group(cookies, origins) -> Vec<SiteRow { site (eTLD+1 via psl, already a dependency), container, cookies, bytes, origins }>`, with sorting and search. Unit-test it.
- Candidate origins come from the cookies' domains plus the store's history origins for that container.
- UI: a Settings > Sites "Site data" sub-panel with a container chooser, search, a ListEditor (Site, Cookies, Size, remove), "Delete all shown" with confirmation, and a per-site expand list of cookies (name, domain, expiry), each removable.

## Clear browsing data dialog: time range by kind, per container (`clear-data-dialog`)

Key: `clear-data-dialog` · engine patch: no · state: not written.

Chrome's Clear browsing data has a time range (Last hour, Last 24 hours, Last 7 days, Last 4 weeks, All time) and checkboxes: Browsing history, Download history, Cookies and other site data, Cached images and files, Passwords, Autofill form data, Site settings, and Hosted app data. In wubble, `Action::ClearBrowsingData` opens a menu (crates/wubble/src/menu.rs `clear_entries`) that deletes cookies, cache and auth of containers at all time, and forgets one site. History has its own range (`pages::ClearRange`, three ranges).

Build:
- In crates/wubble-core/src/pages.rs, extend `ClearRange` with LastWeek and Last4Weeks. Add a pure `ClearPlan { range, kinds: BTreeSet<ClearKind>, containers }` and `fn effects(plan, now) -> Vec<ClearEffect>`, with unit tests.
- Effects: history and recently-closed through the store's existing range delete; downloads list rows in range (wubble-store downloads.rs); passwords created in range (wubble-store logins.rs); site_permissions and content settings with `decided_at` in range; cert exceptions; cookies created in range through a cookie visitor that deletes when `creation >= from` (`ImplCookieVisitor` with `delete_cookie=1`, in crates/wubble-engine/src/storage.rs); site storage per origin visited in range (Storage.clearDataForOrigin through a page of the container, the site-data-viewer helper); and the HTTP cache. CEF has no ranged cache clear, so the row says "all time" for cache when the range is not All. Use `storage::clear`'s ClearHttpCache.
- UI: a sheet (overlay `Kind::Dialog`) with a range select, kit check boxes, a container choice (This container / All), and "Delete data". It replaces the menu, and the History page's clear opens it preset.

## Mark downloaded files with their origin (quarantine / MOTW / xattr) (`download-source-marks`)

Key: `download-source-marks` · engine patch: no · state: not written.

Chrome annotates every finished download. On macOS it sets the `com.apple.quarantine` xattr (with the agent name and the origin URL, so Gatekeeper checks the file) and `kMDItemWhereFroms`. On Windows it writes the Mark-of-the-Web `Zone.Identifier` ADS (ZoneId=3, ReferrerUrl, HostUrl) through IAttachmentExecute. On Linux it sets `user.xdg.origin.url` and `user.xdg.referrer.url`. With CEF and wubble's download handling it is not known that these are written, and the media downloader (wubble-ffmpeg muxes) writes files itself without them.

Build:
- Add a pure `wubble_core::download::source_marks(url, referrer, platform) -> Vec<Mark>` that strips credentials from URLs, as Chromium's `quarantine` component does (components/services/quarantine, BSD-3-Clause, with a source comment). Unit-test it.
- Add one shared trait `Quarantine` with a per-platform implementation (AGENTS.md: per-platform code implements one shared trait) in crates/wubble (or wubble-sysmem-style OS code). Linux uses `xattr` setxattr (the `xattr` crate, MIT/Apache). macOS uses setxattr `com.apple.quarantine` with the `0081;<hex time>;wubble;<uuid>` format plus the WhereFroms plist. Windows writes `<path>:Zone.Identifier`. It runs only when the mark is missing, so a CEF-written mark is kept.
- Call it from crates/wubble/src/host/downloads.rs on the Complete transition and from the media downloader's finish.
- A failure is logged, never fatal.

## Auto-revoke permissions of unused sites (Safety check) (`permission-auto-revoke`)

Key: `permission-auto-revoke` · engine patch: no · state: not written.

Chrome's Safety Hub (`UnusedSitePermissionsService`) removes granted permissions from sites not visited in 60 days. It tells the user under "Permissions removed from unused sites" with Undo, and the setting "Remove permissions from unused sites" is on by default. Notification grants for abusive sites are also revoked. wubble remembers grants forever (crates/wubble-core/src/site.rs, the store's `site_permissions.decided_at`).

Build:
- Add a setting `site.auto_revoke` (bool, default on) in settings.rs.
- wubble-core site.rs: add a pure `fn unused(rows: &[(container, origin, kind, decision, decided_at)], last_visit: impl Fn(container, origin) -> Option<i64>, now) -> Vec<Revocation>`. Only Allow decisions of prompt kinds (not Popups, ExternalLinks, SavePasswords or Block rows) are revoked, after 60 days without a visit and 60 days since the decision. Unit-test it.
- Host: at start, after the store loads (crates/wubble/src/persist.rs), compute it with the history store's last visit per origin (add a query in wubble-store history.rs: max visit time grouped by origin). Delete those rows, and record them in a new store table `revoked_permissions` (container, origin, kind, revoked_at) so they can be undone.
- UI: a Settings > Sites row "Permissions removed from unused sites" listing them, with Restore (put the row back) and Dismiss (forget the record).
- Depends on permissions-per-container for the container key.

