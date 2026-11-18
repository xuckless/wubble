//! Scheduling of CEF's external message pump, as one pure state machine.
//!
//! CEF asks for `do_message_loop_work` through `OnScheduleMessagePumpWork`
//! (any thread, with a delay); the host turns each request into a
//! [`PumpEvent::Schedule`] on its own loop. The rules are a port of cefclient's
//! `MainMessageLoopExternalPump` (chromiumembedded/cef
//! `tests/shared/browser/main_message_loop_external_pump.cc`, BSD-3-Clause):
//! `OnScheduleWork` is [`PumpEvent::Schedule`], `OnTimerTimeout` is
//! [`PumpEvent::TimerFired`], and the tail of `DoWork` is
//! [`PumpEvent::WorkDone`], which the executor reports after every
//! [`PumpEffect::RunWork`].
//!
//! Differences from cefclient: the requests `DoWork` posts to itself are
//! applied in the same transition instead of being posted again, and a
//! re-entrant request only marks the running work (the outer `WorkDone` then
//! re-runs it), which leaves the same state behind.

use std::time::Duration;

/// cefclient's `kTimerDelayPlaceholder`: the "pump at the latest after the
/// maximum delay" request made after each run of work. Ignored while a timer
/// is pending, so it never postpones a sooner request.
pub const TIMER_DELAY_PLACEHOLDER: i64 = i32::MAX as i64;

/// cefclient's `kMaxTimerDelay`: the longest the host waits between two runs
/// of work (30 Hz), whatever delay CEF asks for.
pub const MAX_TIMER_DELAY: Duration = Duration::from_millis(1000 / 30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PumpState {
    /// A single-shot timer is armed and will report [`PumpEvent::TimerFired`].
    pub timer_pending: bool,
    /// [`PumpEffect::RunWork`] was emitted and its [`PumpEvent::WorkDone`]
    /// has not arrived yet.
    pub active: bool,
    /// Work was requested while `active`; it runs again on `WorkDone`.
    pub reentrant: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpEvent {
    /// CEF asked for work after this many milliseconds (`<= 0`: now).
    Schedule(i64),
    /// The timer from the last [`PumpEffect::StartTimer`] fired.
    TimerFired,
    /// The work from [`PumpEffect::RunWork`] has returned.
    WorkDone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PumpEffect {
    /// Arm the single-shot timer, replacing any armed one.
    StartTimer(Duration),
    CancelTimer,
    /// Call `do_message_loop_work` once, then report [`PumpEvent::WorkDone`].
    RunWork,
}

/// The next state and the effects to run, in order.
pub fn transition(state: PumpState, event: PumpEvent) -> (PumpState, Vec<PumpEffect>) {
    let mut effects = Vec::new();
    let next = match event {
        PumpEvent::Schedule(delay_ms) => schedule(state, delay_ms, &mut effects),
        // OnTimerTimeout: KillTimer (the single-shot timer has already
        // stopped), then DoWork.
        PumpEvent::TimerFired => request_work(
            PumpState {
                timer_pending: false,
                ..state
            },
            &mut effects,
        ),
        // A WorkDone without work running is stale; ignore it.
        PumpEvent::WorkDone if !state.active => state,
        // DoWork after PerformMessageLoopWork: re-run re-entrant work now,
        // else make sure CEF is pumped within the maximum delay.
        PumpEvent::WorkDone => {
            let idle = PumpState {
                active: false,
                reentrant: false,
                ..state
            };
            if state.reentrant {
                schedule(idle, 0, &mut effects)
            } else if !state.timer_pending {
                schedule(idle, TIMER_DELAY_PLACEHOLDER, &mut effects)
            } else {
                idle
            }
        }
    };
    (next, effects)
}

/// OnScheduleWork.
fn schedule(state: PumpState, delay_ms: i64, effects: &mut Vec<PumpEffect>) -> PumpState {
    if delay_ms == TIMER_DELAY_PLACEHOLDER && state.timer_pending {
        return state;
    }
    if state.timer_pending {
        effects.push(PumpEffect::CancelTimer);
    }
    let state = PumpState {
        timer_pending: false,
        ..state
    };
    if delay_ms <= 0 {
        return request_work(state, effects);
    }
    let delay = Duration::from_millis(delay_ms.unsigned_abs()).min(MAX_TIMER_DELAY);
    effects.push(PumpEffect::StartTimer(delay));
    PumpState {
        timer_pending: true,
        ..state
    }
}

/// PerformMessageLoopWork's entry: run now, or mark re-entrancy.
fn request_work(state: PumpState, effects: &mut Vec<PumpEffect>) -> PumpState {
    if state.active {
        return PumpState {
            reentrant: true,
            ..state
        };
    }
    effects.push(PumpEffect::RunWork);
    PumpState {
        active: true,
        ..state
    }
}

#[cfg(test)]
mod tests {
    use super::{PumpEffect::*, PumpEvent::*, *};

    const MAX: Duration = MAX_TIMER_DELAY;

    fn st(timer_pending: bool, active: bool, reentrant: bool) -> PumpState {
        PumpState {
            timer_pending,
            active,
            reentrant,
        }
    }

    fn states() -> Vec<PumpState> {
        (0..8)
            .map(|bits| st(bits & 4 != 0, bits & 2 != 0, bits & 1 != 0))
            .collect()
    }

    const EVENTS: [PumpEvent; 8] = [
        Schedule(i64::MIN),
        Schedule(-5),
        Schedule(0),
        Schedule(10),
        Schedule(100),
        Schedule(TIMER_DELAY_PLACEHOLDER),
        TimerFired,
        WorkDone,
    ];

    /// The cefclient rules restated per case, independent of the helpers above.
    fn expected(s: PumpState, event: PumpEvent) -> (PumpState, Vec<PumpEffect>) {
        let PumpState {
            timer_pending: t,
            active: a,
            reentrant: r,
        } = s;
        let cancel = || if t { vec![CancelTimer] } else { vec![] };
        let with = |mut v: Vec<PumpEffect>, e| {
            v.push(e);
            v
        };
        match event {
            Schedule(TIMER_DELAY_PLACEHOLDER) if t => (s, vec![]),
            Schedule(TIMER_DELAY_PLACEHOLDER) | Schedule(100) => {
                (st(true, a, r), with(cancel(), StartTimer(MAX)))
            }
            Schedule(10) => (
                st(true, a, r),
                with(cancel(), StartTimer(Duration::from_millis(10))),
            ),
            Schedule(_) if a => (st(false, true, true), cancel()),
            Schedule(_) => (st(false, true, r), with(cancel(), RunWork)),
            TimerFired if a => (st(false, true, true), vec![]),
            TimerFired => (st(false, true, r), vec![RunWork]),
            WorkDone if !a => (s, vec![]),
            WorkDone if r => (st(false, true, false), with(cancel(), RunWork)),
            WorkDone if t => (st(true, false, false), vec![]),
            WorkDone => (st(true, false, false), vec![StartTimer(MAX)]),
        }
    }

    #[test]
    fn table_covers_every_pair() {
        for state in states() {
            for event in EVENTS {
                assert_eq!(
                    transition(state, event),
                    expected(state, event),
                    "{state:?} + {event:?}"
                );
            }
        }
    }

    #[test]
    fn placeholder_is_ignored_while_a_timer_is_pending() {
        let pending = st(true, false, false);
        assert_eq!(
            transition(pending, Schedule(TIMER_DELAY_PLACEHOLDER)),
            (pending, vec![])
        );
    }

    #[test]
    fn a_positive_delay_restarts_the_timer_clamped_to_the_maximum() {
        assert_eq!(
            transition(st(true, false, false), Schedule(5)),
            (
                st(true, false, false),
                vec![CancelTimer, StartTimer(Duration::from_millis(5))]
            )
        );
        assert_eq!(
            transition(PumpState::default(), Schedule(1_000)),
            (st(true, false, false), vec![StartTimer(MAX)])
        );
        assert_eq!(MAX, Duration::from_millis(33));
    }

    #[test]
    fn zero_or_negative_delay_runs_work_now() {
        for delay in [0, -1, i64::MIN] {
            assert_eq!(
                transition(PumpState::default(), Schedule(delay)),
                (st(false, true, false), vec![RunWork]),
                "{delay}"
            );
        }
    }

    #[test]
    fn work_requested_while_active_only_marks_reentrancy() {
        assert_eq!(
            transition(st(false, true, false), Schedule(0)),
            (st(false, true, true), vec![])
        );
    }

    #[test]
    fn work_done_reruns_reentrant_work() {
        assert_eq!(
            transition(st(false, true, true), WorkDone),
            (st(false, true, false), vec![RunWork])
        );
    }

    #[test]
    fn work_done_arms_the_placeholder_only_without_a_pending_timer() {
        assert_eq!(
            transition(st(false, true, false), WorkDone),
            (st(true, false, false), vec![StartTimer(MAX)])
        );
        assert_eq!(
            transition(st(true, true, false), WorkDone),
            (st(true, false, false), vec![])
        );
    }

    #[test]
    fn timer_fired_clears_pending_and_runs_work() {
        assert_eq!(
            transition(st(true, false, false), TimerFired),
            (st(false, true, false), vec![RunWork])
        );
    }

    #[test]
    fn work_never_runs_while_active_or_twice_per_event() {
        for state in states() {
            for event in EVENTS {
                let (_, effects) = transition(state, event);
                let runs = effects.iter().filter(|e| **e == RunWork).count();
                let allowed = if state.active && event != WorkDone {
                    0
                } else {
                    1
                };
                assert!(runs <= allowed, "{state:?} + {event:?}: {effects:?}");
            }
        }
    }

    /// Runs `events` like the engine does: every RunWork is followed by its
    /// WorkDone before the next host event.
    fn drive(events: &[PumpEvent]) -> (PumpState, Vec<PumpEffect>) {
        let mut state = PumpState::default();
        let mut all = vec![];
        for &event in events {
            let mut queue = vec![event];
            while let Some(event) = queue.pop() {
                let (next, effects) = transition(state, event);
                state = next;
                if effects.contains(&RunWork) {
                    queue.push(WorkDone);
                }
                all.extend(effects);
            }
        }
        (state, all)
    }

    /// The steady state: each run of work leaves the 33 ms timer armed, so a
    /// lost wake-up delays CEF by at most the maximum delay.
    #[test]
    fn every_run_leaves_a_timer_armed() {
        for path in [
            &[Schedule(0)][..],
            &[Schedule(5), TimerFired],
            &[TimerFired],
        ] {
            let (state, effects) = drive(path);
            assert_eq!(state, st(true, false, false), "{path:?}");
            assert_eq!(effects.last(), Some(&StartTimer(MAX)), "{path:?}");
        }
    }
}
