# Benchmarks and profiles (hand run)

What the optimization pass measures with, kept so every step has a before and an after on the same
build. Nothing here runs in `cargo test` or CI, and nothing asserts a number: take each figure several
times, give the spread and the load average (`cut -d' ' -f1-3 /proc/loadavg`). Results go to
[docs/measurements.md](../../docs/measurements.md); raw outputs stay outside the repo.

Every window run goes on the invisible display, under `timeout`:
`scripts/headless.sh timeout 120 <command>`. That display is one virtual monitor at 60 Hz
(1360x880 by default, `WUBBLE_HEADLESS_SIZE`); numbers tied to the refresh rate (frames, renders,
CPU per second) differ on a real 300 Hz display.

| File | What it measures |
|---|---|
| `profile.sh` | `perf record` on wubble's whole process tree while it shows a page served from loopback; writes the flame graphs (`NAME.svg`, `NAME-main.svg` for the main thread), `NAME.breakdown.txt` (below) and `NAME.cpu.txt` (tree and main-process CPU from `/proc`, user + system, with the load average) |
| `pgo.sh` | not a measurement: a profile-guided `dist` build of wubble into `target/pgo` (instrumented build, the `--measure` workloads, rebuild with the profile); untested, compare it with the plain build before use |
| `video-page.sh` | the profiling fixture: an 8 s 1080p60 VP9 clip looping muted at 854x480 in a still page (`video.html`) |
| `fold.py` | perf's samples to folded stacks: each Chromium helper process is one frame, runs of `libcef.so` frames in the main process collapse to one, LBR stacks (32 frames at most) are completed from the most common caller chain |
| `breakdown.py` | the folded stacks as two tables: samples by process, and wubble's main thread by what it does (frame arrival, the Slint render and its parts, the CEF pump); its rules name functions of the pinned Slint, Skia and wgpu |
| `runner.py` | one browser (wubble, Chromium or Firefox) on Speedometer 3.1 or MotionMark 1.3 served from loopback inside a 1000x680 frame; prints a JSON line: score, CPU seconds over the process tree, peak PSS |
| `linux-vs-firefox.py` | wubble against Firefox, the Mac harness's states (idle, video, video under the find bar and the command bar, scrolling, typing), a fresh launch of each per state: CPU per process group, RSS/PSS, GPU (nvidia-smi), package W (RAPL, if readable) once a second; `samples.csv` and `results.md` (per state, wubble / Firefox, a verdict) in `.local/bench/<time>/`. Run from the repo as `~/Dev/wubble/.local/bin/run.sh timeout 900 scripts/headless.sh python3 scripts/bench/linux-vs-firefox.py --wubble target/release/wubble` (about 8 min a round; `--rounds 2` needs `timeout 1800`) |
| `common.py` | what the harnesses share: the process tree and its memory from `/proc`, a loopback server pages report to (`/log?k=v`), wubble's trace summary lines |
