// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/orbits.ts:8-80
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).

//! Orbits: particles on tilted orbits, the "working" state. No nucleus: ghost paths and the
//! particles doing the work.

use super::prim::{finalize_frame, hash_d, make_proj, radius_scale};
use super::{Dot, ModeOpts, OrbFrame, from};
use std::f64::consts::PI;

pub fn frame_orbits(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = (size / 2.0) * 0.82;
    let pt = make_proj(t * 0.12, 0.3, cx, cy, 1.0);
    let rs = radius_scale(size, o.get("rsPow").unwrap_or(0.6));

    let mut dots: Vec<Dot> = vec![];
    let orbit_n = o.get("orbitN").unwrap_or(12.0);
    let ghost_n = o.get("ghostN").unwrap_or(40.0);
    let particles = o.get("particles").unwrap_or(3.0);

    // orbits: each a tilted circle, a ghost path + running particles
    for orb in from(0.0, orbit_n) {
        let h1 = hash_d(orb, 1.7);
        let h2 = hash_d(orb, 5.2);
        let h3 = hash_d(orb, 8.9);
        let ro = r * (0.45 + 0.52 * h1);
        let th = h1 * 2.0 * PI;
        let phi = f64::acos(2.0 * h2 - 1.0);
        // orbit plane basis (u, v ⟂ normal n)
        let nx = phi.sin() * th.cos();
        let ny = phi.cos();
        let nz = phi.sin() * th.sin();
        let mut ux = -ny;
        let mut uy = nx;
        let uz = 0.0;
        let ul = f64::max(1e-6, f64::sqrt(ux * ux + uy * uy));
        ux /= ul;
        uy /= ul;
        let vx = ny * uz - nz * uy;
        let vy = nz * ux - nx * uz;
        let vz = nx * uy - ny * ux;
        let speed = (0.25 + 0.55 * h3) * (if h3 > 0.5 { 1.0 } else { -1.0 });

        // ghost path
        for k in from(0.0, ghost_n) {
            let a = (k / ghost_n) * 2.0 * PI;
            let [px, py, z] = pt(
                (ux * a.cos() + vx * a.sin()) * ro,
                (uy * a.cos() + vy * a.sin()) * ro,
                (uz * a.cos() + vz * a.sin()) * ro,
            );
            let depth = (z / ro + 1.0) / 2.0;
            dots.push(Dot {
                x: px,
                y: py,
                z,
                r: o.get("ghostR").unwrap_or(0.9) * rs,
                white: 0.72,
                a: Some(o.get("ghostA").unwrap_or(0.5) * (0.4 + 0.6 * depth)),
            });
        }
        // the particles doing the work
        for m in from(0.0, particles) {
            let a = t * speed + (m / particles) * 2.0 * PI + h2 * 6.0;
            let [px, py, z] = pt(
                (ux * a.cos() + vx * a.sin()) * ro,
                (uy * a.cos() + vy * a.sin()) * ro,
                (uz * a.cos() + vz * a.sin()) * ro,
            );
            let depth = (z / ro + 1.0) / 2.0;
            dots.push(Dot {
                x: px,
                y: py,
                z,
                r: (o.get("partR").unwrap_or(1.2) + o.get("partRDepth").unwrap_or(1.6) * depth)
                    * rs,
                white: 0.3 - 0.22 * depth,
                a: None,
            });
        }
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}
