//! The CEF process lifecycle as one pure state machine.
//!
//! CEF browsers close asynchronously and `cef::shutdown` with a live browser
//! crashes, so shutdown waits in [`State::ShuttingDown`] until the last
//! [`Event::BrowserClosed`]. Whoever creates and destroys browsers must report
//! [`Event::BrowserOpened`] / [`Event::BrowserClosed`] for every one of them:
//! the engine's views and every other browser CEF creates (`app::extra`).
//!
//! Policy: only [`Event::ShutdownRequested`] (the host quitting) ever leads to
//! [`Effect::ShutdownCef`]. Closing the last browser while running does not:
//! a count reaching zero is never a reason to stop.

/// Pump turns ([`Event::Tick`]) a shutdown waits for its browsers' closed
/// reports before it gives up on them ([`Effect::AbandonCef`]). A count, not a
/// clock: it only ends a quit that would otherwise hang. A DevTools window
/// closes in a few hundred turns of the host's drain loop; an idle turn took
/// about 4 us on the owner's Linux machine, so this is seconds, not a hang.
pub const SHUTDOWN_TURNS: u32 = 1 << 22;

/// Pump turns between the last browser's closed report and `cef::shutdown`.
/// `OnBeforeClose` of a Chrome-style browser (a DevTools window) comes before
/// Chromium deletes its `Browser` and native window, which takes more pumping
/// (on Wayland, a round trip to the compositor); `cef::shutdown` before that
/// crashed in `TabDragServiceImpl` under `BrowserManagerService::Shutdown`.
/// A count, not a clock: an idle turn of the drain loop took about 4 us on the
/// owner's Linux machine, so this is tens of milliseconds there.
pub const SETTLE_TURNS: u32 = 1 << 14;

/// Where the CEF runtime is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Uninitialised,
    Running {
        browsers: u32,
    },
    /// `turns`: pump turns since the shutdown began.
    ShuttingDown {
        browsers: u32,
        turns: u32,
    },
    /// Every browser reported closed; CEF gets [`SETTLE_TURNS`] more pump
    /// turns to finish deleting them before `cef::shutdown`.
    Settling {
        turns: u32,
    },
    Stopped,
    /// Stopped without `cef::shutdown`: `browsers` never reported closed.
    Abandoned {
        browsers: u32,
    },
}

impl State {
    /// CEF is initialised, so its message loop may be pumped.
    pub fn pumps(self) -> bool {
        matches!(
            self,
            Self::Running { .. } | Self::ShuttingDown { .. } | Self::Settling { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Initialised,
    BrowserOpened,
    BrowserClosed,
    ShutdownRequested,
    /// The host asks for one synchronous run of CEF's work (no event loop:
    /// `--smoke boot` and the shutdown drain).
    Tick,
}

/// Work the engine performs on behalf of a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    PumpOnce,
    /// Ask every browser not yet asked to close (views and extra browsers).
    CloseAllBrowsers,
    ShutdownCef,
    /// Browsers never reported closed within [`SHUTDOWN_TURNS`]: stop
    /// without `cef::shutdown`, which would crash on them; log it.
    AbandonCef,
}

/// Every (state, event) pair not listed here is a no-op.
pub fn transition(state: State, event: Event) -> (State, Vec<Effect>) {
    use {Effect::*, Event::*, State::*};
    match (state, event) {
        (Uninitialised, Initialised) => (Running { browsers: 0 }, vec![]),
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
        (Running { browsers: 0 }, ShutdownRequested) => (Settling { turns: 0 }, vec![]),
        (Running { browsers }, ShutdownRequested) => {
            (ShuttingDown { browsers, turns: 0 }, vec![CloseAllBrowsers])
        }
        (Running { .. }, Tick) => (state, vec![PumpOnce]),
        // Created after the quit began (a DevTools window opened just before):
        // it is waited for, and closed like the rest.
        (ShuttingDown { browsers, turns }, BrowserOpened) => (
            ShuttingDown {
                browsers: browsers.saturating_add(1),
                turns,
            },
            vec![CloseAllBrowsers],
        ),
        (ShuttingDown { browsers, turns }, Tick) if turns.saturating_add(1) >= SHUTDOWN_TURNS => {
            (Abandoned { browsers }, vec![AbandonCef])
        }
        (ShuttingDown { browsers, turns }, Tick) => (
            ShuttingDown {
                browsers,
                turns: turns + 1,
            },
            vec![PumpOnce],
        ),
        (ShuttingDown { browsers: 1, .. }, BrowserClosed) => (Settling { turns: 0 }, vec![]),
        (Settling { turns }, Tick) if turns.saturating_add(1) >= SETTLE_TURNS => {
            (Stopped, vec![ShutdownCef])
        }
        (Settling { turns }, Tick) => (Settling { turns: turns + 1 }, vec![PumpOnce]),
        // Created while settling (asked for just before the quit): closed and
        // waited for like the rest.
        (Settling { .. }, BrowserOpened) => (
            ShuttingDown {
                browsers: 1,
                turns: 0,
            },
            vec![CloseAllBrowsers],
        ),
        (ShuttingDown { browsers, turns }, BrowserClosed) if browsers > 1 => (
            ShuttingDown {
                browsers: browsers - 1,
                turns,
            },
            vec![],
        ),
        _ => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect::*, Event::*, State::*, *};

    const EVENTS: [Event; 5] = [
        Initialised,
        BrowserOpened,
        BrowserClosed,
        ShutdownRequested,
        Tick,
    ];

    const SETTLING: State = Settling { turns: 0 };

    fn down(browsers: u32) -> State {
        ShuttingDown { browsers, turns: 0 }
    }

    fn states() -> Vec<State> {
        let mut s = vec![
            Uninitialised,
            Stopped,
            Abandoned { browsers: 1 },
            SETTLING,
            Settling {
                turns: SETTLE_TURNS - 1,
            },
        ];
        for n in 0..3 {
            s.push(Running { browsers: n });
            s.push(down(n));
            s.push(ShuttingDown {
                browsers: n,
                turns: SHUTDOWN_TURNS - 1,
            });
        }
        s
    }

    fn expected(state: State, event: Event) -> (State, Vec<Effect>) {
        let last = SHUTDOWN_TURNS - 1;
        match (state, event) {
            (Uninitialised, Initialised) => (Running { browsers: 0 }, vec![]),
            (Running { browsers: n }, BrowserOpened) => (Running { browsers: n + 1 }, vec![]),
            (Running { browsers: 0 }, BrowserClosed) => (Running { browsers: 0 }, vec![]),
            (Running { browsers: n }, BrowserClosed) => (Running { browsers: n - 1 }, vec![]),
            (Running { browsers: 0 }, ShutdownRequested) => (SETTLING, vec![]),
            (Running { browsers: n }, ShutdownRequested) => (down(n), vec![CloseAllBrowsers]),
            (Running { .. }, Tick) => (state, vec![PumpOnce]),
            (ShuttingDown { browsers: n, turns }, BrowserOpened) => (
                ShuttingDown {
                    browsers: n + 1,
                    turns,
                },
                vec![CloseAllBrowsers],
            ),
            (ShuttingDown { browsers, turns }, Tick) if turns == last => {
                (Abandoned { browsers }, vec![AbandonCef])
            }
            (ShuttingDown { browsers, turns }, Tick) => (
                ShuttingDown {
                    browsers,
                    turns: turns + 1,
                },
                vec![PumpOnce],
            ),
            (ShuttingDown { browsers: 0, .. }, BrowserClosed) => (state, vec![]),
            (ShuttingDown { browsers: 1, .. }, BrowserClosed) => (SETTLING, vec![]),
            (ShuttingDown { browsers: n, turns }, BrowserClosed) => (
                ShuttingDown {
                    browsers: n - 1,
                    turns,
                },
                vec![],
            ),
            (Settling { turns }, Tick) if turns == SETTLE_TURNS - 1 => (Stopped, vec![ShutdownCef]),
            (Settling { turns }, Tick) => (Settling { turns: turns + 1 }, vec![PumpOnce]),
            (Settling { .. }, BrowserOpened) => (down(1), vec![CloseAllBrowsers]),
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
    fn stopped_and_abandoned_are_absorbing() {
        for state in [Stopped, Abandoned { browsers: 1 }] {
            for event in EVENTS {
                assert_eq!(transition(state, event), (state, vec![]));
            }
        }
    }

    #[test]
    fn tick_outside_running_does_nothing() {
        assert_eq!(transition(Uninitialised, Tick), (Uninitialised, vec![]));
        assert_eq!(transition(Stopped, Tick), (Stopped, vec![]));
    }

    #[test]
    fn browser_opened_ignored_before_start_and_after_stop() {
        for state in [Uninitialised, Stopped] {
            assert_eq!(transition(state, BrowserOpened), (state, vec![]));
        }
    }

    /// Runs `events` from `state` and returns the final state and every effect.
    fn run_from(state: State, events: &[Event]) -> (State, Vec<Effect>) {
        events
            .iter()
            .fold((state, vec![]), |(state, mut all), &event| {
                let (next, effects) = transition(state, event);
                all.extend(effects);
                (next, all)
            })
    }

    fn run(events: &[Event]) -> (State, Vec<Effect>) {
        run_from(Uninitialised, events)
    }

    /// The drain's ticks until the state stops pumping; the effects of the last.
    fn tick_out(mut state: State) -> (State, u32, Vec<Effect>) {
        let mut turns = 0;
        loop {
            let (next, effects) = transition(state, Tick);
            turns += 1;
            state = next;
            if !state.pumps() {
                return (state, turns, effects);
            }
            assert_eq!(effects, vec![PumpOnce], "{state:?}");
        }
    }

    fn shutdown_count(effects: &[Effect]) -> usize {
        effects.iter().filter(|e| **e == ShutdownCef).count()
    }

    #[test]
    fn shutdown_cef_exactly_once_on_every_path() {
        let paths: [&[Event]; 4] = [
            &[Initialised, Tick, ShutdownRequested, ShutdownRequested],
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
            &[
                Initialised,
                BrowserOpened,
                ShutdownRequested,
                BrowserOpened,
                BrowserClosed,
                Tick,
                BrowserClosed,
            ],
        ];
        for path in paths {
            let (state, mut effects) = run(path);
            assert_eq!(state, SETTLING, "{path:?}");
            let (state, _, last) = tick_out(state);
            effects.extend(last);
            assert_eq!(state, Stopped, "{path:?}");
            assert_eq!(shutdown_count(&effects), 1, "{path:?}");
        }
    }

    /// Every browser closed is not yet the end: CEF still deletes Chrome-style
    /// browsers after their `OnBeforeClose`, so `cef::shutdown` waits a fixed
    /// number of turns.
    #[test]
    fn the_last_close_settles_before_cef_shuts_down() {
        let (state, effects) = run(&[Initialised, BrowserOpened, ShutdownRequested, Tick, Tick]);
        assert_eq!(
            state,
            ShuttingDown {
                browsers: 1,
                turns: 2
            }
        );
        assert_eq!(effects, vec![CloseAllBrowsers, PumpOnce, PumpOnce]);
        assert_eq!(transition(state, BrowserClosed), (SETTLING, vec![]));
        assert_eq!(
            tick_out(SETTLING),
            (Stopped, SETTLE_TURNS, vec![ShutdownCef])
        );
        // with nothing open, the quit settles too
        assert_eq!(run(&[Initialised, ShutdownRequested]), (SETTLING, vec![]));
    }

    #[test]
    fn shutting_down_waits_for_last_browser() {
        let mut state = Running { browsers: 3 };
        state = transition(state, ShutdownRequested).0;
        for left in [2, 1] {
            let (next, effects) = transition(state, BrowserClosed);
            assert_eq!(next, down(left));
            assert!(effects.is_empty());
            state = next;
        }
        assert_eq!(transition(state, BrowserClosed), (SETTLING, vec![]));
    }

    /// The crash this guards: a view and its DevTools window open at quit. The
    /// view closing first must not shut CEF down under the DevTools window.
    #[test]
    fn quit_with_devtools_open_waits_for_both() {
        let (state, effects) = run(&[
            Initialised,
            BrowserOpened, // the view
            BrowserOpened, // its DevTools window
            ShutdownRequested,
            Tick,
            BrowserClosed, // the view
            Tick,
        ]);
        assert_eq!(
            state,
            ShuttingDown {
                browsers: 1,
                turns: 2
            }
        );
        assert_eq!(shutdown_count(&effects), 0);
        assert_eq!(transition(state, BrowserClosed), (SETTLING, vec![]));
    }

    /// DevTools asked for just before the quit reports itself created after it,
    /// while closing or while settling.
    #[test]
    fn a_browser_created_during_shutdown_is_closed_and_waited_for() {
        let (state, effects) = run(&[
            Initialised,
            BrowserOpened,
            ShutdownRequested,
            BrowserOpened,
            BrowserClosed,
        ]);
        assert_eq!(state, down(1));
        assert_eq!(effects, vec![CloseAllBrowsers, CloseAllBrowsers]);
        assert_eq!(transition(state, BrowserClosed), (SETTLING, vec![]));
        let (state, effects) = run_from(SETTLING, &[Tick, BrowserOpened, Tick]);
        assert_eq!(
            state,
            ShuttingDown {
                browsers: 1,
                turns: 1
            }
        );
        assert_eq!(effects, vec![PumpOnce, CloseAllBrowsers, PumpOnce]);
    }

    /// A close reported after CEF stopped changes nothing.
    #[test]
    fn a_late_close_after_the_quit_is_ignored() {
        let (state, _, _) =
            tick_out(run(&[Initialised, BrowserOpened, ShutdownRequested, BrowserClosed]).0);
        assert_eq!(state, Stopped);
        assert_eq!(run_from(state, &[BrowserClosed, Tick]), (Stopped, vec![]));
        // and one while settling is not counted
        assert_eq!(transition(SETTLING, BrowserClosed), (SETTLING, vec![]));
    }

    /// Policy: closing the last browser while running never stops CEF; only a
    /// quit does.
    #[test]
    fn closing_the_last_browser_without_a_quit_keeps_running() {
        let (state, effects) = run(&[
            Initialised,
            BrowserOpened,
            BrowserOpened,
            BrowserClosed,
            BrowserClosed,
            BrowserClosed,
            Tick,
        ]);
        assert_eq!(state, Running { browsers: 0 });
        assert_eq!(effects, vec![PumpOnce]);
    }

    /// A browser that never reports closed ends the quit after a bounded
    /// number of turns, without `cef::shutdown`.
    #[test]
    fn a_browser_that_never_closes_is_abandoned_after_the_bound() {
        let mut state = transition(Running { browsers: 2 }, ShutdownRequested).0;
        state = transition(state, BrowserClosed).0;
        let (state, turns, effects) = tick_out(state);
        assert_eq!(turns, SHUTDOWN_TURNS);
        assert_eq!(state, Abandoned { browsers: 1 });
        assert_eq!(effects, vec![AbandonCef]);
    }
}
