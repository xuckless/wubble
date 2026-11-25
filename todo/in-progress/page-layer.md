# Page layer

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

The page on its own OS layer, the first road to the under-5 % CPU goal ([01-optimization.md](../upcoming/01-optimization.md)).
Research: [docs/research/page-layer.md](../../docs/research/page-layer.md) (newer on `untested/page-layer-2`).
Test apps on the Mac: `/Users/Shared/wubble-store-test/wubble.app` and `/Users/Shared/wubble-layer-test/wubble.app`.

## Layer: fullscreen
On `store/lane` behind `--page-layer=fullscreen` (off by default), tested; the owner measured it on an M2 during
fullscreen 60 fps video: main process 26 % -> 12 %, picture smooth and correct.
- [ ] `set_page_fullscreen`'s window ownership; a fullscreen tab moved to another window leaves its old window in page fullscreen until the page's own exit (no two-window smoke)

## Layer: windowed
- [~] Layer: windowed (`--page-layer=window`): the page's layer below the shell, seen through a kit `PageHole`. On branch `untested/page-layer-2`: green on Linux, compiled and tested on macOS; the owner's first window run pending, then the owner's hand-check list (`docs/research/page-layer.md` on that branch). Expected about 12 % for windowed pages. Supersedes the older plan of each pane's page on its own `wl_subsurface` (macOS: a `CALayer` on the IOSurface) with glass faked from a low-rate sample of the page; the prototype was to be measured first
- Measured by the owner (2026-11-23, M2, windowed 1080p60 video): on its layer the main process is 10 to 12 % (off: 28 to 37 %) with 4 to 5 shell renders a second. While a wubble glass surface is over the page (Notice, Toast, VideoStrip) the route sits in Entering with the OS buffers kept and both paths run: about 30 % main and high GPU; a sticky notice held it 110 s; held and focused, the shell rendered 105 to 119 times a second for a 60 fps video (unexplained)
- Overlay fix measured working (owner, 2026-11-24, M2, 1080p60 video): 159 of 166 s on Layer through overlays, left only for a resize and the SeamBow at start (each ended by the one-second rule), 8 shell renders and 4 snapshots a second (decisions.md "Overlays over a page layer")
- [~] Default on macOS from 0.6.2 (decisions.md "Page layer default"): on `untested/page-layer-2`, green on Linux; the macOS code not yet compiled after the merge. A fifth pool slot for frames in a burst (`pool_busy` 67 over 159 s with four); route lines and the summary only with `WUBBLE_TRACE=1`; Settings, Performance, "Show pages on their own layer (macOS)" turns it off
- [ ] The owner's hand-check list for a default-on release (`docs/research/page-layer.md` "Default on macOS (0.6.2)"), then merge to main
- [ ] Page frame rate on the layer under glass: 49 a second for a 60 fps video (62 before the fix); likely the shell presenting with the transaction for each snapshot render while the main thread is CEF's; measure `copy_wait` on the Mac, then present with the transaction only while a route or placement changes
- [ ] A frame dropped for a busy pool is lost: a burst's last leaves the layer one frame behind until the page paints again (ask CEF to repaint, or keep it)
