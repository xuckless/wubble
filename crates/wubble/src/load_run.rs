//! `--measure load`: the shell loads the loopback pages ([`crate::load_server`]) one after the
//! other and records, per load, what the page says about itself (navigation start to first paint,
//! first contentful paint, DOMContentLoaded and load end, from `assets/probe.js`) and the time
//! from the load request to the first frame shown.
//!
//! The order is a pure machine ([`next`], bounded by ticks of the measure timer, no clock): a
//! warm-up page, then each page cold (its assets not in the HTTP cache) and warm (the same
//! assets from it, under another document URL). A cold sample needs an empty cache, which is a
//! fresh profile, which is a fresh process: `scripts/measure-load.sh` runs the process per cold
//! sample. The shell half below reads the clock and the server; the numbers are printed, never
//! asserted.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use wubble_engine::nav::WebEvent;

use crate::load_server::{LoadServer, Timing};
use crate::measure::summary;

/// Timer ticks (16 ms each) a step waits for its timing before the run gives up.
pub const MAX_TICKS: u32 = 3_000;

/// What the engine is given to hook requests (`--load-variant`), to price the hooks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Variant {
    /// As shipped: the request filter and the response observer, the blocker on.
    #[default]
    Blocker,
    /// Both hooks, the blocker switched off: the hooks' fixed cost.
    BlockerOff,
    /// Neither hook: welding installs no resource-request handler at all.
    NoHook,
    /// The request filter only.
    NoObserver,
    /// The response observer only.
    ObserverOnly,
}

impl Variant {
    /// The name `--load-variant` takes (the clap value, so the CSV column matches the argument).
    pub fn name(self) -> String {
        clap::ValueEnum::to_possible_value(&self)
            .map(|v| v.get_name().to_owned())
            .unwrap_or_default()
    }

    /// Which of the engine's two hooks are installed: (request filter, response observer).
    pub fn hooks(self) -> (bool, bool) {
        match self {
            Self::Blocker | Self::BlockerOff => (true, true),
            Self::NoHook => (false, false),
            Self::NoObserver => (true, false),
            Self::ObserverOnly => (false, true),
        }
    }

    /// Whether the blocker's lists are applied.
    pub fn blocks(self) -> bool {
        matches!(self, Self::Blocker | Self::NoObserver)
    }
}

/// What a load measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temp {
    /// Loaded once to start the renderer and the first connection; not reported.
    Warmup,
    Cold,
    Warm,
    /// Another page to leave the last one, so that going back has somewhere to come from; not
    /// reported.
    Away,
    /// History back to the page before: from the back/forward cache if it keeps the page, else
    /// from the HTTP cache.
    Back,
}

impl Temp {
    pub fn name(self) -> &'static str {
        match self {
            Self::Warmup => "warmup",
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Away => "away",
            Self::Back => "back",
        }
    }

    /// Whether the load goes into the report.
    pub fn reported(self) -> bool {
        !matches!(self, Self::Warmup | Self::Away)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub page: String,
    pub temp: Temp,
}

impl Step {
    /// The path loaded: a warm load is another document URL, so it is not the same-URL reload.
    pub fn path(&self, warm_run: usize) -> String {
        match self.temp {
            Temp::Warm => format!("{}?w={warm_run}", self.page),
            Temp::Away => format!("{}?away", self.page),
            _ => self.page.clone(),
        }
    }
}

/// The warm-up, then each page once cold and `warm` times warm; with `back`, the last page is
/// left for another and returned to by history.
pub fn plan(pages: &[String], warm: usize, back: bool) -> Vec<Step> {
    let step = |page: &str, temp| Step {
        page: page.to_owned(),
        temp,
    };
    std::iter::once(step("/blank", Temp::Warmup))
        .chain(pages.iter().flat_map(|p| {
            std::iter::once(step(p, Temp::Cold)).chain((0..warm).map(move |_| step(p, Temp::Warm)))
        }))
        .chain(
            pages
                .last()
                .filter(|_| back)
                .into_iter()
                .flat_map(|p| [step("/blank", Temp::Away), step(p, Temp::Back)]),
        )
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Until the first frame is up and the blocker has its lists.
    Waiting {
        ticks: u32,
    },
    Loading {
        step: usize,
        ticks: u32,
        /// The new document committed (its URL changed).
        committed: bool,
        /// A CEF frame was taken after the commit; the next render shows it.
        taken: bool,
        /// A Slint render followed that frame.
        framed: bool,
        /// The page reported its timings.
        timed: bool,
    },
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// The measure timer; `ready`: first frame taken and the blocker loaded.
    Tick {
        ready: bool,
    },
    Committed,
    Taken,
    Rendered,
    Timing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Load(usize),
    /// The first render of the first frame after the commit: stamp it.
    Framed,
    /// Step `.0` has its timings and its frame: file the sample.
    Record(usize),
    Quit,
    Fail(String),
}

fn loading(step: usize) -> State {
    State::Loading {
        step,
        ticks: 0,
        committed: false,
        taken: false,
        framed: false,
        timed: false,
    }
}

/// `steps` is the plan's length.
pub fn next(state: State, input: Input, steps: usize) -> (State, Vec<Action>) {
    use State::*;
    match (state, input) {
        (Waiting { ticks }, Input::Tick { ready }) => {
            if ready {
                (loading(0), vec![Action::Load(0)])
            } else if ticks >= MAX_TICKS {
                (
                    Done,
                    vec![Action::Fail(
                        "no first frame or no blocker lists in time".into(),
                    )],
                )
            } else {
                (Waiting { ticks: ticks + 1 }, vec![])
            }
        }
        (
            Loading {
                step,
                ticks,
                committed,
                taken,
                framed,
                timed,
            },
            input,
        ) => {
            let mut s = (committed, taken, framed, timed, ticks);
            let mut actions = vec![];
            match input {
                Input::Tick { .. } => s.4 += 1,
                Input::Committed => s.0 = true,
                Input::Taken if s.0 => s.1 = true,
                Input::Rendered if s.1 && !s.2 => {
                    s.2 = true;
                    actions.push(Action::Framed);
                }
                Input::Timing => s.3 = true,
                _ => {}
            }
            if s.4 > MAX_TICKS {
                return (
                    Done,
                    vec![Action::Fail(format!(
                        "step {step}: no timing after {MAX_TICKS} ticks"
                    ))],
                );
            }
            if s.2 && s.3 {
                actions.push(Action::Record(step));
                return if step + 1 < steps {
                    actions.push(Action::Load(step + 1));
                    (loading(step + 1), actions)
                } else {
                    actions.push(Action::Quit);
                    (Done, actions)
                };
            }
            (
                Loading {
                    step,
                    ticks: s.4,
                    committed: s.0,
                    taken: s.1,
                    framed: s.2,
                    timed: s.3,
                },
                actions,
            )
        }
        (state, _) => (state, vec![]),
    }
}

// ----- the report: pure

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub page: String,
    pub temp: Temp,
    pub timing: Timing,
    /// From the load request to the first Slint render of the first frame after the commit.
    pub host_ms: Option<f64>,
    /// Requests the server saw from the load request to the timing report, the document's own
    /// included (the blockable page asks for 200 more: the rest were blocked).
    pub served: usize,
    /// Those requests, kept for pages that are not the standard set.
    pub log: Vec<String>,
}

pub const CSV_HEADER: &str =
    "browser,variant,page,temp,run,fp,fcp,dcl,load,build,s0,host_first_frame,served,persisted";

fn cell(v: Option<f64>) -> String {
    v.map_or_else(String::new, |v| format!("{v:.1}"))
}

/// One row per sample; `variant` names the build of the run (`blocker`, `no-hook`, ...).
pub fn csv(variant: &str, samples: &[Sample]) -> String {
    let mut out = String::new();
    let mut seen: Vec<(&str, Temp, usize)> = vec![];
    for s in samples.iter().filter(|s| s.temp.reported()) {
        let run = match seen
            .iter_mut()
            .find(|(p, t, _)| *p == s.page && *t == s.temp)
        {
            Some((_, _, n)) => {
                *n += 1;
                *n
            }
            None => {
                seen.push((&s.page, s.temp, 0));
                0
            }
        };
        let t = &s.timing;
        let _ = writeln!(
            out,
            "wubble,{variant},{},{},{run},{},{},{},{},{},{},{},{},{}",
            s.page,
            s.temp.name(),
            cell(t.fp),
            cell(t.fcp),
            cell(t.dcl),
            cell(t.load),
            cell(t.build),
            cell(t.s0),
            cell(s.host_ms),
            s.served,
            t.persisted.map_or("", |p| if p { "1" } else { "0" })
        );
    }
    out
}

/// Median and 95th percentile (nearest rank) of the values that are there, as `p50/p95`.
fn spread(values: impl Iterator<Item = Option<f64>>) -> String {
    let values: Vec<f64> = values.flatten().collect();
    summary(&values).map_or_else(|| "-".into(), |s| format!("{:.1}/{:.1}", s.p50, s.p95))
}

/// A table per page and temperature: samples, then p50/p95 of each metric in ms.
pub fn table(samples: &[Sample]) -> String {
    let mut out = format!(
        "{:<12} {:<5} {:>3} {:>13} {:>13} {:>13} {:>13} {:>13} {:>5}\n",
        "page",
        "temp",
        "n",
        "first paint",
        "first content",
        "DOMContent",
        "load end",
        "host frame",
        "reqs"
    );
    let mut keys: Vec<(&str, Temp)> = vec![];
    for s in samples.iter().filter(|s| s.temp.reported()) {
        if !keys.contains(&(s.page.as_str(), s.temp)) {
            keys.push((&s.page, s.temp));
        }
    }
    for (page, temp) in keys {
        let rows: Vec<&Sample> = samples
            .iter()
            .filter(|s| s.page == page && s.temp == temp)
            .collect();
        let _ = writeln!(
            out,
            "{page:<12} {:<5} {:>3} {:>13} {:>13} {:>13} {:>13} {:>13} {:>5}",
            temp.name(),
            rows.len(),
            spread(rows.iter().map(|s| s.timing.fp)),
            spread(rows.iter().map(|s| s.timing.fcp)),
            spread(rows.iter().map(|s| s.timing.dcl)),
            spread(rows.iter().map(|s| s.timing.load)),
            spread(rows.iter().map(|s| s.host_ms)),
            rows.iter().map(|s| s.served).max().unwrap_or(0),
        );
    }
    out
}

// ----- the shell half: the only part that reads a clock

/// The blocker has its lists (`measure::mark` calls this): the run waits for it.
static BLOCKER_READY: AtomicBool = AtomicBool::new(false);

pub fn blocker_ready() {
    BLOCKER_READY.store(true, Ordering::Relaxed);
}

struct Run {
    server: Arc<LoadServer>,
    variant: String,
    steps: Vec<Step>,
    state: State,
    /// A frame was taken at least once (the shell is up).
    first_frame: bool,
    /// Warm loads of the current page so far.
    warm_run: usize,
    requested: Option<Instant>,
    framed: Option<Instant>,
    requests_before: usize,
    timing: Option<Timing>,
    samples: Vec<Sample>,
    failure: Option<String>,
    /// Actions the hooks' inputs led to, run by the next tick.
    pending: Vec<Action>,
}

thread_local! {
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
    /// Why the finished run failed, until [`take_failure`].
    static FAILURE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The failure of the run [`finish`] ended, if it had one.
pub fn take_failure() -> Option<String> {
    FAILURE.take()
}

fn with_run<T>(f: impl FnOnce(&mut Run) -> T) -> Option<T> {
    RUN.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Starts the run: `pages` once cold and `warm` times warm each.
pub fn begin(server: Arc<LoadServer>, variant: &str, pages: &[String], warm: usize, back: bool) {
    crate::blocking::time_decisions();
    RUN.set(Some(Run {
        server,
        variant: variant.into(),
        steps: plan(pages, warm, back),
        state: State::Waiting { ticks: 0 },
        first_frame: false,
        warm_run: 0,
        requested: None,
        framed: None,
        requests_before: 0,
        timing: None,
        samples: vec![],
        failure: None,
        pending: vec![],
    }));
}

/// Whether a load run is on (the measure timer hands its tick over).
pub fn active() -> bool {
    with_run(|_| ()).is_some()
}

impl Run {
    fn feed(&mut self, input: Input) -> Vec<Action> {
        let state = std::mem::replace(&mut self.state, State::Done);
        let (state, actions) = next(state, input, self.steps.len());
        self.state = state;
        actions
    }

    /// An input from a hook: the frame is stamped now, the rest waits for the next tick.
    fn hook(&mut self, input: Input) {
        for action in self.feed(input) {
            if action == Action::Framed {
                self.framed = Some(Instant::now());
            } else {
                self.pending.push(action);
            }
        }
    }

    /// The step the machine is on.
    fn step(&self) -> Option<&Step> {
        match &self.state {
            State::Loading { step, .. } => self.steps.get(*step),
            _ => None,
        }
    }
}

/// Measure timer: reads the server's reports, then lets the machine act. Loads go through the
/// host like a typed address.
pub fn tick(host: &mut crate::host::Host) {
    let Some(actions) = with_run(|run| {
        let mut actions = std::mem::take(&mut run.pending);
        if let Some(t) = run.server.take_timing()
            && run.step().is_some_and(|s| s.page == t.page)
        {
            run.timing = Some(t);
            actions.extend(run.feed(Input::Timing));
        }
        let ready = run.first_frame && BLOCKER_READY.load(Ordering::Relaxed);
        actions.extend(run.feed(Input::Tick { ready }));
        actions
    }) else {
        return;
    };
    for action in actions {
        match action {
            Action::Load(i) => {
                let Some((url, host_tab)) = with_run(|run| {
                    let step = run.steps[i].clone();
                    if step.temp == Temp::Warm {
                        run.warm_run += 1;
                    } else {
                        run.warm_run = 0;
                    }
                    run.framed = None;
                    run.timing = None;
                    run.requests_before = run.server.requests();
                    Some(run.server.url(&step.path(run.warm_run)))
                })
                .flatten()
                .zip(host.shown()) else {
                    continue;
                };
                with_run(|run| run.requested = Some(Instant::now()));
                if with_run(|run| run.steps[i].temp == Temp::Back) == Some(true) {
                    host.perform(wubble_core::action::Action::Back);
                } else {
                    host.dispatch(wubble_core::model::Command::Navigate { tab: host_tab, url });
                }
            }
            // Stamped by the hook that fed the render.
            Action::Framed => {}
            Action::Record(i) => {
                with_run(|run| {
                    let step = run.steps[i].clone();
                    let Some(timing) = run.timing.take() else {
                        return;
                    };
                    let log = run.server.log_since(run.requests_before);
                    let standard = crate::load_server::PAGES.contains(&step.page.as_str())
                        || step.page == "/blank";
                    run.samples.push(Sample {
                        page: step.page,
                        temp: step.temp,
                        timing,
                        host_ms: run
                            .requested
                            .zip(run.framed)
                            .map(|(a, b)| b.saturating_duration_since(a).as_secs_f64() * 1e3),
                        served: log.len(),
                        log: if standard { vec![] } else { log },
                    });
                });
            }
            Action::Quit => crate::quit(),
            Action::Fail(reason) => {
                with_run(|run| run.failure = Some(reason));
                crate::quit();
            }
        }
    }
}

/// Window events: a commit starts a page's frame count.
pub fn web_event(event: &WebEvent) {
    if matches!(event, WebEvent::UrlChanged(_)) {
        with_run(|run| run.hook(Input::Committed));
    }
}

/// A CEF frame was taken for the shown tab.
pub fn frame_taken() {
    with_run(|run| {
        run.first_frame = true;
        run.hook(Input::Taken);
    });
}

/// Slint finished a render.
pub fn rendered() {
    with_run(|run| run.hook(Input::Rendered));
}

/// The run's report and CSV; `None` when none was started.
pub fn finish() -> Option<(String, String)> {
    let run = RUN.take()?;
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_else(|_| "n/a".into());
    let mut report = format!(
        "measure: scenario=load variant={} loadavg {}\n",
        run.variant,
        load.trim()
    );
    if let Some(reason) = &run.failure {
        let _ = writeln!(report, "measure: FAILED: {reason}");
        FAILURE.set(Some(reason.clone()));
    }
    report.push_str(&table(&run.samples));
    for s in run.samples.iter().filter(|s| !s.log.is_empty()) {
        let _ = writeln!(report, "requests for {} ({}):", s.page, s.temp.name());
        for line in &s.log {
            let _ = writeln!(report, "  {line}");
        }
    }
    let (us, calls, stopped) = crate::blocking::decision_cost();
    let _ = writeln!(
        report,
        "blocker decisions: {calls} ({stopped} not allowed), mean {us:.2} us each (inside the request filter, IO thread)"
    );
    let _ = writeln!(
        report,
        "connections accepted by the server: {}",
        run.server.connections()
    );
    let csv = format!("{CSV_HEADER}\n{}", csv(&run.variant, &run.samples));
    Some((report, csv))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> Vec<String> {
        vec!["/a".into(), "/b".into()]
    }

    #[test]
    fn variants_install_the_hooks_they_name() {
        for (v, hooks, blocks) in [
            (Variant::Blocker, (true, true), true),
            (Variant::BlockerOff, (true, true), false),
            (Variant::NoHook, (false, false), false),
            (Variant::NoObserver, (true, false), true),
            (Variant::ObserverOnly, (false, true), false),
        ] {
            assert_eq!((v.hooks(), v.blocks()), (hooks, blocks), "{}", v.name());
        }
    }

    #[test]
    fn the_plan_is_a_warmup_then_cold_and_warm_per_page() {
        let steps = plan(&pages(), 2, false);
        let seen: Vec<(String, &str)> = steps
            .iter()
            .map(|s| (s.page.clone(), s.temp.name()))
            .collect();
        assert_eq!(
            seen,
            [
                ("/blank".to_owned(), "warmup"),
                ("/a".into(), "cold"),
                ("/a".into(), "warm"),
                ("/a".into(), "warm"),
                ("/b".into(), "cold"),
                ("/b".into(), "warm"),
                ("/b".into(), "warm"),
            ]
        );
        assert_eq!(steps[1].path(0), "/a");
        assert_eq!(steps[2].path(1), "/a?w=1");
    }

    #[test]
    fn going_back_leaves_the_last_page_and_returns() {
        let steps = plan(&pages(), 0, true);
        let tail: Vec<(&str, &str)> = steps[3..]
            .iter()
            .map(|s| (s.page.as_str(), s.temp.name()))
            .collect();
        assert_eq!(tail, [("/blank", "away"), ("/b", "back")]);
        assert_eq!(steps[3].path(0), "/blank?away");
        assert!(!Temp::Away.reported() && Temp::Back.reported());
    }

    fn feed(mut state: State, inputs: &[Input], steps: usize) -> (State, Vec<Action>) {
        let mut all = vec![];
        for &i in inputs {
            let (s, a) = next(state, i, steps);
            state = s;
            all.extend(a);
        }
        (state, all)
    }

    const TICK: Input = Input::Tick { ready: true };

    #[test]
    fn waits_until_ready_then_loads_the_first_step() {
        let waiting = State::Waiting { ticks: 0 };
        let (s, a) = feed(waiting, &[Input::Tick { ready: false }; 3], 2);
        assert_eq!((s.clone(), a), (State::Waiting { ticks: 3 }, vec![]));
        let (s, a) = next(s, TICK, 2);
        assert_eq!(a, [Action::Load(0)]);
        assert_eq!(s, loading(0));
    }

    #[test]
    fn a_step_ends_on_its_timing_and_its_frame_in_either_order() {
        let order = [
            [
                Input::Committed,
                Input::Taken,
                Input::Rendered,
                Input::Timing,
            ],
            [
                Input::Timing,
                Input::Committed,
                Input::Taken,
                Input::Rendered,
            ],
        ];
        for inputs in order {
            let (s, a) = feed(loading(0), &inputs, 2);
            assert_eq!(a, [Action::Framed, Action::Record(0), Action::Load(1)]);
            assert_eq!(s, loading(1));
        }
    }

    #[test]
    fn frames_before_the_commit_are_the_old_page() {
        let (s, a) = feed(
            loading(0),
            &[Input::Taken, Input::Rendered, Input::Timing],
            2,
        );
        assert_eq!(a, []);
        assert!(matches!(
            s,
            State::Loading {
                committed: false,
                taken: false,
                framed: false,
                timed: true,
                ..
            }
        ));
        // A render before the post-commit frame is taken does not count either.
        let (_, a) = feed(loading(0), &[Input::Committed, Input::Rendered], 2);
        assert_eq!(a, []);
    }

    #[test]
    fn the_last_step_quits() {
        let (s, a) = feed(
            loading(1),
            &[
                Input::Committed,
                Input::Taken,
                Input::Rendered,
                Input::Timing,
            ],
            2,
        );
        assert_eq!(a, [Action::Framed, Action::Record(1), Action::Quit]);
        assert_eq!(s, State::Done);
        assert_eq!(next(State::Done, TICK, 2), (State::Done, vec![]));
    }

    #[test]
    fn a_failed_run_hands_its_reason_to_the_exit_code() {
        let server = LoadServer::start(0, false).expect("bind");
        begin(server, "blocker", &["article".into()], 1, false);
        assert!(take_failure().is_none());
        with_run(|run| run.failure = Some("no timing".into()));
        let (report, _) = finish().expect("a run was started");
        assert!(report.contains("measure: FAILED: no timing"));
        assert_eq!(take_failure().as_deref(), Some("no timing"));
        assert!(take_failure().is_none());
    }

    #[test]
    fn a_step_without_a_timing_fails_after_the_tick_bound() {
        let (s, a) = feed(loading(0), &[TICK; MAX_TICKS as usize], 2);
        assert_eq!(a, []);
        let (s, a) = next(s, TICK, 2);
        assert_eq!(s, State::Done);
        assert!(matches!(a.as_slice(), [Action::Fail(_)]));
    }

    fn sample(page: &str, temp: Temp, fcp: f64, served: usize) -> Sample {
        Sample {
            page: page.into(),
            temp,
            timing: Timing {
                page: page.into(),
                fp: Some(fcp - 1.0),
                fcp: Some(fcp),
                dcl: Some(fcp + 5.0),
                load: Some(fcp + 9.5),
                build: None,
                s0: Some(2.0),
                persisted: None,
            },
            host_ms: Some(fcp + 20.0),
            served,
            log: vec![],
        }
    }

    #[test]
    fn csv_and_table_on_fixed_samples() {
        let samples = [
            sample("/blank", Temp::Warmup, 5.0, 1),
            sample("/a", Temp::Cold, 10.0, 3),
            sample("/a", Temp::Warm, 20.0, 1),
            sample("/a", Temp::Warm, 40.0, 1),
        ];
        assert_eq!(
            csv("blocker", &samples),
            "wubble,blocker,/a,cold,0,9.0,10.0,15.0,19.5,,2.0,30.0,3,\n\
             wubble,blocker,/a,warm,0,19.0,20.0,25.0,29.5,,2.0,40.0,1,\n\
             wubble,blocker,/a,warm,1,39.0,40.0,45.0,49.5,,2.0,60.0,1,\n"
        );
        let table = table(&samples);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3, "{table}");
        assert!(lines[1].contains("/a") && lines[1].contains("cold"));
        // Nearest rank: the median of two is the first, the 95th the second.
        assert!(lines[2].contains("20.0/40.0"), "{}", lines[2]);
        assert!(!table.contains("warmup"));
    }
}
