//! `--measure memory`: wubble's process tree in scripted states, each process's RSS, PSS and
//! USS (Linux), footprint (macOS) and GPU memory (NVIDIA), as a table and CSV. Hand run only:
//! the numbers are printed, never asserted.
//!
//! States ([`COUNTS`]): an empty window, then 1, 10 and 50 tabs of one static article, each
//! tab on its own site (`t<n>.localhost`, served on loopback), so Chromium may give each its
//! own renderer as it would real sites; then the sleep policy at its defaults: one lifecycle
//! tick past `freeze_after` (every hidden tab frozen), one past `discard_after` (every hidden
//! tab discarded). The ticks carry a clock moved ahead, so nothing waits real minutes and the
//! settings stay as they are. Each state is sampled once it holds (tabs open and loaded, the
//! blocker idle, the policy's effects applied) and has held for [`SETTLE_TICKS`].
//!
//! [`next`] is pure: the host (`host/memory.rs`) hands it [`Seen`] and the time of the tick
//! and runs the actions, the sampling below reads the OS.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use wubble_core::memreport;
use wubble_core::sleep::SleepPolicy;

/// The tab-count states, sampled in order.
pub const COUNTS: [(&str, usize); 4] = [("empty", 0), ("one", 1), ("ten", 10), ("fifty", 50)];

/// Measure ticks ([`crate::measure::TICK`] each) a state must hold before it is sampled: about 3 s, for the
/// renderers' first garbage collections and Chromium's post-load tasks.
pub const SETTLE_TICKS: u32 = 190;

/// Measure ticks a state may take to be reached (about 2 minutes) before the run fails.
pub const STUCK_TICKS: u32 = 7_500;

/// The browser process's largest mappings listed per state (Linux).
const TOP_MAPPINGS: usize = 10;

/// How far past the policy's delay a lifecycle tick is placed.
const PAST: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// `COUNTS[i]`'s tabs.
    Tabs(usize),
    /// The last count's tabs, every hidden one frozen.
    Frozen,
    /// Every hidden one discarded.
    Discarded,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Tabs(i) => COUNTS[i].0,
            Self::Frozen => "fifty-frozen",
            Self::Discarded => "fifty-discarded",
        }
    }
}

/// Which delay of the sleep policy a lifecycle tick goes past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sleep {
    Freeze,
    Discard,
}

/// How far ahead of now the lifecycle tick for `sleep` is placed: just past the policy's delay
/// (every tab was hidden at or before now, so it is past for each).
pub fn ahead(policy: &SleepPolicy, sleep: Sleep) -> Duration {
    PAST + match sleep {
        Sleep::Freeze => policy.freeze_after,
        Sleep::Discard => policy.discard_after,
    }
}

/// What the host sees now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub tabs: usize,
    /// Tabs showing the article's title and not busy (loading done).
    pub loaded: usize,
    /// Tabs the model has frozen.
    pub frozen: usize,
    /// Views the engine holds.
    pub views: usize,
    /// The blocker compiles or updates its lists.
    pub blocker_busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open this many more tabs.
    Open(usize),
    /// Sample the state, reached `ready_after` after the phase began (the tabs opened and
    /// loaded, or the policy applied): wall time between the ticks, to compare settings by.
    Sample {
        state: &'static str,
        ready_after: Duration,
    },
    /// A lifecycle tick past the policy's delay.
    Tick(Sleep),
    Finish,
    Fail(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Driver {
    pub phase: Phase,
    /// Ticks the phase's state has held and the tick its hold began on, once it does.
    held: Option<(u32, Instant)>,
    /// Ticks since the phase began.
    ticks: u32,
    /// When the phase began: the tick that opened its tabs or ticked the policy (the first
    /// tick for the first phase).
    began: Option<Instant>,
    done: bool,
}

impl Default for Driver {
    fn default() -> Self {
        Self {
            phase: Phase::Tabs(0),
            held: None,
            ticks: 0,
            began: None,
            done: false,
        }
    }
}

fn holds(phase: Phase, seen: &Seen) -> bool {
    let last = COUNTS[COUNTS.len() - 1].1;
    !seen.blocker_busy
        && match phase {
            Phase::Tabs(i) => seen.tabs == COUNTS[i].1 && seen.loaded == COUNTS[i].1,
            // Every tab but the visible one.
            Phase::Frozen => seen.tabs == last && seen.frozen + 1 == last,
            Phase::Discarded => seen.tabs == last && seen.views == 1,
        }
}

/// One measure tick at `now`.
pub fn next(d: Driver, seen: &Seen, now: Instant) -> (Driver, Vec<Action>) {
    if d.done {
        return (d, vec![]);
    }
    let ticks = d.ticks + 1;
    if ticks > STUCK_TICKS {
        let why = format!(
            "stuck before {} after {STUCK_TICKS} ticks: {seen:?}",
            d.phase.name()
        );
        return (Driver { done: true, ..d }, vec![Action::Fail(why)]);
    }
    let began = d.began.unwrap_or(now);
    let held = holds(d.phase, seen).then(|| d.held.map_or((0, now), |(n, at)| (n + 1, at)));
    let Some((_, hold_began)) = held.filter(|&(n, _)| n >= SETTLE_TICKS) else {
        let began = Some(began);
        return (
            Driver {
                held,
                ticks,
                began,
                ..d
            },
            vec![],
        );
    };
    let sample = Action::Sample {
        state: d.phase.name(),
        ready_after: hold_began.saturating_duration_since(began),
    };
    let (phase, then) = match d.phase {
        Phase::Tabs(i) if i + 1 < COUNTS.len() => (
            Phase::Tabs(i + 1),
            Action::Open(COUNTS[i + 1].1 - COUNTS[i].1),
        ),
        Phase::Tabs(_) => (Phase::Frozen, Action::Tick(Sleep::Freeze)),
        Phase::Frozen => (Phase::Discarded, Action::Tick(Sleep::Discard)),
        Phase::Discarded => {
            return (Driver { done: true, ..d }, vec![sample, Action::Finish]);
        }
    };
    (
        Driver {
            phase,
            held: None,
            ticks: 0,
            began: Some(now),
            done: false,
        },
        vec![sample, then],
    )
}

// ----- the run: the only impure part (reads the OS)

struct Run {
    driver: Driver,
    /// The article's URL for tab `n`: `{}` is replaced by its host label.
    url: String,
    meter: wubble_sysmem::Meter,
    report: String,
    csv: String,
}

thread_local! {
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// Starts the run; `url` is the article's URL with `{}` where each tab's host label goes.
pub fn begin(url: String) {
    RUN.set(Some(Run {
        driver: Driver::default(),
        url,
        meter: wubble_sysmem::Meter::default(),
        report: String::new(),
        csv: memreport::CSV_HEADER.into(),
    }));
}

/// Whether a memory run is on.
pub fn active() -> bool {
    RUN.with_borrow(Option::is_some)
}

/// The URL of tab `n`: on a site of its own.
pub fn url(n: usize) -> Option<String> {
    RUN.with_borrow(|r| r.as_ref().map(|r| r.url.replace("{}", &format!("t{n}"))))
}

/// One step of the driver.
pub fn step(seen: &Seen, now: Instant) -> Vec<Action> {
    RUN.with_borrow_mut(|r| {
        let Some(r) = r.as_mut() else {
            return vec![];
        };
        let (driver, actions) = next(r.driver, seen, now);
        r.driver = driver;
        actions
    })
}

/// Samples the tree for `state` into the report.
/// `renderers` are the pids the tabs' frames report: on Linux a renderer's command line is the
/// zygote's it was forked from, so only this tells it apart, and a sandboxed renderer reports
/// its id in the sandbox's PID namespace.
/// `window` is the window's physical size and scale, `wgpu` the GPU allocator's bytes.
pub fn sample(
    state: &str,
    ready_after: Duration,
    seen: &Seen,
    renderers: &BTreeSet<u32>,
    window: &str,
    wgpu: Option<(u64, u64)>,
) {
    RUN.with_borrow_mut(|r| {
        let Some(r) = r.as_mut() else {
            return;
        };
        let mut procs = wubble_sysmem::memory(&r.meter.sample());
        // Linux forks the GPU process and the renderers from the zygote: their command lines
        // say Zygote.
        let gpu = r.meter.gpu_pid();
        for p in &mut procs {
            if Some(p.pid) == gpu {
                p.label = "GPU".into();
            } else if renderers.contains(&p.pid)
                || (p.label == "Zygote"
                    && wubble_sysmem::namespace_pid(p.pid).is_some_and(|n| renderers.contains(&n)))
            {
                p.label = "Renderer".into();
            }
        }
        let _ = writeln!(
            r.report,
            "\ntabs={} views={} frozen={} window={window} ready_after_ms={}",
            seen.tabs,
            seen.views,
            seen.frozen,
            ready_after.as_millis()
        );
        r.report.push_str(&memreport::table(state, &procs));
        if let Some(b) = wubble_sysmem::rollup(std::process::id()) {
            let opt = |v: Option<u64>| v.map_or_else(|| "-".into(), memreport::mib);
            let _ = writeln!(
                r.report,
                "browser process: pss anon {} MiB, file {} MiB",
                opt(b.pss_anon),
                opt(b.pss_file)
            );
        }
        if let Ok(smaps) = std::fs::read_to_string("/proc/self/smaps") {
            for (name, bytes) in memreport::smaps_by_mapping(&smaps, TOP_MAPPINGS) {
                let _ = writeln!(r.report, "  {} MiB {name}", memreport::mib(bytes));
            }
        }
        // macOS: the browser process's footprint by region type (IOSurface, IOAccelerator,
        // MALLOC, ...), from the system's `vmmap`.
        if cfg!(target_os = "macos")
            && let Ok(out) = std::process::Command::new("vmmap")
                .args(["--summary", &std::process::id().to_string()])
                .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            let table = text
                .lines()
                .skip_while(|l| !l.starts_with("REGION TYPE"))
                .take_while(|l| !l.starts_with("TOTAL"));
            for line in table {
                let _ = writeln!(r.report, "  {line}");
            }
        }
        if let Some((allocated, reserved)) = wgpu {
            let _ = writeln!(
                r.report,
                "wgpu allocator: allocated {} MiB, reserved {} MiB",
                memreport::mib(allocated),
                memreport::mib(reserved)
            );
        }
        // the frame rings alive and Skia's caches: a cache stays level over open-close runs, a
        // leak climbs
        let _ = writeln!(
            r.report,
            "gpu_targets {}, skia cache {} MiB",
            wubble_gfx::import::live_targets(),
            memreport::mib(i_slint_renderer_skia::cache::bytes() as u64)
        );
        if let Some(line) = crate::alloc_count::line() {
            let _ = writeln!(r.report, "{line}");
        }
        r.csv.push_str(&memreport::csv_rows(state, &procs));
        eprintln!("measure: memory {state} sampled");
    });
}

/// Ends the run: the report and the CSV, `None` if none ran or nothing was sampled.
pub fn finish() -> Option<(String, String)> {
    let r = RUN.take()?;
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    (!r.report.is_empty()).then(|| {
        (
            format!("measure: scenario=memory build={build}{}", r.report),
            r.csv,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(tabs: usize, loaded: usize) -> Seen {
        Seen {
            tabs,
            loaded,
            views: tabs,
            ..Seen::default()
        }
    }

    fn sample(state: &'static str) -> Action {
        Action::Sample {
            state,
            ready_after: Duration::ZERO,
        }
    }

    /// One instant for every tick of a test that does not look at the time.
    fn t0() -> Instant {
        static T0: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
        *T0
    }

    /// Runs the driver for `n` ticks against `seen`, all at [`t0`], collecting the actions.
    fn run(mut d: Driver, seen: &Seen, n: u32) -> (Driver, Vec<Action>) {
        let now = t0();
        let mut all = vec![];
        for _ in 0..n {
            let (next_d, actions) = next(d, seen, now);
            d = next_d;
            all.extend(actions);
        }
        (d, all)
    }

    #[test]
    fn sleep_ticks_land_past_the_default_policy() {
        let policy = SleepPolicy::default();
        assert!(ahead(&policy, Sleep::Freeze) > policy.freeze_after);
        assert!(ahead(&policy, Sleep::Discard) > policy.discard_after);
        // The freeze tick does not reach the discard delay.
        assert!(ahead(&policy, Sleep::Freeze) < policy.discard_after);
    }

    #[test]
    fn each_state_is_sampled_after_it_held_then_the_next_begins() {
        let d = Driver::default();
        // Not yet held long enough.
        let (d, a) = run(d, &seen(0, 0), SETTLE_TICKS);
        assert!(a.is_empty());
        let (d, a) = next(d, &seen(0, 0), t0());
        assert_eq!(a, vec![sample("empty"), Action::Open(1)]);
        // A tab still loading does not count; the hold starts over when it breaks.
        let (d, a) = run(d, &seen(1, 0), SETTLE_TICKS * 2);
        assert!(a.is_empty());
        let (d, _) = run(d, &seen(1, 1), SETTLE_TICKS - 1);
        let busy = Seen {
            blocker_busy: true,
            ..seen(1, 1)
        };
        let (d, a) = run(d, &busy, 3);
        assert!(a.is_empty());
        let (d, a) = run(d, &seen(1, 1), SETTLE_TICKS + 1);
        assert_eq!(a, vec![sample("one"), Action::Open(9)]);
        let (d, a) = run(d, &seen(10, 10), SETTLE_TICKS + 1);
        assert_eq!(a, vec![sample("ten"), Action::Open(40)]);
        let (d, a) = run(d, &seen(50, 50), SETTLE_TICKS + 1);
        assert_eq!(a, vec![sample("fifty"), Action::Tick(Sleep::Freeze)]);
        let frozen = Seen {
            frozen: 49,
            ..seen(50, 50)
        };
        let (d, a) = run(d, &frozen, SETTLE_TICKS + 1);
        assert_eq!(
            a,
            vec![sample("fifty-frozen"), Action::Tick(Sleep::Discard)]
        );
        let discarded = Seen {
            views: 1,
            ..seen(50, 50)
        };
        let (d, a) = run(d, &discarded, SETTLE_TICKS + 1);
        assert_eq!(a, vec![sample("fifty-discarded"), Action::Finish]);
        assert_eq!(run(d, &discarded, 5).1, vec![]);
    }

    /// The time to ready is the clock's, from the tick that began the phase to the one its
    /// last unbroken hold began on, however few ticks fell in between (a blocked main thread
    /// fires the timer late and drops the ticks it missed).
    #[test]
    fn ready_after_is_wall_time_to_the_last_hold() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        // Empty: the first tick begins the phase, the state holds from it on.
        let mut d = Driver::default();
        let mut a = vec![];
        for i in 0..=u64::from(SETTLE_TICKS) {
            let (n, act) = next(d, &seen(0, 0), at(i));
            d = n;
            a.extend(act);
        }
        let empty = Action::Sample {
            state: "empty",
            ready_after: Duration::ZERO,
        };
        assert_eq!(a, vec![empty, Action::Open(1)]);
        // The phase began at the opening tick (SETTLE ms); one late tick 900 ms on still
        // loading, a hold broken by the blocker, then the hold that lasts from 1500 ms on.
        let began = u64::from(SETTLE_TICKS);
        let (d, a) = next(d, &seen(1, 0), at(began + 900));
        assert!(a.is_empty());
        let (d, _) = next(d, &seen(1, 1), at(began + 1000));
        let busy = Seen {
            blocker_busy: true,
            ..seen(1, 1)
        };
        let (mut d, _) = next(d, &busy, at(began + 1200));
        let mut a = vec![];
        for i in 0..=u64::from(SETTLE_TICKS) {
            let (n, act) = next(d, &seen(1, 1), at(began + 1500 + i));
            d = n;
            a.extend(act);
        }
        let one = Action::Sample {
            state: "one",
            ready_after: Duration::from_millis(1500),
        };
        assert_eq!(a, vec![one, Action::Open(9)]);
    }

    #[test]
    fn a_state_never_reached_fails_once() {
        let (_, a) = run(Driver::default(), &seen(3, 0), STUCK_TICKS + 10);
        assert_eq!(a.len(), 1);
        assert!(matches!(&a[0], Action::Fail(why) if why.contains("empty")));
    }
}
