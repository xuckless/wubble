# Page layer

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

The page on its own OS layer, the first road to the under-5 % CPU goal ([01-optimization.md](../upcoming/01-optimization.md)).
Research: [docs/research/page-layer.md](../../docs/research/page-layer.md).
Shipped in 0.6.2 ([done/0.6.2.md](../done/0.6.2.md)): fullscreen and window modes, window mode on by default on macOS.
Test apps on the Mac: `/Users/Shared/wubble-store-test/wubble.app` and `/Users/Shared/wubble-layer-test/wubble.app`.

- Dropped video frames on the layer (shelved by the owner for 0.6.2): [video-dropped-frames.md](../shelved/video-dropped-frames.md)
- [ ] `set_page_fullscreen`'s window ownership; a fullscreen tab moved to another window leaves its old window in page fullscreen until the page's own exit (no two-window smoke)
- [ ] The owner's hand-check list for the default-on layer (`docs/research/page-layer.md` "Default on macOS (0.6.2)")
- [ ] Page frame rate on the layer under glass: 49 a second for a 60 fps video (62 before the fix); likely the shell presenting with the transaction for each snapshot render while the main thread is CEF's; measure `copy_wait` on the Mac, then present with the transaction only while a route or placement changes
- [ ] A frame dropped for a busy pool is lost: a burst's last leaves the layer one frame behind until the page paints again (ask CEF to repaint, or keep it)
- Measured by the owner (2026-11-23, M2, windowed 1080p60 video): on its layer the main process is 10 to 12 % (off: 28 to 37 %); with the overlay fix (2026-11-24) 159 of 166 s on Layer through overlays, 8 shell renders and 4 snapshots a second (decisions.md "Overlays over a page layer"); held and focused before the fix, the shell rendered 105 to 119 times a second for a 60 fps video (unexplained)
