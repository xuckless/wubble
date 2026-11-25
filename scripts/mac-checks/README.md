# mac-checks

The owner's macOS hand-check list for the page layer
([docs/research/page-layer.md](../../docs/research/page-layer.md), "Owner's hand-check list for a
default-on release"), driven on the Mac's own screen. Hand run only, never in CI or `cargo test`:
it takes over the screen and judges by pixels and the trace.

    python3 scripts/mac-checks/run.py /path/to/wubble.app [--out DIR] [--only 1,4,o] [--flame 10]

- `run.py` starts the app with `WUBBLE_TRACE=1` on a throwaway profile (picture-in-picture on tab
  switch turned on in it), puts the window at (60, 60) 1400x900 and posts keys and mouse events.
  Fixture pages come from loopback with a known background colour each, so each pane is found and
  checked by colour (present, where it should be, not black or a transparent hole); an animated
  square keeps them painting, and `?still=1` drops it. Each check is judged from those pixels and
  from the page layer's route lines and per-second summary (`cef_frames`, `slint_renders`,
  `layer_snapshots`, `pool_busy`, each pane's route).
- Once a second it samples, without sudo, wubble's main, GPU and renderer processes: CPU, instructions
  and cycles retired, energy (`proc_pid_rusage`), GPU time (the driver's per-client `AppUsage` in the
  IO registry), plus the whole GPU's utilisation and the other processes using it most (a video
  call's `avconferenced`, `WindowServer`): those share the GPU, so read wubble's own GPU ms/s.
- `--flame N`: after the checks, `sample` on the main, GPU and busiest renderer process for N seconds,
  video alone and with the find bar over it; `flame/*-all.svg` (every thread) and `*-cpu.svg` (waits
  left out), per process and together. Needs a clone of
  [FlameGraph](https://github.com/brendangregg/FlameGraph) (`--flamegraph-dir`, default
  `~/wubble-mac-checks/tools/FlameGraph`) and, for Rust names, `rustfilt` (`cargo install rustfilt`).
  Symbols: a dist build is stripped; `sample` finds its dSYM (the release's
  `wubble-<v>-macos-aarch64.debug.tar.xz`) through Spotlight, so unpack it outside any folder excluded
  from Spotlight. CEF's framework ships stripped: its frames carry the nearest exported name.
- `helper.swift` (compiled by `run.py` with `swiftc`): keys, mouse, windows (CGWindowList,
  Accessibility), pixels (ImageIO), `proc_pid_rusage`.

Permissions for the terminal that runs it (System Settings > Privacy & Security): **Accessibility**
(keys, mouse, window moves) and **Screen Recording** (window shots). Needs Xcode or its command-line
tools (`swiftc`), Python 3 (standard library), `ffmpeg` (the video fixture; video checks are skipped
without it).

Results go to `DIR` (default `~/wubble-mac-checks/<time>`): `results.md` (the table, the manual list),
`wubble.log`, `samples.csv`, `shots/`, `flame/`. The profile and fixture are deleted at the end.

Automated: 1 split, 2 sidebar spring and peek, 3 seam drag, 4 tab switch and close, 6 drag and drop
inside the page then the next click, 7 find and command bar over a video, 8 context menu, 9 `<select>`,
10 picture-in-picture on a tab switch (and from a split pane), 11 page fullscreen, 13 resize,
14 minimise and restore, 16 the OS screenshot, 17 a private window; `s` a still page reaching its layer;
`e` macOS editing keys in a plain `<input>` (Option/Cmd+Delete, Cmd+Left, Shift+Cmd+Left, Ctrl+A/E/K;
judged from the value the page reports to the harness's `/log`) and Cmd+Left as Back outside a field;
`r` the `CEF frame rate` log line at 120 on the built-in display; `o` shell popups (find bar, command bar, context menu, zoom toast) held over a playing video against
the video alone: CPU, GPU, renders, snapshots. `b` Settings in a split pane beside a page (SKIP on a build that refuses it, as 0.6.2 does). By hand (printed at the end): 5 input methods, 6 OS drag and drop, 12 second
display, 15 sleep and wake, 16 `--shot`, 18 an external 60 Hz display, and everything by eye.
