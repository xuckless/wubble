//! Touchpad gestures on the page: pinch zooms the page in browser zoom steps,
//! a three-finger sideways swipe goes back or forward.
//!
//! Pinch is browser zoom, as Firefox and Safari's ctrl+wheel fallback do, not
//! CEF touch events: pages would see synthetic touches, and a windowless
//! CEF view has no pinch-to-page-scale. Two fingers sideways stay page
//! scroll (CEF does not say when a page consumed a horizontal scroll), so the
//! history swipe takes three, as GNOME's own gestures leave them to apps.
//! Written new: the design bucket has no gesture code.

/// ln-scale one zoom step needs (a fifth larger or smaller).
pub const PINCH_STEP: f64 = 0.182_321_556_793_954_6; // ln(1.2)

/// A pinch zoom step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
}

/// A pinch: the ln-scale not yet turned into zoom steps.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pinch {
    acc: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PinchInput {
    /// The scale changed by this factor (1.1 = 10 % larger). macOS gives the
    /// magnification delta `m` (factor `1 + m`); Wayland gives the scale since
    /// the pinch began, which the caller turns into the factor since the last update.
    Scaled(f64),
    End,
}

impl Pinch {
    /// The zoom steps `input` completes (at most a few per update).
    pub fn input(self, input: PinchInput) -> (Pinch, Vec<Zoom>) {
        match input {
            PinchInput::End => (Pinch::default(), vec![]),
            // winit says the delta may be NaN; a zero or negative factor is not a scale.
            PinchInput::Scaled(f) if !f.is_finite() || f <= 0.0 => (self, vec![]),
            PinchInput::Scaled(f) => {
                let mut acc = self.acc + f.ln();
                let mut steps = vec![];
                while acc.abs() >= PINCH_STEP && steps.len() < 4 {
                    let step = if acc > 0.0 { Zoom::In } else { Zoom::Out };
                    acc -= PINCH_STEP.copysign(acc);
                    steps.push(step);
                }
                if acc.abs() >= PINCH_STEP {
                    acc = 0.0;
                }
                (Pinch { acc }, steps)
            }
        }
    }
}

/// Fingers a history swipe needs.
pub const SWIPE_FINGERS: u32 = 3;
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
    /// Not ours (vertical, too few fingers); waits for the end.
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwipeInput {
    Begin { fingers: u32 },
    Update { dx: f64, dy: f64 },
    End { cancelled: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwipeEffect {
    /// Show the indicator: `progress` -1..=1 of the way to forward (negative)
    /// or back (positive); 1 or -1 means a release goes there.
    Indicate(f64),
    Hide,
    Go(Nav),
}

/// Where a sideways travel of `dx` stands, -1..=1.
pub fn progress(dx: f64) -> f64 {
    (dx / SWIPE_COMMIT).clamp(-1.0, 1.0)
}

pub fn transition(state: Swipe, input: SwipeInput) -> (Swipe, Vec<SwipeEffect>) {
    use SwipeEffect as E;
    match (state, input) {
        (_, SwipeInput::Begin { fingers }) if fingers == SWIPE_FINGERS => {
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
        (Swipe::Tracking { dx }, SwipeInput::End { cancelled }) => {
            let p = progress(dx);
            let mut effects = vec![E::Hide];
            if !cancelled && p.abs() >= 1.0 {
                effects.push(E::Go(if p > 0.0 { Nav::Back } else { Nav::Forward }));
            }
            (Swipe::Idle, effects)
        }
        (_, SwipeInput::End { .. }) => (Swipe::Idle, vec![]),
        (state, SwipeInput::Update { .. }) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pinch_zooms_one_step_per_fifth() {
        let p = Pinch::default();
        let (p, z) = p.input(PinchInput::Scaled(1.1));
        assert!(z.is_empty());
        let (p, z) = p.input(PinchInput::Scaled(1.1));
        assert_eq!(z, [Zoom::In]);
        let (p, z) = p.input(PinchInput::Scaled(1.0 / 1.25));
        assert_eq!(z, [Zoom::Out]);
        let (_, z) = p.input(PinchInput::Scaled(1.25 * 1.25));
        assert_eq!(z, [Zoom::In, Zoom::In]);
    }

    #[test]
    fn a_pinch_forgets_its_remainder_at_the_end() {
        let (p, _) = Pinch::default().input(PinchInput::Scaled(1.15));
        let (p, _) = p.input(PinchInput::End);
        let (_, z) = p.input(PinchInput::Scaled(1.1));
        assert!(z.is_empty());
    }

    #[test]
    fn a_pinch_ignores_what_is_not_a_scale() {
        for f in [f64::NAN, f64::INFINITY, 0.0, -2.0, 1.0] {
            let (p, z) = Pinch::default().input(PinchInput::Scaled(f));
            assert!(z.is_empty(), "{f}");
            assert_eq!(p, Pinch::default(), "{f}");
        }
        // A huge jump zooms a few steps, not dozens.
        let (p, z) = Pinch::default().input(PinchInput::Scaled(1e9));
        assert_eq!(z.len(), 4);
        assert_eq!(p, Pinch::default());
    }

    fn run(inputs: &[SwipeInput]) -> (Swipe, Vec<SwipeEffect>) {
        inputs
            .iter()
            .fold((Swipe::Idle, vec![]), |(s, mut all), &i| {
                let (s, fx) = transition(s, i);
                all.extend(fx);
                (s, all)
            })
    }

    use SwipeInput::{Begin, End, Update};

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
                SwipeEffect::Go(Nav::Back)
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
        assert_eq!(fx.last(), Some(&SwipeEffect::Go(Nav::Forward)));
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
