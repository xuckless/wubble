# wubble changes to i-slint-renderer-skia

Vendored from crates.io `i-slint-renderer-skia` 1.19.0 (repository slint-ui/slint), licensed
`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`; wubble
takes it under GPL-3.0-only. Copied unmodified: everything the published crate ships
(`Cargo.toml` is the registry's normalised manifest). Added: `partial.rs`, `rustfmt.toml` and
this file. Patched in through the root `Cargo.toml`'s `[patch.crates-io]`.

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
   - `lib.rs` `render_components_to_canvas`: with partial rendering the `BeforeRendering`
     notifier runs before the dirty region is computed (upstream: after the clear, which a
     notifier drawing into the target needs), so what it sets in a frame (the shell's springs,
     new images) is drawn in that frame.
   - `lib.rs` `take_snapshot`: the next frame is drawn whole (a snapshot's render takes the
     changes it draws off the partial renderer's books; the kept frame has not seen them).
   - `wgpu_renderer.rs`: `SkiaWGPU30Renderer::render_to_kept_texture` renders offscreen the
     same way (the test's path); `render_to_texture` forgets the kept texture.
   - `partial.rs` (new): the switch `SLINT_SKIA_PARTIAL_RENDERING` (unset: on; `off`: upstream's
     whole frames; `verify`: on, and every presented frame is compared with a whole redraw, a
     readback per frame, mismatches logged), and the counts `--measure` prints (frames, the
     share of the window redrawn, items drawn: `itemrenderer.rs` counts each draw call).
   - `lib.rs` `verify_presented`, `Surface::read_presented`: the `verify` mode.

   A texture Slint shows and the application writes in place behind an image that stays the
   same is invisible to the partial renderer: its writer marks the window dirty
   (`crates/wubble/src/partial.rs`, the scene's frames).
