//! Tab lifecycle as one explicit state machine.
//!
//! [`transition`] is a pure function from (policy, state, event) to (state, effects).
//! The engine executes the effects; nothing here touches a browser or a clock.
//!
//! Model adapted, no code copied, from:
//! - Chromium `chrome/browser/resource_coordinator/lifecycle_unit_state.mojom`
//!   (`LifecycleUnitState`, `LifecycleUnitDiscardReason`; BSD-3-Clause). The
//!   deprecated `PENDING_*` states are deliberately not modelled.
//! - Chromium `components/performance_manager/freezing/freezing_policy.cc`
//!   (`CannotFreezeReason`; BSD-3-Clause): our [`Activity::Busy`].
//! - Firefox `browser/components/tabbrowser/TabUnloader.sys.mjs` (MPL-2.0):
//!   playing audio, PiP and WebRTC tabs are never unloaded. Unlike Firefox, pinned
//!   tabs are never discarded here (TODO.md "Decided").

use std::time::Instant;

use crate::sleep::{Pressure, SleepPolicy};
use crate::tab::Liveness;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pin {
    Unpinned,
    Pinned,
}

/// Playing audio, capturing media, holding a connection the user cares about, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Busy,
}

/// The single owner of a tab's liveness, pin and busy state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifecycle {
    pub liveness: Liveness,
    pub pin: Pin,
    pub activity: Activity,
    /// When the tab was hidden. Set while Background or Frozen, `None` otherwise.
    pub hidden_since: Option<Instant>,
}

impl Lifecycle {
    /// A visible, idle, unpinned tab.
    pub fn new() -> Self {
        Self {
            liveness: Liveness::Active,
            pin: Pin::Unpinned,
            activity: Activity::Idle,
            hidden_since: None,
        }
    }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Shown,
    Hidden {
        now: Instant,
    },
    /// Periodic evaluation of the sleep policy for a hidden tab.
    Tick {
        now: Instant,
        pressure: Pressure,
    },
    BusyChanged(Activity),
    PinnedChanged(Pin),
    /// The renderer process crashed or was killed. A visible tab is reloaded at
    /// once; a hidden one becomes Discarded and is restored when shown.
    RendererGone,
    /// The tab is being closed. Terminal: the owner drops the tab and sends no
    /// further events (`Shown` afterwards would restore it).
    Closed,
}

/// Work for the engine to perform. Effects are listed in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Show,
    Hide,
    Freeze,
    Unfreeze,
    /// Grab a texture of the page so the UI can show it while discarded.
    CaptureSnapshot,
    /// Destroy the renderer. May follow `Freeze` or come straight from Background.
    /// The snapshot taken at `Hidden` is kept for the UI.
    Discard,
    /// Reload from the URL and restore scroll.
    Restore,
    /// Drop the snapshot taken at `Hidden`. Sent when a hidden tab is closed. It is
    /// not sent on restore: the engine drops it on the first new frame.
    ReleaseSnapshot,
    ReleaseTexture,
}

pub fn transition(policy: &SleepPolicy, s: Lifecycle, e: Event) -> (Lifecycle, Vec<Effect>) {
    use Effect::*;
    use Liveness::*;

    let discarded = Lifecycle {
        liveness: Discarded,
        hidden_since: None,
        ..s
    };

    match (s.liveness, e) {
        (Active, Event::Shown) => (s, vec![]),
        (Background, Event::Shown) => (active(s), vec![Show]),
        (Frozen, Event::Shown) => (active(s), vec![Unfreeze, Show]),
        (Discarded, Event::Shown) => (active(s), vec![Restore, Show]),

        (Active, Event::Hidden { now }) => (
            Lifecycle {
                liveness: Background,
                hidden_since: Some(now),
                ..s
            },
            vec![CaptureSnapshot, Hide],
        ),
        (_, Event::Hidden { .. }) => (s, vec![]),

        (Background | Frozen, Event::Tick { now, pressure }) => {
            let Some(since) = s.hidden_since else {
                debug_assert!(false, "hidden tab without hidden_since");
                return (s, vec![]);
            };
            let hidden = now.saturating_duration_since(since);
            match (
                s.liveness,
                policy.target(hidden, s.pin, s.activity, pressure),
            ) {
                (Background, Frozen) => (
                    Lifecycle {
                        liveness: Frozen,
                        ..s
                    },
                    vec![Freeze],
                ),
                (_, Discarded) => (discarded, vec![Discard, ReleaseTexture]),
                _ => (s, vec![]),
            }
        }
        (Active | Discarded, Event::Tick { .. }) => (s, vec![]),

        (Frozen, Event::BusyChanged(Activity::Busy)) => (
            Lifecycle {
                liveness: Background,
                activity: Activity::Busy,
                ..s
            },
            vec![Unfreeze],
        ),
        (_, Event::BusyChanged(activity)) => (Lifecycle { activity, ..s }, vec![]),
        (_, Event::PinnedChanged(pin)) => (Lifecycle { pin, ..s }, vec![]),

        (Active, Event::RendererGone) => (s, vec![Restore]),
        (Discarded, Event::RendererGone) => (s, vec![]),
        (_, Event::RendererGone) => (discarded, vec![ReleaseTexture]),

        // An active tab may still show its snapshot (closed while restoring).
        (Active | Background | Frozen, Event::Closed) => {
            (discarded, vec![ReleaseTexture, ReleaseSnapshot])
        }
        (Discarded, Event::Closed) => (s, vec![ReleaseSnapshot]),
    }
}

fn active(s: Lifecycle) -> Lifecycle {
    Lifecycle {
        liveness: Liveness::Active,
        hidden_since: None,
        ..s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect::*;
    use Liveness::*;
    use std::time::Duration;

    fn base() -> Instant {
        Instant::now() + Duration::from_secs(10_000)
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    /// A tab in `liveness`, hidden since `base()` unless Active/Discarded.
    fn tab(liveness: Liveness, pin: Pin, activity: Activity) -> Lifecycle {
        let hidden = matches!(liveness, Background | Frozen);
        Lifecycle {
            liveness,
            pin,
            activity,
            hidden_since: hidden.then(base),
        }
    }

    fn idle(liveness: Liveness) -> Lifecycle {
        tab(liveness, Pin::Unpinned, Activity::Idle)
    }

    fn tick(after: u64, pressure: Pressure) -> Event {
        Event::Tick {
            now: base() + secs(after),
            pressure,
        }
    }

    fn run(s: Lifecycle, e: Event) -> (Lifecycle, Vec<Effect>) {
        transition(&SleepPolicy::default(), s, e)
    }

    const DISCARD: [Effect; 2] = [Discard, ReleaseTexture];

    #[test]
    fn every_state_and_event_pair() {
        let busy = Event::BusyChanged(Activity::Busy);
        let idle_ev = Event::BusyChanged(Activity::Idle);
        let pin = Event::PinnedChanged(Pin::Pinned);
        let hidden = Event::Hidden { now: base() };
        let none = Pressure::None;
        let crit = Pressure::Critical;

        // (name, state, event, expected liveness, expected effects)
        let table: Vec<(&str, Lifecycle, Event, Liveness, Vec<Effect>)> = vec![
            ("active shown", idle(Active), Event::Shown, Active, vec![]),
            (
                "bg shown",
                idle(Background),
                Event::Shown,
                Active,
                vec![Show],
            ),
            (
                "frozen shown",
                idle(Frozen),
                Event::Shown,
                Active,
                vec![Unfreeze, Show],
            ),
            (
                "disc shown",
                idle(Discarded),
                Event::Shown,
                Active,
                vec![Restore, Show],
            ),
            (
                "active hidden",
                idle(Active),
                hidden,
                Background,
                vec![CaptureSnapshot, Hide],
            ),
            ("bg hidden", idle(Background), hidden, Background, vec![]),
            ("frozen hidden", idle(Frozen), hidden, Frozen, vec![]),
            ("disc hidden", idle(Discarded), hidden, Discarded, vec![]),
            (
                "active tick",
                idle(Active),
                tick(3600, crit),
                Active,
                vec![],
            ),
            (
                "bg tick early",
                idle(Background),
                tick(5, none),
                Background,
                vec![],
            ),
            (
                "bg tick 60s",
                idle(Background),
                tick(60, none),
                Frozen,
                vec![Freeze],
            ),
            (
                "bg tick 15m",
                idle(Background),
                tick(900, none),
                Discarded,
                DISCARD.to_vec(),
            ),
            (
                "frozen tick early",
                idle(Frozen),
                tick(61, none),
                Frozen,
                vec![],
            ),
            (
                "frozen tick 15m",
                idle(Frozen),
                tick(900, none),
                Discarded,
                DISCARD.to_vec(),
            ),
            (
                "disc tick",
                idle(Discarded),
                tick(3600, crit),
                Discarded,
                vec![],
            ),
            ("active busy", idle(Active), busy, Active, vec![]),
            ("bg busy", idle(Background), busy, Background, vec![]),
            (
                "frozen busy",
                idle(Frozen),
                busy,
                Background,
                vec![Unfreeze],
            ),
            ("disc busy", idle(Discarded), busy, Discarded, vec![]),
            ("active idle", idle(Active), idle_ev, Active, vec![]),
            ("frozen idle", idle(Frozen), idle_ev, Frozen, vec![]),
            ("active pin", idle(Active), pin, Active, vec![]),
            ("bg pin", idle(Background), pin, Background, vec![]),
            ("frozen pin", idle(Frozen), pin, Frozen, vec![]),
            ("disc pin", idle(Discarded), pin, Discarded, vec![]),
            (
                "active gone",
                idle(Active),
                Event::RendererGone,
                Active,
                vec![Restore],
            ),
            (
                "bg gone",
                idle(Background),
                Event::RendererGone,
                Discarded,
                vec![ReleaseTexture],
            ),
            (
                "frozen gone",
                idle(Frozen),
                Event::RendererGone,
                Discarded,
                vec![ReleaseTexture],
            ),
            (
                "disc gone",
                idle(Discarded),
                Event::RendererGone,
                Discarded,
                vec![],
            ),
            (
                "active closed",
                idle(Active),
                Event::Closed,
                Discarded,
                vec![ReleaseTexture, ReleaseSnapshot],
            ),
            (
                "bg closed",
                idle(Background),
                Event::Closed,
                Discarded,
                vec![ReleaseTexture, ReleaseSnapshot],
            ),
            (
                "frozen closed",
                idle(Frozen),
                Event::Closed,
                Discarded,
                vec![ReleaseTexture, ReleaseSnapshot],
            ),
            (
                "disc closed",
                idle(Discarded),
                Event::Closed,
                Discarded,
                vec![ReleaseSnapshot],
            ),
        ];

        for (name, state, event, want, effects) in table {
            let (next, got) = run(state, event);
            assert_eq!(next.liveness, want, "{name}: liveness");
            assert_eq!(got, effects, "{name}: effects");
            let hidden = matches!(next.liveness, Background | Frozen);
            assert_eq!(next.hidden_since.is_some(), hidden, "{name}: hidden_since");
            assert_eq!(
                next.pin,
                if matches!(event, Event::PinnedChanged(_)) {
                    Pin::Pinned
                } else {
                    state.pin
                },
                "{name}: pin"
            );
        }
    }

    #[test]
    fn freezes_at_60s_not_59s() {
        assert_eq!(
            run(idle(Background), tick(59, Pressure::None)).0.liveness,
            Background
        );
        assert_eq!(
            run(idle(Background), tick(60, Pressure::None)).0.liveness,
            Frozen
        );
    }

    #[test]
    fn critical_pressure_discards_at_once() {
        for l in [Background, Frozen] {
            let (s, fx) = run(idle(l), tick(1, Pressure::Critical));
            assert_eq!(s.liveness, Discarded);
            assert_eq!(fx, DISCARD);
        }
    }

    #[test]
    fn pinned_freezes_but_never_discards() {
        let pinned = tab(Background, Pin::Pinned, Activity::Idle);
        let (frozen, fx) = run(pinned, tick(3600, Pressure::Critical));
        assert_eq!((frozen.liveness, fx), (Frozen, vec![Freeze]));
        assert_eq!(run(frozen, tick(7200, Pressure::Critical)).1, vec![]);
    }

    #[test]
    fn busy_tabs_are_left_alone() {
        let busy = tab(Background, Pin::Unpinned, Activity::Busy);
        assert_eq!(run(busy, tick(3600, Pressure::Critical)), (busy, vec![]));
        let (s, fx) = run(idle(Frozen), Event::BusyChanged(Activity::Busy));
        assert_eq!((s.liveness, fx), (Background, vec![Unfreeze]));
        assert_eq!(run(s, tick(3600, Pressure::Critical)), (s, vec![]));
    }

    #[test]
    fn pinning_does_not_resurrect_a_discarded_tab() {
        let (s, fx) = run(idle(Discarded), Event::PinnedChanged(Pin::Pinned));
        assert_eq!((s.liveness, s.pin, fx), (Discarded, Pin::Pinned, vec![]));
    }

    #[test]
    fn crashed_tab_restores_only_when_shown() {
        let (gone, _) = run(idle(Background), Event::RendererGone);
        assert_eq!(run(gone, tick(3600, Pressure::None)).1, vec![]);
        let (shown, fx) = run(gone, Event::Shown);
        assert_eq!((shown.liveness, fx), (Active, vec![Restore, Show]));
    }

    #[test]
    fn frozen_tab_stays_frozen_when_pressure_eases() {
        let (s, _) = run(idle(Background), tick(30, Pressure::Moderate));
        assert_eq!(s.liveness, Frozen);
        assert_eq!(run(s, tick(31, Pressure::None)), (s, vec![]));
    }

    #[test]
    fn crashed_visible_tab_reloads_in_place() {
        let (s, fx) = run(Lifecycle::new(), Event::RendererGone);
        assert_eq!((s.liveness, fx), (Liveness::Active, vec![Restore]));
    }

    #[test]
    fn discard_does_not_recapture_the_snapshot() {
        let (_, fx) = run(idle(Frozen), tick(900, Pressure::None));
        assert!(!fx.contains(&CaptureSnapshot));
    }

    #[test]
    fn closing_releases_the_snapshot_of_a_discarded_tab() {
        let (s, _) = run(idle(Background), tick(900, Pressure::None));
        assert_eq!(run(s, Event::Closed).1, vec![ReleaseSnapshot]);
    }

    #[test]
    fn closing_a_restoring_tab_releases_the_snapshot_it_still_shows() {
        // Discarded, restored, closed before its first frame replaced the snapshot.
        let (s, _) = run(idle(Background), tick(900, Pressure::None));
        let (s, _) = run(s, Event::Shown);
        assert_eq!(s.liveness, Liveness::Active);
        assert!(run(s, Event::Closed).1.contains(&ReleaseSnapshot));
    }

    #[test]
    fn full_cycle_hide_freeze_discard_restore() {
        let (s, _) = run(Lifecycle::new(), Event::Hidden { now: base() });
        let (s, _) = run(s, tick(60, Pressure::None));
        let (s, _) = run(s, tick(900, Pressure::None));
        assert_eq!(s.liveness, Discarded);
        assert_eq!(run(s, Event::Shown).1, vec![Restore, Show]);
    }
}
