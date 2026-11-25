# wubble changes to i-slint-renderer-skia

Vendored from crates.io `i-slint-renderer-skia` 1.19.0 (repository slint-ui/slint), licensed
`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`; wubble
takes it under GPL-3.0-only. Copied unmodified: everything the published crate ships
(`Cargo.toml` is the registry's normalised manifest). Added: `partial.rs`, `rustfmt.toml` and
this file (and `presenting.rs`, change 2; `hole.rs`, change 3; `cache.rs`, change 4; `present_mode.rs`, change 5). Patched in through the root `Cargo.toml`'s `[patch.crates-io]`.

The copy is excluded from the wubble workspace, so `cargo clippy --workspace` does not lint it;
`rustfmt.toml` sets `disable_all_formatting = true` to keep upstream formatting and a diff
against 1.19.0 that holds only the changes below. It is tested through wubble-ui's
`partial_render` and `crisp_text` tests. When moving to a newer Slint (the `slint`, `wgpu`,
`welding` and `cef` pins move together), start from the new upstream release and re-apply each
change, or drop the patch if upstream has partial rendering on its wgpu surface by then.

## Changes

Every change is marked `wubble change` in the source.

1. **Partial rendering on the wgpu 30 surface.** Upstream's wgpu surface draws every frame whole
   (buffer age 0, a cleared swapchain image), so one page frame cost a redraw of the whole shell.
   - `wgpu_30_surface.rs`: the windowed surface keeps the frame Skia draws into between frames
     (`retained_target`: a texture of the swapchain's size and format, cleared once through wgpu
     so wgpu counts it initialised), renders into it with buffer age 1 when it holds the last
     frame (0 after a resize or a new texture: whole), and copies it into the swapchain image
     with `copy_texture_to_texture` before presenting (the swapchain takes `COPY_DST`; without
     it the surface keeps upstream's path). `use_partial_rendering` is true.
   - `lib.rs`: `default_wgpu_30` makes the partial rendering state; `partial_rendering_state`
     keeps it although a rendering notifier is set (upstream turns it off then, as a notifier
     might draw into the target: wubble's draws only into its own textures).
   - `lib.rs` `render_to_canvas`: with partial rendering the `BeforeRendering` notifier runs
     first, before `draw_contents` (upstream: after the clear, which a notifier drawing into the
     target needs). `draw_contents` instantiates the tree's `if` and `for` elements before it
     calls the renderer, so an element a notifier shows (wubble's exit page on a tab switch)
     appeared one frame late, and a frame showed the pane empty, upstream too.
   - `lib.rs` `render_components_to_canvas`: the dirty region is rounded out to whole physical
     pixels before items are tested against it. The clip already was, so an edge pixel the
     logical region only partly covered was cleared and redrawn without the items that cover
     only its other part: a one-pixel residue.
   - `lib.rs` `take_snapshot`: the next frame is drawn whole (a snapshot's render takes the
     changes it draws off the partial renderer's books; the kept frame has not seen them).
   - `wgpu_renderer.rs`: `SkiaWGPU30Renderer::render_to_kept_texture` renders offscreen the
     same way (the test's path); `render_to_texture` forgets the kept texture.
   - `partial.rs` (new): the switch `SLINT_SKIA_PARTIAL_RENDERING` (unset: on; `off`: upstream's
     whole frames; `verify`: on, and every presented frame is compared with a whole redraw, a
     readback per frame, mismatches logged), and the counts `--measure` prints (frames, the
     share of the window redrawn, items drawn: `itemrenderer.rs` counts each draw call).
   - `lib.rs` `verify_presented`, `Surface::read_presented`, `Surface::render_reference`: the
     `verify` mode; the reference is drawn whole into a texture like the kept frame (a Skia
     snapshot surface anti-aliases some edge pixels differently).

   A texture Slint shows and the application writes in place behind an image that stays the
   same is invisible to the partial renderer: its writer marks the window dirty
   (`crates/wubble/src/partial.rs`, the scene's frames).

2. **A hook before each present.** wubble hides a page's OS layer (`wubble_core::page_layer`) only
   once the shell presents a frame that drew the page, in the same Core Animation commit as that
   frame; upstream's `pre_present_callback` is the winit backend's own, out of the application's
   reach.
   - `presenting.rs` (new): a thread-local hook (`set_hook`) called with the window (`notify`).
   - `wgpu_30_surface.rs` `render`: the window parameter (unused upstream, `_window`) is passed to
     `presenting::notify` right after `pre_present_callback`, before both `queue.present` calls.
   - `lib.rs`: `pub mod presenting`.

3. **The page hole.** wubble shows a pane's page on an OS layer under the window's surface
   (`wubble_core::page_layer`, stage 2) and needs the shell's frame transparent there, though the
   window's ground, glass and the pane's fill were drawn under the pane first: a draw that removes
   pixels, which Slint has no element for.
   - `hole.rs` (new): a sentinel image (`image()`: one 1x1 RGBA8 buffer per thread; an image is
     the hole when it equals it, i.e. shares its buffer) and the `BlendMode::Clear` paint.
   - `itemrenderer.rs` `draw_image`: an image element showing the sentinel draws its geometry with
     that paint instead of the image, so it clears to transparent within the current clip (a
     rounded clip gives rounded, anti-aliased corners).
   - `lib.rs`: `pub mod hole`.
   wubble's kit draws it as `PageHole` (`crates/wubble-ui/ui/kit/material/page-hole.slint`); tested
   by wubble-ui's `partial_render` (its last rows: the hole and the window around it).

4. **Skia's resource cache, bounded and cleaned up.** Upstream keeps Skia's default cache (256
   MiB) and its wgpu surface has no way to free it; Chromium bounds its contexts' caches and
   cleans them when idle, hidden or under pressure (`GrCacheController`, the idea only).
   - `cache.rs` (new): every windowed wgpu 30 surface's `DirectContext` on the thread
     (`register`); `set_limit` (`set_resource_cache_limit` on each, and on each made later),
     `cleanup(CacheCleanup::Deferred(unused) | PurgeAll)` (`perform_deferred_cleanup`,
     `purge_unlocked_resources`), `bytes` (`resource_cache_usage`). A context that is rendering
     is skipped.
   - `wgpu_30_surface.rs`: `gr_context` is an `Rc<RefCell<..>>`, registered in `new`.
   - `lib.rs`: `pub mod cache` (with `wgpu-30`).
   wubble's policy is `wubble_core::skia_cache` (when), run by the host's lifecycle tick, its
   renders and its windows' occlusion. Not tested here (a GPU is needed).

5. **A present mode the application sets, paced.** Upstream presents `AutoVsync` (FIFO)
   always. wubble's Wayland page layer (`wubble_core::page_layer`, a `wl_subsurface` under the
   window) needs Mailbox while a layer exists: FIFO with a synced subsurface stalled mutter's
   headless display (wubble `docs/research/linux-page-layer.md`, section 7); Mailbox presents as
   often as it is asked (7.x), so it is paced here.
   - `present_mode.rs` (new): a thread-local hook (`set_hook`) asked with the window before each
     frame (`PresentMode::Vsync` or `Paced(period)`; `None` keeps the last answer), `wait` (what
     is left of the period since the last present) and `wgpu_mode` (Mailbox only where the
     surface offers it, else `AutoVsync`); table tests. `summary` says, for a log line, what the
     last surface made on the thread offers (`offered`, recorded as it is made) and which mode
     each `PresentMode` takes there.
   - `wgpu_30_surface.rs`: the surface keeps its offered present modes, its current mode and its
     last present's time; `render` first calls `follow_present_mode` (reconfigures the swapchain
     on a change, before the frame is acquired; a paced Mailbox frame sleeps out the rest of the
     period, as FIFO's acquire would block), and both present paths stamp the time.
   - `lib.rs`: `pub mod present_mode` (with `wgpu-30`).
   Without a hook every window presents as upstream.

