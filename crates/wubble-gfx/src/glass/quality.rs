//! Glass quality: one ordered enum and the pure governor that moves it.
//! The caller owns the clock and the window of recent costs; nothing here
//! reads time.

use std::time::Duration;

/// How the glass backdrop is produced, best first. `Ord` follows cost:
/// a greater value is cheaper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quality {
    /// Dual-Kawase pyramid from the full frame, half-resolution output.
    Full,
    /// Same with a 1-tap first level and quarter-resolution output.
    HalfRes,
    /// Reuses the last backdrops, refreshing every `CACHED_REFRESH_CALLS`-th call.
    Cached,
    /// No backdrop: the tier's flat tint only.
    Flat,
}

impl Quality {
    /// One step cheaper; `Flat` stays `Flat`.
    pub const fn lower(self) -> Self {
        match self {
            Quality::Full => Quality::HalfRes,
            Quality::HalfRes => Quality::Cached,
            Quality::Cached | Quality::Flat => Quality::Flat,
        }
    }

    /// One step better; `Full` stays `Full`.
    pub const fn higher(self) -> Self {
        match self {
            Quality::Full | Quality::HalfRes => Quality::Full,
            Quality::Cached => Quality::HalfRes,
            Quality::Flat => Quality::Cached,
        }
    }
}

/// Number of most recent frame costs the governor looks at.
pub const WINDOW: usize = 8;
/// Step down when at least this many of the window are over the down line.
pub const DOWN_COUNT: usize = 3;
/// The down line, as a fraction of the frame budget.
pub const DOWN_FRACTION: f64 = 0.9;
/// Step up only when the whole window is under this fraction of the budget.
pub const UP_FRACTION: f64 = 0.6;
/// Under `Cached`, one real render per this many calls.
pub const CACHED_REFRESH_CALLS: u32 = 8;

/// The quality for the next frame, from the costs of the most recent frames
/// (oldest first; only the last `WINDOW` count) and the frame budget
/// (8.33 ms at 120 Hz). Steps down one level when `DOWN_COUNT` costs exceed
/// `budget * DOWN_FRACTION`; steps up one level only on a full window that is
/// entirely under `budget * UP_FRACTION`; otherwise holds. The gap between
/// the two lines is the hysteresis.
///
/// The host clears its window whenever the quality changes, so each level is
/// judged on its own frames.
pub fn next_quality(current: Quality, recent: &[Duration], budget: Duration) -> Quality {
    let window = &recent[recent.len().saturating_sub(WINDOW)..];
    let down = budget.mul_f64(DOWN_FRACTION);
    let up = budget.mul_f64(UP_FRACTION);
    if window.iter().filter(|cost| **cost > down).count() >= DOWN_COUNT {
        current.lower()
    } else if window.len() == WINDOW && window.iter().all(|cost| *cost < up) {
        current.higher()
    } else {
        current
    }
}

/// Whether `GlassBackdrop::render` does GPU work this call, given how many
/// calls in a row it has skipped. `Cached` renders once every
/// `CACHED_REFRESH_CALLS` calls; a counter, not a clock.
pub const fn should_render(quality: Quality, skipped: u32) -> bool {
    match quality {
        Quality::Full | Quality::HalfRes => true,
        Quality::Cached => skipped + 1 >= CACHED_REFRESH_CALLS,
        Quality::Flat => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: Duration = Duration::from_micros(8333);
    const OVER: Duration = Duration::from_millis(9);
    const CALM: Duration = Duration::from_millis(3);
    /// Between the up and the down line.
    const MIDDLE: Duration = Duration::from_millis(6);

    #[test]
    fn ordering_follows_cost() {
        assert!(Quality::Full < Quality::HalfRes);
        assert!(Quality::HalfRes < Quality::Cached);
        assert!(Quality::Cached < Quality::Flat);
        assert_eq!(Quality::Flat.lower(), Quality::Flat);
        assert_eq!(Quality::Full.higher(), Quality::Full);
        for q in [Quality::Full, Quality::HalfRes, Quality::Cached] {
            assert!(q.lower() > q);
            assert_eq!(q.lower().higher(), q);
        }
    }

    #[test]
    fn over_budget_steps_down_one_level_to_flat() {
        let costs = [CALM, OVER, CALM, OVER, OVER];
        let mut q = Quality::Full;
        for expected in [
            Quality::HalfRes,
            Quality::Cached,
            Quality::Flat,
            Quality::Flat,
        ] {
            q = next_quality(q, &costs, BUDGET);
            assert_eq!(q, expected);
        }
    }

    #[test]
    fn calm_full_window_steps_up_one_level_to_full() {
        let costs = [CALM; WINDOW];
        let mut q = Quality::Flat;
        for expected in [
            Quality::Cached,
            Quality::HalfRes,
            Quality::Full,
            Quality::Full,
        ] {
            q = next_quality(q, &costs, BUDGET);
            assert_eq!(q, expected);
        }
    }

    #[test]
    fn short_empty_mixed_and_middle_windows_hold() {
        let hold = |costs: &[Duration]| {
            assert_eq!(
                next_quality(Quality::HalfRes, costs, BUDGET),
                Quality::HalfRes
            );
        };
        hold(&[]);
        hold(&[CALM; WINDOW - 1]);
        hold(&[CALM, OVER, CALM, CALM, OVER, CALM, CALM, CALM]);
        hold(&[MIDDLE; WINDOW]);
        // A cost exactly on the down line is not over it.
        hold(&[BUDGET.mul_f64(DOWN_FRACTION); WINDOW]);
    }

    #[test]
    fn only_the_last_window_counts() {
        let mut costs = vec![OVER; 5];
        costs.extend([CALM; WINDOW]);
        assert_eq!(
            next_quality(Quality::Cached, &costs, BUDGET),
            Quality::HalfRes
        );
    }

    #[test]
    fn cached_renders_on_every_eighth_call() {
        // The pass starts with `skipped = CACHED_REFRESH_CALLS - 1` so its
        // first call renders, then skips the next seven.
        let mut skipped = CACHED_REFRESH_CALLS - 1;
        let rendered: Vec<bool> = (0..17)
            .map(|_| {
                let render = should_render(Quality::Cached, skipped);
                skipped = if render { 0 } else { skipped + 1 };
                render
            })
            .collect();
        let calls: Vec<usize> = (0..17).filter(|i| rendered[*i]).collect();
        assert_eq!(calls, [0, 8, 16]);
    }

    #[test]
    fn other_qualities_always_or_never_render() {
        for skipped in [0, 1, 7, 100] {
            assert!(should_render(Quality::Full, skipped));
            assert!(should_render(Quality::HalfRes, skipped));
            assert!(!should_render(Quality::Flat, skipped));
        }
    }
}
