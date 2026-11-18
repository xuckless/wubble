//! The living backdrop and real glass under the UI: the owner's design scene
//! (the-wallpaper-project `src/wubble/slint`, `render.rs` + `shaders/`) on
//! wubble's shared wgpu device.
//!
//! A stable-fluids liquid in the Space's colours drifts on the dark ground;
//! the pointer pushes it and taps send a ring wave through it. Glass panels
//! refract a blurred pyramid of that backdrop toward their rim, split per
//! channel, with a pointer light, a 1 px inner rim, the sheen ring while
//! touched, press waves and the border beam. Everything is drawn into one
//! `Rgba8Unorm` texture that Slint shows at the back of the window; Slint
//! draws the chrome's content (text, icons) on top.
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
//! - `scene.set_beam(Some(address_bar))` with the layout and `scene.lap_beam()`
//!   when a page starts loading;
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
//! - The web view stays opaque above the scene with its own radius and gap:
//!   glass refracts the backdrop, never web pixels. Glass over the page is
//!   effects only (the beam is drawn that way).
//! - No OS window transparency: the scene image is opaque.
//! - A redraw every frame while the scene shows (`before_rendering` requests
//!   it); pausing while hidden is not done yet.
//!
//! Left out: the design's start page (bubble, pops, minis, soft body, voice
//! glow, W field: `bubble.wgsl`, `pop.wgsl`, `wfield.rs`, `softbody.rs`,
//! `minis.rs`, `voice.rs` in the bucket) and its overlay mode. glass.wgsl is
//! the design's with the voice uniforms sent as zero, plus one branch:
//! `uMaterial` 2, the flat fill of [`Quality::Flat`].

mod quality;
mod render;
mod state;

pub use quality::{
    BackdropClock, CACHED_REFRESH_CALLS, DOWN_COUNT, DOWN_FRACTION, Governor, MAX_BACKDROP_DT,
    MAX_BACKOFF, MAX_DT, PROBE_FRAMES, Quality, UP_FRACTION, WINDOW, should_render,
};
pub use render::{Renderer, readback_rgba8};
pub use state::{
    BEAM_LAP, Beam, BeamRect, FluidOptions, GlassRect, GlassSurface, MAX_GLASS, Rect, Rng, Scene,
    SceneOptions, Tier, hex, lap_beam_at, surface_of,
};
