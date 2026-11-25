# macOS

Owner, 2026-11-19: the Mac is driven only from a session on the Mac itself; other sessions do not ssh to it unless the owner says so; report macOS as not run.

## Bundle

macOS runs CEF only from an app bundle (`ssh pixl-mac`, plain cargo there; rsync the
worktree without `target/` and `.git`; clippy needs `rustup component add clippy` once):

    cargo build --workspace && cargo run -p wubble-bundle   # writes target/debug/bundle/wubble.app
    target/debug/bundle/wubble.app/Contents/MacOS/wubble --smoke boot   # any wubble args; `open` fails over ssh
    cargo build --workspace --examples && cargo run -p wubble-bundle -- --exe target/debug/examples/frame_smoke
    target/debug/bundle/wubble.app/Contents/MacOS/wubble   # windowless zero-copy and colour check; re-bundle without --exe after
    cargo run -p wubble-engine --example frame_smoke   # the same check on Linux

## Console smoke loop

Window smokes need a GUI session: over ssh they hang creating the window, so always
wrap them in `timeout`. Log in at the Mac console as `pixlbot`
(fast user switching; the toolchain and `~/wubble-main` are that user's), open Terminal,
`cd ~/wubble-main && export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH`, build the bundle
(above), then:

    for s in boot nav tabs session frames=120 split page settings pages autofill windows media video boost builtins quit block parity; do echo "== $s"; timeout 120 target/debug/bundle/wubble.app/Contents/MacOS/wubble --smoke $s > /tmp/wubble-$s.log 2>&1; echo "exit $?"; grep -E '^smoke|panicked|^wubble:' /tmp/wubble-$s.log; done; pgrep -fl MacOS/wubble

Each section ends `exit 0` with its `smoke:` lines (`frames=120`: `smoke: pixels ok` and
`smoke: glass ok`); `exit 124` is a hang killed by `timeout`; any other exit is a crash
(full output in `/tmp/wubble-<smoke>.log`). A `panicked` line about handling an event
while another is handled is the cue for a dispatch2 main-queue pump. `pgrep` prints nothing.
`block_smoke` runs like `frame_smoke` (bundled with `--exe`, over plain ssh).

## Hand-check harness

`scripts/mac-checks/run.py /path/to/wubble.app`: the page layer's hand-check list driven on the Mac's
screen (pixels, trace, per-process CPU and GPU, flame graphs); from a session on the Mac, as the
owner: [scripts/mac-checks/README.md](../scripts/mac-checks/README.md).

## Job runner

When the owner has started `~/wubble-runner/run.sh` in a Terminal of their GUI session, a window
smoke runs from ssh as a job: `printf "wubble-<dir>\n<timeout s>\n<args>\n" > ~/wubble-runner/queue/<fresh name>.job`
runs `~/wubble-<dir>/target/debug/bundle/wubble.app/Contents/MacOS/wubble <args>` there (only `wubble-*`
dirs; no environment variables: use flags; write output files to `/tmp`) and writes
`/tmp/wubble-runner-out/<name>.log` and `.exit` (124 = timeout).

- It runs one job at a time for every lane; it cannot run the examples (they replace the bundle's binary).
- Poll for the `.exit` file, then read the `.log`; keep runs under 30 s and few (windows open on the owner's screen).
- `rm -rf target/debug/bundle` before re-bundling: an in-place rewrite of the binary gets it killed (exit 137).
