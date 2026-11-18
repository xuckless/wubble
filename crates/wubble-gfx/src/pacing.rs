//! Frame pacing inputs and frame statistics. Pure: no clock is read here.
//!
//! Default pacing: CEF paints on its own timer at `windowless_frame_rate`,
//! which wubble sets to the display refresh rate. [`BeginFrames`] is the
//! alternative, CEF's external begin frames driven by Slint's renders; it is
//! opt-in (`--begin-frames`) because it measured worse here (TODO.md
//! "Measured").

use std::time::Duration;

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
    /// No view to drive yet.
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
        (Off | Idle, Wake) | (Active { unanswered: 0 }, Vsync | Wake) => send(1),
        (Active { unanswered }, Timer) => send(unanswered.saturating_add(1)),
        (Idle, Timer) => (Idle, vec![Send, ArmTimer(IDLE_INTERVAL)]),
        (Active { .. } | Idle, FrameArrived) => (Active { unanswered: 0 }, vec![]),
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
    use super::*;

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
            // Nothing runs before the first view.
            (Off, Vsync, Off, vec![]),
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
        ];
        for (state, event, want, effects) in cases {
            assert_eq!(
                transition(state, event, p),
                (want, effects),
                "{state:?} {event:?}"
            );
        }
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
