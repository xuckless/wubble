//! Touch sizing and the sidebar's Space swipe. Pure rules; the host feeds the input kind and
//! the scroll deltas, with time passed in.
//!
//! Not in the design bucket (it has no touch row size and no Space swipe): written new. The
//! 44 px row is the design's only 44 px size, the favourites tile (`browser.css` `.wb-fav`); the
//! new-gesture gap is the scroller's (`browser/scroller.tsx` `newGestureMs`, 260 ms).

/// The pointer the user last used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pointer {
    #[default]
    Mouse,
    Touch,
}

/// The row size setting (`ui.row_size`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowSize {
    /// Follows the last pointer: touch rows after a touch, pointer rows after a mouse move.
    #[default]
    Auto,
    Pointer,
    Touch,
}

impl RowSize {
    /// From the setting's stored index (0 auto, 1 pointer, 2 touch).
    pub fn from_index(i: i64) -> Self {
        match i {
            1 => RowSize::Pointer,
            2 => RowSize::Touch,
            _ => RowSize::Auto,
        }
    }
}

/// A sidebar row for the mouse (the design's 36) and for a finger (44).
pub const POINTER_ROW: f64 = 36.0;
pub const TOUCH_ROW: f64 = 44.0;

/// The height of a tab row (and a menu row).
pub fn row_height(setting: RowSize, last: Pointer) -> f64 {
    match (setting, last) {
        (RowSize::Touch, _) | (RowSize::Auto, Pointer::Touch) => TOUCH_ROW,
        _ => POINTER_ROW,
    }
}

/// px of horizontal travel before a swipe switches the Space.
pub const SWIPE_THRESHOLD: f64 = 60.0;
/// ms without a scroll event after which the next one starts a new gesture.
pub const SWIPE_GAP_MS: f64 = 260.0;

/// A two-finger horizontal swipe on the sidebar: one Space per gesture, ends do not wrap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceSwipe {
    acc: f64,
    last: f64,
    /// This gesture already switched: the rest of it is ignored.
    spent: bool,
}

impl Default for SpaceSwipe {
    fn default() -> Self {
        Self {
            acc: 0.0,
            last: f64::NEG_INFINITY,
            spent: false,
        }
    }
}

impl SpaceSwipe {
    /// A horizontal scroll delta `dx` (px; negative: the fingers moved left) at `now` ms. With
    /// the Space at `active` of `count`, the index to switch to, if this delta completes a swipe.
    /// Fingers left go to the next Space, right to the previous one.
    pub fn scroll(&mut self, dx: f64, now: f64, active: usize, count: usize) -> Option<usize> {
        if now - self.last > SWIPE_GAP_MS {
            *self = SpaceSwipe::default();
        }
        self.last = now;
        if self.spent {
            return None;
        }
        self.acc += dx;
        if self.acc.abs() < SWIPE_THRESHOLD {
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
    fn row_height_follows_the_setting_then_the_pointer() {
        assert_eq!(row_height(RowSize::Auto, Pointer::Mouse), 36.0);
        assert_eq!(row_height(RowSize::Auto, Pointer::Touch), 44.0);
        assert_eq!(row_height(RowSize::Touch, Pointer::Mouse), 44.0);
        assert_eq!(row_height(RowSize::Pointer, Pointer::Touch), 36.0);
        assert_eq!(RowSize::from_index(2), RowSize::Touch);
        assert_eq!(RowSize::from_index(9), RowSize::Auto);
    }

    #[test]
    fn a_swipe_switches_once_past_the_threshold() {
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-30.0, 0.0, 0, 3), None);
        assert_eq!(s.scroll(-40.0, 16.0, 0, 3), Some(1));
        // the rest of the same gesture does nothing
        assert_eq!(s.scroll(-200.0, 32.0, 0, 3), None);
        // a pause starts a new gesture
        assert_eq!(s.scroll(70.0, 32.0 + SWIPE_GAP_MS + 1.0, 1, 3), Some(0));
    }

    #[test]
    fn the_ends_do_not_wrap() {
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(100.0, 0.0, 0, 3), None);
        let mut s = SpaceSwipe::default();
        assert_eq!(s.scroll(-100.0, 0.0, 2, 3), None);
    }

    #[test]
    fn back_and_forth_within_the_threshold_is_not_a_swipe() {
        let mut s = SpaceSwipe::default();
        for i in 0..20 {
            let dx = if i % 2 == 0 { 40.0 } else { -40.0 };
            assert_eq!(s.scroll(dx, f64::from(i) * 10.0, 1, 3), None);
        }
    }
}
