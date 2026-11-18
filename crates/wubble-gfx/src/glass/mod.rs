//! Glass backdrops: the blurred, saturated web content behind glass panels.
//!
//! wubble draws its own glass (TODO.md, Decided). Slint 1.19 has a native
//! `backdrop-blur` on `Rectangle` (femtovg included), but no saturate, no
//! per-tier sharing and no intermediate quality, so the backdrop is made here
//! and Slint only layers tint, specular, rim and shadow on top of it.
//!
//! - [`Tier`]: Thin / Regular / Thick, the blur and saturate table.
//! - [`Quality`] and [`next_quality`]: the degrade ladder
//!   (Full -> HalfRes -> Cached -> Flat) and the pure governor that moves it.
//! - [`GlassBackdrop`]: the GPU pass. One shared dual-Kawase down chain from
//!   the frame, one up chain per tier in use, saturate folded into the last up
//!   pass, outputs double-buffered per tier as `slint::Image`s.
//!
//! # Host call sequence (wired in the next batch)
//!
//! Per web frame, in the same host tick as `FramePresenter::present` and
//! before Slint renders:
//!
//! ```ignore
//! let image = presenter.present(native)?;
//! let frame = presenter.current_texture().expect("just presented");
//! let backdrops = glass.render(frame, quality, tiers_in_use, window.scale_factor())?;
//! // Set the `GlassBackdrops` Slint global: thin / regular / thick from
//! // `backdrops` (an empty image where None),
//! // `scale` = backdrops.px_per_frame_px * window.scale_factor(),
//! // `quality` = the Slint mirror of `quality`, frame-x / frame-y = where the
//! // web frame sits in the window.
//! ```
//!
//! The host measures the cost of each frame it renders, keeps the last
//! [`quality::WINDOW`] costs, and once per frame sets
//! `quality = next_quality(quality, &costs, budget)` with the 8.33 ms budget,
//! clearing its window whenever the quality changes. Glass over a static page
//! still needs one `render` after the shell starts showing glass.

mod pass;
pub mod plan;
pub mod quality;

pub use pass::{Backdrops, GlassBackdrop};
pub use plan::{Tier, TierSet};
pub use quality::{Quality, next_quality};

#[cfg(test)]
mod tests;
