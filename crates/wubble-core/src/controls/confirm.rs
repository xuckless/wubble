//! Confirm in place: the owner's design `pages/settings.tsx:354-373` (`ConfirmButton`,
//! the-wallpaper-project @598f814). A click while idle runs the action; with a busy label and a
//! busy time the button shows it (a spinner) for that time; then the done label (a check, mint)
//! with one pulse, back to idle after [`CONFIRM::doneMs`]. Clicks are ignored unless idle.
//!
//! The TSX's two `setTimeout`s are countdowns run by `Tick(dt)`: one enum, one pure
//! [`transition`], no clock.

use crate::kit_gen::settings::CONFIRM;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Busy { left_ms: f64 },
    Done { left_ms: f64 },
}

impl Phase {
    /// 0 idle, 1 busy, 2 done: what the Slint side shows.
    pub fn index(self) -> i32 {
        match self {
            Phase::Idle => 0,
            Phase::Busy { .. } => 1,
            Phase::Done { .. } => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Click,
    Tick(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Run the action (`onRun`), at the click.
    Run,
    /// Pulse once, as the done label arrives.
    Pulse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub phase: Phase,
    /// How long the busy label shows, ms (`ms`; 0: straight to done, as without a busy label).
    pub busy_ms: f64,
    /// Counts [`Effect::Pulse`], so a view that only sees the latest state replays each pulse.
    pub pulses: i32,
}

impl State {
    pub const fn new(busy_ms: f64) -> Self {
        Self {
            phase: Phase::Idle,
            busy_ms,
            pulses: 0,
        }
    }

    /// Whether a countdown is running (the frame driver keeps ticking).
    pub fn moving(&self) -> bool {
        self.phase != Phase::Idle
    }
}

fn done(s: &mut State, fx: &mut Vec<Effect>) -> Phase {
    s.pulses = s.pulses.wrapping_add(1);
    fx.push(Effect::Pulse);
    Phase::Done {
        left_ms: CONFIRM::doneMs,
    }
}

/// One step of the confirm button. Pure.
pub fn transition(mut s: State, ev: Event) -> (State, Vec<Effect>) {
    let mut fx = Vec::new();
    match (s.phase, ev) {
        (Phase::Idle, Event::Click) => {
            fx.push(Effect::Run);
            s.phase = if s.busy_ms > 0.0 {
                Phase::Busy { left_ms: s.busy_ms }
            } else {
                done(&mut s, &mut fx)
            };
        }
        (_, Event::Click) | (Phase::Idle, Event::Tick(_)) => {}
        (Phase::Busy { left_ms }, Event::Tick(dt)) => {
            let left = left_ms - dt * 1000.0;
            s.phase = if left <= 0.0 {
                done(&mut s, &mut fx)
            } else {
                Phase::Busy { left_ms: left }
            };
        }
        (Phase::Done { left_ms }, Event::Tick(dt)) => {
            let left = left_ms - dt * 1000.0;
            s.phase = if left <= 0.0 {
                Phase::Idle
            } else {
                Phase::Done { left_ms: left }
            };
        }
    }
    (s, fx)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    /// Ticks until the phase changes; returns the new state, the effects on the way, the frames.
    fn until_change(mut s: State) -> (State, Vec<Effect>, usize) {
        let from = s.phase.index();
        let mut all = Vec::new();
        for n in 1..100_000 {
            let (next, fx) = transition(s, Event::Tick(DT));
            s = next;
            all.extend(fx);
            if s.phase.index() != from {
                return (s, all, n);
            }
        }
        panic!("stuck in {from}");
    }

    #[test]
    fn idle_busy_done_idle() {
        let s = State::new(1000.0);
        let (s, fx) = transition(s, Event::Click);
        assert_eq!(fx, [Effect::Run]);
        assert_eq!(s.phase.index(), 1);
        let (s, fx, n) = until_change(s);
        assert_eq!(fx, [Effect::Pulse]);
        assert_eq!(s.phase.index(), 2);
        assert!((119..=121).contains(&n), "{n}");
        let (s, fx, n) = until_change(s);
        assert!(fx.is_empty());
        assert_eq!(s.phase, Phase::Idle);
        let want = (CONFIRM::doneMs / 1000.0 / DT).round() as usize;
        assert!(n.abs_diff(want) <= 1, "{n} vs {want}");
        assert!(!s.moving());
    }

    #[test]
    fn without_a_busy_time_it_goes_straight_to_done() {
        let (s, fx) = transition(State::new(0.0), Event::Click);
        assert_eq!(fx, [Effect::Run, Effect::Pulse]);
        assert_eq!(s.phase.index(), 2);
        assert_eq!(s.pulses, 1, "the state counts the pulse");
    }

    #[test]
    fn clicks_are_ignored_unless_idle() {
        let (s, _) = transition(State::new(500.0), Event::Click);
        let (s, fx) = transition(s, Event::Click);
        assert!(fx.is_empty());
        assert_eq!(s.phase.index(), 1);
        let (s, _, _) = until_change(s);
        let (s, fx) = transition(s, Event::Click);
        assert!(fx.is_empty());
        assert_eq!(s.phase.index(), 2);
    }
}
