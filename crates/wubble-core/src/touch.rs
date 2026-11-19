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

/// The kind of pointer the user is on: rows grow to 44 px under a finger (`Chrome.rowTouch`).
/// Not in the design bucket (it has the token only): written new.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum InputKind {
    #[default]
    Mouse,
    /// Last touched at this window position (logical px).
    Touch { x: f64, y: f64 },
}

/// A pointer event as the input kind sees it (window position, logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    Touch { x: f64, y: f64 },
    MouseMove { x: f64, y: f64 },
}

/// A mouse move within this many px of the last touch point is the touch's own emulated mouse
/// event (Windows pen/touch compatibility, some X11 setups), not a real mouse.
pub const EMULATED_SLOP: f64 = 2.0;

/// The next input kind: a touch flips to touch, a real mouse move flips back.
pub fn next(kind: InputKind, event: InputEvent) -> InputKind {
    match (kind, event) {
        (_, InputEvent::Touch { x, y }) => InputKind::Touch { x, y },
        (InputKind::Touch { x: tx, y: ty }, InputEvent::MouseMove { x, y })
            if (x - tx).abs() <= EMULATED_SLOP && (y - ty).abs() <= EMULATED_SLOP =>
        {
            kind
        }
        (_, InputEvent::MouseMove { .. }) => InputKind::Mouse,
    }
}

/// The `ui.touch_sizing` setting: follow the input, or force one size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sizing {
    #[default]
    Auto,
    Touch,
    Mouse,
}

impl Sizing {
    /// From the setting's stored choice; unknown values are `Auto`.
    pub fn parse(s: &str) -> Self {
        match s {
            "touch" => Sizing::Touch,
            "mouse" => Sizing::Mouse,
            _ => Sizing::Auto,
        }
    }
}

/// Whether the chrome uses touch sizing (44 px rows).
pub fn is_touch(sizing: Sizing, kind: InputKind) -> bool {
    match sizing {
        Sizing::Touch => true,
        Sizing::Mouse => false,
        Sizing::Auto => matches!(kind, InputKind::Touch { .. }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_flips_to_touch_and_a_real_mouse_move_flips_back() {
        let k = InputKind::Mouse;
        let k = next(k, InputEvent::Touch { x: 10.0, y: 10.0 });
        assert!(is_touch(Sizing::Auto, k));
        // the touch's emulated mouse move, at the touch point: still touch
        let k = next(k, InputEvent::MouseMove { x: 11.0, y: 9.0 });
        assert!(is_touch(Sizing::Auto, k));
        // a real move away
        let k = next(k, InputEvent::MouseMove { x: 40.0, y: 10.0 });
        assert_eq!(k, InputKind::Mouse);
        assert!(!is_touch(Sizing::Auto, k));
        // mouse moves stay mouse
        assert_eq!(
            next(k, InputEvent::MouseMove { x: 10.0, y: 10.0 }),
            InputKind::Mouse
        );
    }

    #[test]
    fn the_setting_forces_a_size() {
        let touch = InputKind::Touch { x: 0.0, y: 0.0 };
        for (sizing, mouse, touched) in [
            (Sizing::Auto, false, true),
            (Sizing::Touch, true, true),
            (Sizing::Mouse, false, false),
        ] {
            assert_eq!(is_touch(sizing, InputKind::Mouse), mouse);
            assert_eq!(is_touch(sizing, touch), touched);
        }
        assert_eq!(Sizing::parse("touch"), Sizing::Touch);
        assert_eq!(Sizing::parse("mouse"), Sizing::Mouse);
        assert_eq!(Sizing::parse("x"), Sizing::Auto);
    }

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
