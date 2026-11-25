# 01 Optimization: under 5 % CPU, then memory and battery

Items of this area that need an engine rebuild: [engine-build.md](engine-build.md#optimization).

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

## Engine build and measuring
The engine patches and the one engine build: [engine-build.md](engine-build.md). The own engine (the old milestone M6) rides on it; its other items are shelved in [own-engine.md](../shelved/own-engine.md).

Measure with [scripts/bench/](../../scripts/bench/README.md) (`profile.sh`, `breakdown.py`, `runner.py`) so every step has a
before and an after on the same build; the numbers to beat are in [docs/measurements.md](../../docs/measurements.md). Measured by
hand only: nothing in `cargo test` or CI asserts a timing.

## The goal (owner, 2026-11-23)
The main process under 5 % CPU in steady states (idle, video, scrolling, typing); page loads and animations may spike.
Measured: 26 to 31 % for windowed video on the Mac; on Linux 8 to 9 % for video or a spinner at 60 Hz, 14 % on the
300 Hz display, 1 to 2 % idle. Roads, in this order (decisions.md "Main-process target", "Page roads"):

- 1. Layer: windowed: shipped in 0.6.2, on by default on macOS. Measured on the owner's M2 with windowed 1080p60 video: 10 to 15 % main on its layer (off: 28 to 37 %), the page staying on its layer through overlays. Follow-ups: [docs/research/page-layer.md](../../docs/research/page-layer.md)
  Done: the owner's 18 hand checks of the default-on layer passed (2026-11-25, `docs/research/page-layer.md` "Default on macOS (0.6.2)"). Its open follow-ups:
  - [ ] `set_page_fullscreen`'s window ownership; a fullscreen tab moved to another window leaves its old window in page fullscreen until the page's own exit (no two-window smoke)
- [ ] Measure first what the remaining 10 to 15 % is made of, on the 0.6.2 build with the page on its layer (frame receipt, the pool copy, the layer commit, the begin-frame timer, the accessibility mirror); every road below is judged against it
- Dropped video frames on the layer, shelved for 0.6.2: [video-dropped-frames.md](../shelved/video-dropped-frames.md)
- HDR through the OS layer, and the idea that the page never leaves the OS layer (to be decided before the CEF patch of road 3 is built): [engine-build.md](../shelved/engine-build.md)

## Then (owner, 2026-11-23)
- [ ] Memory under a target, low-hanging fruit first ("inactive tabs holding memory etc"): the main process measured at about 740 to 800 MB; the breakdown items below come first
- [ ] Battery and heat on the Mac: measure first (`scripts/bench/mac-energy.sh`); the open Mac-heat choices are in [decisions.md](../in-progress/decisions.md)
- [ ] Linux counterpart of the page layer, a short feasibility look, not a build: "see if we can copy the surfacing of macOS; if it needs a lot of effort then ignore Linux for now"

## Performance pass 3 (owner, 2026-11-22)
Approved by the owner on 2026-11-22 (all six, the core fork and the faked glass included); the first three are in
`0.6.1` with PR #4's performance work (scene dirty rects, the blur's reach, idle timers, cached bind groups); each still
wants its own measurement. The page on a compositor surface is now Layer: windowed above.
- [ ] Measure each perf-pass-3 commit alone: `--measure idle` (pump runs a second), the spinner and caret pages on the 300 Hz display (CEF frames a second), `scripts/bench/profile.sh`
- [ ] Quiet pump (written, `pump::Tempo::Quiet`): 250 ms between runs once nothing painted for a second, not 33 ms. Left: check nothing waits on the pump as a clock (`Host::show_frame` runs after each pump; start-up, downloads, an agent's page); the begin-frame driver's own 33 ms idle timer on macOS; then try no ceiling at all
- [ ] Profile-guided build (written, `scripts/bench/pgo.sh`, never run): compare with the plain `dist` build, then decide whether releases use it
- [ ] Fork `i-slint-core` (vendored beside the Skia renderer): skip unchanged sub-trees in the render walk, then in the dirty-region walk (renderer-roads.md section 6, changes 1 and 2; up to 30 % of the main thread during page frames)

## From the owner's Mac run of preview.5 (2026-11-21)
Numbers behind it: [docs/measurements.md](../../docs/measurements.md) "Browser benchmarks", "Main-process flame graph" (2026-11-20) and
"The owner's Mac, preview.5". Target: the shell's cost follows what is visible, not "match Chrome". Five findings, in the order
to take them; each starts with a measurement on the Mac that names the cause before any code changes (`sample` on the main
process, the `--measure` counters, `scripts/bench/mac-energy.sh`), because the causes below are suspects, not findings.
- [ ] 1. One video costs the main process about 26 % and the GPU 60 to 78 % (5K display, 120 Hz). Suspect: the window is drawn per page frame at the display's rate. Steps, each measured on its own: (a) count CEF frames, Slint renders and presents per second during a 60 fps video on the Mac (`--measure video` there): is it 60 or 120; (b) CEF's paint rate follows the content (written: "Frame rate by content" above: video at its own rate, a still page none); (c) the opaque window on macOS: the window is transparent (`background: transparent`, so the Metal layer is not opaque) and WindowServer blends the whole 5K window on every present; an opaque window where the shell covers it all, after a look on the Mac; (d) fullscreen video is Layer: fullscreen above; (e) what is left is the second item walk (the i-slint-core fork decision)
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
- [ ] An inset shadow (`inner-shadow-*`) at a fractional device position: its anti-aliased edge column comes out up to 5 levels different when a partial frame's clip covers it than in a whole frame, and stays until the item is redrawn (Settings' section pill, the address field; gallery 69 in 2 of 6 runs). Snapping the shadow image to the pixel grid removes it but moves the shadow's edges by up to 3 levels against upstream: the owner's call (measurements.md "Pixel questions"; [decisions.md](../in-progress/decisions.md))
- [ ] Chromium's GPU process exits (133, a trap) right after the gallery's agent scenes 12 and 13 end their session (`agents_stop`) in most runs, on main too; when it happens with the page alive, that page lays out again with smaller body text until it is reloaded (`42-toast-question`; reproduced on main by killing the GPU process)
- [ ] Profile again with kernel time (needs root for `perf`): GPU driver calls and present, about a third of the main process's CPU, unseen so far
- [ ] The CEF message pump on the main thread (14 % during video): what runs there per frame
- [ ] Find the Speedometer gap (16 % behind Chromium): run it in plain CEF with our `libcef.so`, no shell
- [ ] CEF build with PGO and ThinLTO (`scripts/build-cef.sh` sets `chrome_pgo_phase=0 use_thin_lto=false`), if the plain-CEF run is also behind
- [ ] Memory breakdown per allocation in the main and GPU processes, before any memory change
- [ ] A page frame touches the scene and the blur pyramid only while overlay glass or motion is on screen (1.3 % today: low value)
- [ ] Re-run the benchmarks, the flame graph and the video CPU/RAM profile against Chromium and Firefox after each step (`scripts/bench/`; the Speedometer and MotionMark checkouts are not kept: fetch them into a scratch directory)

## Performance pass (older, open)
- [ ] macOS memory: a Wikipedia article in wubble and in Chrome alone, at the console
- [ ] Browser GPU memory: the unattributed +20 MiB from 1 to 10 tabs; re-measure on macOS
- [ ] glibc heap after a mass discard (28 MiB): decide with hardening whether to `malloc_trim`
- [ ] Video under load: the real browser in its scope, hardware decode under load, the decoder CEF picks on macOS
- [ ] macOS idle with the still backdrop, video figures, occluded pausing by hand
- [ ] CPU and GPU during video on Linux

## From the old milestones and the MCP list
- [ ] Spring spike: a per-frame spring (k 300 / c 15) with no missed frames while a page scrolls (M0)
- [ ] Re-measure `--begin-frames` on a 120 Hz (and the 300 Hz) display on Linux: it was left off there in b2 for a worse video render interval at 240 Hz (M0)
- [ ] Benchmark against Chrome and Zen: 200 tabs and after 1 h idle (Chromium and Firefox at 1/10/50 done) (M3)
- [ ] Agent views keep rendering while no window shows them (a hidden view has no accessibility tree and no screenshot): find a cheaper way (MCP)

## Build-out features (2026-11-22)

## Agent views no window shows render at a trickle, at full rate only when asked (`agent-hidden-view-throttle`)

Key: `agent-hidden-view-throttle` · engine patch: no · state: not written.

TODO line 85. An agent session's tabs stay "visible" for CEF even when no window shows them, because a hidden view has no accessibility tree and no screenshot (crates/wubble/src/host/agent.rs ~1999 and ~5136). So they paint at the full windowless frame rate. Keep them unhidden, but drop the rate. When an agent tab is not in any window's panes, call `View::set_frame_rate(1)` (CEF `set_windowless_frame_rate`, crates/wubble-engine/src/view.rs ~876). Raise it to the pane rate (`wubble_gfx::pacing::frame_rate_for`) for the length of any call that needs fresh pixels (`screenshot`, `capture`, a shot-based measure) or a fresh layout, then lower it again when the call ends. Also skip importing their frames into the GPU texture path (`wubble-gfx` import) while unshown, if the import runs per paint. Put the rule as a pure function in wubble-core (shown, call in flight, headless → rate) with unit tests, executed by the host. No timing tests.

## macOS: opaque window where the shell covers it (`macos-opaque-window`)

Key: `macos-opaque-window` · engine patch: no · state: not written.

Chrome's macOS windows are opaque, so WindowServer does not blend them. wubble's root window has `background: transparent` (crates/wubble-ui/ui/app.slint:99), so its CAMetalLayer is not opaque. On a 5K display WindowServer then blends the whole window on every present (TODO Optimization 1c and the macOS transparency item). On macOS the window keeps AppKit's frame (`native-frame`, window.rs `NATIVE_FRAME`) and the shell covers every pixel, so nothing needs the transparency.

To build: (1) In app.slint, bind the window background to an opaque ground when `native-frame` is true. To obey the kit rules, the shell file must not hold the token: add a kit property (for example a `WindowGround` value in kit/derived.slint, recorded in docs/fidelity/derived-review.md) that resolves to the scene's ground colour, and keep it transparent elsewhere. (2) In the macOS window setup (crates/wubble/src/host/window.rs, plus crates/wubble-engine/src/mac_app.rs or a new mac helper using objc2 via the raw window handle), set `NSWindow.opaque = YES` and `backgroundColor`, and set `CAMetalLayer.opaque = YES` on the content view's layer. Metal advertises only `PostMultiplied` alpha, so `composite_alpha_mode` in vendor/i-slint-renderer-skia/wgpu_30_surface.rs finds no Opaque mode; making the layer itself opaque is what counts. If the vendored surface needs any change, mark it `wubble change` in WUBBLE_CHANGES.md. (3) Keep the scene's backdrop texture opaque under the sidebar so nothing reads alpha. Put it behind a flag such as `WUBBLE_MAC_OPAQUE=0` for A/B runs. macOS only.

## Page fullscreen draws the page alone (`fullscreen-page-alone`)

Key: `fullscreen-page-alone` · engine patch: no · state: not written.

In Chrome, fullscreen video is the page's compositor frame alone. In wubble, page fullscreen already hides the sidebar, frame and seam, and the scene does not step (decisions "Page fullscreen", "Scene follows the run state"). Slint still walks the shell's item tree, the scene's textures and over-layer images stay allocated and bound, and each page frame goes through the full render (TODO Optimization 1d).

To build: in crates/wubble-ui/ui/app.slint, put every shell subtree (sidebar, bars, overlays that are not the page's own: find, prompts and video controls stay) behind `if !root.page-fullscreen` conditionals. The shell file only lays out kit components, so no new visuals are needed. Then only the page `Image` and the page-owned surfaces remain. In crates/wubble/src/backdrop.rs, when `get_page_fullscreen()` turns on, release the scene's output and over-layer images (`set_scene`/`set_over_*` to an empty image) and the page pyramid P (`Renderer::set_pages(vec![])` path in crates/wubble-gfx/src/scene/render.rs). Rebuild them when fullscreen ends, the same way a resize does. In host/panes.rs, a fullscreen pane always takes the in-place damage path (`FramePresenter::present_damaged`), so only the video's rect is marked dirty. Count `fullscreen_frames` in `--measure` so it can be compared with Chromium. All platforms.

## Scene renders mark only what the scene shows through (`scene-damage-shows-through`)

Key: `scene-damage-shows-through` · engine patch: no · state: not written.

A scene render (ambience, about 8 to 10 per second during video and scroll) currently returns `Damage::Whole` unless only the start page's card changed (crates/wubble-gfx/src/scene/start.rs `Damage`; crates/wubble/src/backdrop.rs `frame` calls `partial::mark_window`). Slint then redraws the whole window, including the page card, which covers the scene.

To build: extend `Damage` with a region variant, for example `Rects(SmallVec<[Rect; 4]>)`, and make `Renderer::before_rendering` (scene/render.rs) compute it. When only the window-glass ambience was rebuilt, the damage is the window's glass surfaces where they do not lie under an opaque page card. Use `pages::by_layer`, `rect_of` and the `PageRect`s already held; the visible scene is the sidebar, the gaps around the card and any transparent pane (split, bowed edge, start). When over-layer glass was redrawn, the damage is those surfaces' rects. Use Whole only for a palette change, a resize, new textures or a backdrop that moved under everything. Add `Damage::and` merging for the new variant. In backdrop.rs, call `partial::mark_rect` once per rect; Slint's dirty region merges them. Pure unit tests in start.rs and pages.rs for which rects come out, and a `partial_render` test in wubble-ui for a scene frame that does not redraw the page. All platforms.

## Page damage rebuilds the page blur only within the ambience's reach (`ambience-damage-reach`)

Key: `ambience-damage-reach` · engine patch: no · state: not written.

Damage far from any glass, such as a spinner in the middle of the page, still marks P (the page-inclusive blur pyramid) stale. The ambience then rebuilds at 8 Hz, each time as a whole-window frame. This item also covers the TODO item that a page frame should touch the scene and pyramid only while overlay glass or motion is on screen.

To build: in crates/wubble-gfx/src/scene/pages.rs, add an `AMBIENCE_REACH` (about 130 logical px from a pane edge, from the ambience's blur radius) and a pure `fn reaches(damage: Rect, page: &PageRect, overlays: &[Rect]) -> bool`. It is true when the damage lies within the reach band of the pane's edge, or under an overlay-glass rect. Change `Renderer::pages_written` and `set_pages` in scene/render.rs to take the frame's damage: the in-place `Damage` from `FramePresenter::present_damaged` (crates/wubble-gfx/src/import.rs), where a whole frame counts as reaching. Set `pages_stale` only when it reaches. Pass that damage from crates/wubble/src/host/panes.rs (around line 839, next to `mark_rect`) through `Glass::pages_written` in backdrop.rs. In `page_blur`, a stale page with only the ambience on and no overlay keeps its low rate. With no overlay glass over a pane and the ambience at 0, the host skips `set_pages`/`schedule_pages` for page frames altogether. Unit tests in pages.rs. All platforms.

## A popover's entrance dirties only its own region, so partial rendering helps (`popover-entrance-partial`)

Key: `popover-entrance-partial` · engine patch: no · state: not written.

TODO line 140. During a popover's entrance, the glass's scene scale and alpha change each frame, and that marks the whole window dirty (`wubble-gfx` scene `state.rs`/`render.rs`, the partial renderer in `vendor/i-slint-renderer-skia`, the glass in `wubble_core::glass`). Make the glass layer's damage the union of its old and new bounds (scaled rect plus blur radius), not the window. Find where a change in a glass region's transform or opacity calls the full-window invalidate (probably the scene's backdrop or blur pass assuming a global change), and report the region instead. Check that the backdrop blur under the popover samples only the area it needs. After the change the owner measures (`--measure idle --seconds 4` must report 0 to 2 `slint_renders`, per AGENTS.md); do not run it yourself. No timing tests.

## Recover page layout after a GPU process restart (`gpu-restart-recovery`)

Key: `gpu-restart-recovery` · engine patch: no · state: not written.

After Chromium's GPU process exits (seen as a trap, exit 133, right after the gallery's agent scenes call `agents_stop`) and restarts, a page that stays alive lays out again with smaller body text until it is reloaded. In Chrome the renderer gets its screen info again and keeps its scale. wubble's watcher (crates/wubble/src/host/gpu.rs, `gpu_looked`) only counts the restart and calls `View::repaint()` (crates/wubble-engine/src/view.rs:1280, `request_repaint`).

To build: (1) On a detected restart, also call CEF `BrowserHost::notify_screen_info_changed()` and `was_resized()` for every view. Add `View::refresh_screen()` in view.rs, using welding's existing host access (`self.send(|host| ...)`), so Chromium asks `GetScreenInfo`/`GetViewRect` again and gets the right device scale factor. Then call `repaint`. Make sure welding's render handler answers `get_screen_info` from the current metrics (vendor/welding/src/linux_cef, macos_cef), not a cached default. Mark any welding change `wubble change`. (2) Make the trap easier to find: log the GPU process's exit status when the watcher sees the pid vanish (wubble_sysmem), and under `WUBBLE_TRACE=1` log the agent teardown steps that ran just before (host/agent.rs `Effect::Release`/teardown order: closing DevTools sessions and views). If teardown closes a DevTools session after its view is gone, reorder it to close sessions first. Keep `wubble_core::gpu::watch` pure. Linux and macOS.

## Later (owner, 2026-11-25: "later, non blocking")
- [ ] The Mac compared with Safari and Chrome, maybe Zen, through the Mac harness
