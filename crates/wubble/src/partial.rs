//! Partial rendering (Slint's Skia renderer, vendored in `vendor/i-slint-renderer-skia`): a
//! frame redraws only the region whose items changed, so a page frame redraws its pane and not
//! the sidebar and bars. Slint sees a change through its properties; a texture written in place
//! behind an image that stays the same (the scene's, `Renderer::before_rendering`) it cannot
//! see, so whoever writes one marks the window here.

use i_slint_core::lengths::{LogicalPoint, LogicalRect, LogicalSize};
use i_slint_core::window::WindowInner;
pub use i_slint_renderer_skia::partial::{Totals, mode, totals};

/// The next frame of `window` redraws all of it. Called from the rendering notifier, it is the
/// frame being drawn (the notifier runs before the dirty region is computed).
pub fn mark_window(window: &slint::Window) {
    let size = window.size().to_logical(window.scale_factor());
    let rect = LogicalRect::new(
        LogicalPoint::default(),
        LogicalSize::new(size.width, size.height),
    );
    WindowInner::from_pub(window)
        .window_adapter()
        .renderer()
        .mark_dirty_region(rect.into());
}

/// `--measure`'s line: frames drawn, how many redrew part of the window, the mean share redrawn
/// and the items drawn per frame (Slint's draw calls reaching Skia).
pub fn report(totals: Totals) -> String {
    let frames = totals.frames.max(1) as f64;
    format!(
        "partial: mode={:?} frames={} partial={} dirty_share_mean={:.3} items_per_frame={:.1} verify_mismatches={}",
        mode(),
        totals.frames,
        totals.partial,
        totals.dirty_share_millionths as f64 / 1e6 / frames,
        totals.items_drawn as f64 / frames,
        totals.verify_mismatches,
    )
}

/// The counts since `start`.
pub fn since(start: Totals) -> Totals {
    let now = totals();
    Totals {
        frames: now.frames - start.frames,
        partial: now.partial - start.partial,
        dirty_share_millionths: now.dirty_share_millionths - start.dirty_share_millionths,
        items_drawn: now.items_drawn - start.items_drawn,
        verify_mismatches: now.verify_mismatches - start.verify_mismatches,
    }
}
