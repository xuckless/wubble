# Apple Passwords (iCloud Keychain) and passkeys in a CEF browser: research

Question: How can wubble use Apple's Passwords app (iCloud Keychain) and passkeys on macOS with prebuilt CEF?
Status: Shelved (TODO.md "Shelved ideas"): the polished path is Apple's browser-extension native messaging helper, which needs an entitlement request the owner files; passkeys follow the same account.

Date: 2026-11-19. Research only; nothing in the repo was changed.

Tags: **[V]** verified from a cited source or read directly off the Mac (macOS 27.3,
read-only); **[I]** inferred or background knowledge; **[?]** not found.

Local checks were `codesign -d`, `strings`, `plutil`, `defaults read` and header reads in
`/Applications/Xcode.app` SDK `MacOSX27.sdk`. Nothing was launched, installed or written there.

---

## 0. Summary (read this first)

1. **No public API gives a third-party app another site's iCloud Keychain password.** [V] Keychain
   access groups hide Safari's and the Passwords app's items (Apple DTS, thread 676891). The
   password APIs (`ASAuthorizationPasswordProvider`, shared web credentials) only serve the app's
   own associated domains (Apple Frameworks engineer, thread 134077; Apple "Password AutoFill"
   article). The Passwords app has no URL scheme for it and no AppleScript (checked on the Mac).
2. **Orion does it with the one system UI that works for any app: the AutoFill > "Passwords…" panel
   of a native `NSTextField`.** [V] An Orion developer said so on the feedback site: that popup "only
   appears for native NSTextField controls … completely handled by macOS, so Orion used a tricky
   solution". The panel is given no site, so it lists every login, and the search box is not
   pre-filled. That is the "ux hell" the owner saw. The two "passwords buttons" are WebKit's
   key-icon AutoFill button (Orion's own menu) and the system provider panel behind "Password
   Providers…". [I for the exact mapping, V for the pieces].
3. **The polished path is the one Chrome, Edge, Firefox, Arc, Brave, Zen and Orion's extension use:
   Apple's `PasswordManagerBrowserExtensionHelper` (native messaging host, protocol
   `com.apple.passwordmanager`).** [V] It takes the site URL, returns only matching logins,
   verification codes (TOTP), saves new logins, asks Touch ID itself. Since macOS 15.5 it only
   talks to allow-listed browsers, enforced by **launch constraints signed into the helper**. [V]
4. **New finding, read off the helper on this Mac (macOS 27.3): its parent launch constraint is
   an `$or` of (a) any parent holding the entitlement
   `com.apple.developer.web-browser.public-key-credential` = true, or (b) a list of
   (signing-identifier, team-identifier) pairs.** [V] So a browser that holds the passkey
   entitlement (which wubble wants anyway) satisfies the constraint with no Apple allow-list
   entry. Whether older macOS (15.5 to 26.x) has clause (a) is [?]; the list alone was the
   original design. A Swift browser project (Crest, PR 716) bet on the same thing and never ran it. [V]
5. **Wubble can speak that protocol itself, with no extension system (so before M6)**: spawn the
   helper from the main process, SRP-6a (3072-bit) pairing with a 6-digit code, AES-GCM session,
   JSON commands. Two Apache-2.0 reference clients exist. [V] It is undocumented, so there is
   breakage and policy risk. [I]
6. **Passkeys need the entitlement, and wubble's windowless (Alloy-style) views have no
   WebAuthn dialog host in Chromium, so they need a JS-shim plus `ASAuthorizationController` bridge.**
   [V reports, I for the fix] Chromium's own iCloud Keychain code is in `device/fido/mac` and uses
   a private SPI; it is not usable from OSR. A public-API bridge exists (clientData with origin,
   macOS 13.5+). [V headers]
7. **What the owner needs from Apple:** an organisation Apple Developer account whose Account
   Holder submits the "macOS Browsers Passkeys" form. [V] Review is manual and slow (one report:
   7+ weeks pending). Everything else (Developer ID signing, notarisation, a provisioning profile
   carrying the managed capability) is routine.

Recommended order: ship the Orion-style system panel as a stop-gap only if wanted (it is cheap but
not better than Orion); build the native helper client behind a flag as the real feature; request
the entitlement now (long lead time); file the GitHub issue to be allow-listed in parallel; build
the passkey bridge once the entitlement exists.

---

## 1. Mechanisms for a third-party macOS app to get a saved iCloud password

### 1.1 System AutoFill panel through a native text field (what Orion does)

- **API**: a real `NSTextField`/`NSSecureTextField` with `contentType = .username / .password`
  (`NSTextContentType`, AppKit). [V] Apple's Password AutoFill article says AppKit apps have full
  support "starting with macOS Big Sur" (quoted by a developer in Apple forum thread 809587; the
  Apple article itself covers iOS first). The same system UI is the right-click menu item
  AutoFill > Passwords…. [V user reports; Orion forum]
- **Entitlements**: none for the panel itself. For *domain-filtered* suggestions the app needs an
  Associated Domains entitlement (`webcredentials:`) and an AASA file on that domain. [V] Apple:
  Password AutoFill "recommends credentials only for your app's associated domain". That means
  the app's own domains; it cannot name arbitrary sites. [I from the same text]
- **Domain**: not passed. [V] KeePassium's support told an Orion user that when AutoFill is
  opened from Orion the "Context" is empty, while Safari supplies the page URL
  (orionfeedback 6932 post 18). An Orion user listing the Passwords panel steps: "every single
  login you have saved … instead of just the logins for the site" (8590 post 33).
- **What the app receives**: the system types the chosen username/password into the native field;
  the app reads the field. [I] (No delegate callback exists for this path. `performAutoFillAssistedRequests` is iOS/visionOS only. [V Apple docs])
- **UI**: the macOS AutoFill/Passwords list under the field, Touch ID/password to unlock. [V]
- **Custom NSView with `NSTextInputClient`**: [?] nothing found that makes a custom view show the
  panel; Orion's developer says only native `NSTextField` controls get it. [V]
- **Min macOS**: Big Sur (11) for AppKit AutoFill [V quote]; works on current macOS per Orion 1.1.4 behaviour.
- **Fit for wubble**: feasible without Apple approval: add a tiny hidden `NSTextField` pair as a
  subview of winit's `NSView` (via `objc2`), make it first responder, read the values after the
  user picks. Same weakness as Orion: whole list, no site. [I] Possible refinement to spike:
  whether pre-selecting text or setting the field's value makes the panel search pre-filter [?].

### 1.2 `ASAuthorizationController` + `ASAuthorizationPasswordProvider`

- **API**: `ASAuthorizationPasswordRequest`, macOS 10.15+. [V SDK header: `macos(10.15)`]
- **Entitlements/domain**: an Apple Frameworks engineer: the app "must have a domain associated
  with it through the Associated Domains entitlement, and a matching credential must already exist
  in the keychain for that domain" (thread 134077). [V] A developer in the same thread got errors
  `AKAuthenticationError -7089` / `AuthorizationError 1000` on macOS with a correct setup. [V]
- **Verdict**: not a browser mechanism. A browser cannot associate every site's domain. [I]
  `com.apple.developer.web-browser.public-key-credential` covers passkeys and security keys only
  ("passkeys and security keys for any relying party identifier"). [V Apple entitlement page]
  There is no equivalent browser entitlement for passwords. [?] none found.
- **Safari's own private entitlement** `com.apple.authentication-services.allow-authentication-request-any-rpid`
  is on Safari's signature. [V `codesign` on the Mac] Orion's engineer calls the domain-aware
  API "available to Safari only" (8590 post 12); Strongbox/KeePassium AutoFill gets the URL only
  from Safari. [V]

### 1.3 `SecRequestSharedWebCredential` / shared web credentials

- Deprecated: `macos(10.16,11.0)` "Use ASAuthorizationController to make an ASAuthorizationPasswordRequest". [V SDK header]
- Needs permission "from the app, the website, and the user" and associated domains. [V Apple article]
- Dead end for a browser. [I]

### 1.4 Keychain Services direct read

- iCloud Keychain on Mac is the data-protection keychain; items are only visible to access groups
  the app is entitled to, and Apple will not issue provisioning profiles for Apple's own groups
  (DTS Quinn, thread 676891, for `com.apple.ssh.passphrases`). [V] Safari's/Passwords' login
  groups are the same class of thing. [I] Orion's developer: "Apple does not allow apps writing in each
  other's keychain" (8089 post 4) and passwords in Safari "can be accessed, but not updated"
  only through the system API. [V]
- Orion's own Keychain is separate: its entitlement is `keychain-access-groups =
  TFVG979488.com.kagi.shared` plus iCloud containers. [V `codesign`] It syncs through iCloud
  Keychain among Orion installs, and is not the Passwords app's data. [V Orion forum, Orion strings
  "keychain_explanation"]

### 1.5 URL schemes / AppleScript / App Intents for the Passwords app

- `Passwords.app` registers only `otpauth`, `apple-otpauth`, `otpauth-migration`,
  `apple-otpauth-migration`. No `NSAppleScriptEnabled`, no scripting definition. [V on the Mac]
- The system has `PasswordManagerAppIntentsExtension.appex` (App Intents). [V] Its intents were
  not inspectable without launching; nothing public says it returns passwords. [?]
- Nothing here returns a credential to a third party. [V for URL/AppleScript, ? for intents]

### 1.6 Apple's iCloud Passwords extension and its native messaging helper

This is the polished route. How it works:

- **Helper**: `/System/Cryptexes/App/System/Library/CoreServices/PasswordManagerBrowserExtensionHelper.app`
  (bundle id `com.apple.PasswordManagerBrowserExtensionHelper`, macOS 14+). [V on the Mac;
  au2001 README says Sonoma and later preinstalled]
- **Registration**: macOS ships native messaging manifests
  `/Library/Google/Chrome/NativeMessagingHosts/com.apple.passwordmanager.json` (allowed origins
  `chrome-extension://pejdijmoenmkgeppbflobdenhhabjlaj/` and `…mfbcdcnpokpoajjciilocoachedjkima/`)
  and `/Library/Application Support/Mozilla/NativeMessagingHosts/…json` (allowed extensions
  `password-manager-firefox-extension@apple.com`, `apple-passwords-firefox-extension@apple.com`). [V on the Mac]
- **Transport**: Chrome-style native messaging: stdio, length-prefixed JSON. [V Xenon PR 38]
- **Pairing**: SRP-6a, 3072-bit group, SHA-256, a 6-digit code shown by the helper; then
  AES-GCM per message. [V Crest PR 716; au2001 `src/utils/srp.ts`, `api.ts`: `GET_CAPABILITIES`,
  `HANDSHAKE` with `PAKE` messages `MSGTypes` 0..3, `secretSessionVersion` 1 (RFC verification)]
  Apple support: after quitting the browser a new code is needed. [V Apple Community doc 250007756]
- **Commands** (from the helper's strings and apw `const.ts`): `GET_LOGIN_NAMES_FOR_URL`,
  `GET_PASSWORD_FOR_LOGIN_NAME`, `SET_PASSWORD_FOR_LOGIN_NAME_AND_URL`, `NEW_ACCOUNT_FOR_URL`,
  `GET_ONE_TIME_CODES`, `DID_FILL_ONE_TIME_CODE`, `GetStrongPasswordSuggestions`,
  `LAUNCH_PASSWORDS_APP`, `OPEN_URL_IN_SAFARI`, `TAB_EVENT`, `GET_CAPABILITIES`. [V]
- **Domain**: passed as the URL in the request body (`URL`, `frameURLs`). So only the matching
  logins come back, and an Orion-style "whole list" problem does not exist. [V apw `client.ts`]
- **UI**: the helper shows the pairing code, the Touch ID/password prompt and add/update
  confirmation alerts itself, placed "over client browser window" (selectors
  `showAlertOverClientBrowser:`, `requestAuthForClientBrowser:`). [V strings in the helper] The
  browser draws the login picker. wubble would draw it in Slint. [I]
- **Values**: the client receives username and password as JSON values after the user picks,
  and fills the page itself. [V au2001 `FILL_PASSWORD`; apw]
- **The gate (macOS 15.5+)**: Apple's README: "As of macOS Sequoia version 15.5 and above" the
  helper communicates "only with known web browsers"; implemented with launch environment
  constraints signed into the helper; "an operating system update is required for additional
  web browsers"; to be added "raise a GitHub issue or submit a pull request" at
  `apple/password-manager-resources` (`quirks/web-browser-extension-distribution-information.json`).
  [V] A web browser is defined there as an app that opens http/https and, on launch, shows a URL
  field, search tools or bookmarks. [V]
- **What the constraint is on this Mac (macOS 27.3)** [V `codesign -d -vvvv` on the helper]:
  `Has Self Launch Constraints`, `Has Parent Launch Constraints`; parent must satisfy
  `$or-array` of: entitlement query `com.apple.developer.web-browser.public-key-credential`
  equals true; **or** (`signing-identifier` in a set, `team-identifier` = X) for each listed
  browser. The list includes Chrome, Edge, Firefox, LibreWolf, Arc/Dia, Brave, Vivaldi, Opera,
  Orion (`com.kagi.kagimacOS`, team `TFVG979488`), ungoogled-chromium, Zen, DuckDuckGo, ChatGPT
  Atlas, Comet, Waterfox, Island, and ~70 more (Apple's JSON has the same set). The helper also
  self-checks: "Missing Required Launch Constraints … file a radar" in its strings. [V]
- **Same helper on Windows**: iCloud for Windows ships the equivalent; Apple support lists Chrome,
  Edge, Firefox for Windows. [V Apple support guide icw2babf5e03 via search] Windows launch
  checks: a Tor Project issue says there is an allow-list and a signature check "introduced later"
  [secondary, could not read: 403]; so Windows policy is [?].
- **Why Orion needed Apple's help**: after macOS 15.4 the extension broke in Orion, Zen,
  ungoogled-chromium, Dia. A Kagi staff member (dino): it was "resolved by providing required
  information to Apple team, which they included in latest macOS releases" (orionfeedback 11230
  post 3). Another user read the allow-list out of the helper binary to see that Orion RC was
  missing (10025 post 22). [V] Apple's Ricky Mondello is cited as saying a solution was in the
  works (post 18, link not retrievable). [V second-hand]
- **Reimplementations of the client side** [V from repos/PRs]:
  - `au2001/icloud-passwords-firefox`: Apache-2.0, TypeScript, full SRP + AES-GCM client
    (`src/utils/srp.ts`, `crypto.ts`, `api.ts`, `enums.ts`); lists/fills/copies/generates;
    no TOTP, passkeys, save.
  - `TowardInfinity/icloudkeys`: Apache-2.0, TypeScript + Rust installer; SRP-6a + AES-GCM;
    Hide My Email.
  - `bendews/apw` (GPL-3.0, Deno): CLI; drives Apple's real extension in a headless browser
    through DevTools, so no protocol code of its own. `apwlib/apwcli` do the same.
  - `pauljoda/Crest` PR 716 (MPL-2.0 project, closed unmerged 2026-11-06, "held until after
    the stable release"): a browser with its own Swift client that **spawns the helper from the
    main process and passes the launch constraint through the passkey entitlement**; "never run
    against Apple's real binary", and it "relies on behaviour Apple does not document". [V] This
    is the closest prior art to the wubble plan and no one has proved it works yet.
  - `ThaRae/Xenon` PR 38: a Ladybird-based browser that implements Chrome-style native messaging
    so Apple's extension can run; "whether Apple's helper checks that its parent process is Chrome
    has not been tried by hand". [V]
- **Legal/ToS**: no Apple statement forbids a browser speaking the protocol; Apple invites
  browsers to be allow-listed. Using an undocumented protocol with Apple's helper is still
  outside any documented API. [I] Open licences above are GPL-3.0-compatible (Apache-2.0, MPL-2.0). [I]
  Copied or ported code needs a source comment and a `THIRD_PARTY_NOTICES.md` entry (project rule).

### 1.7 What the helper does *not* give

- Passkeys (au2001: not implemented; passkeys go through §3). [V]
- Anything for Windows beyond Apple's own extension flow. [?]

---

## 2. How Orion most likely does it (evidence)

Facts from the installed Orion 1.1.4 (build 153) on the Mac and from Orion's feedback site:

- WebKit (system `WebKit.framework`, ObjC SPI). Orion's strings include `WKAutoFillButtonType`,
  `_webView:didClickAutoFillButtonWithUserInfo:` (WebKit's private delegate for the key-icon
  button in a password field), `PasswordSuggestionMenuViewController`,
  `PasswordSuggestionMenuPasswordCellView`, `password_provider_options` ("Password Providers…"),
  `other_passwords` ("Other Passwords…"), `open_password_preferences`, `password_provider_label`,
  `third_party_provider`, `orion_performAutoFillAssistedRequests`, `AutoFillWebAuthn`. [V]
- Entitlements: `com.apple.developer.web-browser.public-key-credential` = true (passkeys),
  `keychain-access-groups` = `TFVG979488.com.kagi.shared`, iCloud CloudKit containers, no
  associated-domains. A Developer ID build with an `embedded.provisionprofile`. [V]
- Orion has its own Keychain (default on). Settings text in the bundle: "Passwords saved in
  Safari or 3rd-party password providers can be accessed, but not updated." [V]
- Kagi's help page: to use Safari passwords, turn off "Use Orion's Keychain", click the key icon,
  choose "Passwords", unlock with Touch ID or password, pick a login, click "Choose Password". [V]
- Orion developer dino (orionfeedback 6932 post 7): "there's no accessible API available to use
  these 3rd party password providers. This popup … only appears for native NSTextField controls
  (without autofill option), as this is completely handled by macOS, so Orion used a tricky
  solution to at-least have this as an option for login forms." [V] Vlad (post 3): "The menu you
  see is generated by macOS and we can only render it allow the user to click." [V]
- Users' flows (8590 posts 14, 33, 36): key icon, "Password Providers…", "Passwords", then the
  whole list; no site context; 2FA code not filled; sometimes "nothing shows up". [V]

**Inference** [I]: Orion's two "passwords" entries are (a) its own suggestion menu for Orion's
Keychain (the part that "does nothing" when that keychain is off or empty) and (b) the path that
hands focus to a native text field so macOS shows the system provider panel (Passwords app, 1Password,
KeePassium, Strongbox). The panel is opened with no `ASCredentialServiceIdentifier`, which is why
the site is not detected: a third-party developer pointed out (8590 post 32) that Orion "needs to
pass at least one serviceIdentifier". Whether a non-Safari app can pass one to the system panel is
[?]; Orion's staff said no (8590 post 12: "that API is available to Safari only"). [V claim, not verified by Apple]

Orion also lists "iCloud Passwords" (Chrome extension id `pejdijmoenmkgeppbflobdenhhabjlaj`)
among its bundled popular extensions; it runs Apple's extension through its WebExtension layer, and
was added to Apple's allow-list in 2025. [V Orion strings, 10025/11230]

So Orion has **three** things: its own keychain, the OS panel trick, and Apple's extension. The
"ux hell" is mostly from layering all three. [I]

---

## 3. Passkeys for a third-party browser on macOS

- **Entitlement**: `com.apple.developer.web-browser.public-key-credential`: "make registration and
  assertion requests for passkeys and security keys for any relying party identifier"; macOS 13.3+;
  "Only add this entitlement if your app can act as a user's web browser." [V Apple page]
- **Criteria** [V]: the app lists the `http` and `https` schemes in `Info.plist`; on launch it
  has a URL field, search tools, or curated bookmarks; opening an http(s) URL navigates there
  and renders the expected content (not redirecting elsewhere); locked-down modes allowed; may show
  Safe Browsing-like warnings; may offer a native sign-in UI.
- **How to request** [V]: Account Holder role of an organisation's developer account; form at
  `https://developer.apple.com/contact/request/macos-browsers-passkeys/`; Apple reviews; if
  approved it is a managed capability, added to the account (then enabled for an App ID and put
  into a provisioning profile). Individual accounts: [?] DTS did not answer (thread 852550); that
  thread's developer (Chromium browser, Developer ID, individual account) had a request pending
  from 2026-09-23 to at least late October, and support had said "issued" while the portal still
  showed "Submitted". [V] Error code 1004 reports after approval: [V search snippet, thread 790037/777992].
- **Relation to default browser**: separate. `com.apple.developer.web-browser` (iOS/iPadOS 14+) is
  the default-browser entitlement. DTS in 852550: the team had "Default Web Browser" but not
  "macOS Web Browser Public Key Credential". [V] On macOS, being default browser needs no
  entitlement. [I]
- **Public API** [V headers in MacOSX27.sdk, docs]:
  - `ASAuthorizationWebBrowserPublicKeyCredentialManager`: `requestAuthorizationForPublicKeyCredentials`,
    `authorizationStateForPlatformCredentials`, `platformCredentials(forRelyingParty:)`,
    `isDeviceConfiguredForPasskeys` (macOS 26.2+). macOS 13.3+. The user sees a TCC prompt;
    settings page "Passkeys Access for Web Browsers", reset with
    `tccutil reset WebBrowserPublicKeyCredential`. [V imperialviolet]
  - `ASAuthorizationWebBrowserPlatformPublicKeyCredentialProvider`, `…AssertionRequest`,
    `…RegistrationRequest`, `…SecurityKey…`: macOS 13.5+ (security key variants 14.4+).
  - `ASPublicKeyCredentialClientData(challenge:origin:)` with `topOrigin` and `crossOrigin`
    (macOS 13.5+). This is how the browser tells the system the origin. [V header]
  - `ASAuthorizationPlatformPublicKeyCredentialAssertionRequest` for the iCloud Keychain item
    (macOS 12+).
- **What Chromium and Firefox do**: both pass the full `clientDataHash` through a private
  AuthenticationServicesCore SPI `-[ASAuthorizationController _requestContextWithRequests:error:]`
  (Chromium comment: "needed to implement several behaviours that browsers require. Most
  importantly, specifying the full clientDataHash rather than the challenge"; the symbol is
  present in the installed Chrome and Zen/Firefox `XUL` on the Mac). [V] The public client-data
  object lets a browser avoid the SPI by letting the system build `clientDataJSON` from
  challenge and origin. [I]
- **Chromium's code location**: `device/fido/mac/icloud_keychain*.{h,mm}` and
  `device/fido/fido_request_handler_base.cc` (the iCloud Keychain authenticator type), driven by
  `content/browser/webauth` and a `content::AuthenticatorRequestClientDelegate`; Chrome's
  `chrome/browser/webauthn` supplies the dialog and policy. [V file reads, I for the exact split
  of dialogs] So the *authenticator* code is in `device/` and `content/` (in CEF); the *dialog host*
  is Chrome's. Chrome's entitlements on the Mac: `public-key-credential`, `applinks.read-write`,
  keychain groups `…Chrome.webauthn`, `…webauthn-uvk` (and a provisioning profile). [V]
- **CEF**: CEF issue 3505 (2023): the maintainer said the Chrome runtime "should" support WebAuthn
  if Chrome does, closed not planned. [V] Third-party projects report that in windowless
  (Alloy-style) CEF "a passkey request waits forever" / no dialog host / the iCloud Keychain path
  is skipped (mxl-browser-source PR 3, herdr-app issue 3, abromux PR 94). [V reports, secondary]
  Wubble's own code says "Windowless rendering is Alloy style" (`media.rs`) while CEF 156's
  Chrome runtime owns profiles (`storage.rs`); so wubble's pages run Alloy-style windowless. [V repo]
  CEF's own `--disable-blink-features=WebAuth` flag mentioned by herdr: [V, secondary].
- **What a CEF embedder must do** [I]: (1) get the entitlement, an App ID with the capability and
  a Developer ID provisioning profile embedded in the main app; hardened runtime + notarisation;
  (2) either test whether Chromium's request even reaches the delegate in OSR (likely not), or
  (3) inject a document-start shim replacing `navigator.credentials.create/get` (and
  `PublicKeyCredential` capability queries) for `publicKey` requests, send the request with the
  frame's origin from the browser process to a native bridge, run
  `ASAuthorizationController` with the Web Browser provider requests anchored on the wubble
  window, then hand the `PublicKeyCredential` back to the page. Origin must come from the
  browser process, not from the page. [V herdr-app rule list] The same bridge can show passkeys in
  wubble's autofill menu with `platformCredentials(forRelyingParty:)`. [I]
- **Orion, Zen, DuckDuckGo, Chrome all carry the entitlement** (read from the Mac) and each
  ships an `embedded.provisionprofile`; Zen is Developer ID signed and not App Store. [V]
- **Windows**: Chromium wraps `webauthn.dll` in `device/fido/win` (needs an HWND; loaded
  dynamically; Windows Hello and security keys). [V] Whether it works in windowless Alloy CEF:
  [?] untested; CEF issue 3275 (Windows Hello SSO): maintainer said the Chrome runtime should
  handle it and Alloy likely not. [V] Windows has no Apple-style entitlement. [I] iCloud
  Passwords on Windows works through iCloud for Windows and Apple's extension; a native client
  would need the Windows helper's path/manifest and its launch checks [?].

---

## 4. Recommendation for wubble

### 4.1 Ranking (polish, then feasibility, with prebuilt CEF)

| # | Path | Polish | Needs Apple? | Needs M6? | Notes |
|---|---|---|---|---|---|
| 1 | **Native client for the Apple helper** ("From Apple Passwords…") | High: site known, matches only, Touch ID, TOTP, save | Entitlement or allow-list entry | No | Undocumented protocol; port from au2001/icloudkeys (Apache-2.0); flag-gated |
| 2 | System AutoFill panel via native text fields (Orion) | Low-medium: whole list, no site | No | No | Works today; do not market it as integration |
| 3 | Apple's extension itself | High | Allow-list or entitlement | Yes (extension system, loading the store CRX id `pejdij…` is also hard outside the Web Store) | Defer |
| 4 | Import/export CSV from the Passwords app | Low | No | No | Already on TODO as fallback; user-driven |
| x | Keychain read, `SecRequestSharedWebCredential`, `ASAuthorizationPasswordProvider`, URL schemes | none | none | none | Dead ends (§1) |

Passkeys are a separate track (§3): entitlement plus a JS-shim bridge; not blocked by M6. [I]

### 4.2 What needs the Apple developer account

1. **Organisation account + Account Holder** to file "macOS Browsers Passkeys" (the entitlement).
   Start now: lead time unknown to 7+ weeks. [V] Check whether the owner's account is an
   organisation; an individual account's eligibility is [?].
2. After approval: enable the capability on the main app's App ID, create a **Developer ID
   provisioning profile** (managed capability), embed as `Contents/embedded.provisionprofile`,
   add the key to the main app's entitlements, sign everything with hardened runtime, **notarise**
   and staple. [V pattern read from Orion/Chrome/Zen; I for steps] The helper `wubble-helper`
   needs no entitlement for these; the launch-constraint parent is the main app. [I]
3. **Allow-list**: open an issue/PR at `apple/password-manager-resources` with the bundle id,
   signing identifier and team id (and the extension path field in the JSON). It takes an OS
   release to land, so it supplements the entitlement and covers older macOS. [V process]
4. **Pre-requisites for review**: http/https schemes in `Info.plist`, a URL field on launch,
   renders pages normally (wubble already does). [V criteria]
5. Developer Program membership is not App Review: no Mac App Store here, but **notarisation is
   required** for Gatekeeper, and it does not check API usage beyond malware scanning. [I]

### 4.3 What needs M6 (own Chromium build)

- Loading Apple's extension in wubble (extension system is off: `disable-extensions`, TODO.md).
- Chrome's own WebAuthn dialogs and conditional UI (chrome layer). [I]
- Neither is needed for paths 1 and 2 above.

### 4.4 Sketch of the UX (path 1)

- One entry point in wubble's existing key/autofill menu: **"From Apple Passwords…"** (macOS
  only, shown when the helper is present; hidden otherwise).
- Open state machine in `wubble-core` (one enum, pure `transition`, effects at the edge in
  `wubble-engine`/a new macOS module): `Unpaired -> Pairing{code} -> Ready -> Querying(url) ->
  Picked(entry) -> Filled`, plus `HelperUnavailable`, `Denied`, `SessionLost`. No scattered
  booleans (project rule).
- On click with a login form focused: send `GET_LOGIN_NAMES_FOR_URL` with the frame URL. Show
  the matching entries (usually one or two) in the same menu. If none, a **single** "Open Passwords
  app" row (`LAUNCH_PASSWORDS_APP`), no second button.
- First use per launch: wubble shows "Enter the code Apple Passwords shows" (6 digits). The helper
  draws the code and Touch ID itself. Remember nothing between launches unless Apple's session
  data (the helper's keychain item) is stable [?].
- On pick: `GET_PASSWORD_FOR_LOGIN_NAME` for that entry, then fill through wubble's existing
  autofill path (the same fill used for the vault), bound to the frame origin from the browser
  process, https only. **Never write it to wubble's vault** unless the user presses "Save to wubble"
  explicitly. Zeroise the value after fill. Offer verification codes (`GET_ONE_TIME_CODES`) when a
  code field is focused. Save prompts can go to `SET_PASSWORD_FOR_LOGIN_NAME_AND_URL` with an
  explicit choice (Apple, wubble vault).
- Passkeys: same menu shows "Passkey for this site" from
  `platformCredentials(forRelyingParty:)`, TCC prompt on first use.
- Off by default behind a setting; the helper is spawned only when the user opens the menu. Nothing
  phones home (the helper talks to Apple, not wubble).

### 4.5 Risks and cautions

- **Private APIs**: do not use `_requestContextWithRequests:error:`, `AuthenticationServicesCore`,
  or WebKit private SPI in wubble's own code. [project rule/I] The public `ASPublicKeyCredentialClientData`
  route exists. Chromium's own use (inside CEF) is Google's.
- **Protocol drift**: undocumented; Apple already changed it once (macOS 15.4 broke third-party
  browsers). Keep the client behind a trait, version-gate on `GET_CAPABILITIES`
  (`secretSessionVersion` must be 1) and fail closed. [V au2001; Crest also "fails closed"]
- **Entitlement path unverified**: no one has shown the helper working from an entitlement-only
  parent; do a spike on the owner's Mac first (needs the entitlement or an allow-listed signature;
  an unsigned dev build will be killed by the launch constraint). [I]
- **Per-launch 6-digit code** is Apple's design and shows up in every browser. [V]
- **Windows**: do not promise parity. Windows Hello via Chromium's `webauthn.dll` path is a
  spike; Apple Passwords on Windows depends on Apple's Windows helper/signature policy [?].
- **Orion-style stop-gap (path 2)** is cheap but would reproduce the exact complaint (no site,
  nothing filtered, no codes); only do it if there is a long gap before path 1.
- **Security**: an origin bug lets any page get any saved login. Fill only after an explicit pick,
  bind to the top frame https origin, no iframe fill without opt-in. [V Crest/herdr rules]

### 4.6 First spike (smallest test of the whole thing)

On the owner's Mac (macOS 27.3): a signed, notarised wubble dev build with the entitlement (or a
build signed under a listed identity is not possible) that spawns the helper with Apple's extension
origin as the argument, sends `GET_CAPABILITIES`, and prints the reply. If the helper stays alive,
the constraint is satisfied and pairing can proceed with the SRP code from au2001 (`srp.ts`). If
it is killed immediately, the entitlement clause is not enough on that OS and only the allow-list is
left. Cost: about a day once the entitlement exists. [I]

---

## 5. Sources

Apple
- Entitlement page: https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.web-browser.public-key-credential (read via the tutorials/data JSON)
- Request form: https://developer.apple.com/contact/request/macos-browsers-passkeys/
- `ASAuthorizationWebBrowserPublicKeyCredentialManager`: https://developer.apple.com/documentation/authenticationservices/asauthorizationwebbrowserpublickeycredentialmanager
- Password AutoFill: https://developer.apple.com/documentation/security/password-autofill
- Shared web credentials: https://developer.apple.com/documentation/security/shared-web-credentials
- Forum 134077 (ASAuthorizationPasswordProvider on macOS): https://developer.apple.com/forums/thread/134077
- Forum 809587 (Password AutoFill on macOS): https://developer.apple.com/forums/thread/809587
- Forum 676891 (iCloud keychain via Keychain Services, DTS): https://developer.apple.com/forums/thread/676891
- Forum 829169 and 852550 (requesting the entitlement; DTS answers): https://developer.apple.com/forums/thread/829169 , https://developer.apple.com/forums/thread/852550
- Apple Community, iCloud Passwords with third-party browsers: https://discussions.apple.com/docs/DOC-250007756
- Passwords app, get browser extension: https://support.apple.com/guide/passwords/get-extensions-mchlf7ac261e/mac
- iCloud for Windows passwords: https://support.apple.com/guide/icloud-windows/icw2babf5e03
- `apple/password-manager-resources` README and `quirks/web-browser-extension-distribution-information.json`: https://github.com/apple/password-manager-resources
- Local reads: `MacOSX27.sdk/…/AuthenticationServices.framework/Headers`, `Security.framework/Headers/SecSharedCredential.h`

Orion / Kagi
- Help, password management: https://help.kagi.com/orion/features/password-management.html
- Feedback threads (Flarum API used to read them): 6932 https://orionfeedback.org/d/6932-suggest-a-password-from-a-3rd-party-password-managers-automatically ; 8590 https://orionfeedback.org/d/8590-passwords-app-integration ; 8175 https://orionfeedback.org/d/8175-apple-passwords-integration-safari-like ; 8089 https://orionfeedback.org/d/8089-icloud-keychain-native-autofill ; 10025 https://orionfeedback.org/d/10025-icloud-passwords-extension-not-working-on-macos-sequoia-154-beta ; 11230 https://orionfeedback.org/d/11230-apples-password-extension-in-broken-for-me-on-orion-rc ; 3025 https://orionfeedback.org/d/3025-support-passkeys ; 2053 https://orionfeedback.org/d/2053-use-icloud-keychain-via-chrome-extension
- Local: `/Applications/Orion.app` entitlements, `strings`, `en.lproj/Localizable.strings`

Chromium / CEF / Mozilla
- `device/fido/mac/icloud_keychain.{h,mm}`, `icloud_keychain_internals.h`, `fido_request_handler_base.cc` (googlesource main)
- Chrome blog, passkeys on iCloud Keychain: https://developer.chrome.com/blog/passkeys-on-icloud-keychain ; Adam Langley: https://imperialviolet.org/2023/10/18/icloudkeychain.html
- CEF issues 3505 https://github.com/chromiumembedded/cef/issues/3505 and 3275 https://bitbucket.org/chromiumembedded/cef/issues/3275/sso-authentication-issue-when-using
- Windowless/passkey reports: https://github.com/LeeO86/mxl-browser-source/pull/3 , https://github.com/cameronsjo/herdr-app/issues/3 , https://github.com/ZilvinasAbr/abromux/pull/94
- Mozilla bugs 1853230, 1865379, 1650212, 1856067: https://bugzilla.mozilla.org/show_bug.cgi?id=1853230
- Zen issue https://github.com/zen-browser/desktop/issues/7210 ; ungoogled-chromium https://github.com/ungoogled-software/ungoogled-chromium-macos/issues/144

Open implementations of the helper protocol
- https://github.com/au2001/icloud-passwords-firefox (Apache-2.0)
- https://github.com/TowardInfinity/icloudkeys (Apache-2.0)
- https://github.com/bendews/apw (GPL-3.0; drives the real extension)
- https://github.com/pauljoda/Crest/pull/716 (closed, MPL-2.0 project)
- https://github.com/ThaRae/Xenon/pull/38

Local verification commands (read-only, on the Mac)
- `codesign -d --entitlements - /Applications/{Orion,Google Chrome,Zen Browser,DuckDuckGo,Safari}.app`
- `codesign -d -vvvv /System/Cryptexes/App/System/Library/CoreServices/PasswordManagerBrowserExtensionHelper.app`
- `strings` on the helper, Orion, Chrome Framework, Zen `XUL`; native messaging manifests under `/Library/…/NativeMessagingHosts`
