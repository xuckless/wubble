//! One `wubble_core::hovercard` machine with its clock and timer, for each hover surface the
//! host draws (the tab preview card, the link status bubble, tooltips). The timer is armed only
//! while the machine waits for a deadline, and calls back into the host on its tick.

use std::time::{Duration, Instant};

use wubble_core::hovercard::{self, Effect, Event, Hover, Timing};

use super::Host;
use crate::with_host;

pub(super) struct HoverClock<K> {
    state: Hover<K>,
    timing: Timing,
    timer: slint::Timer,
    /// The monotonic zero of the machine's milliseconds.
    epoch: Instant,
}

impl<K: Copy + PartialEq> HoverClock<K> {
    pub fn new(timing: Timing) -> Self {
        Self {
            state: Hover::Idle,
            timing,
            timer: slint::Timer::default(),
            epoch: Instant::now(),
        }
    }

    pub fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    pub fn state(&self) -> Hover<K> {
        self.state
    }

    /// Runs `event`; re-arms the timer for the next deadline, whose tick calls `tick` on the
    /// host. Returns the state before the step with the effect.
    pub fn step(&mut self, event: Event<K>, tick: fn(&mut Host)) -> (Hover<K>, Option<Effect<K>>) {
        let before = self.state;
        let (state, effect) = hovercard::transition(before, event, &self.timing);
        self.state = state;
        match hovercard::deadline(state) {
            Some(at) => {
                let wait = at.saturating_sub(self.now());
                self.timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(wait),
                    move || with_host(tick),
                );
            }
            None => self.timer.stop(),
        }
        (before, effect)
    }
}
