// translates: ../ultra-reactivate/vendor/thinking-orbs/engine/core.ts:42-99; ../ultra-reactivate/vendor/thinking-orbs/engine/core.ts:113-125; ../ultra-reactivate/vendor/thinking-orbs/engine/core.ts:135-136; ../ultra-reactivate/vendor/thinking-orbs/engine/core.ts:152-153; ../ultra-reactivate/vendor/thinking-orbs/engine/core.ts:173-183; ../ultra-reactivate/vendor/thinking-orbs/engine/core.ts:199-201
// Source: Ultra Reactivate's vendored thinking orbs (the owner's design repo,
// xuckless/the-wallpaper-project @ 598f814, src/ultra-reactivate/vendor/thinking-orbs), copied
// there from Libraries.dev (github.com/Jakubantalik/Libraries.dev, MIT, (c) 2026 Jakub Antalik).
// Translated line for line; the numbers are the TS's, in its order
// (crates/wubble-core/tests/orb_constants.rs).

//! The shared primitives of the dotted orbs (engine/core.ts): value noise, the hash, the
//! Fibonacci sphere, the projection, the ink ramp, and the step that turns a mode's raw dots into
//! a finished, z-sorted frame.

use super::{Dot, Ink, Line, OrbFrame, OrbTint, V3, js_round};
use std::f64::consts::PI;

pub fn lerp(a: f64, b: f64, f: f64) -> f64 {
    a + (b - a) * f
}

pub fn frac(x: f64) -> f64 {
    x - x.floor()
}

/// Value noise on a 2D lattice: smooth, deterministic, cheap.
pub fn vnoise(x: f64, y: f64) -> f64 {
    let xi = x.floor();
    let yi = y.floor();
    let mut fx = x - xi;
    let mut fy = y - yi;
    fx = fx * fx * (3.0 - 2.0 * fx);
    fy = fy * fy * (3.0 - 2.0 * fy);
    let a = hash_d(xi, yi);
    let b = hash_d(xi + 1.0, yi);
    let c = hash_d(xi, yi + 1.0);
    let d = hash_d(xi + 1.0, yi + 1.0);
    a + (b - a) * fx + (c - a) * fy + (a - b - c + d) * fx * fy
}

/// Deterministic hash in [0, 1).
pub fn hash_d(a: f64, b: f64) -> f64 {
    let h = (a * 12.9898 + b * 78.233).sin() * 43758.5453;
    h - h.floor()
}

/// Stable directions on a unit sphere (Fibonacci lattice).
pub fn fib_dir(i: f64, n: f64) -> V3 {
    let golden = PI * (3.0 - f64::sqrt(5.0));
    let y = 1.0 - (2.0 * (i + 0.5)) / n;
    let rad = f64::sqrt(1.0 - y * y);
    let a = i * golden;
    [rad * a.cos(), y, rad * a.sin()]
}

/// Shortest signed angular distance, wrapped to (-π, π].
pub fn angle_delta(a: f64, b: f64) -> f64 {
    f64::atan2((a - b).sin(), (a - b).cos())
}

/// Shared spin + tilt + orthographic projection.
pub fn make_proj(
    yaw: f64,
    tilt: f64,
    cx: f64,
    cy: f64,
    scale: f64,
) -> impl Fn(f64, f64, f64) -> V3 {
    let st = tilt.sin();
    let ct = tilt.cos();
    let sy = yaw.sin();
    let cyw = yaw.cos();
    move |x, y, z| {
        let x1 = x * cyw + z * sy;
        let z1 = -x * sy + z * cyw;
        let y1 = y * ct - z1 * st;
        let z2 = y * st + z1 * ct;
        [cx + x1 * scale, cy - y1 * scale, z2]
    }
}

/// One ink value as a colour (0-255 channels, alpha 0-1): grey by default; with a tint the ramp
/// runs on the tint, toward black on dark substrates and toward white on light ones.
pub fn ink_color(w: f64, alpha: f64, dark: bool, tint: Option<OrbTint>) -> Ink {
    let Some(tint) = tint else {
        let g = js_round((if dark { 1.0 - w } else { w }) * 255.0);
        return [g, g, g, alpha];
    };
    let ramp = |c: f64| {
        js_round(if dark {
            c * (1.0 - w)
        } else {
            c + (255.0 - c) * w
        })
    };
    [ramp(tint.r), ramp(tint.g), ramp(tint.b), alpha]
}

/// The painter's colour for a dot (`paint`): its alpha, its ink clamped to 0..1.
// `Math.min(1, Math.max(0, w))` as written, so the numbers read as the source's
#[allow(clippy::manual_clamp)]
pub fn dot_ink(d: &Dot, dark: bool, tint: Option<OrbTint>) -> Ink {
    let alpha = d.a.unwrap_or(1.0);
    let w = f64::min(1.0, f64::max(0.0, d.white));
    ink_color(w, alpha, dark, tint)
}

/// The same for an edge (`paintLines`).
// `Math.min(1, Math.max(0, w))` as written, so the numbers read as the source's
#[allow(clippy::manual_clamp)]
pub fn line_ink(l: &Line, dark: bool, tint: Option<OrbTint>) -> Ink {
    let alpha = l.a.unwrap_or(1.0);
    let w = f64::min(1.0, f64::max(0.0, l.white));
    ink_color(w, alpha, dark, tint)
}

/// Raw mode output to a finished frame: invisible marks dropped, radii clamped to the mode's
/// floor, dots z-sorted far to near (a stable sort, as V8's).
pub fn finalize_frame(dots: Vec<Dot>, lines: Vec<Line>, r_min: Option<f64>) -> OrbFrame {
    let r_min = r_min.unwrap_or(0.3);
    let mut visible: Vec<Dot> = vec![];
    for mut d in dots {
        if d.a.unwrap_or(1.0) < 0.02 {
            continue;
        }
        d.r = f64::max(r_min, d.r);
        visible.push(d);
    }
    visible.sort_by(|a, b| a.z.partial_cmp(&b.z).unwrap_or(std::cmp::Ordering::Equal));
    let lines = lines
        .into_iter()
        .filter(|l| l.a.unwrap_or(1.0) >= 0.02)
        .collect();
    OrbFrame {
        dots: visible,
        lines,
    }
}

/// Dot radii were tuned for a 300 pt frame; sub-linear scaling keeps small spinners legible.
pub fn radius_scale(size: f64, pow: f64) -> f64 {
    (size / 300.0).powf(pow)
}
