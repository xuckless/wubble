//! Memory pressure: what the OS readers report, the thresholds, and the gauge that turns
//! readings into the model's [`Pressure`].
//!
//! Pure. The readers (files, sysctl, Win32) live in `wubble-sysmem`; they hand their text or
//! numbers to the parsers here. Critical means the model discards eligible hidden tabs at once,
//! oldest first (TODO.md "Decided").
//!
//! Threshold numbers, no code copied:
//! - PSI: Android lmkd (AOSP `system/memory/lmkd/lmkd.cpp`, Apache-2.0) acts on a partial stall
//!   of 70 ms per 1 s window (7 %) and a complete stall of 700 ms (70 %). Our `avg10` is a 10 s
//!   average, so the numbers are lower and meant to be tuned by hand.
//! - MemAvailable: earlyoom (rfjakob/earlyoom, MIT) acts at 10 % available (SIGTERM) and 5 %
//!   (SIGKILL).
//! - macOS: Chromium `base/memory/memory_pressure_monitor_mac.cc` (BSD-3-Clause) reads the
//!   kernel's `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warn, 4 critical.
//! - The PSI file format: Linux `Documentation/accounting/psi.rst`.

use crate::sleep::Pressure;

/// The 10 s averages of `/proc/pressure/memory`, in percent of wall time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Psi {
    /// Some task stalled on memory.
    pub some_avg10: f32,
    /// Every non-idle task stalled on memory at once.
    pub full_avg10: f32,
}

/// One reading of a pressure source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    Psi(Psi),
    /// Bytes available for new allocations without swapping, and the total.
    Available {
        avail: u64,
        total: u64,
    },
    /// The OS says the level itself (macOS, Windows).
    Level(Pressure),
    /// Nothing to read (no source on this system): no pressure.
    Unknown,
}

/// When a level is entered. Data: one row per level, with where its numbers come from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threshold {
    pub level: Pressure,
    /// PSI `some avg10` at or above this (percent).
    pub psi_some: f32,
    /// PSI `full avg10` at or above this (percent).
    pub psi_full: f32,
    /// MemAvailable at or below this share of MemTotal (percent).
    pub avail_percent: f32,
    pub why: &'static str,
}

pub const THRESHOLDS: [Threshold; 2] = [
    Threshold {
        level: Pressure::Moderate,
        psi_some: 10.0,
        psi_full: 5.0,
        avail_percent: 10.0,
        why: "lmkd's partial stall (7 % of 1 s) over a 10 s average; earlyoom's SIGTERM at 10 %",
    },
    Threshold {
        level: Pressure::Critical,
        psi_some: 40.0,
        psi_full: 10.0,
        avail_percent: 5.0,
        why: "lmkd's complete stall (70 % of 1 s) over a 10 s average; earlyoom's SIGKILL at 5 %",
    },
];

/// A level is left only once the reading is below this share of its threshold (above its
/// inverse for MemAvailable), so a reading hovering at a threshold does not flap.
pub const HYSTERESIS: f32 = 0.7;

/// `/proc/pressure/memory`. `None` when the `some` line is missing or malformed; a missing
/// `full` line reads as no full stall.
pub fn parse_psi(text: &str) -> Option<Psi> {
    let avg10 = |kind: &str| -> Option<Option<f32>> {
        let Some(line) = text
            .lines()
            .find(|l| l.split_whitespace().next() == Some(kind))
        else {
            return Some(None);
        };
        let value = line
            .split_whitespace()
            .find_map(|f| f.strip_prefix("avg10="))?
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0)?;
        Some(Some(value))
    };
    Some(Psi {
        some_avg10: avg10("some")??,
        full_avg10: avg10("full")?.unwrap_or(0.0),
    })
}

/// `/proc/meminfo`'s MemAvailable and MemTotal, in bytes. `None` without both (kernels before
/// 3.14 have no MemAvailable).
pub fn parse_meminfo(text: &str) -> Option<Reading> {
    let kib = |key: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        let rest = line.strip_prefix(key)?.strip_prefix(':')?;
        let mut fields = rest.split_whitespace();
        let n: u64 = fields.next()?.parse().ok()?;
        (fields.next() == Some("kB")).then_some(n.checked_mul(1024)?)
    };
    let total = kib("MemTotal").filter(|&t| t > 0)?;
    Some(Reading::Available {
        avail: kib("MemAvailable")?,
        total,
    })
}

/// macOS `kern.memorystatus_vm_pressure_level`.
pub fn mac_level(value: i64) -> Pressure {
    match value {
        4 => Pressure::Critical,
        2 => Pressure::Moderate,
        _ => Pressure::None,
    }
}

/// Whether `reading` reaches `t`, with its numbers scaled by `scale` (1 to enter,
/// [`HYSTERESIS`] to stay).
fn reaches(reading: &Reading, t: &Threshold, scale: f32) -> bool {
    match *reading {
        Reading::Psi(p) => p.some_avg10 >= t.psi_some * scale || p.full_avg10 >= t.psi_full * scale,
        Reading::Available { avail, total } => {
            let percent = avail as f64 * 100.0 / total.max(1) as f64;
            percent <= f64::from(t.avail_percent / scale)
        }
        Reading::Level(level) => level >= t.level,
        Reading::Unknown => false,
    }
}

/// The level `reading` reaches, without hysteresis.
pub fn level_of(reading: &Reading) -> Pressure {
    level_at(reading, 1.0)
}

fn level_at(reading: &Reading, scale: f32) -> Pressure {
    THRESHOLDS
        .iter()
        .rev()
        .find(|t| reaches(reading, t, scale))
        .map_or(Pressure::None, |t| t.level)
}

/// The pressure level with hysteresis: one per window, stepped on the lifecycle tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gauge(Pressure);

impl Default for Gauge {
    fn default() -> Self {
        Gauge(Pressure::None)
    }
}

impl Gauge {
    pub fn level(self) -> Pressure {
        self.0
    }

    /// Rises to a level as soon as the reading reaches it; falls only once the reading is
    /// below the level's threshold scaled by [`HYSTERESIS`].
    pub fn step(self, reading: Reading) -> (Gauge, Pressure) {
        let enter = level_at(&reading, 1.0);
        let hold = level_at(&reading, HYSTERESIS);
        let level = enter.max(self.level().min(hold));
        (Gauge(level), level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine's `/proc/pressure/memory`, read on 2026-11-18.
    const REAL: &str = "some avg10=0.00 avg60=0.00 avg300=0.01 total=414857821\n\
                        full avg10=0.00 avg60=0.00 avg300=0.00 total=399868566\n";

    fn psi(some: f32, full: f32) -> Reading {
        Reading::Psi(Psi {
            some_avg10: some,
            full_avg10: full,
        })
    }

    #[test]
    fn parses_psi() {
        assert_eq!(
            parse_psi(REAL),
            Some(Psi {
                some_avg10: 0.0,
                full_avg10: 0.0
            })
        );
        let high = "some avg10=55.31 avg60=20.00 avg300=4.00 total=1\n\
                    full avg10=12.50 avg60=3.00 avg300=1.00 total=1\n";
        assert_eq!(
            parse_psi(high),
            Some(Psi {
                some_avg10: 55.31,
                full_avg10: 12.5
            })
        );
        // No `full` line (CPU-style file): no full stall.
        assert_eq!(
            parse_psi("some avg10=3.00 avg60=0 avg300=0 total=0\n"),
            Some(Psi {
                some_avg10: 3.0,
                full_avg10: 0.0
            })
        );
        assert_eq!(parse_psi("some avg10=x avg60=0\n"), None);
        assert_eq!(parse_psi("full avg10=1.00\n"), None);
        assert_eq!(parse_psi(""), None);
        assert_eq!(parse_psi("some avg10=-1.00\n"), None);
    }

    #[test]
    fn parses_meminfo() {
        let text = "MemTotal:       32621056 kB\nMemFree:         2129556 kB\n\
                    MemAvailable:   22013576 kB\n";
        assert_eq!(
            parse_meminfo(text),
            Some(Reading::Available {
                avail: 22_013_576 * 1024,
                total: 32_621_056 * 1024
            })
        );
        assert_eq!(parse_meminfo("MemTotal: 100 kB\nMemFree: 5 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: 0 kB\nMemAvailable: 0 kB\n"), None);
        assert_eq!(parse_meminfo("MemTotal: x kB\nMemAvailable: 1 kB\n"), None);
    }

    #[test]
    fn available_boundaries() {
        let at = |avail| level_at(&Reading::Available { avail, total: 1000 }, 1.0);
        assert_eq!(at(101), Pressure::None);
        assert_eq!(at(100), Pressure::Moderate);
        assert_eq!(at(51), Pressure::Moderate);
        assert_eq!(at(50), Pressure::Critical);
        assert_eq!(at(0), Pressure::Critical);
    }

    #[test]
    fn psi_levels() {
        assert_eq!(level_at(&psi(9.9, 0.0), 1.0), Pressure::None);
        assert_eq!(level_at(&psi(10.0, 0.0), 1.0), Pressure::Moderate);
        assert_eq!(level_at(&psi(0.0, 5.0), 1.0), Pressure::Moderate);
        assert_eq!(level_at(&psi(40.0, 0.0), 1.0), Pressure::Critical);
        assert_eq!(level_at(&psi(20.0, 10.0), 1.0), Pressure::Critical);
        assert_eq!(level_at(&Reading::Unknown, 1.0), Pressure::None);
    }

    #[test]
    fn mac_levels() {
        assert_eq!(mac_level(1), Pressure::None);
        assert_eq!(mac_level(2), Pressure::Moderate);
        assert_eq!(mac_level(4), Pressure::Critical);
        assert_eq!(mac_level(0), Pressure::None);
    }

    #[test]
    fn the_gauge_enters_at_once_and_leaves_with_hysteresis() {
        let g = Gauge::default();
        assert_eq!(g.level(), Pressure::None);
        let (g, p) = g.step(psi(45.0, 0.0));
        assert_eq!(p, Pressure::Critical);
        // Below the critical threshold but above 70 % of it: still critical.
        let (g, p) = g.step(psi(30.0, 0.0));
        assert_eq!(p, Pressure::Critical);
        // Below 70 % of critical, above moderate: moderate.
        let (g, p) = g.step(psi(20.0, 0.0));
        assert_eq!(p, Pressure::Moderate);
        // Hovering just under moderate: stays.
        let (g, p) = g.step(psi(8.0, 0.0));
        assert_eq!(p, Pressure::Moderate);
        let (g, p) = g.step(psi(6.0, 0.0));
        assert_eq!(p, Pressure::None);
        // Straight from none to critical through MemAvailable, then a level reading.
        let (g, p) = g.step(Reading::Available {
            avail: 4,
            total: 100,
        });
        assert_eq!(p, Pressure::Critical);
        let (g, p) = g.step(Reading::Available {
            avail: 6,
            total: 100,
        });
        assert_eq!(p, Pressure::Critical, "6 % is under 5 % / 0.7");
        let (_, p) = g.step(Reading::Level(Pressure::None));
        assert_eq!(p, Pressure::None);
    }
}
