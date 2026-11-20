//! The component sheet's physics, one module per control: the toggle, the hold and confirm
//! buttons (controls kit A), the slider, the segmented control and the swatches' ring (kit B)
//! and the menu's lifecycle, highlight and keys (kit C); here, when a control's film rim shows.
//! Pure; the frame driver steps them and Slint draws their views. The pin, download stretch,
//! dock magnify and tooltip land with their components.
//!
//! Source: the owner's design repo (xuckless/the-wallpaper-project), `src/wubble/pages/touch.tsx`
//! and `pages/settings.tsx`. The bucket has these only as TSX: the loops are ported onto the
//! springs of [`crate::motion`], their constants from `kit_gen`. Check boxes are CSS
//! transitions in the design and stay Slint tweens; so do the menu's curves.
//!
//! Pointer positions are px along the control's own axis; time is `dt` in seconds or `now` in ms
//! passed in, never read.

pub mod confirm;
pub mod hold;
pub mod list_editor;
pub mod menu;
pub mod segmented;
pub mod slider;
pub mod swatches;
pub mod toggle;

pub use segmented::{SegmentView, Segmented};
pub use slider::{Slider, SliderView};
pub use swatches::{Ring, RingView};
pub use toggle::{Size as ToggleSize, Toggle, View as ToggleView};

/// Whether a control's film rim shows: hover, press, drag or focus; never at rest (the design's
/// `wb-film`: "every touched rim").
pub fn rim_on(hover: bool, pressed: bool, dragging: bool, focused: bool) -> bool {
    hover || pressed || dragging || focused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rim_shows_only_when_touched() {
        assert!(!rim_on(false, false, false, false));
        for i in 0..4 {
            let b = |n| i == n;
            assert!(rim_on(b(0), b(1), b(2), b(3)));
        }
    }
}
