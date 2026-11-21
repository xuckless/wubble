# wubble roadmap

A Chromium-based browser with a Slint shell: Chromium renders off-screen, Slint composites it as GPU textures.
Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md) ·
fidelity: [docs/fidelity/](docs/fidelity/kit-plan.md) · research: [docs/research/](docs/research/) · commands: [docs/smokes.md](docs/smokes.md)
Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

## Status (2026-11-21)
`main` builds, lints and passes 2293 tests; every window smoke passes on the invisible test display
(`scripts/headless.sh`), and an idle window draws nothing. Released: `0.1.0-preview.4` for Linux (drag and drop, pinch, spellcheck, partial rendering, the autofill-server request off; its macOS build is not made yet: the owner's, on the Mac, from the tag, then signed and added here; the Mac stays on preview.3 until then) and `0.1.0-preview.3` for Linux and macOS (preview.3 fixes MCP hosting on macOS; preview.2, with codecs,
added the MCP server, off by default: live hosting from Settings, 20 tools, sessions held to their sites, agent apps
registered on turn-on); the macOS (Apple silicon) build, signed and notarised, is in the same release and in the update manifest
(made by the owner on the Mac: `sh /Users/Shared/wubble-preview2-mac.sh`, then signed and uploaded from here). Extensions are
built but not released: they need the patched engine, which the release engine is not. The npm launcher
(`packages/wubble-mcp`) is built, not published. Windows is not green in CI.

## Hardening pass (2026-11-21), merged, not yet released
Four lanes drove every Settings section and the features behind it on throwaway profiles (checklists in
`~/Dev/wubble-wt/notes/hp/*-checklist.md`): the Settings controls, the built-in pages and clearing data, privacy
and Spaces, passwords with autofill and sign-ins for agent sessions. What they left open is in the lists below.
Owner decisions waiting: whether `update.googleapis.com`, `*.gvt1.com` and `*.gvt2.com` stay unresolvable inside the browser; releasing
this as `0.1.0-preview.4`. Not run on macOS: any of it (run `cargo test --workspace` and `--smoke passwords` with
`WUBBLE_SMOKE_KEYRING=os` there before the release).

## Now: the optimization pass
The build-out through M5, extensions and the MCP server is done; the next work is the "Optimization roadmap" below,
in its order. Steps 1 and 2 (partial rendering of the shell, CEF's damage through the frame path) are done.
Measure with [scripts/bench/](scripts/bench/README.md) (`profile.sh`, `breakdown.py`, `runner.py`) so every step has a
before and an after on the same build; the numbers to beat are in [docs/measurements.md](docs/measurements.md). Measured by
hand only: nothing in `cargo test` or CI asserts a timing.

Beside it, small and unordered:
- [ ] Build the engine with patch 40, then verify the touchpad pinch road (`--smoke pinch` takes it when the marker is there; unit-tested only so far); macOS engine too, and a real trackpad on GNOME and the Mac (the smoke injects through `Host::pinch`)
- [ ] Wire the thinking orb (in the kit, gallery only so far): the agent mark while a call is in flight, the activity page header, the approval sheet; after the owner reads the proposal in `docs/research/agent-widgets.md` section 5
- [ ] The mascots are researched, not ported: their fabric and plastic shading is CPU-baked with per-frame readbacks in the web version, so a faithful port needs a GPU rewrite (`docs/research/agent-widgets.md` section 3.3); owner decides whether to do it
- [ ] Thinking orb: the `gravity` cursor warp is not ported; one moving 64 px orb costs a whole-window redraw per frame (about 8 to 9 % of a core at 60 Hz, hand run): tied to "Stop redrawing the shell" in the optimization roadmap
- [ ] The agent glyph: the design's `sparkle` stands in (one alias, `Glyph.agent`) and reads as a crosshair at badge size; the owner's glyph, a mascot or the orb, then one swap
- [ ] The Settings Select's list is drawn by Slint (no blur): the scene's window-level glass lies over the page; the design's blur needs the page's lists in a window-level layer (owner's call)
- [ ] Gallery: a Settings section picked far from where the page is is shot mid-jump (`settings-extensions` on the patched engine shows Filter lists passing): wait for the page's spring to land, or jump without it in the gallery
- [ ] Hand check: the rail's sound badge on a tab that really plays audio (the gallery fakes the audible state; no smoke presses mute), and Settings' Spaces list with an agent Space
- [ ] Releasing extensions (the next release ships without them: on an engine without the patch queue wubble offers none): the patch queue in the release engine on Linux, macOS and Windows, then the gaps under M6 (Bitwarden's popup width first)
- Later, in its own build: a passthrough to OS password managers and passkey keychains (fingerprint, passkeys where the laptop has them)

## After the optimization pass (owner, 2026-11-21)

Built as planned in the research, each as its own pass; nothing downloads or asks a server unless the owner turns it on.

- [ ] Page translation, on-device: the pipeline in [docs/research/translate.md](docs/research/translate.md)
- [ ] Live text in images, on-device OCR: the pipeline in [docs/research/ocr.md](docs/research/ocr.md)

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
- [ ] Autofill: offer the login form a page shows right after a submit (the save check consumes that report today)
- [ ] Autofill: re-probe Reddit (bot wall) and Wikipedia (redirects to auth.wikimedia.org) logins by hand
- [ ] Agent sign-ins: copy a site's localStorage/IndexedDB too (DevTools DOMStorage needs a live page per container; MSAL, Firebase and SPA tokens live there)
- [ ] Agent sign-ins: CHIPS-partitioned cookies arrive unpartitioned (CEF's cookie struct has no partition key)
- [ ] Agent sign-ins: `--smoke mcp` under load failed once in 87 loaded runs with the session's first request to the shared site carrying no cookie ("sent: none") after the copy answered; not reproduced or traced yet (the copy's report was not printed)
- [ ] Passwords: the owner's "saved by hand, then gone" still unexplained (a keyring answering "no key" made a new one over the rows until now: guarded); run `--smoke passwords` with the Keychain on the Mac
- [ ] Agent views keep rendering while no window shows them (a hidden view has no accessibility tree and no screenshot): find a cheaper way
- [ ] Guardrails, after use shows the need (plan sections 2 to 7): the airlocks
- [ ] Agent tabs ask the human (the airlocks' prompt) for a permission, a download or an external link, instead of refusing them outright as now
- [ ] Clipboard isolation for agent tabs on every platform (the Wayland bridge cannot tell an agent's copy or paste from the human's; elsewhere Chromium writes itself)

## Hardening pass: built-in pages and their data (2026-11-21, open findings; checklist in the report)
- [ ] Task manager on Linux with the sandbox: each renderer is pid 1 in its own PID namespace, so a tab cannot be tied to its renderer (tab rows have no memory, CPU or End) and renderers are listed as "Zygote"; needs a way to ask CEF's browser process for a tab's renderer pid
- [ ] List pages (History, Bookmarks, Downloads, Archive): a row cannot be opened from the keyboard (only its Delete is a Tab stop; a screen reader's default action opens it); no select, delete selection, undo, copy address or keyboard open in a new tab; a bookmark folder is deleted with its contents by one press; needs a focusable kit row and a design for selection and undo
- [ ] Media downloader: the Mac `wubble-ffmpeg` (scripts/release/build-ffmpeg.sh, signed in package-macos.sh) is written, never run: build and notarise it on the Mac, then try a YouTube pair and an HLS download in the app
- [ ] Media downloader: YouTube's player API answered "Sign in to confirm you're not a bot" to the `android_vr` client from the dev box (shown to the user now, not worked around); its format URLs' own fetch is untried; an owner call: allow one 1 MB ranged fetch from a free video, and say which sites the owner tried
- [ ] Media downloader: a DASH manifest of several periods is refused ("Not supported"); the default quality takes the first row of a height whatever its codec (HEVC or AV1 before H.264)
- [ ] Downloads: Retry restarts from any tab of the download's container, not the tab it came from (a login-gated file can come back as a sign-in page)
- [ ] Agent sessions' pages are filed in the profile's icon table (origin and icon) though their visits are not history; the agent activity log cannot be cleared
- [ ] `--smoke data` runs each process on its own CEF cache: a cookie cleared right before a SIGKILL and read after the next start is not checked

## Review findings (perpetual)
The design-fidelity phase is over; this list never closes and gates nothing: findings from reviews, QA rounds and the
owner's eye land here and are fixed as they come up.
Done on 2026-11-19 and 2026-11-20, after the kit merges listed under "Design fidelity":

Open, for the owner's eye:
- [ ] Glass over white: faint ink (disabled rows, shortcuts, counts) sits near 2:1; glass reads flat grey
- [ ] Menus grow from the design's 236 to 320 for long rows (derived; on the review sheet)
- [ ] Sidebar mid-collapse shows the rail look below 140 wide (the design's own behaviour; QA read it as a reflow)
- [ ] Rail close badge size; video strip: should a collapse carry over to later pages
- [ ] Find bar closes on reload (Chromium keeps it)
- [ ] Archive's icon: the stacked trays, shared with the Spaces section (the design's set has no archive glyph);
      Passwords/Sites share the lock, Updates/Downloads the arrow
- [ ] Spaces popup: its alignment (Space tile's left edge), no rename/delete on a row
- [ ] Media downloader: a video a service worker answers is not seen by the detection (the worker's own fetch has no tab; `--smoke media=probe` on fixtures `sw`): the save control says nothing was found; fullscreen shows the strip without its glass (the scene does not render there): look by hand; an embedded YouTube player (an iframe of youtube.com) has no lookup, since the lookup keys on the tab's own address
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
- [ ] Gallery: scenes for the hover preview card and the sidebar's empty-area menu; the page menu's position varies by page
- [ ] Spell check in other languages: the English dictionary is bundled; add an opt-in (off by default) download of others, from `redirector.gvt1.com` or a mirror, into `<profile>/Dictionaries`
- [ ] Shift+Insert paste in shell fields (Slint has no such shortcut); tests for the list editor draft and middle-click
- [ ] `--smoke overlay=dark` fails a hue check under the command bar (also before the glass fix)
- [ ] The autofill key chip sits over the page's top left and is not pane-aware in split view: it needs the focused field's rect from the engine
- [ ] Rebinding the sidebar shortcut also unbinds its second chord, Mod+Alt+S (once compact mode's), and the shortcut editor shows only the first chord
- [ ] Narrow Settings (content width 759 px or less): the slider and select rows do not wrap to full width as the design's narrow block does (`settings.css`:120-130); repro `WUBBLE_SHOT_SIZE=960x640 --smoke settings --shot`
- [ ] The built-in pages' list rows take Tab and Enter but not Up/Down, and a focused row is not scrolled into view
- [ ] `FocusKeys` with `native-button: false` fires again on key repeat (a held Enter or Space)
- [ ] The settings search no longer finds "Your own filter lists" (the setting moved to `Section::Hidden`; the Filter lists section is not searchable)
- [ ] A Space button opened by keyboard does not give the Spaces menu the keyboard focus
- [ ] The popovers' scroll thumbs' touch area covers the rows' right edge
- [ ] The Settings Select clips a long value at its width ("Ask on first sea" for the search engine with none chosen; on main since the placeholder came, before PR #2): fit the control to its longest label, or elide
- [ ] A popover's entrance marks the whole window dirty (the glass's scene scale and alpha change each frame), so partial rendering does not help during it

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
- [ ] Clipboard and drag: non-text page clipboard formats; custom MIME to other apps
- [ ] macOS drags (`wl/macos.rs`) run on a Mac: written and compile-checked from Linux only. Out: session starts from winit's `mouseDragged:` (the current event), image placement and 2x sharpness, Finder takes the promised picture, Safari or Notes the link with its title; in: the `draggingEntered:` family added to winit's view (file drops now come through it, not winit's `DroppedFile`), the page's answer reaching `draggingUpdated:`
- [ ] Drag out on Windows (the stub: `wl::Null`)
- [ ] Drags out of the window and in from other apps checked by hand on a desktop (the smoke cannot move the compositor's pointer: `--smoke dnd` covers drags inside the window, the Wayland offer is unit-tested)
- [ ] A bookmark dropped into a folder row lands at its end; Chrome's drop line between rows (a position) waits for drag reorder of bookmarks
- [ ] Page context menu: the Picture in Picture row; media rows act through a page script (no `MediaPlayerAction`)
- [ ] Text selection handles (touch) and a match-case toggle in the find bar
- [ ] Media session (metadata, action handlers, OS controls); a real audible signal (CEF patch, M6)
- [ ] Web content as a real accessibility subtree (waits for a Slint graft API or a winit backend patch)
- [ ] Windows sandbox: restructure as a client DLL behind CEF's `bootstrap.exe`

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

## Privacy and containers (hardening pass, open)
- [ ] "Allow pop-ups" for a site never takes effect without a click, and the pop-up pill never appears: Chromium refuses a script `window.open` with no gesture before the host hears of it (`CanCreateWindow`, default pop-up setting block), so `popup::decide`'s no-gesture rows are dead; set the request context's pop-up content setting to allow (`CefRequestContext::SetContentSetting`, a welding call) so the table decides (with a click it works: remembered Allow/Block read right and survive a restart)
- [ ] No way to create, rename or delete a container (only agent sessions make one): the routing rules' Container column can name only Default or a live agent container
- [ ] Site permission decisions are per origin, not per container (a camera allowed in one container is allowed in all)
- [ ] Routing rules: renaming or deleting a Space (or container) a rule names silently turns the rule off, with no mark in the list; Space names are not unique; a Space named with `,` or `->` cannot be named by a rule; the row label does not say typed addresses and page links are not routed
- [ ] Pill and counter: no check drives the shell by pointer for the blocker menu, the Sites list revoke or the Spaces list in a running window (only the engine side is driven, `notes/hp/privtools` of the pass)

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
- [ ] Read aloud with an on-device voice (tiny-voca or the OS voice); a voice, not a screen reader ([docs/research/tts.md](docs/research/tts.md))
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
- [ ] macOS: the Mac archive was never in the update manifest before preview.2, so a Mac on preview.1 needs one install by hand; video playback (H.264, AAC) on the Mac build is not checked (it is built on the owner's codec CEF, `/Users/ali/cef-codecs`)
- [ ] Before every release: run `cargo test --workspace` on the Mac (preview.2 shipped with MCP hosting broken on macOS; one test there would have caught it), and run the whole suite there once now (never done)
- [ ] Release gaps: Ubuntu's user-namespace limit vs the no-sudo sandbox; rollback misses a build that cannot load;
      shellcheck not run on `install.sh`
- [ ] Import app wiring: pick a profile, apply, replace the live model, show the summary
- [ ] Import: Zen legacy tables, Arc history, Little Arc, Helium/Chromium; verify Zen macOS and Arc Windows paths
- [ ] Security update cadence: track Chromium stable
- [ ] Packaging: Flatpak/RPM, MSI/winget, signed and notarised DMG; auto-update
- [ ] Opt-in crash reports that open a GitHub issue (scrubbed)
- [ ] Default-browser registration, URL handlers
- [ ] Hardening: vault unlock off the store thread

## Optimization roadmap (the current work)
Numbers behind it: [docs/measurements.md](docs/measurements.md) "Browser benchmarks" and "Main-process flame graph" (2026-11-20). Target: the shell's cost follows what is visible, not "match Chrome". In order:
- [ ] The shell's items are still walked twice per page frame (Slint's dirty-region pass and render walk): the roads and estimates are in docs/research/renderer-roads.md section 6; the owner decides on a core fork or waiting for slint#15028
- [ ] An inset shadow (`inner-shadow-*`) at a fractional device position: its anti-aliased edge column comes out up to 5 levels different when a partial frame's clip covers it than in a whole frame, and stays until the item is redrawn (Settings' section pill, the address field; gallery 69 in 2 of 6 runs). Snapping the shadow image to the pixel grid removes it but moves the shadow's edges by up to 3 levels against upstream: the owner's call (measurements.md "Pixel questions")
- [ ] Chromium's GPU process exits (133, a trap) right after the gallery's agent scenes 12 and 13 end their session (`agents_stop`) in most runs, on main too; when it happens with the page alive, that page lays out again with smaller body text until it is reloaded (`42-toast-question`; reproduced on main by killing the GPU process)
- [ ] A scene render (ambience, 8 to 10 a second during video and scroll) marks the whole window dirty; mark only what the scene shows through
- [ ] A page's damage far from the glass (a spinner mid-page) still rebuilds the page blur for the ambience at 8 Hz, a whole-window frame: build it only when the damage is within the ambience's reach (about 130 px of a pane edge) or under overlay glass
- [ ] The start page renders the scene and the window at the display's rate for as long as it shows (57/s here, 120 on the owner's Mac): a lower rate for its liquid and bubble is the owner's call (measurements.md "Energy per scenario")
- [ ] The CEF pump wakes 30 times a second on a still page (its 33 ms ceiling, a recorded decision): report the energy case to the owner before changing it
- [ ] macOS: the window is transparent (`background: transparent`, so the Metal layer is not opaque) and WindowServer blends the whole window on every present; an opaque window where the shell covers it all, after a look on the Mac
- [ ] Content-driven frame rate: at most one page frame per refresh; CEF's rate follows the content (still: none, video: its own rate, input and motion: vsync); one pure state machine in `wubble_gfx::pacing`
- [ ] Profile again with kernel time (needs root for `perf`): GPU driver calls and present, about a third of the main process's CPU, unseen so far
- [ ] The CEF message pump on the main thread (14 % during video): what runs there per frame
- [ ] Find the Speedometer gap (16 % behind Chromium): run it in plain CEF with our `libcef.so`, no shell
- [ ] CEF build with PGO and ThinLTO (`scripts/build-cef.sh` sets `chrome_pgo_phase=0 use_thin_lto=false`), if the plain-CEF run is also behind
- [ ] Memory breakdown per allocation in the main and GPU processes, before any memory change
- [ ] Hidden tabs free their frame targets; the blur pyramid exists only while glass is open; revisit Skia's 256 MiB layer budget
- [ ] A page frame touches the scene and the blur pyramid only while overlay glass or motion is on screen (1.3 % today: low value)
- [ ] Re-run the benchmarks, the flame graph and the video CPU/RAM profile against Chromium and Firefox after each step (`scripts/bench/`; the Speedometer and MotionMark checkouts are not kept: fetch them into a scratch directory)
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
