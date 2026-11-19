//! Hold to confirm: the owner's design `pages/settings.tsx:300-352` (`HOLD`, `HoldButton`;
//! @source settings-hold, the-wallpaper-project @598f814). The coral fills from the left in real
//! time while held (`HOLD.ms`, linear: the hold is honest) and the button stretches as it fills;
//! let go early and the fill drains on a spring while the stretch springs back; full, it strikes
//! (an extra kick of the stretch), reports done and pulses, then resets after `HOLD.doneMs`.
//!
//! One enum of states and one pure [`transition`]; the TSX's `setTimeout` is a countdown run by
//! `Tick(dt)`, so nothing reads a clock. Constants: [`crate::kit_gen::settings::HOLD`].

use crate::kit_gen::settings::HOLD;
use crate::motion::{Spring, clamp, spring, squash_of, step_k};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Idle,
    /// Pointer or Space/Enter down and not yet full.
    Held,
    /// Full: the done label shows for `left_ms` more.
    Done {
        left_ms: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Pointer down (left button) or Space/Enter down (not a repeat).
    Start,
    /// Pointer up, cancelled or lost; the key up.
    End,
    Tick(f64),
    /// The button is disabled (or enabled again).
    Disable(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The hold completed: run the action (`onDone`).
    Done,
    /// Pulse once (`tp-pulse`).
    Pulse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub phase: Phase,
    /// The fill, 0..=1 (a spring while draining).
    p: Spring,
    /// The stretch along the button (its cross axis keeps the area).
    k: Spring,
    disabled: bool,
    moving: bool,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// What the button draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// The fill's share of the width, 0..=1 (`inset(0 (1-p)*100% 0 0)`).
    pub p: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub done: bool,
    pub held: bool,
}

impl State {
    pub const fn new() -> Self {
        Self {
            phase: Phase::Idle,
            p: spring(0.0),
            k: spring(1.0),
            disabled: false,
            moving: false,
        }
    }

    pub fn moving(&self) -> bool {
        self.moving
    }

    pub fn view(&self) -> View {
        View {
            p: clamp(self.p.x, 0.0, 1.0),
            scale_x: self.k.x,
            scale_y: squash_of(self.k.x),
            done: matches!(self.phase, Phase::Done { .. }),
            held: self.phase == Phase::Held,
        }
    }
}

/// One step of the hold button. Pure.
pub fn transition(mut s: State, ev: Event) -> (State, Vec<Effect>) {
    let mut fx = Vec::new();
    match ev {
        // `start`: refused while disabled or done
        Event::Start => {
            if !s.disabled && s.phase == Phase::Idle {
                s.phase = Phase::Held;
            }
        }
        Event::End => {
            if s.phase == Phase::Held {
                s.phase = Phase::Idle;
            }
        }
        Event::Disable(d) => {
            s.disabled = d;
            // wubble: a disabled button lets go (the web's disabled button stops getting the
            // pointer up; notes/fidelity/derived-review.md)
            if d && s.phase == Phase::Held {
                s.phase = Phase::Idle;
            }
        }
        Event::Tick(dt) => {
            let mut m = false;
            if s.phase == Phase::Held {
                s.p.x = (s.p.x + dt * 1000.0 / HOLD::ms).min(1.0);
                s.p.v = 0.0;
                m = true;
                step_k(&mut s.k, 1.0 + HOLD::grow * s.p.x, HOLD::hold, dt);
                if s.p.x >= 1.0 {
                    s.phase = Phase::Done {
                        left_ms: HOLD::doneMs,
                    };
                    s.k.v += HOLD::strike;
                    fx.push(Effect::Done);
                    fx.push(Effect::Pulse);
                }
            } else {
                let done = matches!(s.phase, Phase::Done { .. });
                m |= step_k(&mut s.p, if done { 1.0 } else { 0.0 }, HOLD::drain, dt);
                m |= step_k(&mut s.k, 1.0, HOLD::release, dt);
                // the TSX's `setTimeout(doneMs)`, counted down by the frames
                if let Phase::Done { left_ms } = s.phase {
                    let left = left_ms - dt * 1000.0;
                    s.phase = if left <= 0.0 {
                        Phase::Idle
                    } else {
                        Phase::Done { left_ms: left }
                    };
                    m = true;
                }
            }
            s.moving = m;
        }
    }
    (s, fx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed frame: 1/120 s, as exactly 1/120 of a second in ms below.
    const DT: f64 = 1.0 / 120.0;

    fn tick(s: State) -> (State, Vec<Effect>) {
        transition(s, Event::Tick(DT))
    }

    #[test]
    fn a_full_hold_takes_exactly_its_time_and_is_done_once() {
        let (mut s, _) = transition(State::new(), Event::Start);
        let frames = (HOLD::ms / 1000.0 / DT).round() as usize;
        let mut done = 0;
        let mut at = 0;
        for i in 1..=frames + 10 {
            let (n, fx) = tick(s);
            s = n;
            if fx.contains(&Effect::Done) {
                assert!(fx.contains(&Effect::Pulse));
                done += 1;
                at = i;
            }
        }
        assert_eq!(done, 1);
        // 1100 ms at 120 Hz is 132 frames; float rounding may take one more
        assert!(at == frames || at == frames + 1, "{at} vs {frames}");
        assert!(s.view().done);
        assert_eq!(s.view().p, 1.0);
    }

    #[test]
    fn the_strike_kicks_the_stretch() {
        let (mut s, _) = transition(State::new(), Event::Start);
        let before = loop {
            let v = s.k.v;
            let (n, fx) = tick(s);
            s = n;
            if fx.contains(&Effect::Done) {
                break v;
            }
        };
        assert!(s.k.v > before + 1.0, "{} {}", s.k.v, before);
        let mut peak: f64 = 0.0;
        for _ in 0..60 {
            s = tick(s).0;
            peak = peak.max(s.view().scale_x);
        }
        assert!(peak > 1.0 + HOLD::grow, "{peak}");
    }

    #[test]
    fn an_early_release_drains_and_never_completes() {
        let (mut s, _) = transition(State::new(), Event::Start);
        for _ in 0..60 {
            s = tick(s).0;
        }
        let p = s.view().p;
        assert!(p > 0.4 && p < 0.5, "{p}");
        assert!(s.view().scale_x > 1.0, "stretched while held");
        s = transition(s, Event::End).0;
        let mut n = 0;
        loop {
            let (next, fx) = tick(s);
            s = next;
            assert!(fx.is_empty());
            n += 1;
            if !s.moving() {
                break;
            }
            assert!(n < 5000);
        }
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!((s.view().p, s.view().scale_x), (0.0, 1.0));
    }

    #[test]
    fn disabled_refuses_a_start_and_lets_go() {
        let (s, _) = transition(State::new(), Event::Disable(true));
        let (s, _) = transition(s, Event::Start);
        assert_eq!(s.phase, Phase::Idle);
        let (s, _) = transition(s, Event::Disable(false));
        let (s, _) = transition(s, Event::Start);
        assert_eq!(s.phase, Phase::Held);
        let (s, _) = transition(s, Event::Disable(true));
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn done_resets_after_its_time_and_ignores_starts_meanwhile() {
        let (mut s, _) = transition(State::new(), Event::Start);
        while !s.view().done {
            s = tick(s).0;
        }
        s = transition(s, Event::End).0;
        s = transition(s, Event::Start).0;
        assert!(s.view().done, "a start while done is refused");
        let frames = (HOLD::doneMs / 1000.0 / DT).round() as usize;
        for _ in 0..frames - 2 {
            s = tick(s).0;
        }
        assert!(s.view().done);
        for _ in 0..4 {
            s = tick(s).0;
        }
        assert!(!s.view().done);
        while s.moving() {
            s = tick(s).0;
        }
        assert_eq!(s.view().p, 0.0);
    }

    #[test]
    fn reduced_motion_keeps_the_honest_fill() {
        crate::motion::set_reduced(true);
        let (mut s, _) = transition(State::new(), Event::Start);
        for _ in 0..12 {
            s = tick(s).0;
        }
        crate::motion::set_reduced(false);
        // p still fills in real time; the stretch lands at once
        assert!((s.view().p - 0.0909).abs() < 1e-3, "{}", s.view().p);
        assert!((s.view().scale_x - (1.0 + HOLD::grow * s.view().p)).abs() < 1e-9);
    }
}
