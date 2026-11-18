//! Glass backdrops: the blurred, saturated web content behind glass panels.
//!
//! wubble draws its own glass (TODO.md, Decided). Slint 1.19 has a native
//! `backdrop-blur` on `Rectangle` (femtovg included), but no saturate, no
//! per-tier sharing and no intermediate quality, so the backdrop is made here
//! and Slint only layers tint, specular, rim and shadow on top of it.
//!
//! - [`Tier`]: Thin / Regular / Thick, the blur and saturate table.
//! - [`Quality`] and [`Governor`]: the degrade ladder
//!   (Full -> HalfRes -> Cached -> Flat) and the pure governor that moves it,
//!   with a retry backoff so steady load cannot flip it between two levels.
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
//! // `backdrops.get(tier)` (an empty image where None; empty means flat),
//! // `scale` = backdrops.px_per_frame_px * window.scale_factor(),
//! // frame-x / frame-y = where the web frame sits in the window.
//! ```
//!
//! `quality` is `governor.quality()`. The host keeps one [`Governor`]
//! (`Governor::new(Quality::Full)`), measures the cost of each frame it
//! renders and, once per frame, sets `governor = governor.next(cost, budget)`
//! with the 8.33 ms budget. Glass over a static page still needs one `render`
//! after the shell starts showing glass.

mod pass;
pub mod plan;
pub mod quality;

pub use pass::{Backdrops, GlassBackdrop};
pub use plan::{Tier, TierSet};
pub use quality::{Governor, Quality};

#[cfg(test)]
mod tests;
