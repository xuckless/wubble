//! Touchpad gestures over the window (`wubble_core::gesture`): pinch zooms the visible page in
//! the zoom keys' steps, a sideways swipe over a page goes back or forward with the edge chip
//! (`swipe-chip.slint`) following the fingers, unless the page scrolls sideways with it.
//! Wayland reports both through `crate::wl`; macOS pinch is winit's `PinchGesture`.

use wubble_core::gesture::{self, Pinch, PinchInput, Swipe, SwipeEffect, SwipeInput};
use wubble_core::motion;
use wubble_core::tab::TabId;
use wubble_ui::AppWindow;

use super::Host;

/// How far the chip travels in from the edge at full progress (logical px) before the
/// rubber band, and the band's asymptote (the design's seams and scroll ends, motion.rs).
const CHIP_TRAVEL: f64 = 96.0;
const CHIP_BAND: f64 = 34.0;

#[derive(Default)]
pub(super) struct Gestures {
    pinch: Pinch,
    swipe: Swipe,
    /// The page a swipe is over, and its root's horizontal scroll at the begin.
    page: Option<(TabId, f64)>,
}

/// The chip's travel in from the edge at `progress`: straight to the mark, rubber-banded past
/// it (the travel the fingers go on beyond the commit point).
fn chip_shift(progress: f64, overshoot: f64) -> f64 {
    progress.abs() * CHIP_TRAVEL + motion::band(overshoot.max(0.0), CHIP_BAND)
}

impl Host {
    pub(super) fn pinch(&mut self, input: PinchInput) {
        let (next, steps) = self.gestures.pinch.input(input);
        self.gestures.pinch = next;
        for step in steps {
            self.perform(step);
        }
    }

    /// A swipe's input; a begin `at` a window point (a two-finger scroll) counts only over a
    /// page, one without (a pointer-gestures swipe) is over the visible page.
    pub(super) fn history_swipe(
        &mut self,
        window: &AppWindow,
        input: SwipeInput,
        at: Option<(f32, f32)>,
    ) {
        match input {
            SwipeInput::Begin { .. } => {
                let tab = match at {
                    Some(at) => self.pane_under(window, at),
                    None => self.shown(),
                };
                self.gestures.page = tab.and_then(|t| Some((t, self.tab_view(t)?.scroll_x())));
                if self.gestures.page.is_none() {
                    // Over the sidebar (its own Space swipe) or no page: not a history swipe.
                    return self.swipe_input(window, SwipeInput::End { cancelled: true });
                }
            }
            SwipeInput::Update { .. } | SwipeInput::End { .. } => {
                let scrolled = self
                    .gestures
                    .page
                    .is_some_and(|(tab, x)| self.tab_view(tab).is_some_and(|v| v.scroll_x() != x));
                if scrolled {
                    self.swipe_input(window, SwipeInput::PageScrolled);
                }
            }
            SwipeInput::PageScrolled => {}
        }
        self.swipe_input(window, input);
    }

    fn swipe_input(&mut self, window: &AppWindow, input: SwipeInput) {
        let (next, effects) = gesture::transition(self.gestures.swipe, input);
        self.gestures.swipe = next;
        let overshoot = match next {
            Swipe::Tracking { dx } => dx.abs() - gesture::SWIPE_COMMIT,
            _ => 0.0,
        };
        for effect in effects {
            match effect {
                SwipeEffect::Indicate(p) => {
                    window.set_swipe_progress(p as f32);
                    window.set_swipe_shift(chip_shift(p, overshoot) as f32);
                }
                SwipeEffect::Hide => window.set_swipe_progress(0.0),
                SwipeEffect::Go(action) => self.perform(action),
            }
        }
    }

    /// macOS: winit's `PinchGesture` (`delta` is the magnification change, may be NaN).
    pub(super) fn pinch_gesture(
        &mut self,
        delta: f64,
        phase: slint::winit_030::winit::event::TouchPhase,
    ) {
        use slint::winit_030::winit::event::TouchPhase as P;
        match phase {
            P::Started | P::Moved => self.pinch(PinchInput::Scaled(1.0 + delta)),
            P::Ended | P::Cancelled => self.pinch(PinchInput::End),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_runs_to_the_mark_then_bands() {
        assert_eq!(chip_shift(0.5, 0.0), 48.0);
        assert_eq!(chip_shift(-1.0, 0.0), CHIP_TRAVEL);
        let past = chip_shift(1.0, 1000.0);
        assert!(past > CHIP_TRAVEL && past < CHIP_TRAVEL + CHIP_BAND);
    }
}
