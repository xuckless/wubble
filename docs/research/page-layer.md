# Page layer: the page on its own OS layer

Plan by a design agent, 2026-11-23, read from `untested/perf3` (nothing run).

[v] checked in sources, [u] must be tested on a machine.

## Facts
- Slint surface can show a hole: macOS window transparent when root background has alpha (i-slint-backend-winit 1.19 winitwindowadapter.rs:758), vendored renderer picks PostMultiplied, wgpu-hal setOpaque(false) (metal/surface.rs:273) [v]. Wayland: winit sets no opaque region on a transparent window [v].
- macOS: winit content view is layer-backed; wgpu adds its CAMetalLayer as sublayer of view.layer. Page layer: root.insertSublayer_below(page, metal_layer) [v].
- Crates already locked: objc2 0.6.5, objc2-quartz-core 0.3.2 (turn on `CATransaction` feature), objc2-io-surface 0.3.2, objc2-metal, objc2-app-kit; Linux wayland-client 0.31, wayland-protocols 0.32, sctk 0.20, gbm 0.18, ash 0.38.
- CEF's buffer must be copied (pool reuses it after the callback): welding `native_frame/metal.rs copy_metal_to_owned`, `vulkan_dmabuf.rs copy_dmabuf_to_owned`.
- Key move: change the copy's TARGET. Copy into a per-pane pool of 3-4 OS-presentable buffers (IOSurface-backed MTLTexture via newTextureWithDescriptor:iosurface:plane:, already used in import_metal; GBM BOs on Linux [u NVIDIA modifiers]). Same buffer is a wgpu texture (today's path) and an OS buffer (layer). One copy either way.

## macOS
- NSView from RawWindowHandle::AppKit (as wl/macos.rs:264). Find CAMetalLayer sublayer by class. One plain CALayer per pane below it: contentsScale = backingScaleFactor, frame = pane logical rect, contentsRect = clip/size, contentsGravity resize, opaque, cornerRadius Radius.md + masksToBounds.
- Per frame on main thread: CATransaction begin; setDisableActions(true); layer.setContents(iosurface); commit. Pool slot reused only when !is_in_use() and not latest; all busy -> drop frame. Re-setting same object may not refresh [u].
- Colour: tag IOSurfaces sRGB [u]; both routes must look identical or switches flash. HDR later.
- Resize: route Composited while resizing (cleared ~150 ms after last resize).
- Pacing: BeginFrames' one-period Timer fallback drives it in Layer route (stage 1); later NSView.displayLink -> PaceEvent::Vsync.

## Wayland
- wl_display/wl_surface from raw handles as wl/wayland.rs:137,217-228; second EventQueue on that connection owned by the main thread; in show_frame flush, dispatch_pending, non-blocking prepare_read().read() [u].
- wl_subsurface place_below(parent), set_desync; wp_viewporter set_destination(logical); opaque region; zwp_linux_dmabuf_v1 v4 XRGB8888, format/modifier from feedback, LINEAR fallback; one wl_buffer per BO, reuse after release. Per frame attach, damage_buffer, frame(), commit; hold newest while a callback is outstanding.
- Resize/layout: set_sync or route Composited. Fractional scale: hole 1 physical px smaller per side. Corners: Slint pixels cover them. Scanout is a bonus (KWin/wlroots maybe, mutter no) [u]. X11: Unsupported.

## What reads page pixels (falls back or is fed slowly)
host/panes.rs sync_scene_pages_with (PagePane.texture); scene/pages.rs page-inclusive blur for Card/Peek/Top glass + ambience; mark_damage -> glass.pages_written; EdgeMask; stale-frame stretch, prev fade, SplitPane.frame/edge/prev; glance card, Pip card, exit-page, Entering; snapshots.rs; drag ghost; snapshot(window)/take_snapshot (--shot, --smoke frames). MCP screenshots use CDP (unaffected). Windowless agent sessions never get a layer.
- Glass/ambience faked: in Layer route feed the presenter the newest pool buffer at most every 125 ms (AMBIENCE_HZ).
- Snapshot, exit page, ghost, shot: import latest buffer once.

## State (wubble-core/src/page_layer.rs, pure, one per pane)
enum Route { Unsupported, Composited, Entering, Layer, Leaving }
enum Need { Transition, Resize, Stretch, Masked, Glance, Shot, Ghost }  // set
enum Event { Capable(bool), Need(Need, bool), Presented, LayerFailed }
enum Effect { ShowLayer, HideLayer, Hole(bool), FeedPresenter, RequestRedraw }
fn transition(Route, needs, Event) -> (Route, Vec<Effect>)
Invariant: the layer is visible whenever a hole is drawn. Entering: show layer, hole on Presented. Leaving: FeedPresenter + draw image, hide layer after next Presented. LayerFailed -> Unsupported.

## Plug-in points
- welding: copies go into a pool slot; CopiedFrame gains `os: OsBuffer` (new wubble change).
- wubble-gfx/src/layer.rs: trait PageLayer { place(rect, clip, radius); show(&OsBuffer, damage) -> Result; hide(); dispatch() } with mac::CaLayer, wayland::Subsurface. FramePresenter::present unchanged, called only on FeedPresenter.
- host present_panes: Route::Layer -> layers[tab].show(...), skip present_damaged, mark_damage, render_panes, request_redraw. show_frame dispatches layers.
- Hole: kit `PageHole` drawn by vendored itemrenderer.rs draw_image with BlendMode::Clear for a sentinel texture. NEW PRIMITIVE: waits for the owner. Overturns decisions "Shell partial rendering" (present without a Slint render) and "Frame copy".

## Stages (flag --page-layer=off|fullscreen|on, setting render.page_layer)
1. macOS page fullscreen + Pip window only (no hole needed; video controls raise Leaving).
2. macOS every single pane (hole, cornerRadius, faked glass, fallbacks); 2b displayLink pacing.
3. macOS split, peek, DevTools.
4. Wayland single pane. 5. Wayland split. 6. CEF keep-buffer patch, HDR.

## Parity for free?
No: input, cursors, IME, drag and drop, clipboard, touch, <select> stay on the OSR side. Gains: main-process CPU, ~1 stage latency, a road to HDR, maybe overlay planes. Windowed CEF would give native IME/DnD/select/touch/overlay video/HDR but no shell pixels above the page, X11/XWayland only on Linux [u], and a rewrite of host/panes.rs and the scene's page work.

## Stage 2: windowed pages under the shell (`--page-layer=window`, macOS)

Design agent, 2026-11-23, on `untested/page-layer-2` (base f9730e4 + this file). Written blind: nothing
here was compiled or run. Owner approved the new kit primitive (the hole) the same day. Stage 1 as built is
`notes/store/layer.md`; its rows stay as they are for `fullscreen`.

The page's `CALayer` goes BELOW the shell's `CAMetalLayer`; the shell clears a rounded hole where a pane on its
layer shows, so everything the shell draws (sidebar, popovers, toasts, seams, the status bubble) stays on top
with no route change. Only what reads the page's pixels sends the pane back to the shell.

### The hole
- The window is already non-opaque on macOS: app.slint:102 `background: transparent` (and :89 `no-frame`), so
  i-slint-backend-winit 1.19 `winitwindowadapter.rs:687-689` `wants_transparent` is true and :759-761 asks winit
  for `with_transparent(true)`; winit 0.30.13 `platform_impl/macos/window_delegate.rs:659-662` then sets
  `setOpaque(false)` and a clear background colour; the vendored surface picks a translucent alpha mode
  (`wgpu_30_surface.rs:63-74`, :132-134: Metal offers only `PostMultiplied`), and wgpu-hal 30.0.2
  `metal/surface.rs:272-273` makes the `CAMetalLayer` `setOpaque(false)`. Nothing to change there.
- Clear colour: with partial rendering each dirty region is first cleared to the window's background
  (`vendor/i-slint-renderer-skia/lib.rs:1018-1025`), here transparent. But the window is painted edge to edge
  under the page (app.slint:970-1002: the scene image, the ground, the window's glass; :1041 the content's
  `Palette.web`; split-view.slint:108 the pane's own `Palette.web`), so a hole must remove pixels already drawn:
  a draw with `BlendMode::Clear`.
- How: vendored renderer change (3), `hole.rs`: a sentinel `Image` (one 1x1 RGBA8 buffer per thread, compared by
  its buffer as i-slint-core `graphics/image.rs:859-866` does); `itemrenderer.rs` `draw_image` (:715) draws an
  image that is the sentinel as a `Clear` rect of its geometry. The shape comes from the clip it is drawn in: the
  pane's `clip: true` with `border-radius` (split-view.slint:106-107) is `clip_rrect(.., antialias: true)`
  (`itemrenderer.rs:978-983`), so the hole has the pane's rounded, anti-aliased corners. A Clear inside a Skia
  layer would only clear the layer: the page's ancestors make none at rest (`Opacity::need_layer` is false at
  opacity 1, i-slint-core `items.rs:959-962`; the entrance's `OverlapLook` (kit/motion/overlap.slint) is at
  opacity 1 at rest, and while it plays the entrance is `Coverage::Pixels`, below). With partial rendering the
  kept frame keeps the cleared pixels; a later dirty rect over the pane redraws what is under the hole and the
  hole clears it again (it is an item in the region like any other).
- The kit primitive: `crates/wubble-ui/ui/kit/material/page-hole.slint`, `PageHole` (an `Image` of the
  sentinel) and the global `PageHoleSource` the host fills once per window (`i_slint_renderer_skia::hole::image()`,
  only when the mode's layer sits under the shell). `// derived:` header; no value of its own; on the owner's
  sheet (`docs/fidelity/derived-review.md`, "Page layer"). Composed by the kit's `SplitView`: a pane whose
  `SplitPane.hole` is set draws `PageHole` over its fill instead of its frame. Shell `.slint` files gain only the
  global's export, so `kit_lock` and `surface_lock` stay at their baselines (no literal, no paint, no
  visibility of the shell's own).
- Other renderers (the software renderer in tests, Linux) draw the sentinel as one transparent pixel: nothing,
  and the host never sets `hole` there (no layer is made off macOS).

### Glass
- Glass over a pane reads the page from the scene's page-inclusive blur, which the host feeds the presenter's
  texture of each pane (`host/panes.rs:708-729` `sync_scene_pages_with`; `wubble-gfx/src/scene/pages.rs:1-8`).
  With the page on its layer the presenter is not fed, so that blur would be stale.
- Rule: a surface whose row has glass over the pane is already `Coverage::Pixels` while the scene draws the
  overlays' glass (`overlay.rs:604-610`; `Facts.glass` is `GlassLayer.over-on`, `host/layers.rs:251`, which is
  off over a fullscreen page, app.slint:116). `Needs::covered` now sets two needs: `Covered` (Part or more) and
  `Sampled` (Pixels). In `window` mode `Sampled` holds the pane on the shell briefly (buffers kept, below) and
  `Covered` alone does not. `fullscreen` holds on both, as before.
- Kinds this sends back to the shell (glass rows with a cover, scene glass on): Notice, Permission, Find,
  Dialog, BookmarkSheet, PrintSheet, ExtensionSheet, AgentSheet, Region, CommandBar, Downloads, Extensions,
  Spaces, Menu, ErrorPanel, Boost, Toast, Preview, AutofillMenu, AutofillPrompt, VideoStrip/Tab/Speeds (their
  pane only), PipStrip, Reader, Glance, Peek; and, glass or not, Entrance and SeamBow (they move or cut the
  page). Kinds that stay on the layer (`Part`, no glass): Popup, Tooltip, Status, AutofillChip, PipCard, Swipe,
  ExitPage, SplitRing, SplitSeams, DevTools, TabGhost, DropZone, PageGhost, DropMark, Autoscroll, Specimen;
  Hints is `None`. With the scene's glass off every glass row is `Part`: the Slint-only glass (opaque .94 fill)
  draws over the hole.
- Not covered by the rule: the window's own glass picks up the page's light at the pane's edges (ambience) from
  the same blur; while a pane is on its layer that light is the pane's last composited frame. Accepted for
  stage 2 (the plan's 125 ms feed is later work).

### Geometry
- The layer's frame is the pane's rect exactly as the shell lays it out: content origin plus the pane's x, the
  content's top, the pane's width, the content's height (`host/page_layer.rs` `Show`, the rects of
  `host/panes.rs:185-199` `pane_rects`, the same the `SplitPane` rows are built from), snapped by
  `page_layer::place`. Corners: `pane_radius` (`host/panes.rs:64-70`) reads the generated `Radius.md` global, the
  token `SplitView` gets (app.slint:1070, split-view.slint:106); none in page fullscreen. No second copy.
- Sidebar springs, split-seam drags, live resize and a fullscreen change all resize the pane: `Resizing` (the
  stretch machine is not live) holds the pane on the shell for that time, so the hole is closed while the pane
  moves and the layer's place does not matter then. A move without a resize (panes reordered) keeps the hole: the
  layer is placed again at every render's start (`Changed`, from `panes_frame`) for a pane with a hole, in the
  same Core Animation transaction as that render's drawable: the shell presents with the transaction while any
  layer of the window is visible (stage 1's `present_in_transaction`; wgpu-hal reads it when it acquires,
  `metal/surface.rs:400`, which is before the render's `BeforeRendering`: `wgpu_30_surface.rs:614`), and
  `CaLayer`'s changes join the ambient transaction while it does (no commit of their own).
- Tab switch: the old tab's pane leaves (`layers_gone`), the new tab's comes with its entrance (`Pixels`). A
  forced exit of a pane that still has its hole (the tab left, the layer failed) would leave the shell's last
  frame with a hole over nothing: the layer is retired instead, kept visible under the shell until the window's
  next present (which draws no hole there), then hidden in that present's transaction and dropped.
- One layer per pane (the entries are per tab): a split's panes each have theirs, all below the shell's layer.
  The peek's page, an extension's popup and a start page have no pane of their own (`Need::Unplaced`) and never
  get a layer.

### Machine (`wubble-core/src/page_layer.rs`)
- `Need` gains `Sampled` and `Unplaced`; `Owners` gains `placed`. A mode is a row of `MODES` (name, `Order` of the
  layer: `Above` or `Below` the shell, and what each need does: `Hold::{Free, Brief, Long}`); `Needs::hold(mode)`
  is the strongest hold. `transition(route, hold, event)`: where stage 1 read "blocked" it now reads "held"
  (`Brief` or `Long`: leave the layer, stay off it) or "lasting" (`Long`: the OS buffers go too). `Brief` keeps
  them: the pane stays `Entering` (buffers asked for, the shell drawing every frame) and goes back to its layer at
  the first frame after the hold ends. In `fullscreen` every need is `Long`, so every stage 1 row is unchanged
  (the stage 1 table is kept as it was; a second table has the `Brief` rows).
- `window`: Windowed `Free`, Covered `Free`, Sampled `Brief`, Resizing `Brief`, Pixels `Brief`, Unplaced `Long`.
  `layer_unused` (drop a hidden entry) now asks whether a place need (Windowed, Unplaced) holds `Long` in the mode.
- `Route::holed(mode)`: the hole is drawn exactly while the route is `Layer` and the mode's layer is `Below`
  (`Leaving` has handed its frame to the shell, which draws it over the layer, so the layer may go once that
  render is presented, as stage 1). The invariant "the layer is visible whenever a hole is drawn" follows from the
  stage 1 invariant (visible exactly in `Layer` and `Leaving`).
- Pixel readers are unchanged: a shot, a pixel check and a snapshot are `Need::Pixels` (`Brief` in `window`),
  snapshots and the exit page first hand the layer's latest frame over (`layer_pixels`). The picture-in-picture
  window's pane gets its layer like any other.

### Input, cursors, IME, drag and drop, accessibility
- Unchanged by construction: the pane's `TouchArea`, its cursor and the page's input stay Slint's
  (split-view.slint, after the hole in the tree), IME and popups are placed from the same pane origin, drag and
  drop is the shell's, the page's accessibility tree is `PageAx` (Slint/AccessKit). The page layer has no
  accessibility and takes no events.
- One macOS risk [u]: AppKit may pass clicks through fully transparent pixels of a non-opaque window. The hole is
  transparent only in the shell's drawable; the composited window has the page there, so it should not, but the
  cheapest check is a click in the page with `--page-layer=window`; the fix would be `setIgnoresMouseEvents(false)`
  on the window.

### Linux later
Nothing here is macOS-shaped above the gfx trait: `PageLayer::new(.., below)` is what a Wayland subsurface
implements (`place_below` the shell's surface, a viewport for the frame, a rounded hole the shell draws: the
same `PageHole`), the machine and `MODES` rows are the same. Not written.

### As written (blind)
Commits on `untested/page-layer-2`: core (`MODES` rows, `Hold`, `Sampled`/`Unplaced`, the brief rows and walks), gfx
(`CaLayer::new(.., below)`, every change joins the shell's transaction while it presents with one), host (holes into
`SplitPane.hole`, `place_layer` at each render's start for a holed pane, retired layers), Skia change 3 (`hole.rs`, the
`partial_render` rows), the kit (`PageHole`, `SplitView`'s `pane-hole`, the global set in `WindowFactory::open`), the flag.
Note for the reviewer: `CaLayer::place`/`show` used their own transaction in stage 1; they now join the shell's while it
presents with one, in `fullscreen` too (the implicit transaction of that run-loop turn, as the hide already did).

### Owner's hand-check list (macOS, release build)
Command: `cargo run -p wubble --release -- --page-layer=window <url>`; compare with `--page-layer=off` on the same page.
1. A still page, then a scrolling one, then a 1080p video in a plain window: the page looks as with `off` (colour, the
   pane's rounded corners against the window's glass, no seam of another colour at the corners or edges); Activity Monitor's
   main-process CPU during the video well under `off`'s; stderr has no `wubble: page layer` line.
2. Sidebar: toggle it (springs), drag its seam, hover to peek it while hidden: the page stretches as with `off` while it
   moves (it is on the shell then) and nothing is left at the old place when it settles; no flash of the desktop.
3. Split view (two and three panes), drag a seam, reorder the panes, close one: each pane's page under its own corners; the
   seams, ring and pills over the pages.
4. Live window resize and the green fullscreen button, both ways; page fullscreen from a video and back.
5. Over a page on its layer: a context menu, the command bar, the find bar, a toast, a permission prompt, the downloads
   popover, the video strip on hover, a `<select>` list, a tooltip, the status bubble, a tab drag's ghost: the glass ones
   blur the live page (it goes to the shell while they show), the others are drawn over it; no frame of a stale page as
   each opens or closes.
6. Tab switches (with and without reduced motion), Shift+click peek, closing the shown tab, a tab moved to another
   window: no frame with a transparent hole (desktop showing through), no old page under the new one.
7. Input: click, drag-select, scroll, a text field with IME (Japanese) and its candidate window, drag a link out, drop a
   file in, the page's cursors; VoiceOver over the page as with `off`. A click in the page must not reach the window
   behind (the transparent-pixel pass-through risk).
8. `--shot` and the tab hover preview with `--page-layer=window`: the page is in them (not transparent).
9. Picture-in-picture: the Pip window's page on its layer, its strip on hover, the Main window's card.
10. `--measure idle --seconds 4` with `window`: 0 to 2 `slint_renders`; and with `off` the same as before.
11. Debug > View hierarchy (Xcode) with `off`: one `CAMetalLayer` and no other sublayer; with `window`: one `CALayer` per
    pane under it.

### Lines for TODO.md / decisions.md (not edited here)
- TODO.md: "Page layer stage 2 (`--page-layer=window`, branch `untested/page-layer-2`): compile and test on the
  Mac; the owner's hand-check list in docs/research/page-layer.md; then ambience feed, displayLink pacing."
- decisions.md: "Page layer stage 2: windowed panes on macOS go under the shell's layer through a hole the
  shell clears (`PageHole`, a sentinel image the vendored Skia renderer draws with `BlendMode::Clear`, change 3);
  glass over a pane (`Coverage::Pixels`), a resize and a pixel reader hold it on the shell with its OS buffers
  kept (`Hold::Brief`); `--page-layer=window`, default off."
- decisions.md: "Page layer modes are rows of `page_layer::MODES` (order of the layer, a hold per need); a new
  mode is a row."
