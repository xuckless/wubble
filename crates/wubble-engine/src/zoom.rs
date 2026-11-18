//! Page zoom as a percentage. CEF zooms by level: the factor is
//! `1.2^level` (Chromium `third_party/blink/common/page/page_zoom.cc`,
//! `kTextSizeMultiplierRatio`), and `BrowserHost::Zoom` steps Chromium's own
//! preset ladder. There is no zoom-changed callback in CEF 156, and zoom is
//! per host, so a navigation can change it: the view re-reads the level after
//! each command, at the end of each load and on each address change, and
//! [`Zoom::observe`] reports a change.

use crate::page::PageEvent;

/// Blink's minimum and maximum page zoom (0.25 and 5.0).
pub const MIN_PERCENT: u32 = 25;
pub const MAX_PERCENT: u32 = 500;

const RATIO: f64 = 1.2;

/// The percentage CEF's zoom `level` stands for.
pub fn percent(level: f64) -> u32 {
    (100.0 * RATIO.powf(level))
        .round()
        .clamp(MIN_PERCENT as f64, MAX_PERCENT as f64) as u32
}

/// The CEF zoom level for `percent`, clamped to Blink's range.
pub fn level(percent: u32) -> f64 {
    let factor = percent.clamp(MIN_PERCENT, MAX_PERCENT) as f64 / 100.0;
    factor.ln() / RATIO.ln()
}

/// The zoom the host last heard of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zoom {
    pub percent: u32,
}

impl Default for Zoom {
    fn default() -> Self {
        Self { percent: 100 }
    }
}

impl Zoom {
    /// Record the level CEF reports; an event when the percentage changed.
    pub fn observe(&mut self, level: f64) -> Option<PageEvent> {
        let now = percent(level);
        (now != self.percent).then(|| {
            self.percent = now;
            PageEvent::ZoomChanged { percent: now }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chromium's `kPresetBrowserZoomFactors` (page_zoom.cc), as percentages.
    const PRESETS: [u32; 17] = [
        25, 33, 50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500,
    ];

    #[test]
    fn presets_round_trip() {
        for p in PRESETS {
            assert_eq!(percent(level(p)), p, "{p}%");
        }
        assert_eq!(level(100), 0.0);
        assert_eq!(percent(1.0), 120);
    }

    #[test]
    fn out_of_range_is_clamped() {
        assert_eq!(percent(level(5)), MIN_PERCENT);
        assert_eq!(percent(level(9000)), MAX_PERCENT);
        assert_eq!(percent(-40.0), MIN_PERCENT);
        assert_eq!(percent(40.0), MAX_PERCENT);
    }

    #[test]
    fn observe_reports_changes_only() {
        let mut zoom = Zoom::default();
        assert_eq!(zoom.observe(0.0), None);
        assert_eq!(
            zoom.observe(1.0),
            Some(PageEvent::ZoomChanged { percent: 120 })
        );
        assert_eq!(zoom.observe(1.0), None);
        assert_eq!(
            zoom.observe(0.0),
            Some(PageEvent::ZoomChanged { percent: 100 })
        );
    }
}
