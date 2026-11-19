//! The component sheet's physics: slider, toggle, segmented control and swatches, and when a
//! control's film rim shows. Pure; the frame driver steps them and Slint draws their views. The
//! pin, download stretch, dock magnify and tooltip land with their components.
//!
//! Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
//! `src/wubble/pages/touch.tsx` (`TOGGLE`; @source touch-toggle). The bucket has these only as
//! TSX: the loops are ported here onto the springs of [`crate::motion`], the constants copied
//! verbatim. The slider, the segmented control and the swatches' ring are in their own modules
//! (controls kit B), with their constants from `kit_gen`.
//! Check boxes and menus are CSS transitions in the design and stay Slint tweens.
//!
//! Pointer positions are px along the control's own axis; time is `dt` in seconds or `now` in ms
//! passed in, never read.

pub mod segmented;
pub mod slider;
pub mod swatches;

pub use segmented::{SegmentView, Segmented};
pub use slider::{Slider, SliderView};
pub use swatches::{Ring, RingView};

use crate::motion::{
    Spring, Stretch, Velocity, band_clamp, clamp, spring, squash_at, squash_of, step_k,
};

// @source touch-toggle
#[allow(non_snake_case, non_upper_case_globals)]
pub mod TOGGLE {
    /// px: track 92 − padding 2×5 − knob 42
    pub const travel: f64 = 40.0;
    pub const follow: [f64; 2] = crate::motion::K::follow;
    pub const band: f64 = 26.0;
    pub const hold: [f64; 2] = [700.0, 24.0];
    pub const holdScale: f64 = 1.14;
    /// px/s: hitting the wall faster than this squashes the knob
    pub const landing: f64 = 90.0;
    /// px/s for the full-strength squash
    pub const fullSquash: f64 = 650.0;
    /// s of drag velocity projected when deciding the side
    pub const fling: f64 = 0.12;
    /// px before a press becomes a drag
    pub const slop: f64 = 3.0;
}

/// What the toggle draws. Lengths are in the design's 92 px track; Slint scales them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToggleView {
    /// Knob offset along the travel, 0 (off) ..= [`TOGGLE::travel`] (on), banded past both.
    pub x: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Transform origin across the knob (0 left edge, 1 right edge).
    pub origin: f64,
    /// How far the accent fill has bloomed out of the knob, 0..=1.
    pub fill: f64,
    pub on: bool,
}

#[derive(Clone, Copy, Debug)]
struct Squash {
    /// 0..1 of the squash token; 1 = at rest
    t: f64,
    amp: f64,
    side: f64,
}

/// The toggle: the knob rides the follow spring and cannot overshoot the track; the wall takes
/// its speed as a squash. Hold widens it, a drag follows 1:1, a fling decides the side.
#[derive(Clone, Debug)]
pub struct Toggle {
    on: bool,
    x: Spring,
    st: Stretch,
    hs: Spring,
    held: bool,
    drag: bool,
    x0: f64,
    p0: f64,
    raw: f64,
    vel: Velocity,
    q: Squash,
}

impl Toggle {
    pub fn new(on: bool) -> Self {
        Self {
            on,
            x: spring(if on { TOGGLE::travel } else { 0.0 }),
            st: Stretch::new(),
            hs: spring(1.0),
            held: false,
            drag: false,
            x0: 0.0,
            p0: 0.0,
            raw: 0.0,
            vel: Velocity::new(),
            q: Squash {
                t: 1.0,
                amp: 1.0,
                side: 1.0,
            },
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// A value from outside (the setting, the keyboard).
    pub fn set(&mut self, on: bool) {
        self.on = on;
    }

    /// `px` along the track, `t_ms` the event's time (for the fling's velocity).
    pub fn press(&mut self, px: f64, t_ms: f64) {
        self.held = true;
        self.drag = false;
        self.x0 = px;
        self.raw = px;
        self.p0 = self.x.x;
        self.vel.reset();
        self.vel.push(px, 0.0, t_ms);
    }

    pub fn drag(&mut self, px: f64, t_ms: f64) {
        if !self.held {
            return;
        }
        self.raw = px;
        self.vel.push(px, 0.0, t_ms);
        if !self.drag && (self.raw - self.x0).abs() > TOGGLE::slop {
            self.drag = true;
        }
    }

    /// The side it lands on, if it was held: a tap flips, a drag lands where it is flung.
    pub fn release(&mut self) -> Option<bool> {
        if !self.held {
            return None;
        }
        self.held = false;
        let on = if self.drag {
            self.drag = false;
            self.x.x + self.vel.get().0 * TOGGLE::fling > TOGGLE::travel / 2.0
        } else {
            !self.on
        };
        self.on = on;
        Some(on)
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        let t = TOGGLE::travel;
        if self.held && self.drag {
            self.x = spring(band_clamp(
                self.p0 + self.raw - self.x0,
                0.0,
                t,
                TOGGLE::band,
            ));
            moving = true;
        } else {
            let tgt = if self.on { t } else { 0.0 };
            moving |= step_k(&mut self.x, tgt, TOGGLE::follow, dt);
            let into = if self.on {
                self.x.x > t && self.x.v > 0.0
            } else {
                self.x.x < 0.0 && self.x.v < 0.0
            };
            if into {
                // the wall takes the energy
                let speed = self.x.v.abs();
                self.x = spring(tgt);
                if speed > TOGGLE::landing {
                    self.q = Squash {
                        t: 0.0,
                        amp: clamp(speed / TOGGLE::fullSquash, 0.35, 1.0),
                        side: if self.on { 1.0 } else { -1.0 },
                    };
                }
            }
        }
        if self.q.t < 1.0 {
            self.q.t += dt * 1000.0 / crate::motion::SQUASH_MS;
            moving = true;
        }
        let hold = if self.held { TOGGLE::holdScale } else { 1.0 };
        moving |= step_k(&mut self.hs, hold, TOGGLE::hold, dt);
        moving |= self.st.update(self.x.x, dt);
        moving
    }

    pub fn view(&self) -> ToggleView {
        let k = self.st.x().min(1.35) * self.hs.x;
        let (qx, qy) = squash_at(self.q.t, self.q.amp);
        let squashing = self.q.t < 1.0;
        // held: widen away from the travel; moving: from the trailing side; squash: the wall hit
        let origin = if squashing {
            if self.q.side > 0.0 { 1.0 } else { 0.0 }
        } else if self.held {
            if self.on { 1.0 } else { 0.0 }
        } else if self.st.dir > 0.0 {
            0.0
        } else {
            1.0
        };
        ToggleView {
            x: self.x.x,
            scale_x: k * qx,
            scale_y: squash_of(k) * qy,
            origin,
            fill: clamp(self.x.x / TOGGLE::travel, 0.0, 1.0).powf(0.8),
            on: self.on,
        }
    }
}

/// Whether a control's film rim shows: hover, press, drag or focus; never at rest (the design's
/// `wb-film`: "every touched rim").
pub fn rim_on(hover: bool, pressed: bool, dragging: bool, focused: bool) -> bool {
    hover || pressed || dragging || focused
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    fn settle(mut f: impl FnMut() -> bool) -> usize {
        let mut n = 0;
        while f() {
            n += 1;
            assert!(n < 20_000, "never settled");
        }
        n
    }

    #[test]
    fn toggle_lands_with_a_squash_that_scales_with_speed() {
        let mut t = Toggle::new(false);
        t.press(10.0, 0.0);
        assert_eq!(t.release(), Some(true));
        let mut peak: f64 = 1.0;
        settle(|| {
            let m = t.step(DT);
            peak = peak.max(t.view().scale_y.max(t.view().scale_x));
            m
        });
        assert!(t.q.amp >= 0.35, "the wall squashed it");
        assert!(peak > 1.05);
        let v = t.view();
        assert_eq!((v.x, v.fill), (TOGGLE::travel, 1.0));
        assert!((v.scale_x - 1.0).abs() < 1e-3 && (v.scale_y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_fling_decides_the_side() {
        let mut t = Toggle::new(false);
        t.press(0.0, 0.0);
        // a short fast drag right: the knob is under half way, the fling carries it over
        t.drag(8.0, 10.0);
        t.drag(16.0, 20.0);
        t.step(DT);
        assert!(t.view().x < TOGGLE::travel / 2.0);
        assert_eq!(t.release(), Some(true));
        let mut t = Toggle::new(true);
        t.press(40.0, 0.0);
        t.drag(36.0, 100.0);
        t.drag(35.0, 200.0);
        assert_eq!(t.release(), Some(true), "slow and short: stays on");
    }

    #[test]
    fn rim_shows_only_when_touched() {
        assert!(!rim_on(false, false, false, false));
        for i in 0..4 {
            let b = |n| i == n;
            assert!(rim_on(b(0), b(1), b(2), b(3)));
        }
    }
}
