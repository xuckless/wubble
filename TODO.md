# wubble roadmap

A Chromium-based browser with a Slint shell: Chromium renders off-screen, Slint composites it as GPU textures.
Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md) ·
fidelity: [docs/fidelity/](docs/fidelity/kit-plan.md) · research: [docs/research/](docs/research/) · commands: [docs/smokes.md](docs/smokes.md)
Legend: `[ ]` open · `[~]` in progress · `[x]` done, with the commit that brought it to `main` (`git show <hash>`).

## Status (2026-11-20)
`main` builds, lints and passes 1922 tests; every window smoke passes on the invisible test display
(`scripts/headless.sh`), and an idle window draws nothing. M0 to M5 and the built-ins work on Linux. The
design-fidelity phase ended with the owner's sign-off on 2026-11-20: every surface is on kit components and the kit
widget work is done; what reviews raise is fixed as it comes up ("Review findings"). The auto-updater and the release tooling are
in; the first Linux preview release (`0.1.0-preview.1`, with codecs) follows the last two fixes on `qa4/tabs`.
macOS is driven only from the owner's own session and has not been run since batch eight; Windows is not green in CI.

## Now (2026-11-20): extensions, password managers, the MCP server
- [ ] Extensions on our own Chromium build (M6), far enough that third-party password managers (Bitwarden, 1Password) work as extensions
- [ ] The MCP server, basic version (M7, [docs/research/mcp-server.md](docs/research/mcp-server.md) section 0)
- Later, in its own build: a passthrough to OS password managers and passkey keychains (fingerprint, passkeys where the laptop has them)
- Then the "Optimization roadmap". The release follow-ups in M7 wait for the next version.

## Review findings (perpetual)
The design-fidelity phase is over; this list never closes and gates nothing: findings from reviews, QA rounds and the
owner's eye land here and are fixed as they come up.
Done on 2026-11-19 and 2026-11-20, after the kit merges listed under "Design fidelity":
- [x] New-tab page: capsule at 60%, bubble higher, "Let's Wubble" (4c6633a); docs tidy: lean TODO/AGENTS, `docs/` (fad1969)
- [x] Page copy buttons: `navigator.clipboard.writeText`/`write` reach the system clipboard, verified on the desktop (4992603)
- [x] QA: one overlay stack: one modal at a time, Escape closes the topmost, tab-scoped overlays, SponsorBlock banner (4f9fbe0)
- [x] QA: Shift on symbol chords (Unsplit), actions outrank web search, compact mode, "+ New tab" is Ctrl+T (db07570)
- [x] QA: glass over a page uses the design's CSS tiers plus one luminance ceiling; no seam across the sidebar edge (8326b57)
- [x] QA: tab-switch fade on the wall clock (no page bleed), Reader keeps the scroll position, Settings first wheel (1cd0b67)
- [x] QA: address pill elision, menus fit their rows, sidebar collapse clip, task manager columns, "Start" tab title,
      sidebar empty-area menu, start-page hover card (b2b7f68)
- [x] Invisible test display with focus, clipboard and GPU; save prompt lost when a sign-in beats page load (6d70c9e)
- [x] Paste (Ctrl+V) in shell fields: one clipboard path through the host's Wayland worker (6885c31)
- [x] Menu glass lost while Settings' closed Selects shared its key: the glass list is rebuilt each frame (3a9ac6e)
- [x] Sidebar footer: current Space + "^" Spaces popup with a pinned New Space, drag a tab onto it; DevTools button
      removed; Settings links the built-in pages in its nav and its search (2c3a366)
- [x] A press that closes a popup is consumed (Spaces popup reopened on its "v"); a closed tab returns to its opener by
      Chromium's rules (8523faf); `--smoke video` waits for the strip and needs Slint debug info in dist builds (b3c12e1)

Open, for the owner's eye:
- [ ] Glass over white: faint ink (disabled rows, shortcuts, counts) sits near 2:1; glass reads flat grey
- [ ] Menus grow from the design's 236 to 320 for long rows (derived; on the review sheet)
- [ ] Sidebar mid-collapse shows the rail look below 140 wide (the design's own behaviour; QA read it as a reflow)
- [ ] Rail close badge size; video strip: should a collapse carry over to later pages
- [ ] Find bar closes on reload (Chromium keeps it); Escape does not dismiss the SponsorBlock question
- [ ] Compact mode duplicates the hide-sidebar toggle (or make it sidebar <-> rail)
- [ ] Archive's icon: the stacked trays, shared with the Spaces section (the design's set has no archive glyph);
      Passwords/Sites share the lock, Updates/Downloads the arrow
- [ ] Spaces popup: its alignment (Space tile's left edge), no rename/delete on a row
- [ ] Video strip button reads "Save video"; a second click mid-download re-enters the downloader
- [ ] The derived-pieces review sheet ([docs/fidelity/derived-review.md](docs/fidelity/derived-review.md))

Open, to build or check:
- [ ] QA retry by hand: Alt+1/2 Spaces, Shift+click peek, audible badge on a playing YouTube tab (tests pass; not reproduced)
- [ ] `--smoke parity` flakes about 1 in 6 on the test display: early key presses arrive without their default action
- [ ] In-place rename for a new Space in the sidebar (needs a design); Linux alternates for the chords GNOME takes
- [ ] Audible badge: the probe misses iframes, shadow DOM and WebAudio (a real signal needs the CEF patch, M6)
- [ ] Page copy buttons carry text only (no image items)
- [ ] Start page tiles are stand-ins: the kit tile look and its hover spring
- [ ] Settings against the web: Reset per section; rows folding while searching
- [ ] List editor: the selected row stays highlighted while a draft is open
- [ ] Downloads page has no search (the host has no filter path)
- [ ] Gallery: scenes for the hover preview card and the sidebar's empty-area menu; the page menu's position varies by page
- [x] Extensions button in the sidebar footer (the free place before Downloads): the puzzle button, shown once an extension is installed, opens the popover of enabled extensions' actions
- [ ] Requests to Google on a fresh profile with or without extensions: component updater, network time, a component diff, the spellcheck dictionary, `www.google.com/async/folae` (net log in [docs/research/extensions.md](docs/research/extensions.md) section 3); "nothing phones home"
- [ ] Shift+Insert paste in shell fields (Slint has no such shortcut); tests for the list editor draft and middle-click
- [ ] `--smoke overlay=dark` fails a hue check under the command bar (also before the glass fix)
- [ ] `controls::bump` reads then writes a property inside the draw (one extra frame; the pattern that looped in glass)

## M0: Frame path
- [x] Workspace, CEF init and pump, helper, Linux dmabuf import, blit into `slint::Image` (c6b168a)
- [x] Frameless window, NVIDIA dmabuf specifics (e6f826a); resize without waiting for CEF, first measurements (a5b2321)
- [x] Skia renderer, design scene as the glass, frame pacing, macOS IOSurface port, CI on Linux and macOS (7bdc46c)
- [x] welding forked into `vendor/welding` (f83b314); CI Linux green on warm runs (a70f416); Skia and glass cost (cbef727)
- [ ] Spring spike: a per-frame spring (k 300 / c 15) with no missed frames while a page scrolls
- [ ] Transparent window with compositor blur per platform (macOS vibrancy, KDE blur; GNOME stays opaque)
- [ ] Re-measure `--begin-frames` on a 120 Hz display; on the Mac `--begin-frames`, `video`, `typing`, `resize`
- [ ] Mac console hand checks: ProMotion, window, Cmd keys, IME, popups, Keychain, file chooser, media keys, fullscreen
- [ ] macOS: CEF does not finish shutting down after a windowless run over ssh; find the commit and the cause
- [ ] Windows CI green (then drop the job's `continue-on-error`); the D3D11 shared-handle port after

## M1: Engine layer
- [x] View lifecycle, input, key routing, cursors, IME, popups, multi-window, clipboard, fullscreen, find, a11y (7bdc46c)
- [x] Dialogs, prompts and choosers (9236462); external links, Save As, status bubble, crash recovery (f742613)
- [x] Gestures, lock keys, Wayland drops and drag-out, custom MIME in the window, print sheet, Pip window (a70f416)
- [x] Picture drag-out on Wayland (90727ed)
- [ ] Hand checks: dead keys, autoscroll cursor feel, IME with ibus, popups, `window.open` and its badge, a download, fullscreen
- [ ] Per platform: view lifecycle on the Mac and Windows; GPU process watch on macOS and Windows; macOS and Windows popup frame
- [ ] Input gaps: kinetic wheel phase on Linux (host-side fling), touch input, the AltGr flag, CSS `url()` and zoom cursors
- [ ] IME gaps: no caret rect before the first composed character; no underline styles or replacement range
- [ ] Real CEF popup browsers for `window.opener` / `window.close()` (popup sign-ins need it)
- [ ] Clipboard and drag: non-text page clipboard formats; custom MIME to other apps; picture drag-out on macOS and Windows
- [ ] Page context menu: the Picture in Picture row; media rows act through a page script (no `MediaPlayerAction`)
- [ ] Text selection handles (touch) and a match-case toggle in the find bar
- [ ] Media session (metadata, action handlers, OS controls); a real audible signal (CEF patch, M6)
- [ ] Web content as a real accessibility subtree (waits for a Slint graft API or a winit backend patch)
- [ ] Windows sandbox: restructure as a client DLL behind CEF's `bootstrap.exe`

## M2: Shell
- [x] Tokens, fonts, glass tiers, scene in the host, motion library, model, tabs, keys, omnibox, store, session (e6f826a)
- [x] Scene run state, suggestion fetch (d37af49); tab drag and windows (73e29a6); Settings page (e2cbfc8)
- [x] Touch sizing, reduced motion, exits, glance, platform icons (1ef78a1); command bar over the frosted page (2d27bea)
- [x] Spaces: switcher, tint, containers, swipe, New Space from "+", the action and Settings (60c86ca)
- [ ] Film rim on the remaining controls (buttons, menu rows)
- [ ] Touch-kit pieces not yet ported: pin, download button, dock with gliding tooltip, the check box's mask grow
- [ ] Gallery sweep: the autofill key chip overlaps the Space header
- [ ] Bookmarks: drag reorder, a bookmarks bar, bookmarks as command-bar candidates

## M3: Memory and sleeping
- [x] Sleep policy, lifecycle, freeze, discard, snapshots, busy tabs, pressure, task manager, process knobs (3fa5dc6)
- [ ] Benchmark against Chrome and Zen: 200 tabs and after 1 h idle (Chromium and Firefox at 1/10/50 done)

## M4: DevTools and built-ins
- [x] DevTools docked and detached, remote debugging, view-source (3fa5dc6); blocker (f742613); reader, PDF (32092ba)
- [x] Password manager: vault (e6f826a), store (609d428), autofill and save prompt (18cc2f6)
- [ ] DevTools bottom dock (no design source; needs vertical pane rects)
- [ ] Inspect-at-point in the docked panel (no protocol path; the detached window today)
- [ ] Zen/Arc password import (CSV only) and a notice when the keyring is unavailable

## M5: Arc-style features
- [x] Archive, routing rules, Little window, hover previews (73e29a6); Boosts (32092ba); Pip on tab switch (a70f416)
- [ ] Sync (deferred)

## Built in instead of extensions
- [x] Media downloader, uBO lists (59c2f8b); dark mode, reader, video controls, screenshots, hints, SponsorBlock (32092ba)
- [x] Cookie banners and tracking parameters (f742613)
- [ ] Content blocker: uBO scriptlet coverage

## Design fidelity
- [x] Phase 0: harness, generator, kit lock (cd40516); glass, backdrop, page frost, motion, icons (2d27bea)
- [x] Start page logic (7ef1371) and on screen (b2b3f36); controls kit C (28cd99e), A and B (9f19662)
- [x] Chrome first half (78756cb), second half (69efb8c); Settings and overlays A (79ae01e)
- [x] Start-page field, rail close, new Space (60c86ca)
- [x] List pages, overlays B, video strip (607ec72); list editor, Spaces in Settings, status bubble (3ee2f9d)
- [x] New-tab page: capsule at 60% of the page, bubble higher, heading "Let's Wubble" (4c6633a)
- [x] Scoring tool removed: review by screenshots and the owner's eye (187d7a4)
- [x] Phase ended: the owner's sign-off on the kit widget work (2026-11-20)
- [ ] Empty the kit lock baseline (107 rows, mostly `kit/derived.slint` literals); alias the value-equal `Shell` literals
- [ ] Voice glow shader into the kit (no microphone)

## Performance pass
- [x] Start-up, page load, input, release profile (cbef727); memory (a01325c); Linux video upload path (5e80397)
- [ ] macOS memory: a Wikipedia article in wubble and in Chrome alone, at the console
- [ ] Browser GPU memory: the unattributed +20 MiB from 1 to 10 tabs; re-measure on macOS
- [ ] glibc heap after a mass discard (28 MiB): decide with hardening whether to `malloc_trim`
- [ ] Video under load: the real browser in its scope, hardware decode under load, the decoder CEF picks on macOS
- [ ] macOS idle with the still backdrop, video figures, occluded pausing by hand
- [ ] CPU and GPU during video on Linux

## Feel pass (before release)
- [ ] Stretch and squash on every pressable and moving surface (menus, popovers, sheets, toasts, command bar, strip)
- [ ] Liquid glass under every floating surface, with the flat fallback kept
- [ ] The glowing ring and border beam wherever something is focused, loading or newly opened
- [ ] One audit: walk every gallery surface against the three effects

## Shelved ideas
- [ ] Translate pages on-device ([docs/research/translate.md](docs/research/translate.md))
- [ ] Live text in images, on-device OCR ([docs/research/ocr.md](docs/research/ocr.md))
- [ ] OS password managers and passkey keychains, as a passthrough in a dedicated build after extensions ([docs/research/apple-passwords.md](docs/research/apple-passwords.md))

## M6: Own engine (ungoogled Chromium + CEF)
- [x] CEF with proprietary codecs on Linux x64, asserted codec rows (84e5c62)
- [ ] macOS arm64 codec build (same recipe, `--arm64-build`)
- [ ] Reproducible recipe: Chromium at the CEF branch + ungoogled patches + CEF patches, kept as a patch queue
- [ ] Build with `enable_widevine` and codecs; ship no CDM; a CDM loader (installed CDM, else opt-in download)
- [ ] Document DRM limits (Linux L3 around 720p; services that need a VMP-signed browser refuse us)
- [ ] Build infrastructure: Linux and macOS local; Windows on a runner large enough for Chromium
- [x] Extensions research ([docs/research/extensions.md](docs/research/extensions.md)); the extension system on only when one is installed; install from a directory or a .crx (no UI yet; `--smoke extensions`) (669c29b)
- [ ] Extensions: patches P1 (tabs.query, tab events, windows), P2 (extension-opened tabs and windows), P3 (action button and popup), P4 (permission prompts); then the install page with permissions and "restart to apply"
- [ ] Extensions: `chrome.tabs`/`windows`/actions on wubble's model; action buttons, popups, options, install, Web Store
- [x] Extensions: P1 to P3 as the patch queue `patches/chromium` (`scripts/build-cef.sh patch`, `cef_wubble_ext_v1`): `chrome.tabs`, `chrome.windows`, `chrome.action` (engine API) and extension-opened tabs on wubble's model; `--smoke extensions` checks them on a patched CEF, Linux (21af397)
- [x] Extensions review fixes: opened tabs in the asking container and the named window's Space, the global profile's opens refused, `tabs.create({})` opens the start page, onActivated for a tab created active, `--smoke extensions` through the host's own request code
- [x] Extensions shell UI (derived from the kit; the design repo has none, so the pieces are on [derived-review.md](docs/fidelity/derived-review.md) "Extensions popover"): Settings section (list, switch, remove, Options, install from file or folder, restart note), the install prompt with permissions through the overlay stack, the footer popover with action icon and badge, the action popup as a windowless view in a card (25x25 to 800x600, Escape or an outside press closes, keys go to it), options page as a tab, gallery scenes; popup and action rows are untested live (need the patched CEF)
- [ ] Extensions shell: shrink-to-fit popup sizing (today the page's body decides, a body without a width fills 800), a tooltip on the footer button (needs an `Action`), the popup's own pointer origin for `<select>` lists, `chrome.action` popups of the global profile, screenshots by the owner's eye
- [ ] Extensions (acceptance gap): `__MSG_name__` names show raw in the install prompt, the list and the popover; resolve them from `_locales` ([docs/research/extensions.md](docs/research/extensions.md) "Acceptance", gap 1)
- [ ] Extensions (acceptance gap): `chrome.tabs.getCurrent()` in an action popup returns a tab (Chrome: undefined), so Bitwarden's popup lays out 57 px wide; patch `TabsGetCurrentFunction` and `getViews` for views that are not wubble tabs (gap 2)
- [ ] Extensions (acceptance gap): `chrome.permissions.request` rejects with "Could not find an active window." (P4: answer it in wubble's UI), which Bitwarden's first-run "default password manager" step calls (gap 3)
- [ ] Extensions (acceptance gap): Bitwarden's inline menu and autofill icon need an account: hand check with a test vault on a loopback login form (gap 4)
- [ ] Extensions: the patch queue on macOS and Windows builds, and in the release's CEF (owner go)
- [ ] Extensions: `tabs.onReplaced` when a sleeping tab wakes, `onMoved`/`onAttached`/`onDetached`, window bounds; `windows.create` is refused for now
- [ ] Swap in behind `wubble-engine`, shell unchanged
- [ ] CEF OSR patch: hide a view's compositor on `Hide()` (frees each hidden tab's backbuffer)
- [ ] Skia cache hygiene in Slint's renderer (Chromium's cache controller: a limit, deferred cleanup, free when idle)
- [~] HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch

## M7: Ship
- [x] Auto-updater (signed manifest, check on open, apply on next launch, rollback), `install.sh`, packaging scripts,
      signing workflow, `docs/release.md` (4ccd853); inert until the public key is set
- [x] First preview release, Linux: `v0.1.0-preview.1`, built, packed and signed locally, on GitHub Releases with
      `install.sh`; README install section
- [ ] At the next version: `docs/release.md` still describes signing in CI: rewrite its steps for local signing (as done for preview.1)
- [ ] At the next version: confirm by hand that an installed build's update check reads the live manifest (Settings → Updates → Check now)
- [ ] macOS release, on the owner's Mac after the Linux one: `package-macos.sh` (never run), a macOS codec CEF, the
      usage strings added in 4fae73d (never compiled), then `release-sign.yml` again
- [ ] At the next version: release gaps: Ubuntu's user-namespace limit vs the no-sudo sandbox; rollback misses a build that cannot load;
      shellcheck not run on `install.sh`; `install.sh` checks only the sha256 when `minisign` is not installed
- [x] Video strip save button: enters the media downloader, absent on DRM, progress and saved states (4b4c9f1)
- [ ] MCP server, after the release: the basic version first (plan section 0), guardrails after: [docs/research/mcp-server.md](docs/research/mcp-server.md)
- [x] Import readers (f83b314) and store-side apply (e6f826a)
- [ ] Import app wiring: pick a profile, apply, replace the live model, show the summary
- [ ] Import: Zen legacy tables, Arc history, Little Arc, Helium/Chromium; verify Zen macOS and Arc Windows paths
- [ ] Security update cadence: track Chromium stable
- [ ] Packaging: Flatpak/RPM, MSI/winget, signed and notarised DMG; auto-update
- [ ] Opt-in crash reports that open a GitHub issue (scrubbed)
- [ ] Default-browser registration, URL handlers
- [ ] Hardening: vault unlock off the store thread; clear the clipboard after a password copy

## Optimization roadmap (after extensions, password managers and the MCP server)
Numbers behind it: [docs/measurements.md](docs/measurements.md) "Browser benchmarks" and "Main-process flame graph" (2026-11-20). Target: the shell's cost follows what is visible, not "match Chrome". In order:
- [x] Profile the main process during video: 76 % of the main thread's user CPU is Slint redrawing the whole window per page frame; the scene is 1.3 %, the frame import 7 % (flame graph, 2026-11-20)
- [ ] Stop redrawing the shell per page frame: a page frame redraws only its pane; the sidebar and bars are not re-walked or re-recorded (partial rendering in Slint's Skia renderer on wgpu, or the shell cached as a layer)
- [ ] Partial rendering inside the page: CEF's damage rects through the frame path, so only the video or the moving part redraws
- [ ] Content-driven frame rate: at most one page frame per refresh; CEF's rate follows the content (still: none, video: its own rate, input and motion: vsync); one pure state machine in `wubble_gfx::pacing`
- [ ] Profile again with kernel time (needs root for `perf`): GPU driver calls and present, about a third of the main process's CPU, unseen so far
- [ ] The CEF message pump on the main thread (14 % during video): what runs there per frame
- [ ] Find the Speedometer gap (16 % behind Chromium): run it in plain CEF with our `libcef.so`, no shell
- [ ] CEF build with PGO and ThinLTO (`scripts/build-cef.sh` sets `chrome_pgo_phase=0 use_thin_lto=false`), if the plain-CEF run is also behind
- [ ] Memory breakdown per allocation in the main and GPU processes, before any memory change
- [ ] Hidden tabs free their frame targets; the blur pyramid exists only while glass is open; revisit Skia's 256 MiB layer budget
- [ ] A page frame touches the scene and the blur pyramid only while overlay glass or motion is on screen (1.3 % today: low value)
- [ ] Re-run the benchmarks, the flame graph and the video CPU/RAM profile against Chromium and Firefox after each step; keep the harness and the profile scripts in `scripts/`
- [ ] Only if plain video still costs far more than Chromium after the above: the page on a native OS surface with faked glass (overturns four frame-path decisions; owner decides)

## Open decisions (owner)
- Hardware video decode: keep the opt-in row (unsandboxed GPU process) or remove it
- Start page liquid: stirring per second or per frame
- Ship codecs by default or opt-in (AVC/AAC patent exposure)
- Delete the 38 GB Chromium checkout used for the codec build
- Signed and notarised macOS bundle (needs the owner's signing details)
- Apple Passwords / passkeys entitlement request (the owner files it)
- Video strip: does a collapse carry over to later pages; Pip: a cropped video or the page's own element fullscreen
- Sidebar day labels ("Today": the design draws none); whether `cpu_fallback > 0` fails `--smoke frames`
- If Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it

## Risks
1. Slint's wgpu texture import is an `unstable-` feature and can change on any minor: pin versions.
2. Own glass and backdrop must fit the 8.33 ms frame beside Chromium's GPU work; the quality ladder is the fallback.
3. Prebuilt CEF is not ungoogled and has no proprietary codecs until M6.
4. `welding` pins CEF; every CEF upgrade is ours to do in the fork.
5. Extensions under windowless CEF are partial: a real Chromium patching project in M6.
6. OSR costs one extra composite per frame and bypasses direct scanout for video.
7. An ungoogled + CEF patch queue rebased every Chromium milestone is permanent work for one person.
8. Windows is CI-only: input, IME and windowing bugs there are found late.
9. Turso is pre-release: keep the file format SQLite-compatible and back up before migrations.
