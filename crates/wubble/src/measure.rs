//! `--measure <scenario>`: a hand-run measurement of the frame path.
//!
//! A bundled page runs for `--seconds` after a short warm-up and every frame
//! is recorded: when CEF painted it and what the in-callback copy cost
//! (welding's `CopiedFrame`), when the host took it, when Slint rendered it
//! (`RenderingState::AfterRendering`, which with FIFO present follows the
//! display), how many CEF frames were replaced before the host took one, and
//! for `typing` the time from an injected key or click to the first rendered
//! frame whose probe pixel changed. It also times the UI thread's stages: each
//! run of CEF work and the gap between runs, the presenter's blit, the scene's
//! CPU side and Slint's render (`BeforeRendering` to `AfterRendering`). The
//! report is printed (and the CSV written) before CEF shuts down. It prints a table (nearest-rank
//! p50/p95/p99/max), counts and histograms, and writes the samples as CSV.
//!
//! `idle` holds a still page for `--seconds` after a warm-up longer than the scene's pointer
//! hold, with the browser's host timers running, and reports what still happens: CEF frames,
//! Slint and scene renders, pump runs and every timer that woke the UI thread ([`Count`]),
//! the CPU time of wubble's process tree over the window (`wubble_sysmem::Meter`) and, on
//! Linux with an NVIDIA GPU, the GPU's busy share from an `nvidia-smi` child. `startup`
//! stamps the start-up stages ([`Stage`]) from the top of `main` and quits at the first
//! render that shows page pixels. Every scenario also prints the counts and the CPU table.
//!
//! The numbers are printed, never asserted: nothing here runs in `cargo
//! test` except the pure [`Recorder`], summary, histogram and CSV code on
//! fixed inputs. [`Recorder`] reads no clock; the hooks at the bottom stamp
//! events and are the only impure part.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::keyboard::{Key, KeyCode, KeyLocation, PhysicalKey};
use wubble_engine::input::{Buttons, KeyInput, Modifiers, MouseButton, Point};
use wubble_gfx::pacing::PaceEvent;
use wubble_gfx::{FrameSize, FrameTiming, NativeFrame};
use wubble_ui::AppWindow;

use crate::host::Host;

/// Time from the first frame before recording starts: page load and the
/// first layout settle.
const WARMUP: Duration = Duration::from_secs(1);

/// How often [`tick`] runs: input injection and window resizes.
pub const TICK: Duration = Duration::from_millis(16);

/// `idle`'s warm-up: long enough for the scene to settle after the window opens (a window
/// opening under the pointer gets one move, and the light catches up), so the window is measured
/// in its idle state.
const IDLE_WARMUP: Duration = Duration::from_secs(4);

/// `tabs`' warm-up: the other two tabs open and load behind the first.
const TABS_WARMUP: Duration = Duration::from_secs(2);

/// Ticks between two tab switches in `tabs` (about a second).
const TICKS_PER_SWITCH: u32 = 60;

/// `idle`'s tick: it only ends the run, and is itself a wake-up (counted).
const IDLE_TICK: Duration = Duration::from_secs(1);

/// Ticks without a first frame after which the run gives up (about 20 s).
const MAX_WAIT_TICKS: u32 = 1250;

/// Ticks between two injected inputs in `typing`.
const TICKS_PER_INPUT: u32 = 12;

/// The probe pixel, in physical pixels of the frame: inside the page's
/// probe square (`measure-typing.html`, 0..64 DIP).
pub const PROBE: (u32, u32) = (16, 16);

/// Where injected clicks land, in view-relative logical pixels: on the probe
/// square, alternating between two points further apart than a click series
/// may move, so every injected click is a single click.
const CLICK_AT: [Point; 2] = [Point { x: 16.0, y: 16.0 }, Point { x: 48.0, y: 48.0 }];

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
    /// The content blocker alone, windowless: compile, cache and per-request
    /// cost of the bundled lists (`blocking::measure`).
    Blocker,
    /// A still page, the pointer still, the host's timers running: what keeps drawing.
    Idle,
    /// Process start to the first render showing page pixels, by stage.
    Startup,
    /// Loopback pages loaded cold and warm: the page's own timings and the first frame shown
    /// (`load_run`).
    Load,
    /// The process tree's memory with no tab, 1, 10 and 50 tabs, then frozen and discarded
    /// by the sleep policy (`memory.rs`).
    Memory,
    /// Three bundled pages in tabs (the first animating: `video`'s), the next one shown every
    /// [`TICKS_PER_SWITCH`]: per switch the time to the new tab's first frame on screen and
    /// the frame intervals while the shell's switch animation runs.
    Tabs,
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Self::Scroll => "scroll",
            Self::Video => "video",
            Self::Resize => "resize",
            Self::Typing => "typing",
            Self::Blocker => "blocker",
            Self::Idle => "idle",
            Self::Startup => "startup",
            Self::Load => "load",
            Self::Memory => "memory",
            Self::Tabs => "tabs",
        }
    }

    pub fn page(self) -> &'static str {
        match self {
            Self::Scroll => include_str!("../assets/measure-scroll.html"),
            Self::Video | Self::Tabs => include_str!("../assets/measure-video.html"),
            Self::Resize => include_str!("../assets/measure-resize.html"),
            Self::Typing => include_str!("../assets/measure-typing.html"),
            Self::Idle | Self::Startup | Self::Load => include_str!("../assets/measure-idle.html"),
            // Windowless: `main` runs `blocking::measure` instead of a page.
            // The memory run opens its own pages.
            Self::Blocker | Self::Memory => "",
        }
    }

    /// Whether each taken frame is probed (a GPU readback and wait).
    fn probes(self) -> bool {
        self == Self::Typing
    }

    /// `tabs`: the pages of the tabs opened behind the first ([`Scenario::page`]), still ones.
    pub fn tab_pages() -> [&'static str; 2] {
        [
            include_str!("../assets/measure-idle.html"),
            include_str!("../assets/measure-typing.html"),
        ]
    }

    fn warmup(self) -> Duration {
        match self {
            Self::Idle => IDLE_WARMUP,
            Self::Tabs => TABS_WARMUP,
            _ => WARMUP,
        }
    }

    /// How often [`tick`] runs.
    pub fn tick_every(self) -> Duration {
        match self {
            Self::Idle => IDLE_TICK,
            _ => TICK,
        }
    }

    /// Whether the browser's own timers (media poll, video clock, audible probe, lifecycle)
    /// run, as they do when browsing.
    pub fn host_timers(self) -> bool {
        self == Self::Idle
    }
}

/// What woke the UI thread or drew, counted over the recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    /// A frame the host took from CEF, any view.
    CefFrame,
    /// ... of which only CEF's damage was written, in place (`FramePresenter::present_damaged`).
    CefFrameDamaged,
    SlintRender,
    /// A frame of the wgpu scene (the glass backdrop).
    SceneRender,
    /// A pump request handed to the host (CEF's schedule or the pump timer).
    Pump,
    /// ... of which CEF did work.
    PumpWorked,
    /// The host's timers.
    MediaPoll,
    SurfaceTick,
    AudibleProbe,
    Lifecycle,
    /// `--measure`'s own tick.
    MeasureTick,
    /// The scene's run state changed.
    RunChange,
    /// What woke a still or paused scene (`run_state::Runner::on`): the pointer moving,
    /// pressing or scrolling; the pointer coming in; a key or the IME; a touch; the window
    /// shown, focused or resized; the address beam or a Space's palette; the ambience; the start
    /// page; a frame drawn for a change (glass list, size, page blur) that left something moving.
    WakePointer,
    WakeEntered,
    WakeKey,
    WakeTouch,
    WakeShown,
    WakeBeam,
    WakeAmbience,
    WakeStart,
    WakeChange,
}

impl Count {
    pub const ALL: [Self; 21] = [
        Self::CefFrame,
        Self::CefFrameDamaged,
        Self::SlintRender,
        Self::SceneRender,
        Self::Pump,
        Self::PumpWorked,
        Self::MediaPoll,
        Self::SurfaceTick,
        Self::AudibleProbe,
        Self::Lifecycle,
        Self::MeasureTick,
        Self::RunChange,
        Self::WakePointer,
        Self::WakeEntered,
        Self::WakeKey,
        Self::WakeTouch,
        Self::WakeShown,
        Self::WakeBeam,
        Self::WakeAmbience,
        Self::WakeStart,
        Self::WakeChange,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::CefFrame => "cef_frames",
            Self::SlintRender => "slint_renders",
            Self::SceneRender => "scene_renders",
            Self::Pump => "pump_runs",
            Self::PumpWorked => "pump_worked",
            Self::MediaPoll => "media_polls",
            Self::SurfaceTick => "surface_ticks",
            Self::AudibleProbe => "audible_probes",
            Self::Lifecycle => "lifecycle_ticks",
            Self::MeasureTick => "measure_ticks",
            Self::RunChange => "run_state_changes",
            Self::CefFrameDamaged => "cef_frames_damaged",
            Self::WakePointer => "scene_wake_pointer",
            Self::WakeEntered => "scene_wake_entered",
            Self::WakeKey => "scene_wake_key",
            Self::WakeTouch => "scene_wake_touch",
            Self::WakeShown => "scene_wake_shown",
            Self::WakeBeam => "scene_wake_beam",
            Self::WakeAmbience => "scene_wake_ambience",
            Self::WakeStart => "scene_wake_start",
            Self::WakeChange => "scene_wake_change",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }
}

/// One recorded series, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Metric {
    /// Between the `OnAcceleratedPaint` entries of two frames the host took.
    /// Not CEF's paint interval: a paint replaced before the host took it
    /// (coalesced) is skipped, so that interval reads double.
    TakenInterval,
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
    /// One run of CEF work on the host (`do_message_loop_work`, which runs
    /// `OnAcceleratedPaint` and its copy, then the host's handling).
    PumpWork,
    /// Between the starts of two runs of CEF work.
    PumpGap,
    /// The presenter's import and blit of one taken frame (CPU side).
    Blit,
    /// The scene's step, encode and submit in the rendering notifier (CPU side).
    Scene,
    /// Slint's `BeforeRendering` to `AfterRendering`: the shell step, the
    /// scene, Skia's draw and submit and the surface acquire.
    SlintRender,
    /// `tabs`: a tab switch to the render that showed the new tab's first frame from CEF.
    SwitchToFrame,
    /// `tabs`: between two renders while the shell's switch animation runs (the page's
    /// entrance or the hidden page's exit).
    SwitchInterval,
}

impl Metric {
    pub const ALL: [Self; 15] = [
        Self::TakenInterval,
        Self::Copy,
        Self::CopyWait,
        Self::ArrivalToTake,
        Self::ArrivalToPresent,
        Self::PresentInterval,
        Self::KeyToPixels,
        Self::PointerToPixels,
        Self::PumpWork,
        Self::PumpGap,
        Self::Blit,
        Self::Scene,
        Self::SlintRender,
        Self::SwitchToFrame,
        Self::SwitchInterval,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::TakenInterval => "taken_paint_interval",
            Self::Copy => "callback_copy",
            Self::CopyWait => "callback_copy_wait",
            Self::ArrivalToTake => "arrival_to_take",
            Self::ArrivalToPresent => "arrival_to_present",
            Self::PresentInterval => "present_interval",
            Self::KeyToPixels => "key_to_pixels",
            Self::PointerToPixels => "pointer_to_pixels",
            Self::PumpWork => "pump_work",
            Self::PumpGap => "pump_gap",
            Self::Blit => "blit",
            Self::Scene => "scene_cpu",
            Self::SlintRender => "slint_render",
            Self::SwitchToFrame => "switch_to_frame",
            Self::SwitchInterval => "switch_interval",
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
    /// Slint started rendering the window (`BeforeRendering`).
    RenderBegan(Instant),
    /// Slint finished rendering the window.
    Rendered(Instant),
    /// A run of CEF work started at `at` and took `took`.
    Pumped {
        at: Instant,
        took: Duration,
    },
    /// One stage of the frame path took this long ([`Metric::Blit`],
    /// [`Metric::Scene`]).
    Stage(Metric, Duration),
    Injected {
        at: Instant,
        kind: InputKind,
    },
    ResizeRequested,
    /// `tabs` showed another tab.
    Switched(Instant),
    /// The render about to start draws the shell's switch animation, or not.
    Switching(bool),
    /// One shell frame (the panes' step), what it laid out.
    Shell(Shell),
    /// Something counted happened once.
    Count(Count),
}

/// One shell frame: whether the scene's texture and the panes were laid out for this frame's
/// window size, and whether a pane was stretched (drawn past filling it, scale > 1) or masked
/// (a bowed edge).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shell {
    pub scene_off: bool,
    pub pane_off: bool,
    pub stretched: bool,
    pub bowed: bool,
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
    /// Shell frames; of them, the scene's texture not at the window's size, the panes not
    /// spanning the content width, a pane stretched, a pane masked.
    pub shell_frames: u64,
    pub scene_off: u64,
    pub pane_off: u64,
    pub stretched: u64,
    pub bowed: u64,
    pub inputs: u64,
    /// Inputs replaced by the next one before any frame showed them.
    pub unanswered: u64,
}

/// Where the last tab switch is on its way to the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SwitchWait {
    #[default]
    None,
    /// Switched at this time; no frame of the new tab taken yet.
    Frame(Instant),
    /// Its first frame is taken, the render showing it is next.
    Render(Instant),
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
    render_began: Option<Instant>,
    last_pump: Option<Instant>,
    /// The taken frame not rendered yet: when CEF painted it, if known.
    awaiting_render: Option<Option<Instant>>,
    new_frame: bool,
    stale_run: u64,
    last_probe: Option<[u8; 4]>,
    /// An input no frame has shown yet, and the probe pixel before it.
    input: Option<(Instant, InputKind, Option<[u8; 4]>)>,
    /// An input a taken frame shows, waiting for that frame's render.
    shown: Option<(Instant, InputKind)>,
    /// The last tab switch, until its new tab's first frame is rendered.
    switch: SwitchWait,
    /// The render under way draws the switch animation.
    switching: bool,
    /// Per [`Count`], in [`Count::ALL`] order.
    counted: [u64; Count::ALL.len()],
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl Recorder {
    pub fn record(&mut self, event: Event) {
        match event {
            Event::Taken { at, frame, probe } => self.taken(at, frame, probe),
            Event::RenderBegan(at) => self.render_began = Some(at),
            Event::Rendered(at) => self.rendered(at),
            Event::Pumped { at, took } => {
                if let Some(last) = self.last_pump.replace(at) {
                    self.push(Metric::PumpGap, at.saturating_duration_since(last));
                }
                self.push(Metric::PumpWork, took);
            }
            Event::Stage(metric, took) => self.push(metric, took),
            Event::Injected { at, kind } => {
                self.counts.inputs += 1;
                if self.input.is_some() {
                    self.counts.unanswered += 1;
                }
                self.input = Some((at, kind, self.last_probe));
            }
            Event::ResizeRequested => self.counts.resizes += 1,
            Event::Switched(at) => self.switch = SwitchWait::Frame(at),
            Event::Switching(on) => self.switching = on,
            Event::Shell(shell) => {
                let c = &mut self.counts;
                c.shell_frames += 1;
                c.scene_off += u64::from(shell.scene_off);
                c.pane_off += u64::from(shell.pane_off);
                c.stretched += u64::from(shell.stretched);
                c.bowed += u64::from(shell.bowed);
            }
            Event::Count(count) => self.counted[count.index()] += 1,
        }
    }

    pub fn counted(&self, count: Count) -> u64 {
        self.counted[count.index()]
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
        if let SwitchWait::Frame(at) = self.switch {
            self.switch = SwitchWait::Render(at);
        }
        let arrived = frame.timing.map(|t| t.arrived);
        self.awaiting_render = Some(arrived);
        if let Some(t) = frame.timing {
            if let Some(last) = self.last_arrived {
                self.push(
                    Metric::TakenInterval,
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
        if let Some(began) = self.render_began.take() {
            self.push(Metric::SlintRender, at.saturating_duration_since(began));
        }
        if let Some(last) = self.last_render {
            self.push(Metric::PresentInterval, at.saturating_duration_since(last));
            if self.switching {
                self.push(Metric::SwitchInterval, at.saturating_duration_since(last));
            }
            if !self.new_frame {
                self.counts.repeats += 1;
            }
        }
        if let SwitchWait::Render(switched) = self.switch {
            self.push(
                Metric::SwitchToFrame,
                at.saturating_duration_since(switched),
            );
            self.switch = SwitchWait::None;
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
    /// refresh period, if known; `seconds` the recording's length, for the rates.
    pub fn report(&self, period: Option<Duration>, seconds: f64) -> String {
        let mut out = self.counts_text(seconds);
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
            "shell: frames={} scene_off={} pane_off={} stretched={} bowed={}",
            c.shell_frames, c.scene_off, c.pane_off, c.stretched, c.bowed
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

impl Recorder {
    /// Each [`Count`] and its rate per second over `seconds`.
    pub fn counts_text(&self, seconds: f64) -> String {
        let mut out = format!("{:<22} {:>8} {:>9}\n", "count", "n", "per s");
        for count in Count::ALL {
            let n = self.counted(count);
            let rate = if seconds > 0.0 {
                n as f64 / seconds
            } else {
                0.0
            };
            let _ = writeln!(out, "{:<22} {n:>8} {rate:>9.2}", count.name());
        }
        out.push('\n');
        out
    }
}

/// One process's CPU time when sampled: pid, label (`wubble_sysmem::label`), milliseconds.
pub type CpuTime = (u32, String, u64);

/// When the tree was sampled: its processes, then the browser process's threads (Linux).
type CpuSnapshot = (Instant, (Vec<CpuTime>, Vec<CpuTime>));

/// One process's CPU use over the recording.
#[derive(Clone, Debug, PartialEq)]
pub struct CpuRow {
    pub pid: u32,
    pub label: String,
    pub cpu_ms: u64,
    /// Percent of one core.
    pub percent: f64,
}

/// CPU used over `wall` per process of `end`; one that started meanwhile counts all its time.
/// A process gone by `end` is left out (its time is unknown).
pub fn cpu_rows(start: &[CpuTime], end: &[CpuTime], wall: Duration) -> Vec<CpuRow> {
    let wall_ms = ms(wall);
    end.iter()
        .map(|(pid, label, at_end)| {
            let at_start = start
                .iter()
                .find(|(p, ..)| p == pid)
                .map_or(0, |(_, _, t)| *t);
            let cpu_ms = at_end.saturating_sub(at_start);
            CpuRow {
                pid: *pid,
                label: label.clone(),
                cpu_ms,
                percent: if wall_ms > 0.0 {
                    cpu_ms as f64 * 100.0 / wall_ms
                } else {
                    0.0
                },
            }
        })
        .collect()
}

/// The CPU table, with a total row.
pub fn cpu_text(rows: &[CpuRow], wall: Duration) -> String {
    let mut out = format!(
        "cpu over {:.1} s ({} processes)\n{:<12} {:>8} {:>9} {:>10}\n",
        wall.as_secs_f64(),
        rows.len(),
        "process",
        "pid",
        "cpu ms",
        "% of core"
    );
    for r in rows {
        let _ = writeln!(
            out,
            "{:<12} {:>8} {:>9} {:>10.2}",
            r.label, r.pid, r.cpu_ms, r.percent
        );
    }
    let total: u64 = rows.iter().map(|r| r.cpu_ms).sum();
    let percent: f64 = rows.iter().map(|r| r.percent).sum();
    let _ = writeln!(out, "{:<12} {:>8} {total:>9} {percent:>10.2}", "total", "");
    out
}

/// The CPU rows as `process,pid,cpu_ms,percent_of_core`.
pub fn cpu_csv(rows: &[CpuRow]) -> String {
    let mut out = String::from("process,pid,cpu_ms,percent_of_core\n");
    for r in rows {
        let _ = writeln!(out, "{},{},{},{:.3}", r.label, r.pid, r.cpu_ms, r.percent);
    }
    out
}

/// The mean of `nvidia-smi --query-gpu=utilization.gpu,power.draw --format=csv,noheader,nounits`
/// lines: (samples, busy percent, watts). Lines it cannot read are skipped.
pub fn gpu_mean(text: &str) -> Option<(usize, f64, f64)> {
    let samples: Vec<(f64, f64)> = text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(',').map(str::trim);
            let busy = fields.next()?.parse().ok()?;
            let watts = fields.next()?.parse().ok()?;
            Some((busy, watts))
        })
        .collect();
    let n = samples.len();
    (n > 0).then(|| {
        let (busy, watts) = samples
            .iter()
            .fold((0.0, 0.0), |(b, w), (sb, sw)| (b + sb, w + sw));
        (n, busy / n as f64, watts / n as f64)
    })
}

/// A start-up stage, stamped once ([`mark`]) when `--measure startup` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The saved session and settings arrived from the store thread.
    StoreLoaded,
    /// The shared wgpu device exists.
    Gpu,
    /// Slint's backend is selected.
    SlintBackend,
    /// `Engine::start` returned (CEF initialised).
    CefStarted,
    /// The blocker was started (its cache loaded on this thread when it matches).
    BlockerStarted,
    /// The first window is built (not shown).
    WindowOpened,
    /// `show` returned.
    WindowShown,
    /// The session's other windows are open.
    SessionRestored,
    /// The first view (browser) was created.
    FirstView,
    /// Slint's first render.
    FirstRender,
    /// The host took CEF's first frame.
    FirstCefFrame,
    /// The first render after that frame: page pixels on screen.
    FirstPixels,
    /// The blocker's lists are in force (from the cache or compiled).
    BlockerReady,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::StoreLoaded => "store_loaded",
            Self::Gpu => "gpu_device",
            Self::SlintBackend => "slint_backend",
            Self::CefStarted => "cef_started",
            Self::BlockerStarted => "blocker_started",
            Self::WindowOpened => "window_opened",
            Self::WindowShown => "window_shown",
            Self::SessionRestored => "session_restored",
            Self::FirstView => "first_view",
            Self::FirstRender => "first_render",
            Self::FirstCefFrame => "first_cef_frame",
            Self::FirstPixels => "first_page_pixels",
            Self::BlockerReady => "blocker_ready",
        }
    }
}

/// The stages in time order, each with its time since `start` and since the stage before
/// (milliseconds); `premain` (process creation to `main`, if known) leads as its own row.
pub fn startup_rows(
    start: Instant,
    premain_ms: Option<f64>,
    stages: &[(Stage, Instant)],
) -> Vec<(&'static str, f64, f64)> {
    let mut sorted = stages.to_vec();
    sorted.sort_by_key(|(_, at)| *at);
    let mut rows = Vec::new();
    if let Some(p) = premain_ms {
        rows.push(("process_to_main", 0.0, p));
    }
    let mut last = start;
    for (stage, at) in sorted {
        rows.push((
            stage.name(),
            ms(at.saturating_duration_since(start)),
            ms(at.saturating_duration_since(last)),
        ));
        last = at;
    }
    rows
}

/// The start-up table and its CSV (`stage,since_main_ms,delta_ms`).
pub fn startup_text(rows: &[(&str, f64, f64)]) -> (String, String) {
    let mut text = format!(
        "{:<20} {:>10} {:>10}  (ms)\n",
        "stage", "since main", "delta"
    );
    let mut csv = String::from("stage,since_main_ms,delta_ms\n");
    for (name, t, d) in rows {
        let _ = writeln!(text, "{name:<20} {t:>10.1} {d:>10.1}");
        let _ = writeln!(csv, "{name},{t:.3},{d:.3}");
    }
    (text, csv)
}

/// Process creation to now from `/proc/self/stat`'s start time (field 22, clock ticks after
/// boot; the fork, not the exec: under `cargo run`, which execs the binary in its own process,
/// it holds cargo's run too, so run the binary itself, or through a runner that forks) and
/// `/proc/uptime`, milliseconds. The command name (field 2) may hold spaces and parentheses,
/// so the fields are counted after its last `)`.
#[cfg(any(target_os = "linux", test))]
pub fn since_exec_ms(stat: &str, uptime: &str, ticks_per_s: f64) -> Option<f64> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    // Field 3 is the first after the name: field 22 is the 20th from there.
    let start_ticks: f64 = after_name.split_whitespace().nth(19)?.parse().ok()?;
    let up: f64 = uptime.split_whitespace().next()?.parse().ok()?;
    let ms = (up - start_ticks / ticks_per_s) * 1000.0;
    (ms >= 0.0).then_some(ms)
}

/// When `main` started, set by [`process_started`].
static MAIN_START: OnceLock<(Instant, Option<f64>)> = OnceLock::new();
/// `--measure startup` is running: [`mark`] records.
static STARTUP: AtomicBool = AtomicBool::new(false);
static STAGES: Mutex<Vec<(Stage, Instant)>> = Mutex::new(Vec::new());

/// The first statement of `main`: the clock start-up stages count from, and (Linux) how long
/// the process ran before it (the dynamic loader mapping CEF).
pub fn process_started() {
    MAIN_START.get_or_init(|| {
        let now = Instant::now();
        #[cfg(target_os = "linux")]
        let premain = std::fs::read_to_string("/proc/self/stat")
            .ok()
            .zip(std::fs::read_to_string("/proc/uptime").ok())
            // USER_HZ: 100 on every Linux wubble runs on.
            .and_then(|(stat, up)| since_exec_ms(&stat, &up, 100.0));
        #[cfg(not(target_os = "linux"))]
        let premain = None;
        (now, premain)
    });
}

/// Stamps `stage` the first time it is reached, while `--measure startup` runs. Any thread.
pub fn mark(stage: Stage) {
    if stage == Stage::BlockerReady {
        crate::load_run::blocker_ready();
    }
    if !STARTUP.load(Ordering::Relaxed) {
        return;
    }
    let mut stages = STAGES.lock().unwrap_or_else(PoisonError::into_inner);
    if !stages.iter().any(|(s, _)| *s == stage) {
        stages.push((stage, Instant::now()));
    }
}

fn marked(stage: Stage) -> bool {
    STARTUP.load(Ordering::Relaxed)
        && STAGES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(s, _)| *s == stage)
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
    /// From the first frame to the start of the recording.
    warmup: Duration,
    recorder: Recorder,
    /// The first frame; recording starts at the first tick [`Scenario::warmup`] later.
    first_frame: Option<Instant>,
    /// The page measured is drawn by the shell (the start page), so no CEF frame comes: the
    /// first Slint render counts as the first frame.
    shell_page: bool,
    /// When recording started and whether it has ended: every count and the CPU times cover
    /// this one window, which the rates divide by.
    started: Option<Instant>,
    ended: bool,
    ticks: u32,
    /// The window's logical size when `resize` started.
    base: Option<(f32, f32)>,
    /// The display's refresh and the window, read when recording starts.
    refresh_mhz: Option<u32>,
    window: String,
    /// The process tree, found by a thread from the first frame on (a walk of every process
    /// on the system, kept off the UI thread), then the CPU times when recording started and
    /// ended (only the tree's own processes: [`wubble_sysmem::Meter::resample`]).
    finding: Option<std::thread::JoinHandle<wubble_sysmem::Meter>>,
    meter: Option<wubble_sysmem::Meter>,
    cpu_start: Option<CpuSnapshot>,
    cpu_end: Option<CpuSnapshot>,
    /// `nvidia-smi` sampling the GPU over the recording, then what it printed.
    gpu: Option<Child>,
    gpu_text: Option<String>,
    /// Partial rendering's counts when recording started, then over the recording.
    partial_start: Option<crate::partial::Totals>,
    partial: Option<crate::partial::Totals>,
}

impl Session {
    fn new(scenario: Scenario, seconds: u32, warmup: Option<Duration>) -> Self {
        Self {
            scenario,
            seconds,
            warmup: warmup.unwrap_or_else(|| scenario.warmup()),
            recorder: Recorder::default(),
            first_frame: None,
            shell_page: false,
            started: None,
            ended: false,
            ticks: 0,
            base: None,
            refresh_mhz: None,
            window: "n/a".into(),
            finding: None,
            meter: None,
            cpu_start: None,
            cpu_end: None,
            gpu: None,
            gpu_text: None,
            partial_start: None,
            partial: None,
        }
    }

    /// The warm-up after the first frame is over: recording may start.
    fn warm(&self, now: Instant) -> bool {
        self.first_frame.is_some_and(|t| now >= t + self.warmup)
    }

    fn recording(&self, now: Instant) -> bool {
        !self.ended && self.started.is_some_and(|t| now >= t)
    }

    fn finished(&self, now: Instant) -> bool {
        self.started
            .is_some_and(|t| now >= t + Duration::from_secs(self.seconds.into()))
    }

    /// The first frame arrived at `now`: the tree is looked for from here, on a thread.
    fn frame_arrived(&mut self, now: Instant) {
        if self.first_frame.is_some() {
            return;
        }
        self.first_frame = Some(now);
        if self.scenario != Scenario::Startup {
            self.finding = std::thread::Builder::new()
                .name("wubble-measure".into())
                .spawn(|| {
                    let mut meter = wubble_sysmem::Meter::default();
                    meter.sample();
                    meter
                })
                .ok();
        }
    }

    /// The tree's processes, then the browser process's threads (Linux); `None` without a
    /// meter.
    fn cpu_times(&mut self) -> Option<(Vec<CpuTime>, Vec<CpuTime>)> {
        let meter = self.meter.as_mut()?;
        let procs = meter
            .resample()
            .into_iter()
            .map(|p| (p.pid, p.label, p.cpu_ms))
            .collect();
        Some((procs, meter.threads()))
    }

    /// Recording starts at `now`: the CPU times and, where there is one, the GPU sampler.
    fn start(&mut self, now: Instant) {
        self.started = Some(now);
        if self.scenario == Scenario::Startup {
            return;
        }
        self.partial_start = Some(crate::partial::totals());
        // Long done: the warm-up is seconds, the walk tens of milliseconds.
        self.meter = self.finding.take().and_then(|t| t.join().ok());
        self.cpu_start = self.cpu_times().map(|times| (Instant::now(), times));
        self.gpu = gpu_sampler();
    }

    /// Recording ends (once).
    fn end_samples(&mut self) {
        if self.started.is_none() || self.ended {
            return;
        }
        self.ended = true;
        self.partial = self.partial_start.map(crate::partial::since);
        self.cpu_end = self.cpu_times().map(|times| (Instant::now(), times));
        if let Some(mut child) = self.gpu.take() {
            let _ = child.kill();
            self.gpu_text = child
                .wait_with_output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        }
    }
}

/// `nvidia-smi` printing the GPU's busy share and power every 500 ms (Linux only, if it is
/// installed). Measuring only: no network, killed when recording ends.
fn gpu_sampler() -> Option<Child> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,power.draw",
            "--format=csv,noheader,nounits",
            "-lms",
            "500",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

fn with_session<T>(f: impl FnOnce(&mut Session) -> T) -> Option<T> {
    SESSION.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Starts recording `scenario` for `seconds` once the first frame arrives.
pub fn begin(scenario: Scenario, seconds: u32, warmup: Option<Duration>) {
    SESSION.set(Some(Session::new(scenario, seconds, warmup)));
    STARTUP.store(scenario == Scenario::Startup, Ordering::Relaxed);
}

/// The page measured is one the shell draws (`--measure-page wubble://start`): no CEF frame
/// starts the session, the first render does.
pub fn shell_page() {
    with_session(|s| s.shell_page = true);
}

/// Counts `count` while a session records (a thread-local lookup and nothing else otherwise).
pub fn count(count: Count) {
    with_session(|s| {
        if s.recording(Instant::now()) {
            s.recorder.record(Event::Count(count));
        }
    });
}

/// Host hook: `frame` was set on the window. `probe` reads its probe pixel
/// and runs only when the scenario probes. Nothing happens when not
/// measuring.
pub fn taken(frame: Frame, probe: impl FnOnce() -> Option<[u8; 4]>) {
    crate::load_run::frame_taken();
    mark(Stage::FirstCefFrame);
    let Some(probes) = with_session(|s| {
        let now = Instant::now();
        s.frame_arrived(now);
        s.recording(now).then_some(s.scenario.probes())
    })
    .flatten() else {
        return;
    };
    let probe = probes.then(probe).flatten();
    let at = Instant::now();
    with_session(|s| s.recorder.record(Event::Taken { at, frame, probe }));
}

/// Rendering-notifier hook: Slint finished a render. `startup` ends at the first one after
/// the first page frame.
pub fn rendered() {
    crate::load_run::rendered();
    mark(Stage::FirstRender);
    if marked(Stage::FirstCefFrame) && !marked(Stage::FirstPixels) {
        mark(Stage::FirstPixels);
        crate::quit();
    }
    count(Count::SlintRender);
    with_session(|s| {
        let now = Instant::now();
        if s.shell_page {
            s.frame_arrived(now);
        }
        if s.recording(now) {
            s.recorder.record(Event::Rendered(now));
        }
    });
}

/// Rendering-notifier hook: Slint is about to render (`BeforeRendering`, stamped `at`) in
/// `window`; `tabs` notes whether this render draws the shell's switch animation.
pub fn render_began(at: Instant, window: &AppWindow) {
    record(at, Event::RenderBegan(at));
    if with_session(|s| s.scenario == Scenario::Tabs) == Some(true) {
        let switching = window.get_page_enter() < 1.0 || window.get_exit_t() < 1.0;
        record(at, Event::Switching(switching));
    }
}

/// Host hook: a run of CEF work started at `at` and ended now.
pub fn pumped(at: Instant) {
    let took = at.elapsed();
    record(at, Event::Pumped { at, took });
}

/// A stage of the frame path that started at `at` ended now.
pub fn stage(metric: Metric, at: Instant) {
    let took = at.elapsed();
    record(at, Event::Stage(metric, took));
}

/// Records `event` when a session is recording at `at`.
fn record(at: Instant, event: Event) {
    with_session(|s| {
        if s.recording(at) {
            s.recorder.record(event);
        }
    });
}

/// Panes hook: one shell frame laid out (observations only, never asserted).
pub fn shell(shell: Shell) {
    with_session(|s| {
        if s.recording(Instant::now()) {
            s.recorder.record(Event::Shell(shell));
        }
    });
}

/// Timer hook, every [`TICK`] while measuring: injects input (`typing`) or
/// resizes the window (`resize`), and quits once the run is over.
pub fn tick(host: &mut Host, window: &AppWindow) {
    if crate::load_run::active() {
        return crate::load_run::tick(host);
    }
    if with_session(|s| s.scenario) == Some(Scenario::Memory) {
        return host.memory_tick();
    }
    count(Count::MeasureTick);
    let now = Instant::now();
    // `tabs` opens its other tabs as soon as its first page has painted
    if with_session(|s| s.scenario == Scenario::Tabs && s.first_frame.is_some()) == Some(true) {
        host.measure_open_tabs(&Scenario::tab_pages().map(crate::page_url));
    }
    let Some((scenario, step, base)) = with_session(|s| {
        if s.finished(now) {
            s.end_samples();
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
        if !s.warm(now) {
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
            s.start(now);
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
        Scenario::Tabs if step.is_multiple_of(TICKS_PER_SWITCH) => {
            let at = Instant::now();
            host.measure_next_tab();
            with_session(|s| s.recorder.record(Event::Switched(at)));
        }
        Scenario::Typing if step.is_multiple_of(TICKS_PER_INPUT) => {
            let kind = if (step / TICKS_PER_INPUT).is_multiple_of(2) {
                InputKind::Key
            } else {
                InputKind::Pointer
            };
            inject(host, kind, step / TICKS_PER_INPUT);
        }
        _ => {}
    }
}

/// One key press and release, or one click, sent through the same `View`
/// calls real input uses; stamped just before sending.
/// `n` counts the injections.
fn inject(host: &mut Host, kind: InputKind, n: u32) {
    host.pace(PaceEvent::Wake);
    let now = host.input.now();
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
            let point = CLICK_AT[n as usize % CLICK_AT.len()];
            for pressed in [true, false] {
                let modifiers = Modifiers {
                    buttons: Buttons::default().with(MouseButton::Left, pressed),
                    ..Modifiers::default()
                };
                view.mouse_button(MouseButton::Left, pressed, point, &modifiers, now);
            }
        }
    }
    with_session(|s| s.recorder.record(Event::Injected { at, kind }));
}

/// Ends the run: the report text and the CSV, `None` if none was started.
/// `adapter` names the GPU for the header.
/// A run in which no frame arrived recorded nothing and is `None` too.
pub fn finish(adapter: &str) -> Option<(String, String)> {
    if let Some(load) = crate::load_run::finish() {
        return Some(load);
    }
    let session = SESSION.take()?;
    if session.scenario == Scenario::Memory {
        return crate::memory::finish();
    }
    let mut session = Some(session).filter(|s| s.first_frame.is_some())?;
    if session.scenario == Scenario::Startup {
        return Some(finish_startup(adapter));
    }
    // A run quit early (the window closed) still reports its window so far.
    session.end_samples();
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
        "measure: scenario={} seconds={} pacing={} refresh={refresh} window={} adapter={}\nmeasure: loadavg {}\n",
        session.scenario.name(),
        session.seconds,
        crate::pacing().name(),
        session.window,
        adapter,
        load.trim()
    );
    let cpu = session
        .cpu_start
        .as_ref()
        .zip(session.cpu_end.as_ref())
        .map(|((t0, start), (t1, end))| {
            let wall = t1.saturating_duration_since(*t0);
            let mut threads = cpu_rows(&start.1, &end.1, wall);
            threads.retain(|t| t.cpu_ms > 0);
            threads.sort_by_key(|t| std::cmp::Reverse(t.cpu_ms));
            (wall, cpu_rows(&start.0, &end.0, wall), threads)
        });
    let seconds = cpu
        .as_ref()
        .map_or(f64::from(session.seconds), |(wall, ..)| wall.as_secs_f64());
    let mut report = header + &session.recorder.report(period, seconds);
    let mut csv = session.recorder.csv();
    if let Some((wall, rows, threads)) = &cpu {
        report.push('\n');
        report.push_str(&cpu_text(rows, *wall));
        if !threads.is_empty() {
            report.push_str("\nbrowser process threads that ran:\n");
            report.push_str(&cpu_text(threads, *wall));
        }
        csv.push('\n');
        csv.push_str(&cpu_csv(rows));
    }
    let gpu = match session.gpu_text.as_deref().map(gpu_mean) {
        Some(Some((n, busy, watts))) => {
            format!(
                "gpu: {busy:.1} % busy, {watts:.1} W (mean of {n} nvidia-smi samples, whole GPU)"
            )
        }
        Some(None) => "gpu: nvidia-smi printed nothing readable".into(),
        None => "gpu: n/a (no nvidia-smi)".into(),
    };
    let _ = writeln!(report, "{gpu}");
    if let Some(partial) = session.partial {
        let _ = writeln!(report, "{}", crate::partial::report(partial));
    }
    Some((report, csv))
}

/// `startup`'s report: the stage table, its CSV.
fn finish_startup(adapter: &str) -> (String, String) {
    let (start, premain) = MAIN_START
        .get()
        .copied()
        .unwrap_or_else(|| (Instant::now(), None));
    let stages = STAGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let (text, csv) = startup_text(&startup_rows(start, premain, &stages));
    let missing: Vec<&str> = [Stage::BlockerReady, Stage::FirstPixels]
        .into_iter()
        .filter(|s| !stages.iter().any(|(m, _)| m == s))
        .map(Stage::name)
        .collect();
    let mut report = format!("measure: scenario=startup adapter={adapter}\n{text}");
    if !missing.is_empty() {
        let _ = writeln!(
            report,
            "not reached before the run ended: {}",
            missing.join(", ")
        );
    }
    (report, csv)
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
        assert_eq!(r.values(Metric::TakenInterval), vec![9.0, 3.0]);
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
    fn stages_pumps_and_render_spans_from_a_fixed_sequence() {
        let t0 = Instant::now();
        let mut r = Recorder::default();
        for e in [
            Event::Pumped {
                at: at(t0, 0),
                took: Duration::from_millis(2),
            },
            Event::RenderBegan(at(t0, 3)),
            Event::Stage(Metric::Scene, Duration::from_millis(1)),
            Event::Rendered(at(t0, 6)),
            Event::Pumped {
                at: at(t0, 5),
                took: Duration::from_millis(1),
            },
            // A render without its start adds no render span.
            Event::Rendered(at(t0, 14)),
        ] {
            r.record(e);
        }
        assert_eq!(r.values(Metric::PumpWork), vec![2.0, 1.0]);
        assert_eq!(r.values(Metric::PumpGap), vec![5.0]);
        assert_eq!(r.values(Metric::Scene), vec![1.0]);
        assert_eq!(r.values(Metric::SlintRender), vec![3.0]);
    }

    #[test]
    fn a_switch_is_timed_to_the_render_of_its_first_frame() {
        let t0 = Instant::now();
        let mut r = Recorder::default();
        let taken = |ms| Event::Taken {
            at: at(t0, ms),
            frame: frame(7, Some(at(t0, ms)), 10, 10),
            probe: None,
        };
        for e in [
            Event::Rendered(at(t0, 0)),
            Event::Switched(at(t0, 2)),
            // the animation's renders before the new tab's frame
            Event::Switching(true),
            Event::Rendered(at(t0, 16)),
            taken(20),
            Event::Rendered(at(t0, 33)),
            Event::Switching(false),
            Event::Rendered(at(t0, 50)),
            // a later frame is not the switch's
            taken(60),
            Event::Rendered(at(t0, 66)),
        ] {
            r.record(e);
        }
        assert_eq!(r.values(Metric::SwitchToFrame), vec![31.0]);
        assert_eq!(r.values(Metric::SwitchInterval), vec![16.0, 17.0]);
        assert_eq!(r.values(Metric::PresentInterval).len(), 4);
    }

    #[test]
    fn counts_and_their_rates() {
        let mut r = Recorder::default();
        for c in [Count::Pump, Count::Pump, Count::MediaPoll, Count::Pump] {
            r.record(Event::Count(c));
        }
        assert_eq!(r.counted(Count::Pump), 3);
        assert_eq!(r.counted(Count::CefFrame), 0);
        let text = r.counts_text(2.0);
        assert!(
            text.contains("pump_runs                     3      1.50\n"),
            "{text}"
        );
        assert!(
            text.contains("media_polls                   1      0.50\n"),
            "{text}"
        );
        // no time, no rate
        assert!(
            r.counts_text(0.0)
                .contains("pump_runs                     3      0.00")
        );
    }

    #[test]
    fn cpu_rows_are_the_time_used_over_the_window() {
        let start = vec![(1, "Browser".into(), 1_000), (2, "GPU".into(), 500)];
        let end = vec![
            (1, "Browser".into(), 1_050),
            (2, "GPU".into(), 500),
            // started meanwhile: all of its time counts
            (3, "Renderer".into(), 20),
        ];
        let rows = cpu_rows(&start, &end, Duration::from_secs(10));
        let got: Vec<(u32, u64, f64)> = rows.iter().map(|r| (r.pid, r.cpu_ms, r.percent)).collect();
        assert_eq!(got, [(1, 50, 0.5), (2, 0, 0.0), (3, 20, 0.2)]);
        let text = cpu_text(&rows, Duration::from_secs(10));
        assert!(
            text.ends_with("total                        70       0.70\n"),
            "{text}"
        );
        assert_eq!(
            cpu_csv(&rows[..1]),
            "process,pid,cpu_ms,percent_of_core\nBrowser,1,50,0.500\n"
        );
        // a zero window gives no percentages, not a division by zero
        assert_eq!(cpu_rows(&start, &end, Duration::ZERO)[0].percent, 0.0);
    }

    #[test]
    fn gpu_mean_of_nvidia_smi_lines() {
        assert_eq!(gpu_mean(""), None);
        assert_eq!(gpu_mean("garbage\n"), None);
        assert_eq!(
            gpu_mean("10, 30.00\n20, 34.00\n[N/A], 1\n"),
            Some((2, 15.0, 32.0))
        );
    }

    #[test]
    fn startup_stages_in_time_order_with_deltas() {
        let t0 = Instant::now();
        let stages = [
            (Stage::CefStarted, at(t0, 120)),
            (Stage::StoreLoaded, at(t0, 20)),
            (Stage::FirstPixels, at(t0, 400)),
        ];
        let rows = startup_rows(t0, Some(35.0), &stages);
        assert_eq!(
            rows,
            [
                ("process_to_main", 0.0, 35.0),
                ("store_loaded", 20.0, 20.0),
                ("cef_started", 120.0, 100.0),
                ("first_page_pixels", 400.0, 280.0),
            ]
        );
        let (text, csv) = startup_text(&rows[1..2]);
        assert_eq!(
            text,
            "stage                since main      delta  (ms)\nstore_loaded               20.0       20.0\n"
        );
        assert_eq!(
            csv,
            "stage,since_main_ms,delta_ms\nstore_loaded,20.000,20.000\n"
        );
    }

    #[test]
    fn exec_time_counts_fields_after_the_last_parenthesis() {
        // the command name holds a space and a ')': fields are counted after the last ')'
        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        // field 22: start time, 500 ticks after boot
        fields[19] = "500";
        let stat = format!("1234 (we ird) name) {}", fields.join(" "));
        assert_eq!(since_exec_ms(&stat, "5.25 99.0\n", 100.0), Some(250.0));
        // a start in the future (clock skew) or junk gives nothing
        assert_eq!(since_exec_ms(&stat, "4.0 1.0", 100.0), None);
        assert_eq!(since_exec_ms("no parens", "5.0 1.0", 100.0), None);
    }

    #[test]
    fn the_idle_scenario_warms_up_longer_than_the_others() {
        // a window opening under the pointer gets one move: the recording must start still
        const { assert!(IDLE_WARMUP.as_millis() > WARMUP.as_millis()) };
        assert_eq!(Scenario::Idle.warmup(), IDLE_WARMUP);
        assert_eq!(Scenario::Scroll.warmup(), WARMUP);
        assert!(Scenario::Idle.host_timers() && !Scenario::Scroll.host_timers());
    }

    #[test]
    fn a_run_without_frames_reports_nothing() {
        begin(Scenario::Scroll, 1, None);
        assert_eq!(finish("test"), None);
        begin(Scenario::Scroll, 1, None);
        with_session(|s| s.first_frame = Some(Instant::now()));
        assert!(finish("test").is_some());
    }

    #[test]
    fn counts_and_cpu_times_cover_one_window() {
        // Startup takes no CPU samples (no thread, no meter): the window logic alone.
        let mut s = Session::new(Scenario::Startup, 10, None);
        let t0 = Instant::now();
        s.frame_arrived(t0);
        let warm = t0 + Scenario::Startup.warmup();
        // Warm, but the tick that starts recording (and samples the CPU) has not run yet.
        assert!(s.warm(warm) && !s.recording(warm) && !s.finished(warm));
        let tick = warm + IDLE_TICK;
        s.start(tick);
        assert!(!s.recording(warm) && s.recording(tick));
        // The run is as long as asked from the start, not from the warm-up's end.
        let secs = Duration::from_secs;
        assert!(!s.finished(tick + secs(9)) && s.finished(tick + secs(10)));
        s.end_samples();
        assert!(!s.recording(tick + secs(1)));
    }

    #[test]
    fn every_injected_click_is_a_single_click() {
        let mut clicks = wubble_engine::input::ClickTracker::default();
        // The same instant for every press: only the alternating point keeps
        // each one out of the previous click's series.
        for n in 0..6 {
            let at = CLICK_AT[n % CLICK_AT.len()];
            assert_eq!(clicks.press(Duration::ZERO, at, MouseButton::Left), 1);
        }
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
