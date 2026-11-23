# wubble roadmap

A Chromium-based browser with a Slint shell: Chromium renders off-screen, Slint composites it as GPU textures.
Decisions: [docs/decisions.md](docs/decisions.md) · numbers: [docs/measurements.md](docs/measurements.md) ·
fidelity: [docs/fidelity/](docs/fidelity/kit-plan.md) · research: [docs/research/](docs/research/) · commands: [docs/smokes.md](docs/smokes.md)
Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

## Status (2026-11-21)
`main` builds, lints and passes 2293 tests; every window smoke passes on the invisible test display
(`scripts/headless.sh`), and an idle window draws nothing. CI is green on Linux and macOS; Windows does not pass yet.
Released: `0.1.0-preview.5` for Linux and macOS (signed; the Mac build notarised, made by the owner on the Mac with
`sh /Users/Shared/wubble-preview<N>-mac.sh`, then signed and uploaded from here). preview.4 brought drag and drop,
pinch zoom, spellcheck, the media save control with a bundled ffmpeg, partial rendering of the shell, the hardening
pass and PRs #1 and #2, and switched off Chromium's autofill-server request; preview.5 fixes updates on macOS
(quarantine), released on the owner's word without a run on a Mac. The site is live at https://wubble.cc (`site/`).
Not released: extensions (they need the patched engine, which the release engine is not) and the npm launcher
(`packages/wubble-mcp`, built, unpublished). Never run on a Mac by hand: the macOS drags, Cmd+Q, the socket
check, the quarantine fix, Cmd+Z undo close tab; `cargo test --workspace` there passes in CI only.

## Website (wubble.cc)
`site/` is plain HTML, CSS and JS, served by a Cloudflare Worker (`site/README.md`); it went live as first built, with
README wording as placeholder copy.
- [ ] The copy and what the page says first: with the owner (the sentences not from README are listed in `site/README.md`'s history; nothing invented, no numbers without a measurement)
- [ ] Reshoot the app shots on a clean profile (the Space is named "Agent: draft the issue", one shot shows a test page), and put a real site in the window under the hero in place of a second start page
- [ ] The feature section reads as a list (bullets left, image right, four times; empty left columns on wide screens): recompose it from the design's pages
- [ ] The SVG diagrams' text is unreadable at 360 px; the install command is cut off in its capsule
- [ ] Owner: "Always Use HTTPS" for the zone (plain http serves the page without a redirect)
- [ ] Each release: bump `VERSION` in `site/assets/js/site.js` and `npx wrangler deploy` in `site/` (in docs/release.md)

## Now: the optimization pass
The build-out through M5, extensions and the MCP server is done; the next work is the "Optimization roadmap" below,
in its order. Steps 1 and 2 (partial rendering of the shell, CEF's damage through the frame path) are done.
Measure with [scripts/bench/](scripts/bench/README.md) (`profile.sh`, `breakdown.py`, `runner.py`) so every step has a
before and an after on the same build; the numbers to beat are in [docs/measurements.md](docs/measurements.md). Measured by
hand only: nothing in `cargo test` or CI asserts a timing.

Beside it, small and unordered:
- [ ] Undo close tab (Mod+Z) and Mod+Shift+T bring back the URL only: no split, no back/forward history (the recently-closed list would need CEF's navigation entries)
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

- [ ] macOS: the quarantine fix for updates (`settle_quarantine`, the updater's clear before placing) is compiled but never run on a Mac: on the owner's Mac, an install updated by wubble itself must start from `/Applications` (Settings → Updates not "managed elsewhere") and offer the next update; preview.5 shipped it unchecked

## After the optimization pass (owner, 2026-11-21)

Built as planned in the research, each as its own pass; nothing downloads or asks a server unless the owner turns it on.

- [ ] Page translation, on-device: the pipeline in [docs/research/translate.md](docs/research/translate.md) (build-out `translate-detect-offer`, `translate-bergamot-backend`, `translate-selection-reader`). First, spikes of about 1.5 days: does Chrome's `Translator` API answer in CEF's windowless mode (`View::eval` of `Translator.availability`; expected not), Bergamot's HTML and alignment API, the model registry and each model's licence (the models repo is archived). Then a `translate` frame channel and walker script (the cosmetic channel's pattern), `wubble_core::translate` (state, `whatlang` detection), a helper process running bergamot-translator; about 17 days in all
- [ ] Live text in images, on-device OCR: the pipeline in [docs/research/ocr.md](docs/research/ocr.md)

## MCP server
Plan: [docs/research/mcp-server.md](docs/research/mcp-server.md) (section 0: build order; section 0.1: after the first tests). Released in `0.1.0-preview.2`: 20 tools (22 since, with `select` and `upload_file`), sessions in their own Space and container held to their sites, the approval sheet, live hosting and agent-app registration from Settings, off by default.
Tested on 2026-11-20 by two agents on real sites (GitHub, Excalidraw, DuckDuckGo, Wikipedia). In order:
- [ ] An element inside a frame is still placed from the accessibility mirror's boxes, which a scrolled page leaves behind (the top document uses the live layout); the mirror's coordinates are wrong after a scroll for the screen-reader pane too (Chromium sends no root scroll offset)
- [ ] a tab moved into an agent session's Space keeps its back-forward cache and any prerender made before: activating either runs no navigation throttle, so the guard is not asked and only the commit backstop takes the tab back (after the page showed); drop those for a tab that joins a session (or disable the cache and prerender for session tabs) and add a smoke probe
- [ ] publish wubble-mcp to npm (owner go, owner's npm account; steps in docs/release.md)
- [ ] agent apps wubble does not write yet (VS Code, Copilot CLI, Goose; Windsurf on macOS): verify each file's name and entry shape, then make them `Format`s in `agent::harness`
- [ ] Windows: the agent apps' paths (`agent::harness` lists Linux and macOS only) and the manifest's runtime dir
- [ ] `scroll` with a direction over a nested scroller (an `overflow: auto` pane): the wheel event reaches the pane but in most `--smoke mcp` runs the document takes the scroll; a user's wheel goes the same path (`View::wheel`): check by hand, then fix (the smoke checks the page only)
- [ ] `wait_for load` does not follow an in-page (history API) navigation's fetches (documented in the tool; `url` and `navigation` cover the address change): network idle would need DevTools' network events through the engine
- [ ] `read_page` lists no `<option>`, list box or `<select multiple>`/`size` (a collapsed select's options are AX-invisible; `listBox` and its options are no `wubble_ax` item kind): `select` takes an option's ref and a multiple list, but no ref reaches one yet
- [ ] `select` on a `<select multiple>` sets one choice (the others cleared); adding a choice is not offered
- [ ] `type`'s focus check runs in the top document: focus moving between fields inside one frame is not seen (a frame counts as editable), and a custom element whose shadow root is closed counts as editable once focus moved to it
- [ ] `upload_file` into a frame of another site (its own process): the top target's DevTools session neither reaches its input (`DOM.setFileInputFiles`) nor intercepts its chooser, so the click's chooser is cancelled and the call fails; reaching it needs `Target.setAutoAttach` with flattened sessions through the engine
- [ ] `upload_file`'s path check runs on a thread of its own per call: a path on a hung mount fails the call at 5 s, but its thread stays blocked until the file system answers
- [ ] `evaluate`'s object group is shared by a tab's calls and not released when one times out or its session goes (`select`'s is per call and always released)
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
- [ ] From PR #4's review (2026-11-22), not fixed in the merge: a headless agent session's dialogs are answered for it (alert accepted, confirm and prompt cancelled, leave-page accepted) and the tool call in flight is not told, and the log line says "refused" whatever the answer; a sign-in copy of which no cookie was accepted now stops a starting session (`CookiesNotCopied`): owner's call whether that is wanted; the approval sheet counts its open Selects, so one destroyed while open (the bindings rewritten) leaves Escape dead; a second Save click while a media download runs does nothing and says nothing; a failed ffmpeg join now discards the fetched tracks; `space-switcher.slint` got a spacing the design does not have and `crossfade.slint` a clip that may cut the swap motion (derived-review sheet); a scroll thumb that hides mid-press loses its release
- [ ] The scene's partial marking (PR #4): after a scene render only the strips around the one page and the overlay glass are redrawn, when that page fills the content area; checked with `SLINT_SKIA_PARTIAL_RENDERING=verify` on the video, overlay, page, split and tabs smokes (0 mismatches), not yet on a tab drag preview, a history swipe or a split closing to one pane
- [ ] QA retry by hand: Alt+1/2 Spaces, Shift+click peek, audible badge on a playing YouTube tab (tests pass; not reproduced)
- [ ] The command bar's panel still takes clicks while it shrinks away (176 ms): a click on the page under it then is lost; check the design's exit (pointer-events)
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
- [ ] macOS: a held key's repeats reach the page with `repeat: false` (CEF 156 builds its NSEvent with `isARepeat:NO`): a one-line CEF patch, then drop parity's `A.space-repeat` Mac gap; check YouTube's Space tap and hold-for-2x on the Mac
- [ ] The settings search no longer finds "Your own filter lists" (the setting moved to `Section::Hidden`; the Filter lists section is not searchable)
- [ ] A Space button opened by keyboard does not give the Spaces menu the keyboard focus
- [ ] A popover's entrance marks the whole window dirty (the glass's scene scale and alpha change each frame), so partial rendering does not help during it

## M0: Frame path
- [ ] Spring spike: a per-frame spring (k 300 / c 15) with no missed frames while a page scrolls
- [ ] Transparent window with compositor blur per platform (macOS vibrancy, KDE blur; GNOME stays opaque)
- [ ] Re-measure `--begin-frames` on a 120 Hz (and the 300 Hz) display on Linux: it was left off there in b2 for a worse video render interval at 240 Hz
- [ ] Mac console hand checks: ProMotion, window, Cmd keys, IME, popups, Keychain, file chooser, media keys, fullscreen
- [ ] macOS: CEF does not finish shutting down after a windowless run over ssh; find the commit and the cause
- [ ] Windows CI: the Test step now finishes and 4 tests of the `wubble` binary fail there (run 44988591703, 2026-11-22): `instance::tests::the_private_root_by_platform`, `mcp::tests::a_connection_answers_discovery_and_hands_calls_to_the_host`, `mcp::tests::a_stop_closes_open_connections_and_ids_never_repeat`, `updater::install::tests::linux_stages_switches_current_and_rolls_back` (the last is a Linux-only test not gated)

## M1: Engine layer
- [ ] Hand checks: dead keys, autoscroll cursor feel, IME with ibus, popups, `window.open` and its badge, a download, fullscreen
- [ ] Per platform: view lifecycle on the Mac and Windows; GPU process watch on macOS and Windows; macOS and Windows popup frame
- [ ] Input gaps: kinetic wheel phase on Linux (host-side fling), touch input, the AltGr flag, CSS `url()` and zoom cursors
- [ ] IME gaps: no caret rect before the first composed character; no underline styles or replacement range
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

## Performance pass 3 (owner, 2026-11-22)
Target: the main process under 5 % of a core while a page moves (it is 8 to 9 % for video or a spinner at 60 Hz, 14 % on the 300 Hz display; idle is 1 to 2 %). The owner approved all six on 2026-11-22, the core fork and the faked glass included. Branch `untested/perf3` holds the first three as code written without a compile or a run (the machine was busy): build, test and measure it before anything else. Not in a release yet: PR #4's performance work (fcc4d23: scene dirty rects, the blur's reach, idle timers, cached bind groups), merged after preview.7 was tagged.
- [ ] Build and test `untested/perf3` (`cargo test -p wubble-core -p wubble-gfx`, then the workspace), then measure each commit alone: `--measure idle` (pump runs a second), the spinner and caret pages on the 300 Hz display (CEF frames a second), `scripts/bench/profile.sh`
- [ ] Frame rate by content (written, `pacing::ContentRate`): a page with small damage and no input for 300 ms paints at 60 Hz at most. Left: the shell's begin frames (macOS) are not capped, so a spinner there still paints at 120 Hz; a pop-up view takes its opener's ceiling; look at a small animation at 60 on the 300 Hz display by eye
- [ ] Quiet pump (written, `pump::Tempo::Quiet`): 250 ms between runs once nothing painted for a second, not 33 ms. Left: check nothing waits on the pump as a clock (`Host::show_frame` runs after each pump; start-up, downloads, an agent's page); the begin-frame driver's own 33 ms idle timer on macOS; then try no ceiling at all
- [ ] Profile-guided build (written, `scripts/bench/pgo.sh`, never run): compare with the plain `dist` build, then decide whether releases use it
- [ ] Fork `i-slint-core` (vendored beside the Skia renderer): skip unchanged sub-trees in the render walk, then in the dirty-region walk (renderer-roads.md section 6, changes 1 and 2; up to 30 % of the main thread during page frames)
- [ ] One copy less per page frame, by a CEF patch: OSR keeps the frame's buffer until wubble releases it and delivers RGBA; wubble imports each pooled dmabuf (IOSurface) once and samples it, so the copy and its wait in the paint callback (`vendor/welding/src/native_frame/mod.rs`), the swizzle blit and the per-frame import go
- [ ] The page on a compositor surface: each pane's page on its own `wl_subsurface` (macOS: a `CALayer` on the IOSurface) with CEF's buffer attached directly, the shell above it; glass over a page is faked (a low-rate sample of the page, as the ambience's 8 Hz). A page frame then costs one attach and commit: no import, blit, Slint walk, flush or present. Overturns four frame-path decisions; prototype one pane without glass on Linux and measure first

## Web view parity with Chrome (owner, 2026-11-22)
The owner wants the page view to feel like Chrome's, 1:1. Audit of 2026-11-22, read from the code, nothing run; drags inside a window, between panes, onto the shell and to and from other apps on Wayland already work. In the order a user feels them:
- [ ] Wayland clipboard is text only: a page's copy and paste loses HTML, images, files and custom formats (`clipboard.rs` `bridge_needed`, `wl/wayland.rs` `TEXT_TYPES`; build-out `wayland-clipboard-rich`)
- [ ] The page is told the window is the screen: `screen.width`, `availLeft`, `window.screenX` (welding `linux_cef/cef_backed.rs` `screen_info`, engine `ScreenArea`); report the monitor and the window's place, keep pop-ups clamped in the host
- [ ] Touch and pen never reach a page (Slint turns a touch into mouse events): no touchstart, no pen PointerEvents, no touch scroll or long press (build-out `touch-input`, `pen-input`)
- [ ] Cursors: CSS image cursors and their hotspot are dropped (`on_cursor_change` ignores the custom cursor info); zoom-in, zoom-out, copy, alias, context-menu, all-scroll and cell show the arrow (build-out `cursor-custom-altgr`)
- [ ] Drags to and from other apps: none on X11 and Windows beyond dropped files (`wl::Null`), macOS written and never run; custom dataTransfer types leave the window nowhere; a page drag to a second wubble window (build-out `x11-xdnd`, `windows-drag-drop`, `drag-custom-types-out`, `cross-window-page-drag`)
- [ ] Wheel: check by hand that a mouse wheel gets Blink's smooth scroll (the deltas are sent as precise); no fling on Linux touchpads; no momentum phases or elastic overscroll on macOS (needs a CEF patch)
- [ ] IME: no caret rect before the first composed character (the candidate window starts misplaced), no underline styles, on in password fields on Linux and macOS (build-out `ime-caret-underlines`)
- [ ] Pointer lock: the permission prompt exists, nothing confines the pointer or sends relative motion (a CEF patch, likely)
- [ ] The back and forward mouse buttons navigate in the shell and never reach the page (no `auxclick`)
- [ ] `<datalist>` shows nothing; the context menu lacks Open Link in New Window, Add to dictionary, Search image; `window.open` ignores the position

## Features no plan has yet (scout, 2026-11-22)
Checked against the build-out list and this file; the owner picks.
- [ ] Preferred languages: Accept-Language and the page locale (nothing sets them; translation and spell languages need it first)
- [ ] QR code for a page or a link; search an image (an engine's URL template); copy link to highlight (text fragments)
- [ ] Device pickers for WebUSB, Serial, HID and Bluetooth; FedCM and Payment Request (spike each: does CEF's windowless mode forward them)
- [ ] Per-site user agent ("request desktop site"); history grouped by task; sidebar web panels; the shell in other languages

## Optimization roadmap (the current work)
Numbers behind it: [docs/measurements.md](docs/measurements.md) "Browser benchmarks" and "Main-process flame graph" (2026-11-20). Target: the shell's cost follows what is visible, not "match Chrome". In order:
### From the owner's Mac run of preview.5 (2026-11-21)
The numbers are in docs/measurements.md ("The owner's Mac, preview.5"). Five findings, in the order to take them; each
starts with a measurement on the Mac that names the cause before any code changes (`sample` on the main process, the
`--measure` counters, `scripts/bench/mac-energy.sh`), because the causes below are suspects, not findings.
- [ ] 1. One video costs the main process about 26 % and the GPU 60 to 78 % (5K display, 120 Hz). Suspect: the window is drawn per page frame at the display's rate. Steps, each measured on its own: (a) count CEF frames, Slint renders and presents per second during a 60 fps video on the Mac (`--measure video` there): is it 60 or 120; (b) CEF's paint rate follows the content ("Content-driven frame rate" below: video at its own rate, a still page none); (c) the opaque window on macOS (WindowServer blends the whole 5K window per present today); (d) fullscreen video: draw the page alone, no shell items, no glass; (e) what is left is the second item walk (the i-slint-core fork decision)
- [ ] 2. Ten idle tabs keep the main process at about 10 %: measured 2026-11-21 (measurements.md, "with stack samples"): it is the accessibility mirror, on for every tab of an agent session, applying updates from pages that animate while hidden (6.6 of the 8.8 % the main thread is busy); the benchmark drives all its tabs through a session. Two things follow: (a) agent sessions: do not mirror a tab that is hidden or that no call is reading (keep CEF's tree on, apply updates lazily at `read_page`, or turn the tree on per call), since every MCP user pays this in every phase (3 to 8 points); (b) measure ten idle tabs opened by hand, outside a session, to learn what a person's idle costs (the owner opens them, `sampler.sh` over ssh)
- [ ] `package-macos.sh` now packs `wubble-<V>-macos-aarch64.debug.tar.xz` (the dSYMs); its first run on the Mac checks it (written without a Mac): the app's size against preview.5's, then a `sample` during video read with the dSYM
- [ ] Mac frame drops, reproduced 2026-11-21 in Low Power Mode (measurements.md "frame runs in Low Power Mode"), in this order:
  - [ ] the start page: its frames redraw only the page card (Linux: dirty share 1.0 to 0.78, 56 to 20 items a frame), and it settles 2 s after the last input and then draws nothing (owner, 2026-11-21); measure both on the Mac
  - [ ] shell-paced begin frames are the macOS default now (decisions.md "Frame pacing"): on the Mac, `--measure scroll`, `video`, `canvas`, `typing`, `resize` and `idle` with and without `--no-begin-frames`, and a PiP window on a display of another rate (the begin-frame timer follows the first display's rate)
  - [ ] the message pump was woken 33 to 55 ms late after each run that spent CEF's 10 ms slice (fixed, measurements.md "Why the pump woke late"): repeat the frame runs on the Mac, with Low Power Mode on and off, both pacings
  - [ ] a still page was redrawn on the Mac (86 renders in 10 s, all whole-window scene frames, in two runs of about 1 s and 0.4 s from 2.5 s into the recording, 4 wakes of the scene): `--measure idle` now names each wake of the scene (`scene_wake_*`); repeat it there without touching the pointer, and once with the pointer resting over the window
  - [ ] resize holds the main process at 87 % with 428 stale frames in 10 s; typing shows a key 53 ms later (p95 88 ms): measure again after the three above
  - [ ] measure tab switches on the Mac: `--measure tabs` (smokes.md), both pacings; the tester's first complaint
- [ ] 3. A 60 fps VP9 video in a background tab drops 1.3 % of its frames, more on a tab switch (the one clip macOS decodes in hardware). Measure first whether the drops are decode or presentation (`video.getVideoPlaybackQuality()` against the GPU process's samples). Suspects: a hidden view still holds its frame targets and competes for the GPU, the switch re-creates a surface, the hidden page's frame clock is throttled unevenly. Tied to "Hidden tabs free their frame targets" below and the CEF patch "hide a view's compositor on `Hide()`" (M6)
- [ ] 4. The main process keeps about 330 MB after every tab closes (850 MB at start, 1.18 GB after). Tell a cache from a leak: repeat open-10/close-10 five times and watch whether the footprint keeps rising or levels off; then a per-allocation breakdown (`heap`/`vmmap` on the Mac, "Memory breakdown" below). Suspects: Skia's resource cache (256 MiB budget, "Skia cache hygiene" in M6), imported page textures or their wgpu bind groups not dropped with the view, Slint image caches for favicons and snapshots, closed tabs' history and snapshots kept for reopen
- [ ] 5. The GPU stays 60 to 78 % busy during video: the same cause as 1; re-measure after each step of 1 and stop when it follows the video's own area and rate
- [ ] An inset shadow (`inner-shadow-*`) at a fractional device position: its anti-aliased edge column comes out up to 5 levels different when a partial frame's clip covers it than in a whole frame, and stays until the item is redrawn (Settings' section pill, the address field; gallery 69 in 2 of 6 runs). Snapping the shadow image to the pixel grid removes it but moves the shadow's edges by up to 3 levels against upstream: the owner's call (measurements.md "Pixel questions")
- [ ] Chromium's GPU process exits (133, a trap) right after the gallery's agent scenes 12 and 13 end their session (`agents_stop`) in most runs, on main too; when it happens with the page alive, that page lays out again with smaller body text until it is reloaded (`42-toast-question`; reproduced on main by killing the GPU process)
- [ ] macOS: the window is transparent (`background: transparent`, so the Metal layer is not opaque) and WindowServer blends the whole window on every present; an opaque window where the shell covers it all, after a look on the Mac
- [ ] Profile again with kernel time (needs root for `perf`): GPU driver calls and present, about a third of the main process's CPU, unseen so far
- [ ] The CEF message pump on the main thread (14 % during video): what runs there per frame
- [ ] Find the Speedometer gap (16 % behind Chromium): run it in plain CEF with our `libcef.so`, no shell
- [ ] CEF build with PGO and ThinLTO (`scripts/build-cef.sh` sets `chrome_pgo_phase=0 use_thin_lto=false`), if the plain-CEF run is also behind
- [ ] Memory breakdown per allocation in the main and GPU processes, before any memory change
- [ ] Hidden tabs free their frame targets; the blur pyramid exists only while glass is open; revisit Skia's 256 MiB layer budget
- [ ] A page frame touches the scene and the blur pyramid only while overlay glass or motion is on screen (1.3 % today: low value)
- [ ] Re-run the benchmarks, the flame graph and the video CPU/RAM profile against Chromium and Firefox after each step (`scripts/bench/`; the Speedometer and MotionMark checkouts are not kept: fetch them into a scratch directory)

## Open decisions (owner)
- Mac heat (measure first with `scripts/bench/mac-energy.sh`): an opaque window on macOS, a lower rate for the start page's liquid
- PR #1's defaults as merged: `install.sh` installs on the sha256 alone without `minisign` (a warning), page Escape, sentence-case labels, agent-tab refusals, the shortcut reset
- PR #2's choices as merged: compact mode is the sidebar toggle; the autofill key chip at the page's top left
- Attach FFmpeg's source tarball to each release, or keep the link in the notes
- The inset-shadow edge under partial rendering: snap the shadow to the pixel grid (moves its edges up to 3 levels) or leave it
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

  - [ ] `E.hover-cursor FAIL: cursor 0` once in 10 parity runs with `--begin-frames` on Linux (2026-11-21), 0 in 8 repeats of the E items in either mode: watch for it; cause unknown
