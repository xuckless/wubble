// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/braid.ts:8-50
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).

//! Braid: three strands plait around the sphere, the "weaving" state. Each strand runs pole to
//! pole on a helix, and a radial breathing term makes them trade places (the over/under).

use super::prim::{fib_dir, finalize_frame, frac, make_proj, radius_scale};
use super::{Dot, ModeOpts, OrbFrame, from};
use std::f64::consts::PI;

pub fn frame_braid(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = (size / 2.0) * 0.76;
    let pt = make_proj(t * 0.4, 0.3, cx, cy, 1.0);
    let rs = radius_scale(size, o.get("rsPow").unwrap_or(0.6));

    let mut dots: Vec<Dot> = vec![];
    let ghost_n = o.get("ghostN").unwrap_or(150.0);
    for i in from(0.0, ghost_n) {
        let d = fib_dir(i, ghost_n);
        let [px, py, z] = pt(d[0] * r, d[1] * r, d[2] * r);
        let depth = (z / r + 1.0) / 2.0;
        dots.push(Dot {
            x: px,
            y: py,
            z,
            r: 0.8 * rs,
            white: 0.78,
            a: Some(0.1 + 0.22 * depth),
        });
    }

    let strand_n = o.get("strandN").unwrap_or(52.0);
    let turns = o.get("turns").unwrap_or(3.0);
    for s in from(0.0, 3.0) {
        let phase = (s / 3.0) * 2.0 * PI;
        for i in from(0.0, strand_n) {
            // u walks pole to pole; the frac() drift slides the whole strand along
            let u = (frac(i / strand_n + t * 0.045) * 2.0 - 1.0) * 0.96;
            let surf = f64::sqrt(f64::max(0.0, 1.0 - u * u));
            let end_fade = f64::min(1.0, (1.0 - u.abs()) / 0.1);
            let a = u * PI * turns + phase;
            // radial breathing: strands trade places, the over/under of a plait
            let weave = 1.0 + 0.075 * (u * PI * turns * 2.0 + phase * 2.0 + t * 0.8).sin();
            let rr = surf * r * weave;
            let [px, py, zr] = pt(a.cos() * rr, u * r * weave, a.sin() * rr);
            let depth = (zr / r + 1.0) / 2.0;
            dots.push(Dot {
                x: px,
                y: py,
                z: zr,
                r: (o.get("rBase").unwrap_or(1.2) + o.get("rDepth").unwrap_or(1.8) * depth) * rs,
                white: 0.55 - 0.45 * depth,
                a: Some(end_fade * (0.45 + 0.55 * depth)),
            });
        }
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}
