//! The sidebar's Space swipe. A pure rule; the host feeds the scroll deltas, with time passed in.
//!
//! Not in the design bucket (it has no Space swipe): written new. The new-gesture gap is the
//! scroller's (`browser/scroller.tsx` `newGestureMs`, 260 ms).

/// px of horizontal travel before a swipe switches the Space.
pub const SWIPE_THRESHOLD: f64 = 60.0;
/// ms without a scroll event after which the next one starts a new gesture.
pub const SWIPE_GAP_MS: f64 = 260.0;
/// A swipe's horizontal travel must be this many times its vertical travel, so the sideways
/// drift of a long list scroll never adds up to a swipe.
pub const SWIPE_AXIS_RATIO: f64 = 2.0;

/// A two-finger horizontal swipe on the sidebar: one Space per gesture, ends do not wrap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceSwipe {
    acc: f64,
    /// The gesture's vertical travel, px (sum of |dy|).
    travel_y: f64,
    last: f64,
    /// This gesture already switched: the rest of it is ignored.
    spent: bool,
}

impl Default for SpaceSwipe {
    fn default() -> Self {
        Self {
            acc: 0.0,
            travel_y: 0.0,
            last: f64::NEG_INFINITY,
            spent: false,
        }
    }
}

impl SpaceSwipe {
    /// A scroll delta (`dx`, `dy`; px, web sign: negative `dx` scrolls left) at `now` ms. With
    /// the Space at `active` of `count`, the index to switch to, if this delta completes a swipe:
    /// a negative `dx` goes to the next Space, a positive one to the previous. A gesture whose
    /// horizontal travel is not [`SWIPE_AXIS_RATIO`] times its vertical travel is a list scroll.
    pub fn scroll(
        &mut self,
        dx: f64,
        dy: f64,
        now: f64,
        active: usize,
        count: usize,
    ) -> Option<usize> {
        if now - self.last > SWIPE_GAP_MS {
            *self = SpaceSwipe::default();
        }
        self.last = now;
        if self.spent {
            return None;
        }
        self.acc += dx;
        self.travel_y += dy.abs();
        if self.acc.abs() < SWIPE_THRESHOLD || self.acc.abs() < SWIPE_AXIS_RATIO * self.travel_y {
            return None;
        }
        self.spent = true;
        if self.acc < 0.0 {
            (active + 1 < count).then_some(active + 1)
        } else {
            active.checked_sub(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swipe_switches_once_past_the_threshold() {
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-30.0, 0.0, 0.0, 0, 3), None);
        assert_eq!(s.scroll(-40.0, 0.0, 16.0, 0, 3), Some(1));
        // the rest of the same gesture does nothing
        assert_eq!(s.scroll(-200.0, 0.0, 32.0, 0, 3), None);
        // a pause starts a new gesture
        assert_eq!(
            s.scroll(70.0, 0.0, 32.0 + SWIPE_GAP_MS + 1.0, 1, 3),
            Some(0)
        );
    }

    #[test]
    fn the_ends_do_not_wrap() {
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(100.0, 0.0, 0.0, 0, 3), None);
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-100.0, 0.0, 0.0, 2, 3), None);
    }

    #[test]
    fn the_drift_of_a_list_scroll_is_not_a_swipe() {
        // 1200 px down with 5 % sideways drift, one continuous gesture
        let mut s = SpaceSwipe::default();
        for i in 0..120 {
            assert_eq!(s.scroll(-0.5, 10.0, f64::from(i) * 16.0, 1, 3), None, "{i}");
        }
        // a mostly horizontal swipe with a little vertical wobble still switches
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-30.0, 4.0, 0.0, 1, 3), None);
        assert_eq!(s.scroll(-40.0, -3.0, 16.0, 1, 3), Some(2));
    }

    #[test]
    fn back_and_forth_within_the_threshold_is_not_a_swipe() {
        let mut s = SpaceSwipe::default();
        for i in 0..20 {
            let dx = if i % 2 == 0 { 40.0 } else { -40.0 };
            assert_eq!(s.scroll(dx, 0.0, f64::from(i) * 10.0, 1, 3), None);
        }
    }
}
