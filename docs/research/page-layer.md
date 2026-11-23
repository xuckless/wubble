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
