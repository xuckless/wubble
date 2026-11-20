// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/lattice.ts:19-205
// Source: Ultra Reactivate's vendored thinking orbs (xuckless/the-wallpaper-project @ 598f814,
// src/ultra-reactivate/vendor/thinking-orbs), from Libraries.dev (MIT, (c) 2026 Jakub Antalik).

//! The sphere-lattice modes: globe (searching), rubik (solving) and wave (listening). All draw a
//! lat/long dot field with their own motion, then hand off to the shared z-sorted painter.

use super::prim::{angle_delta, finalize_frame, hash_d, make_proj, radius_scale};
use super::{Dot, ModeOpts, OrbFrame, V3, from, ix, js_round, through};
use std::f64::consts::PI;

/// One quarter turn of a band (`Move`): about `axis` (0 x, 1 y, 2 z), for coordinates in
/// `lo..hi`.
struct Move {
    axis: f64,
    lo: f64,
    hi: f64,
    ang: f64,
}

struct Cycle {
    amount: Vec<f64>,
    active: f64,
}

// The solver's heartbeat: rapid eased moves scramble, then replay in reverse (a palindrome) so
// everything clicks back to solved, rests, repeats.
fn solve_cycle(time: f64, count: f64, slot_dur: f64, rest: f64) -> Cycle {
    let cyc = 2.0 * count * slot_dur + rest;
    let tc = time % cyc;
    let mut amount = vec![0.0; ix(count)];
    let mut active = -1.0;
    if tc < 2.0 * count * slot_dur {
        let slot = (tc / slot_dur).floor();
        let p = (tc - slot * slot_dur) / slot_dur;
        let cl = f64::min(1.0, p / 0.7);
        let ep = 1.0 - (1.0 - cl).powf(3.0); // machine ease-out
        if slot < count {
            for i in from(0.0, slot) {
                amount[ix(i)] = 1.0;
            }
            amount[ix(slot)] = ep;
            active = slot;
        } else {
            let u = 2.0 * count - 1.0 - slot;
            for i in from(0.0, u) {
                amount[ix(i)] = 1.0;
            }
            amount[ix(u)] = 1.0 - ep;
            active = u;
        }
    }
    Cycle { amount, active }
}

fn apply_moves(pt3: V3, moves: &[Move], sc: &Cycle) -> (f64, f64, f64, bool) {
    let [mut x, mut y, mut z] = pt3;
    let mut in_active = false;
    for i in from(0.0, moves.len() as f64) {
        if sc.amount[ix(i)] <= 0.0 {
            continue;
        }
        let mv = &moves[ix(i)];
        let coord = if mv.axis == 0.0 {
            x
        } else if mv.axis == 1.0 {
            y
        } else {
            z
        };
        if coord < mv.lo || coord >= mv.hi {
            continue;
        }
        if i == sc.active {
            in_active = true;
        }
        let a = mv.ang * sc.amount[ix(i)];
        let ca = a.cos();
        let sa = a.sin();
        if mv.axis == 0.0 {
            let y2 = y * ca - z * sa;
            z = y * sa + z * ca;
            y = y2;
        } else if mv.axis == 1.0 {
            let x2 = x * ca + z * sa;
            z = -x * sa + z * ca;
            x = x2;
        } else {
            let x2 = x * ca - y * sa;
            y = x * sa + y * ca;
            x = x2;
        }
    }
    (x, y, z, in_active)
}

fn make_moves(count: f64) -> Vec<Move> {
    let mut moves: Vec<Move> = vec![];
    for i in from(0.0, count) {
        let axis = f64::min(2.0, (hash_d(i, 2.3) * 3.0).floor()); // wubble change: the TS casts it `as 0 | 1 | 2`
        let lo = -1.0 + 0.5 * f64::min(3.0, (hash_d(i, 5.9) * 4.0).floor()); // wubble change: (the cast above)
        let dir = if hash_d(i, 7.7) < 0.5 { 1.0 } else { -1.0 };
        moves.push(Move {
            axis,
            lo,
            hi: lo + 0.5,
            ang: (dir * PI) / 2.0,
        });
    }
    moves
}

// Globe: lat/long field, a scan meridian sweeps: searching

pub fn frame_globe(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let spin = 0.5;
    let cx = size / 2.0;
    let cy = size / 2.0;
    let radius = (size / 2.0) * 0.82;
    let tilt = 0.4 + 0.06 * (t * 0.35).sin();
    let pt = make_proj(t * spin, tilt, cx, cy, radius);
    // scan sweeps relative to the spin; scanMul scales that relative rate
    let scan = t * (spin + (1.7 - spin) * o.get("scanMul").unwrap_or(1.0));
    let rs = radius_scale(size, o.get("rsPow").unwrap_or(0.6));
    let dim_base = o.get("dimBase").unwrap_or(1.0);

    let mut dots: Vec<Dot> = vec![];
    let lat_rings = o.get("latRings").unwrap_or(17.0);
    let lon_density = o.get("lonDensity").unwrap_or(44.0);
    for li in through(0.0, lat_rings) {
        let lat = -PI / 2.0 + (li / lat_rings) * PI;
        let cos_lat = lat.cos();
        let sin_lat = lat.sin();
        let lon_count = f64::max(1.0, js_round(cos_lat.abs() * lon_density));
        for lj in from(0.0, lon_count) {
            let lon = (lj / lon_count) * 2.0 * PI;
            let [px, py, z] = pt(cos_lat * lon.cos(), sin_lat, cos_lat * lon.sin());
            let depth = (z + 1.0) / 2.0;
            // the scan: a moving meridian read as a size ripple, not a shine
            let d = angle_delta(lon + t * spin, scan);
            let boost = f64::exp(-(d * d) / 0.18) * f64::max(0.0, z);
            dots.push(Dot {
                x: px,
                y: py,
                z,
                r: (o.get("rBase").unwrap_or(0.6)
                    + o.get("rDepth").unwrap_or(1.7) * depth
                    + o.get("rBoost").unwrap_or(1.0) * boost)
                    * rs,
                white: o.get("inkFar").unwrap_or(0.62) - o.get("inkSpan").unwrap_or(0.54) * depth,
                // dimBase < 1 fades un-scanned dots so the meridian reads clearly
                a: Some(dim_base + (1.0 - dim_base) * f64::min(1.0, boost)),
            });
        }
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}

// Rubik: bands twist in quarter turns, scramble then solve: solving

pub fn frame_rubik(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let cx = size / 2.0;
    let cy = size / 2.0;
    let r = (size / 2.0) * 0.82;
    let pt = make_proj(t * 0.55, 0.35 + 0.1 * (t * 0.9).sin(), cx, cy, r);
    let rs = radius_scale(size, o.get("rsPow").unwrap_or(0.6));
    let move_count = o.get("moveCount").unwrap_or(14.0);
    let moves = make_moves(move_count);
    let sc = solve_cycle(t, move_count, 0.42, 1.2);

    let mut dots: Vec<Dot> = vec![];
    let lat_rings = o.get("latRings").unwrap_or(15.0);
    let lon_density = o.get("lonDensity").unwrap_or(40.0);
    for li in through(0.0, lat_rings) {
        let lat = -PI / 2.0 + (li / lat_rings) * PI;
        let cos_lat = lat.cos();
        let sin_lat = lat.sin();
        let lon_count = f64::max(1.0, js_round(cos_lat.abs() * lon_density));
        for lj in from(0.0, lon_count) {
            let lon = (lj / lon_count) * 2.0 * PI;
            let (x, y, z, in_active) = apply_moves(
                [cos_lat * lon.cos(), sin_lat, cos_lat * lon.sin()],
                &moves,
                &sc,
            );
            let [px, py, zr] = pt(x, y, z);
            let depth = (zr + 1.0) / 2.0;
            // the band being turned inks a touch darker: the "hand"
            dots.push(Dot {
                x: px,
                y: py,
                z: zr,
                r: (o.get("rBase").unwrap_or(0.6)
                    + o.get("rDepth").unwrap_or(1.7) * depth
                    + (if in_active {
                        o.get("rActive").unwrap_or(0.3)
                    } else {
                        0.0
                    }))
                    * rs,
                white: o.get("inkFar").unwrap_or(0.62)
                    - o.get("inkSpan").unwrap_or(0.54) * depth
                    - (if in_active { 0.14 } else { 0.0 }),
                a: None,
            });
        }
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}

// Wave: a waveform rolls through the rings: listening

pub fn frame_wave(size: f64, t: f64, o: &ModeOpts) -> OrbFrame {
    let cx = size / 2.0;
    let cy = size / 2.0;
    // 0.76 base × 1.15: the undulation pulls the sphere inward, so wave read ~15% smaller than
    // the other lattice modes; scaled up to match them
    let r = (size / 2.0) * 0.874;
    let pt = make_proj(t * 0.18, 0.38, cx, cy, 1.0);
    let rs = radius_scale(size, o.get("rsPow").unwrap_or(0.6));

    let mut dots: Vec<Dot> = vec![];
    let rings = o.get("rings").unwrap_or(15.0);
    let lon_density = o.get("lonDensity").unwrap_or(40.0);
    for ri in through(0.0, rings) {
        let lat = -PI / 2.0 + (ri / rings) * PI;
        let cos_lat = lat.cos();
        let sin_lat = lat.sin();
        // two waves, different tempi: organic, never quite repeating
        let w = 0.62 * (t * 2.1 - ri * 0.52).sin() + 0.38 * (t * 1.27 + ri * 0.83).sin();
        let rr = r * (0.88 + 0.105 * w);
        let lon_count = f64::max(1.0, js_round(cos_lat.abs() * lon_density));
        for lj in from(0.0, lon_count) {
            let lon = (lj / lon_count) * 2.0 * PI;
            let [px, py, z] = pt(
                cos_lat * lon.cos() * rr,
                sin_lat * rr,
                cos_lat * lon.sin() * rr,
            );
            let depth = (z / r + 1.0) / 2.0;
            let crest = f64::max(0.0, w);
            dots.push(Dot {
                x: px,
                y: py,
                z,
                r: (o.get("rBase").unwrap_or(0.6) + o.get("rDepth").unwrap_or(1.7) * depth)
                    * (1.0 + 0.4 * crest)
                    * rs,
                white: 0.66 - 0.56 * depth - 0.1 * crest,
                a: None,
            });
        }
    }
    finalize_frame(dots, vec![], o.get("rMin"))
}
