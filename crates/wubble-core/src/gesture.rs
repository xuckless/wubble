//! Touchpad gestures on the page: a pinch magnifies the page under the pointer
//! ([`PinchInput`], driven in `wubble-engine/src/pinch.rs`), a sideways swipe goes back or
//! forward.
//!
//! The history swipe is two fingers sideways, as in Firefox and Chromium on
//! Linux: the touchpad's sideways scroll, which the page gets too. CEF does
//! not say whether a page used a scroll, so the swipe gives way
//! ([`SwipeInput::PageScrolled`]) once the page's root scrolls sideways: it
//! goes back from a page at its left edge, forward from one at its right
//! edge or that cannot scroll sideways. A scroller inside the page (a
//! carousel) does not move the root and so does not stop it. Three fingers
//! (the pointer-gestures swipe) do the same on compositors that pass them to
//! apps; GNOME Shell keeps three and more for itself since GNOME 40
//! (gnome-shell `js/ui/swipeTracker.js`, `GESTURE_FINGER_COUNT`).
//! Written new: the design bucket has no gesture code.

use crate::action::Action;

/// A pinch on the trackpad, as the platform reports it. It magnifies the page under the
/// pointer like a picture (Blink's visual viewport, `wubble-engine/src/pinch.rs`), never the
/// page zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PinchInput {
    Begin,
    /// The scale changed by this factor (1.1 = 10 % larger). macOS gives the
    /// magnification delta `m` (factor `1 + m`); Wayland gives the scale since
    /// the pinch began, which the caller turns into the factor since the last update.
    Scaled(f64),
    End,
}

/// Fingers a history swipe takes: two (a sideways scroll) or three (a
/// pointer-gestures swipe).
pub const SWIPE_FINGERS: std::ops::RangeInclusive<u32> = 2..=3;
/// Sideways travel (logical px) that commits a swipe.
pub const SWIPE_COMMIT: f64 = 120.0;
/// Travel after which the swipe's axis is decided.
const SWIPE_SLOP: f64 = 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Swipe {
    #[default]
    Idle,
    /// Fingers down, the axis not decided yet.
    Starting { dx: f64, dy: f64 },
    /// A sideways swipe, `dx` px so far (positive: fingers moved right).
    Tracking { dx: f64 },
    /// Not ours (vertical, other finger counts, the page scrolled); waits for the end.
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwipeInput {
    Begin {
        fingers: u32,
    },
    Update {
        dx: f64,
        dy: f64,
    },
    /// The page's root scrolled sideways since the begin: the swipe was the page's.
    PageScrolled,
    End {
        cancelled: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwipeEffect {
    /// Show the indicator: `progress` -1..=1 of the way to forward (negative)
    /// or back (positive); 1 or -1 means a release goes there.
    Indicate(f64),
    Hide,
    /// [`Action::Back`] or [`Action::Forward`].
    Go(Action),
}

/// Where a sideways travel of `dx` stands, -1..=1.
pub fn progress(dx: f64) -> f64 {
    (dx / SWIPE_COMMIT).clamp(-1.0, 1.0)
}

pub fn transition(state: Swipe, input: SwipeInput) -> (Swipe, Vec<SwipeEffect>) {
    use SwipeEffect as E;
    match (state, input) {
        (_, SwipeInput::Begin { fingers }) if SWIPE_FINGERS.contains(&fingers) => {
            let hide = matches!(state, Swipe::Tracking { .. });
            (
                Swipe::Starting { dx: 0.0, dy: 0.0 },
                if hide { vec![E::Hide] } else { vec![] },
            )
        }
        (_, SwipeInput::Begin { .. }) => (Swipe::Ignored, vec![]),
        (_, SwipeInput::Update { dx, dy }) if !dx.is_finite() || !dy.is_finite() => (state, vec![]),
        (Swipe::Starting { dx: x, dy: y }, SwipeInput::Update { dx, dy }) => {
            let (x, y) = (x + dx, y + dy);
            if x.abs().max(y.abs()) < SWIPE_SLOP {
                (Swipe::Starting { dx: x, dy: y }, vec![])
            } else if x.abs() > 2.0 * y.abs() {
                (Swipe::Tracking { dx: x }, vec![E::Indicate(progress(x))])
            } else {
                (Swipe::Ignored, vec![])
            }
        }
        (Swipe::Tracking { dx: x }, SwipeInput::Update { dx, .. }) => {
            let x = x + dx;
            (Swipe::Tracking { dx: x }, vec![E::Indicate(progress(x))])
        }
        (Swipe::Starting { .. }, SwipeInput::PageScrolled) => (Swipe::Ignored, vec![]),
        (Swipe::Tracking { .. }, SwipeInput::PageScrolled) => (Swipe::Ignored, vec![E::Hide]),
        (Swipe::Tracking { dx }, SwipeInput::End { cancelled }) => {
            let p = progress(dx);
            let mut effects = vec![E::Hide];
            if !cancelled && p.abs() >= 1.0 {
                effects.push(E::Go(if p > 0.0 {
                    Action::Back
                } else {
                    Action::Forward
                }));
            }
            (Swipe::Idle, effects)
        }
        (_, SwipeInput::End { .. }) => (Swipe::Idle, vec![]),
        (state, SwipeInput::Update { .. } | SwipeInput::PageScrolled) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(inputs: &[SwipeInput]) -> (Swipe, Vec<SwipeEffect>) {
        inputs
            .iter()
            .fold((Swipe::Idle, vec![]), |(s, mut all), &i| {
                let (s, fx) = transition(s, i);
                all.extend(fx);
                (s, all)
            })
    }

    use SwipeInput::{Begin, End, PageScrolled, Update};

    #[test]
    fn a_swipe_right_past_the_mark_goes_back() {
        let (s, fx) = run(&[
            Begin { fingers: 3 },
            Update { dx: 20.0, dy: 2.0 },
            Update { dx: 110.0, dy: 0.0 },
            End { cancelled: false },
        ]);
        assert_eq!(s, Swipe::Idle);
        assert_eq!(
            fx,
            [
                SwipeEffect::Indicate(20.0 / 120.0),
                SwipeEffect::Indicate(1.0),
                SwipeEffect::Hide,
                SwipeEffect::Go(Action::Back)
            ]
        );
        let (_, fx) = run(&[
            Begin { fingers: 3 },
            Update {
                dx: -130.0,
                dy: 0.0,
            },
            End { cancelled: false },
        ]);
        assert_eq!(fx.last(), Some(&SwipeEffect::Go(Action::Forward)));
    }

    #[test]
    fn two_fingers_swipe_as_three_do() {
        let (_, fx) = run(&[
            Begin { fingers: 2 },
            Update { dx: 130.0, dy: 0.0 },
            End { cancelled: false },
        ]);
        assert_eq!(fx.last(), Some(&SwipeEffect::Go(Action::Back)));
    }

    #[test]
    fn a_swipe_the_page_scrolled_with_is_the_page_s() {
        // Decided, then the page's root moved: the chip goes, the release does nothing.
        let (s, fx) = run(&[
            Begin { fingers: 2 },
            Update { dx: 50.0, dy: 0.0 },
            PageScrolled,
            Update { dx: 200.0, dy: 0.0 },
            End { cancelled: false },
        ]);
        assert_eq!(s, Swipe::Idle);
        assert_eq!(fx, [SwipeEffect::Indicate(50.0 / 120.0), SwipeEffect::Hide]);
        // Before the axis is decided.
        let (s, fx) = run(&[
            Begin { fingers: 2 },
            PageScrolled,
            Update { dx: 300.0, dy: 0.0 },
        ]);
        assert_eq!(s, Swipe::Ignored);
        assert!(fx.is_empty());
        // A scroll report while idle is nothing.
        assert_eq!(transition(Swipe::Idle, PageScrolled), (Swipe::Idle, vec![]));
    }

    #[test]
    fn a_short_or_cancelled_swipe_goes_nowhere() {
        let (_, fx) = run(&[
            Begin { fingers: 3 },
            Update { dx: 60.0, dy: 0.0 },
            End { cancelled: false },
        ]);
        assert_eq!(fx.last(), Some(&SwipeEffect::Hide));
        let (_, fx) = run(&[
            Begin { fingers: 3 },
            Update { dx: 300.0, dy: 0.0 },
            End { cancelled: true },
        ]);
        assert_eq!(fx.last(), Some(&SwipeEffect::Hide));
    }

    #[test]
    fn vertical_swipes_and_other_finger_counts_are_not_history() {
        let (s, fx) = run(&[Begin { fingers: 3 }, Update { dx: 10.0, dy: 30.0 }]);
        assert_eq!(s, Swipe::Ignored);
        assert!(fx.is_empty());
        let (_, fx) = run(&[Update { dx: 500.0, dy: 0.0 }, End { cancelled: false }]);
        assert!(fx.is_empty());
        let (s, fx) = run(&[Begin { fingers: 4 }, Update { dx: 500.0, dy: 0.0 }]);
        assert_eq!(s, Swipe::Ignored);
        assert!(fx.is_empty());
        let (s, fx) = run(&[Begin { fingers: 1 }, Update { dx: 500.0, dy: 0.0 }]);
        assert_eq!(s, Swipe::Ignored);
        assert!(fx.is_empty());
        let (s, _) = run(&[
            Begin { fingers: 3 },
            Update {
                dx: 5.0,
                dy: f64::NAN,
            },
        ]);
        assert_eq!(s, Swipe::Starting { dx: 0.0, dy: 0.0 });
    }
}
