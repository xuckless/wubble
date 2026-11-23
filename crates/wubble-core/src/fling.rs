//! Kinetic scrolling for a touchpad's two-finger scroll where the platform has none: the page
//! goes on scrolling after the fingers lift and slows to a stop, as Chrome does on Linux from
//! the scroll's stop (Wayland's `wl_pointer.axis_stop`). CEF's windowless wheel has no phase or
//! momentum field (`cef_mouse_event_t`, `SendMouseWheelEvent`), so the glide is sent as more
//! wheel deltas.
//!
//! Written new, not Chromium's curve (`ui/events/gestures/fling_curve.cc`): a plain exponential
//! decay of the lift velocity, `v(t) = v0·e^(−t/τ)`, whose travel is `v0·τ·(1 − e^(−t/τ))`.
//! The constants below are chosen, not measured against Chrome. Pure: the host feeds the
//! scrolls, the lift and its frame ticks, with time passed in (ms).

use crate::motion::Velocity;

/// The decay's time constant, ms: the speed falls to 37 % after this.
pub const DECAY_MS: f64 = 325.0;
/// Below this lift speed (px/s) the fingers stopped: no glide.
pub const MIN_START: f64 = 100.0;
/// The glide ends once it is slower than this (px/s).
pub const MIN_SPEED: f64 = 20.0;
/// The fastest glide (px/s), so one hard flick does not throw the page to its end.
pub const MAX_SPEED: f64 = 8000.0;
/// Fingers that rested this long (ms) before lifting stopped the scroll: no glide.
pub const REST_MS: f64 = 50.0;

#[derive(Clone, Debug)]
pub enum Fling<T> {
    Idle,
    /// Fingers scrolling `target`: their travel so far (px), its recent path, the last scroll.
    Tracking {
        target: T,
        travel: (f64, f64),
        path: Velocity,
        last: f64,
    },
    /// Gliding on `target` since `start` ms at `v0` px/s; `sent` px of it are out.
    Gliding {
        target: T,
        v0: (f64, f64),
        start: f64,
        sent: (f64, f64),
    },
}

impl<T: Copy> Fling<T> {
    /// What the fingers scroll or the glide moves, if anything.
    pub fn target(&self) -> Option<T> {
        match self {
            Fling::Idle => None,
            Fling::Tracking { target, .. } | Fling::Gliding { target, .. } => Some(*target),
        }
    }
}

// By hand: a derive would ask `T: Default`, which a pane id is not.
impl<T> Default for Fling<T> {
    fn default() -> Self {
        Fling::Idle
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlingInput<T> {
    /// A scroll of `dx`, `dy` px over `target` at `now`.
    Scroll {
        target: T,
        dx: f64,
        dy: f64,
        now: f64,
    },
    /// The fingers lifted (the scroll's stop).
    Lift { now: f64 },
    /// A frame of the glide.
    Tick { now: f64 },
    /// Other input (a press, a touch): the glide stops, as a finger on a touchpad stops it.
    Interrupt,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlingEffect<T> {
    /// Scroll `target` by this much more, in the units of [`FlingInput::Scroll`].
    Scroll { target: T, dx: f64, dy: f64 },
    /// Start ticking the glide.
    Start,
    /// Stop ticking.
    Stop,
}

/// Every row is an arm, one per (state, input); add behaviour as an arm.
pub fn transition<T: Copy + PartialEq>(
    state: Fling<T>,
    input: FlingInput<T>,
) -> (Fling<T>, Vec<FlingEffect<T>>) {
    use FlingEffect as E;
    use FlingInput as I;
    let speed = |(x, y): (f64, f64)| x.hypot(y);
    match (state, input) {
        // A scroll that moves nothing (the stop's own empty frame) changes nothing.
        (state, I::Scroll { dx, dy, .. }) if dx == 0.0 && dy == 0.0 => (state, vec![]),
        (
            Fling::Tracking {
                target,
                travel,
                mut path,
                ..
            },
            I::Scroll {
                target: on,
                dx,
                dy,
                now,
            },
        ) if on == target => {
            let travel = (travel.0 + dx, travel.1 + dy);
            path.push(travel.0, travel.1, now);
            (
                Fling::Tracking {
                    target,
                    travel,
                    path,
                    last: now,
                },
                vec![],
            )
        }
        // The first scroll, one over another target, or new fingers during a glide.
        (
            state,
            I::Scroll {
                target,
                dx,
                dy,
                now,
            },
        ) => {
            // One point: the speed is measured from the first scroll on, so a short flick's
            // first frame does not count as travel in no time.
            let mut path = Velocity::new();
            path.push(dx, dy, now);
            let stop = matches!(state, Fling::Gliding { .. });
            (
                Fling::Tracking {
                    target,
                    travel: (dx, dy),
                    path,
                    last: now,
                },
                if stop { vec![E::Stop] } else { vec![] },
            )
        }
        (
            Fling::Tracking {
                target, path, last, ..
            },
            I::Lift { now },
        ) => {
            let v = path.get();
            if now - last > REST_MS || speed(v) < MIN_START {
                return (Fling::Idle, vec![]);
            }
            let k = (MAX_SPEED / speed(v)).min(1.0);
            (
                Fling::Gliding {
                    target,
                    v0: (v.0 * k, v.1 * k),
                    start: now,
                    sent: (0.0, 0.0),
                },
                vec![E::Start],
            )
        }
        (state @ (Fling::Idle | Fling::Gliding { .. }), I::Lift { .. }) => (state, vec![]),
        (
            Fling::Gliding {
                target,
                v0,
                start,
                sent,
            },
            I::Tick { now },
        ) => {
            let t = (now - start).max(0.0);
            let decay = (-t / DECAY_MS).exp();
            let travel = |v: f64| v * DECAY_MS / 1000.0 * (1.0 - decay);
            let at = (travel(v0.0), travel(v0.1));
            let mut effects = vec![E::Scroll {
                target,
                dx: at.0 - sent.0,
                dy: at.1 - sent.1,
            }];
            if speed(v0) * decay < MIN_SPEED {
                effects.push(E::Stop);
                return (Fling::Idle, effects);
            }
            (
                Fling::Gliding {
                    target,
                    v0,
                    start,
                    sent: at,
                },
                effects,
            )
        }
        // A tick with no glide is a timer left running.
        (state @ (Fling::Idle | Fling::Tracking { .. }), I::Tick { .. }) => (state, vec![E::Stop]),
        (Fling::Gliding { .. }, I::Interrupt) => (Fling::Idle, vec![E::Stop]),
        (Fling::Idle | Fling::Tracking { .. }, I::Interrupt) => (Fling::Idle, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(inputs: &[FlingInput<u8>]) -> (Fling<u8>, Vec<FlingEffect<u8>>) {
        inputs
            .iter()
            .fold((Fling::Idle, vec![]), |(s, mut all), &i| {
                let (s, fx) = transition(s, i);
                all.extend(fx);
                (s, all)
            })
    }

    /// Fingers on target 1 moving `dy` px every 10 ms for `n` frames from 0 ms.
    fn scroll(n: u32, dy: f64) -> Vec<FlingInput<u8>> {
        (0..n)
            .map(|i| FlingInput::Scroll {
                target: 1,
                dx: 0.0,
                dy,
                now: f64::from(i) * 10.0,
            })
            .collect()
    }

    fn gliding(s: &Fling<u8>) -> bool {
        matches!(s, Fling::Gliding { .. })
    }

    #[test]
    fn a_lift_after_a_flick_glides_and_slows_to_a_stop() {
        // 20 px per 10 ms: 2000 px/s
        let mut inputs = scroll(6, 20.0);
        inputs.push(FlingInput::Lift { now: 55.0 });
        let (s, fx) = run(&inputs);
        assert!(gliding(&s));
        assert_eq!(fx, [FlingEffect::Start]);
        // the glide goes the scroll's way, each step shorter, until it stops itself
        let (mut s, mut last, mut total) = (s, f64::INFINITY, 0.0);
        for i in 1..1000 {
            let (next, fx) = transition(
                s,
                FlingInput::Tick {
                    now: 55.0 + f64::from(i) * 8.0,
                },
            );
            s = next;
            let FlingEffect::Scroll { target: 1, dx, dy } = fx[0] else {
                panic!("{fx:?}");
            };
            assert_eq!(dx, 0.0);
            assert!(dy > 0.0 && dy <= last, "{i}: {dy}");
            (last, total) = (dy, total + dy);
            if fx.len() == 2 {
                assert_eq!(fx[1], FlingEffect::Stop);
                break;
            }
        }
        assert!(matches!(s, Fling::Idle));
        // the whole glide travels about v0·τ
        assert!((total - 2000.0 * DECAY_MS / 1000.0).abs() < 2000.0 * DECAY_MS / 1000.0 * 0.05);
    }

    #[test]
    fn no_glide_after_still_fingers_or_a_slow_scroll() {
        // rested before lifting
        let mut inputs = scroll(6, 20.0);
        inputs.push(FlingInput::Lift {
            now: 50.0 + REST_MS + 1.0,
        });
        let (s, fx) = run(&inputs);
        assert!(matches!(s, Fling::Idle) && fx.is_empty());
        // 0.5 px per 10 ms: 50 px/s
        let mut inputs = scroll(6, 0.5);
        inputs.push(FlingInput::Lift { now: 55.0 });
        assert!(matches!(run(&inputs).0, Fling::Idle));
    }

    #[test]
    fn a_hard_flick_glides_at_the_cap() {
        let mut inputs = scroll(6, 1000.0);
        inputs.push(FlingInput::Lift { now: 55.0 });
        let Fling::Gliding { v0, .. } = run(&inputs).0 else {
            panic!();
        };
        assert!((v0.0.hypot(v0.1) - MAX_SPEED).abs() < 1e-6);
    }

    #[test]
    fn every_state_meets_every_input() {
        let flick = || {
            let mut i = scroll(6, 20.0);
            i.push(FlingInput::Lift { now: 55.0 });
            i
        };
        let scroll_on = |target, dy| FlingInput::Scroll {
            target,
            dx: 0.0,
            dy,
            now: 60.0,
        };
        // (inputs to reach the state, input, the state after is gliding, its effects)
        let rows = vec![
            // idle
            (vec![], scroll_on(1, 5.0), "tracking", vec![]),
            (vec![], scroll_on(1, 0.0), "idle", vec![]),
            (vec![], FlingInput::Lift { now: 0.0 }, "idle", vec![]),
            (
                vec![],
                FlingInput::Tick { now: 0.0 },
                "idle",
                vec![FlingEffect::Stop],
            ),
            (vec![], FlingInput::Interrupt, "idle", vec![]),
            // tracking
            (scroll(3, 20.0), scroll_on(1, 5.0), "tracking", vec![]),
            (scroll(3, 20.0), scroll_on(2, 5.0), "tracking", vec![]),
            (scroll(3, 20.0), scroll_on(1, 0.0), "tracking", vec![]),
            (
                scroll(3, 20.0),
                FlingInput::Tick { now: 30.0 },
                "tracking",
                vec![FlingEffect::Stop],
            ),
            (scroll(3, 20.0), FlingInput::Interrupt, "idle", vec![]),
            (
                scroll(3, 20.0),
                FlingInput::Lift { now: 25.0 },
                "gliding",
                vec![FlingEffect::Start],
            ),
            // gliding: new fingers stop it and track; the stop's empty frame does not
            (
                flick(),
                scroll_on(1, 5.0),
                "tracking",
                vec![FlingEffect::Stop],
            ),
            (flick(), scroll_on(1, 0.0), "gliding", vec![]),
            (flick(), FlingInput::Lift { now: 60.0 }, "gliding", vec![]),
            (
                flick(),
                FlingInput::Interrupt,
                "idle",
                vec![FlingEffect::Stop],
            ),
        ];
        for (i, (before, input, after, effects)) in rows.into_iter().enumerate() {
            let (s, _) = run(&before);
            let (s, fx) = transition(s, input);
            let name = match s {
                Fling::Idle => "idle",
                Fling::Tracking { .. } => "tracking",
                Fling::Gliding { .. } => "gliding",
            };
            assert_eq!((name, fx), (after, effects), "row {i}");
        }
    }

    #[test]
    fn a_scroll_over_another_target_starts_over() {
        let mut inputs = scroll(6, 20.0);
        inputs.push(FlingInput::Scroll {
            target: 2,
            dx: 0.0,
            dy: 20.0,
            now: 60.0,
        });
        inputs.push(FlingInput::Lift { now: 65.0 });
        // one scroll's path has no speed yet: no glide, and never on the old target
        let (s, _) = run(&inputs);
        assert!(matches!(s, Fling::Idle));
    }
}
