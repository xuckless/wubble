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

The frame path's own numbers come from wubble itself: `wubble --measure idle|scroll|video|resize|typing`
([docs/smokes.md](../../docs/smokes.md)).

## perf without root

The box gives no root and `kernel.perf_event_paranoid` is 2, so perf records user space only (no
kernel time: GPU driver ioctls and present are not seen). Fedora's `perf` package unpacks into any
directory:

    mkdir -p TOOLS/rpms TOOLS/root && cd TOOLS/rpms
    dnf download perf libtraceevent libbabeltrace   # as the user; keep the x86_64 ones
    cd ../root && for r in ../rpms/*x86_64.rpm; do rpm2cpio "$r" | cpio -idm; done
    # TOOLS/perf: LD_LIBRARY_PATH=TOOLS/root/usr/lib64 exec TOOLS/root/usr/bin/perf "$@"

DWARF unwinding of the wubble binary failed in perf 7.3; `-e cycles:u --call-graph lbr` (the
default here, `-g lbr`) works on this Intel CPU. FlameGraph (`git clone
https://github.com/brendangregg/FlameGraph`) needs the `use open qw(:std :utf8);` line of
`flamegraph.pl` commented out (wide-character errors on the folded stacks otherwise).

## Runs

    cargo build --release --workspace          # release keeps the symbol table; dist strips it
    scripts/bench/video-page.sh FIXTURE
    PERF=TOOLS/perf scripts/headless.sh timeout 120 \
      scripts/bench/profile.sh -b target/release/wubble -s FIXTURE -o OUT -f FLAMEGRAPH video video.html
    scripts/headless.sh timeout 60 target/release/wubble --measure idle --seconds 4
    scripts/headless.sh timeout 60 target/release/wubble --measure scroll --seconds 10
    WUBBLE=target/release/wubble scripts/headless.sh timeout 900 \
      python3 scripts/bench/runner.py SUITES OUT wubble speedometer

`profile.sh` serves on a port the kernel picks and stops unless wubble fetched the page (and nothing
it asked for was missing), so a run never profiles a wrong page.
