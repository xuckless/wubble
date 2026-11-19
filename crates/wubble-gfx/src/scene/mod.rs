//! The living backdrop and real glass under the UI: the owner's design scene
//! (the-wallpaper-project `src/wubble/slint`, `render.rs` + `shaders/`) on
//! wubble's shared wgpu device.
//!
//! The Space's colours lie in a dye on the dark ground: a stable-fluids liquid
//! that drifts (the design's, [`Backdrop::Liquid`]) or, in the browser window, the
//! same dye held still ([`Backdrop::Still`], owner override 2026-11-19); taps send
//! a ring wave through it. Glass panels are the design's coloured frost (598f814):
//! the two deepest levels of a blurred pyramid of that backdrop, the light pulled
//! toward a milky floor and the colour lifted, a slight bend at the rim, grain, a
//! soft pointer light and inner edge, the sheen ring while touched, press waves and
//! the border beam. Everything is drawn into one `Rgba8Unorm` texture that Slint
//! shows at the back of the window; Slint draws the chrome's content (text, icons)
//! on top.
//!
//! - [`Scene`]: the CPU state the host drives (glass rects, palette, pointer,
//!   taps, beam). Pure and unit-tested.
//! - [`Renderer`]: the GPU half; owns a `Scene` as [`Renderer::scene`].
//! - [`Quality`] and [`Governor`]: the degrade ladder. Glass degrades before
//!   animation drops a frame: lower levels shrink the backdrop, then refresh
//!   it every 8th frame (at its real speed), then freeze it and draw the
//!   glass as a flat fill.
//!
//! # Host wiring
//!
//! The window has one rendering notifier (Slint 1.19 refuses a second one with
//! `SetRenderingNotifierError::AlreadySet`), owned by the `wubble-ui` driver,
//! so the scene does not install its own: the host (`crates/wubble/src/backdrop.rs`)
//! builds it with [`Renderer::try_new`] on the `GpuContext` (falling back to
//! Slint-only glass on `Err`) and calls [`Renderer::before_rendering`] from
//! the driver's per-frame hook, setting the returned image as the window's
//! `scene`. It feeds the scene from the window's state:
//!
//! - `scene.set_glass(&rects)` whenever the panels' reported list changes
//!   (back to front, logical px, at most [`MAX_GLASS`]);
//! - `scene.set_palette(space_colours)` on a Space switch (600 ms crossfade);
//! - `scene.pointer_move(x, y)` / `pointer_leave()` from the window's pointer
//!   events, `scene.tap(x, y)` on a press;
//! - the address bar's rect marked `beam` in that list, and `scene.set_beam_shape(shape)`
//!   every frame (the shell's `wubble_core::beam` owns the lap);
//! - `renderer.set_quality(governor.quality())` after
//!   `governor = governor.next(cost, budget)`, for each GPU frame cost
//!   [`Renderer::take_cost`] hands back (GPU timestamps; none without them).
//!
//! # What the window needs
//!
//! - An `Image` at the very back filling the window (`source: scene`,
//!   `image-fit: fill`), then the web view, then the chrome.
//! - A transparent Slint background (`Window { background: transparent; }`)
//!   and transparent glass panels: the glass material is in the scene image,
//!   so Slint draws only their content. Any opaque Slint fill hides it.
//! - The web view stays opaque above the scene with its own radius and gap. The
//!   scene also gets each pane's latest frame ([`Renderer::set_pages`]): overlay
//!   glass over a pane frosts the page, drawn above it in one image per
//!   [`OverLayer`] ([`Frame::over`]); the glass under the page picks up the page's
//!   light at its edge (the ambience). See [`pages`].
//! - No OS window transparency: the scene image is opaque.
//! - Frames from the host by the run state ([`run`]): every vsync while something moves,
//!   none once nothing does (`Still`) or while the window is hidden. A still backdrop's
//!   composite and blur pyramid render only when its dye, its ripples or its size change;
//!   the other frames redraw the glass over them. `before_rendering` does not ask for the
//!   next frame itself; [`Renderer::animating`] says whether it wants one.
//!
//! Left out: the design's start page (bubble, pops, minis, soft body, voice
//! glow, W field: `bubble.wgsl`, `pop.wgsl`, `wfield.rs`, `softbody.rs`,
//! `minis.rs`, `voice.rs` in the bucket) and its overlay mode. glass.wgsl is
//! the design's with the voice uniforms sent as zero, plus one branch:
//! `uMaterial` 2, the flat fill of [`Quality::Flat`].

pub mod pages;
mod quality;
mod render;
pub mod run;
mod state;

pub use pages::{OverLayer, PageBlur, PageBlurIn, PageRect, by_layer, overlaps, page_blur};
pub use quality::{
    AVG_WEIGHT, BackdropClock, CACHED_REFRESH_CALLS, DOWN_FRACTION, FIT_FRACTION, Governor,
    HELP_FRACTION, HOLD_FRAMES, MAX_BACKDROP_DT, MAX_BACKOFF, MAX_DT, PROBE_FRAMES, Quality,
    SAMPLE_CAP, SETTLE_FRAMES, Signal, UP_FRACTION, should_render,
};
pub(crate) use render::with_prelude;
pub use render::{
    AMBIENCE_HZ, AMBIENCE_LEVEL, AMBIENCE_STRENGTH, BLUR_SIGMA, DPR_MAX, Frame, GLASS_PAD,
    PYRAMID_DIV, PagePane, Renderer, ambience_every, out_size, readback_rgba8, view_of,
};
pub use state::{
    Backdrop, Beam, Blob, FluidOptions, GlassRect, GlassSurface, MAX_GLASS, Rect, Rng, STILL_DYE,
    Scene, SceneOptions, Tier, film_level, hex, still_blobs, surface_of,
};
