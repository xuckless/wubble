# Milestones M0 to M7

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.
The open lines left in each milestone; the build-out through M5, extensions and the MCP server is done.

## M0: Frame path
- [ ] Spring spike: a per-frame spring (k 300 / c 15) with no missed frames while a page scrolls
- [ ] Transparent window with compositor blur per platform (macOS vibrancy, KDE blur; GNOME stays opaque)
- [ ] Re-measure `--begin-frames` on a 120 Hz (and the 300 Hz) display on Linux: it was left off there in b2 for a worse video render interval at 240 Hz
- [ ] Mac console hand checks: ProMotion, window, Cmd keys, IME, popups, Keychain, file chooser, media keys, fullscreen
- [ ] macOS: CEF does not finish shutting down after a windowless run over ssh; find the commit and the cause

## M1: Engine layer
- [ ] Hand checks: dead keys, autoscroll cursor feel, IME with ibus, popups, `window.open` and its badge, a download, fullscreen
- [ ] Per platform: view lifecycle on the Mac and Windows; GPU process watch on macOS and Windows; macOS and Windows popup frame
- [ ] Input gaps: kinetic wheel phase on Linux (host-side fling), touch input, the AltGr flag, CSS `url()` and zoom cursors
- [ ] IME gaps: no caret rect before the first composed character; no underline styles or replacement range
- [ ] Drag: custom MIME types to other apps
- [ ] macOS drags (`wl/macos.rs`) run on a Mac: written and compile-checked from Linux only. Out: session starts from winit's `mouseDragged:` (the current event), image placement and 2x sharpness, Finder takes the promised picture, Safari or Notes the link with its title; in: the `draggingEntered:` family added to winit's view (file drops now come through it, not winit's `DroppedFile`), the page's answer reaching `draggingUpdated:`
- [ ] Drags out of the window and in from other apps checked by hand on a desktop (the smoke cannot move the compositor's pointer: `--smoke dnd` covers drags inside the window, the Wayland offer is unit-tested)
- [ ] A bookmark dropped into a folder row lands at its end; Chrome's drop line between rows (a position) waits for drag reorder of bookmarks
- [ ] Page context menu: the Picture in Picture row; media rows act through a page script (no `MediaPlayerAction`)
- [ ] Text selection handles (touch) and a match-case toggle in the find bar
- [ ] Media session (metadata, action handlers, OS controls); a real audible signal (CEF patch, M6)
- [ ] Web content as a real accessibility subtree (waits for a Slint graft API or a winit backend patch)

## M2: Shell
- [ ] Film rim on the remaining controls (buttons, menu rows)
- [ ] Touch-kit pieces not yet ported: pin, download button, dock with gliding tooltip, the check box's mask grow
- [ ] Bookmarks: drag reorder, a bookmarks bar

## M3: Memory and sleeping
- [ ] Benchmark against Chrome and Zen: 200 tabs and after 1 h idle (Chromium and Firefox at 1/10/50 done)

## M4: DevTools and built-ins
- [ ] DevTools bottom dock (no design source; needs vertical pane rects)
- [ ] Inspect-at-point in the docked panel (no protocol path; the detached window today)
- [ ] Zen/Arc password import (CSV only)

## M5: Arc-style features
- [ ] Sync (deferred)

## Built in instead of extensions
- [ ] Content blocker: uBO scriptlet coverage
- [ ] Content blocker start-up race: with no compiled-engine cache (first run, list change), requests pass until the lists compile (seconds); hold the first main-frame navigation at the host until the blocker reports ready (a start-up state, bounded by an event), since CEF's IO thread must never wait

## M6: Own engine (ungoogled Chromium + CEF)
- [ ] Releasing extensions (releases ship without them: on an engine without the patch queue wubble offers none): the patch queue in the release engine on Linux and macOS (Windows parked), then the extension gaps below (Bitwarden's popup width first)
- [ ] macOS arm64 codec build (same recipe, `--arm64-build`)
- [ ] Reproducible recipe: Chromium at the CEF branch + ungoogled patches + CEF patches, kept as a patch queue
- [ ] Build with `enable_widevine` and codecs; ship no CDM; a CDM loader (installed CDM, else opt-in download)
- [ ] Document DRM limits (Linux L3 around 720p; services that need a VMP-signed browser refuse us)
- [ ] Build infrastructure: Linux and macOS local; Windows on a runner large enough for Chromium
- [ ] Extensions: P4 (permission prompts answered in wubble's UI); install from the Web Store; the install page's "restart to apply"
- [ ] Extensions shell: shrink-to-fit popup sizing (today the page's body decides, a body without a width fills 800), a tooltip on the footer button (needs an `Action`), the popup's own pointer origin for `<select>` lists, `chrome.action` popups of the global profile, screenshots by the owner's eye
- [ ] Extensions (acceptance gap): `__MSG_name__` names show raw in the install prompt, the list and the popover; resolve them from `_locales` ([docs/research/extensions.md](../research/extensions.md) "Acceptance", gap 1)
- [ ] Extensions (acceptance gap): `chrome.tabs.getCurrent()` in an action popup returns a tab (Chrome: undefined), so Bitwarden's popup lays out 57 px wide; patch `TabsGetCurrentFunction` and `getViews` for views that are not wubble tabs (gap 2)
- [ ] Extensions (acceptance gap): `chrome.permissions.request` rejects with "Could not find an active window." (P4: answer it in wubble's UI), which Bitwarden's first-run "default password manager" step calls (gap 3)
- [ ] Extensions (acceptance gap): Bitwarden's inline menu and autofill icon need an account: hand check with a test vault on a loopback login form (gap 4)
- [ ] Extensions: `tabs.onReplaced` when a sleeping tab wakes, `onMoved`/`onAttached`/`onDetached`, window bounds; `windows.create` is refused for now
- [ ] Swap in behind `wubble-engine`, shell unchanged
- [ ] CEF OSR patch: hide a view's compositor on `Hide()` (frees each hidden tab's backbuffer)
- [ ] Skia cache hygiene in Slint's renderer (Chromium's cache controller: a limit, deferred cleanup, free when idle)
- [~] HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch

## M7: Ship
- [ ] macOS: the Mac archive was never in the update manifest before preview.2, so a Mac on preview.1 needs one install by hand; video playback (H.264, AAC) on the Mac build is not checked (it is built on the owner's codec CEF, `/Users/ali/cef-codecs`)
- [ ] Before every release: run `cargo test --workspace` on the Mac (it passes there in CI only; preview.2 shipped with MCP hosting broken on macOS; one test there would have caught it), and run the whole suite there once now (never done)
- [ ] Release gaps: Ubuntu's user-namespace limit vs the no-sudo sandbox; rollback misses a build that cannot load;
      shellcheck not run on `install.sh`
- [ ] Import app wiring: pick a profile, apply, replace the live model, show the summary
- [ ] Import: Zen legacy tables, Arc history, Little Arc, Helium/Chromium; verify Zen macOS and Arc Windows paths
- [ ] Security update cadence: track Chromium stable
- [ ] Packaging: Flatpak/RPM, MSI/winget, signed and notarised DMG; auto-update
- [ ] Opt-in crash reports that open a GitHub issue (scrubbed)
- [ ] Default-browser registration, URL handlers
- [ ] Hardening: vault unlock off the store thread
