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
- [~] The fix, in progress on `untested/page-layer-2`: overlays never pull the page off its layer and blur a low-resolution, slowly refreshed snapshot of it; only overlays whose rect meets the pane count; a hold that remains releases the layer's surfaces after about a second (decisions.md "Overlays over a page layer")
