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
Superseded 2026-11-24: glass no longer holds a layered pane; it blurs a slow snapshot of the layer ("Overlays over
a layered page", below).
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

## Overlays over a layered page

Fix agent, 2026-11-24, on `untested/page-layer-2` (08b719e), read only (nothing built or run here; the Mac builds
and tests it). Replaces Stage 2's "Glass" rule (glass over a pane holds it on the shell, `Hold::Brief`). Owner's
measurement (M2, windowed 1080p60 video): on its layer the main process is 10-12 % with 4-5 shell renders a second,
but while Notice, Toast or VideoStrip glass is over the page the pane sits in `Entering` with both paths running
(about 30 %, 60-119 renders a second; `~/Downloads/wubble-layer-log*.txt`, lines like `Layer -> Leaving on Changed;
needs: Windowed, Covered, Sampled Brief [Status Part, Notice Pixels, Toast Pixels, VideoStrip Pixels]`).

### Where glass gets its backdrop today
- The pane's texture: every render of the panes hands the scene each pane's presenter texture itself, no copy
  (`host/panes.rs:712-733` `sync_scene_pages_with`, :725 `frame.image.to_wgpu_30_texture()`; `backdrop.rs:487-495`
  `Glass::set_pages`; `scene/render.rs:744-756`). A frame written in place marks the page blur stale through
  `mark_damage` (`host/panes.rs:905-944`, :941-943 `pages_written`; `backdrop.rs:499-506`; `render.rs:767-776`
  `pages_written_rect`, which asks `pages.rs:65-106` `damage_reaches_glass`).
- The page blur `P`: the backdrop's half-resolution `bg` with every pane's texture drawn on it as its card, then
  blurred like the backdrop (`render.rs:24-30`, :18-20 the levels: ¼, ⅛, 1/16), built when `pages.rs:163-177`
  `page_blur` says so (`render.rs:1148-1155`): an overlay's glass over a pane follows every stale page frame, the
  ambience alone at `AMBIENCE_HZ` = 8 (`render.rs:99-111`) through the scene's own wake-up (`backdrop.rs:655-675`
  `schedule_pages`, `page_wake`).
- The `GlassLayer`/`over-on` path: `over-on` is the scene running and no page fullscreen (`app.slint:117-121`); a
  panel with `over-layer > 0` then lets the scene draw its material (`kit/material/glass.slint:32` `over-wgpu`), the
  scene draws overlay glass into one window-sized image per layer, above the pages (`app.slint:1351-1358`
  `over-card`, :1725-1730 `over-top`; the peek's in `kit/chrome/sidebar.slint:64`). `Facts.glass` is `over-on`
  (`host/layers.rs:251`).
- The glass sweep: each panel reports its rectangle as data (`glass.slint:58-80`, `GlassLayer.report`); the list
  reaches the scene in `Glass::frame` (`backdrop.rs:358-360`, `set_items`) at the start of each render, before the
  panes' step (`main.rs:498-515`). Glass is opaque inside its shape, its shadow black at the shadow's alpha
  (`shaders/glass.wgsl:203-204`).
- The ambience: the glass under the page mixes in `P` (`render.rs:99-111`); `mark_damage` is what makes `P` stale
  for it. While a pane is on its layer nothing feeds the presenter, so both the overlay glass and the ambience blur
  the pane's last composited frame: that is why Stage 2 held glass on the shell.
- Partial rendering: a scene frame marks only the overlay glass rects (padded by `GLASS_PAD`) and the strips round
  a single pane (`backdrop.rs:449-481`, :723-748 `scene_dirty_rects`); a page frame through the shell marks its
  damage in the pane (`panes.rs:938`).

### The snapshot source: the layer's own pool, no import
Yes. The layer's latest frame (`host/page_layer.rs:72-74`, kept at :448) is a `welding::NativeFrame::Copied` whose
`texture` is already a `wgpu::Texture` over the pool slot's IOSurface: `copy_metal_to_owned` copies into the slot's
Metal texture (`vendor/welding/src/native_frame/metal.rs:166-168`) and grafts that texture into wgpu
(:200-216); `CopiedFrame` says "`texture` is the same memory" (`native_frame/mod.rs:548-564`), and
`WgpuTextureImporter::import` of a copied frame only moves it out (`mod.rs:796`). The slot is not copied into again
while the frame lives (`metal.rs:358-360` `in_use`: a live `OsBuffer` clone; `latest` and `layer.rs:105,191-192`
`shown` hold them). So the host can read the latest pool surface by reference, on demand, with nothing new
imported and the composited path left off (no `Present`, no `mark_damage`, no Slint damage). The one step needed:
the pool texture is `Bgra8UnormSrgb` (`metal.rs:143`) while the scene samples display-encoded `Rgba8Unorm`
(`wubble-gfx/src/import.rs:283-288`), so it goes through the presenter's existing blit, which re-encodes
(`import.rs:589-603`), into the pane's own ring target, in place (`FramePresenter::present` consumes the frame,
`import.rs:394-412`; the new `refresh` borrows it). The pane draws its hole, not that image, so Slint sees nothing
change; `P` is marked stale and the scene's next frame rebuilds it. Cost per refresh: one full-size GPU blit
(1080p: about 8 MB read and written) and one `P` build, a few times a second.

### Low resolution
Nothing new: `P` draws the pane's texture at the scene's half resolution and the glass reads its ⅛ and 1/16 levels
(`render.rs:18-20`, :24-26). The snapshot is low resolution by the blur that already runs; a second shrink pass
would only add one. "Slow refresh" is the new part: `page_layer::SNAPSHOT_MS`.

### Which rows use the snapshot, which still hold
- Snapshot (every row with glass and a cover, the scene's glass on; `overlay.rs:620-623`): Notice, Permission,
  Find, Dialog, BookmarkSheet, PrintSheet, ExtensionSheet, AgentSheet, Region, CommandBar, Downloads, Extensions,
  Spaces, Menu, ErrorPanel, Boost, Toast, Preview, AutofillMenu, AutofillPrompt, VideoStrip, VideoTab, VideoSpeeds,
  PipStrip, Reader, Glance, Peek. They become `Coverage::Glass`.
- Still hold (live pixels in the shell's textures): no overlay. `Coverage::Pixels` stays for Entrance and SeamBow,
  which are not glass but move or cut the page itself (the shell draws the page scaled or masked); `Need::Resizing`
  (the stretch); `Need::Pixels` (a shot, a pixel check, a snapshot or the preview card's thumbnail, which already
  hand the layer's latest frame over: `layer_pixels`). `Need::Unplaced` is a place, held for long as before.

### Window-scope glass counts against every pane
`overlay.rs:617`: only `Scope::Pane` rows are filtered by pane; a menu, a toast or the tab preview over the sidebar
counts for every pane. The rectangles are known only to the scene (the glass sweep), so the host makes
`Facts.glass` per pane: `over-on` and some overlay glass surface's rectangle meets the pane's
(`Renderer::overlay_over`, the test `page_in` already does for all panes, `render.rs:827-829`). Non-glass rows
stay as they are: in `window` they hold nothing, and in `fullscreen` the pane is the whole window.

### The hole under an overlay
No change. The hole is a `Clear` draw at its place in the tree (`vendor/i-slint-renderer-skia/itemrenderer.rs:728-729`);
the overlay layers' images and the overlays' own content come later in the tree (`app.slint:1351`, :1725) and draw
over the cleared pixels as usual, so under a glass panel the shell's pixels are the glass (opaque inside its shape,
the shadow and anti-aliased rim blending over the layer as they would over the page). A dirty rect over the overlay
redraws hole and glass together in order. While a glass fades in or out its alpha shows the live page under it, as
over a composited page.

### Design
- `Coverage::Glass` (between `Part` and `Pixels`), from the row data that already says glass (`row.glass`), not a
  branch; `Need::Frosted` is it, `Hold::Frost` (between `Free` and `Brief`) what it does: `window` `Frost`,
  `fullscreen` `Long` (unchanged). Under `Frost` the page stays on its layer; the machine emits `Effect::Snapshot`
  when the snapshot is due: at once when glass comes over the page, then at a frame or a render once
  `SNAPSHOT_MS` (250) passed and the layer shows a newer frame. `Route::Layer` carries the last one (`Snap`: which
  frame, when); no glass, no snapshot (the field goes back to `None`). No timer: frames and renders drive it, so idle
  stays idle. A page that stops (pause, end of a scroll) keeps the snapshot taken before its last frame until the
  next render: at most `SNAPSHOT_MS` of the page behind, blurred.
- The host's snapshot slot is the pane's presenter ring target (no new texture): `Effect::Snapshot` blits the
  layer's latest frame there (`FramePresenter::refresh`) and marks `P` stale (`Glass::pages_written(.., None)`); the
  scene's frame then marks only the overlay rects and the strips round the pane.
- A brief hold expires: `Route::Entering { expires }` carries its deadline (`BRIEF_MS` = 1000 after the hold began
  there); `Route::hold` makes a brief hold past it a long one, so the next event sends the pane to `Composited` and
  lets the buffers go; `Composited` stays while anything holds it (it went there for long), and comes back at the
  first event free of holds. The host reads the clock once per event (`surface_now`, the store's clock).
- Diagnostics: the summary line gains `layer_snapshots`; a route change by an expired hold ends in
  `(brief hold expired: Long)`.

### As written (blind)
Commits on `untested/page-layer-2` after 08b719e: this section; core (`Coverage::Glass`, `Need::Frosted`,
`Hold::Frost`, `Effect::Snapshot`, `Route::Layer { snap }`, `Route::Entering { expires }`, `Route::hold`,
`SNAPSHOT_MS`, `BRIEF_MS`, their tables and walks); gfx and host (`FramePresenter::refresh`, `pages::overlay_over`,
`Renderer`/`Glass::overlay_over`, `Glass::pages_written(.., None)`, per-pane `page_covers`, the `Snapshot` executor,
`layer_snapshots`, the expiry note on route lines). No kit or shell change: the hole and the glass images need none.
`--page-layer=off` runs none of it (no pane has a layer); `fullscreen` holds every need for long, as before. Flags
unchanged (`docs/smokes.md` untouched). Not done: a trailing snapshot after a page's last frame when nothing renders
(the glass may show the page up to `SNAPSHOT_MS` behind until the next render); a per-kind glass rectangle (the
per-pane fact is "some overlay glass meets the pane", so the log may name a toast elsewhere as `Glass` while a menu is
over the pane).

### Owner's hand-check list for this fix (macOS, release build)
Command: `cargo run -p wubble --release -- --page-layer=window <1080p video page>`; stderr shows the route lines and
the per-second summary (`wubble: page layer: 1.00s: ...; tab N Layer; ...`). Stage 2's item 5 changes: glass now
blurs a snapshot refreshed about 4 times a second and the pane stays on its layer.
1. A popup left up (SponsorBlock's sticky notice, or a permission prompt) over the playing video for 30 s: no route
   line after the first `Entering -> Layer`; each summary has `layer_shows` near 60, `layer_snapshots 3` or `4`,
   `slint_renders` about 4 to 10 and `tab N Layer`; main process near the 10-12 % of no overlay, not 30 %. The glass
   shows the video moving, in steps.
2. A toast (Mod+= for "125%") over the video: no route line; `layer_snapshots` while it shows, none a second after it
   went.
3. The pointer moved over the video and kept moving for 10 s (the strip shows): no `Layer -> Leaving` line;
   `layer_snapshots` at most 4 a second; the strip's glass blurs the video under it.
4. A tab's context menu over the sidebar, clear of the page: no `layer_snapshots` in the summary, no route line. The
   page's context menu over the page: `layer_snapshots` appear while it shows.
5. Resize: drag the window's edge and keep dragging for 2 s: `Layer -> Leaving on Changed; needs: Windowed, Resizing
   Brief [stretch ...]`, `Leaving -> Entering on Presenting ...`, about a second later `Entering -> Composited on
   Frame { os: true, ... }; needs: Windowed, Resizing Brief [stretch ...] (brief hold expired: Long)` (or `on
   Changed`); on release `Composited -> Entering on Changed; needs: Windowed`, then `Entering -> Layer on Frame`.
   A sidebar spring (under a second) shows `Leaving`, `Entering`, `Layer` and no `expired`.
6. `--measure idle --seconds 4` with `window` and nothing over the page: 0 to 2 `slint_renders`; with a sticky notice
   over a still page: one `layer_snapshots` as it opens, then nothing more.
7. By eye against `--page-layer=off` on the same page: the glass over the page looks the same (blur, tint, its
   shadow and rim over the live page at its edges, no seam); as a glass fades in, its first frame may blur an older
   frame of the page [u]; the pane's rounded corners and the hole unchanged elsewhere.

### Lines for TODO.md / decisions.md (not edited here)
- decisions.md: "Page layer: glass over a page on its layer blurs a snapshot of the layer (`Coverage::Glass`,
  `Hold::Frost`), refreshed at most every `SNAPSHOT_MS` (250); only glass whose rectangle meets the pane counts; a
  brief hold expires to long after `BRIEF_MS` (1 s)."

## Default on macOS (0.6.2)
The owner's M2 run with the overlay fix (2026-11-24, 1080p60 video, `~/Downloads/wubble-layer-log-3.txt`): 159 of
166 s on Layer, through overlays; left only for a resize and the SeamBow at start, each ended by the one-second rule;
`slint_renders` median 8 a second, `layer_snapshots` 4. Owner's decision the same day: on by default on macOS.

### Dropped frames (`pool_busy`)
0 before the fix, 67 over 159 s after (in 43 seconds, up to 4 a second). Read from the code: the snapshot holds no
pool slot past its blit. `Effect::Snapshot` borrows the layer's latest frame (`pane.latest`, the same slot as the
layer's `shown[0]`), `FramePresenter::refresh` submits its blit at once (`Blitter::run` ends in `queue.submit`) and
keeps no `OsBuffer`; so it adds no hold the layer does not already have. The holds as a frame is copied: the layer's
frame and the one before (`CaLayer::shown`, `OsBuffer` clones), a frame copied but not yet taken (the pool's
`latest`, never chosen), and whatever the compositor still reads (`IOSurfaceIsInUse`). The busy seconds are the
seconds with the most frames (28 seconds with one drop average 60 frames, the 120 without any 46): frames that come in
a burst, several Core Animation commits within one display frame, leave the compositor holding two older surfaces
while the layer holds two, and four slots are gone. Fixed with a fifth slot (`POOL_SLOTS`; 20 MB more per layered
pane at 2732 x 1866 px, 31 MB for a full 3456 x 2234 window), table test `every_holder_at_once_leaves_the_next_frame_a_slot`.
A frame still dropped as `Busy` is lost: if it was a burst's last, the layer shows the one before until the page
paints again [u].

### The frame rate on the layer
`cef_frames` median 49 a second for the 60 fps video (run 1 before the fix: 62; run 2: 73, another page). The drops
explain under one frame a second of the 11 missing. The counts swing 30 to 70 from one second to the next (run 1: 58
to 74), so frames reach the host late and in bursts. What is new on the layer's frame path: 4 snapshot renders a
second, each a shell render whose drawable is presented with the Core Animation transaction (`presentsWithTransaction`
is on whenever the page is on its layer), so the main thread, which is also CEF's UI thread and runs the frame copy,
waits in that commit for the glass's GPU work; and the copy itself (`waitUntilCompleted`) waits behind that work. Our
path, likely, unmeasured: not changed blind. Next, on the Mac: `--measure` with `window` and a glass surface over
the video for `copy_wait` and the main thread's time in the present; the candidate fix is to present with the
transaction only while a route or a placement changes (a hide, a hole opening or closing, a move), not for a render
that only redraws glass over a layer that stays.

### The default, the switch, the diagnostics
- `page_layer::PLATFORM_MODES`: `("macos", Window)`; every other platform `off` (no layer). The browser takes it;
  every smoke, `--measure`, `--shot-dir` gallery run stays `off` unless it passes `--page-layer`, so they behave as
  before. A plain run with `--shot` takes the default; its pending shot holds the page on the shell (`Need::Pixels`),
  as do the frames smoke's pixel checks.
- Settings, Performance, "Show pages on their own layer (macOS)" (`settings::PAGE_LAYER`, `page.layer`, restart):
  off, the browser starts `off`. `--page-layer=off|fullscreen|window` overrides both.
- The route lines and the per-second summary print only with `WUBBLE_TRACE=1` (as `wubble: trace +Nms pid P: page
  layer: ...`); a failure (`wubble: page layer: ...`) always prints.

### Owner's hand-check list for a default-on release (macOS, release build, by risk)
Run the release app with no flag and `WUBBLE_TRACE=1` from a terminal; each item by eye against Settings' row off.
1. Split view with two pages (two layers): both show, each in its pane with its corners; no page over the other or
   over the seam.
2. Sidebar toggle, peek and spring: the pane's page follows the pane in the same frame, no gap or doubled edge; the
   peek's page stays on the shell (`Unplaced`).
3. Split seam drag: both pages follow the seam, stretch then settle; no stale strip under the seam.
4. Tab switch, and closing the shown tab: the new page at once, never the old one through the hole; no black pane.
5. Typing with an input method (Japanese, Chinese) and dead keys (´ then e): the candidate window at the caret, the
   text in the page.
6. Drag and drop into the page (a file, a link) and out of it (a picture, selected text): the ghost drawn over the
   page, the drop lands.
7. Find bar and command bar over the page: glass over the live page, the page stays on its layer (no route line).
8. A context menu over the page: drawn over it, its glass blurring the page.
9. `<select>`: the popup over the page, opens where the select is, picks.
10. Picture-in-picture: the video in the Pip window, back into the page.
11. Page fullscreen and back: fullscreen video smooth; on exit the pane right, no stale frame.
12. Second display and moving the window to it; a display change (unplug, resolution): page sharp, placed right.
13. Window resize: the stretch then the page sharp on its layer again within about a second.
14. Minimise and restore: the page there at once, live.
15. Sleep and wake: the page live after wake, not a stale frame.
16. Screenshots (the OS's, and `--shot`): the OS's shows the page (the layer is in the window); `--shot` a page.
17. A private window: its pages on their layer too, its tint and chip over them.
18. An external 60 Hz display: video at 60 a second (the summary's `cef_frames`), no dropped frames (`pool_busy 0`).
