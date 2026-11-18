//! Deterministic motion maths: springs, rubber band, stretch, easing constants.
//!
//! Ported from the wubble design pack (owner-authored, same repo):
//! `ux/pack/project/Components.dc.html` lines 331-338 and 357-364, and
//! `ux/pack/project/RoomToPlay.dc.html` lines 99-106 and 118-132
//! (`springStep`, `band`, `bandClamp`, presets). Easing values come from the
//! Motion section of `ux/pack/project/Tokens.dc.html`.
//!
//! Everything is pure data plus pure functions. No clocks, no hidden state.

/// Longest time step a spring will integrate, in seconds.
pub const MAX_DT: f32 = 0.032;
/// A spring rests when speed and distance to target are both under this.
pub const REST_EPS: f32 = 0.02;

/// Spring stiffness `k` and damping `c`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub k: f32,
    pub c: f32,
}

// The one preset table.
pub const FOLLOW: Params = Params { k: 300.0, c: 15.0 };
pub const STRETCH: Params = Params { k: 700.0, c: 24.0 };
pub const SEAM_HELD: Params = Params { k: 150.0, c: 20.0 };
pub const SEAM: Params = Params { k: 260.0, c: 24.0 };
pub const SEAM_WOBBLE: Params = Params { k: 520.0, c: 9.0 };
pub const CONTENTS: Params = Params { k: 210.0, c: 21.0 };

/// A spring is plain data; advance it with [`step`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    pub params: Params,
}

impl Spring {
    /// A spring resting at `value`.
    pub fn at(value: f32, params: Params) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
            params,
        }
    }

    /// Aim at a new target, keeping current value and velocity.
    /// A non-finite target is ignored.
    pub fn retarget(self, target: f32) -> Self {
        if !target.is_finite() {
            return self;
        }
        Self { target, ..self }
    }

    pub fn is_at_rest(&self) -> bool {
        self.velocity.abs() < REST_EPS && (self.value - self.target).abs() < REST_EPS
    }
}

/// Semi-implicit Euler step. `dt` is clamped to [0, MAX_DT]; a non-finite or
/// non-positive `dt` returns the spring unchanged. Snaps to target when at rest.
pub fn step(s: Spring, dt: f32) -> Spring {
    if !dt.is_finite() || dt <= 0.0 {
        return s;
    }
    let dt = dt.min(MAX_DT);
    let a = -s.params.k * (s.value - s.target) - s.params.c * s.velocity;
    let mut out = s;
    out.velocity += a * dt;
    out.value += out.velocity * dt;
    // Poisoned state (NaN or inf in value, velocity or target) snaps to a
    // finite target, or rests where it is when the target is bad too.
    if !(out.value.is_finite() && out.velocity.is_finite()) {
        out.value = if out.target.is_finite() {
            out.target
        } else {
            0.0
        };
        out.target = out.value;
        out.velocity = 0.0;
    }
    if out.is_at_rest() {
        out.value = out.target;
        out.velocity = 0.0;
    }
    out
}

pub const BAND_CONTROL: f32 = 26.0;
pub const BAND_SEAM: f32 = 34.0;
pub const BAND_SEAM_BEND: f32 = 70.0;

/// Rubber band: `k * (1 - 1 / (|d| / k + 1))`, sign preserved, bounded by `k`.
/// A non-positive or non-finite `k` gives 0.
pub fn rubber_band(d: f32, k: f32) -> f32 {
    if !(k.is_finite() && k > 0.0) || d == 0.0 || d.is_nan() {
        return 0.0;
    }
    d.signum() * k * (1.0 - 1.0 / (d.abs() / k + 1.0))
}

/// Identity inside `[lo, hi]`, rubber-banded beyond either end.
pub fn rubber_band_clamp(v: f32, lo: f32, hi: f32, k: f32) -> f32 {
    if v < lo {
        lo + rubber_band(v - lo, k)
    } else if v > hi {
        hi + rubber_band(v - hi, k)
    } else {
        v
    }
}

pub const STRETCH_MAX: f32 = 0.55;
pub const STRETCH_REF_SPEED: f32 = 1500.0;

/// Target scale along the travel axis for a speed in px/s.
/// A non-finite speed gives no stretch.
pub fn stretch_target(speed: f32) -> f32 {
    if !speed.is_finite() {
        return 1.0;
    }
    1.0 + (speed.abs() / STRETCH_REF_SPEED).min(STRETCH_MAX)
}

/// Cross-axis scale that preserves volume for along-axis scale `s`.
/// A non-positive or non-finite `s` gives 1.
pub fn cross_scale(s: f32) -> f32 {
    if !(s.is_finite() && s > 0.0) {
        return 1.0;
    }
    1.0 / s.sqrt()
}

/// Velocity smoothing used before driving the stretch target.
pub fn smooth_velocity(prev: f32, instant: f32) -> f32 {
    prev * 0.6 + instant * 0.4
}

/// CSS-style cubic-bezier control points. Slint evaluates the curve itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bezier(pub f32, pub f32, pub f32, pub f32);

pub const EASE: Bezier = Bezier(0.22, 1.0, 0.36, 1.0);
pub const SPRING_EASE: Bezier = Bezier(0.3, 1.55, 0.5, 1.0);
pub const SPRING_SOFT: Bezier = Bezier(0.3, 1.35, 0.5, 1.0);
pub const IN_OUT: Bezier = Bezier(0.65, 0.0, 0.35, 1.0);

#[cfg(test)]
mod tests {
    use super::*;

    const PRESETS: [Params; 6] = [FOLLOW, STRETCH, SEAM_HELD, SEAM, SEAM_WOBBLE, CONTENTS];
    // Step-count bound, not a time bound: 60 s of simulated frames.
    const BOUND: usize = 4000;

    #[test]
    fn presets_converge_exactly() {
        for p in PRESETS {
            let mut s = Spring::at(0.0, p).retarget(100.0);
            let mut n = 0;
            while !s.is_at_rest() || s.value != s.target {
                s = step(s, 0.016);
                n += 1;
                assert!(n < BOUND, "no convergence for {p:?}");
            }
            assert_eq!(s.value, 100.0);
            assert_eq!(s.velocity, 0.0);
        }
    }

    #[test]
    fn rest_thresholds() {
        let p = FOLLOW;
        let near = Spring {
            value: 0.019,
            velocity: 0.019,
            target: 0.0,
            params: p,
        };
        assert!(near.is_at_rest());
        let far = Spring {
            value: 0.021,
            ..near
        };
        assert!(!far.is_at_rest());
        let fast = Spring {
            velocity: 0.021,
            ..near
        };
        assert!(!fast.is_at_rest());
        // Snapping: a tiny residual after a step lands exactly on target.
        let s = step(Spring::at(5.0, p), 0.016);
        assert_eq!((s.value, s.velocity), (5.0, 0.0));
    }

    #[test]
    fn bad_dt_is_finite_or_unchanged() {
        let s = Spring::at(0.0, STRETCH).retarget(50.0);
        for dt in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0] {
            assert_eq!(step(s, dt), s);
        }
        for dt in [1.0, 10.0, 1e9, f32::MAX] {
            let o = step(s, dt);
            assert_eq!(o, step(s, MAX_DT));
        }
    }

    #[test]
    fn huge_dt_long_run_stays_bounded() {
        for p in PRESETS {
            let mut s = Spring::at(0.0, p).retarget(100.0);
            for _ in 0..BOUND {
                s = step(s, 10.0);
                assert!(s.value.is_finite() && s.velocity.is_finite());
                assert!(s.value.abs() < 1000.0);
            }
        }
    }

    #[test]
    fn one_step_matches_reference() {
        let s = step(Spring::at(0.0, FOLLOW).retarget(100.0), 0.016);
        assert!((s.velocity - 480.0).abs() < 1e-3);
        assert!((s.value - 7.68).abs() < 1e-3);
    }

    #[test]
    fn retarget_keeps_value_and_velocity() {
        let mut s = Spring::at(0.0, FOLLOW).retarget(100.0);
        for _ in 0..5 {
            s = step(s, 0.016);
        }
        let r = s.retarget(-40.0);
        assert_eq!(r.value.to_bits(), s.value.to_bits());
        assert_eq!(r.velocity.to_bits(), s.velocity.to_bits());
        assert_eq!(r.target, -40.0);
    }

    #[test]
    fn rubber_band_properties() {
        assert_eq!(rubber_band(0.0, 26.0), 0.0);
        assert!((rubber_band(26.0, 26.0) - 13.0).abs() < 1e-5);
        assert_eq!(rubber_band(-30.0, 26.0), -rubber_band(30.0, 26.0));
        let mut prev = 0.0;
        for i in 1..2000 {
            let r = rubber_band(i as f32, 34.0);
            assert!(r > prev && r < 34.0);
            prev = r;
        }
        assert!(rubber_band(1e30, 26.0) <= 26.0);
        for k in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(rubber_band(10.0, k), 0.0);
        }
        assert_eq!(rubber_band(f32::NAN, 26.0), 0.0);
    }

    #[test]
    fn rubber_band_clamp_identity_and_continuity() {
        assert_eq!(rubber_band_clamp(5.0, 0.0, 10.0, 26.0), 5.0);
        assert_eq!(rubber_band_clamp(0.0, 0.0, 10.0, 26.0), 0.0);
        assert_eq!(rubber_band_clamp(10.0, 0.0, 10.0, 26.0), 10.0);
        assert!(rubber_band_clamp(-5.0, 0.0, 10.0, 26.0) < 0.0);
        assert!(rubber_band_clamp(15.0, 0.0, 10.0, 26.0) > 10.0);
        assert!(rubber_band_clamp(-1e-3, 0.0, 10.0, 26.0).abs() <= 1.1e-3);
        assert!(rubber_band_clamp(1e9, 0.0, 10.0, 26.0) <= 36.0);
    }

    #[test]
    fn stretch_and_volume() {
        assert_eq!(stretch_target(0.0), 1.0);
        assert!((stretch_target(1500.0) - 1.55).abs() < 1e-6);
        assert!((stretch_target(-1500.0) - 1.55).abs() < 1e-6);
        assert_eq!(stretch_target(1e9), 1.55);
        for i in 0..=200 {
            let s = stretch_target(i as f32 * 10.0);
            let c = cross_scale(s);
            assert!((s * c * c - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn non_finite_inputs_are_contained() {
        assert_eq!(stretch_target(f32::NAN), 1.0);
        assert_eq!(stretch_target(f32::INFINITY), 1.0);
        for s in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(cross_scale(s), 1.0);
        }
        let s = Spring::at(3.0, FOLLOW);
        for t in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(s.retarget(t), s);
        }
        let bad = Spring {
            value: f32::NAN,
            velocity: f32::NAN,
            target: 7.0,
            params: FOLLOW,
        };
        let o = step(bad, 0.016);
        assert_eq!((o.value, o.velocity), (7.0, 0.0));
        let worse = Spring {
            target: f32::NAN,
            ..bad
        };
        let o = step(worse, 0.016);
        assert!(o.value.is_finite() && o.target.is_finite() && o.is_at_rest());
    }

    #[test]
    fn smoothing_weights() {
        assert!((smooth_velocity(10.0, 10.0) - 10.0).abs() < 1e-6);
        let mut v = 0.0;
        for _ in 0..100 {
            v = smooth_velocity(v, 500.0);
        }
        assert!((v - 500.0).abs() < 1e-2);
    }

    #[test]
    fn bezier_constants() {
        for b in [EASE, SPRING_EASE, SPRING_SOFT, IN_OUT] {
            assert!((0.0..=1.0).contains(&b.0) && (0.0..=1.0).contains(&b.2));
        }
    }
}
