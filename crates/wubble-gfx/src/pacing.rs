//! Frame pacing inputs and frame statistics. Pure: no clock is read here.
//!
//! Two pacings ([`Pacing`]): CEF paints on its own timer at `windowless_frame_rate`, which
//! wubble sets to the display refresh rate, or the shell paces it with CEF's external begin
//! frames, one per Slint render ([`BeginFrames`]). Which one a platform ships is
//! [`PLATFORM_PACING`] (decisions.md "Frame pacing"); `--begin-frames` and `--no-begin-frames`
//! override it for a run.

use std::time::{Duration, Instant};

/// CEF frame rate when the display does not report a refresh rate.
pub const DEFAULT_FRAME_RATE: u32 = 60;

/// Upper bound for the requested CEF frame rate. CEF documents only the
/// minimum (1); this is our sanity cap against bogus monitor reports.
pub const MAX_FRAME_RATE: u32 = 240;

/// CEF `windowless_frame_rate` for a display refresh rate in millihertz
/// (winit's unit), rounded to the nearest hertz. `None` or 0 means unknown.
pub fn frame_rate_for(refresh_mhz: Option<u32>) -> u32 {
    match refresh_mhz {
        None | Some(0) => DEFAULT_FRAME_RATE,
        Some(mhz) => (mhz.saturating_add(500) / 1000).clamp(1, MAX_FRAME_RATE),
    }
}

/// The frame-rate ceiling of a page no window shows but that must keep painting (an agent
/// session's: its layout, accessibility tree and screenshots), so it is not drawn at the
/// display's rate for nobody.
pub const UNSHOWN_FRAME_RATE: u32 = 15;

/// CEF's frame-rate ceiling for one view: the display's rate where a window shows it, else
/// [`UNSHOWN_FRAME_RATE`] (never above the display's).
pub fn view_frame_rate(display: u32, shown: bool) -> u32 {
    if shown {
        display
    } else {
        display.min(UNSHOWN_FRAME_RATE)
    }
}

/// The frame-rate ceiling of a page whose frames change little (a spinner, a caret, a small
/// CSS animation): nobody sees 240 Hz in 16 px, and every page frame is a frame of the shell.
pub const CALM_FRAME_RATE: u32 = 60;

/// How long input or a large change keeps a page at its full ceiling.
pub const LIVELY_HOLD: Duration = Duration::from_millis(300);

/// A frame is large when its damage covers at least one part in this many of it. A video
/// in a page and a scroll are large; a spinner and a caret are not.
pub const LARGE_DAMAGE_PART: u64 = 20;

/// Whether a frame of `frame` (width, height) with `damage` (width, height) of it changed is
/// large. `None`: CEF did not say what changed, so maybe all of it.
pub fn large_damage(damage: Option<(u32, u32)>, frame: (u32, u32)) -> bool {
    let area = |(w, h): (u32, u32)| u64::from(w) * u64::from(h);
    damage.is_none_or(|d| area(d) * LARGE_DAMAGE_PART >= area(frame))
}

/// How lively a page's content is: what its frame-rate ceiling follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentRate {
    /// The full ceiling, until `until` (`None`: a new view, calm from its first small frame).
    Lively { until: Option<Instant> },
    /// Small changes only and no input for [`LIVELY_HOLD`]: [`CALM_FRAME_RATE`].
    Calm,
}

impl Default for ContentRate {
    fn default() -> Self {
        Self::Lively { until: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentEvent {
    /// The page got input (pointer, wheel, key).
    Input,
    /// The host took a frame; `large`: [`large_damage`].
    Frame { large: bool },
}

impl ContentRate {
    /// The state after `event` at `now`. Input and a large frame start the hold again; a
    /// small frame ends a hold that has run out. Nothing else does: a page that stops
    /// painting keeps its state until its next frame.
    pub fn after(self, event: ContentEvent, now: Instant) -> Self {
        match (self, event) {
            (_, ContentEvent::Input | ContentEvent::Frame { large: true }) => Self::Lively {
                until: Some(now + LIVELY_HOLD),
            },
            (Self::Lively { until }, ContentEvent::Frame { large: false })
                if until.is_some_and(|until| now < until) =>
            {
                self
            }
            (_, ContentEvent::Frame { large: false }) => Self::Calm,
        }
    }

    /// CEF's frame rate under `ceiling` (the display's rate, or [`view_frame_rate`]'s).
    pub fn frame_rate(self, ceiling: u32) -> u32 {
        match self {
            Self::Lively { .. } => ceiling,
            Self::Calm => ceiling.min(CALM_FRAME_RATE),
        }
    }
}

/// `Pacing::Shell`: whether the `tick`th begin frame the shell offers a view at `ceiling` (its
/// window's rate) goes to CEF, for a page whose content wants `rate`
/// ([`ContentRate::frame_rate`]): every `ceiling / rate`th, as CEF's timer would pace it (CEF's
/// `windowless_frame_rate` does not apply to external begin frames).
pub fn begin_frame_due(ceiling: u32, rate: u32, tick: u32) -> bool {
    tick.is_multiple_of((ceiling / rate.max(1)).max(1))
}

/// Who decides when CEF paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pacing {
    /// CEF's own timer, at the view's `windowless_frame_rate`.
    CefTimer,
    /// External begin frames from the shell: one per render of the window showing the page.
    Shell,
}

impl Pacing {
    /// How `--measure` names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::CefTimer => "cef-timer",
            Self::Shell => "shell",
        }
    }
}

/// The pacing each platform ships, by `std::env::consts::OS`: with the shell drawing its pages,
/// and with them on their own layer under it (`wubble_core::page_layer`, `Order::Below`). macOS
/// composited: the shell's (the owner's Mac, measurements.md "frame runs in Low Power Mode": 58
/// to 59 page frames a second against 35 to 56 on CEF's timer). macOS on the layer: CEF's own
/// begin-frame source, since no shell render paces a page there and the shell's fallback timer
/// would be the clock (one main-thread timer per frame, not locked to the display); elsewhere
/// CEF's timer, which measured as well or better on Linux. CEF fixes a view's source when it
/// opens, so a run keeps one.
#[rustfmt::skip]
pub const PLATFORM_PACING: &[(&str, Pacing, Pacing)] = &[
    // (os, composited, on the layer)
    ("macos", Pacing::Shell, Pacing::CefTimer),
    ("linux", Pacing::CefTimer, Pacing::CefTimer),
    ("windows", Pacing::CefTimer, Pacing::CefTimer),
];

/// The shipped pacing of platform `os`, its pages on their layer (`layered`) or not (CEF's timer
/// for a platform the table does not name).
pub fn platform_pacing(os: &str, layered: bool) -> Pacing {
    PLATFORM_PACING
        .iter()
        .find_map(|&(name, composited, on_layer)| {
            (name == os).then_some(if layered { on_layer } else { composited })
        })
        .unwrap_or(Pacing::CefTimer)
}

/// Unanswered begin frames after which the driver idles.
pub const IDLE_AFTER: u8 = 4;

/// Begin-frame rate while idle, so a page that starts animating by itself
/// is picked up within this many milliseconds (the pump's `MAX_TIMER_DELAY`).
pub const IDLE_INTERVAL: Duration = Duration::from_millis(33);

/// The external begin-frame driver for the visible view. One begin frame
/// per Slint render, one outstanding at a time (cefclient's single chain,
/// `osr_render_handler_win.cc` `begin_frame_pending_`), a timer one refresh
/// period long as the fallback clock when Slint has nothing to render, and a
/// slow timer once the page stops answering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BeginFrames {
    /// Not started: no render of the window and no wake yet.
    #[default]
    Off,
    /// Begin frames sent since the last frame arrived.
    Active { unanswered: u8 },
    /// The page painted nothing for [`IDLE_AFTER`] begin frames.
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaceEvent {
    /// Slint finished a render (with FIFO present, once per displayed frame).
    Vsync,
    /// The driver's timer fired.
    Timer,
    /// The host took a frame from CEF.
    FrameArrived,
    /// Input, a layout change or a newly shown view: paint soon.
    Wake,
    /// The last [`PaceEffect::Send`] reached no page: each took only its share
    /// ([`begin_frame_due`]), so it is not a begin frame left unanswered.
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaceEffect {
    /// `SendExternalBeginFrame` to the visible view.
    Send,
    /// (Re)start the single-shot timer.
    ArmTimer(Duration),
}

/// `period` is the display's refresh period.
pub fn transition(
    state: BeginFrames,
    event: PaceEvent,
    period: Duration,
) -> (BeginFrames, Vec<PaceEffect>) {
    use {BeginFrames::*, PaceEffect::*, PaceEvent::*};
    let send = |unanswered: u8| {
        if unanswered >= IDLE_AFTER {
            (Idle, vec![Send, ArmTimer(IDLE_INTERVAL)])
        } else {
            (Active { unanswered }, vec![Send, ArmTimer(period)])
        }
    };
    match (state, event) {
        // A window's first render starts its driver: its pages may have opened before the
        // window existed, when no Wake could reach it.
        (Off | Idle, Wake)
        | (Off | Active { unanswered: 0 }, Vsync)
        | (Active { unanswered: 0 }, Wake) => send(1),
        (Active { unanswered }, Timer) => send(unanswered.saturating_add(1)),
        (Idle, Timer) => (Idle, vec![Send, ArmTimer(IDLE_INTERVAL)]),
        (Active { .. } | Idle, FrameArrived) => (Active { unanswered: 0 }, vec![]),
        (Active { unanswered }, Skipped) => (
            Active {
                unanswered: unanswered.saturating_sub(1),
            },
            vec![],
        ),
        _ => (state, vec![]),
    }
}

/// Counts of frames, fed by the host loop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Zero-copy frames (GPU texture import) set on the window.
    pub accelerated: u64,
    /// CPU `OnPaint` frames CEF sent instead of a shared texture (never
    /// shown). `None` while the producer does not count them (not Linux).
    pub cpu_fallback: Option<u64>,
    interval_sum: Duration,
    intervals: u64,
}

impl FrameStats {
    /// One accelerated frame was presented; `interval` is the time since the
    /// previous presented frame (`None` for the first).
    pub fn record_accelerated(&mut self, interval: Option<Duration>) {
        self.accelerated += 1;
        if let Some(interval) = interval {
            self.interval_sum += interval;
            self.intervals += 1;
        }
    }

    /// The producer's running CPU fallback total. Returns how many fallback
    /// frames are new since the last call, for the host to log.
    pub fn record_cpu_fallback(&mut self, total: Option<u64>) -> u64 {
        let new = match (total, self.cpu_fallback) {
            (Some(total), seen) => total.saturating_sub(seen.unwrap_or(0)),
            (None, _) => 0,
        };
        self.cpu_fallback = total;
        new
    }

    /// Mean time between presented frames, 0 when fewer than two frames.
    pub fn avg_interval_ms(&self) -> f64 {
        if self.intervals == 0 {
            return 0.0;
        }
        self.interval_sum.as_secs_f64() * 1000.0 / self.intervals as f64
    }

    /// The `--smoke frames` result line. Every presented frame is
    /// accelerated; `cpu_fallback` is the measured count, `n/a` where it is
    /// not counted. The interval is informational only.
    pub fn summary_line(&self) -> String {
        let cpu = self
            .cpu_fallback
            .map_or_else(|| "n/a".to_owned(), |n| n.to_string());
        format!(
            "smoke: frames={0} accelerated={0} cpu_fallback={cpu} avg_interval_ms={1:.1}",
            self.accelerated,
            self.avg_interval_ms()
        )
    }

    /// The frame path produced real zero-copy frames.
    pub fn is_success(&self) -> bool {
        self.accelerated > 0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_page_no_window_shows_paints_slowly() {
        assert_eq!(super::view_frame_rate(120, true), 120);
        assert_eq!(
            super::view_frame_rate(120, false),
            super::UNSHOWN_FRAME_RATE
        );
        assert_eq!(super::view_frame_rate(10, false), 10);
    }

    use super::*;

    #[test]
    fn a_calm_page_takes_a_share_of_the_shells_begin_frames() {
        // (ceiling, rate, the ticks 0..6 that go to CEF)
        for (ceiling, rate, due) in [
            (60, 60, [true; 6]),
            (120, 120, [true; 6]),
            (120, 60, [true, false, true, false, true, false]),
            (240, 60, [true, false, false, false, true, false]),
            (90, 60, [true; 6]),
            (15, 15, [true; 6]),
        ] {
            let got: Vec<bool> = (0..6).map(|t| begin_frame_due(ceiling, rate, t)).collect();
            assert_eq!(got, due, "{ceiling} {rate}");
        }
    }

    #[test]
    fn a_frame_is_large_from_a_twentieth_of_its_area() {
        let frame = (1000, 800);
        for (name, damage, large) in [
            ("no rect: maybe all of it", None, true),
            ("the whole frame", Some(frame), true),
            ("a video in the page", Some((854, 480)), true),
            ("exactly a twentieth", Some((200, 200)), true),
            ("just under a twentieth", Some((200, 199)), false),
            ("a spinner", Some((16, 16)), false),
            ("a caret", Some((2, 20)), false),
            ("nothing changed", Some((0, 0)), false),
        ] {
            assert_eq!(large_damage(damage, frame), large, "{name}");
        }
        assert!(large_damage(Some((0, 0)), (0, 0)), "an empty frame");
        assert!(
            !large_damage(Some((16, 16)), (u32::MAX, u32::MAX)),
            "no overflow"
        );
    }

    #[test]
    fn content_rate_follows_input_and_damage() {
        use {ContentEvent::*, ContentRate::*};
        let t = Instant::now();
        let held = Lively {
            until: Some(t + LIVELY_HOLD),
        };
        let ms = Duration::from_millis;
        const SMALL: ContentEvent = Frame { large: false };
        const LARGE: ContentEvent = Frame { large: true };
        // (name, state, event, time since t, next): one row per state and event
        for (name, state, event, at, next) in [
            (
                "a new view's large frame holds",
                ContentRate::default(),
                LARGE,
                ms(0),
                held,
            ),
            (
                "a new view's small frame calms it",
                ContentRate::default(),
                SMALL,
                ms(0),
                Calm,
            ),
            (
                "a new view's input holds",
                ContentRate::default(),
                Input,
                ms(0),
                held,
            ),
            (
                "a small frame inside the hold keeps it",
                held,
                SMALL,
                ms(299),
                held,
            ),
            (
                "a small frame at the hold's end calms",
                held,
                SMALL,
                ms(300),
                Calm,
            ),
            (
                "a small frame long after calms",
                held,
                SMALL,
                ms(60_000),
                Calm,
            ),
            (
                "a large frame starts the hold again",
                held,
                LARGE,
                ms(200),
                Lively {
                    until: Some(t + ms(500)),
                },
            ),
            (
                "input starts the hold again",
                held,
                Input,
                ms(200),
                Lively {
                    until: Some(t + ms(500)),
                },
            ),
            (
                "a calm page stays calm on small frames",
                Calm,
                SMALL,
                ms(0),
                Calm,
            ),
            ("a large frame wakes a calm page", Calm, LARGE, ms(0), held),
            ("input wakes a calm page", Calm, Input, ms(0), held),
        ] {
            assert_eq!(state.after(event, t + at), next, "{name}");
        }
    }

    #[test]
    fn a_calm_page_paints_at_sixty_at_most() {
        for (ceiling, lively, calm) in [
            (240, 240, CALM_FRAME_RATE),
            (120, 120, CALM_FRAME_RATE),
            (60, 60, 60),
            (UNSHOWN_FRAME_RATE, UNSHOWN_FRAME_RATE, UNSHOWN_FRAME_RATE),
        ] {
            assert_eq!(ContentRate::default().frame_rate(ceiling), lively);
            assert_eq!(ContentRate::Calm.frame_rate(ceiling), calm);
        }
    }

    #[test]
    fn the_mac_ships_shell_pacing_and_the_rest_cefs_timer() {
        for (os, layered, pacing) in [
            ("macos", false, Pacing::Shell),
            ("macos", true, Pacing::CefTimer),
            ("linux", false, Pacing::CefTimer),
            ("linux", true, Pacing::CefTimer),
            ("windows", false, Pacing::CefTimer),
            ("freebsd", false, Pacing::CefTimer),
            ("freebsd", true, Pacing::CefTimer),
        ] {
            assert_eq!(platform_pacing(os, layered), pacing, "{os} {layered}");
        }
    }

    #[test]
    fn frame_rate_follows_the_display() {
        assert_eq!(frame_rate_for(None), DEFAULT_FRAME_RATE);
        assert_eq!(frame_rate_for(Some(0)), DEFAULT_FRAME_RATE);
        assert_eq!(frame_rate_for(Some(1)), 1);
        assert_eq!(frame_rate_for(Some(59_940)), 60);
        assert_eq!(frame_rate_for(Some(60_000)), 60);
        assert_eq!(frame_rate_for(Some(60_001)), 60);
        assert_eq!(frame_rate_for(Some(143_981)), 144);
        assert_eq!(frame_rate_for(Some(1_000_000)), MAX_FRAME_RATE);
        assert_eq!(frame_rate_for(Some(u32::MAX)), MAX_FRAME_RATE);
    }

    #[test]
    fn begin_frames_keep_one_chain_and_idle_when_unanswered() {
        use {BeginFrames::*, PaceEffect::*, PaceEvent::*};
        let p = Duration::from_micros(8_333);
        let fast = vec![Send, ArmTimer(p)];
        let slow = vec![Send, ArmTimer(IDLE_INTERVAL)];
        let cases = [
            // Nothing runs before the window's first render or wake.
            (Off, Vsync, Active { unanswered: 1 }, fast.clone()),
            (Off, Timer, Off, vec![]),
            (Off, FrameArrived, Off, vec![]),
            (Off, Wake, Active { unanswered: 1 }, fast.clone()),
            // One outstanding: a render sends only once the frame came.
            (
                Active { unanswered: 0 },
                Vsync,
                Active { unanswered: 1 },
                fast.clone(),
            ),
            (
                Active { unanswered: 1 },
                Vsync,
                Active { unanswered: 1 },
                vec![],
            ),
            (
                Active { unanswered: 1 },
                Wake,
                Active { unanswered: 1 },
                vec![],
            ),
            (
                Active { unanswered: 2 },
                FrameArrived,
                Active { unanswered: 0 },
                vec![],
            ),
            // The timer is the clock when nothing renders.
            (
                Active { unanswered: 1 },
                Timer,
                Active { unanswered: 2 },
                fast.clone(),
            ),
            (Active { unanswered: 3 }, Timer, Idle, slow.clone()),
            (Idle, Timer, Idle, slow.clone()),
            (Idle, Vsync, Idle, vec![]),
            (Idle, Wake, Active { unanswered: 1 }, fast.clone()),
            (Idle, FrameArrived, Active { unanswered: 0 }, vec![]),
            // A send no page took does not count toward idling.
            (
                Active { unanswered: 2 },
                Skipped,
                Active { unanswered: 1 },
                vec![],
            ),
            (
                Active { unanswered: 0 },
                Skipped,
                Active { unanswered: 0 },
                vec![],
            ),
            (Idle, Skipped, Idle, vec![]),
            (Off, Skipped, Off, vec![]),
        ];
        for (state, event, want, effects) in cases {
            assert_eq!(
                transition(state, event, p),
                (want, effects),
                "{state:?} {event:?}"
            );
        }
    }

    /// A page on its OS layer (`wubble_core::page_layer`) costs no shell render, so no `Vsync`
    /// comes: the timer every `Send` arms is the clock, one begin frame per period while the
    /// page answers, the idle rate once it stops, and back on its next frame.
    #[test]
    fn page_layer_route_pacing_has_no_vsync() {
        use {BeginFrames::*, PaceEffect::*, PaceEvent::*};
        let p = Duration::from_micros(8_333);
        let mut state = Active { unanswered: 0 };
        for step in 0..10 {
            let (next, effects) = transition(state, Timer, p);
            assert_eq!(
                (next, effects),
                (Active { unanswered: 1 }, vec![Send, ArmTimer(p)]),
                "{step}"
            );
            state = transition(next, FrameArrived, p).0;
            assert_eq!(state, Active { unanswered: 0 }, "{step}");
        }
        for _ in 0..IDLE_AFTER {
            state = transition(state, Timer, p).0;
        }
        assert_eq!(state, Idle);
        assert_eq!(
            transition(Idle, Timer, p),
            (Idle, vec![Send, ArmTimer(IDLE_INTERVAL)])
        );
        state = transition(Idle, FrameArrived, p).0;
        assert_eq!(transition(state, Timer, p).1, vec![Send, ArmTimer(p)]);
    }

    #[test]
    fn a_page_that_stops_painting_idles_after_the_limit() {
        let p = Duration::from_millis(4);
        let mut state = transition(BeginFrames::Off, PaceEvent::Wake, p).0;
        for _ in 1..IDLE_AFTER {
            assert!(matches!(state, BeginFrames::Active { .. }));
            state = transition(state, PaceEvent::Timer, p).0;
        }
        assert_eq!(state, BeginFrames::Idle);
    }

    #[test]
    fn stats_count_and_average_fixed_intervals() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.avg_interval_ms(), 0.0);
        assert!(!stats.is_success());
        stats.record_accelerated(None);
        assert_eq!(stats.avg_interval_ms(), 0.0);
        stats.record_accelerated(Some(Duration::from_millis(8)));
        stats.record_accelerated(Some(Duration::from_millis(10)));
        assert_eq!(stats.accelerated, 3);
        assert_eq!(stats.avg_interval_ms(), 9.0);
        assert!(stats.is_success());
        assert_eq!(
            stats.summary_line(),
            "smoke: frames=3 accelerated=3 cpu_fallback=n/a avg_interval_ms=9.0"
        );
    }

    #[test]
    fn cpu_fallback_is_the_measured_total() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.record_cpu_fallback(Some(0)), 0);
        assert_eq!(stats.record_cpu_fallback(Some(2)), 2);
        assert_eq!(stats.record_cpu_fallback(Some(2)), 0);
        stats.record_accelerated(None);
        assert_eq!(
            stats.summary_line(),
            "smoke: frames=1 accelerated=1 cpu_fallback=2 avg_interval_ms=0.0"
        );
        // Fallback frames never count as success.
        assert!(stats.is_success());
        assert!(
            !FrameStats {
                cpu_fallback: Some(5),
                ..FrameStats::default()
            }
            .is_success()
        );
    }

    #[test]
    fn uncounted_cpu_fallback_reads_not_available() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.record_cpu_fallback(None), 0);
        assert_eq!(stats.cpu_fallback, None);
        assert!(stats.summary_line().contains("cpu_fallback=n/a "));
    }
}
