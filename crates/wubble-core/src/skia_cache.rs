//! When the shell's Skia caches are bounded and cleaned (the vendored renderer's `cache`, wubble
//! change 4). Chromium's `GrCacheController` (BSD-3-Clause, the idea only): a cache that has not
//! drawn for a few seconds frees what it has not used for a few seconds; a hidden app or one
//! under critical memory pressure frees everything unlocked; moderate pressure frees what is not
//! in use now. Pure: the host feeds it its renders, its lifecycle tick, its windows' occlusion
//! and the pressure, and runs the [`Cleanup`] it returns.

use std::time::{Duration, Instant};

use crate::sleep::Pressure;

/// No render for this long: the cache is idle.
pub const IDLE_AFTER: Duration = Duration::from_secs(5);

/// What an idle cleanup frees: the resources not used for this long.
pub const UNUSED: Duration = Duration::from_secs(5);

/// The cache's state. Every transition is a row of [`next`]; a new rule is a row there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cache {
    /// The shell drew at `last`.
    Drawing { last: Instant },
    /// Idle, its unused resources freed: nothing more to free until it draws again.
    Cleaned,
    /// Everything unlocked freed (hidden, critical pressure).
    Purged,
}

impl Default for Cache {
    /// Before the first render there is nothing to free.
    fn default() -> Self {
        Self::Cleaned
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A window rendered.
    Rendered(Instant),
    /// The lifecycle tick.
    Tick(Instant),
    /// Every window is hidden (minimised, occluded).
    Hidden,
    /// The memory pressure the lifecycle tick read.
    Pressure(Pressure),
}

/// What to free (the renderer's `CacheCleanup`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cleanup {
    /// The resources not used for this long.
    Deferred(Duration),
    /// Everything unlocked.
    PurgeAll,
}

/// The state after `event`, and what to free.
pub fn next(cache: Cache, event: Event) -> (Cache, Option<Cleanup>) {
    use {Cache::*, Event::*};
    match (cache, event) {
        (_, Rendered(at)) => (Drawing { last: at }, None),
        (Drawing { last }, Tick(now)) if now.saturating_duration_since(last) >= IDLE_AFTER => {
            (Cleaned, Some(Cleanup::Deferred(UNUSED)))
        }
        (_, Tick(_)) => (cache, None),
        // nothing was drawn since everything went
        (Purged, Hidden | Pressure(_)) => (Purged, None),
        (_, Hidden | Pressure(crate::sleep::Pressure::Critical)) => {
            (Purged, Some(Cleanup::PurgeAll))
        }
        (Drawing { .. }, Pressure(crate::sleep::Pressure::Moderate)) => {
            (Cleaned, Some(Cleanup::Deferred(Duration::ZERO)))
        }
        (_, Pressure(_)) => (cache, None),
    }
}

/// The byte budget of a window's cache for a window of `physical_px` pixels: four window-sized
/// RGBA layers (the shell's glass, opacity and clip layers at most at once) and room for its
/// glyphs and images, between 48 and 192 MiB. Chromium sizes its caches from the device's
/// memory (`DetermineGrCacheLimitsFromAvailableMemory`: 96 MiB, 256 MiB on large machines, the
/// idea only); the shell's need follows its window instead.
pub fn limit(physical_px: u64) -> usize {
    const MIB: u64 = 1 << 20;
    let bytes = physical_px.saturating_mul(16).clamp(48 * MIB, 192 * MIB);
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::Pressure as P;

    #[test]
    fn every_state_and_event() {
        use {Cache::*, Cleanup::*, Event::*};
        let t = Instant::now();
        let s = Duration::from_secs;
        let drawing = Drawing { last: t };
        let idle = Some(Deferred(UNUSED));
        let all = Some(PurgeAll);
        let unused_now = Some(Deferred(Duration::ZERO));
        // (name, state, event, next, cleanup)
        for (name, state, event, want, cleanup) in [
            (
                "a render starts drawing",
                Cleaned,
                Rendered(t),
                drawing,
                None,
            ),
            ("a render after a purge", Purged, Rendered(t), drawing, None),
            (
                "a render while drawing",
                Drawing { last: t - s(1) },
                Rendered(t),
                drawing,
                None,
            ),
            (
                "a tick soon after a render",
                drawing,
                Tick(t + s(4)),
                drawing,
                None,
            ),
            (
                "a tick at the idle mark",
                drawing,
                Tick(t + IDLE_AFTER),
                Cleaned,
                idle,
            ),
            (
                "a tick long after",
                drawing,
                Tick(t + s(600)),
                Cleaned,
                idle,
            ),
            (
                "time going backwards",
                drawing,
                Tick(t - s(10)),
                drawing,
                None,
            ),
            (
                "a tick when cleaned",
                Cleaned,
                Tick(t + s(600)),
                Cleaned,
                None,
            ),
            ("a tick when purged", Purged, Tick(t + s(600)), Purged, None),
            ("hidden while drawing", drawing, Hidden, Purged, all),
            ("hidden when cleaned", Cleaned, Hidden, Purged, all),
            ("hidden again", Purged, Hidden, Purged, None),
            (
                "no pressure while drawing",
                drawing,
                Pressure(P::None),
                drawing,
                None,
            ),
            (
                "no pressure when cleaned",
                Cleaned,
                Pressure(P::None),
                Cleaned,
                None,
            ),
            (
                "moderate while drawing",
                drawing,
                Pressure(P::Moderate),
                Cleaned,
                unused_now,
            ),
            (
                "moderate when cleaned",
                Cleaned,
                Pressure(P::Moderate),
                Cleaned,
                None,
            ),
            (
                "moderate when purged",
                Purged,
                Pressure(P::Moderate),
                Purged,
                None,
            ),
            (
                "critical while drawing",
                drawing,
                Pressure(P::Critical),
                Purged,
                all,
            ),
            (
                "critical when cleaned",
                Cleaned,
                Pressure(P::Critical),
                Purged,
                all,
            ),
            (
                "critical when purged",
                Purged,
                Pressure(P::Critical),
                Purged,
                None,
            ),
        ] {
            assert_eq!(next(state, event), (want, cleanup), "{name}");
        }
    }

    #[test]
    fn the_budget_follows_the_window() {
        const MIB: usize = 1 << 20;
        for (name, px, mib) in [
            ("no window", 0, 48),
            ("1280 x 800", 1280 * 800, 48),
            ("3456 x 2234 (a 16-inch Retina)", 3456 * 2234, 117),
            ("6016 x 3384 (a 6K display)", 6016 * 3384, 192),
            ("absurd", u64::MAX, 192),
        ] {
            assert_eq!(limit(px) / MIB, mib, "{name}");
        }
    }
}
