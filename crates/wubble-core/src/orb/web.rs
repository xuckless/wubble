// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/web.ts:8-97
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).

//! Web: a constellation wires itself, the "connecting" state. Nodes drift on the sphere under
//! slow value noise; any pair closer than `thr` grows an edge, and bright packets run along
//! randomly re-picked node pairs.

use super::prim::{fib_dir, finalize_frame, frac, hash_d, lerp, make_proj, radius_scale, vnoise};
use super::{Dot, Line, ModeOpts, OrbFrame, V3, from, ix};

pub fn frame_web(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = (size / 2.0) * 0.8 * o.get("spread").unwrap_or(1.0);
    // the projector carries the radius as its scale, so node vectors stay unit-length and
    // distances below are in unit-sphere space
    let pt = make_proj(t * 0.12, 0.32, cx, cy, r);
    let rs = radius_scale(size, o.get("rsPow").unwrap_or(0.6));

    let node_n = o.get("nodeN").unwrap_or(30.0);
    let thr = o.get("thr").unwrap_or(0.72);
    let node_r = o.get("nodeR").unwrap_or(1.4);
    let node_r_depth = o.get("nodeRDepth").unwrap_or(1.8);

    // nodes: fib lattice + slow noise wander, renormalised to the surface
    let mut nodes: Vec<V3> = vec![];
    for i in from(0.0, node_n) {
        let d = fib_dir(i, node_n);
        let x = d[0] + 0.3 * (vnoise(i * 0.31 + 9.0, t * 0.24) - 0.5) * 2.0;
        let y = d[1] + 0.3 * (vnoise(i * 0.53 + 27.0, t * 0.21) - 0.5) * 2.0;
        let z = d[2] + 0.3 * (vnoise(i * 0.77 + 55.0, t * 0.27) - 0.5) * 2.0;
        let l = f64::sqrt(x * x + y * y + z * z);
        nodes.push([x / l, y / l, z / l]);
    }

    let mut lines: Vec<Line> = vec![];
    let mut dots: Vec<Dot> = vec![];

    // edges between close neighbours, alpha by proximity + depth
    for i in from(0.0, node_n) {
        for j in from(i + 1.0, node_n) {
            let (ni, nj) = (nodes[ix(i)], nodes[ix(j)]);
            let dx = ni[0] - nj[0];
            let dy = ni[1] - nj[1];
            let dz = ni[2] - nj[2];
            let dist = f64::sqrt(dx * dx + dy * dy + dz * dz);
            if dist >= thr {
                continue;
            }
            let [x1, y1, z1] = pt(ni[0], ni[1], ni[2]);
            let [x2, y2, z2] = pt(nj[0], nj[1], nj[2]);
            let depth = ((z1 + z2) / 2.0 + 1.0) / 2.0;
            lines.push(Line {
                x1,
                y1,
                x2,
                y2,
                white: 0.42,
                a: Some((1.0 - dist / thr) * (0.3 + 0.55 * depth)),
                w: f64::max(0.6, o.get("lineW").unwrap_or(0.8) * rs),
            });
        }
    }

    for i in from(0.0, node_n) {
        let n = nodes[ix(i)];
        let [px, py, z] = pt(n[0], n[1], n[2]);
        let depth = (z + 1.0) / 2.0;
        let pulse = 1.0 + 0.25 * (t * 1.4 + i * 2.7).sin();
        dots.push(Dot {
            x: px,
            y: py,
            z,
            r: (node_r + node_r_depth * depth) * pulse * rs,
            white: 0.55 - 0.45 * depth,
            a: None,
        });
    }

    // signals: bright packets running between paired nodes
    let signals = o.get("signals").unwrap_or(5.0);
    for s in from(0.0, signals) {
        let seg = (t * 0.55 + s * 7.31).floor();
        let a = (hash_d(seg, s * 3.1 + 1.7) * node_n).floor();
        let b = (hash_d(seg, s * 5.7 + 4.2) * node_n).floor();
        if a == b {
            continue;
        }
        let f = frac(t * 0.55 + s * 7.31);
        let (na, nb) = (nodes[ix(a)], nodes[ix(b)]);
        let x = lerp(na[0], nb[0], f);
        let y = lerp(na[1], nb[1], f);
        let z = lerp(na[2], nb[2], f);
        let l = f64::max(1e-6, f64::sqrt(x * x + y * y + z * z));
        let [px, py, zr] = pt(x / l, y / l, z / l);
        let depth = (zr + 1.0) / 2.0;
        dots.push(Dot {
            x: px,
            y: py,
            z: zr,
            r: (node_r * 1.5 + node_r_depth * depth) * rs,
            white: 0.05,
            a: Some(0.5 + 0.5 * depth),
        });
    }

    finalize_frame(dots, lines, o.get("rMin"))
}
