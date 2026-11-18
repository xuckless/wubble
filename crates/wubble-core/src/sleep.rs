//! Sleep policy: decides the target [`Liveness`] for a hidden tab.
//!
//! Pure function of tab state + time + memory pressure, so it can be tested
//! and tuned without a browser running.

use std::time::Duration;

use crate::lifecycle::{Activity, Pin};
use crate::tab::Liveness;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepPolicy {
    pub freeze_after: Duration,
    pub discard_after: Duration,
    /// Pinned tabs are frozen but never discarded on a timer.
    pub keep_pinned_alive: bool,
}

impl Default for SleepPolicy {
    fn default() -> Self {
        Self {
            freeze_after: Duration::from_secs(60),
            discard_after: Duration::from_secs(15 * 60),
            keep_pinned_alive: true,
        }
    }
}

/// System memory pressure as seen by the host (e.g. from PSI on Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    None,
    Moderate,
    Critical,
}

impl SleepPolicy {
    /// The policy the settings ask for (`settings::SLEEP_*`). Discarding never comes before
    /// freezing: a discard delay below the freeze delay is raised to it.
    pub fn from_settings(freeze_after_s: u32, discard_after_s: u32, keep_pinned: bool) -> Self {
        let freeze_after = Duration::from_secs(u64::from(freeze_after_s));
        Self {
            freeze_after,
            discard_after: Duration::from_secs(u64::from(discard_after_s)).max(freeze_after),
            keep_pinned_alive: keep_pinned,
        }
    }

    /// Target liveness for a tab that has been hidden for `hidden`.
    pub fn target(
        &self,
        hidden: Duration,
        pin: Pin,
        activity: Activity,
        pressure: Pressure,
    ) -> Liveness {
        if activity == Activity::Busy {
            return Liveness::Background;
        }
        let discardable = !(pin == Pin::Pinned && self.keep_pinned_alive);

        if discardable && (pressure == Pressure::Critical || hidden >= self.discard_after) {
            Liveness::Discarded
        } else if pressure >= Pressure::Moderate || hidden >= self.freeze_after {
            Liveness::Frozen
        } else {
            Liveness::Background
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(secs: u64, pin: Pin, activity: Activity, pressure: Pressure) -> Liveness {
        SleepPolicy::default().target(Duration::from_secs(secs), pin, activity, pressure)
    }

    #[test]
    fn escalates_with_time() {
        let t = |s| target(s, Pin::Unpinned, Activity::Idle, Pressure::None);
        assert_eq!(t(5), Liveness::Background);
        assert_eq!(t(120), Liveness::Frozen);
        assert_eq!(t(3600), Liveness::Discarded);
    }

    #[test]
    fn busy_and_pinned_are_protected() {
        assert_eq!(
            target(3600, Pin::Unpinned, Activity::Busy, Pressure::Critical),
            Liveness::Background
        );
        assert_eq!(
            target(3600, Pin::Pinned, Activity::Idle, Pressure::Critical),
            Liveness::Frozen
        );
    }

    #[test]
    fn from_settings_keeps_discard_after_freeze() {
        let p = SleepPolicy::from_settings(60, 900, true);
        assert_eq!(p, SleepPolicy::default());
        let p = SleepPolicy::from_settings(600, 60, false);
        assert_eq!(p.discard_after, Duration::from_secs(600));
        assert!(!p.keep_pinned_alive);
    }

    #[test]
    fn critical_pressure_discards_immediately() {
        assert_eq!(
            target(1, Pin::Unpinned, Activity::Idle, Pressure::Critical),
            Liveness::Discarded
        );
    }
}
