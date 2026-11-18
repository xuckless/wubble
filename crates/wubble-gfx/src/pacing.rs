//! Frame pacing inputs and frame statistics. Pure: no clock is read here.
//!
//! Interim pacing (M0 first frame): CEF paints on its own timer at
//! `windowless_frame_rate`, which wubble sets to the display refresh rate.
//! Compositor-driven begin-frames need a welding fork (TODO.md, "Frame pacing").

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

/// Software `OnPaint` frames cannot be counted yet: welding's Linux producer
/// has no `on_paint`, so CEF's software frames are dropped unseen and a broken
/// GPU path shows up as `accelerated=0` instead. Counting them needs the
/// welding fork (TODO.md, "CPU fallback count"). Printed with every summary.
pub const CPU_FALLBACK_UNOBSERVED: &str =
    "cpu_fallback not observable: welding has no OnPaint, so 0 is not a measurement";

/// Counts of zero-copy frames handed to the window, fed by the host loop.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Zero-copy frames (GPU texture import) set on the window.
    pub accelerated: u64,
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

    /// Mean time between presented frames, 0 when fewer than two frames.
    pub fn avg_interval_ms(&self) -> f64 {
        if self.intervals == 0 {
            return 0.0;
        }
        self.interval_sum.as_secs_f64() * 1000.0 / self.intervals as f64
    }

    /// The `--smoke frames` result line. Every presented frame is accelerated
    /// and `cpu_fallback` is fixed at 0 (see [`CPU_FALLBACK_UNOBSERVED`]).
    /// The interval is informational only.
    pub fn summary_line(&self) -> String {
        format!(
            "smoke: frames={0} accelerated={0} cpu_fallback=0 avg_interval_ms={1:.1}",
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
            "smoke: frames=3 accelerated=3 cpu_fallback=0 avg_interval_ms=9.0"
        );
    }
}
