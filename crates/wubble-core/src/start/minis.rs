// Source: the owner's design repo (xuckless/the-wallpaper-project @ 598f814),
// `src/wubble/slint/minis.rs` (a 1:1 port of `core/minis.ts`), with the marked `wubble change`s.
// Every tuning number comes from `kit_gen::minis` (read from `core/minis.ts`).

//! Little bubbles: born from a point (a touch, a confirmation), they rise with buoyancy,
//! wander, get pushed off obstacles (the big bubble) and pop when their film runs out.
//!
//! The TS pushes each mini's `BubbleState` into `scene.bubbles` and calls `scene.pop()`. Here the
//! minis own their states (`list[i].b`, in spawn order, which is the TS draw order) and a pop
//! comes back as a `PopState` in `pops` for the owner to drain. `scene.pop()` draws its seed from
//! `Math.random()` at the moment of the pop, so the pop's seed is drawn here, at that moment, from
//! the same `Random` (wubble change: the web's draw order, kept exactly).

use super::rand::Random;
use super::softbody::{BubbleState, PopState};
use crate::kit_gen::{minis as K, scene as S};
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq)]
pub struct Mini {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub r: f64,
    pub age: f64,
    pub life: f64,
    pub ph: f64,
    pub born: f64,
    pub b: BubbleState,
}

/// An obstacle the minis are pushed off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// How a spawn throws its minis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Throw {
    /// outward and fast (a touch)
    Burst,
    /// drifting up (ambient)
    Gentle,
}

// @source slint-minis
#[derive(Clone, Debug, Default)]
pub struct Minis {
    pub list: Vec<Mini>,
    /// pops since the owner last drained them, oldest first
    pub pops: Vec<PopState>,
}

impl Minis {
    pub fn new() -> Self {
        Self::default()
    }

    /// `burst` throws them outward; otherwise they drift up gently. `size` scales the radius.
    pub fn spawn(
        &mut self,
        x: f64,
        y: f64,
        n: usize,
        throw: Throw,
        size: f64,
        rnd: &mut impl Random,
    ) {
        let burst = throw == Throw::Burst;
        for _ in 0..n {
            let a = -PI / 2.0
                + (rnd.random() - 0.5)
                    * (if burst {
                        K::burst_spread
                    } else {
                        K::gentle_spread
                    });
            let sp = if burst {
                K::burst_speed + rnd.random() * K::burst_speed_range
            } else {
                K::gentle_speed + rnd.random() * K::gentle_speed_range
            };
            let r = (K::r_min + rnd.random() * K::r_range) * size;
            let b = BubbleState {
                x,
                y,
                r: 0.0,
                body: 0.0,
                alpha: 1.0,
                seed: rnd.random() * K::seed,
                swirl: 0.0,
                ..Default::default()
            };
            let life = K::life_min + rnd.random() * K::life_range;
            let ph = rnd.random() * K::phase_range;
            self.list.push(Mini {
                x,
                y,
                vx: a.cos() * sp,
                vy: a.sin() * sp,
                r,
                age: 0.0,
                life,
                ph,
                born: 0.0,
                b,
            });
        }
        while self.list.len() > K::max {
            self.list.remove(0);
        }
    }

    /// Pop whichever mini is under (x, y), the newest first. Returns true if one popped.
    pub fn hit(&mut self, x: f64, y: f64, rnd: &mut impl Random) -> bool {
        for i in (0..self.list.len()).rev() {
            let m = &self.list[i];
            if (x - m.x).powf(2.0) + (y - m.y).powf(2.0) < (m.r + K::hit_slack).powf(2.0) {
                let ang = (y - m.y).atan2(x - m.x);
                self.pop(i, ang, rnd);
                return true;
            }
        }
        false
    }

    /// `remove(i, true, angle)` → `scene.pop(x, y, r, angle)`.
    fn pop(&mut self, i: usize, angle: f64, rnd: &mut impl Random) {
        let m = self.list.remove(i);
        self.pops.push(PopState {
            x: m.x,
            y: m.y,
            r: m.r,
            age: 0.0,
            life: S::pop_life,
            angle,
            seed: rnd.random() * S::pop_seed,
        });
    }

    // @source slint-minis-step
    /// `avoid`: circles the minis are pushed off; `t`: the scene's time (s).
    pub fn step(&mut self, dt: f64, t: f64, avoid: &[Circle], rnd: &mut impl Random) {
        let top = -K::top;
        let mut i = self.list.len();
        while i > 0 {
            i -= 1;
            let m = &mut self.list[i];
            m.age += dt;
            m.born = 1f64.min(m.born + dt * K::born_rate);
            // buoyancy, drag, a sideways wander
            m.vy -= K::buoyancy * dt;
            m.vy *= (-dt * K::drag_y).exp();
            m.vx *= (-dt * K::drag_x).exp();
            m.x += (m.vx + (t * K::wander_freq + m.ph).sin() * K::wander) * dt;
            m.y += m.vy * dt;
            for o in avoid {
                let (bx, by) = (m.x - o.x, m.y - o.y);
                let bd = bx.hypot(by);
                let min_d = o.r + m.r + K::avoid_gap;
                if bd < min_d && bd > 0.0 {
                    let push = min_d - bd;
                    m.x += (bx / bd) * push;
                    m.y += (by / bd) * push;
                    m.vx += (bx / bd) * push * K::avoid_kick;
                }
            }
            if m.age > m.life || m.y < top {
                if m.y > top + K::pop_margin {
                    self.pop(i, -PI / 2.0, rnd);
                } else {
                    self.list.remove(i);
                }
                continue;
            }
            let born = 1.0 - (1.0 - m.born).powf(3.0);
            let b = &mut m.b;
            b.x = m.x;
            b.y = m.y;
            b.r = m.r * born * (1.0 + (m.age * K::breathe_freq + m.ph).sin() * K::breathe);
            b.swirl = m.age * K::swirl;
        }
    }

    /// The states to draw, in the TS `scene.bubbles` order.
    pub fn bubbles(&self) -> impl Iterator<Item = &BubbleState> {
        self.list.iter().map(|m| &m.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::start::rand::{Replay, Rng};

    #[test]
    fn max_is_enforced_oldest_first() {
        let mut m = Minis::new();
        let mut rng = Rng::new(1);
        m.spawn(0.0, 0.0, 30, Throw::Gentle, 1.0, &mut rng);
        let first_kept = m.list[14].clone();
        m.spawn(0.0, 0.0, 20, Throw::Gentle, 1.0, &mut rng);
        assert_eq!(m.list.len(), K::max);
        assert_eq!(m.list[0], first_kept);
        assert!(m.pops.is_empty(), "overflow removes without a pop");
    }

    #[test]
    fn a_spawn_draws_six_numbers_per_mini_in_the_web_order() {
        let mut m = Minis::new();
        // a, sp, r, seed, life, ph
        let mut r = Replay::new(vec![0.5, 0.0, 1.0, 0.2, 0.0, 0.5]);
        m.spawn(10.0, 20.0, 1, Throw::Burst, 1.0, &mut r);
        let mi = &m.list[0];
        assert_eq!(r.taken(), 6);
        assert!((mi.vx).abs() < 1e-9 && mi.vy == -K::burst_speed);
        assert_eq!(mi.r, K::r_min + K::r_range);
        assert_eq!(mi.b.seed, 0.2 * K::seed);
        assert_eq!(mi.life, K::life_min);
        assert_eq!(mi.ph, 0.5 * K::phase_range);
    }

    #[test]
    fn hit_pops_the_newest_first_with_a_seed_drawn_after() {
        let mut m = Minis::new();
        let mut rng = Rng::new(3);
        m.spawn(100.0, 100.0, 2, Throw::Gentle, 1.0, &mut rng);
        let newest = m.list[1].clone();
        let mut r = Replay::new(vec![0.25]);
        assert!(m.hit(100.0, 100.0, &mut r));
        assert_eq!(m.list.len(), 1);
        assert_ne!(m.list[0], newest);
        let p = m.pops[0];
        assert_eq!((p.x, p.y, p.r, p.age), (newest.x, newest.y, newest.r, 0.0));
        assert_eq!(p.life, S::pop_life);
        assert_eq!(p.seed, 0.25 * S::pop_seed);
        assert!(!m.hit(1000.0, 1000.0, &mut r));
    }
}
