//! Sleep policy: decides the target [`Liveness`] for a hidden tab.
//!
//! Pure function of tab state + time + memory pressure, so it can be tested
//! and tuned without a browser running.

use std::time::{Duration, Instant};

use crate::tab::{Liveness, Tab};

#[derive(Debug, Clone)]
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
    /// Target liveness for a tab that is not currently visible.
    pub fn target(&self, tab: &Tab, now: Instant, pressure: Pressure) -> Liveness {
        if tab.busy {
            return Liveness::Background;
        }
        let hidden = now.saturating_duration_since(tab.last_visible);
        let discardable = !(tab.pinned && self.keep_pinned_alive);

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
    use crate::tab::TabId;

    fn tab(hidden_secs: u64, now: Instant) -> Tab {
        Tab {
            id: TabId(1),
            url: String::new(),
            title: String::new(),
            liveness: Liveness::Background,
            pinned: false,
            busy: false,
            last_visible: now - Duration::from_secs(hidden_secs),
        }
    }

    #[test]
    fn escalates_with_time() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let p = SleepPolicy::default();
        assert_eq!(p.target(&tab(5, now), now, Pressure::None), Liveness::Background);
        assert_eq!(p.target(&tab(120, now), now, Pressure::None), Liveness::Frozen);
        assert_eq!(p.target(&tab(3600, now), now, Pressure::None), Liveness::Discarded);
    }

    #[test]
    fn busy_and_pinned_are_protected() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let p = SleepPolicy::default();

        let mut busy = tab(3600, now);
        busy.busy = true;
        assert_eq!(p.target(&busy, now, Pressure::Critical), Liveness::Background);

        let mut pinned = tab(3600, now);
        pinned.pinned = true;
        assert_eq!(p.target(&pinned, now, Pressure::Critical), Liveness::Frozen);
    }

    #[test]
    fn critical_pressure_discards_immediately() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let p = SleepPolicy::default();
        assert_eq!(p.target(&tab(1, now), now, Pressure::Critical), Liveness::Discarded);
    }
}
