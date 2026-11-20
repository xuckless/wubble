// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/morph.ts:14-66; ../ultra-reactivate/vendor/thinking-orbs/engine/morph.ts:70-141
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).
// HOLD and MORPH are generated (`kit_gen::orb`).

//! Morph: a dotted outline cycling circle → triangle → square → circle, the "shaping" state.
//! Each shape is a closed path by arc length (top centre, clockwise); every frame the two
//! neighbouring paths are blended and the dots laid evenly along the blend.

use super::prim::finalize_frame;
use super::{Dot, ModeOpts, OrbFrame, V2, from, inc, ix, js_round, nz};
use crate::kit_gen::orb::{HOLD, MORPH};
use std::f64::consts::PI;

fn smooth_e(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}

/// A closed polygon walked by arc length (`polyPath`).
struct Poly {
    verts: &'static [V2],
    l: Vec<f64>,
    total: f64,
}

fn poly_path(verts: &'static [V2]) -> Poly {
    let v = verts.len() as f64;
    let mut l: Vec<f64> = vec![];
    let mut total = 0.0;
    for i in from(0.0, v) {
        let a = verts[ix(i)];
        let b = verts[ix((i + 1.0) % v)];
        let len = f64::hypot(b[0] - a[0], b[1] - a[1]);
        l.push(len);
        total += len;
    }
    Poly { verts, l, total }
}

impl Poly {
    fn at(&self, f: f64) -> V2 {
        let (verts, l) = (self.verts, &self.l);
        let v = verts.len() as f64;
        let mut target = f * self.total;
        let mut i = 0.0;
        while target > l[ix(i)] && i < v - 1.0 {
            target -= l[ix(i)];
            inc(&mut i);
        }
        let a = verts[ix(i)];
        let b = verts[ix((i + 1.0) % v)];
        let ff = if nz(l[ix(i)]) {
            f64::min(1.0, target / l[ix(i)])
        } else {
            0.0
        };
        [a[0] + (b[0] - a[0]) * ff, a[1] + (b[1] - a[1]) * ff]
    }
}

/// One of the cycle's outlines (`Path`): the circle is a function, the others polygons.
enum Shape {
    Circle,
    Poly(Poly),
}

impl Shape {
    fn at(&self, f: f64) -> V2 {
        match self {
            Shape::Circle => circle(f),
            Shape::Poly(p) => p.at(f),
        }
    }
}

fn circle(f: f64) -> V2 {
    let a = -PI / 2.0 + f * 2.0 * PI;
    [a.cos() * 0.24, a.sin() * 0.24]
}
const TRIANGLE: &[V2] = &[[0.0, -0.26], [0.24, 0.16], [-0.24, 0.16]];
// 5-vertex walk so the path STARTS at top-centre like the other shapes
const SQUARE: &[V2] = &[
    [0.0, -0.2],
    [0.2, -0.2],
    [0.2, 0.2],
    [-0.2, 0.2],
    [-0.2, -0.2],
];
fn cycle() -> [Shape; 3] {
    // wubble change: the TS builds `CYCLE` once at load, of three shapes
    [
        Shape::Circle,
        Shape::Poly(poly_path(TRIANGLE)),
        Shape::Poly(poly_path(SQUARE)),
    ]
}

// low floor keeps sparse outlines possible while never degenerating
fn morph_n(d: f64) -> f64 {
    f64::max(6.0, js_round(34.0 * d))
}

// Tuned in inkform through a blur + threshold "goo" filter; drawn as plain circles here (as in
// the web port): the geometry is identical, the dots read a touch softer.

pub fn frame_morph(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let seg_t = HOLD + MORPH; // wubble change: the TS's module-level `SEG`
    let cycle = cycle();
    let k_n = cycle.len() as f64;
    let tc = t % (seg_t * k_n);
    // `shape` 0 / 1 / 2 holds one outline instead of cycling
    let held = match o.get("shape") {
        Some(shape) if shape >= 0.0 && shape < k_n => shape.floor(),
        _ => -1.0,
    };
    let k = if held >= 0.0 {
        held
    } else {
        (tc / seg_t).floor()
    };
    let local = if held >= 0.0 {
        t % seg_t
    } else {
        tc - k * seg_t
    };
    let m = if held >= 0.0 {
        0.0
    } else if local > HOLD {
        smooth_e((local - HOLD) / MORPH)
    } else {
        0.0
    };
    let sprd = o.get("spread").unwrap_or(1.0);

    // blend the two shape PATHS at m, then measure the blended outline
    let p_a = &cycle[ix(k)];
    let p_b = if held >= 0.0 {
        p_a
    } else {
        &cycle[ix((k + 1.0) % k_n)]
    };
    let big_m = 160.0;
    let mut pts: Vec<V2> = vec![];
    for i in from(0.0, big_m) {
        let f = i / big_m;
        let a = p_a.at(f);
        let b = p_b.at(f);
        pts.push([
            (a[0] + (b[0] - a[0]) * m) * sprd,
            (a[1] + (b[1] - a[1]) * m) * sprd,
        ]);
    }
    let mut l: Vec<f64> = vec![];
    let mut total = 0.0;
    for i in from(0.0, big_m) {
        let a = pts[ix(i)];
        let b = pts[ix((i + 1.0) % big_m)];
        let len = f64::hypot(b[0] - a[0], b[1] - a[1]);
        l.push(len);
        total += len;
    }

    // dot radius depends ONLY on rDot; the count sets the gaps. Formed shapes breathe a little.
    let n = morph_n(o.get("iconD").unwrap_or(1.0));
    let re = o.get("rDot").unwrap_or(0.021) * 1.35 * sprd;
    let pulse = 1.0 + 0.02 * (local * 3.1).sin();

    let mut dots: Vec<Dot> = vec![];
    let c2 = size / 2.0;
    let mut seg = 0.0;
    let mut acc = 0.0;
    for k2 in from(0.0, n) {
        let target = (k2 / n) * total;
        while acc + l[ix(seg)] < target && seg < big_m - 1.0 {
            acc += l[ix(seg)];
            inc(&mut seg);
        }
        let a = pts[ix(seg)];
        let b = pts[ix((seg + 1.0) % big_m)];
        let f = if nz(l[ix(seg)]) {
            f64::min(1.0, (target - acc) / l[ix(seg)])
        } else {
            0.0
        };
        let x = (a[0] + (b[0] - a[0]) * f) * pulse;
        let y = (a[1] + (b[1] - a[1]) * f) * pulse;
        dots.push(Dot {
            x: c2 + x * size,
            y: c2 + y * size,
            z: 0.0,
            r: f64::max(0.35, re * size),
            white: 0.1,
            a: None,
        });
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}
