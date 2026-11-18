//! The CEF process lifecycle as one pure state machine.
//!
//! CEF browsers close asynchronously and `cef::shutdown` with a live browser
//! crashes, so shutdown waits in [`State::ShuttingDown`] until the last
//! [`Event::BrowserClosed`]. Whoever creates and destroys browsers must report
//! [`Event::BrowserOpened`] / [`Event::BrowserClosed`] for every one of them.

/// Where the CEF runtime is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Uninitialised,
    Running { browsers: u32 },
    ShuttingDown { browsers: u32 },
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Initialised,
    InitFailed,
    BrowserOpened,
    BrowserClosed,
    ShutdownRequested,
    /// The host loop gives CEF a chance to do its work.
    Tick,
}

/// Work the engine performs on behalf of a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    PumpOnce,
    CloseAllBrowsers,
    ShutdownCef,
}

/// Every (state, event) pair not listed here is a no-op.
pub fn transition(state: State, event: Event) -> (State, Vec<Effect>) {
    use {Effect::*, Event::*, State::*};
    match (state, event) {
        (Uninitialised, Initialised) => (Running { browsers: 0 }, vec![]),
        (Uninitialised, InitFailed) => (Stopped, vec![]),
        (Running { browsers }, BrowserOpened) => (
            Running {
                browsers: browsers.saturating_add(1),
            },
            vec![],
        ),
        (Running { browsers }, BrowserClosed) if browsers > 0 => (
            Running {
                browsers: browsers - 1,
            },
            vec![],
        ),
        (Running { browsers: 0 }, ShutdownRequested) => (Stopped, vec![ShutdownCef]),
        (Running { browsers }, ShutdownRequested) => {
            (ShuttingDown { browsers }, vec![CloseAllBrowsers])
        }
        (Running { .. } | ShuttingDown { .. }, Tick) => (state, vec![PumpOnce]),
        (ShuttingDown { browsers: 1 }, BrowserClosed) => (Stopped, vec![ShutdownCef]),
        (ShuttingDown { browsers }, BrowserClosed) if browsers > 1 => (
            ShuttingDown {
                browsers: browsers - 1,
            },
            vec![],
        ),
        _ => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect::*, Event::*, State::*, *};

    const EVENTS: [Event; 6] = [
        Initialised,
        InitFailed,
        BrowserOpened,
        BrowserClosed,
        ShutdownRequested,
        Tick,
    ];

    fn states() -> Vec<State> {
        let mut s = vec![Uninitialised, Stopped];
        for n in 0..3 {
            s.push(Running { browsers: n });
            s.push(ShuttingDown { browsers: n });
        }
        s
    }

    fn expected(state: State, event: Event) -> (State, Vec<Effect>) {
        match (state, event) {
            (Uninitialised, Initialised) => (Running { browsers: 0 }, vec![]),
            (Uninitialised, InitFailed) => (Stopped, vec![]),
            (Running { browsers: n }, BrowserOpened) => (Running { browsers: n + 1 }, vec![]),
            (Running { browsers: 0 }, BrowserClosed) => (Running { browsers: 0 }, vec![]),
            (Running { browsers: n }, BrowserClosed) => (Running { browsers: n - 1 }, vec![]),
            (Running { browsers: 0 }, ShutdownRequested) => (Stopped, vec![ShutdownCef]),
            (Running { browsers: n }, ShutdownRequested) => {
                (ShuttingDown { browsers: n }, vec![CloseAllBrowsers])
            }
            (Running { .. }, Tick) | (ShuttingDown { .. }, Tick) => (state, vec![PumpOnce]),
            (ShuttingDown { browsers: 0 }, BrowserClosed) => (state, vec![]),
            (ShuttingDown { browsers: 1 }, BrowserClosed) => (Stopped, vec![ShutdownCef]),
            (ShuttingDown { browsers: n }, BrowserClosed) => {
                (ShuttingDown { browsers: n - 1 }, vec![])
            }
            _ => (state, vec![]),
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
    fn stopped_is_absorbing() {
        for event in EVENTS {
            assert_eq!(transition(Stopped, event), (Stopped, vec![]));
        }
    }

    #[test]
    fn tick_outside_running_does_nothing() {
        assert_eq!(transition(Uninitialised, Tick), (Uninitialised, vec![]));
        assert_eq!(transition(Stopped, Tick), (Stopped, vec![]));
    }

    #[test]
    fn browser_opened_ignored_when_not_running() {
        for state in [Uninitialised, ShuttingDown { browsers: 2 }, Stopped] {
            assert_eq!(transition(state, BrowserOpened), (state, vec![]));
        }
    }

    /// Runs `events` from `Uninitialised` and returns the final state and every effect.
    fn run(events: &[Event]) -> (State, Vec<Effect>) {
        events
            .iter()
            .fold((Uninitialised, vec![]), |(state, mut all), &event| {
                let (next, effects) = transition(state, event);
                all.extend(effects);
                (next, all)
            })
    }

    fn shutdown_count(effects: &[Effect]) -> usize {
        effects.iter().filter(|e| **e == ShutdownCef).count()
    }

    #[test]
    fn shutdown_cef_exactly_once_on_every_path() {
        let paths: [&[Event]; 3] = [
            &[
                Initialised,
                Tick,
                ShutdownRequested,
                ShutdownRequested,
                Tick,
            ],
            &[
                Initialised,
                BrowserOpened,
                BrowserOpened,
                ShutdownRequested,
                Tick,
                BrowserClosed,
                BrowserClosed,
                BrowserClosed,
                ShutdownRequested,
            ],
            &[
                Initialised,
                BrowserOpened,
                BrowserClosed,
                ShutdownRequested,
                BrowserClosed,
            ],
        ];
        for path in paths {
            let (state, effects) = run(path);
            assert_eq!(state, Stopped, "{path:?}");
            assert_eq!(shutdown_count(&effects), 1, "{path:?}");
        }
        let (state, effects) = run(&[InitFailed, ShutdownRequested, Tick]);
        assert_eq!(state, Stopped);
        assert_eq!(shutdown_count(&effects), 0, "nothing to shut down");
    }

    #[test]
    fn shutting_down_waits_for_last_browser() {
        let mut state = Running { browsers: 3 };
        state = transition(state, ShutdownRequested).0;
        for left in [2, 1] {
            let (next, effects) = transition(state, BrowserClosed);
            assert_eq!(next, ShuttingDown { browsers: left });
            assert!(effects.is_empty());
            state = next;
        }
        assert_eq!(
            transition(state, BrowserClosed),
            (Stopped, vec![ShutdownCef])
        );
    }
}
