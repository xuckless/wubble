//! The tab preview card's timing as one state machine: when a hovered tab row shows its card,
//! when the card moves to the next row, when it hides.
//!
//! The numbers and the shape are Firefox's tab hover preview (`tab-hover-preview.mjs`,
//! MPL-2.0; no code copied): the card shows after `ui.tooltip.delay_ms` (500 ms); once one has
//! shown and hidden, the next row shows its card at once for 300 ms (`ZERO_DELAY_ACTIVATION_TIME`);
//! and the card hides 100 ms after the pointer leaves (`HOVER_PANEL_STICKY_TIME`), so crossing
//! from row to row does not flicker.
//!
//! Times are milliseconds on any monotonic clock the host chooses. The machine never reads a
//! clock; the host arms one timer for [`deadline`] only while the state is not `Idle`.

use crate::tab::TabId;

/// How long the pointer rests on a row before its card shows.
pub const SHOW_DELAY_MS: u64 = 500;
/// After a card hid, a row hovered within this time shows its card at once.
pub const ZERO_DELAY_WINDOW_MS: u64 = 300;
/// How long a card stays after the pointer left its row.
pub const STICKY_MS: u64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hover {
    #[default]
    Idle,
    /// The pointer is on `tab`; its card shows at `due`.
    Pending { tab: TabId, due: u64 },
    /// The card is up, for `tab`.
    Shown { tab: TabId },
    /// The pointer left a shown card's row; the card hides at `at` unless a row is entered.
    Leaving { tab: TabId, at: u64 },
    /// The card just hid; until `until` the next row shows at once.
    Cooling { until: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Enter {
        tab: TabId,
        now: u64,
    },
    /// The pointer left the row it was on, onto nothing the list knows.
    Leave {
        now: u64,
    },
    /// The armed timer fired.
    Tick {
        now: u64,
    },
    /// A press, drag, scroll, tab switch, menu or window blur: no card, no instant re-show.
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Show the card for this tab (moving it if it is up for another).
    Show(TabId),
    Hide,
}

pub fn transition(state: Hover, event: Event) -> (Hover, Option<Effect>) {
    use Event::*;
    use Hover::*;
    match (state, event) {
        (Idle, Enter { tab, now }) => (
            Pending {
                tab,
                due: now + SHOW_DELAY_MS,
            },
            None,
        ),
        // the same row again keeps its time; another row starts over
        (Pending { tab: on, .. }, Enter { tab, .. }) if on == tab => (state, None),
        (Pending { .. }, Enter { tab, now }) => (
            Pending {
                tab,
                due: now + SHOW_DELAY_MS,
            },
            None,
        ),
        (Pending { tab, due }, Tick { now }) if now >= due => {
            (Shown { tab }, Some(Effect::Show(tab)))
        }
        (Pending { .. }, Leave { .. } | Dismiss) => (Idle, None),

        (Shown { tab: on }, Enter { tab, .. }) | (Leaving { tab: on, .. }, Enter { tab, .. }) => {
            let effect = (on != tab).then_some(Effect::Show(tab));
            (Shown { tab }, effect)
        }
        (Shown { tab }, Leave { now }) => (
            Leaving {
                tab,
                at: now + STICKY_MS,
            },
            None,
        ),
        (Leaving { at, .. }, Tick { now }) if now >= at => (
            Cooling {
                until: at + ZERO_DELAY_WINDOW_MS,
            },
            Some(Effect::Hide),
        ),
        (Shown { .. } | Leaving { .. }, Dismiss) => (Idle, Some(Effect::Hide)),

        (Cooling { until }, Enter { tab, now }) => {
            if now < until {
                (Shown { tab }, Some(Effect::Show(tab)))
            } else {
                (
                    Pending {
                        tab,
                        due: now + SHOW_DELAY_MS,
                    },
                    None,
                )
            }
        }
        (Cooling { until }, Tick { now }) if now >= until => (Idle, None),
        (Cooling { .. }, Dismiss) => (Idle, None),

        // nothing happens: a tick before its time, a leave with no card, a dismiss at rest
        (state, _) => (state, None),
    }
}

/// When the host's timer must fire next; `None` at rest (no timer armed).
pub fn deadline(state: Hover) -> Option<u64> {
    match state {
        Hover::Pending { due, .. } => Some(due),
        Hover::Leaving { at, .. } => Some(at),
        Hover::Cooling { until } => Some(until),
        Hover::Idle | Hover::Shown { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    fn run(mut s: Hover, events: &[Event]) -> (Hover, Vec<Effect>) {
        let mut fx = vec![];
        for &e in events {
            let (next, effect) = transition(s, e);
            s = next;
            fx.extend(effect);
        }
        (s, fx)
    }

    fn enter(tab: TabId, now: u64) -> Event {
        Event::Enter { tab, now }
    }

    #[test]
    fn the_card_shows_exactly_at_the_delay() {
        let (s, fx) = run(Hover::Idle, &[enter(A, 1_000), Event::Tick { now: 1_499 }]);
        assert_eq!(
            (s, fx.as_slice()),
            (Hover::Pending { tab: A, due: 1_500 }, &[][..])
        );
        assert_eq!(deadline(s), Some(1_500));
        let (s, fx) = run(s, &[Event::Tick { now: 1_500 }]);
        assert_eq!((s, fx), (Hover::Shown { tab: A }, vec![Effect::Show(A)]));
        assert_eq!(deadline(s), None);
    }

    #[test]
    fn moving_to_another_row_restarts_the_delay_and_leaving_cancels_it() {
        let (s, fx) = run(Hover::Idle, &[enter(A, 0), enter(B, 300)]);
        assert_eq!((s, fx.len()), (Hover::Pending { tab: B, due: 800 }, 0));
        // the same row again does not push the card back
        let (s, _) = run(s, &[enter(B, 400)]);
        assert_eq!(s, Hover::Pending { tab: B, due: 800 });
        let (s, fx) = run(s, &[Event::Leave { now: 500 }, Event::Tick { now: 900 }]);
        assert_eq!((s, fx.len()), (Hover::Idle, 0));
    }

    #[test]
    fn crossing_from_row_to_row_moves_the_card_without_a_delay_or_a_hide() {
        let shown = Hover::Shown { tab: A };
        let (s, fx) = run(shown, &[Event::Leave { now: 10 }, enter(B, 60)]);
        assert_eq!((s, fx), (Hover::Shown { tab: B }, vec![Effect::Show(B)]));
        // back onto the same row inside the sticky time: nothing changes
        let (s, fx) = run(shown, &[Event::Leave { now: 10 }, enter(A, 50)]);
        assert_eq!((s, fx.len()), (Hover::Shown { tab: A }, 0));
    }

    #[test]
    fn the_card_hides_after_the_sticky_time_then_the_next_row_shows_at_once() {
        let (s, fx) = run(
            Hover::Shown { tab: A },
            &[Event::Leave { now: 1_000 }, Event::Tick { now: 1_099 }],
        );
        assert_eq!((s, fx.len()), (Hover::Leaving { tab: A, at: 1_100 }, 0));
        let (s, fx) = run(s, &[Event::Tick { now: 1_100 }]);
        assert_eq!(
            (s, fx),
            (Hover::Cooling { until: 1_400 }, vec![Effect::Hide])
        );
        // inside the window: at once
        let (s2, fx) = run(s, &[enter(B, 1_399)]);
        assert_eq!((s2, fx), (Hover::Shown { tab: B }, vec![Effect::Show(B)]));
        // at the edge: the delay again
        let (s3, fx) = run(s, &[enter(B, 1_400)]);
        assert_eq!((s3, fx.len()), (Hover::Pending { tab: B, due: 1_900 }, 0));
        // left alone, the window closes and the machine rests
        let (s4, _) = run(s, &[Event::Tick { now: 1_400 }]);
        assert_eq!((s4, deadline(s4)), (Hover::Idle, None));
    }

    #[test]
    fn dismiss_hides_a_card_and_skips_the_instant_window() {
        for s in [Hover::Shown { tab: A }, Hover::Leaving { tab: A, at: 5 }] {
            assert_eq!(
                transition(s, Event::Dismiss),
                (Hover::Idle, Some(Effect::Hide))
            );
        }
        for s in [
            Hover::Pending { tab: A, due: 5 },
            Hover::Cooling { until: 5 },
        ] {
            assert_eq!(transition(s, Event::Dismiss), (Hover::Idle, None));
        }
    }

    #[test]
    fn nothing_happens_at_rest() {
        for e in [
            Event::Leave { now: 5 },
            Event::Tick { now: 5 },
            Event::Dismiss,
        ] {
            assert_eq!(transition(Hover::Idle, e), (Hover::Idle, None));
        }
        assert_eq!(deadline(Hover::Idle), None);
    }
}
