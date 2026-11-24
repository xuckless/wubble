# Performance and the page layer

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

Measure with [scripts/bench/](../../scripts/bench/README.md) (`profile.sh`, `breakdown.py`, `runner.py`) so every step has a
before and an after on the same build; the numbers to beat are in [docs/measurements.md](../measurements.md). Measured by
hand only: nothing in `cargo test` or CI asserts a timing.

## The goal (owner, 2026-11-23)
The main process under 5 % CPU in steady states (idle, video, scrolling, typing); page loads and animations may spike.
Measured: 26 to 31 % for windowed video on the Mac; on Linux 8 to 9 % for video or a spinner at 60 Hz, 14 % on the
300 Hz display, 1 to 2 % idle. Roads, in this order (decisions.md "Main-process target", "Page roads"):

- [~] 1. Layer: windowed (`--page-layer=window`): the page's layer below the shell, seen through a kit `PageHole`; glass over a page falls back to the shell's composited path while it overlaps. On branch `untested/page-layer-2`: passes everything on Linux; macOS compile and first run pending, then the owner's hand-check list (`docs/research/page-layer.md` on that branch). Expected about 12 % for windowed pages. Supersedes the older plan of each pane's page on its own `wl_subsurface` (macOS: a `CALayer` on the IOSurface) with glass faked from a low-rate sample of the page; the prototype was to be measured first
- [ ] 2. Zero-copy of CEF's IOSurface (Linux: dmabuf) and CEF's own begin-frame source (estimated 6 to 10 %). The copy, by a CEF patch: OSR keeps the frame's buffer until wubble releases it and delivers RGBA; wubble imports each pooled dmabuf (IOSurface) once and samples it, so the copy and its wait in the paint callback (`vendor/welding/src/native_frame/mod.rs`), the swizzle blit and the per-frame import go
- [ ] 3. A CEF patch so the GPU process presents the page itself through a CAContext id / `CALayerHost` while off-screen mode stays (estimated 2 to 5 % main; the GPU process also loses its capture copy): about 300 lines in CEF, private API, no Wayland counterpart; three experiments listed in `docs/research/page-layer.md`
- [ ] Check the unverified estimate that the accessibility mirror costs 3 to 8 % of the main process (item 2 under "From the owner's Mac run of preview.5" below)

## Layer: fullscreen
On `store/lane` behind `--page-layer=fullscreen` (off by default), tested; the owner measured it on an M2 during
fullscreen 60 fps video: main process 26 % -> 12 %, picture smooth and correct.
- [ ] `set_page_fullscreen`'s window ownership; a fullscreen tab moved to another window leaves its old window in page fullscreen until the page's own exit (no two-window smoke)

## Shelved (owner)
- [ ] A scroll lag seen once on the store test build on macOS (trackpad, 120 Hz, no flag), not reproducible; a read of the diff found no cause. First check when it returns: `--no-begin-frames`, and the build and launch differences of the unsigned test app

## Performance pass 3 (owner, 2026-11-22)
Approved by the owner on 2026-11-22 (all six, the core fork and the faked glass included); the first three are in
`0.6.1` with PR #4's performance work (scene dirty rects, the blur's reach, idle timers, cached bind groups); each still
wants its own measurement. The page on a compositor surface is now Layer: windowed above.
- [ ] Measure each perf-pass-3 commit alone: `--measure idle` (pump runs a second), the spinner and caret pages on the 300 Hz display (CEF frames a second), `scripts/bench/profile.sh`
- [ ] Frame rate by content (written, `pacing::ContentRate`): a page with small damage and no input for 300 ms paints at 60 Hz at most. Left: the shell's begin frames (macOS) are not capped, so a spinner there still paints at 120 Hz; a pop-up view takes its opener's ceiling; look at a small animation at 60 on the 300 Hz display by eye
- [ ] Quiet pump (written, `pump::Tempo::Quiet`): 250 ms between runs once nothing painted for a second, not 33 ms. Left: check nothing waits on the pump as a clock (`Host::show_frame` runs after each pump; start-up, downloads, an agent's page); the begin-frame driver's own 33 ms idle timer on macOS; then try no ceiling at all
- [ ] Profile-guided build (written, `scripts/bench/pgo.sh`, never run): compare with the plain `dist` build, then decide whether releases use it
- [ ] Fork `i-slint-core` (vendored beside the Skia renderer): skip unchanged sub-trees in the render walk, then in the dirty-region walk (renderer-roads.md section 6, changes 1 and 2; up to 30 % of the main thread during page frames)

## From the owner's Mac run of preview.5 (2026-11-21)
Numbers behind it: [docs/measurements.md](../measurements.md) "Browser benchmarks", "Main-process flame graph" (2026-11-20) and
"The owner's Mac, preview.5". Target: the shell's cost follows what is visible, not "match Chrome". Five findings, in the order
to take them; each starts with a measurement on the Mac that names the cause before any code changes (`sample` on the main
process, the `--measure` counters, `scripts/bench/mac-energy.sh`), because the causes below are suspects, not findings.
- [ ] 1. One video costs the main process about 26 % and the GPU 60 to 78 % (5K display, 120 Hz). Suspect: the window is drawn per page frame at the display's rate. Steps, each measured on its own: (a) count CEF frames, Slint renders and presents per second during a 60 fps video on the Mac (`--measure video` there): is it 60 or 120; (b) CEF's paint rate follows the content (written: "Frame rate by content" above: video at its own rate, a still page none); (c) the opaque window on macOS: the window is transparent (`background: transparent`, so the Metal layer is not opaque) and WindowServer blends the whole 5K window on every present; an opaque window where the shell covers it all, after a look on the Mac; (d) fullscreen video is Layer: fullscreen above; (e) what is left is the second item walk (the i-slint-core fork decision)
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
- [ ] An inset shadow (`inner-shadow-*`) at a fractional device position: its anti-aliased edge column comes out up to 5 levels different when a partial frame's clip covers it than in a whole frame, and stays until the item is redrawn (Settings' section pill, the address field; gallery 69 in 2 of 6 runs). Snapping the shadow image to the pixel grid removes it but moves the shadow's edges by up to 3 levels against upstream: the owner's call (measurements.md "Pixel questions"; TODO.md "Open decisions")
- [ ] Chromium's GPU process exits (133, a trap) right after the gallery's agent scenes 12 and 13 end their session (`agents_stop`) in most runs, on main too; when it happens with the page alive, that page lays out again with smaller body text until it is reloaded (`42-toast-question`; reproduced on main by killing the GPU process)
- [ ] Profile again with kernel time (needs root for `perf`): GPU driver calls and present, about a third of the main process's CPU, unseen so far
- [ ] The CEF message pump on the main thread (14 % during video): what runs there per frame
- [ ] Find the Speedometer gap (16 % behind Chromium): run it in plain CEF with our `libcef.so`, no shell
- [ ] CEF build with PGO and ThinLTO (`scripts/build-cef.sh` sets `chrome_pgo_phase=0 use_thin_lto=false`), if the plain-CEF run is also behind
- [ ] Memory breakdown per allocation in the main and GPU processes, before any memory change
- [ ] Hidden tabs free their frame targets; the blur pyramid exists only while glass is open; revisit Skia's 256 MiB layer budget
- [ ] A page frame touches the scene and the blur pyramid only while overlay glass or motion is on screen (1.3 % today: low value)
- [ ] Re-run the benchmarks, the flame graph and the video CPU/RAM profile against Chromium and Firefox after each step (`scripts/bench/`; the Speedometer and MotionMark checkouts are not kept: fetch them into a scratch directory)

## Performance pass (older, open)
- [ ] macOS memory: a Wikipedia article in wubble and in Chrome alone, at the console
- [ ] Browser GPU memory: the unattributed +20 MiB from 1 to 10 tabs; re-measure on macOS
- [ ] glibc heap after a mass discard (28 MiB): decide with hardening whether to `malloc_trim`
- [ ] Video under load: the real browser in its scope, hardware decode under load, the decoder CEF picks on macOS
- [ ] macOS idle with the still backdrop, video figures, occluded pausing by hand
- [ ] CPU and GPU during video on Linux
