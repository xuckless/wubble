// Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
// `src/wubble/slint/split.rs`, copied verbatim through the `slint-source-seam` block, except the
// marked `wubble change`s. The Source panel's string (`SourceSeam`) docks DevTools (`devtools.rs`);
// the model half (`SpaceTabs`, `SplitGroup`, `even`) lives in `space.rs` and `model.rs` as
// `Model::apply` commands.

//! Split view, ported 1:1 from `browser/split.ts`, plus the Source panel's string (the
//! `source-seam` block of `browser/chrome.tsx`) and the split actions of `browser/model.ts`.
//!
//! Up to three pages side by side, divided by the same rubber strings as the sidebar. Every
//! divider behaves like the sidebar's seam:
//! - held: it follows the pointer on the held spring (150/20), rubber-bands (k 34) past a pane's
//!   minimum, and the string bows toward the pointer at the grab height (saturating at 70 px);
//! - pulled further, past the close point, the pane gives way: it shrinks toward nothing on the
//!   band, showing that letting go will close it;
//! - released: the release spring (260/24) settles it and the bow twangs out on the bend spring
//!   (520/9, ζ ≈ 0.2).
//!
//! Geometry: seam j sits at the centre of the gap between pane j and pane j+1 (`c[j]`, px from the
//! web view's left). Pane j spans [c[j−1] + gap/2, c[j] − gap/2], with c[−1] = −gap/2 and
//! c[n−1] = width + gap/2, so outer panes reach the edges. Closing pane k moves one of its seams
//! onto the other: when they meet, the layout is exactly the layout without that pane.
//!
//! The TS `onClosed` / `onStops` callbacks become `SplitEvent`s queued in `events`, drained by
//! the owner after `step` / `release` / `nudge` / `even`.
//!
//! In Slint: each pane is a clipped Rectangle at `x0..x1` (rounded rect; Slint can't clip by a path),
//! and a held or twanging seam draws `string_line` as a `Path` (see demo.slint / driver.rs).

// wubble change: import order (rustfmt).
use crate::bow::{Curve, cubics};
// wubble change: `K` is imported for `SourceSeam`.
use crate::motion::{K, Spring, band, band_clamp, clamp, spring, step};
use crate::seam::to_fixed2 as f;

// @source slint-split
#[allow(non_snake_case, non_upper_case_globals)]
pub mod SPLIT {
    use crate::motion::K;
    pub const maxPanes: usize = 3;
    /// 8: the window's glass shows between pages, like round the web view (chrome.inset)
    pub const gap: f64 = 8.0;
    /// narrowest a page settles at
    pub const minPane: f64 = 280.0;
    /// rubber band past the minimum (the sidebar seam's k)
    pub const bandK: f64 = 34.0;
    /// the string's bow saturates at 70 px
    pub const bendK: f64 = 70.0;
    /// held narrower than this, the pane gives way; released there, it closes
    pub const toClose: f64 = 140.0;
    /// pane boxes extend this far each side so a bow can bulge outward
    pub const margin: f64 = 72.0;
    pub const held: [f64; 2] = K::seamHeld;
    pub const release: [f64; 2] = K::seamRelease;
    pub const bendSpring: [f64; 2] = K::seamBend;
    pub const keyStep: f64 = 24.0;
    pub const keyKick: f64 = 220.0;
}

/// A bowed vertical edge: how far it bows, and where along it the bow peaks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bow {
    pub bend: f64,
    pub gy: f64,
}

// @source slint-string-rect
/// Edge from (x, top) down to (x, bot), bowed to x + b at height g. Two cubics, straight when b = 0.
// wubble change: the control points come from `bow::Curve` (one owner with the edge table).
fn curves_down(x: f64, b: f64, top: f64, g: f64, bot: f64) -> String {
    cubics(&Curve::Split.points(x, b, top, g, bot))
}
fn edge_down(curve: Curve, x: f64, b: f64, top: f64, g: f64, bot: f64) -> String {
    let points = curve.points(x, b, top, g, bot);
    format!("L{} {} {}", f(x), f(top), cubics(&points))
}
/// The same edge traversed bottom to top.
fn edge_up(curve: Curve, x: f64, b: f64, bot: f64, g: f64, top: f64) -> String {
    let mut points = curve.points(x, b, top, g, bot);
    points.reverse();
    format!("L{} {} {}", f(x), f(bot), cubics(&points))
}

/// A rounded rect from x0 to x1 (height h, corner radius r) whose left and/or right edge is a
/// bowed string. Positive bend bows right. Same string as `stringRect` in split.ts.
pub fn string_rect(x0: f64, x1: f64, h: f64, r: f64, l: Option<Bow>, rt: Option<Bow>) -> String {
    let curve = Curve::Split;
    let (top, bot) = (r, h - r);
    let g = |b: Option<Bow>| clamp(b.map(|b| b.gy).unwrap_or(h / 2.0), top + 4.0, bot - 4.0);
    let rr = 0f64.max(r.min((x1 - x0) / 2.0));
    let right = match rt {
        Some(b) if b.bend.abs() > 0.01 => edge_down(curve, x1, b.bend, top, g(rt), bot),
        _ => format!("L{} {}", f(x1), f(bot)),
    };
    let left = match l {
        Some(b) if b.bend.abs() > 0.01 => edge_up(curve, x0, b.bend, bot, g(l), top),
        _ => format!("L{} {}", f(x0), f(top)),
    };
    format!(
        "M{} 0 L{} 0 Q{} 0 {} {} {} Q{} {} {} {} L{} {} Q{} {} {} {} {} Q{} 0 {} 0 Z",
        f(x0 + rr),
        f(x1 - rr),
        f(x1),
        f(x1),
        f(top),
        right,
        f(x1),
        f(h),
        f(x1 - rr),
        f(h),
        f(x0 + rr),
        f(h),
        f(x0),
        f(h),
        f(x0),
        f(bot),
        left,
        f(x0),
        f(x0 + rr)
    )
}

/// Just the bowed line (for drawing a held string). Same string as `stringLine` in split.ts.
pub fn string_line(x: f64, h: f64, r: f64, b: Bow) -> String {
    let (top, bot) = (r, h - r);
    let g = clamp(b.gy, top + 4.0, bot - 4.0);
    format!(
        "M{} {} {}",
        f(x),
        f(top),
        curves_down(x, b.bend, top, g, bot)
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneRect {
    pub id: u64,
    pub x0: f64,
    pub x1: f64,
    pub l: Option<Bow>,
    pub r: Option<Bow>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SplitEvent {
    /// a closing pane has fully gone; remove it from the model (`act.unsplit`) and store the stops
    Closed { id: u64, stops: Vec<f64> },
    /// a release, nudge or even settled new stops; store them
    Stops(Vec<f64>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Closing {
    k: usize,
    s: usize,
}

#[derive(Clone, Debug)]
pub struct Split {
    pub ids: Vec<u64>,
    /// seam centres, px from the web view's left
    pub c: Vec<Spring>,
    pub bend: Vec<Spring>,
    pub gy: Vec<f64>,
    /// where each seam rests, as a fraction of the width
    pub stops: Vec<f64>,
    pub width: f64,
    pub h: f64,
    /// the held seam, if any (`held = -1` in the TS)
    pub held: Option<usize>,
    pub px: f64,
    pub py: f64,
    closing: Option<Closing>,
    pub events: Vec<SplitEvent>,
    fresh: bool,
}

impl Default for Split {
    fn default() -> Self {
        Self::new()
    }
}

impl Split {
    pub fn new() -> Self {
        Split {
            ids: vec![],
            c: vec![],
            bend: vec![],
            gy: vec![],
            stops: vec![],
            width: 0.0,
            h: 0.0,
            held: None,
            px: 0.0,
            py: 0.0,
            closing: None,
            events: vec![],
            fresh: true,
        }
    }

    pub fn n(&self) -> usize {
        self.ids.len()
    }

    pub fn is_closing(&self) -> bool {
        self.closing.is_some()
    }

    /// Forget the layout (switching Space): the next sync places panes at their stops, no animation.
    pub fn reset(&mut self) {
        self.ids.clear();
        self.c.clear();
        self.bend.clear();
        self.gy.clear();
        self.closing = None;
        self.held = None;
        self.fresh = true;
    }

    fn left(&self, j: isize) -> f64 {
        if j < 0 {
            -SPLIT::gap / 2.0
        } else {
            self.c[j as usize].x
        }
    }
    fn right(&self, j: isize) -> f64 {
        if j >= self.n() as isize - 1 {
            self.width + SPLIT::gap / 2.0
        } else {
            self.c[j as usize].x
        }
    }

    // @source slint-split-sync
    /// Adopt the model's panes. A new pane enters with no width where it will appear (beside the
    /// page it joins), and the seams around it spring open; existing seams keep their springs.
    pub fn sync(&mut self, ids: &[u64], stops: &[f64]) {
        if self.closing.is_some() {
            return; // the closing animation owns the layout until it hands back
        }
        let same = ids == self.ids.as_slice();
        if !same {
            let (mut c, mut bend, mut gy) = (vec![], vec![], vec![]);
            // NaN: width unknown yet, place at the stop
            let end = if self.width != 0.0 && !self.fresh {
                self.width + SPLIT::gap / 2.0
            } else {
                f64::NAN
            };
            let old_n = self.n();
            let old_c = self.c.clone();
            let right_of = |o: usize| if o + 1 < old_n { old_c[o].x } else { end };
            let left_of = |o: usize| {
                if o > 0 {
                    old_c[o - 1].x
                } else {
                    -SPLIT::gap / 2.0
                }
            };
            // wubble change: the held seam follows its spring, or is let go when it no longer
            // separates the same two panes (it indexed past the seams once a pane left).
            let mut held = None;
            for j in 0..ids.len().saturating_sub(1) {
                // a seam that already separated these two panes keeps its spring
                let old = self.ids.iter().position(|&x| x == ids[j]);
                let old_next = self.ids.iter().position(|&x| x == ids[j + 1]);
                let keep = matches!(old, Some(o) if o < old_c.len() && self.ids.get(o + 1) == Some(&ids[j + 1]));
                // a new seam starts where the pane it opens has no width yet
                let start = match (old, old_next) {
                    (Some(o), _) => right_of(o),
                    (None, Some(o)) => left_of(o),
                    _ => end,
                };
                if keep {
                    let o = old.unwrap();
                    if self.held == Some(o) {
                        held = Some(j);
                    }
                    c.push(self.c[o]);
                    bend.push(self.bend[o]);
                    gy.push(self.gy[o]);
                } else {
                    c.push(spring(if old_n != 0 { start } else { end }));
                    bend.push(spring(0.0));
                    gy.push(self.h / 2.0);
                    if !self.fresh {
                        bend[j].v = -260.0; // opening twangs the new string toward the page it makes room for
                    }
                }
            }
            self.ids = ids.to_vec();
            self.held = held;
            self.c = c;
            self.bend = bend;
            self.gy = gy;
        }
        self.stops = stops.to_vec();
        self.fresh = false;
    }

    /// Close a pane the same way a drag does: its seam carries it shut, then `Closed` fires.
    pub fn close(&mut self, id: u64) {
        let Some(k) = self.ids.iter().position(|&x| x == id) else {
            return;
        };
        if self.n() < 2 {
            return;
        }
        let s = if k + 1 < self.n() { k } else { k - 1 };
        self.closing = Some(Closing { k, s });
        self.bend[s].v += (if s == k { -1.0 } else { 1.0 }) * 260.0;
    }

    // @source slint-split-drag
    pub fn grab(&mut self, j: usize, px: f64, py: f64) {
        // wubble change: a seam that no longer exists cannot be grabbed.
        if j >= self.c.len() {
            return;
        }
        self.held = Some(j);
        self.px = px;
        self.py = py;
        self.gy[j] = clamp(py, 18.0, self.h - 18.0);
    }

    pub fn drag(&mut self, px: f64, py: f64) {
        self.px = px;
        self.py = py;
    }

    pub fn release(&mut self) {
        let Some(j) = self.held else { return };
        self.held = None;
        // wubble change: an unmeasured width closes no pane and makes no NaN stop.
        if self.width <= 0.0 {
            return;
        }
        let (lo, hi) = (self.left(j as isize - 1), self.right(j as isize + 1));
        let wl = self.px - lo - SPLIT::gap;
        let wr = hi - self.px - SPLIT::gap;
        if wl < SPLIT::toClose {
            self.closing = Some(Closing { k: j, s: j });
            return;
        }
        if wr < SPLIT::toClose {
            self.closing = Some(Closing { k: j + 1, s: j });
            return;
        }
        self.stops[j] = clamp(
            self.c[j].x,
            lo + SPLIT::gap + SPLIT::minPane,
            hi - SPLIT::gap - SPLIT::minPane,
        ) / self.width;
        self.events.push(SplitEvent::Stops(self.stops.clone()));
    }

    /// Keyboard: move a seam a step, with a twang.
    pub fn nudge(&mut self, j: usize, dir: f64) {
        // wubble change: ignore out-of-range seams or unmeasured widths.
        if j >= self.c.len() || self.width <= 0.0 {
            return;
        }
        let (lo, hi) = (self.left(j as isize - 1), self.right(j as isize + 1));
        self.stops[j] = clamp(
            self.stops[j] * self.width + dir * SPLIT::keyStep,
            lo + SPLIT::gap + SPLIT::minPane,
            hi - SPLIT::gap - SPLIT::minPane,
        ) / self.width;
        self.bend[j].v += dir * SPLIT::keyKick;
        self.gy[j] = self.h / 2.0;
        self.events.push(SplitEvent::Stops(self.stops.clone()));
    }

    /// Even shares (double-click a seam).
    pub fn even(&mut self) {
        let n = self.n() as f64;
        self.stops = (0..self.c.len()).map(|j| (j as f64 + 1.0) / n).collect();
        for j in 0..self.c.len() {
            self.bend[j].v += 160.0 * (if j % 2 == 1 { -1.0 } else { 1.0 });
        }
        self.events.push(SplitEvent::Stops(self.stops.clone()));
    }

    // @source slint-split-step
    /// `width`, `h`: the web view's size. Returns true while anything moves.
    pub fn step(&mut self, dt: f64, width: f64, h: f64) -> bool {
        if self.width != 0.0 && width != self.width && self.n() != 0 {
            // the web view resized (sidebar, Source, window): seams keep their share, no spring lag
            let k = width / self.width;
            for c in &mut self.c {
                c.x *= k;
            }
        }
        self.width = width;
        self.h = h;
        let mut moving = false;
        // Seams synced before the width was known start at NaN and are placed at their stops. The
        // TS does this inside the loop, so on that first frame seam j still sees seam j+1 as NaN:
        // in JS `Math.max(minL, NaN)` is NaN and seam j steps to NaN for one frame (then recovers).
        // Rust's f64::max ignores NaN, so placing them all first gives the result the TS intends.
        for (j, c) in self.c.iter_mut().enumerate() {
            if c.x.is_nan() {
                c.x = self.stops[j] * width;
            }
        }
        let mut j = 0;
        while j + 1 < self.n() {
            let (lo, hi) = (self.left(j as isize - 1), self.right(j as isize + 1));
            let min_l = lo + SPLIT::gap + SPLIT::minPane;
            let max_r = hi - SPLIT::gap - SPLIT::minPane;
            if let Some(cl) = self.closing.filter(|cl| cl.s == j) {
                // carry the closing pane shut: this seam goes to the far side of pane k
                let to = if cl.k == j { lo } else { hi };
                step(&mut self.c[j], to, SPLIT::release[0], SPLIT::release[1], dt);
                step(
                    &mut self.bend[j],
                    0.0,
                    SPLIT::bendSpring[0],
                    SPLIT::bendSpring[1],
                    dt,
                );
                if (self.c[j].x - to).abs() < 0.75 && self.c[j].v.abs() < 40.0 {
                    let id = self.ids.remove(cl.k);
                    self.c.remove(j);
                    self.bend.remove(j);
                    self.gy.remove(j);
                    self.stops.remove(j);
                    self.closing = None;
                    // wubble change: a seam held while another closes keeps its index in step.
                    self.held = match self.held {
                        Some(h) if h > j => Some(h - 1),
                        Some(h) if h == j => None,
                        h => h,
                    };
                    let stops = (0..self.stops.len()).map(|i| self.c[i].x / width).collect();
                    self.events.push(SplitEvent::Closed { id, stops });
                    return true;
                }
                moving = true;
            } else if self.held == Some(j) {
                let p = self.px;
                // past a pane's minimum: resistance; past its close point: it gives way toward nothing
                let want = if p - lo - SPLIT::gap < SPLIT::toClose {
                    lo + band(p - lo, SPLIT::bandK)
                } else if hi - p - SPLIT::gap < SPLIT::toClose {
                    hi - band(hi - p, SPLIT::bandK)
                } else {
                    band_clamp(p, min_l, max_r, SPLIT::bandK)
                };
                step(&mut self.c[j], want, SPLIT::held[0], SPLIT::held[1], dt);
                self.bend[j].x = band(p - self.c[j].x, SPLIT::bendK);
                self.bend[j].v = 0.0;
                self.gy[j] += (clamp(self.py, 18.0, h - 18.0) - self.gy[j]) * 1f64.min(dt * 30.0);
                moving = true;
            } else {
                let to = clamp(self.stops[j] * width, min_l, min_l.max(max_r));
                moving =
                    step(&mut self.c[j], to, SPLIT::release[0], SPLIT::release[1], dt) || moving;
                moving = step(
                    &mut self.bend[j],
                    0.0,
                    SPLIT::bendSpring[0],
                    SPLIT::bendSpring[1],
                    dt,
                ) || moving;
            }
            j += 1;
        }
        moving
    }

    /// Pane rects with their bowed edges, in web-view coordinates. Empty panes are skipped.
    pub fn panes(&self) -> Vec<PaneRect> {
        let mut out = vec![];
        for k in 0..self.n() {
            let x0 = self.left(k as isize - 1) + SPLIT::gap / 2.0;
            let x1 = self.right(k as isize) - SPLIT::gap / 2.0;
            if x1 - x0 < 0.5 {
                continue;
            }
            out.push(PaneRect {
                id: self.ids[k],
                x0,
                x1,
                l: if k > 0 {
                    Some(Bow {
                        bend: self.bend[k - 1].x,
                        gy: self.gy[k - 1],
                    })
                } else {
                    None
                },
                r: if k + 1 < self.n() {
                    Some(Bow {
                        bend: self.bend[k].x,
                        gy: self.gy[k],
                    })
                } else {
                    None
                },
            });
        }
        out
    }

    /// Seam centres and bows, for grab handles and the held string: (x, bend, gy).
    pub fn seams(&self) -> Vec<(f64, f64, f64)> {
        self.c
            .iter()
            .enumerate()
            .map(|(j, c)| (c.x, self.bend[j].x, self.gy[j]))
            .collect()
    }
}

// @source slint-source-seam
/// The Source panel's edge is a string too (`source-seam` in chrome.tsx): same held / release /
/// bend springs and band as the sidebar. Held: 1:1 with rubber band past 320–720 (and 60% of the
/// window); pulled under 200 it gives way toward closed, and released there it closes.
#[derive(Clone, Debug)]
pub struct SourceSeam {
    /// the panel's resting width while open
    pub want: f64,
    pub held: bool,
    pub px: f64,
    pub py: f64,
    pub bend: Spring,
    pub gy: f64,
    /// the panel's width (`srcW` in the TS)
    pub w: Spring,
    pub open: bool,
}

impl Default for SourceSeam {
    fn default() -> Self {
        SourceSeam {
            want: 480.0,
            held: false,
            px: 0.0,
            py: 0.0,
            bend: spring(0.0),
            gy: 0.0,
            w: spring(0.0),
            open: false,
        }
    }
}

impl SourceSeam {
    const INSET: f64 = 8.0; // chrome.inset

    /// Pointer down on the grab (window px). The TS sets gy straight to the pointer (no clamp).
    pub fn grab(&mut self, px: f64, py: f64) {
        self.held = true;
        self.px = px;
        self.py = py;
        self.gy = py;
    }

    pub fn drag(&mut self, px: f64, py: f64) {
        if self.held {
            self.px = px;
            self.py = py;
        }
    }

    /// Returns true if the release closed the panel (the owner clears its `source` state).
    pub fn release(&mut self) -> bool {
        if !self.held {
            return false;
        }
        self.held = false;
        if self.w.x < 200.0 {
            self.open = false;
            true
        } else {
            self.want = clamp(self.w.x, 320.0, 720.0);
            false
        }
    }

    /// Arrow keys: ±24 px with a twang (ArrowLeft widens: dir = 1).
    pub fn nudge(&mut self, dir: f64, h: f64) {
        self.want = clamp(self.want + dir * 24.0, 320.0, 720.0);
        self.bend.v -= dir * 220.0;
        self.gy = h / 2.0;
    }

    /// `w_win`, `h_win`: the window's inner size. Returns true while anything moves.
    pub fn step(&mut self, dt: f64, w_win: f64, h_win: f64) -> bool {
        let target = if self.open { self.want } else { 0.0 };
        if self.held {
            let raw = w_win - Self::INSET - self.px;
            let want = if raw < 200.0 {
                band(raw, K_BAND)
            } else {
                band_clamp(raw, 320.0, 720f64.min(w_win * 0.6), K_BAND)
            };
            step(&mut self.w, want, K::seamHeld[0], K::seamHeld[1], dt);
            self.bend.x = band(self.px - (w_win - Self::INSET - self.w.x - 4.0), 70.0);
            self.bend.v = 0.0;
            self.gy += (clamp(self.py, 18.0, h_win - 18.0) - self.gy) * 1f64.min(dt * 30.0);
            true
        } else {
            let a = step(
                &mut self.w,
                target,
                K::seamRelease[0],
                K::seamRelease[1],
                dt,
            );
            let b = step(&mut self.bend, 0.0, K::seamBend[0], K::seamBend[1], dt);
            a || b
        }
    }
}

/// SEAM.bandK (seam.ts), used by the Source string.
const K_BAND: f64 = 34.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::even;

    #[test]
    fn string_paths_match_the_ts() {
        // expected strings produced by split.ts in Node
        assert_eq!(
            string_line(
                100.125,
                400.0,
                14.0,
                Bow {
                    bend: 12.5,
                    gy: 150.0
                }
            ),
            "M100.13 14.00 C100.13 82.00 112.63 88.80 112.63 150.00 C112.63 256.20 100.13 268.00 100.13 386.00"
        );
    }

    #[test]
    fn drag_past_close_point_closes_the_pane() {
        let mut s = Split::new();
        s.sync(&[1, 2, 3], &even(3));
        for _ in 0..5 {
            s.step(1.0 / 60.0, 1200.0, 700.0);
        }
        assert!((s.c[0].x - 400.0).abs() < 1e-9);
        s.grab(0, 400.0, 300.0);
        s.drag(60.0, 300.0);
        for _ in 0..30 {
            s.step(1.0 / 60.0, 1200.0, 700.0);
        }
        s.release();
        let mut n = 0;
        while s.events.is_empty() {
            s.step(1.0 / 60.0, 1200.0, 700.0);
            n += 1;
            assert!(n < 2000);
        }
        assert!(matches!(&s.events[0], SplitEvent::Closed { id: 1, .. }));
        assert_eq!(s.ids, vec![2, 3]);
    }

    // wubble change: the tests below are wubble's.

    /// One comma-joined number list, as the TS fixture writes it.
    fn numbers(v: &serde_json::Value) -> Vec<f64> {
        v.as_str()
            .unwrap()
            .split(',')
            .map(|x| x.parse().unwrap())
            .collect()
    }

    fn floats(v: &serde_json::Value) -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_f64().unwrap())
            .collect()
    }

    /// The design repo's parity3 scenario (slint/crate/examples/parity3.rs) against the same run
    /// of browser/split.ts under bun, stored in the fixture's third line. Exact: the maths is
    /// + - * / and sqrt.
    #[test]
    fn parity_with_ts() {
        let line = include_str!("../tests/fixtures/ts_parity.jsonl")
            .lines()
            .nth(2)
            .unwrap();
        let ts: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            string_line(
                100.125,
                400.0,
                14.0,
                Bow {
                    bend: 12.5,
                    gy: 150.0
                }
            ),
            ts["line"].as_str().unwrap()
        );
        let rect = string_rect(
            72.0,
            72.0 + 333.375,
            700.0,
            14.0,
            Some(Bow {
                bend: -7.25,
                gy: 5.0,
            }),
            Some(Bow {
                bend: 30.0625,
                gy: 690.0,
            }),
        );
        assert_eq!(rect, ts["rect"].as_str().unwrap());
        assert_eq!(
            string_rect(0.5, 10.0, 700.0, 14.0, None, None),
            ts["rectStraight"].as_str().unwrap()
        );

        let mut s = Split::new();
        let dt = 1.0 / 60.0;
        let mut tr = vec![];
        s.sync(&[1, 2], &[0.5]);
        for _ in 0..3 {
            s.step(dt, 1200.0, 700.0);
        }
        s.sync(&[1, 3, 2], &[1.0 / 3.0, 2.0 / 3.0]);
        for _ in 0..40 {
            s.step(dt, 1200.0, 700.0);
            tr.extend(s.c.iter().map(|c| c.x));
            tr.extend(s.bend.iter().map(|b| b.x));
        }
        let x = s.c[1].x;
        s.grab(1, x, 300.0);
        for i in 1..=30 {
            let x = s.c[1].x + if i < 20 { -9.0 } else { 4.0 };
            s.drag(x, 300.0 + f64::from(i));
            s.step(dt, 1100.0, 700.0);
            tr.extend(s.c.iter().map(|c| c.x));
            tr.extend([s.bend[1].x, s.gy[1]]);
        }
        s.release();
        for _ in 0..40 {
            s.step(dt, 1100.0, 700.0);
            tr.extend(s.c.iter().map(|c| c.x));
        }
        s.nudge(0, 1.0);
        s.even();
        for _ in 0..20 {
            s.step(dt, 1100.0, 700.0);
        }
        let x = s.c[0].x;
        s.grab(0, x, 200.0);
        s.drag(80.0, 210.0);
        for _ in 0..20 {
            s.step(dt, 1100.0, 700.0);
        }
        s.release();
        let mut i = 0;
        while i < 200 && s.n() > 2 {
            s.step(dt, 1100.0, 700.0);
            i += 1;
        }
        tr.extend(s.c.iter().map(|c| c.x));
        assert_eq!(tr, numbers(&ts["tr"]));

        let want_ev = ts["ev"].as_array().unwrap();
        assert_eq!(s.events.len(), want_ev.len());
        for (e, w) in s.events.iter().zip(want_ev) {
            match e {
                SplitEvent::Stops(st) => {
                    assert_eq!(w[0], "stops");
                    assert_eq!(*st, floats(&w[1]));
                }
                SplitEvent::Closed { id, stops } => {
                    assert_eq!(w[0], "closed");
                    assert_eq!(w[1].as_u64(), Some(*id));
                    assert_eq!(*stops, floats(&w[2]));
                }
            }
        }
        let ids: Vec<u64> = ts["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect();
        assert_eq!(s.ids, ids);
        let panes = s.panes();
        let want = ts["panes"].as_array().unwrap();
        assert_eq!(panes.len(), want.len());
        let bend = |b: Option<Bow>| b.map(|b| b.bend);
        for (p, w) in panes.iter().zip(want) {
            assert_eq!(Some(p.id), w[0].as_u64());
            assert_eq!(Some(p.x0), w[1].as_f64());
            assert_eq!(Some(p.x1), w[2].as_f64());
            assert_eq!(bend(p.l), w[3].as_f64());
            assert_eq!(bend(p.r), w[4].as_f64());
        }
    }

    /// A pane leaving while a seam is held: the seam still between the same two panes stays
    /// held, one that went is let go, and letting go of it does nothing.
    #[test]
    fn a_pane_leaving_lets_go_of_its_held_seam() {
        let settled = |ids: &[u64]| {
            let mut s = Split::new();
            s.sync(ids, &even(ids.len()));
            s.step(1.0 / 60.0, 1200.0, 700.0);
            s
        };
        let mut s = settled(&[1, 2, 3]);
        s.grab(1, 800.0, 300.0);
        s.sync(&[1, 2], &[0.5]);
        assert_eq!(s.held, None);
        s.release();
        assert!(s.events.is_empty());

        let mut s = settled(&[1, 2, 3]);
        s.grab(0, 400.0, 300.0);
        s.sync(&[1, 2], &[0.5]);
        assert_eq!(s.held, Some(0));
        s.release();
        assert!(matches!(s.events.as_slice(), [SplitEvent::Stops(_)]));

        // a seam held while the pane left of it closes moves down an index
        let mut s = settled(&[1, 2, 3]);
        s.close(1);
        s.grab(1, 800.0, 300.0);
        for _ in 0..2000 {
            if !s.events.is_empty() {
                break;
            }
            s.step(1.0 / 60.0, 1200.0, 700.0);
        }
        assert_eq!(s.ids, vec![2, 3]);
        assert_eq!(s.held, Some(0));
        s.release();
    }

    /// Three panes in a window narrower than three minimum panes: the panes shrink instead of
    /// overlapping, and every rect stays ordered with a positive width.
    #[test]
    fn narrow_width_keeps_panes_ordered() {
        let mut s = Split::new();
        s.sync(&[1, 2, 3], &even(3));
        for _ in 0..200 {
            s.step(1.0 / 60.0, 600.0, 500.0);
        }
        let panes = s.panes();
        assert_eq!(panes.len(), 3);
        let mut last = f64::NEG_INFINITY;
        for p in &panes {
            assert!(p.x1 > p.x0, "{p:?}");
            assert!(p.x0 >= last, "{p:?}");
            assert!(p.x0 >= 0.0 && p.x1 <= 600.0, "{p:?}");
            last = p.x1;
        }
    }

    fn run(s: &mut SourceSeam, frames: usize) {
        for _ in 0..frames {
            s.step(1.0 / 60.0, 1200.0, 700.0);
        }
    }

    #[test]
    fn the_source_panel_opens_to_its_width_and_holds_within_its_band() {
        let mut s = SourceSeam {
            open: true,
            ..SourceSeam::default()
        };
        run(&mut s, 240);
        assert!((s.w.x - 480.0).abs() < 1.0, "{}", s.w.x);
        // held far left: the width rubber-bands past 720 (and 60% of 1200), never snaps
        s.grab(1000.0, 300.0);
        s.drag(100.0, 300.0);
        run(&mut s, 240);
        assert!(s.w.x > 700.0 && s.w.x < 800.0, "{}", s.w.x);
        assert!(!s.release());
        assert!((s.want - 720.0).abs() < 1.0, "{}", s.want);
    }

    #[test]
    fn pulled_under_200_a_release_closes_the_panel() {
        let mut s = SourceSeam {
            open: true,
            ..SourceSeam::default()
        };
        run(&mut s, 240);
        s.grab(700.0, 300.0);
        s.drag(1150.0, 300.0);
        run(&mut s, 240);
        assert!(s.w.x < 200.0, "{}", s.w.x);
        assert!(s.release());
        assert!(!s.open);
        run(&mut s, 480);
        assert!(s.w.x.abs() < 1.0);
        // a release with nothing held does nothing
        assert!(!s.release());
    }

    #[test]
    fn arrow_keys_nudge_by_24_within_the_band() {
        let mut s = SourceSeam::default();
        s.nudge(1.0, 700.0);
        assert_eq!(s.want, 504.0);
        for _ in 0..20 {
            s.nudge(-1.0, 700.0);
        }
        assert_eq!(s.want, 320.0);
    }

    #[test]
    fn nudge_and_release_guard_oob_and_zero_width() {
        let mut s = Split::new();
        s.sync(&[1, 2], &[0.5]);
        // width is still 0.0: neither nudge nor release may panic or produce NaN stops.
        s.nudge(0, 1.0);
        s.nudge(5, 1.0);
        s.grab(0, 200.0, 200.0);
        s.release();
        assert_eq!(s.stops, vec![0.5]);
        assert!(s.stops[0].is_finite());
        // and an unmeasured split closes no pane
        assert!(s.closing.is_none());
    }
}
