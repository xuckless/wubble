//! Frame pacing inputs and frame statistics. Pure: no clock is read here.
//!
//! Interim pacing (M0 first frame): CEF paints on its own timer at
//! `windowless_frame_rate`, which wubble sets to the display refresh rate.
//! Compositor-driven begin-frames are plumbed through the welding fork but
//! still off (TODO.md, "Frame pacing").

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
