# Measured

Status: moved from TODO.md on 2026-11-19 as it stood; numbers are hand runs on the owner's Linux box and Mac, never asserted in tests. Newer runs add rows here.
Decisions drawn from these numbers are in [decisions.md](decisions.md).

2026-11-18, RTX 5060 / NVIDIA 615.71.09 / Wayland (GNOME), display 300 Hz (CEF capped at 240), window 1280x800 @1, debug build, 10 s per run; other builds running (load average 2.6 to 6.6). Full tables: lane notes (not kept in the repo). Milliseconds, p50 / p95 / p99. The paint interval is between the paints of frames the host took; a coalesced paint is skipped, so its interval reads double.

| Scenario | taken-frame paint interval | copy wait in paint callback | paint to render | render interval | other |
|---|---|---|---|---|---|
| scroll | 4.16 / 5.73 / 11.14 | 0.03 / 0.09 / 0.17 | 0.97 / 1.77 / 3.06 | 4.16 / 5.22 / 12.81 | 4 % of CEF frames coalesced |
| video | 4.16 / 6.25 / 8.74 | 0.03 / 0.08 / 0.16 | 1.04 / 2.64 / 3.60 | 4.14 / 6.26 / 11.22 | with `--begin-frames`: render 5.80 / 7.84 / 15.15 |
| typing | 4.16 / 6.26 / 12.63 | 0.03 / 0.12 / 0.19 | 1.11 / 2.34 / 3.94 | 4.14 / 6.38 / 22.88 | key to pixels 9.1 / 13.7, click 8.9 / 13.8 (25 each) |
| resize | 8.23 / 12.51 / 12.85 | 0.04 / 0.07 / 0.18 | 1.91 / 4.39 / 4.79 | 10.65 / 32.21 / 42.05 | 551 of 1297 frames stale, longest run 8 |
| resize, stretchy (b5) | 8.27 / 12.76 / 13.17 | 0.04 / 0.08 / 0.17 | 2.63 / 5.03 / 5.53 | 3.93 / 32.75 / 42.77 | 521 of 1229 frames stale (shown stretched), longest run 4; 489 resize requests; renders 1003; load 3.0. The render interval's p50 drops because the panes' stretch asks for frames; p95 is still the surface reconfigure per size |
| resize, before b6 (base 8ec9e5c) | 11.01 / 12.98 / 14.87 | 0.05 / 0.25 / 0.31 | 3.54 / 5.39 / 6.02 | 22.79 / 85.82 / 121.71 | 567 of 1010 frames stale, longest run 14; 461 resize requests; renders 359; load 7.4 (other lanes building: the render interval is not comparable with the b5 row) |
| resize, bowed edges (b6) | 10.51 / 13.02 / 14.68 | 0.05 / 0.22 / 0.29 | 3.70 / 5.28 / 6.34 | 13.82 / 75.67 / 263.43 | 559 of 1017 frames stale, longest run 19; 467 resize requests; renders 370; load 5.4. Shell frames 370: scene off the window size 0, panes off the content width 0, a pane stretched 369, a pane masked 0 (the sweep moves no seam). Same as before within the noise of the load; observations, not asserted |
| resize, b6 review fixes | 8.75 / 12.92 / 14.18 | 0.04 / 0.09 / 0.18 | 2.01 / 4.04 / 4.86 | 12.78 / 73.94 / 107.37 | 530 of 1059 frames stale, longest run 10; 462 resize requests; renders 468; load 3.7. Shell frames 467: scene off 0, panes off 0, a pane stretched past filling it (scale > 1, the stricter count now) 464, masked 0 (no seam moved). The stretch now starts from rest (a dt-0 frame whose size changed keeps the panes' clock); observations, not asserted |

2026-11-18, macOS: Apple M2 Pro, macOS 27, 120 Hz at 2x, window 2560x1600 physical, debug build (opt-level 1), `--measure scroll --seconds 5` through the owner's job runner. Milliseconds, p50 / p95 / p99.

| Run | render (present) interval | copy wait in paint callback | pump work | scene CPU | Slint render | other |
|---|---|---|---|---|---|---|
| before (main at 4a7bb5e), 3 of 4 runs (one shown) | none | 1.54 / 2.35 / 2.99 | 0.05 / 4.08 / 10.10 | none | none | `renders=0`: CEF painted 602 frames at 8.31 ms, Slint never rendered (pump starvation, Decided "macOS pump delivery") |
| before, the 1 run that rendered | 8.35 / 9.63 / 16.55 | 1.20 / 4.78 / 5.54 | 1.18 / 5.22 / 10.10 | 1.70 / 1.85 / 1.92 | 2.65 / 2.86 / 2.99 | 23 missed vsyncs |
| pump on GCD only | 8.39 / 11.87 / 15.94 | 2.76 / 5.07 / 7.53 | 1.31 / 5.53 / 10.10 | 1.73 / 1.93 / 2.04 | 2.74 / 3.01 / 3.16 | 28 missed vsyncs |
| + copy on its own Metal queue (fb/mac) | 8.31 / 9.18 / 14.21 | 1.21 / 3.17 / 3.35 | 0.02 / 3.58 / 10.26 | 1.68 / 1.95 / 2.02 | 2.63 / 2.91 / 3.00 | 18 missed vsyncs, 590 of 602 frames shown; another run 8.32 / 9.44 / 13.39, copy wait 1.31 / 2.87 / 3.43, 20 missed |
| batch eight (b8/plat), 2026-11-18 | 8.34 / 11.66 / 15.24 | 1.36 / 2.83 / 3.25 | 0.03 / 3.11 / 10.25 | 1.69 / 1.98 / 2.06 | 2.77 / 3.10 / 3.22 | taken paint interval 8.34 / 10.55 / 14.87 (the earlier 8.31 median: same within noise); 585 of 601 frames shown, 16 coalesced, 25 missed vsyncs; the shared Mac had other lanes' builds running |

The copy wait is still the largest single block on the UI thread (a separate queue still waits for the GPU to take the blit); the scene's CPU side (1.7 ms) and femtovg (about 1 ms) are debug-build costs not measured in release.

2026-11-19, b9 (idle, start-up, input, profile), release builds. Linux: RTX 5060 / NVIDIA 615.71.09 / Wayland (GNOME), 300 Hz, window 1280x800 @1, other lanes building (load average 2 to 18, given per row). Mac: M2 Pro, 120 Hz, 2560x1600 @2, through the job runner. Raw output: lane notes (not kept in the repo). Percent is of one core over the window; "renders" is Slint renders a second (the scene renders with each).

| `--measure idle` | renders/s | scene CPU / Slint render p50 (ms) | browser process | other processes | GPU (Linux: whole GPU) |
|---|---|---|---|---|---|
| Linux, before (main + tools), 10 s, load 5.5 | 320.4 | 0.23 / 0.61 | 30.3 % | 0.5 % | 17.6 % busy, 36.3 W |
| Linux, scene image set once, 10 s, load 12.7 | 289.9 | 0.31 / 0.78 | 40.0 % | 0.5 % | 29.7 %, 46.9 W |
| Linux, + pane rows by texture, 10 s, load 17.9 | 16.5 | 1.10 / 2.30 | 8.5 % | 0.9 % | 9.6 %, 31.7 W (desktop alone then: 9.0 %, 31.1 W) |
| Linux, + GPU watch, 20 s, load 10.6 | 15.9 | 0.35 / 0.85 | 3.6 % | 0.5 % | 3.7 %, 13.9 W |
| Linux, same, 8 Hz idle tick (not kept), 20 s, load 4.3 | 8.9 | 0.35 / 0.84 | 2.2 % | 0.4 % | 1.6 %, 16.3 W |
| Linux, no scene (`WUBBLE_GLASS=slint`), 20 s | 0.9 | n/a | 1.0 % (0.9 % without the host timers) | 0.4 % | n/a |
| Mac, before (main + tools), 20 s | 117.7 | 1.60 / 2.31 | 76.0 % | 0.4 % (GPU process 0.3 %) | n/a |
| Mac, after, 20 s | 15.9 | 5.26 / 6.43 | 25.5 % | 1.0 % (GPU process 0.6 %) | n/a |

Every idle run: `cef_frames` 0 (Chromium paints nothing for a still page), 33 to 35 pump runs a second, each doing work (0.03 ms p50). The per-frame cost at 15 Hz is two to four times the same frame at full rate on both machines (the CPU clocks down between frames).

2026-11-19, the liquid's freeze (fb/still), Linux release, same machine and window as above, `--measure idle` through `--warmup N` (new: how long after the first frame recording starts; the default 4 s ends before the freeze). The box was loaded by a Chromium build (load average 21 to 29 over every row below, so the percentages are higher than the b9 rows at lighter load: compare only the rows here). "Idle" is the same binary with `still_after` raised to an hour (the liquid never freezes: main's behaviour, 15 Hz drift), 20 s after the default 4 s warm-up; "Still" is the shipped timing, 20 s after a 20 s warm-up (the freeze starts about 5 s after the first quiet frame and takes 2 s: `WUBBLE_TRACE=1` printed `Idle -> Easing` at calm 4.94 s and `Easing -> Still` 1.9 s later, then nothing).

| `--measure idle` | renders/s (Slint / scene) | idle ticks/s | pump runs/s | browser process | GPU (whole, incl. desktop) |
|---|---|---|---|---|---|
| Idle, drifting, run 1, load 21 | 15.3 / 14.9 | 15.0 | 32.4 | 10.3 % | 4.6 %, 15.2 W |
| Idle, drifting, run 2, load 25 | 15.2 / 14.8 | 14.9 | 32.2 | 10.9 % | 5.3 %, 15.2 W |
| Still, run 1, load 24 | 0.4 / 0 | 0 | 48.4 | 2.4 % (all processes 3.2 %) | 4.4 %, 12.7 W |
| Still, run 2, load 22 | 0.4 / 0 | 0 | 32.4 | 2.0 % (all processes 2.8 %) | 4.0 %, 12.7 W |

Still: no scene frame and no idle timer in 20 s; the 8 Slint renders in the window (0.4 a second) are not the scene's (`scene_renders` 0) and were not traced further. The GPU share is dominated by the desktop. The first Still run measured a pump rate of 48 a second against 32 in every other run (CEF's own schedule, not looked into). Two things woke the liquid before they were fixed, found with the trace: the 5 s lifecycle tick changes the models, the glass registry swept and handed the same two items again, and the scene read that as a changed glass list (`GlassRegistry` now hands a list only when it differs from the last one it handed); and the first build counted every step of an Idle state's own calm time as a change of state. Not measured: the Mac (not reachable from this lane); the freeze as seen by eye.

| `--measure startup`, ms from `main` (median of 5 after one cold run) | GPU device | CEF started | window shown | first view | first CEF frame | first page pixels |
|---|---|---|---|---|---|---|
| Linux, before | 153 | 296 | 320 | 377 | 493 | 504 |
| Linux, GPU device on a thread | 180 (parallel) | 156 | 217 | 311 | 387 | 404 |
| Mac, before | 72 | 197 | 226 | 430 | 514 | 519 |
| Mac, after | 171 (parallel) | 171 | 236 | 410 | 495 | 498 |

(Columns are each stage's own median; the store loads in 7 to 70 ms on Linux and 70 to 120 ms on the Mac, a throwaway profile each run; the blocker's lists are in force before the first CEF frame. Process creation to `main`: 10 to 50 ms on Linux.)

| Input, release | key to pixels | click to pixels | scroll present interval | scene CPU / Slint render (scroll) |
|---|---|---|---|---|
| Linux | 9.93 / 13.86 / 13.99 | 10.03 / 14.64 / 49.85 | 3.33 / 4.40 / 11.30 | 0.27 / 0.68 |
| Mac | 14.37 / 31.11 / 31.64 | 13.16 / 22.01 / 24.02 | 8.36 / 9.52 / 15.70 | 1.25 / 1.84 |

(p50 / p95 / p99 ms; 25 keys and 25 clicks; Mac scroll: 26 missed vsyncs in 10 s, browser process 81 % of a core while scrolling.)

## Page load (b9)

2026-11-19, Linux, RTX 5060 / Wayland, release, CEF 156 against Chromium 157.0.8103.78 headless (Fedora build), load average 2 to 4 (other lanes building and measuring; compare only alternated runs). `scripts/measure-load.sh`, 7 processes (7 cold, 35 warm per cell), p50 / p95 ms, in-page times from navigation start. Chromium is `--headless=new` on a fresh profile after a warm-up page, so its frame clock is 60 Hz and wubble's window is 240 Hz: compare DOMContentLoaded and load end, not first paint.

| Page | | Chromium DOMContent / load | wubble no hook | wubble blocker (shipped) | wubble host frame (shipped) |
|---|---|---|---|---|---|
| article | cold | 7.6 / 8.3 | 6.6 / 7.1 | 7.2 / 7.8 | 32.5 |
| article | warm | 6.2 / 6.6 | 5.2 / 5.7 | 6.1 / 6.8 | 19.7 |
| images (30, 1.7 MB) | cold | 7.3 / 16.9 | 6.7 / 18.7 | 8.4 / 21.9 | 22.2 |
| scripts (20 x 30 KB) | cold | 18.0 / 18.3 | 17.1 / 17.4 | not run | |
| 55 000 nodes + 5 000 | cold | 362 / 365 | 355 / 359 | 355 / 359 | 117 |
| 55 000 nodes + 5 000 | warm | 278 / 293 | 293 / 296 | 298 / 301 | 119 |
| 200 blockable requests | cold | 29 / 51 | 41 / 70 | 46 / 50 (all blocked) | 53 |
| 200 blockable requests | warm | 9.5 / 10.2 | 9.4 / 10.5 | 60 / 65 (all blocked), 9.9 / 10.8 with the blocker off | 67 |

Host frame is wubble's request to the first Slint render of a frame taken after the commit; Chromium has no counterpart. Per-request and per-script numbers, the bfcache and cache findings: the Decided rows named under "Performance pass, Page load".

## Memory (b9, 2026-11-18)

`--measure memory`, one static article per tab, each on its own site; MiB. Linux: RTX 5060 / NVIDIA 615.71.09 / Wayland, window 1280x800@1, sum of PSS over the process tree (debug: opt-level 1). macOS: M2 Pro, macOS 27.3, 2560x1600@2, through the owner's job runner, sum of `phys_footprint`. The two quantities are not comparable across OSes. Run to run the Linux totals move by about 30 MiB (mostly file-backed pages shared with other processes); the rows are single runs, raw logs in lane notes (not kept in the repo).

| Build | empty | 1 tab | 10 tabs | 50 tabs | 50, 49 frozen | 50, 49 discarded | processes at 1 / 10 / 50 |
|---|---|---|---|---|---|---|---|
| Linux debug, before (extensions on) | 521 | 734 | 861 | 1348 | 1350 | 828 | 14 / 23 / 63 |
| Linux debug, `disable-extensions` | 523 | 675 | 800 | 1266 | 1267 | 767 | 10 / 19 / 59 |
| Linux release, before (extensions on) | 487 | 695 | 825 | 1309 | 1310 | 787 | 14 / 23 / 63 |
| Linux release, `disable-extensions`, run 1 | 562 | 715 | 841 | 1325 | 1283 | 763 | 10 / 19 / 59 |
| Linux release, `disable-extensions`, run 2 | 534 | 699 | 826 | 1311 | 1312 | 792 | 10 / 19 / 59 |
| macOS debug | 417 | 570 | 1158 | 3172 | 3170 | 851 | 6 / 15 / 55 |
| macOS release | 409 | 561 | 1124 | 3187 | 3180 | 844 | 6 / 15 / 55 |

Per process, Linux release run 2 (browser / GPU / renderers): empty 340 / 80 / -, 1 tab 348 / 166 / 106 (4 processes labelled Renderer: the tab's, the spare and two more forked from the zygote), 10 tabs 353 / 178 / 221, 50 tabs 403 / 190 / 644, discarded 409 / 189 / 108. macOS release (browser / GPU / renderers): 336 / 28 / -, 360 / 93 / 63 (2), 465 / 247 / 365 (11), 659 / 781 / 1686 (51), discarded 624 / 105 / 62. A live tab costs about 12 MiB PSS on Linux and about 33 MiB footprint per renderer plus about 14 MiB in the GPU process on macOS; freezing changes nothing (a frozen renderer keeps its heap; Decided "Tab freezing" is for CPU), discarding gives back the renderer and the GPU process's share.

Linux browser process (release, 1 tab, about 320 to 370 MiB PSS): anonymous 157 to 162 MiB, file-backed 157 to 209 (libcef.so 76, the wubble binary's code 49: Skia, wgpu and Slint are linked in, `.text` 43 MB), heap 65, unnamed anonymous 71, `/dev/nvidiactl` 22. The Rust heap (`--features alloc-count`, debug) is 18.5 MiB live with no tab, 19.1 with one, 24.1 with fifty; its peak is 38.3 MiB, during the blocker's first compile on the throwaway profile. wgpu's allocator: 11.8 MiB allocated (32 reserved) with no tab, 18.7 with a pane's ring, 19.8 with fifty tabs. NVIDIA VRAM charged to the browser process: 67 MiB empty, 68 at 1 tab, 90 at 10, 130 at 50, 130 after the discards (open item in the Performance pass).

Tab-row entrance cap (x2, 2026-11-19, Linux release, same harness, one run each, logs in lane notes (not kept in the repo)): NVIDIA VRAM, MiB.

| Run | empty | 1 tab | 10 tabs | 50 tabs | 50, 49 frozen | 50, 49 discarded |
|---|---|---|---|---|---|---|
| browser, before (every new row enters) | 67 | 68 | 90 | 130 | 130 | 130 |
| browser, after (at most 4 rows enter at once) | 67 | 68 | 88 | 92 | 92 | 92 |
| GPU process, before | 7 | 30 | 70 | 234 | 234 | 24 |
| GPU process, after | 7 | 30 | 70 | 235 | 235 | 28 |

Debug after: browser 67 / 68 / 86 / 90 / 90 / 90. wgpu's allocator is 11.8 / 18.7 / 19.2 / 19.8 MiB in every run. The GPU process's growth (about 4 MiB per hidden live tab at 1x) is unchanged: it needs the CEF patch (M6).

Against other browsers on Linux (`scripts/mem-compare.sh`, same article on `t<n>.localhost`, settle 15 s, 40 s for 50 tabs, PSS sum over the tree; Chromium 157 is a Fedora build, CEF 156 is ours: close, not identical):

| Browser | 1 tab | 10 tabs | 50 tabs | processes at 1 / 10 / 50 |
|---|---|---|---|---|
| wubble release (`disable-extensions`, run 2) | 699 | 826 | 1311 | 10 / 19 / 59 |
| Chromium 157.0.8103.78 | 590 | 673 | 1002 | 17 / 26 / 64 |
| Firefox 160.0 | 904 | 919 | 1563 | 21 / 30 / 64 |

At 10 tabs Chromium splits as browser 189, GPU 142, renderers 255, utilities 46, zygotes 20, broker 22: the renderers match wubble's (221 to 247), the difference is the browser process (wubble 324 to 373: the shell's GPU side, the larger mapped code of libcef and wubble, and the scene's buffers) and wubble's growth per tab in the browser process (see the open item). Chrome and Zen: not installed on Linux; installed on the Mac but not startable over ssh.

## Video (b9)

2026-11-18, video (b9): i7-14700F (28 threads), RTX 5060 / NVIDIA 615.71.09, Fedora 44, GNOME Wayland, CEF 156.0.30, release, windowless probe with a 1280x720 view unless noted, loopback test clips (ffmpeg, 8-bit unless noted), 8 to 10 s windows. Other lanes were building (load average 8 to 35): ratios held across repeats, absolute numbers are noisy. Raw outputs: lane notes (not kept in the repo).

Decode CPU, % of one core (browser + GPU + renderers):

| Clip | decoder | total | browser | GPU | renderer |
|---|---|---|---|---|---|
| 4K60 VP9 | software (libvpx), 3 runs | 132 / 128 / 187 | 6 / 6 / 9 | 16 / 15 / 23 | 110 / 106 / 155 |
| 4K60 VP9 | VA-API NVDEC (driver package in a scratch dir) | 14.1 | 4.4 | 6.2 | 3.5 |
| 1080p60 VP9 | software / NVDEC | 47 to 60 / 12.7 | | | |
| 1080p60 AV1 | software (dav1d) / NVDEC | 35 to 46 / 30 | | | 20 on NVDEC, unexplained |

Hardware decode with the installed system package (hwdec, 2026-11-19: `libva-nvidia-driver` 0.0.18, `video.hardware_decode` on through the feature path the browser uses, `VaapiOnNvidiaGPUs` only; release windowless probe, 1280x720 view, 10 s windows, % of one core, total of browser + GPU + renderers; other builds ran the whole time: load average 10 to 75 in the table, so absolute numbers are inflated and noisy, the hardware-to-software ratio is what holds). Decoder per clip (media-internals): `VaapiVideoDecoder` for VP9 profile 0 and 2 and AV1 main (8 and 10-bit) at 1080p, 1440p and 2160p; `VpxVideoDecoder` / `Dav1dVideoDecoder` with the setting off. `chrome://gpu`: Decoding lists h264, vp8, vp9 profile 0 and 2, av1 main up to 8192x8192; `decodingInfo` `powerEfficient:true`; **"Sandboxed: false"** (see "Decided", video decode on Linux).

| Clip (60 fps, 8 s loop) | hardware total (3 runs) | software total (3 runs) | dropped frames in 10 s (hardware / software) |
|---|---|---|---|
| 4K VP9 | 53 / 55 / 49 (an unloaded single run earlier: 25; 40 at load 25) | 225 / 234 / 213 (288 at load 25) | 0 to 4 / 69 to 111 (4 at load 25) |
| 4K AV1 | 52 / 39 / 55 (38 at load 25) | 261 / 235 / 286 (230 at load 25) | 1 to 2, one run 83 / 87 to 193 |
| 1440p VP9 / AV1 | 55 / 53 | 193 / 189 | 0 to 1 / 24 to 56 |
| 1080p VP9 / AV1 | 53 / 51 | 134 / 118 | 0 to 1 / 1 to 3 |

Hardware decode costs about 15 to 25 % in the browser process and 15 to 22 % in the GPU process whatever the clip (the probe's own present path), 8 to 11 % in the renderer; software costs 70 to 230 % in the renderer. Frames stay zero-copy (`cpu_fallback` 0). Colours: SMPTE bars in 8 and 10-bit, VP9 and AV1, probed at 12 points: hardware and software agree within 2 levels; the bundled 10-bit PQ clip passes `frame_smoke` on VA-API too. The product itself (`wubble`, setting off): `--smoke frames=120` pixels ok, glass ok, `cpu_fallback=0`; `frame_smoke` `sandbox ok ... gpu=seccomp`; `--smoke video` and `--smoke codecs` ok.

`chrome://gpu` without the driver: "Video Decode: Hardware accelerated" (feature status only) with an empty Decoding list; `decodingInfo` `powerEfficient:false`; media-internals `VpxVideoDecoder` / `Dav1dVideoDecoder`, about 15 `CodecWorker` threads in the renderer at 4K. With it: h264, vp8, vp9 profiles 0 and 2, av1 main up to 8192x8192, `powerEfficient:true`.

`disable-gpu-memory-buffer-video-frames`, software decode, total % of one core, 3 alternating runs: 4K60 VP9 on 132 / 128 / 187, off 217 / 202 / 273; 1080p VP9 and AV1 within noise (41 to 94 against 47 to 105); dropped frames 0 to 9 either way.

Under load, 4K60 VP9 in software, 28 `yes` burners at nice 0 (video frames/s from `getVideoPlaybackQuality`, presented = host frames):

| Condition | video frames/s | dropped | presented/s | present interval p95 / p99 (ms) |
|---|---|---|---|---|
| no extra load | 59.7 | 0 | 59.6 | 19.8 / 22.5 |
| 28 burners, probe in the terminal's scope | 19.6 | 80 | 12.4 | 432 / 525 |
| burners at nice 10 / 19 | 42.6 / 39.6 | 30 / 43 | 40.0 / 33.4 | 47 / 212, 149 / 247 |
| probe in its own scope, CPUWeight=300 (what wubble now does) | 52.4 | 17 | 48.7 | 38 / 157 |
| probe in its own scope, CPUWeight=1000 | 54.5 | 11 | 51.2 | 32 / 129 |
| burners in the idle class (`chrt -i 0`) | 57.3 | 19 | 54.6 | 28 / 69 |
| ~11 key threads raised to nice -11 through rtkit, same cgroup | 31.6 | 33 | 28.1 | 197 / 288 |

Frame path under video (`wubble --measure video --seconds 12`, 240 Hz CEF cap): one copy in the paint callback, one blit into the ring, one composite per page frame, `cpu_fallback` 0. Scene renders before b9: `scene_cpu` n 3462 = `slint_render` n 3462 (the scene stepped on every page frame, fullscreen too). After (`scene_due`, dev profile (opt-level 1), other builds running): `scene_cpu` n 572 against `slint_render` n 3411 (2881 page frames delivered, 2776 presented); scene p50 0.42 ms per render it does. Fullscreen: no scene render and no scene layer.


## Browser benchmarks against Chromium and Firefox (2026-11-20)

2026-11-20, Linux: i7-14700F (28 threads), RTX 5060 / NVIDIA 615.71.09, Fedora 44, GNOME Wayland. wubble 0.1.0-preview.1 (the published tarball, own CEF 156 build) against Chromium 157.0.8103.78 (Fedora build; Google Chrome is not installed) and Firefox 160.0, each on a fresh profile, one browser at a time, load average under 3. Speedometer 3.1 (`release/3.1`) and MotionMark 1.3 served from loopback inside a same-origin frame of 1000x680 CSS px in every browser; the wrapper page starts the suite and posts the score. CPU seconds are user + system over the browser's whole process tree from launch to the score; memory is the tree's peak PSS (`smaps_rollup`, sampled every 5 s). Speedometer: 3 runs each, interleaved; MotionMark: 1 run each (it chose a 144 fps target in all three; why not 300 was not checked). The harness was a scratch script, not kept in the repo.

| Visible windows, 300 Hz display | wubble | Chromium 157 | Firefox 160 |
|---|---|---|---|
| Speedometer 3.1 score | 27.2 to 27.4 | 32.5 to 32.7 | 31.9 to 32.4 |
| Speedometer CPU s per run | 41.4 to 41.7 | 31.9 to 32.5 | 35.1 to 35.9 |
| Speedometer peak PSS (MB) | 1271 to 1384 | 1225 to 1434 | 2886 to 3245 |
| MotionMark 1.3 score | 1708.75 ±1.02 % | 1783.94 ±1.03 % | 1102.74 ±0.92 % |
| MotionMark CPU s per run | 620.4 | 530.8 | 496.2 |
| MotionMark peak PSS (MB) | 1368 | 1093 | 1459 |

MotionMark per test, same runs (Multiply / Canvas Arcs / Leaves / Paths / Canvas Lines / Images / Design / Suits): wubble 1320 / 4254 / 1286 / 10021 / 25173 / 178 / 242 / 928; Chromium 1683 / 4150 / 1528 / 9768 / 21673 / 167 / 303 / 898; Firefox 1104 / 546 / 986 / 3800 / 18931 / 271 / 207 / 910.

The same suites on the headless display (`scripts/headless.sh`, one 60 Hz virtual monitor, 1920x1080), before the frame was fixed: the page filled each window's content area (wubble 1004x784, Chromium 1256x688, Firefox 1332x804), so the MotionMark scores are not comparable across browsers.

| Headless, 60 Hz | wubble | Chromium 157 | Firefox 160 |
|---|---|---|---|
| Speedometer 3.1 score | 28.7 to 28.9 | 33.8 to 34.0 | 31.9 to 32.9 |
| Speedometer CPU s per run | 35.6 to 36.0 | 30.0 to 30.2 | 33.2 to 33.8 |
| Speedometer peak PSS (MB) | 1224 to 1249 | 1080 to 1211 | 2373 to 2455 |
| MotionMark 1.3 score (60 fps target) | 4205.03 ±0.94 % | 3854.82 ±1.49 % | 2612.56 ±1.26 % |
| MotionMark CPU s per run | 514.2 | 431.4 | 385.9 |
| MotionMark peak PSS (MB) | 1525 | 1253 | 1508 |

wubble is about 16 % behind Chromium on Speedometer and within one run's noise on MotionMark (4 % behind), for 17 % (MotionMark) to 28 % (Speedometer) more CPU. From 60 Hz to 300 Hz wubble's Speedometer CPU rose 36 to 41.5 s, Chromium's 30 to 32 s. The Speedometer gap's cause is not isolated: the engine version (156 against 157), the own CEF build's flags, or the shell's compositing competing with the page.

## Main-process flame graph (2026-11-20)

2026-11-20, same machine, visible window on the 300 Hz display (CEF at 240). A 1080p60 VP9 clip (software decode) looping in a still page, shown at 854x480. `perf record -e cycles:u -F 999 --call-graph lbr` on the whole process tree for 20 s, 10 s after start; user space only (`perf_event_paranoid` 2, no root), so kernel time is not in it. The binary is a local `--profile dist` build of the same source with symbols kept (DWARF unwinding of the wubble binary failed in perf 7.3, hence LBR: 32 frames, deeper stacks completed from the most common caller chain). One recording. Tree CPU 42 % of a core, main process 14 % (user + system). The flame graphs and the scripts were kept outside the repo.

| User CPU samples by process | share |
|---|---|
| Chromium page process (with the software decode) | 49.6 % |
| wubble main process | 32.1 % |
| Chromium GPU process | 18.2 % |

| Main thread (2719 of the main process's 3222 samples) | share |
|---|---|
| Slint render of the window, total | 75.8 % |
| item tree walk and property evaluation | 25.0 % |
| Skia flush to the GPU (with the NVIDIA driver) | 19.7 % |
| opacity, clip and layer bookkeeping | 9.0 % |
| recording paths | 7.7 % |
| present | 6.4 % |
| recording rectangles, images, shadows | 4.6 % |
| recording text | 3.4 % |
| CEF message pump (Chromium in-process and host work) | 14.0 % |
| page frame arrival (`on_accelerated_paint`: import and blit) | 7.1 % |
| event loop, Wayland, other | 3.0 % |

Inclusive: the NVIDIA driver 23.2 %, `wubble_gfx::scene::render::Renderer::frame` 1.3 %, `import_vulkan` 3.5 %. The whole window is redrawn for every page frame; the scene is not the cost.

## Pixel questions and the glass governor (pf/perf, 2026-11-21)

One release build of pf/perf after the merge of main 8ac66cb, partial rendering on and off
(`SLINT_SKIA_PARTIAL_RENDERING=off`), alternated, load under 3 (0.5 to 1.2) before each run.

Whole-window frames (`--measure idle --seconds 10 --measure-page wubble://start`, the scene redraws the whole
window every frame), 3 runs each:

| | on | off |
|---|---|---|
| Slint render p50 / p95 | 0.72 to 0.73 / 1.19 to 1.24 ms | 0.63 to 0.65 / 1.09 to 1.14 ms |
| present interval p99 | 17.3 to 19.8 ms | 17.5 to 17.7 ms |
| missed vsyncs in 10 s | 2, 3, 2 | 2, 2, 2 |

A whole-window frame costs 0.1 ms more on the CPU with partial rendering on (the dirty-region walk; the kept
frame's copy to the swapchain is a GPU copy). It does not show in the frame intervals.

The gallery (`--smoke gallery`), 6 runs each: the glass governor changed level in none of the 12 runs. The
earlier steps to Flat (2 of 4 runs with partial on) were taken with the machine loaded; main's own build
stepped down once under the same load.

The gallery shots, with partial on against off (off's runs against each other give the noise):
- The shell and page scenes are within off's own spread. The exceptions are the agent-orb, task-manager and
  history scenes, which animate and differ in every run either way, and the first run of each set.
- `66-settings-sites`: the Sites list's scrollbar thumb is still fading in some runs and gone in others, both
  on and off (shot timing).
- `58-settings`, `69-settings-search-pages` and `75-archive` (2 or 3 of 6 runs, partial on only): up to 6
  levels on the edge columns of two inset-shadowed rounded rects, the Settings section pill and the address
  field. The inset shadow image sits at a fractional device position (x 311.36 for the pill). Skia
  anti-aliases its edge column differently when a partial frame's clip covers it, and the pixel then stays
  until the item is redrawn. The evidence, from `--smoke settings` in verify mode:
  - with inset shadows not drawn, the smoke has no mismatches;
  - the shadow's device position is the same in the partial frame and in the whole redraw (logged);
  - drawing the image through a shader instead leaves it;
  - snapping the image to the pixel grid removes it, but moves the shadow's edges by up to 3 levels against
    upstream (the owner's call, in TODO).
- Verify's other mismatches in `--smoke settings`, single pixels of the section icons on frames drawn whole,
  are not partial rendering's. They remain with every frame forced whole, and follow the draw order (drawing
  the reference first moves them to it). They go away when the inset shadow images are rendered on a CPU
  surface. A frame that renders a new shadow image starts a GPU offscreen surface mid-frame, and Skia then
  rasterizes the icon paths after it slightly differently; verify's reference finds the image cached and
  starts none. Upstream draws each frame once, so it shows the first draw, as the presented frame does.

Against main (a49a1be, 3 runs, its own spread as the noise): every shot of the 12 pf/perf runs, partial on or
off, is within main's spread, but for three kinds of difference. Settings' Agents page and its Updates group
print the binary's path, which differs. The first sidebar scenes vary by 1000 to 2400 px in every pf/perf run,
on and off alike. And the inset-shadow edges above.

`42-toast-question` (1 run in 4 earlier) was not a stale frame. In that run Chromium's GPU process exited
(133) while the page was alive, and from the next resize on the page laid out with smaller body text. Killing
the GPU process at the same point (SIGTRAP to the helper with a `GpuWatchdog` thread, once scene 15 is shot)
reproduces it on main's build, shot for shot. The in-place path presents whole on any size change (`import.rs` `in_place`, unit
tested).

## pf/perf against main, quiet machine (2026-11-21, numbers of record)

The release builds of main at f43b5cf ("base") and of pf/perf at cc5a49a ("final": partial rendering, page damage in
place, the energy fixes), run on the invisible display (60 Hz, 1360x880). The two builds alternated, base then final,
3 rounds, each run waiting for a load average under 3 (one agent and the other lanes were idle or finished; per-run
load averages are 0.8 to 3.4).

Main thread, `scripts/bench/profile.sh` (20 s of `perf` LBR at 999 Hz each): samples, then shares.

| | base 1, 2, 3 | final 1, 2, 3 |
|---|---|---|
| 1080p60 VP9 clip in a page: main-thread samples | 2932, 1882, 2064 | 1527, 1641, 2738 |
| - Slint render of the window | 67.8, 65.5, 64.9 % | 59.7, 60.2, 60.2 % |
| - item tree walk | 20.2, 18.0, 20.9 % | 28.6, 29.8, 31.7 % |
| - Skia flush | 15.9, 17.3, 15.7 % | 7.5, 8.7, 6.8 % |
| - recording (paths, text, rects, layers) | 20.5, 19.6, 17.9 % | 8.7, 8.4, 8.0 % |
| - tree CPU, main process (user + system) | 64/16, 36/10, 42/11 % | 36/8, 36/9, 63/15 % |
| 16 px CSS spinner: main-thread samples | 2633, 3359, 1941 | 1625, 1373, 3396 |
| - tree CPU, main process | 23/14, 30/19, 15/10 % | 13/8, 12/8, 37/20 % |

Each column has one run well above the others (the machine's other work); the medians are what to compare:
video 2064 → 1641 main-thread samples (−20 %), spinner 2633 → 1625 (−38 %).

`--measure scroll` (10 s): render p50 0.34, 0.33, 0.54 ms (base) against 0.23, 0.24, 0.22 ms; frame interval p99
24.7 to 32.5 ms (base: a few late frames each run) against 17.3 to 18.8 ms.

Energy table (`scripts/bench/energy.sh`, 10 s per scenario; base has no `--measure-page`, so only idle, scroll and
canvas). CPU s/s and wake-ups/s, the three runs' range:

| scenario | base | final |
|---|---|---|
| idle | 0.01, 519 to 520 (Slint 0.23/s) | 0.00 to 0.01, 505 to 506 (Slint 0 to 0.05/s) |
| scroll | 0.15 to 0.17, 2558 to 2635 | 0.14 to 0.16, 2531 to 2602 |
| canvas | 0.20 to 0.22, 3364 to 3463 | 0.17 to 0.21, 3107 to 3444 |
| start page | | 0.08 to 0.09, 783 to 797 (scene and Slint 60/s) |
| spinner | | 0.12 to 0.14, 2240 to 2360 |
| caret | | 0.02, 643 to 648 |
| video page | | 0.35 to 0.39, 4683 to 4845 |

The tables below in this file from earlier on 2026-11-21 were taken with a load average of 11 to 40 and stay
as the record of what was measured then, not as numbers to compare against.

## Shell partial rendering (pf/perf step 1, 2026-11-21)

Same machine, release builds (`release` profile, symbols kept) of the commit before and after, on the invisible
display (`scripts/headless.sh`, 1360x880 at 60 Hz). The machine was **not quiet**: other work kept the load
average between 11 and 40 for the whole session, so CPU shares and frame rates swing by 2x between runs of the
same binary; the runs alternated base, step, base, step (3 each, `notes`-side script), so both saw the same
spread. Read the per-sample shares below, not the absolute CPU.

Main thread during the looping 1080p60 VP9 clip (`scripts/bench/profile.sh`, 20 s of `perf` LBR each), share of
the main thread's user samples; base runs 1 to 3, then step 1's:

| Main thread | base | step 1 |
|---|---|---|
| Slint render of the window, total | 63.2, 61.1, 61.5 | 53.2, 60.2, 59.9 |
| - item tree walk and property evaluation | 18.7, 20.8, 20.8 | 24.3, 29.0, 27.3 |
| - Skia flush to the GPU | 15.4, 14.2, 14.6 | 8.8, 9.7, 10.0 |
| - recording paths, text, rectangles, layers | 18.0, 14.2, 14.1 | 7.0, 6.8, 8.4 |
| - present and surface acquire | 5.7, 5.2, 6.2 | 6.3, 6.3, 7.0 |
| page frame arrival (import, blit) | 13.2, 15.5, 12.9 | 18.0, 18.4, 14.9 |
| CEF message pump | 18.4, 17.2, 20.2 | 22.6, 16.7, 19.5 |
| wubble main process, share of the tree's user samples | 25.7, 20.6, 23.1 | 20.9, 19.0, 20.5 |

`--measure video` and `--measure scroll` (10 s each): Slint render p50 0.51, 0.55, 1.00 ms (base) against 0.56,
0.26, 0.38 ms (step 1) for video; 0.48, 1.53, 1.54 against 0.35, 0.82, 1.08 for scroll. Step 1's frames: 64 to 75 %
redraw less than the whole window, 14 to 17 items drawn per frame (the whole shell is several hundred), dirty share
0.68 to 0.82 of the window (a page frame redraws its pane, 82 % of this window; the scene's 8 to 10 renders a
second, the ambience, mark the whole window).

What it buys: recording and flushing the shell's items is gone from page frames (about 13 points of the main
thread); the walk stays. Slint's partial renderer visits every item twice a frame (dirty regions, then the
render walk, which skips only drawing), so the walk is now the largest part; skipping it needs changes in
i-slint-core or a page path around Slint (decisions.md "Shell partial rendering"). `SLINT_SKIA_PARTIAL_RENDERING=verify`
compared every presented frame with a whole redraw: 0 mismatches in `--measure video` and `scroll`; the `tabs`
smoke shows 4 frames where the pane is drawn empty and filled in the whole redraw right after, frames drawn whole
(not a partial-region miss), next to its snapshots; unexplained, TODO.md.

Re-measure on a real display: `scripts/bench/profile.sh` against both builds (README there), and
`SLINT_SKIA_PARTIAL_RENDERING=off` on the same build for an A/B without a rebuild.

## Energy per scenario, Linux (2026-11-21)

`scripts/bench/energy.sh` on the release build of 5c12553, invisible display (60 Hz, 1360x880), 10 s per
scenario after a 9 s warm-up, one run each; load average 19 (other work on the machine). CPU is the whole tree's
(user + system) in seconds per second; wake-ups are context switches of every thread of the tree; the GPU column
(whole GPU, nvidia-smi) includes the desktop's own use.

| scenario | CPU s/s | wake-ups/s | CEF fps | Slint renders/s | scene renders/s | pump runs/s | GPU |
|---|---|---|---|---|---|---|---|
| idle still page | 0.01 | 555 | 0 | 0 | 0 | 32.6 | 4.2 %, 22.9 W |
| start page | 0.12 | 1257 | 0 | 56.8 | 56.8 | 30.1 | 4.4 %, 26.2 W |
| 16 px CSS spinner | 0.23 | 2357 | 55.3 | 57.3 | 7.5 | 157 | 4.7 %, 26.5 W |
| focused text field (caret) | 0.02 | 677 | 1.6 | 3.3 | 1.6 | 35.9 | 2.0 %, 20.8 W |
| scroll (`--measure scroll`) | 0.36 | 2450 | 59.6 | 55.5 | 8.5 | 120 | 4.6 %, 23.9 W |
| canvas (`--measure video`) | 0.74 | 3272 | 50.7 | 35.8 | 7.8 | 135 | 3.8 %, 27.8 W |
| 1080p60 VP9 clip | 0.58 | 5061 | 55.9 | 56.4 | 7.4 | 61.7 | 6.5 %, 24.9 W |

Read: a still page costs the CEF pump's 30 wake-ups a second (its 33 ms ceiling, decisions.md) and Chromium's own;
the start page renders the scene and the whole window at the display's rate for as long as it is shown (its liquid
and bubble); a 16 px spinner makes CEF paint the whole page at the display's rate and the shell redraw the pane
with it. Occluded, minimised and another-workspace windows are not measurable on the headless compositor (it
reports no occlusion); their pages stop by state since 239edd1.

## Page damage in place (pf/perf step 2, 2026-11-21)

`--measure` on the invisible display, the build with step 2 before its page-blur fix (load average 20 to 28,
so CPU columns are not comparable with the table above; the frame shapes are): the share of the window each
Slint frame redrew, the items drawn per frame, and the page frames written in place (CEF's
`capture_update_rect` smaller than the frame).

| page | step 1: dirty share, items | step 2: dirty share, items | frames in place |
|---|---|---|---|
| 16 px CSS spinner | about 0.8, 14 | 0.05, 9.2 | all but the first |
| caret | | 0.000, 8.0 | all |
| 854x480 VP9 clip in a page | 0.78, 15.6 | 0.39, 10.3 | 92 % |
| `--measure scroll`, `--measure video` (whole-page change) | 0.8, 14 to 17 | the same | 0 |

`SLINT_SKIA_PARTIAL_RENDERING=verify` over the spinner and the clip: 0 mismatches. The scene's ambience still
redraws the whole window at 8 Hz while a page changes (TODO.md).

## One thinking orb (xm/orbs)

2026-11-20, Linux release, RTX 5060, headless display (`scripts/headless.sh`, 60 Hz, 1360x880 at 1x), `--measure idle --seconds 8` on the idle page, with `WUBBLE_MEASURE_ORB=<state>` putting one 64 px orb, moving, on the library's stage (`host/gallery_orbs.rs` `measure_one`). Load average 12 to 17 (a build on the box).

| run | slint_renders/s | scene_renders/s | browser main thread | browser process | GPU (whole) |
|---|---|---|---|---|---|
| no orb | 0.13 | 0 | 1.0 % | 2.0 % | 7.1 % |
| composing (566 dots, the most) | 49.4 | 0 | 9.4 % | 10.5 % | 7.7 % |
| working (516 dots) | 59.9 | 0 | 8.0 % | 8.8 % | 9.2 % |

A moving orb redraws the whole window every frame (Skia's renderer has no partial redraw here); the orb's own pass is small next to that. An orb at rest or paused costs no frames (`agent-orbs-paused`: 180 shown, 0 moving).

## Browser benchmarks after the optimization pass (2026-11-21)

2026-11-21, the machine and method of "Browser benchmarks against Chromium and Firefox" above (`scripts/bench/runner.py`,
visible windows on the 300 Hz display, fresh profiles, one browser at a time, load average 1.3 to 2.7 before each run).
"wubble" is the release build of main at 302c571 (partial rendering, page damage in place); "before" is the release
build of main before pf/perf was merged. Speedometer 3.1: 3 runs each, interleaved; MotionMark 1.3: 1 run each
(144 fps target in all).

| | wubble | before | Chromium 157 | Firefox 160 |
|---|---|---|---|---|
| Speedometer 3.1 score | 27.5 to 27.6 | 27.5 | 33.1 to 33.4 | 32.3 to 32.4 |
| Speedometer CPU s per run | 40.5 to 40.9 | 41.4 to 41.6 | 31.4 to 31.5 | 33.7 to 35.2 |
| Speedometer peak PSS (MB) | 1324 to 1367 | 1294 to 1320 | 1223 to 1642 | 2910 to 3310 |
| MotionMark 1.3 score | 1933.03 ±0.86 % | 1899.85 ±0.85 % | 1976.57 ±0.77 % | 1060.85 ±0.78 % |
| MotionMark CPU s per run | 543.1 | 594.8 | 480.4 | 453.1 |
| MotionMark peak PSS (MB) | 1722 | 1654 | 1415 | 1517 |

The pass does not move Speedometer (score and CPU within 2 %): its 17 % gap to Chromium is not the shell's redraw.
MotionMark: 9 % less CPU than before for the same score within noise; 2 % behind Chromium for 13 % more CPU (was 17 %).

## The owner's Mac, preview.5 (2026-11-21)

Taken by the owner on their Mac (Apple silicon, a 5K external display, 120 Hz) with 0.1.0-preview.5, by hand; the
method and tools are the owner's and were not recorded here. CPU is a share of one core; "Tabs" is the page processes
together.

| Step | Main process | GPU process | Tabs | Total CPU (peak) | GPU busy |
|---|---|---|---|---|---|
| Idle, 1 blank tab | 2 % | 0 % | 2 % | 4 % (5 %) | 9 % |
| Opening 10 sites | 53 % | 10 % | 45 % | 123 % (156 %) | 42 % |
| Scrolling through them | 26 % | 6 % | 18 % | 51 % (65 %) | 50 % |
| 10 tabs open, nothing happening | 10 % | 2 % | 13 % | 26 % (33 %) | 4 % |
| 1 video, 1080p60 AV1, on screen | 26 % | 10 % | 57 % | 94 % (115 %) | 61 % |
| Same video, fullscreen | 29 % | 10 % | 54 % | 92 % (117 %) | 68 % |
| 3 videos playing (1 on screen) | 26 % | 12 % | 69 % | 107 % (133 %) | 71 % |
| 3 videos, switching between them | 28 % | 14 % | 66 % | 108 % (131 %) | 78 % |
| 3 videos in background, browsing in front | 24 % | 13 % | 76 % | 114 % (136 %) | 49 % |
| 14 tabs, videos paused | 10 % | 2 % | 16 % | 28 % (37 %) | 2 % |
| Everything closed | 2 % | 0 % | 5 % | 7 % | 4 % |

Also noted: 9 of 10 sites loaded in 0.4 to 1.0 s (The Verge 2.4 s); the on-screen 60 fps AV1 video dropped 4 frames in
about 2 minutes (fullscreen included), the 30 fps one 3; a background 60 fps VP9 video (hardware decode) dropped 130 of
about 10,300 frames, more on tab switches; pausing the videos freed about 2 GB in the GPU process; the main process's
footprint went from about 850 MB at the start to 1.18 GB after everything closed. The main process's 26 % for one video
was 37 to 40 % on preview.1 by the same hand. What follows from it: TODO.md, "From the owner's Mac run of preview.5".

## The owner's Mac, preview.5, Low Power Mode, with stack samples (2026-11-21)

The same workflow (`tools/mcp-bench` on branch `bench/mcp-workflow`, about 9 minutes), run over ssh through the
owner's MCP bridge, the Mac in Low Power Mode on its adapter (80 to 88 %, charging by the end), one run. Beside the
sampler, `sample` took 5 s of the main process and of the GPU process 8 s into each phase (1 ms interval; under load
it got 1400 to 3450 samples in the 5 s). Raw data: `~/Dev/wubble-wt/notes/mac-bench-1/` (not in the repo).

| phase | main CPU % | GPU proc % | tabs % | total % (p95) | main MB | GPU proc MB | GPU util % |
|---|---|---|---|---|---|---|---|
| 1-idle | 1 | 0 | 6 | 8 (16) | 1030 | 363 | 10 |
| 2-browse-open | 61 | 11 | 39 | 126 (135) | 1377 | 2208 | 53 |
| 2-browse-scroll | 29 | 7 | 24 | 60 (91) | 1214 | 2151 | 62 |
| 2-browse-idle | 9 | 2 | 15 | 26 (54) | 1235 | 2199 | 15 |
| 3-video1-play | 30 | 10 | 54 | 95 (111) | 1574 | 3102 | 63 |
| 3-video1-fullscreen | 29 | 10 | 50 | 89 (110) | 1671 | 2873 | 60 |
| 4-video3-play | 29 | 14 | 76 | 121 (160) | 1554 | 2758 | 55 |
| 4-video3-switch | 32 | 16 | 72 | 121 (157) | 1517 | 2757 | 63 |
| 5-mixed | 25 | 15 | 72 | 114 (140) | 1440 | 2817 | 50 |
| 6-paused | 11 | 2 | 22 | 35 (65) | 1321 | 469 | 4 |
| 7-recovered | 2 | 0 | 11 | 13 (49) | 1256 | 522 | 3 |

Within a few points of the owner's own run on mains power: Low Power Mode did not change the picture.

The main thread, from the stack samples (share of the samples in which it was not waiting):

| phase | busy | in wubble's own code | in CEF | the accessibility mirror |
|---|---|---|---|---|
| 1-idle (one blank tab) | 1.8 % | 0.1 % | 0.3 % | 0 |
| 2-browse-idle (10 tabs, still) | 8.8 % | 6.6 % | 1.3 % | about 6.6 % |
| 2-browse-scroll | 35.7 % | 20.7 % | 6.6 % | 7.7 % |
| 3-video1-play | 33.8 % | 20.7 % | 5.4 % | 6.0 % |
| 3-video1-fullscreen | 20.5 % | 11.3 % | 2.7 % | 4.1 % |
| 4-video3-switch | 25.6 % | 14.0 % | 3.9 % | 2.7 % |

What the samples show and do not:
- The shipped Mac binary has no symbols (`strip = "symbols"`), so wubble's own code shows as addresses. One path was
  identified from the strings its code refers to (B-tree assertions beside the accessibility tree's field names,
  `childTreeId`, `tree_data`, `hierarchicalLevel`): the mirror of CEF's accessibility tree (`host/ax.rs`,
  `wubble_ax::PageTree`). Every tab of an agent session has accessibility on (`wanted = on || agent_drives(tab)`),
  and this benchmark drives all its tabs through an agent session. So "10 idle tabs keep the main process at 10 %"
  is the mirror applying updates from pages that animate while hidden: a cost of agent sessions, and of the way the
  benchmark is driven, not of ten tabs a person opened (not yet measured: needs tabs opened outside a session).
  The same path is 3 to 8 points of every other phase's main thread.
- Metal, IOSurface and the GPU driver are 1 to 2 % of the main thread during video: present is not where the main
  thread's time goes. Most of it is in wubble's own stripped code (Slint and Skia are linked into it); naming it
  needs a build with symbols.
- Nothing here counts frames, so the paint rate during video (60 or 120) and the lag on a tab switch a tester
  reported are not answered by this run.

## The owner's Mac, preview.5: frame runs in Low Power Mode (2026-11-21)

`--measure` runs of the installed app, started by the owner (`/Users/Shared/wubble-frames.sh`), 10 s each, Low
Power Mode on, on the adapter, window 2560x1600@2, one run each (scroll twice). The first two runs reported a
120 Hz display, the rest 60 Hz. Raw output: `~/Dev/wubble-wt/notes/mac-bench-3/` (not in the repo).

| run | page frames /s | shell renders /s | missed refreshes | frame interval p50 / p95 / max ms | pump gap p50 / p95 ms | main % | GPU proc % |
|---|---|---|---|---|---|---|---|
| idle (still page) | 0 | 8.6 | 193 | 16.6 / 24.7 / 919 | 33.8 / 37.6 | 7.2 | 0.7 |
| idle on the start page | 0 | 59.8 | 606 | 16.7 / 22.9 / 30 | 38.8 / 51.3 | 51.3 | 0.3 |
| scroll | 34.9 | 48.1 | 246 | 16.7 / 48.6 / 65 | 2.2 / 50.1 | 33.6 | 18.9 |
| scroll, second run | 56.1 | 58.5 | 36 | 16.7 / 24.0 / 48 | 3.4 / 14.4 | 33.2 | 11.6 |
| scroll, `--begin-frames` | 58.1 | 59.4 | 12 | 16.7 / 20.2 / 33 | 4.5 / 11.9 | 37.1 | 10.0 |
| canvas animation | 49.6 | 56.0 | 81 | 16.7 / 36.9 / 51 | 4.0 / 17.9 | 31.9 | 22.3 |
| canvas, `--begin-frames` | 58.9 | 59.9 | 3 | 16.6 / 19.4 / 33 | 7.6 / 16.6 | 32.9 | 15.4 |
| resize | 43.9 | 58.5 | 88 | 16.6 / 28.6 / 51 | 9.6 / 29.8 | 87.1 | 17.7 |
| typing | 41.0 | 44.7 | 224 | 17.4 / 47.8 / 115 | 2.9 / 48.2 | 32.4 | 19.5 |

(frame interval = `present_interval`; typing: key to pixels 53 / 88 / 103 ms, pointer to pixels 45 / 81 / 83 ms;
resize: 428 stale frames, longest run 90.)

- Frames are dropped: with CEF on its own timer, 35 to 56 page frames a second on a 60 Hz display and 36 to 246
  missed refreshes in 10 s; the late frames come in steps of about 50 ms (three refreshes). The message pump
  shows gaps of 33 to 55 ms in every run (idle too), so the main loop is being woken late; why (Low Power Mode's
  timer coalescing, the pump's own delayed-work timer, App Nap) is not yet known, and no run was made with Low
  Power Mode off.
- With `--begin-frames` (one CEF frame per shell render) the same pages hold 58 to 59 frames a second with 3 to
  12 missed refreshes, on less GPU-process time.
- The start page redraws the whole window 60 times a second for ever (dirty share 1.0, no partial frames) and
  holds the main process at 51 % of a core.
- A still page was redrawn 8.6 times a second (whole window each time); the rule on Linux is 0 to 2 in 4 s.

## The start page's damage (Linux, 2026-11-21)

`--measure idle --seconds 10 --measure-page wubble://start` under `scripts/headless.sh` (1360x880@1, 60 Hz, RTX
5060), before and after a scene frame marks only the start page's card instead of the whole window. Linux numbers;
the Mac is not measured yet.

| | slint renders /s | scene renders /s | partial frames | dirty share | items a frame | Browser % | main % |
|---|---|---|---|---|---|---|---|
| before | 59.9 | 59.9 | 0 of 599 | 1.000 | 56.0 | 12.4 | 11.5 |
| after | 59.8 | 59.8 | 598 of 598 | 0.783 | 20.0 | 8.5 | 8.1 |

The card is 78 % of this window, and its liquid moves all over it every frame (the web's, which never settles), so
that is as small as the region gets without changing the motion. `SLINT_SKIA_PARTIAL_RENDERING=verify`: 0 mismatches
in the window; 1 px on the first (whole) frame, as on main.
Browser CPU is noisy on this shared machine: the table is one pair at load 6; two more pairs at load 12 to 18 gave
24.3 and 26.3 % before against 19.3 and 15.9 % after (renders fell to 48 and 36 a second under that load).
