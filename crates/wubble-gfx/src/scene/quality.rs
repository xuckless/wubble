//! Scene quality: one ordered enum and the pure governor that moves it.
//! The caller owns the clock and feeds one measured frame cost per call;
//! nothing here reads time. Glass degrades before animation drops a frame:
//! each level makes the backdrop (fluid, composite, blur pyramid) cheaper,
//! and the last one also draws the glass as a flat fill. The beam and the
//! sheen ring stay live at every level.

use std::time::Duration;

/// How the backdrop behind the glass, and at the last level the glass, is
/// produced, best first. `Ord` follows
/// cost: a greater value is cheaper. Each level is a backdrop resolution
/// ([`Quality::scale`]), a pressure-solver iteration count
/// ([`Quality::pressure_iters`]) and a refresh rule ([`should_render`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quality {
    /// The design's backdrop: composite at half the logical size, pyramid at
    /// 1/4 and 1/8, 18 Jacobi iterations.
    Full,
    /// Composite and pyramid at half of `Full`, 9 Jacobi iterations.
    HalfRes,
    /// As `HalfRes`, but the backdrop moves only every
    /// `CACHED_REFRESH_CALLS`-th frame, by the time of all of them (see
    /// [`BackdropClock`]); the other frames redraw the glass over the last
    /// pyramid.
    Cached,
    /// Frozen backdrop (the last pyramid, rendered only when it does not
    /// exist yet) and flat glass: one fill colour per panel, the rim, the
    /// sheen ring and the beam; no refraction, dispersion, press waves,
    /// specular or pointer light.
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

    /// Size of the composite and blur pyramid targets relative to `Full`
    /// (whose composite is half the logical view size). The fluid grids keep
    /// their size, so the liquid survives a change of level.
    pub const fn scale(self) -> f32 {
        match self {
            Quality::Full => 1.0,
            Quality::HalfRes | Quality::Cached | Quality::Flat => 0.5,
        }
    }

    /// Jacobi pressure iterations per fluid step, given the design's count
    /// at `Full` (18).
    pub const fn pressure_iters(self, full: u32) -> u32 {
        match self {
            Quality::Full => full,
            Quality::HalfRes | Quality::Cached | Quality::Flat => full.div_ceil(2),
        }
    }

    /// Over-budget frames among the last `WINDOW` that step this level down.
    /// `Cached` renders once per `CACHED_REFRESH_CALLS` frames, so one
    /// over-budget frame in the window is already its refresh failing.
    const fn down_count(self) -> u32 {
        match self {
            Quality::Full | Quality::HalfRes => DOWN_COUNT,
            Quality::Cached => 1,
            // Nothing cheaper to go to.
            Quality::Flat => u32::MAX,
        }
    }
}

/// Number of most recent frame costs the down rule looks at (bits of `u8`).
pub const WINDOW: u32 = 8;
/// `Full` and `HalfRes` step down when this many of the window are over the
/// down line.
pub const DOWN_COUNT: u32 = 3;
/// The down line, as a fraction of the frame budget.
pub const DOWN_FRACTION: f64 = 0.9;
/// Calm frames are under this fraction of the budget.
pub const UP_FRACTION: f64 = 0.6;
/// Under `Cached`, one real render per this many calls.
pub const CACHED_REFRESH_CALLS: u32 = 8;
/// A step down within this many frames of a step up is a failed retry.
pub const PROBE_FRAMES: u32 = 4 * WINDOW;
/// Cap on the retry backoff: at most `WINDOW << MAX_BACKOFF` calm frames
/// (about 9 minutes at 120 Hz) before retrying a level that failed.
pub const MAX_BACKOFF: u32 = 16;

/// The governor's state, owned by the host and moved only by
/// [`Governor::next`]. Remembers why it last stepped down, so a level that
/// fails under steady load is retried after exponentially longer calm runs
/// instead of every `WINDOW` frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Governor {
    quality: Quality,
    /// Bit `i` set: the frame `i` calls ago was over the down line.
    /// Cleared on every change, so each level is judged on its own frames.
    over: u8,
    /// Consecutive calm frames at this level.
    calm: u32,
    /// Frames at this level.
    frames: u32,
    /// This level was entered by stepping up, so failing it soon is a
    /// failed retry.
    probing: bool,
    /// Failed retries in a row; the up rule needs `WINDOW << backoff` calm
    /// frames.
    backoff: u32,
}

impl Governor {
    pub const fn new(quality: Quality) -> Self {
        Self {
            quality,
            over: 0,
            calm: 0,
            frames: 0,
            probing: false,
            backoff: 0,
        }
    }

    pub const fn quality(&self) -> Quality {
        self.quality
    }

    /// The state after one frame that cost `cost`, against `budget`
    /// (8.33 ms at 120 Hz). Steps down one level when the window holds
    /// `down_count` costs over `budget * DOWN_FRACTION`; steps up one level
    /// after `WINDOW << backoff` consecutive costs under `budget * UP_FRACTION`;
    /// otherwise holds. A step down within `PROBE_FRAMES` of a step up doubles
    /// the calm run the next step up needs; a level that survives
    /// `PROBE_FRAMES` after a step up resets it.
    #[must_use]
    pub fn next(self, cost: Duration, budget: Duration) -> Self {
        let over = (self.over << 1) | u8::from(cost > budget.mul_f64(DOWN_FRACTION));
        let calm = if cost < budget.mul_f64(UP_FRACTION) {
            self.calm.saturating_add(1)
        } else {
            0
        };
        let frames = self.frames.saturating_add(1);
        let changed = |quality, probing, backoff| Self {
            probing,
            backoff,
            ..Self::new(quality)
        };
        if over.count_ones() >= self.quality.down_count() {
            let backoff = if self.probing {
                (self.backoff + 1).min(MAX_BACKOFF)
            } else {
                self.backoff
            };
            return changed(self.quality.lower(), false, backoff);
        }
        if self.quality != Quality::Full && calm >= WINDOW << self.backoff {
            return changed(self.quality.higher(), true, self.backoff);
        }
        let survived = self.probing && frames >= PROBE_FRAMES;
        Self {
            quality: self.quality,
            over,
            calm,
            frames,
            probing: self.probing && !survived,
            backoff: if survived { 0 } else { self.backoff },
        }
    }
}

/// Whether the backdrop (fluid step, composite, blur pyramid) is rendered
/// this frame, given how many frames in a row it has skipped. `Cached` renders once every
/// `CACHED_REFRESH_CALLS` calls; a counter, not a clock.
pub const fn should_render(quality: Quality, skipped: u32) -> bool {
    match quality {
        Quality::Full | Quality::HalfRes => true,
        Quality::Cached => skipped + 1 >= CACHED_REFRESH_CALLS,
        Quality::Flat => false,
    }
}

/// Longest frame step, seconds: a stall does not fling the liquid.
pub const MAX_DT: f32 = 0.032;
/// Longest backdrop step, seconds: the time of one `Cached` refresh at
/// `MAX_DT` per frame. Advection is semi-Lagrangian and vorticity is
/// clamped, so one long step stays stable.
pub const MAX_BACKDROP_DT: f32 = CACHED_REFRESH_CALLS as f32 * MAX_DT;

/// When the backdrop moves and by how much. Frames whose backdrop is skipped
/// bank their time, so under `Cached` the liquid keeps its real speed and
/// only moves in coarser steps. The bank is capped at [`MAX_BACKDROP_DT`],
/// so leaving `Flat` does not fling it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropClock {
    /// Frames in a row whose backdrop was not rendered.
    skipped: u32,
    /// Seconds not yet stepped by the fluid.
    banked: f32,
}

impl Default for BackdropClock {
    fn default() -> Self {
        Self::new()
    }
}

impl BackdropClock {
    /// The first call renders, also under `Cached`.
    pub const fn new() -> Self {
        Self {
            skipped: CACHED_REFRESH_CALLS - 1,
            banked: 0.0,
        }
    }

    /// One frame of `dt` seconds. Returns the time to step the backdrop by
    /// when it is rendered this frame (`should_render`, or `force` when there
    /// is no backdrop yet), `None` when it is skipped.
    pub fn tick(&mut self, quality: Quality, dt: f32, force: bool) -> Option<f32> {
        self.banked = (self.banked + dt).min(MAX_BACKDROP_DT);
        if force || should_render(quality, self.skipped) {
            self.skipped = 0;
            Some(std::mem::take(&mut self.banked))
        } else {
            self.skipped = self.skipped.saturating_add(1);
            None
        }
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

    fn feed(mut g: Governor, costs: impl IntoIterator<Item = Duration>) -> Governor {
        for cost in costs {
            g = g.next(cost, BUDGET);
        }
        g
    }

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
        let mut g = Governor::new(Quality::Full);
        g = feed(g, [CALM, OVER, CALM, OVER]);
        assert_eq!(g.quality(), Quality::Full);
        g = feed(g, [OVER]);
        assert_eq!(g.quality(), Quality::HalfRes);
        g = feed(g, [OVER; 3]);
        assert_eq!(g.quality(), Quality::Cached);
        // One over-budget frame is enough at Cached: it is the refresh.
        g = feed(g, [OVER]);
        assert_eq!(g.quality(), Quality::Flat);
        g = feed(g, [OVER; 20]);
        assert_eq!(g.quality(), Quality::Flat);
    }

    #[test]
    fn calm_run_steps_up_one_level_to_full() {
        let mut g = Governor::new(Quality::Flat);
        for expected in [Quality::Cached, Quality::HalfRes, Quality::Full] {
            g = feed(g, [CALM; WINDOW as usize - 1]);
            assert_ne!(g.quality(), expected);
            g = feed(g, [CALM]);
            assert_eq!(g.quality(), expected);
        }
        g = feed(g, [CALM; 100]);
        assert_eq!(g.quality(), Quality::Full);
    }

    #[test]
    fn mixed_and_middle_costs_hold() {
        let start = Governor::new(Quality::HalfRes);
        let held = |costs: &[Duration]| feed(start, costs.iter().copied()).quality();
        assert_eq!(held(&[]), Quality::HalfRes);
        assert_eq!(
            held(&[CALM, OVER, CALM, CALM, OVER, CALM, CALM, CALM]),
            Quality::HalfRes
        );
        assert_eq!(held(&[MIDDLE; 100]), Quality::HalfRes);
        // A cost exactly on the down line is not over it.
        assert_eq!(
            held(&[BUDGET.mul_f64(DOWN_FRACTION); 100]),
            Quality::HalfRes
        );
    }

    #[test]
    fn only_the_last_window_counts_down() {
        // Two old over-budget frames slide out before the third arrives.
        let mut costs = vec![OVER, OVER];
        costs.extend([MIDDLE; WINDOW as usize - 1]);
        costs.push(OVER);
        let g = feed(Governor::new(Quality::Full), costs);
        assert_eq!(g.quality(), Quality::Full);
    }

    /// Runs the governor over `frames` frames whose cost `cost(quality,
    /// rendered)` depends on the level (`rendered` is 1 on a `Cached`
    /// refresh); returns the end state and how many frames went over budget.
    fn run(
        start: Quality,
        frames: usize,
        cost: impl Fn(Quality, u32) -> Duration,
    ) -> (Governor, usize) {
        let mut g = Governor::new(start);
        let mut clock = BackdropClock::new();
        let mut over = 0;
        for _ in 0..frames {
            let q = g.quality();
            let render = clock.tick(q, 1.0 / 120.0, false).is_some();
            let c = cost(q, u32::from(render));
            over += usize::from(c > BUDGET);
            g = g.next(c, BUDGET);
        }
        (g, over)
    }

    #[test]
    fn steady_load_does_not_flap_between_two_levels() {
        // Full 9 ms (over), HalfRes 4 ms (calm). Without memory this drops
        // 3 frames in every 11; with backoff the retries thin out.
        let frames = 100_000;
        let (g, over) = run(Quality::Full, frames, |q, _| {
            if q == Quality::Full {
                OVER
            } else {
                Duration::from_millis(4)
            }
        });
        assert_eq!(g.quality(), Quality::HalfRes);
        // Each retry costs DOWN_COUNT frames; retries double their spacing,
        // so there are about log2(frames / WINDOW) of them.
        let retries = (frames as f64 / WINDOW as f64).log2().ceil() as usize + 1;
        assert!(
            over <= DOWN_COUNT as usize * retries,
            "{over} over-budget frames"
        );
    }

    #[test]
    fn slow_cached_refresh_steps_down_and_flat_does_not_flap() {
        // Only the Cached refresh frame is slow; HalfRes and Full are too.
        let frames = 100_000;
        let (g, over) = run(Quality::Cached, frames, |q, rendered| match q {
            Quality::Flat => CALM,
            Quality::Cached if rendered == 0 => CALM,
            _ => OVER,
        });
        assert_eq!(g.quality(), Quality::Flat);
        let retries = (frames as f64 / WINDOW as f64).log2().ceil() as usize + 1;
        assert!(over <= retries, "{over} over-budget frames");
    }

    #[test]
    fn a_retry_that_holds_resets_the_backoff() {
        let mut g = Governor::new(Quality::Full);
        // Fail Full a few times from HalfRes.
        for _ in 0..4 {
            g = feed(g, [OVER; DOWN_COUNT as usize]);
            assert_eq!(g.quality(), Quality::HalfRes);
            while g.quality() == Quality::HalfRes {
                g = g.next(CALM, BUDGET);
            }
        }
        assert_eq!(g.backoff, 3);
        // The load is gone: Full holds past the probe and the memory clears.
        g = feed(g, [CALM; PROBE_FRAMES as usize]);
        assert_eq!((g.quality(), g.backoff), (Quality::Full, 0));
        // A fresh spike then recovers after one plain calm window.
        g = feed(g, [OVER; DOWN_COUNT as usize]);
        assert_eq!(g.quality(), Quality::HalfRes);
        g = feed(g, [CALM; WINDOW as usize]);
        assert_eq!(g.quality(), Quality::Full);
    }

    #[test]
    fn cached_renders_on_every_eighth_call() {
        // The first call renders, then the next seven are skipped.
        let mut clock = BackdropClock::new();
        let calls: Vec<usize> = (0..17)
            .filter(|_| clock.tick(Quality::Cached, 0.01, false).is_some())
            .collect();
        assert_eq!(calls, [0, 8, 16]);
    }

    /// Seconds the backdrop steps over `frames` frames of `dt`.
    fn stepped(quality: Quality, frames: usize, dt: f32) -> f32 {
        let mut clock = BackdropClock::new();
        (0..frames)
            .filter_map(|_| clock.tick(quality, dt, false))
            .sum()
    }

    #[test]
    fn cached_backdrop_keeps_real_speed() {
        // 8 renders over 64 frames: every frame's time is stepped, whether it
        // rendered or not, at 60 Hz and at 120 Hz.
        for dt in [1.0 / 60.0, 1.0 / 120.0] {
            let frames = 8 * CACHED_REFRESH_CALLS as usize;
            let full = stepped(Quality::Full, frames, dt);
            let cached = stepped(Quality::Cached, frames, dt);
            // the last 7 frames are banked for the next refresh
            let banked = (CACHED_REFRESH_CALLS - 1) as f32 * dt;
            assert!((full - frames as f32 * dt).abs() < 1e-4);
            assert!(
                (cached - (full - banked)).abs() < 1e-4,
                "{cached} vs {full}"
            );
        }
    }

    #[test]
    fn banked_time_is_capped_and_flat_steps_only_when_forced() {
        let mut clock = BackdropClock::new();
        assert_eq!(clock.tick(Quality::Flat, MAX_DT, true), Some(MAX_DT));
        for _ in 0..100 {
            assert_eq!(clock.tick(Quality::Flat, MAX_DT, false), None);
        }
        // leaving Flat steps at most one Cached refresh's worth
        assert_eq!(
            clock.tick(Quality::Full, MAX_DT, false),
            Some(MAX_BACKDROP_DT)
        );
        assert_eq!(clock.tick(Quality::Full, 0.01, false), Some(0.01));
    }

    #[test]
    fn cheaper_levels_shrink_the_backdrop() {
        assert_eq!(Quality::Full.scale(), 1.0);
        assert_eq!(Quality::Full.pressure_iters(18), 18);
        for q in [Quality::HalfRes, Quality::Cached, Quality::Flat] {
            assert_eq!(q.scale(), 0.5);
            assert_eq!(q.pressure_iters(18), 9);
        }
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
