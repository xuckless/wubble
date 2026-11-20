// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/ribbon.ts:12-93
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).

//! Ribbon: an undulating sash of parallel strands rides a great circle, the "composing" state
//! (the tuned preset freezes the tumble, spin 0). The same geometry drives "breathing" (ring)
//! through `faceOn`: a face-on circle whose radius undulates.

use super::prim::{fib_dir, finalize_frame, make_proj, radius_scale};
use super::{Dot, ModeOpts, OrbFrame, from, js_round, truthy};
use std::f64::consts::PI;

pub fn frame_ribbon(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = (size / 2.0) * 0.78;
    // spin scales the 3D tumble; spin = 0 freezes the band's orientation
    let spin = o.get("spin").unwrap_or(1.0);
    let cam_tilt = 0.3;
    let pt = make_proj(t * 0.1 * spin, cam_tilt, cx, cy, 1.0);
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

    // The band plane, precessing (frozen when spin = 0). Face-on sets ta = -camTilt so the
    // projection's cos(ta + camTilt) is 1 and the band reads as a true circle.
    let ya = t * 0.24 * spin;
    let ta = if truthy(o.get("faceOn")) {
        -cam_tilt
    } else {
        0.55 + 0.3 * (t * 0.18).sin() * spin
    };
    let ux = ya.cos();
    let uy = 0.0;
    let uz = ya.sin();
    let vx = -uz * ta.sin();
    let vy = ta.cos();
    let vz = ux * ta.sin();
    // plane normal n = u × v
    let nx = uy * vz - uz * vy;
    let ny = uz * vx - ux * vz;
    let nz = ux * vy - uy * vx;

    // Radial lobes swell past R, so pull the base radius in by (most of) the wobble amplitude.
    let wob_amp = 0.23 * o.get("wobMul").unwrap_or(1.0);
    let base_r = if truthy(o.get("faceOn")) {
        r / (1.0 + 0.85 * wob_amp)
    } else {
        r
    };

    let base_lanes = o.get("lanes").unwrap_or(5.0);
    let segs = o.get("segs").unwrap_or(88.0);
    let lanes = f64::max(1.0, js_round(base_lanes * o.get("bandMul").unwrap_or(1.0)));
    for w in from(0.0, lanes) {
        let lane_off = (w - (lanes - 1.0) / 2.0) * 0.075;
        let edge = (w - (lanes - 1.0) / 2.0).abs() / f64::max(1.0, (lanes - 1.0) / 2.0);
        for k in from(0.0, segs) {
            let a = (k / segs) * 2.0 * PI;
            // the undulation: two travelling waves along the band; wobMul scales it
            let wob = (0.16 * (a * 3.0 - t * 1.7 + w * 0.22).sin()
                + 0.07 * (a * 5.0 + t * 1.1).sin())
                * o.get("wobMul").unwrap_or(1.0);
            // ribbon keeps the out-of-plane sash wobble; face-on modulates the in-plane radius
            let radial = if truthy(o.get("faceOn")) {
                1.0 + wob
            } else {
                1.0
            };
            let off = if truthy(o.get("faceOn")) {
                lane_off
            } else {
                lane_off + wob
            };
            let x = ux * a.cos() + vx * a.sin() + nx * off;
            let y = uy * a.cos() + vy * a.sin() + ny * off;
            let z = uz * a.cos() + vz * a.sin() + nz * off;
            let l = f64::sqrt(x * x + y * y + z * z);
            let rr = base_r * radial;
            let [px, py, zr] = pt((x / l) * rr, (y / l) * rr, (z / l) * rr);
            let depth = (zr / r + 1.0) / 2.0;
            dots.push(Dot {
                x: px,
                y: py,
                z: zr,
                r: (o.get("rBase").unwrap_or(1.1) + o.get("rDepth").unwrap_or(1.7) * depth)
                    * (1.0 - 0.25 * edge)
                    * rs,
                white: 0.52 - 0.44 * depth + 0.18 * edge,
                a: Some(0.4 + 0.6 * depth),
            });
        }
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}
