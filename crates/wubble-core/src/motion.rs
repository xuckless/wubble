// Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
// `src/wubble/slint/motion.rs`, copied verbatim except the marked `wubble change`s.
// wubble change: `FrameClock` is not here (it reads `Instant`; wubble-core has no clocks).
// The frame clock lives in `wubble_ui::driver`.

//! Wubble physics, ported 1:1 from `core/physics.ts`: springs, rubber bands, stretch and
//! velocity tracking. Framework-free and `std`-only.
//!
//! Why this lives in Rust and not in Slint: Slint's `animate` blocks are duration + easing
//! tweens. They have no spring, no velocity and cannot retarget from the current velocity, so
//! anything that follows the finger, overshoots once, or is flung (tab indicator, seams,
//! window, the bubble) is integrated here and pushed into Slint `in` properties each frame
//! (see `driver.rs`). Plain fades and the 0.97 press stay in Slint `animate`.
//!
//! Numerics: JS numbers are f64, so everything here is f64. dt is in seconds.

// @source slint-rubber-band
/// `Math.sign` semantics: 0 stays 0 (Rust's `f64::signum(0.0)` is 1.0, which would differ).
#[inline]
pub fn js_sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v // keeps 0.0 / -0.0 / NaN like Math.sign
    }
}

/// Rubber band: an overshoot `d` (px) maps onto an asymptote of `k` px.
/// `k·(1 − 1/(|d|/k + 1))`. k 26 on controls, 34 on seams and scroll ends, 60 on window size.
#[inline]
pub fn band(d: f64, k: f64) -> f64 {
    js_sign(d) * k * (1.0 - 1.0 / (d.abs() / k + 1.0))
}

/// Clamp with rubber banding past both ends.
#[inline]
pub fn band_clamp(v: f64, min: f64, max: f64, k: f64) -> f64 {
    if v < min {
        min + band(v - min, k)
    } else if v > max {
        max + band(v - max, k)
    } else {
        v
    }
}

/// `Math.max(min, Math.min(max, v))`.
#[inline]
pub fn clamp(v: f64, min: f64, max: f64) -> f64 {
    min.max(max.min(v))
}

#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

// @source slint-spring
/// A damped spring's state: position and velocity.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spring {
    pub x: f64,
    pub v: f64,
}

/// `spring(x = 0)`: at rest at `x`.
#[inline]
pub const fn spring(x: f64) -> Spring {
    Spring { x, v: 0.0 }
}

// @source slint-physics-constants
/// Rubber-band limits (k, px) per kind of thing being pulled past its end (`BAND_K` in physics.ts).
#[allow(non_snake_case, non_upper_case_globals)]
pub mod BAND_K {
    pub const controls: f64 = 26.0;
    pub const seams: f64 = 34.0;
    pub const scroll: f64 = 34.0;
    pub const window: f64 = 60.0;
}
/// Rest threshold: a spring rests when |v| and |x − target| are both under this.
pub const REST: f64 = 0.02;
/// dt cap in seconds (32 ms), so a stalled tab never explodes a spring. Applied by the frame
/// driver (`wubble_ui::driver`), not by `step`.
pub const DT_CAP: f64 = 0.032;

// wubble change: reduced motion, ported from `pages/touch-kit.tsx` `RM` / `spr` / `stretchOf` /
// `squashAt` (@source touch-loop): one flag read inside the three chokepoints, so every spring,
// stretch and squash built on them snaps with no per-caller change. Per thread: the UI is single
// threaded, and `cargo test`'s parallel tests stay independent.
thread_local! {
    static REDUCED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// wubble change: true while motion is reduced (the setting, or the OS preference): springs land
/// at once, stretch stays 1, squash is cut.
#[inline]
pub fn reduced() -> bool {
    REDUCED.with(std::cell::Cell::get)
}

/// wubble change: set the reduced-motion flag for this thread (the host's one writer).
pub fn set_reduced(on: bool) {
    REDUCED.with(|r| r.set(on));
}

/// wubble change: the `ui.reduce_motion` setting ("Calmer motion"): on, off, or follow the OS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Calm {
    #[default]
    Auto,
    On,
    Off,
}

impl Calm {
    /// From the setting's stored choice; unknown values are `Auto`.
    pub fn parse(s: &str) -> Self {
        match s {
            "on" => Calm::On,
            "off" => Calm::Off,
            _ => Calm::Auto,
        }
    }

    /// Whether motion is reduced, given the OS preference.
    pub fn reduced(self, os: bool) -> bool {
        match self {
            Calm::On => true,
            Calm::Off => false,
            Calm::Auto => os,
        }
    }
}

/// One semi-implicit Euler step of a damped spring. Returns true while still moving.
/// Velocity first, then position from the new velocity (same order as physics.ts).
/// wubble change: under [`reduced`] it lands at once (the design's `spr`).
#[inline]
pub fn step(s: &mut Spring, target: f64, k: f64, c: f64, dt: f64) -> bool {
    if reduced() {
        s.x = target;
        s.v = 0.0;
        return false;
    }
    let a = -k * (s.x - target) - c * s.v;
    s.v += a * dt;
    s.x += s.v * dt;
    if s.v.abs() < REST && (s.x - target).abs() < REST {
        s.x = target;
        s.v = 0.0;
        return false;
    }
    true
}

// @source slint-spring-presets
/// Spring presets from the tokens board: `[k, c]`. Same names as `K` in physics.ts.
#[allow(non_snake_case, non_upper_case_globals)]
pub mod K {
    pub const follow: [f64; 2] = [300.0, 15.0];
    pub const stretch: [f64; 2] = [700.0, 24.0];
    pub const window: [f64; 2] = [380.0, 30.0];
    pub const seamHeld: [f64; 2] = [150.0, 20.0];
    pub const seamRelease: [f64; 2] = [260.0, 24.0];
    pub const seamBend: [f64; 2] = [520.0, 9.0];
    pub const contents: [f64; 2] = [210.0, 21.0];
    pub const pop: [f64; 2] = [520.0, 16.0];
}

/// Convenience: `step` with a `[k, c]` preset.
#[inline]
pub fn step_k(s: &mut Spring, target: f64, kc: [f64; 2], dt: f64) -> bool {
    step(s, target, kc[0], kc[1], dt)
}

// @source slint-stretch
/// Stretch factor for a moving part: grows with speed up to +35%, springing on its own so it
/// lags and overshoots a little. Pair with `squash_of` for the cross axis.
#[derive(Clone, Debug)]
pub struct Stretch {
    pub s: Spring,
    pub vel: f64,
    pub dir: f64,
    last: f64, // NaN until the first update, like the TS
}

impl Default for Stretch {
    fn default() -> Self {
        Self::new()
    }
}

impl Stretch {
    pub fn new() -> Self {
        Stretch {
            s: spring(1.0),
            vel: 0.0,
            dir: 1.0,
            last: f64::NAN,
        }
    }

    /// Feed the moving value once per frame (defaults `max = 0.35`, `full = 1500` px/s).
    /// Returns true while the stretch is still settling.
    pub fn update(&mut self, value: f64, dt: f64) -> bool {
        self.update_with(value, dt, 0.35, 1500.0)
    }

    pub fn update_with(&mut self, value: f64, dt: f64, max: f64, full: f64) -> bool {
        // wubble change: under reduced motion the stretch stays 1 (the design's `stretchOf`).
        if reduced() {
            self.last = value;
            self.vel = 0.0;
            self.s = spring(1.0);
            return false;
        }
        let inst = if self.last.is_nan() {
            0.0
        } else {
            (value - self.last) / dt.max(1e-3)
        };
        self.last = value;
        self.vel = self.vel * 0.6 + inst * 0.4;
        if self.vel.abs() > 30.0 {
            self.dir = js_sign(self.vel);
        }
        let moving = step(
            &mut self.s,
            1.0 + max.min(self.vel.abs() / full),
            K::stretch[0],
            K::stretch[1],
            dt,
        );
        moving || self.vel.abs() > 1.0
    }

    #[inline]
    pub fn x(&self) -> f64 {
        self.s.x
    }

    /// wubble change: take `value` as the last one seen without measuring a velocity (a frame
    /// with no time passed: the size jumped, nothing moved at a speed). Returns whether it moved
    /// from a known last value: the caller then keeps its clock running, so the next frame
    /// measures a speed.
    pub fn seed(&mut self, value: f64) -> bool {
        let moved = !self.last.is_nan() && self.last != value;
        self.last = value;
        moved
    }

    /// wubble change: forget the last value and the velocity (the part was not stepped for a
    /// while, e.g. off screen): the next update measures from scratch.
    pub fn forget(&mut self) {
        self.last = f64::NAN;
        self.vel = 0.0;
    }
}

/// Cross-axis scale that keeps area constant.
#[inline]
pub fn squash_of(s: f64) -> f64 {
    1.0 / s.sqrt()
}

// wubble change: ported from `pages/touch-kit.tsx` `SQUASH` / `squashAt` (@source touch-squash),
// the one keyframe table the controls share (toggle landing, buttons, check boxes).
/// The squash token as keys of `(t, sx, sy)` over [`SQUASH_MS`]:
/// 1.30×0.78 → 0.93×1.07 → 1.03×0.98 (README › Motion tokens › Squash).
pub const SQUASH_KEYS: [(f64, f64, f64); 5] = [
    (0.0, 1.0, 1.0),
    (0.28, 1.3, 0.78),
    (0.58, 0.93, 1.07),
    (0.8, 1.03, 0.98),
    (1.0, 1.0, 1.0),
];
/// The squash token's length.
pub const SQUASH_MS: f64 = 500.0;

/// The squash at `t` (0..1 of [`SQUASH_MS`]), its strength scaled by `amp` (1 = the token).
/// Ease-out between keys; outside 0..1 it is at rest, `(1, 1)`.
pub fn squash_at(t: f64, amp: f64) -> (f64, f64) {
    // wubble change: cut under reduced motion (the design's `squashAt`).
    if reduced() || !(0.0..1.0).contains(&t) {
        return (1.0, 1.0);
    }
    let k = &SQUASH_KEYS;
    let mut i = 0;
    while i < k.len() - 2 && t > k[i + 1].0 {
        i += 1;
    }
    let (a, b) = (k[i], k[i + 1]);
    let u = (t - a.0) / (b.0 - a.0);
    let e = 1.0 - (1.0 - u) * (1.0 - u);
    let sx = a.1 + (b.1 - a.1) * e;
    let sy = a.2 + (b.2 - a.2) * e;
    (1.0 + (sx - 1.0) * amp, 1.0 + (sy - 1.0) * amp)
}

// @source slint-velocity
/// Tracks pointer velocity (px/s) over the last ~80 ms, for flings.
/// `t` is in milliseconds (the TS uses `performance.now()`); pass your event timestamp.
#[derive(Clone, Debug, Default)]
pub struct Velocity {
    pts: std::collections::VecDeque<(f64, f64, f64)>, // (t ms, x, y)
}

impl Velocity {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, x: f64, y: f64, t: f64) {
        self.pts.push_back((t, x, y));
        while self.pts.len() > 2 && t - self.pts[0].0 > 80.0 {
            self.pts.pop_front();
        }
    }

    /// `{ x, y }` in px/s.
    pub fn get(&self) -> (f64, f64) {
        if self.pts.len() < 2 {
            return (0.0, 0.0);
        }
        let a = self.pts[0];
        let b = self.pts[self.pts.len() - 1];
        let dt = (b.0 - a.0).max(1.0) / 1000.0;
        ((b.1 - a.1) / dt, (b.2 - a.2) / dt)
    }

    pub fn reset(&mut self) {
        self.pts.clear();
    }
}

// @source slint-ease
/// CSS easings from the tokens, as cubic-bezier control points (x1, y1, x2, y2).
/// The same curves are Slint `easing` values in `tokens.slint` (`Motion.ease-*`).
#[allow(non_upper_case_globals)]
pub mod ease {
    pub const out: [f64; 4] = [0.22, 1.0, 0.36, 1.0];
    pub const inOut: [f64; 4] = [0.65, 0.0, 0.35, 1.0];
    pub const spring: [f64; 4] = [0.3, 1.55, 0.5, 1.0];
    /// Only in physics.ts `EASE.in` (not in tokens.ts).
    pub const r#in: [f64; 4] = [0.55, 0.0, 1.0, 0.45];
}

/// A CSS `cubic-bezier(x1, y1, x2, y2)` easing at `t` (0..1): the curve's y where its x is `t`,
/// for tweens a driver times itself (the same curve Slint's `animate` runs). wubble: the design
/// has CSS run them. x is solved by bisection (the x polynomial is monotonic for x1, x2 in 0..1).
pub fn bezier(p: [f64; 4], t: f64) -> f64 {
    // the ends exactly, so a finished tween rests on 1 (an opacity of 0.99999 still costs a layer)
    if t <= 0.0 || t >= 1.0 {
        return t.clamp(0.0, 1.0);
    }
    let at = |a: f64, b: f64, u: f64| {
        3.0 * a * u * (1.0 - u).powi(2) + 3.0 * b * u * u * (1.0 - u) + u.powi(3)
    };
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if at(p[0], p[2], mid) < t {
            lo = mid
        } else {
            hi = mid
        }
    }
    at(p[1], p[3], (lo + hi) / 2.0)
}

/// How far the Overlap entrance (`wb-in` in core/wubble.css: 240 ms on the ease-out curve) is,
/// 0..1, `secs` after it began. The drawing side maps it as `wb-in` does: opacity e, 7 px below
/// → in place, scale 0.98 → 1 (Slint `Entering`; the 4 px blur is left out).
pub fn overlap_in(secs: f64) -> f64 {
    bezier(ease::out, secs / OVERLAP_IN)
}

/// `motion.overlap.in` in tokens.ts, seconds.
pub const OVERLAP_IN: f64 = 0.24;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_snaps_springs_stretch_and_squash() {
        set_reduced(true);
        let mut s = spring(0.0);
        assert!(!step(&mut s, 100.0, 300.0, 15.0, 0.016));
        assert_eq!(s, spring(100.0));
        let mut st = Stretch::new();
        assert!(!st.update(0.0, 0.016));
        assert!(!st.update(500.0, 0.016));
        assert_eq!(st.x(), 1.0);
        assert_eq!(squash_at(0.3, 1.0), (1.0, 1.0));
        set_reduced(false);
        assert!(Calm::parse("on").reduced(false));
        assert!(!Calm::parse("off").reduced(true));
        assert!(Calm::parse("auto").reduced(true) && !Calm::Auto.reduced(false));
        let mut s = spring(0.0);
        assert!(step(&mut s, 100.0, 300.0, 15.0, 0.016));
        assert!(s.x < 100.0);
        assert_ne!(squash_at(0.3, 1.0), (1.0, 1.0));
    }

    #[test]
    fn bezier_matches_the_css_curves() {
        assert_eq!(bezier(ease::out, 0.0), 0.0);
        assert!((bezier(ease::out, 1.0) - 1.0).abs() < 1e-9);
        // linear control points are the identity
        assert!((bezier([1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0], 0.3) - 0.3).abs() < 1e-9);
        // ease-out is front-loaded, in-out symmetric
        assert!(bezier(ease::out, 0.25) > 0.6);
        assert!((bezier(ease::inOut, 0.5) - 0.5).abs() < 1e-9);
        // the spring curve overshoots
        assert!((0..100).any(|i| bezier(ease::spring, i as f64 / 100.0) > 1.0));
    }

    #[test]
    fn overlap_in_runs_from_below_into_place() {
        assert_eq!(overlap_in(0.0), 0.0);
        assert_eq!(overlap_in(OVERLAP_IN), 1.0);
        assert!(
            overlap_in(0.06) > 0.5,
            "ease-out: most of the way in a quarter of the time"
        );
        assert_eq!(overlap_in(10.0), overlap_in(OVERLAP_IN));
    }

    #[test]
    fn spring_settles_exactly_on_target() {
        let mut s = spring(0.0);
        let mut n = 0;
        while step(&mut s, 100.0, K::follow[0], K::follow[1], 1.0 / 60.0) {
            n += 1;
            assert!(n < 10_000);
        }
        assert_eq!(s.x, 100.0);
        assert_eq!(s.v, 0.0);
    }

    #[test]
    fn band_is_asymptotic_and_signed() {
        assert_eq!(band(0.0, 26.0), 0.0);
        assert!(band(1e9, 26.0) < 26.0);
        assert!((band(-26.0, 26.0) + 13.0).abs() < 1e-12);
        assert_eq!(band_clamp(5.0, 0.0, 10.0, 26.0), 5.0);
    }

    #[test]
    fn stretch_rests_at_one() {
        let mut st = Stretch::new();
        let mut y = 0.0;
        for _ in 0..20 {
            y += 20.0;
            st.update(y, 1.0 / 60.0);
        }
        assert!(st.x() > 1.0);
        let mut n = 0;
        while st.update(y, 1.0 / 60.0) {
            n += 1;
            assert!(n < 10_000);
        }
        // rests at 1 + |vel|/1500 with |vel| <= 1, i.e. within 0.0007 of 1 (same as the TS)
        assert!((st.x() - 1.0).abs() < 1.0 / 1500.0 + 1e-12);
    }

    #[test]
    fn squash_hits_its_keys_and_rests_outside() {
        assert_eq!(squash_at(0.28, 1.0), (1.3, 0.78));
        assert_eq!(squash_at(0.58, 1.0), (0.93, 1.07));
        let (x, y) = squash_at(0.8, 1.0);
        assert!((x - 1.03).abs() < 1e-12 && (y - 0.98).abs() < 1e-12);
        assert_eq!(squash_at(1.0, 1.0), (1.0, 1.0));
        assert_eq!(squash_at(-0.1, 1.0), (1.0, 1.0));
        let (x, _) = squash_at(0.28, 0.5);
        assert!((x - 1.15).abs() < 1e-12);
    }

    /// The stretch and banded-spring part of the design repo's parity example, against the TS
    /// run stored in the fixture (second line). Exact comparison.
    #[test]
    fn parity_with_ts() {
        let line = include_str!("../tests/fixtures/ts_parity.jsonl")
            .lines()
            .nth(1)
            .unwrap();
        let ts: serde_json::Value = serde_json::from_str(line).unwrap();
        let dt = 1.0 / 60.0;
        let mut st = Stretch::new();
        let mut y = 0.0;
        let mut out = vec![];
        for i in 0..40 {
            y += if i < 15 { 25.0 } else { 0.0 };
            st.update(y, dt);
            out.push(st.x());
        }
        assert_eq!(out[10], ts["st10"].as_f64().unwrap());
        assert_eq!(out[39], ts["st39"].as_f64().unwrap());
        let mut sp = spring(0.0);
        for _ in 0..50 {
            step(&mut sp, band(300.0, 34.0), 300.0, 15.0, dt);
        }
        assert_eq!(sp.x, ts["s"].as_f64().unwrap());
    }
}
