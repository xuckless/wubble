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

