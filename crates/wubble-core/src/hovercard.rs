//! Hover timing as one state machine, for the tab preview card and for tooltips: when a hovered
//! thing shows its card or tooltip, when that moves to the next thing, when it hides.
//!
//! The shape is Firefox's tab hover preview (`tab-hover-preview.mjs`, MPL-2.0; no code copied):
//! the card shows after a delay; once one has shown and hidden, the next thing shows at once for a
//! while (the "warm" window); and the card hides a little after the pointer leaves (sticky), so
//! crossing from row to row does not flicker. Two timings use it:
//!
//! - [`TAB_CARD`]: Firefox's numbers, `ui.tooltip.delay_ms` (500 ms), `ZERO_DELAY_ACTIVATION_TIME`
//!   (300 ms) and `HOVER_PANEL_STICKY_TIME` (100 ms).
//! - [`TOOLTIP`]: the owner's design (`src/wubble/pages/touch.tsx`, `DOCK`): 280 ms of hover
//!   intent before the first tooltip, 520 ms after one hid during which the next shows at once,
//!   and no sticky time.
//!
//! Times are milliseconds on any monotonic clock the host chooses. The machine never reads a
//! clock; the host arms one timer for [`deadline`] only while the state is not `Idle`.

/// The three durations of one use, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long the pointer rests on a thing before its card shows.
    pub show_ms: u64,
    /// After a card hid, a thing hovered within this time shows its card at once.
    pub warm_ms: u64,
    /// How long a card stays after the pointer left its thing.
    pub sticky_ms: u64,
}

/// The tab preview card (Firefox's numbers).
pub const TAB_CARD: Timing = Timing {
    show_ms: 500,
    warm_ms: 300,
    sticky_ms: 100,
};

/// The link status bubble (Chromium `status_bubble_views.cc`, BSD-3-Clause; numbers only):
/// it shows 80 ms after a link is hovered (`kShowDelay`) and hides 250 ms after the pointer
/// left the link (`kHideDelay`), so moving between links swaps its text without a flicker.
pub const STATUS_BUBBLE: Timing = Timing {
    show_ms: 80,
    warm_ms: 0,
    sticky_ms: 250,
};

/// Tooltips: page `title`s and the shell's icon buttons (the design's dock tooltip).
pub const TOOLTIP: Timing = Timing {
    show_ms: 280,
    warm_ms: 520,
    sticky_ms: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hover<K> {
    #[default]
    Idle,
    /// The pointer is on `key`; its card shows at `due`.
    Pending { key: K, due: u64 },
    /// The card is up, for `key`.
    Shown { key: K },
    /// The pointer left a shown card's thing; the card hides at `at` unless one is entered.
    Leaving { key: K, at: u64 },
    /// The card just hid; until `until` the next thing shows at once.
    Cooling { until: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<K> {
    Enter {
        key: K,
        now: u64,
    },
    /// The pointer left the thing it was on, onto nothing the host knows.
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
pub enum Effect<K> {
    /// Show the card for this key (moving it if it is up for another).
    Show(K),
    Hide,
}

pub fn transition<K: Copy + PartialEq>(
    state: Hover<K>,
    event: Event<K>,
    timing: &Timing,
) -> (Hover<K>, Option<Effect<K>>) {
    use Event::*;
    use Hover::*;
    let pending = |key, now: u64| Pending {
        key,
        due: now + timing.show_ms,
    };
    match (state, event) {
        (Idle, Enter { key, now }) => (pending(key, now), None),
        // the same thing again keeps its time; another starts over
        (Pending { key: on, .. }, Enter { key, .. }) if on == key => (state, None),
        (Pending { .. }, Enter { key, now }) => (pending(key, now), None),
        (Pending { key, due }, Tick { now }) if now >= due => {
            (Shown { key }, Some(Effect::Show(key)))
        }
        (Pending { .. }, Leave { .. } | Dismiss) => (Idle, None),

        (Shown { key: on }, Enter { key, .. }) | (Leaving { key: on, .. }, Enter { key, .. }) => {
            let effect = (on != key).then_some(Effect::Show(key));
            (Shown { key }, effect)
        }
        (Shown { key }, Leave { now }) => (
            Leaving {
                key,
                at: now + timing.sticky_ms,
            },
            None,
        ),
        (Leaving { at, .. }, Tick { now }) if now >= at => (
            Cooling {
                until: at + timing.warm_ms,
            },
            Some(Effect::Hide),
        ),
        (Shown { .. } | Leaving { .. }, Dismiss) => (Idle, Some(Effect::Hide)),

        (Cooling { until }, Enter { key, now }) => {
            if now < until {
                (Shown { key }, Some(Effect::Show(key)))
            } else {
                (pending(key, now), None)
            }
        }
        (Cooling { until }, Tick { now }) if now >= until => (Idle, None),
        (Cooling { .. }, Dismiss) => (Idle, None),

        // nothing happens: a tick before its time, a leave with no card, a dismiss at rest
        (state, _) => (state, None),
    }
}

/// When the host's timer must fire next; `None` at rest (no timer armed).
pub fn deadline<K>(state: Hover<K>) -> Option<u64> {
    match state {
        Hover::Pending { due, .. } => Some(due),
        Hover::Leaving { at, .. } => Some(at),
        Hover::Cooling { until } => Some(until),
        Hover::Idle | Hover::Shown { .. } => None,
    }
}

/// The key a card is up for, or showing on its way out.
pub fn shown<K: Copy>(state: Hover<K>) -> Option<K> {
    match state {
        Hover::Shown { key } | Hover::Leaving { key, .. } => Some(key),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tab::TabId;

    type Hover = super::Hover<TabId>;
    type Event = super::Event<TabId>;
    type Effect = super::Effect<TabId>;

    fn transition(s: Hover, e: Event) -> (Hover, Option<Effect>) {
        super::transition(s, e, &TAB_CARD)
    }

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
        Event::Enter { key: tab, now }
    }

    #[test]
    fn the_card_shows_exactly_at_the_delay() {
        let (s, fx) = run(Hover::Idle, &[enter(A, 1_000), Event::Tick { now: 1_499 }]);
        assert_eq!(
            (s, fx.as_slice()),
            (Hover::Pending { key: A, due: 1_500 }, &[][..])
        );
        assert_eq!(deadline(s), Some(1_500));
        let (s, fx) = run(s, &[Event::Tick { now: 1_500 }]);
        assert_eq!((s, fx), (Hover::Shown { key: A }, vec![Effect::Show(A)]));
        assert_eq!(deadline(s), None);
    }

    #[test]
    fn moving_to_another_row_restarts_the_delay_and_leaving_cancels_it() {
        let (s, fx) = run(Hover::Idle, &[enter(A, 0), enter(B, 300)]);
        assert_eq!((s, fx.len()), (Hover::Pending { key: B, due: 800 }, 0));
        // the same row again does not push the card back
        let (s, _) = run(s, &[enter(B, 400)]);
        assert_eq!(s, Hover::Pending { key: B, due: 800 });
        let (s, fx) = run(s, &[Event::Leave { now: 500 }, Event::Tick { now: 900 }]);
        assert_eq!((s, fx.len()), (Hover::Idle, 0));
    }

    #[test]
    fn crossing_from_row_to_row_moves_the_card_without_a_delay_or_a_hide() {
        let shown = Hover::Shown { key: A };
        let (s, fx) = run(shown, &[Event::Leave { now: 10 }, enter(B, 60)]);
        assert_eq!((s, fx), (Hover::Shown { key: B }, vec![Effect::Show(B)]));
        // back onto the same row inside the sticky time: nothing changes
        let (s, fx) = run(shown, &[Event::Leave { now: 10 }, enter(A, 50)]);
        assert_eq!((s, fx.len()), (Hover::Shown { key: A }, 0));
    }

    #[test]
    fn the_card_hides_after_the_sticky_time_then_the_next_row_shows_at_once() {
        let (s, fx) = run(
            Hover::Shown { key: A },
            &[Event::Leave { now: 1_000 }, Event::Tick { now: 1_099 }],
        );
        assert_eq!((s, fx.len()), (Hover::Leaving { key: A, at: 1_100 }, 0));
        let (s, fx) = run(s, &[Event::Tick { now: 1_100 }]);
        assert_eq!(
            (s, fx),
            (Hover::Cooling { until: 1_400 }, vec![Effect::Hide])
        );
        // inside the window: at once
        let (s2, fx) = run(s, &[enter(B, 1_399)]);
        assert_eq!((s2, fx), (Hover::Shown { key: B }, vec![Effect::Show(B)]));
        // at the edge: the delay again
        let (s3, fx) = run(s, &[enter(B, 1_400)]);
        assert_eq!((s3, fx.len()), (Hover::Pending { key: B, due: 1_900 }, 0));
        // left alone, the window closes and the machine rests
        let (s4, _) = run(s, &[Event::Tick { now: 1_400 }]);
        assert_eq!((s4, deadline(s4)), (Hover::Idle, None));
    }

    #[test]
    fn dismiss_hides_a_card_and_skips_the_instant_window() {
        for s in [Hover::Shown { key: A }, Hover::Leaving { key: A, at: 5 }] {
            assert_eq!(
                transition(s, Event::Dismiss),
                (Hover::Idle, Some(Effect::Hide))
            );
        }
        for s in [
            Hover::Pending { key: A, due: 5 },
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

    fn tip(s: Hover, e: Event) -> (Hover, Option<Effect>) {
        super::transition(s, e, &TOOLTIP)
    }

    fn run_tip(mut s: Hover, events: &[Event]) -> (Hover, Vec<Effect>) {
        let mut fx = vec![];
        for &e in events {
            let (next, effect) = tip(s, e);
            s = next;
            fx.extend(effect);
        }
        (s, fx)
    }

    #[test]
    fn the_status_bubble_shows_after_80_hides_after_250_and_swaps_between() {
        let st = |s, e| super::transition(s, e, &STATUS_BUBBLE);
        let (s, _) = st(Hover::Idle, enter(A, 0));
        assert_eq!(s, Hover::Pending { key: A, due: 80 });
        let (s, fx) = st(s, Event::Tick { now: 80 });
        assert_eq!((s, fx), (Hover::Shown { key: A }, Some(Effect::Show(A))));
        // off the link, onto another within 250 ms: shown throughout
        let (s, _) = st(s, Event::Leave { now: 100 });
        assert_eq!(s, Hover::Leaving { key: A, at: 350 });
        let (s2, fx) = st(s, enter(A, 200));
        assert_eq!((s2, fx), (Hover::Shown { key: A }, None));
        // left alone, it hides, and with no warm window the next waits again
        let (s, fx) = st(s, Event::Tick { now: 350 });
        assert_eq!((s, fx), (Hover::Cooling { until: 350 }, Some(Effect::Hide)));
        let (s, _) = st(s, enter(A, 350));
        assert_eq!(s, Hover::Pending { key: A, due: 430 });
    }

    #[test]
    fn a_tooltip_waits_for_intent_then_shows() {
        let (s, fx) = run_tip(Hover::Idle, &[enter(A, 100), Event::Tick { now: 379 }]);
        assert_eq!((s, fx.len()), (Hover::Pending { key: A, due: 380 }, 0));
        let (s, fx) = run_tip(s, &[Event::Tick { now: 380 }]);
        assert_eq!((s, fx), (Hover::Shown { key: A }, vec![Effect::Show(A)]));
        assert_eq!(shown(s), Some(A));
    }

    #[test]
    fn a_tooltip_hides_at_once_and_the_next_is_warm_for_520() {
        // no sticky time: the hide is due the moment the pointer leaves
        let (s, fx) = run_tip(Hover::Shown { key: A }, &[Event::Leave { now: 1_000 }]);
        assert_eq!((s, fx.len()), (Hover::Leaving { key: A, at: 1_000 }, 0));
        assert_eq!(deadline(s), Some(1_000));
        let (s, fx) = run_tip(s, &[Event::Tick { now: 1_000 }]);
        assert_eq!(
            (s, fx),
            (Hover::Cooling { until: 1_520 }, vec![Effect::Hide])
        );
        let (warm, fx) = run_tip(s, &[enter(B, 1_519)]);
        assert_eq!((warm, fx), (Hover::Shown { key: B }, vec![Effect::Show(B)]));
        let (cold, fx) = run_tip(s, &[enter(B, 1_520)]);
        assert_eq!((cold, fx.len()), (Hover::Pending { key: B, due: 1_800 }, 0));
    }

    #[test]
    fn a_shown_tooltip_glides_to_the_next_key_and_dismiss_drops_it() {
        let (s, fx) = run_tip(Hover::Shown { key: A }, &[enter(B, 5)]);
        assert_eq!((s, fx), (Hover::Shown { key: B }, vec![Effect::Show(B)]));
        assert_eq!(tip(s, Event::Dismiss), (Hover::Idle, Some(Effect::Hide)));
        // leaving before the intent ran out: never shown
        let (s, fx) = run_tip(Hover::Idle, &[enter(A, 0), Event::Leave { now: 100 }]);
        assert_eq!((s, fx.len(), shown(s)), (Hover::Idle, 0, None));
    }
}
