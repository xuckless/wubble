//! `--measure <scenario>`: a hand-run measurement of the frame path.
//!
//! A bundled page runs for `--seconds` after a short warm-up and every frame
//! is recorded: when CEF painted it and what the in-callback copy cost
//! (welding's `CopiedFrame`), when the host took it, when Slint rendered it
//! (`RenderingState::AfterRendering`, which with FIFO present follows the
//! display), how many CEF frames were replaced before the host took one, and
//! for `typing` the time from an injected key or click to the first rendered
//! frame whose probe pixel changed. It prints a table (nearest-rank
//! p50/p95/p99/max), counts and histograms, and writes the samples as CSV.
//!
//! The numbers are printed, never asserted: nothing here runs in `cargo
//! test` except the pure [`Recorder`], summary, histogram and CSV code on
//! fixed inputs. [`Recorder`] reads no clock; the hooks at the bottom stamp
//! events and are the only impure part.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::keyboard::{Key, KeyCode, KeyLocation, PhysicalKey};
use wubble_engine::input::{Buttons, KeyInput, Modifiers, MouseButton, Point};
use wubble_gfx::{FrameSize, FrameTiming, NativeFrame};
use wubble_ui::AppWindow;

use crate::host::Host;

/// Time from the first frame before recording starts: page load and the
/// first layout settle.
const WARMUP: Duration = Duration::from_secs(1);

/// How often [`tick`] runs: input injection and window resizes.
pub const TICK: Duration = Duration::from_millis(16);

/// Ticks without a first frame after which the run gives up (about 20 s).
const MAX_WAIT_TICKS: u32 = 1250;

/// Ticks between two injected inputs in `typing`.
const TICKS_PER_INPUT: u32 = 12;

/// The probe pixel, in physical pixels of the frame: inside the page's
/// probe square (`measure-typing.html`, 0..64 DIP).
pub const PROBE: (u32, u32) = (16, 16);

/// Where injected clicks land, in view-relative logical pixels: on the probe
/// square.
const CLICK_AT: Point = Point { x: 16.0, y: 16.0 };

/// How far `resize` sweeps the window, in logical pixels, and per tick.
const RESIZE_SWEEP: f32 = 320.0;
const RESIZE_STEP: f32 = 8.0;

/// Histogram bucket edges in milliseconds. 8.33 and 16.67 are one and two
/// frames at 120 Hz.
const EDGES_MS: [f64; 9] = [1.0, 2.0, 4.0, 8.33, 12.0, 16.67, 25.0, 33.33, 50.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Scenario {
    /// A tall page scrolled by script every animation frame.
    Scroll,
    /// CSS animation and a canvas redrawn every animation frame.
    Video,
    /// An animated page while the window resizes itself.
    Resize,
    /// Keys and clicks that flip the probe square's colour.
    Typing,
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Self::Scroll => "scroll",
            Self::Video => "video",
            Self::Resize => "resize",
            Self::Typing => "typing",
        }
    }

    pub fn page(self) -> &'static str {
        match self {
            Self::Scroll => include_str!("../assets/measure-scroll.html"),
            Self::Video => include_str!("../assets/measure-video.html"),
            Self::Resize => include_str!("../assets/measure-resize.html"),
            Self::Typing => include_str!("../assets/measure-typing.html"),
        }
    }

    /// Whether each taken frame is probed (a GPU readback and wait).
    fn probes(self) -> bool {
        self == Self::Typing
    }
}

/// One recorded series, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Metric {
    /// Between two `OnAcceleratedPaint` entries of taken frames.
    ArrivalInterval,
    /// The whole in-callback copy (import, submit, wait).
    Copy,
    /// The `device.poll(Wait)` part of `Copy`.
    CopyWait,
    /// `OnAcceleratedPaint` entry to the host taking the frame.
    ArrivalToTake,
    /// `OnAcceleratedPaint` entry to the Slint render that showed it.
    ArrivalToPresent,
    /// Between two Slint renders.
    PresentInterval,
    /// Injected key to the render of the first frame that shows it.
    KeyToPixels,
    /// Injected click to the render of the first frame that shows it.
    PointerToPixels,
}

impl Metric {
    pub const ALL: [Self; 8] = [
        Self::ArrivalInterval,
        Self::Copy,
        Self::CopyWait,
        Self::ArrivalToTake,
        Self::ArrivalToPresent,
        Self::PresentInterval,
        Self::KeyToPixels,
        Self::PointerToPixels,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ArrivalInterval => "cef_arrival_interval",
            Self::Copy => "callback_copy",
            Self::CopyWait => "callback_copy_wait",
            Self::ArrivalToTake => "arrival_to_take",
            Self::ArrivalToPresent => "arrival_to_present",
            Self::PresentInterval => "present_interval",
            Self::KeyToPixels => "key_to_pixels",
            Self::PointerToPixels => "pointer_to_pixels",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Key,
    Pointer,
}

/// What the host saw of one frame it took.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub generation: u64,
    pub timing: Option<FrameTiming>,
    pub size: FrameSize,
    /// The size CEF paints for the window's content area right now.
    pub wanted: FrameSize,
}

impl Frame {
    pub fn of(frame: &NativeFrame, wanted: FrameSize) -> Self {
        Self {
            generation: frame.generation(),
            timing: frame.timing(),
            size: frame.size(),
            wanted,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// The host took `frame` and set it on the window; `probe` is its probe
    /// pixel when the scenario probes.
    Taken {
        at: Instant,
        frame: Frame,
        probe: Option<[u8; 4]>,
    },
    /// Slint finished rendering the window.
    Rendered(Instant),
    Injected {
        at: Instant,
        kind: InputKind,
    },
    ResizeRequested,
}

/// Frame counts over the recording.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Frames CEF painted from the first taken to the last taken one.
    pub delivered: u64,
    /// Frames the host took and set on the window.
    pub presented: u64,
    /// Frames CEF painted that a newer one replaced before the host took it.
    pub coalesced: u64,
    /// Frames set on the window that a newer one replaced before Slint
    /// rendered.
    pub unrendered: u64,
    pub renders: u64,
    /// Renders with no new web frame since the previous render.
    pub repeats: u64,
    /// Taken frames whose size differs from the content area's.
    pub stale_frames: u64,
    /// Most stale frames taken in a row.
    pub longest_stale_run: u64,
    pub resizes: u64,
    pub inputs: u64,
    /// Inputs replaced by the next one before any frame showed them.
    pub unanswered: u64,
}

/// Folds events into samples and counts. Pure: every time comes in an event.
#[derive(Debug, Default)]
pub struct Recorder {
    /// Every sample in arrival order.
    samples: Vec<(Metric, f64)>,
    counts: Counts,
    first_generation: Option<u64>,
    last_generation: Option<u64>,
    last_arrived: Option<Instant>,
    last_render: Option<Instant>,
    /// The taken frame not rendered yet: when CEF painted it, if known.
    awaiting_render: Option<Option<Instant>>,
    new_frame: bool,
    stale_run: u64,
    last_probe: Option<[u8; 4]>,
    /// An input no frame has shown yet, and the probe pixel before it.
    input: Option<(Instant, InputKind, Option<[u8; 4]>)>,
    /// An input a taken frame shows, waiting for that frame's render.
    shown: Option<(Instant, InputKind)>,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl Recorder {
    pub fn record(&mut self, event: Event) {
        match event {
            Event::Taken { at, frame, probe } => self.taken(at, frame, probe),
            Event::Rendered(at) => self.rendered(at),
            Event::Injected { at, kind } => {
                self.counts.inputs += 1;
                if self.input.is_some() {
                    self.counts.unanswered += 1;
                }
                self.input = Some((at, kind, self.last_probe));
            }
            Event::ResizeRequested => self.counts.resizes += 1,
        }
    }

    fn push(&mut self, metric: Metric, value: Duration) {
        self.samples.push((metric, ms(value)));
    }

    fn taken(&mut self, at: Instant, frame: Frame, probe: Option<[u8; 4]>) {
        let c = &mut self.counts;
        c.presented += 1;
        let first = *self.first_generation.get_or_insert(frame.generation);
        // A generation that goes back is another view; only gaps count.
        if let Some(last) = self.last_generation {
            c.coalesced += frame.generation.saturating_sub(last + 1);
        }
        self.last_generation = Some(frame.generation);
        c.delivered = frame.generation.saturating_sub(first) + 1;
        if self.awaiting_render.is_some() {
            c.unrendered += 1;
        }
        if frame.size == frame.wanted {
            self.stale_run = 0;
        } else {
            c.stale_frames += 1;
            self.stale_run += 1;
            c.longest_stale_run = c.longest_stale_run.max(self.stale_run);
        }
        self.new_frame = true;
        let arrived = frame.timing.map(|t| t.arrived);
        self.awaiting_render = Some(arrived);
        if let Some(t) = frame.timing {
            if let Some(last) = self.last_arrived {
                self.push(
                    Metric::ArrivalInterval,
                    t.arrived.saturating_duration_since(last),
                );
            }
            self.last_arrived = Some(t.arrived);
            self.push(Metric::Copy, t.copy);
            self.push(Metric::CopyWait, t.copy_wait);
            self.push(
                Metric::ArrivalToTake,
                at.saturating_duration_since(t.arrived),
            );
        }
        if let Some(pixel) = probe {
            if let Some((at, kind, before)) = self.input
                && before != Some(pixel)
            {
                self.shown = Some((at, kind));
                self.input = None;
            }
            self.last_probe = Some(pixel);
        }
    }

    fn rendered(&mut self, at: Instant) {
        self.counts.renders += 1;
        if let Some(last) = self.last_render {
            self.push(Metric::PresentInterval, at.saturating_duration_since(last));
            if !self.new_frame {
                self.counts.repeats += 1;
            }
        }
        self.last_render = Some(at);
        self.new_frame = false;
        if let Some(Some(arrived)) = self.awaiting_render.take() {
            self.push(
                Metric::ArrivalToPresent,
                at.saturating_duration_since(arrived),
            );
        }
        if let Some((injected, kind)) = self.shown.take() {
            let metric = match kind {
                InputKind::Key => Metric::KeyToPixels,
                InputKind::Pointer => Metric::PointerToPixels,
            };
            self.push(metric, at.saturating_duration_since(injected));
        }
    }

    #[cfg(test)]
    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    pub fn values(&self, metric: Metric) -> Vec<f64> {
        self.samples
            .iter()
            .filter(|(m, _)| *m == metric)
            .map(|(_, v)| *v)
            .collect()
    }

    /// Renders that came later than one refresh `period`: the vsyncs in
    /// between showed no new frame (rounded to whole periods).
    pub fn missed_vsyncs(&self, period: Duration) -> u64 {
        let period = ms(period);
        if period <= 0.0 {
            return 0;
        }
        self.values(Metric::PresentInterval)
            .iter()
            .map(|v| ((v / period).round() as u64).saturating_sub(1))
            .sum()
    }

    /// The samples as `metric,index,value_ms`, one row each, `index`
    /// counting within its metric.
    pub fn csv(&self) -> String {
        let mut out = String::from("metric,index,value_ms\n");
        let mut seen = [0usize; Metric::ALL.len()];
        for (metric, value) in &self.samples {
            let i = Metric::ALL.iter().position(|m| m == metric).unwrap_or(0);
            let _ = writeln!(out, "{},{},{value:.3}", metric.name(), seen[i]);
            seen[i] += 1;
        }
        out
    }

    /// The summary table, counts and histograms. `period` is the display's
    /// refresh period, if known.
    pub fn report(&self, period: Option<Duration>) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{:<22} {:>6} {:>8} {:>8} {:>8} {:>8}  (ms)",
            "metric", "n", "p50", "p95", "p99", "max"
        );
        for metric in Metric::ALL {
            match summary(&self.values(metric)) {
                Some(s) => {
                    let _ = writeln!(
                        out,
                        "{:<22} {:>6} {:>8.2} {:>8.2} {:>8.2} {:>8.2}",
                        metric.name(),
                        s.n,
                        s.p50,
                        s.p95,
                        s.p99,
                        s.max
                    );
                }
                None => {
                    let _ = writeln!(out, "{:<22} {:>6}", metric.name(), 0);
                }
            }
        }
        let c = &self.counts;
        let missed = period.map_or_else(|| "n/a".into(), |p| self.missed_vsyncs(p).to_string());
        let _ = writeln!(
            out,
            "frames: delivered={} presented={} coalesced={} unrendered={} renders={} repeats={} missed_vsyncs={missed}",
            c.delivered, c.presented, c.coalesced, c.unrendered, c.renders, c.repeats
        );
        let _ = writeln!(
            out,
            "resize: requests={} stale_frames={} longest_stale_run={}",
            c.resizes, c.stale_frames, c.longest_stale_run
        );
        let _ = writeln!(
            out,
            "input: injected={} unanswered={}",
            c.inputs, c.unanswered
        );
        for metric in Metric::ALL {
            let values = self.values(metric);
            if !values.is_empty() {
                let _ = writeln!(out, "\n{} (ms)", metric.name());
                out.push_str(&histogram_text(&histogram(&values, &EDGES_MS), &EDGES_MS));
            }
        }
        out
    }
}

/// Sample count and nearest-rank percentiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Nearest-rank percentiles (the smallest value with at least p% of the
/// samples at or below it); `None` without samples.
pub fn summary(values: &[f64]) -> Option<Summary> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let rank = |p: usize| sorted[(p * n).div_ceil(100).max(1) - 1];
    Some(Summary {
        n,
        p50: rank(50),
        p95: rank(95),
        p99: rank(99),
        max: *sorted.last()?,
    })
}

/// Counts per bucket: below `edges[0]`, each `[edges[i-1], edges[i])`, and
/// at or above the last edge. `edges.len() + 1` buckets.
pub fn histogram(values: &[f64], edges: &[f64]) -> Vec<usize> {
    let mut buckets = vec![0; edges.len() + 1];
    for v in values {
        buckets[edges.partition_point(|e| e <= v)] += 1;
    }
    buckets
}

/// Width of the longest histogram bar.
const BAR: usize = 40;

fn histogram_text(buckets: &[usize], edges: &[f64]) -> String {
    let most = buckets.iter().copied().max().unwrap_or(0).max(1);
    let mut out = String::new();
    for (i, &count) in buckets.iter().enumerate() {
        let label = match i {
            0 => format!("< {:.2}", edges[0]),
            i if i == edges.len() => format!(">= {:.2}", edges[i - 1]),
            i => format!("{:.2}-{:.2}", edges[i - 1], edges[i]),
        };
        let bar = "#".repeat((count * BAR).div_ceil(most).min(BAR));
        let _ = writeln!(out, "  {label:>12} {count:>6} {bar}");
    }
    out
}

/// The window size `resize` asks for at `step`: a triangle wave from `base`
/// up by [`RESIZE_SWEEP`] and back, both sides, [`RESIZE_STEP`] per tick.
pub fn resize_size(base: (f32, f32), step: u32) -> (f32, f32) {
    let steps = (RESIZE_SWEEP / RESIZE_STEP) as u32;
    let phase = step % (2 * steps);
    let offset = RESIZE_STEP * phase.min(2 * steps - phase) as f32;
    (base.0 + offset, base.1 + offset / 2.0)
}

// ----- the hooks: the only code here that reads a clock

/// One measurement run, on the Slint thread.
struct Session {
    scenario: Scenario,
    seconds: u32,
    recorder: Recorder,
    /// The first frame; recording starts [`WARMUP`] later.
    first_frame: Option<Instant>,
    ticks: u32,
    /// The window's logical size when `resize` started.
    base: Option<(f32, f32)>,
    /// The display's refresh and the window, read when recording starts.
    refresh_mhz: Option<u32>,
    window: String,
}

impl Session {
    fn recording(&self, now: Instant) -> bool {
        self.first_frame.is_some_and(|t| now >= t + WARMUP)
    }

    fn finished(&self, now: Instant) -> bool {
        self.first_frame
            .is_some_and(|t| now >= t + WARMUP + Duration::from_secs(self.seconds.into()))
    }
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

fn with_session<T>(f: impl FnOnce(&mut Session) -> T) -> Option<T> {
    SESSION.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Starts recording `scenario` for `seconds` once the first frame arrives.
pub fn begin(scenario: Scenario, seconds: u32) {
    SESSION.set(Some(Session {
        scenario,
        seconds,
        recorder: Recorder::default(),
        first_frame: None,
        ticks: 0,
        base: None,
        refresh_mhz: None,
        window: "n/a".into(),
    }));
}

/// Host hook: `frame` was set on the window. `probe` reads its probe pixel
/// and runs only when the scenario probes. Nothing happens when not
/// measuring.
pub fn taken(frame: Frame, probe: impl FnOnce() -> Option<[u8; 4]>) {
    let Some(probes) = with_session(|s| {
        let now = Instant::now();
        s.first_frame.get_or_insert(now);
        s.recording(now).then_some(s.scenario.probes())
    })
    .flatten() else {
        return;
    };
    let probe = probes.then(probe).flatten();
    let at = Instant::now();
    with_session(|s| s.recorder.record(Event::Taken { at, frame, probe }));
}

/// Rendering-notifier hook: Slint finished a render.
pub fn rendered() {
    with_session(|s| {
        let now = Instant::now();
        if s.recording(now) {
            s.recorder.record(Event::Rendered(now));
        }
    });
}

/// Timer hook, every [`TICK`] while measuring: injects input (`typing`) or
/// resizes the window (`resize`), and quits once the run is over.
pub fn tick(host: &mut Host, window: &AppWindow) {
    let now = Instant::now();
    let Some((scenario, step, base)) = with_session(|s| {
        if s.finished(now) {
            crate::quit();
            return None;
        }
        if s.first_frame.is_none() {
            s.ticks += 1;
            if s.ticks > MAX_WAIT_TICKS {
                eprintln!("measure: no frame arrived; giving up");
                crate::quit();
            }
            return None;
        }
        if !s.recording(now) {
            s.ticks = 0;
            return None;
        }
        s.ticks += 1;
        let scale = window.window().scale_factor();
        let size = window.window().size();
        if s.base.is_none() {
            s.refresh_mhz = window
                .window()
                .with_winit_window(crate::display_refresh_mhz)
                .flatten();
            s.window = format!("{}x{}@{scale}", size.width, size.height);
        }
        let size = size.to_logical(scale);
        let base = *s.base.get_or_insert((size.width, size.height));
        Some((s.scenario, s.ticks, base))
    })
    .flatten() else {
        return;
    };
    match scenario {
        Scenario::Resize => {
            let (width, height) = resize_size(base, step);
            window
                .window()
                .set_size(slint::LogicalSize::new(width, height));
            with_session(|s| s.recorder.record(Event::ResizeRequested));
        }
        Scenario::Typing if step.is_multiple_of(TICKS_PER_INPUT) => {
            let kind = if (step / TICKS_PER_INPUT).is_multiple_of(2) {
                InputKind::Key
            } else {
                InputKind::Pointer
            };
            inject(host, kind);
        }
        _ => {}
    }
}

/// One key press and release, or one click, sent through the same `View`
/// calls real input uses; stamped just before sending.
fn inject(host: &mut Host, kind: InputKind) {
    let Some(view) = host.view() else {
        return;
    };
    let at = Instant::now();
    match kind {
        InputKind::Key => {
            view.focus(true);
            for pressed in [true, false] {
                view.key(&KeyInput {
                    pressed,
                    repeat: false,
                    physical: PhysicalKey::Code(KeyCode::KeyA),
                    // evdev KEY_A
                    scancode: Some(30),
                    unmodified: Key::Character("a".into()),
                    text: pressed.then(|| "a".into()),
                    location: KeyLocation::Standard,
                    modifiers: Modifiers::default(),
                });
            }
        }
        InputKind::Pointer => {
            let now = at.elapsed();
            for pressed in [true, false] {
                let modifiers = Modifiers {
                    buttons: Buttons::default().with(MouseButton::Left, pressed),
                    ..Modifiers::default()
                };
                view.mouse_button(MouseButton::Left, pressed, CLICK_AT, &modifiers, now);
            }
        }
    }
    with_session(|s| s.recorder.record(Event::Injected { at, kind }));
}

/// Ends the run: the report text and the CSV, `None` if none was started.
/// `adapter` names the GPU for the header.
pub fn finish(adapter: &str) -> Option<(String, String)> {
    let session = SESSION.take()?;
    let period = session
        .refresh_mhz
        .filter(|&m| m > 0)
        .map(|m| Duration::from_secs_f64(1000.0 / f64::from(m)));
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_else(|_| "n/a".into());
    let refresh = session.refresh_mhz.map_or_else(
        || "n/a".into(),
        |m| format!("{:.3} Hz", f64::from(m) / 1000.0),
    );
    let header = format!(
        "measure: scenario={} seconds={} refresh={refresh} window={} adapter={}\nmeasure: loadavg {}\n",
        session.scenario.name(),
        session.seconds,
        session.window,
        adapter,
        load.trim()
    );
    let report = header + &session.recorder.report(period);
    Some((report, session.recorder.csv()))
}

/// Where `--out` writes the CSV.
pub fn write_csv(path: &PathBuf, csv: &str) -> std::io::Result<()> {
    std::fs::write(path, csv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(generation: u64, arrived: Option<Instant>, size: u32, wanted: u32) -> Frame {
        Frame {
            generation,
            timing: arrived.map(|arrived| FrameTiming {
                arrived,
                copy: Duration::from_micros(500),
                copy_wait: Duration::from_micros(300),
            }),
            size: FrameSize::new(size, size),
            wanted: FrameSize::new(wanted, wanted),
        }
    }

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn nearest_rank_percentiles() {
        assert_eq!(summary(&[]), None);
        assert_eq!(
            summary(&[7.0]),
            Some(Summary {
                n: 1,
                p50: 7.0,
                p95: 7.0,
                p99: 7.0,
                max: 7.0
            })
        );
        let hundred: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        let s = summary(&hundred).unwrap();
        assert_eq!(
            (s.n, s.p50, s.p95, s.p99, s.max),
            (100, 50.0, 95.0, 99.0, 100.0)
        );
        let s = summary(&[2.0, 2.0, 2.0, 9.0]).unwrap();
        assert_eq!((s.p50, s.p95, s.p99, s.max), (2.0, 9.0, 9.0, 9.0));
        let ten: Vec<f64> = (1..=10).map(f64::from).collect();
        let s = summary(&ten).unwrap();
        assert_eq!((s.p50, s.p95, s.p99), (5.0, 10.0, 10.0));
    }

    #[test]
    fn histogram_buckets_include_their_lower_edge() {
        let edges = [1.0, 2.0, 4.0];
        assert_eq!(
            histogram(&[0.5, 1.0, 1.5, 2.0, 3.9, 4.0, 99.0], &edges),
            vec![1, 2, 2, 2]
        );
        assert_eq!(histogram(&[], &edges), vec![0, 0, 0, 0]);
        assert_eq!(
            histogram_text(&[0, 2, 1, 0], &edges),
            "        < 1.00      0 \n     1.00-2.00      2 ########################################\n     2.00-4.00      1 ####################\n       >= 4.00      0 \n"
        );
    }

    #[test]
    fn coalesced_unrendered_and_repeats_from_a_fixed_sequence() {
        let t0 = Instant::now();
        let mut r = Recorder::default();
        let events = [
            // Generation 3 taken and rendered.
            Event::Taken {
                at: at(t0, 2),
                frame: frame(3, Some(at(t0, 0)), 10, 10),
                probe: None,
            },
            Event::Rendered(at(t0, 8)),
            // 4 and 5 were replaced in the slot: 6 is taken, then 7 before a
            // render, so 6 is never rendered.
            Event::Taken {
                at: at(t0, 10),
                frame: frame(6, Some(at(t0, 9)), 10, 10),
                probe: None,
            },
            Event::Taken {
                at: at(t0, 13),
                frame: frame(7, Some(at(t0, 12)), 10, 10),
                probe: None,
            },
            Event::Rendered(at(t0, 16)),
            // A render 17 ms later without a new frame: one repeat.
            Event::Rendered(at(t0, 33)),
        ];
        for e in events {
            r.record(e);
        }
        let c = r.counts();
        assert_eq!(c.delivered, 5);
        assert_eq!(c.presented, 3);
        assert_eq!(c.coalesced, 2);
        assert_eq!(c.unrendered, 1);
        assert_eq!(c.renders, 3);
        assert_eq!(c.repeats, 1);
        assert_eq!(r.values(Metric::PresentInterval), vec![8.0, 17.0]);
        assert_eq!(r.values(Metric::ArrivalInterval), vec![9.0, 3.0]);
        assert_eq!(r.values(Metric::ArrivalToTake), vec![2.0, 1.0, 1.0]);
        assert_eq!(r.values(Metric::ArrivalToPresent), vec![8.0, 4.0]);
        assert_eq!(r.values(Metric::CopyWait), vec![0.3, 0.3, 0.3]);
        // 8 ms is one period, 17 ms two: one vsync without a new frame.
        assert_eq!(r.missed_vsyncs(Duration::from_micros(8_333)), 1);
    }

    #[test]
    fn longest_stale_run_from_a_fixed_size_sequence() {
        let t0 = Instant::now();
        let mut r = Recorder::default();
        // (frame size, content size): stale, stale, fresh, stale x3, fresh.
        let sizes = [
            (10, 12),
            (10, 12),
            (12, 12),
            (12, 14),
            (12, 16),
            (14, 16),
            (16, 16),
        ];
        for (i, (size, wanted)) in sizes.into_iter().enumerate() {
            r.record(Event::Taken {
                at: t0,
                frame: frame(i as u64 + 1, None, size, wanted),
                probe: None,
            });
        }
        assert_eq!(r.counts().stale_frames, 5);
        assert_eq!(r.counts().longest_stale_run, 3);
        // Frames without CEF timing add no timing samples.
        assert!(r.values(Metric::ArrivalToTake).is_empty());
    }

    #[test]
    fn input_latency_ends_at_the_render_of_the_first_changed_frame() {
        let t0 = Instant::now();
        let white = Some([255; 4]);
        let black = Some([0, 0, 0, 255]);
        let taken = |ms, generation, probe| Event::Taken {
            at: at(t0, ms),
            frame: frame(generation, None, 10, 10),
            probe,
        };
        let mut r = Recorder::default();
        for e in [
            taken(0, 1, white),
            Event::Injected {
                at: at(t0, 5),
                kind: InputKind::Key,
            },
            // Not changed yet.
            taken(8, 2, white),
            Event::Rendered(at(t0, 9)),
            taken(16, 3, black),
            Event::Rendered(at(t0, 21)),
            // An input no frame answers before the next one.
            Event::Injected {
                at: at(t0, 30),
                kind: InputKind::Pointer,
            },
            Event::Injected {
                at: at(t0, 40),
                kind: InputKind::Pointer,
            },
            taken(45, 4, white),
            Event::Rendered(at(t0, 50)),
        ] {
            r.record(e);
        }
        assert_eq!(r.values(Metric::KeyToPixels), vec![16.0]);
        assert_eq!(r.values(Metric::PointerToPixels), vec![10.0]);
        assert_eq!(r.counts().inputs, 3);
        assert_eq!(r.counts().unanswered, 1);
    }

    #[test]
    fn csv_is_one_row_per_sample_indexed_per_metric() {
        let t0 = Instant::now();
        let mut r = Recorder::default();
        for e in [
            Event::Rendered(at(t0, 0)),
            Event::Rendered(at(t0, 8)),
            Event::Taken {
                at: at(t0, 9),
                frame: frame(1, Some(at(t0, 8)), 4, 4),
                probe: None,
            },
            Event::Rendered(at(t0, 17)),
        ] {
            r.record(e);
        }
        assert_eq!(
            r.csv(),
            "metric,index,value_ms\n\
             present_interval,0,8.000\n\
             callback_copy,0,0.500\n\
             callback_copy_wait,0,0.300\n\
             arrival_to_take,0,1.000\n\
             present_interval,1,9.000\n\
             arrival_to_present,0,9.000\n"
        );
    }

    #[test]
    fn resize_sweeps_up_and_back() {
        let base = (800.0, 600.0);
        assert_eq!(resize_size(base, 0), base);
        assert_eq!(resize_size(base, 1), (808.0, 604.0));
        assert_eq!(resize_size(base, 40), (1120.0, 760.0));
        assert_eq!(resize_size(base, 41), (1112.0, 756.0));
        assert_eq!(resize_size(base, 80), base);
    }
}
