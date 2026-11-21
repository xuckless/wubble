# wubble roadmap

A Chromium-based browser with a Slint shell: Chromium renders off-screen, Slint composites it as GPU textures.
Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md) ·
fidelity: [docs/fidelity/](docs/fidelity/kit-plan.md) · research: [docs/research/](docs/research/) · commands: [docs/smokes.md](docs/smokes.md)
Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

## Status (2026-11-20)
`main` builds, lints and passes 2117 tests; every window smoke passes on the invisible test display
(`scripts/headless.sh`), and an idle window draws nothing. Released: `0.1.0-preview.2` for Linux (with codecs), which
adds the MCP server (off by default: live hosting from Settings, 20 tools, sessions held to their sites, agent apps
registered on turn-on); its macOS build is made by the owner on the Mac from the same tag
(`sh /Users/Shared/wubble-preview2-mac.sh`), then signed, added to the manifest and uploaded from here. Extensions are
built but not released: they need the patched engine, which the release engine is not. The npm launcher
(`packages/wubble-mcp`) is built, not published. Windows is not green in CI.

## Now: the optimization pass
The build-out through M5, extensions and the MCP server is done; the next work is the "Optimization roadmap" below,
in its order. First step: stop redrawing the shell per page frame (the flame graph's 76 % of the main thread).
How to start it: measure first with the kept harness (`~/Dev/wubble-wt/notes/bench-2026-11-20/`: `runner.py`,
`profile-lbr.sh`, `fold.py`; move them into `scripts/` as the first commit) so every step has a before and an after
on the same build; the numbers to beat are in [docs/measurements.md](docs/measurements.md) (2026-11-20). Measured by
hand only: nothing in `cargo test` or CI asserts a timing.

Beside it, small and unordered:
- [ ] Finish preview.2: the Mac files into the release and the manifest (then the README's install line says both platforms)
- [ ] Wire the thinking orb (in the kit, gallery only so far): the agent mark while a call is in flight, the activity page header, the approval sheet; after the owner reads the proposal in `docs/research/agent-widgets.md` section 5
- [ ] The mascots are researched, not ported: their fabric and plastic shading is CPU-baked with per-frame readbacks in the web version, so a faithful port needs a GPU rewrite (`docs/research/agent-widgets.md` section 3.3); owner decides whether to do it
- [ ] Thinking orb: the `gravity` cursor warp is not ported; one moving 64 px orb costs a whole-window redraw per frame (about 8 to 9 % of a core at 60 Hz, hand run): tied to "Stop redrawing the shell" in the optimization roadmap
- [ ] The agent glyph: the design's `sparkle` stands in (one alias, `Glyph.agent`) and reads as a crosshair at badge size; the owner's glyph, a mascot or the orb, then one swap
- [ ] Settings section list has no keyboard handling (its rows are pointer and assistive-tech targets, not focusable): Tab or arrows into the list, selection moving with Down/Up; `NavScroller.reveal` is ready for it
- [ ] Gallery: a Settings section picked far from where the page is is shot mid-jump (`settings-extensions` on the patched engine shows Filter lists passing): wait for the page's spring to land, or jump without it in the gallery
- [ ] Hand check: the rail's sound badge on a tab that really plays audio (the gallery fakes the audible state; no smoke presses mute), and Settings' Spaces list with an agent Space
- [ ] Releasing extensions (the next release ships without them: on an engine without the patch queue wubble offers none): the patch queue in the release engine on Linux, macOS and Windows, then the gaps under M6 (Bitwarden's popup width first)
- Later, in its own build: a passthrough to OS password managers and passkey keychains (fingerprint, passkeys where the laptop has them)

## MCP server
Plan: [docs/research/mcp-server.md](docs/research/mcp-server.md) (section 0: build order; section 0.1: after the first tests). Released in `0.1.0-preview.2`: 20 tools, sessions in their own Space and container held to their sites, the approval sheet, live hosting and agent-app registration from Settings, off by default.
Tested on 2026-11-20 by two agents on real sites (GitHub, Excalidraw, DuckDuckGo, Wikipedia). In order:
- [ ] An element inside a frame is still placed from the accessibility mirror's boxes, which a scrolled page leaves behind (the top document uses the live layout); the mirror's coordinates are wrong after a scroll for the screen-reader pane too (Chromium sends no root scroll offset)
- [ ] a tab moved into an agent session's Space keeps its back-forward cache and any prerender made before: activating either runs no navigation throttle, so the guard is not asked and only the commit backstop takes the tab back (after the page showed); drop those for a tab that joins a session (or disable the cache and prerender for session tabs) and add a smoke probe
- [ ] publish wubble-mcp to npm (owner go, owner's npm account; steps in docs/release.md)
- [ ] agent apps wubble does not write yet (VS Code, Copilot CLI, Goose; Windsurf on macOS): verify each file's name and entry shape, then make them `Format`s in `agent::harness`
- [ ] Windows: the agent apps' paths (`agent::harness` lists Linux and macOS only) and the manifest's runtime dir
- [ ] `scroll` with a direction over a nested scroller (an `overflow: auto` pane): the wheel event reaches the pane but in most `--smoke mcp` runs the document takes the scroll; a user's wheel goes the same path (`View::wheel`): check by hand, then fix (the smoke checks the page only)
- [ ] `wait_for load` does not follow an in-page (history API) navigation's fetches (documented in the tool; `url` and `navigation` cover the address change): network idle would need DevTools' network events through the engine
- [ ] `drag` drives pointer events only: an HTML `draggable` drag runs in the window's own drag loop (dropped with nothing, the reply says so)
- [ ] Not yet tested: a right click's menu; the draft-issue run on a signed-in private repository (waits for the owner's sign-in in the session's tab)
- [ ] Later (owner, 2026-11-20): "Move tab to" in a tab's right-click menu, a two-level menu: hovering the row (with its ">") opens the list of user-created Spaces; moving a tab out of an agent Space makes it an ordinary tab of that Space
- [ ] Later (owner, 2026-11-20): clearing an agent Space that still has tabs asks first: "Close anyway", or "Move tabs", which opens the list of user-created Spaces downward and moves them before the Space is cleared (until then the dustbin clears at once)
- [ ] Bind a requested secret name to a vault login on the sheet (a grant holds no secret until then; `fill_secret` is refused cleanly)
- [ ] Agent views keep rendering while no window shows them (a hidden view has no accessibility tree and no screenshot): find a cheaper way
- [ ] Guardrails, after use shows the need (plan sections 2 to 7): no script on a page a secret was filled into, redaction of filled values, the airlocks

## Review findings (perpetual)
The design-fidelity phase is over; this list never closes and gates nothing: findings from reviews, QA rounds and the
owner's eye land here and are fixed as they come up.
Done on 2026-11-19 and 2026-11-20, after the kit merges listed under "Design fidelity":

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
- [ ] Requests to Google on a fresh profile with or without extensions: component updater, network time, a component diff, the spellcheck dictionary, `www.google.com/async/folae` (net log in [docs/research/extensions.md](docs/research/extensions.md) section 3); "nothing phones home"
- [ ] Shift+Insert paste in shell fields (Slint has no such shortcut); tests for the list editor draft and middle-click
- [ ] `--smoke overlay=dark` fails a hue check under the command bar (also before the glass fix)
- [ ] `controls::bump` reads then writes a property inside the draw (one extra frame; the pattern that looped in glass)

## M0: Frame path
- [ ] Spring spike: a per-frame spring (k 300 / c 15) with no missed frames while a page scrolls
- [ ] Transparent window with compositor blur per platform (macOS vibrancy, KDE blur; GNOME stays opaque)
- [ ] Re-measure `--begin-frames` on a 120 Hz display; on the Mac `--begin-frames`, `video`, `typing`, `resize`
- [ ] Mac console hand checks: ProMotion, window, Cmd keys, IME, popups, Keychain, file chooser, media keys, fullscreen
- [ ] macOS: CEF does not finish shutting down after a windowless run over ssh; find the commit and the cause
- [ ] Windows CI green (then drop the job's `continue-on-error`); the D3D11 shared-handle port after

## M1: Engine layer
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
- [ ] Film rim on the remaining controls (buttons, menu rows)
- [ ] Touch-kit pieces not yet ported: pin, download button, dock with gliding tooltip, the check box's mask grow
- [ ] Gallery sweep: the autofill key chip overlaps the Space header
- [ ] Bookmarks: drag reorder, a bookmarks bar, bookmarks as command-bar candidates

## M3: Memory and sleeping
- [ ] Benchmark against Chrome and Zen: 200 tabs and after 1 h idle (Chromium and Firefox at 1/10/50 done)

## M4: DevTools and built-ins
- [ ] DevTools bottom dock (no design source; needs vertical pane rects)
- [ ] Inspect-at-point in the docked panel (no protocol path; the detached window today)
- [ ] Zen/Arc password import (CSV only) and a notice when the keyring is unavailable

## M5: Arc-style features
- [ ] Sync (deferred)

## Built in instead of extensions
- [ ] Content blocker: uBO scriptlet coverage

## Design fidelity
- [ ] Empty the kit lock baseline (107 rows, mostly `kit/derived.slint` literals); alias the value-equal `Shell` literals
- [ ] Voice glow shader into the kit (no microphone)

## Performance pass
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
- [ ] macOS arm64 codec build (same recipe, `--arm64-build`)
- [ ] Reproducible recipe: Chromium at the CEF branch + ungoogled patches + CEF patches, kept as a patch queue
- [ ] Build with `enable_widevine` and codecs; ship no CDM; a CDM loader (installed CDM, else opt-in download)
- [ ] Document DRM limits (Linux L3 around 720p; services that need a VMP-signed browser refuse us)
- [ ] Build infrastructure: Linux and macOS local; Windows on a runner large enough for Chromium
- [ ] Extensions: P4 (permission prompts answered in wubble's UI); install from the Web Store; the install page's "restart to apply"
- [ ] Extensions shell: shrink-to-fit popup sizing (today the page's body decides, a body without a width fills 800), a tooltip on the footer button (needs an `Action`), the popup's own pointer origin for `<select>` lists, `chrome.action` popups of the global profile, screenshots by the owner's eye
- [ ] Extensions (acceptance gap): `__MSG_name__` names show raw in the install prompt, the list and the popover; resolve them from `_locales` ([docs/research/extensions.md](docs/research/extensions.md) "Acceptance", gap 1)
- [ ] Extensions (acceptance gap): `chrome.tabs.getCurrent()` in an action popup returns a tab (Chrome: undefined), so Bitwarden's popup lays out 57 px wide; patch `TabsGetCurrentFunction` and `getViews` for views that are not wubble tabs (gap 2)
- [ ] Extensions (acceptance gap): `chrome.permissions.request` rejects with "Could not find an active window." (P4: answer it in wubble's UI), which Bitwarden's first-run "default password manager" step calls (gap 3)
- [ ] Extensions (acceptance gap): Bitwarden's inline menu and autofill icon need an account: hand check with a test vault on a loopback login form (gap 4)
- [ ] Extensions: `tabs.onReplaced` when a sleeping tab wakes, `onMoved`/`onAttached`/`onDetached`, window bounds; `windows.create` is refused for now
- [ ] Swap in behind `wubble-engine`, shell unchanged
- [ ] CEF OSR patch: hide a view's compositor on `Hide()` (frees each hidden tab's backbuffer)
- [ ] Skia cache hygiene in Slint's renderer (Chromium's cache controller: a limit, deferred cleanup, free when idle)
- [~] HDR output: researched and spiked; needs the own build (F16 capture) and a Slint renderer patch

## M7: Ship
- [ ] macOS: a codec CEF for the Mac (preview builds there use the prebuilt engine: no H.264 or AAC); the Mac archive was never in the manifest before preview.2, so Macs on preview.1 need one manual install
- [ ] Release gaps: Ubuntu's user-namespace limit vs the no-sudo sandbox; rollback misses a build that cannot load;
      shellcheck not run on `install.sh`; `install.sh` checks only the sha256 when `minisign` is not installed
- [ ] Import app wiring: pick a profile, apply, replace the live model, show the summary
- [ ] Import: Zen legacy tables, Arc history, Little Arc, Helium/Chromium; verify Zen macOS and Arc Windows paths
- [ ] Security update cadence: track Chromium stable
- [ ] Packaging: Flatpak/RPM, MSI/winget, signed and notarised DMG; auto-update
- [ ] Opt-in crash reports that open a GitHub issue (scrubbed)
- [ ] Default-browser registration, URL handlers
- [ ] Hardening: vault unlock off the store thread; clear the clipboard after a password copy

## Optimization roadmap (the current work)
Numbers behind it: [docs/measurements.md](docs/measurements.md) "Browser benchmarks" and "Main-process flame graph" (2026-11-20). Target: the shell's cost follows what is visible, not "match Chrome". In order:
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
- Apple Passwords / passkeys entitlement request (the owner files it)
- Video strip: does a collapse carry over to later pages; Pip: a cropped video or the page's own element fullscreen
- Sidebar day labels ("Today": the design draws none); whether `cpu_fallback > 0` fails `--smoke frames`
- If Turso (pre-1.0) proves unstable: the schema stays SQLite-compatible so `rusqlite` can replace it

## Risks
1. Slint's wgpu texture import is an `unstable-` feature and can change on any minor: pin versions.
2. Own glass and backdrop must fit the 8.33 ms frame beside Chromium's GPU work; the quality ladder is the fallback.
3. Prebuilt CEF is not ungoogled and has no proprietary codecs until M6.
4. `welding` pins CEF; every CEF upgrade is ours to do in the fork.
5. Extensions under windowless CEF need our patch queue (`patches/chromium`), rebuilt with every CEF upgrade and not yet built for macOS or Windows.
6. OSR costs one extra composite per frame and bypasses direct scanout for video.
7. An ungoogled + CEF patch queue rebased every Chromium milestone is permanent work for one person.
8. Windows is CI-only: input, IME and windowing bugs there are found late.
9. Turso is pre-release: keep the file format SQLite-compatible and back up before migrations.
