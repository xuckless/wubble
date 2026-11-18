//! The bowed string as data: one curve, sampled into an [`EdgeTable`] that the edges it separates
//! follow (wubble's, at the owner's request: "the sidebar not acting rubberlike"). The sidebar's
//! sections, the sidebar glass's right edge (wgpu scene and the Slint-only outline) and the page's
//! masked edge all read the same table, so they bow exactly along the drawn string.
//!
//! The curve is the design's (the owner's design repo, `src/wubble/browser/seam.ts` `clip()` and
//! `browser/split.ts` `stringRect`): two cubics per edge meeting at the grab height. The two files
//! use slightly different control heights, so [`Curve`] names which one; [`Curve::points`] is the
//! one place their control points are written, and the SVG strings (`Seam::clip`,
//! `split::string_rect`) are formatted from it, so a table and its drawn string cannot drift.

use crate::motion::clamp;
use crate::seam::to_fixed2 as f;

/// Below this |bend| (px) a string is straight: no string is drawn and no edge bows.
pub const BOWED: f64 = 0.05;

/// The table's pitch never goes under this (logical px): linear interpolation of a 70 px bow at
/// 4 px steps is within 0.1 px of the curve.
pub const MIN_STEP: f64 = 4.0;
/// At most this many samples: the scene's bow uniform holds 64 × vec4.
pub const MAX_SAMPLES: usize = 256;

/// Which of the design's strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// The sidebar seam's (`seam.ts` `clip`).
    Seam,
    /// A split seam's (`split.ts` `stringRect`, `stringLine`).
    Split,
}

impl Curve {
    /// The edge from (x, top) down to (x, bot), bowed to x + b at height g: two cubics sharing
    /// (x + b, g), as 7 points from the top.
    pub fn points(self, x: f64, b: f64, top: f64, g: f64, bot: f64) -> [(f64, f64); 7] {
        let xb = x + b;
        let (upper, lower) = match self {
            Curve::Seam => (
                (top + (g - top) * 0.5, top + (g - top) * 0.45),
                (g + (bot - g) * 0.55, g + (bot - g) * 0.5),
            ),
            Curve::Split => (
                (top + (g - top) * 0.5, top + (g - top) * 0.55),
                (g + (bot - g) * 0.45, g + (bot - g) * 0.5),
            ),
        };
        [
            (x, top),
            (x, upper.0),
            (xb, upper.1),
            (xb, g),
            (xb, lower.0),
            (x, lower.1),
            (x, bot),
        ]
    }
}

/// `C… C…` for the cubics through `points` after the first (the current point), formatted like
/// the design's TS (`toFixed(2)`).
pub fn cubics(points: &[(f64, f64)]) -> String {
    points[1..]
        .chunks(3)
        .map(|c| {
            format!(
                "C{} {} {} {} {} {}",
                f(c[0].0),
                f(c[0].1),
                f(c[1].0),
                f(c[1].1),
                f(c[2].0),
                f(c[2].1)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How far a bowed edge is displaced along x, sampled every `step` px of height from `y0`.
/// Empty: straight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EdgeTable {
    pub y0: f64,
    pub step: f64,
    pub offset: Vec<f32>,
}

impl EdgeTable {
    /// The displacement at height `y`, linear between samples, 0 outside the table.
    pub fn at(&self, y: f64) -> f64 {
        let n = self.offset.len();
        if n < 2 || self.step <= 0.0 {
            return 0.0;
        }
        let u = (y - self.y0) / self.step;
        if !(0.0..=(n - 1) as f64).contains(&u) {
            return 0.0;
        }
        let i = (u.floor() as usize).min(n - 2);
        let t = u - i as f64;
        let (a, b) = (f64::from(self.offset[i]), f64::from(self.offset[i + 1]));
        a + (b - a) * t
    }

    /// The same edge, `dy` further down (for another element's coordinates).
    pub fn shifted(&self, dy: f64) -> Self {
        Self {
            y0: self.y0 + dy,
            ..self.clone()
        }
    }

    /// The largest displacement either way.
    pub fn max_abs(&self) -> f64 {
        self.offset
            .iter()
            .fold(0.0, |m, &o| m.max(f64::from(o).abs()))
    }

    pub fn is_straight(&self) -> bool {
        self.max_abs() < BOWED
    }
}

/// A cubic Bézier coordinate at `t`.
fn bezier(p: [f64; 4], t: f64) -> f64 {
    let s = 1.0 - t;
    s * s * s * p[0] + 3.0 * s * s * t * p[1] + 3.0 * s * t * t * p[2] + t * t * t * p[3]
}

/// The point of one cubic (4 of [`Curve::points`]) at height `y`, by bisection on its height,
/// which rises monotonically for both designs' control heights (tested).
fn point_at(c: &[(f64, f64)], y: f64) -> f64 {
    let ys = [c[0].1, c[1].1, c[2].1, c[3].1];
    let xs = [c[0].0, c[1].0, c[2].0, c[3].0];
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if bezier(ys, mid) < y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier(xs, (lo + hi) / 2.0)
}

/// The table's pitch for an edge `h` px long: at least [`MIN_STEP`], at most [`MAX_SAMPLES`].
pub fn step_for(h: f64) -> f64 {
    MIN_STEP.max((h / (MAX_SAMPLES - 1) as f64).ceil())
}

/// `curve` bowed by `bend` at height `gy` between `top` and `bot` (clamped 4 px inside, as the
/// design does), sampled from 0 to `h`. Straight (empty) under [`BOWED`].
pub fn edge_table(curve: Curve, bend: f64, gy: f64, top: f64, bot: f64, h: f64) -> EdgeTable {
    if bend.abs() < BOWED || bot - top < 8.0 || h <= 0.0 {
        return EdgeTable::default();
    }
    let step = step_for(h);
    let g = clamp(gy, top + 4.0, bot - 4.0);
    let p = curve.points(0.0, bend, top, g, bot);
    let n = (h / step).floor() as usize + 1;
    let offset = (0..n)
        .map(|i| {
            let y = i as f64 * step;
            let x = if y <= top || y >= bot {
                0.0
            } else if y < g {
                point_at(&p[0..4], y)
            } else {
                point_at(&p[3..7], y)
            };
            x as f32
        })
        .collect();
    EdgeTable {
        y0: 0.0,
        step,
        offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 784.0;
    const TOP: f64 = 14.0;
    const BOT: f64 = H - 14.0;

    #[test]
    fn straight_is_empty_and_zero_everywhere() {
        let t = edge_table(Curve::Seam, 0.0, 300.0, TOP, BOT, H);
        assert!(t.offset.is_empty() && t.is_straight());
        assert_eq!(t.at(300.0), 0.0);
        assert_eq!(EdgeTable::default().at(10.0), 0.0);
    }

    #[test]
    fn zero_at_both_ends_and_outside() {
        for curve in [Curve::Seam, Curve::Split] {
            let t = edge_table(curve, 40.0, 300.0, TOP, BOT, H);
            assert!(t.offset.len() <= MAX_SAMPLES);
            assert_eq!(t.at(0.0), 0.0);
            // linear between samples: zero up to the last sample above the curve
            assert_eq!(t.at(TOP - MIN_STEP), 0.0);
            assert_eq!(t.at(H), 0.0);
            assert_eq!(t.at(-50.0), 0.0);
            assert_eq!(t.at(H + 50.0), 0.0);
            assert!(t.at(BOT + 2.0).abs() < 1e-6);
        }
    }

    #[test]
    fn the_grab_height_is_displaced_by_the_bend() {
        // g on a sample: 300 = 75 · 4
        let t = edge_table(Curve::Seam, 40.0, 300.0, TOP, BOT, H);
        assert_eq!(t.step, 4.0);
        assert!((t.at(300.0) - 40.0).abs() < 1e-4, "{}", t.at(300.0));
        let t = edge_table(Curve::Split, -25.0, 300.0, TOP, BOT, H);
        assert!((t.at(300.0) + 25.0).abs() < 1e-4);
    }

    /// The table equals the Bézier's own points: evaluate the cubics at t and look the height up.
    #[test]
    fn equals_the_cubic_points() {
        for curve in [Curve::Seam, Curve::Split] {
            // the last: a short, steep upper half (26 px), where 4 px steps lose a little more
            for (bend, gy, tol) in [(40.0, 300.0, 0.1), (-70.0, 600.0, 0.1), (12.5, 40.0, 0.25)] {
                let t = edge_table(curve, bend, gy, TOP, BOT, H);
                let g = clamp(gy, TOP + 4.0, BOT - 4.0);
                let p = curve.points(0.0, bend, TOP, g, BOT);
                for half in [&p[0..4], &p[3..7]] {
                    for k in 1..20 {
                        let s = f64::from(k) / 20.0;
                        let x = bezier([half[0].0, half[1].0, half[2].0, half[3].0], s);
                        let y = bezier([half[0].1, half[1].1, half[2].1, half[3].1], s);
                        // the inversion itself is exact
                        assert!((point_at(half, y) - x).abs() < 1e-6);
                        // sampled every 4 px, linear in between
                        assert!(
                            (t.at(y) - x).abs() < tol,
                            "{curve:?} bend {bend} at y {y}: {} vs {x}",
                            t.at(y)
                        );
                    }
                }
            }
        }
    }

    /// The bisection needs each cubic's height to rise with t.
    #[test]
    fn heights_rise_monotonically() {
        for curve in [Curve::Seam, Curve::Split] {
            for gy in [20.0, 300.0, 760.0] {
                let p = curve.points(0.0, 50.0, TOP, gy, BOT);
                for half in [&p[0..4], &p[3..7]] {
                    let ys = [half[0].1, half[1].1, half[2].1, half[3].1];
                    let mut last = f64::NEG_INFINITY;
                    for k in 0..=200 {
                        let y = bezier(ys, f64::from(k) / 200.0);
                        assert!(y >= last, "{curve:?} gy {gy}");
                        last = y;
                    }
                }
            }
        }
    }

    #[test]
    fn the_sign_follows_the_bend() {
        let t = edge_table(Curve::Seam, 30.0, 400.0, TOP, BOT, H);
        assert!(t.offset.iter().all(|&o| o >= 0.0));
        let t = edge_table(Curve::Seam, -30.0, 400.0, TOP, BOT, H);
        assert!(t.offset.iter().all(|&o| o <= 0.0));
        assert!((t.max_abs() - 30.0).abs() < 1e-3);
    }

    #[test]
    fn tall_edges_keep_to_the_sample_cap() {
        let t = edge_table(Curve::Seam, 30.0, 400.0, TOP, 2146.0, 2160.0);
        assert!(t.offset.len() <= MAX_SAMPLES);
        assert_eq!(t.step, 9.0);
    }

    /// A real-speed drag bows the string visibly: 600 px/s across the resizable range, stepped at
    /// 120 Hz (a count of steps, no clock), reaches more than 10 px of bend through the held
    /// spring's lag alone.
    #[test]
    fn a_fast_drag_bows_more_than_ten_px() {
        use crate::seam::{INSET, Seam};
        let mut s = Seam::new();
        let (h, dt) = (800.0, 1.0 / 120.0);
        s.grab(INSET + 340.0, 300.0, h);
        let mut x = INSET + 340.0;
        let mut most: f64 = 0.0;
        for _ in 0..26 {
            x -= 600.0 * dt;
            s.drag(x, 300.0);
            s.step(dt, h);
            most = most.max(s.bend.x.abs());
        }
        assert!(most > 10.0, "{most}");
        let t = edge_table(
            Curve::Seam,
            s.bend.x,
            s.gy,
            INSET + 14.0,
            h - INSET - 14.0,
            h,
        );
        assert!(t.max_abs() > 10.0);
    }

    #[test]
    fn shifted_moves_it_down() {
        let t = edge_table(Curve::Seam, 30.0, 400.0, TOP, BOT, H);
        let s = t.shifted(8.0);
        assert_eq!(s.at(408.0), t.at(400.0));
    }
}
