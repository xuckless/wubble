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

    /// Position in [`Governor`]'s per-level memory.
    const fn index(self) -> usize {
        self as usize
    }
}

/// Weight of the newest frame in a level's smoothed load once the level has
/// run `1 / AVG_WEIGHT` frames; before that the load is their plain mean, so
/// the first frame after a change does not stand for the level.
pub const AVG_WEIGHT: f64 = 1.0 / 16.0;
/// One frame counts as at most this many budgets: a hitch (a shader compile,
/// a resize, an idle gap) moves the average by a bounded step, so only a
/// sustained overrun steps down.
pub const SAMPLE_CAP: f64 = 2.0;
/// Frames a level runs before its load may step it down.
pub const SETTLE_FRAMES: u32 = 32;
/// Frames a level runs before it may step up: 1.2 s at 300 Hz, 3 s at 120 Hz.
pub const HOLD_FRAMES: u32 = 360;
/// A step down within this many frames of a step up is a failed upgrade.
pub const PROBE_FRAMES: u32 = 4 * HOLD_FRAMES;
/// Cap on the retry back-off: a level known not to fit is retried after at
/// most `HOLD_FRAMES << MAX_BACKOFF` frames (about 77 s at 300 Hz).
pub const MAX_BACKOFF: u32 = 6;
/// The down line: a level whose smoothed load is above this fraction of the
/// budget steps down.
pub const DOWN_FRACTION: f64 = 0.9;
/// A level whose smoothed load is under this fraction of the budget has the
/// headroom to step up.
pub const UP_FRACTION: f64 = 0.6;
/// The level above is predicted to fit when its last measured load is under
/// this fraction: below the down line, so the prediction keeps a margin.
pub const FIT_FRACTION: f64 = 0.8;
/// Without the scene's own cost ([`Signal::Lateness`]), a step down counts as
/// having helped when the lower level's load is at most this fraction of the
/// load that moved it.
pub const HELP_FRACTION: f64 = 0.75;
/// Under `Cached`, one real render per this many calls.
pub const CACHED_REFRESH_CALLS: u32 = 8;

/// What the load fed to [`Governor::next`] measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The scene's own GPU time (timestamps): a load over the line is the
    /// scene's, and a cheaper level lowers it.
    SceneCost,
    /// Only the frame's lateness (no GPU timestamps, wgpu on Metal here): the
    /// whole frame, which can be late for reasons no level changes (the UI
    /// thread blocked elsewhere, a slow page). Each step down is a trial: if
    /// the lower level does not cut the load to [`HELP_FRACTION`] of what it
    /// was, the governor steps back up and holds there for
    /// `HOLD_FRAMES << backoff` frames, the back-off growing with each failed
    /// trial. A frame slow at every level so stays at `Full`.
    Lateness,
}

/// The governor's state, owned by the host and moved only by
/// [`Governor::next`].
///
/// Each level's load is a smoothed average of the frames run at it, kept
/// after leaving it. Stepping up needs headroom at this level and a
/// prediction, from the load last measured at the level above, that it
/// fits. A level predicted not to fit is retried after a hold that doubles
/// each time a step up is soon followed by a step down, so a GPU that cannot
/// afford a level settles below it instead of cycling, and a transient spike
/// costs at most one round trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Governor {
    quality: Quality,
    /// Smoothed load per level ([`Quality::index`]), `None` until measured.
    load: [Option<Duration>; 4],
    /// Frames at this level.
    frames: u32,
    /// This level was entered by stepping up and has not yet held for
    /// `PROBE_FRAMES`.
    probing: bool,
    /// Failed changes in a row (an upgrade soon undone, a downgrade that did
    /// not help); a step up against the prediction, or a step down after a
    /// trial that did not help, waits `HOLD_FRAMES << backoff` frames.
    backoff: u32,
    signal: Signal,
    /// [`Signal::Lateness`]: the level this one was entered from by a step
    /// down that has not yet shown it helped.
    trial: Option<Quality>,
    /// Frames left during which this level does not step down (after a trial
    /// that did not help).
    held: u32,
}

impl Governor {
    pub const fn new(quality: Quality, signal: Signal) -> Self {
        Self {
            quality,
            load: [None; 4],
            frames: 0,
            probing: false,
            backoff: 0,
            signal,
            trial: None,
            held: 0,
        }
    }

    pub const fn quality(&self) -> Quality {
        self.quality
    }

    pub const fn signal(&self) -> Signal {
        self.signal
    }

    /// The smoothed load last measured at `quality`, `None` if it never ran.
    pub const fn load_at(&self, quality: Quality) -> Option<Duration> {
        self.load[quality.index()]
    }

    /// The state after one frame whose load was `cost`, against `budget`
    /// (`backdrop::frame_load` and `backdrop::frame_budget`). Steps down one
    /// level when, after `SETTLE_FRAMES`, the level's smoothed load is over
    /// `budget * DOWN_FRACTION` (unless held, see [`Signal::Lateness`]).
    /// Steps up one level when, after `HOLD_FRAMES`, it is under
    /// `budget * UP_FRACTION` and either the level above last measured under
    /// `budget * FIT_FRACTION` (or never ran) or `HOLD_FRAMES << backoff`
    /// frames have passed. Under [`Signal::Lateness`] a step down that did
    /// not help is undone first. Otherwise holds.
    #[must_use]
    pub fn next(self, cost: Duration, budget: Duration) -> Self {
        let sample = cost.min(budget.mul_f64(SAMPLE_CAP));
        let frames = self.frames.saturating_add(1);
        let here = self.quality.index();
        let weight = (1.0 / f64::from(frames)).max(AVG_WEIGHT);
        let avg = match self.load[here] {
            Some(avg) if self.frames > 0 => avg.mul_f64(1.0 - weight) + sample.mul_f64(weight),
            _ => sample,
        };
        let mut load = self.load;
        load[here] = Some(avg);
        let change = |quality, probing, backoff, trial, held| Self {
            quality,
            load,
            frames: 0,
            probing,
            backoff,
            signal: self.signal,
            trial,
            held,
        };
        let settled = frames >= SETTLE_FRAMES;
        // A step down on lateness alone, judged once this level has settled:
        // no clear drop means the scene was not what made frames late.
        if let Some(from) = self.trial.filter(|_| settled)
            && !load[from.index()].is_some_and(|was| avg <= was.mul_f64(HELP_FRACTION))
        {
            let backoff = (self.backoff + 1).min(MAX_BACKOFF);
            return change(from, false, backoff, None, HOLD_FRAMES << self.backoff);
        }
        let trial = self.trial.filter(|_| !settled);
        let held = self.held.saturating_sub(1);
        if self.quality != Quality::Flat
            && settled
            && self.held == 0
            && avg > budget.mul_f64(DOWN_FRACTION)
        {
            let backoff = if self.probing {
                (self.backoff + 1).min(MAX_BACKOFF)
            } else {
                self.backoff
            };
            let trial = match self.signal {
                Signal::SceneCost => None,
                Signal::Lateness => Some(self.quality),
            };
            return change(self.quality.lower(), false, backoff, trial, 0);
        }
        if self.quality != Quality::Full
            && frames >= HOLD_FRAMES
            && avg < budget.mul_f64(UP_FRACTION)
        {
            let above = load[self.quality.higher().index()];
            let fits = above.is_none_or(|c| c < budget.mul_f64(FIT_FRACTION));
            if fits || frames >= HOLD_FRAMES << self.backoff {
                return change(self.quality.higher(), true, self.backoff, None, 0);
            }
        }
        let survived = self.probing && frames >= PROBE_FRAMES;
        Self {
            quality: self.quality,
            load,
            frames,
            probing: self.probing && !survived,
            backoff: if survived { 0 } else { self.backoff },
            signal: self.signal,
            trial,
            held,
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

    /// `fraction` of the budget.
    fn of(fraction: f64) -> Duration {
        BUDGET.mul_f64(fraction)
    }

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

    /// A deterministic jitter in `-spread..spread` (an LCG), so traces are
    /// noisy without being random.
    fn jitter(seed: &mut u64, spread: f64) -> f64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * spread
    }

    /// Runs `frames` frames whose load is `cost(quality, frame)`; returns
    /// the end state, the level changes, and the frames spent per level.
    fn run(
        start: Quality,
        frames: usize,
        cost: impl FnMut(Quality, usize) -> Duration,
    ) -> (Governor, Vec<(usize, Quality)>, [usize; 4]) {
        run_on(Signal::SceneCost, start, frames, cost)
    }

    fn run_on(
        signal: Signal,
        start: Quality,
        frames: usize,
        mut cost: impl FnMut(Quality, usize) -> Duration,
    ) -> (Governor, Vec<(usize, Quality)>, [usize; 4]) {
        let mut g = Governor::new(start, signal);
        let mut changes = vec![];
        let mut at = [0; 4];
        for frame in 0..frames {
            let q = g.quality();
            at[q.index()] += 1;
            g = g.next(cost(q, frame), BUDGET);
            if g.quality() != q {
                changes.push((frame, g.quality()));
            }
        }
        (g, changes, at)
    }

    /// Per-level loads, as fractions of the budget, with ±10 % noise.
    fn gpu(loads: [f64; 4]) -> impl FnMut(Quality, usize) -> Duration {
        let mut seed = 7;
        move |q, _| of(loads[q.index()] * (1.0 + jitter(&mut seed, 0.1)))
    }

    #[test]
    fn a_gpu_that_cannot_afford_full_settles_below_it() {
        // Full is 20 % over the budget, HalfRes comfortably under.
        let frames = 300_000;
        let (g, changes, at) = run(Quality::Full, frames, gpu([1.2, 0.45, 0.3, 0.1]));
        assert_eq!(g.quality(), Quality::HalfRes);
        // Retries of Full thin out: the back-off doubles up to its cap, then
        // one retry per capped hold. Each is two changes.
        let capped = frames / (HOLD_FRAMES << MAX_BACKOFF) as usize;
        let bound = 1 + 2 * (MAX_BACKOFF as usize + capped + 1);
        assert!(
            changes.len() <= bound,
            "{} changes: {changes:?}",
            changes.len()
        );
        // Never below HalfRes, and nearly all the time at it.
        assert!(
            changes.iter().all(|&(_, q)| q <= Quality::HalfRes),
            "{changes:?}"
        );
        assert!(at[Quality::HalfRes.index()] * 100 >= frames * 98, "{at:?}");
    }

    #[test]
    fn a_gpu_that_affords_only_flat_goes_there_and_stays() {
        let (g, changes, at) = run(Quality::Full, 200_000, gpu([3.0, 2.0, 1.1, 0.2]));
        assert_eq!(g.quality(), Quality::Flat);
        assert!(at[Quality::Flat.index()] * 100 >= 200_000 * 97, "{at:?}");
        assert!(changes.len() <= 40, "{} changes", changes.len());
    }

    #[test]
    fn steady_costs_never_change_the_level() {
        // Under the down line, above the up line: noise included, no change.
        for start in [Quality::Full, Quality::HalfRes, Quality::Cached] {
            let (g, changes, _) = run(start, 100_000, gpu([0.75; 4]));
            assert_eq!((g.quality(), changes.len()), (start, 0));
        }
        // Calm at Full: nothing to change either.
        let (_, changes, _) = run(Quality::Full, 100_000, gpu([0.3; 4]));
        assert!(changes.is_empty());
    }

    #[test]
    fn a_short_spike_changes_nothing() {
        // 5 frames at 10 budgets (a hitch) in a calm run.
        let (g, changes, _) = run(Quality::Full, 10_000, |_, f| {
            if (5_000..5_005).contains(&f) {
                of(10.0)
            } else {
                of(0.3)
            }
        });
        assert_eq!(g.quality(), Quality::Full);
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn a_transient_overload_is_not_a_permanent_downgrade() {
        // A second of real overload (say a heavy page animating), then calm.
        let (g, changes, _) = run(Quality::Full, 20_000, |_, f| {
            if (5_000..5_300).contains(&f) {
                of(1.5)
            } else {
                of(0.3)
            }
        });
        assert_eq!(g.quality(), Quality::Full);
        assert_eq!(g.backoff, 0);
        // The overload is the same at every level, so it goes down to Flat
        // during it and back up one level per hold after it.
        assert!(changes.len() <= 6, "{changes:?}");
        assert_eq!(changes.last().map(|c| c.1), Some(Quality::Full));
    }

    #[test]
    fn a_cheap_scene_climbs_to_full_one_hold_per_level() {
        let mut g = Governor::new(Quality::Flat, Signal::SceneCost);
        for expected in [Quality::Cached, Quality::HalfRes, Quality::Full] {
            g = feed(g, [of(0.2); HOLD_FRAMES as usize - 1]);
            assert_ne!(g.quality(), expected);
            g = feed(g, [of(0.2)]);
            assert_eq!(g.quality(), expected);
        }
    }

    #[test]
    fn a_level_measured_not_to_fit_waits_out_the_back_off() {
        let mut g = Governor::new(Quality::Full, Signal::SceneCost);
        // Full fails, HalfRes is calm, and Full is retried and fails again:
        // each failed retry doubles the wait.
        let mut waits = vec![];
        for _ in 0..4 {
            g = feed(g, [of(1.3); SETTLE_FRAMES as usize]);
            assert_eq!(g.quality(), Quality::HalfRes);
            let mut waited = 0;
            while g.quality() == Quality::HalfRes {
                g = g.next(of(0.3), BUDGET);
                waited += 1;
            }
            waits.push(waited);
        }
        assert_eq!(
            waits,
            [1, 2, 4, 8].map(|n| n * HOLD_FRAMES),
            "the first retry follows the first failure, which was not a probe"
        );
    }

    #[test]
    fn surviving_a_probe_forgets_the_back_off() {
        let mut g = Governor::new(Quality::Full, Signal::SceneCost);
        for _ in 0..3 {
            g = feed(g, [of(1.3); SETTLE_FRAMES as usize]);
            while g.quality() == Quality::HalfRes {
                g = g.next(of(0.3), BUDGET);
            }
        }
        assert_eq!(g.backoff, 2);
        g = feed(g, [of(0.3); PROBE_FRAMES as usize]);
        assert_eq!((g.quality(), g.backoff), (Quality::Full, 0));
    }

    #[test]
    fn without_timestamps_a_frame_late_at_every_level_settles_at_full() {
        // The whole frame is half a budget late whatever the scene does (the
        // UI thread blocked elsewhere): stepping down never helps.
        let frames = 300_000;
        let (g, changes, at) = run_on(
            Signal::Lateness,
            Quality::Full,
            frames,
            gpu([0.5 / DOWN_FRACTION + 0.4; 4]),
        );
        assert_eq!(g.quality(), Quality::Full);
        // Each trial goes one level down and comes back: never further.
        assert!(
            changes.iter().all(|&(_, q)| q <= Quality::HalfRes),
            "{changes:?}"
        );
        // Trials thin out like upgrades do: the hold doubles up to its cap.
        let capped = frames / (HOLD_FRAMES << MAX_BACKOFF) as usize;
        assert!(
            changes.len() <= 2 * (MAX_BACKOFF as usize + capped + 1),
            "{} changes: {changes:?}",
            changes.len()
        );
        assert!(at[Quality::Full.index()] * 1000 >= frames * 995, "{at:?}");
    }

    #[test]
    fn without_timestamps_a_scene_that_is_the_cause_still_steps_down() {
        // Late at Full and at HalfRes, on time from Cached: two trials that
        // help, then it stays.
        let frames = 100_000;
        let (g, changes, at) = run_on(
            Signal::Lateness,
            Quality::Full,
            frames,
            gpu([1.6, 1.0, 0.1, 0.0]),
        );
        assert_eq!(g.quality(), Quality::Cached);
        assert!(at[Quality::Cached.index()] * 100 >= frames * 97, "{at:?}");
        assert_eq!(
            changes[..2],
            [(31, Quality::HalfRes), (63, Quality::Cached)]
        );
    }

    #[test]
    fn without_timestamps_a_step_that_did_not_help_is_undone_and_held() {
        let mut g = Governor::new(Quality::Full, Signal::Lateness);
        g = feed(g, [of(1.5); SETTLE_FRAMES as usize]);
        assert_eq!(g.quality(), Quality::HalfRes);
        // As late at HalfRes: back to Full once HalfRes has settled.
        g = feed(g, [of(1.5); SETTLE_FRAMES as usize]);
        assert_eq!((g.quality(), g.backoff), (Quality::Full, 1));
        // Held at Full, however late, for one hold.
        g = feed(g, [of(1.5); HOLD_FRAMES as usize]);
        assert_eq!(g.quality(), Quality::Full);
        g = feed(g, [of(1.5)]);
        assert_eq!(g.quality(), Quality::HalfRes);
    }

    #[test]
    fn with_timestamps_steps_down_are_not_trials() {
        // The scene's own cost: a level whose cost is the same keeps going
        // down, as before (the Linux behaviour).
        let (g, _, _) = run(Quality::Full, 2_000, gpu([1.5; 4]));
        assert_eq!(g.quality(), Quality::Flat);
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
