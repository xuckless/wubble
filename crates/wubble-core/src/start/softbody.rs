// Source: the owner's design repo (xuckless/the-wallpaper-project @ 598f814),
// `src/wubble/slint/softbody.rs` (a 1:1 port of `core/softbody.ts`), with the marked
// `wubble change`s. Every tuning number comes from `kit_gen::softbody` (the generator reads it
// from `core/softbody.ts`).

//! The Wubble logo as a soft body: a ring of RING_N radial springs coupled to their neighbours
//! (surface tension) and to the mean (volume), a centre that leans toward the pointer and can be
//! dragged and flung, and a W that squashes and wobbles inside it. Call `step(dt)` every frame
//! and hand `state` to the renderer.
//!
//! Numerics: the TS keeps `ring`, `vel`, `acc` and `th` in `Float32Array`s and does the maths in
//! f64. This port does the same (f32 storage, f64 arithmetic, rounding on store), so a run with
//! the same inputs matches the web frame for frame. `x ** n` is ported as `powf` (libm pow, like
//! JS), not `powi`, to keep the last bits the same.

use super::grab::{self, Grab, GrabEffect, GrabEvent};
use crate::kit_gen::{scene as S, softbody as K};
use crate::motion::{Spring, Velocity, band, clamp, js_sign, spring, step_raw};
use std::f64::consts::PI;

// @source slint-bubble-state
/// Ring samples per bubble: `RING_N` in scene.ts (and the WGSL `RING_N`).
pub const RING_N: usize = S::RING_N;

/// The logo W: inverse 2×2 transform (column-major), offset in viewBox units, px per viewBox unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WState {
    pub inv: [f64; 4],
    pub off: [f64; 2],
    pub scale: f64,
}

/// Mirrors `BubbleState` in scene.ts. Coordinates are logical px from the top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct BubbleState {
    pub x: f64,
    pub y: f64,
    pub r: f64,
    /// RING_N radial offsets in px; `None` for a perfect circle
    pub ring: Option<[f32; RING_N]>,
    /// 0 clear soap bubble, 1 the logo's dark-glass body
    pub body: f64,
    pub alpha: f64,
    pub w: Option<WState>,
    /// extra film swirl, added by touches
    pub swirl: f64,
    pub seed: f64,
    /// yaw, pitch in radians: where the bubble's face turns
    pub face: [f64; 2],
    /// distortion waves running through the body (pokes, flings); see `Wave`
    pub waves: Vec<Wave>,
}

impl Default for BubbleState {
    fn default() -> Self {
        // the TS defaults when a field is omitted: body 0, alpha 1, swirl 0, seed 0
        BubbleState {
            x: 0.0,
            y: 0.0,
            r: 0.0,
            ring: None,
            body: 0.0,
            alpha: 1.0,
            w: None,
            swirl: 0.0,
            seed: 0.0,
            face: [0.0; 2],
            waves: Vec::new(),
        }
    }
}

// @source slint-wave
/// A distortion wave travelling through a surface: logical px, seconds. Mirrors `Wave` in scene.ts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wave {
    pub x: f64,
    pub y: f64,
    pub age: f64,
    pub life: f64,
    pub reach: f64,
}

/// `Scene.wave(target, x, y, reach, life = 0.9)`: at most the 4 newest waves are kept.
pub fn push_wave(waves: &mut Vec<Wave>, x: f64, y: f64, reach: f64, life: f64) {
    waves.push(Wave {
        x,
        y,
        age: 0.0,
        life,
        reach,
    });
    if waves.len() > S::wave_max {
        waves.remove(0);
    }
}

/// `Scene.ageWaves` for one surface.
pub fn age_waves(waves: &mut Vec<Wave>, dt: f64) {
    waves.retain_mut(|w| {
        w.age += dt;
        w.age < w.life
    });
}

/// Mirrors `PopState` in scene.ts: a mini's film tearing open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopState {
    pub x: f64,
    pub y: f64,
    pub r: f64,
    pub age: f64,
    pub life: f64,
    pub angle: f64,
    pub seed: f64,
}

/// Voice bands as the soft body reads them (`core/voice.ts`). Not wired in wubble yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoiceIn {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
    pub phase: f64,
}

/// What `up()` reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Poke,
    Fling,
}

// @source slint-softbody
#[allow(non_snake_case)]
#[derive(Clone, Debug)]
pub struct SoftBody {
    pub ring: [f32; RING_N],
    vel: [f32; RING_N],
    acc: [f32; RING_N],
    /// matches the shader's sampling: θᵢ = −π + 2πi/N
    pub th: [f32; RING_N],
    pub home: (f64, f64),
    pub R: f64,
    pub cx: Spring,
    pub cy: Spring,
    wsx: Spring,
    wsy: Spring,
    wr: Spring,
    wx: Spring,
    wy: Spring,
    pub swirl: Spring,
    /// where the face looks: yaw and pitch, radians
    pub yaw: Spring,
    pub pitch: Spring,
    pub t: f64,
    pub wobble: f64,
    pub ptr: Option<(f64, f64)>,
    /// voice bands while listening; `None` when not
    pub voice: Option<VoiceIn>,
    /// wubble change: the pointer's hold (`grab::Grab`) replaces `held`, `dragging`, `start`
    /// and `grab`.
    pub grab: Grab,
    v: Velocity,
    pub state: BubbleState,
}

#[allow(non_snake_case)]
impl SoftBody {
    /// `seed` is the TS `Math.random() * 10`, drawn by the caller so the draw order is its own.
    pub fn new(radius: Option<f64>, wobble: Option<f64>, seed: f64) -> Self {
        let R = radius.unwrap_or(K::radius);
        let wobble = wobble.unwrap_or(K::wobble);
        let mut th = [0f32; RING_N];
        for (i, t) in th.iter_mut().enumerate() {
            *t = (-PI + (PI * 2.0 * i as f64) / RING_N as f64) as f32;
        }
        let ring = [0f32; RING_N];
        SoftBody {
            ring,
            vel: [0.0; RING_N],
            acc: [0.0; RING_N],
            th,
            home: (0.0, 0.0),
            R,
            cx: spring(0.0),
            cy: spring(0.0),
            wsx: spring(1.0),
            wsy: spring(1.0),
            wr: spring(0.0),
            wx: spring(0.0),
            wy: spring(0.0),
            swirl: spring(0.0),
            yaw: spring(0.0),
            pitch: spring(0.0),
            t: 0.0,
            wobble,
            ptr: None,
            voice: None,
            grab: Grab::Free,
            v: Velocity::new(),
            state: BubbleState {
                x: 0.0,
                y: 0.0,
                r: R,
                ring: Some(ring),
                body: 1.0,
                alpha: 1.0,
                w: Some(WState {
                    inv: [1.0, 0.0, 0.0, 1.0],
                    off: [K::w_off_x, K::w_off_y],
                    scale: R / K::w_units,
                }),
                swirl: 0.0,
                seed,
                face: [0.0; 2],
                waves: Vec::new(),
            },
        }
    }

    pub fn center(&self) -> (f64, f64) {
        (self.home.0 + self.cx.x, self.home.1 + self.cy.x)
    }

    /// Is (x, y) on the bubble? (with a little slack, the web's default `hit_slack`)
    pub fn hit(&self, x: f64, y: f64) -> bool {
        let c = self.center();
        let (dx, dy) = (x - c.0, y - c.1);
        dx * dx + dy * dy < (self.R + K::hit_slack).powf(2.0)
    }

    // @source slint-softbody-poke
    /// A tap. Near the middle it squashes the W; on the rim it dents where you touched.
    pub fn poke(&mut self, x: f64, y: f64, strength: f64) {
        let c = self.center();
        let (dx, dy) = (x - c.0, y - c.1);
        let h = dx.hypot(dy);
        let dist = if h == 0.0 { 1.0 } else { h }; // `Math.hypot(dx, dy) || 1`
        if dist < self.R * K::poke_mid {
            self.wsy.v -= K::poke_wsy * strength;
            self.wsx.v += K::poke_wsx * strength;
            // Math.sign(dx || 1)
            self.wr.v += js_sign(if dx == 0.0 { 1.0 } else { dx }) * K::poke_wr * strength;
            for i in 0..RING_N {
                let th = self.th[i] as f64;
                self.vel[i] = (self.vel[i] as f64
                    + -K::poke_mid_ring * strength * (2.0 * (th - PI / 2.0)).cos())
                    as f32;
            }
        } else {
            let phi = dy.atan2(dx);
            for i in 0..RING_N {
                let cs = (self.th[i] as f64 - phi).cos();
                if cs > 0.0 {
                    self.vel[i] =
                        (self.vel[i] as f64 - K::poke_rim_ring * strength * cs.powf(4.0)) as f32;
                }
            }
            self.cx.v -= (dx / dist) * K::poke_rim_centre * strength;
            self.cy.v -= (dy / dist) * K::poke_rim_centre * strength;
            self.wx.v -= (dx / dist) * K::poke_rim_w * strength;
            self.wy.v -= (dy / dist) * K::poke_rim_w * strength;
        }
        self.swirl.v += K::poke_swirl * strength;
    }

    /// `t_ms`: event time in ms (the TS Velocity uses `performance.now()`).
    pub fn down(&mut self, x: f64, y: f64, t_ms: f64) {
        let centre = self.center();
        self.grab = grab::transition(self.grab, GrabEvent::Down { x, y, centre }).0;
        self.v.reset();
        self.v.push(x, y, t_ms);
        self.ptr = Some((x, y));
    }

    pub fn r#move(&mut self, x: f64, y: f64, t_ms: f64) {
        self.ptr = Some((x, y));
        if self.grab.held() {
            self.v.push(x, y, t_ms);
        }
        self.grab = grab::transition(self.grab, GrabEvent::Move { x, y }).0;
    }

    // @source slint-softbody-fling
    /// Returns `Poke` if it was a tap, `Fling` if dragged, `None` if not held.
    pub fn up(&mut self) -> Option<Release> {
        let (g, effect) = grab::transition(self.grab, GrabEvent::Up);
        self.grab = g;
        match effect? {
            GrabEffect::Fling => {
                let f = self.v.get();
                // fling: the centre keeps the release speed, rubber-banded back home
                self.cx.v += clamp(f.0, -K::fling_cap, K::fling_cap) * K::fling_keep;
                self.cy.v += clamp(f.1, -K::fling_cap, K::fling_cap) * K::fling_keep;
                self.swirl.v += K::fling_swirl_max.min(f.0.hypot(f.1) / K::fling_swirl_per);
                // the far side catches up: a wobble opposite the throw
                let a = f.1.atan2(f.0);
                let amt = 1f64.min(f.0.hypot(f.1) / K::fling_full);
                for i in 0..RING_N {
                    self.vel[i] = (self.vel[i] as f64
                        - K::fling_ring * amt * (self.th[i] as f64 - a).cos())
                        as f32;
                }
                Some(Release::Fling)
            }
            GrabEffect::Poke => {
                if let Some((px, py)) = self.ptr {
                    self.poke(px, py, 1.0);
                }
                Some(Release::Poke)
            }
            GrabEffect::ForgetPointer => None,
        }
    }

    pub fn leave(&mut self) {
        let (g, effect) = grab::transition(self.grab, GrabEvent::Leave);
        self.grab = g;
        if effect == Some(GrabEffect::ForgetPointer) {
            self.ptr = None;
        }
    }

    // @source slint-softbody-step
    /// Advance by `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.t += dt;
        let (t, wob, ptr, R) = (self.t, self.wobble, self.ptr, self.R);
        let drag = self.grab.dragging().copied();
        // centre: rubber-band after the pointer while dragged, else lean toward it and breathe
        match (drag, ptr) {
            (Some(h), Some(p)) => {
                step_raw(
                    &mut self.cx,
                    band(p.0 - h.start.0, K::drag_band),
                    K::drag_k,
                    K::drag_c,
                    dt,
                );
                step_raw(
                    &mut self.cy,
                    band(p.1 - h.start.1, K::drag_band),
                    K::drag_k,
                    K::drag_c,
                    dt,
                );
            }
            _ => {
                let (mut tx, mut ty) = (0.0, 0.0);
                if let Some(p) = ptr {
                    tx = clamp((p.0 - self.home.0) * K::lean, -K::lean_max, K::lean_max);
                    ty = clamp((p.1 - self.home.1) * K::lean, -K::lean_max, K::lean_max);
                }
                ty += (t * K::breathe_freq).sin() * K::breathe_amp * wob;
                step_raw(&mut self.cx, tx, K::centre_k, K::centre_c, dt);
                step_raw(&mut self.cy, ty, K::centre_k, K::centre_c, dt);
            }
        }
        // ring: target shape (ambient wobble + drag stretch), spring to it, couple to neighbours, keep volume
        let (mut drag_a, mut drag_amt) = (0.0, 0.0);
        if let (Some(h), Some(p)) = (drag, ptr) {
            let c = self.center();
            let (ddx, ddy) = (p.0 - c.0, p.1 - c.1);
            drag_a = ddy.atan2(ddx);
            drag_amt = (R * K::drag_amt_max).min(
                (ddx - h.grab.0 * K::drag_grab).hypot(ddy - h.grab.1 * K::drag_grab) * K::drag_amt
                    + self.cx.v.hypot(self.cy.v) * K::drag_speed,
            );
        }
        // speed stretches the body along its travel; volume is kept by the mean term
        let sv = self.cx.v.hypot(self.cy.v);
        let sa = self.cy.v.atan2(self.cx.v);
        let s_amt = (R * K::speed_max).min(sv * K::speed_stretch);
        let mut mean = 0.0;
        for r in self.ring {
            mean += r as f64;
        }
        mean /= RING_N as f64;
        for i in 0..RING_N {
            let th = self.th[i] as f64;
            let mut target = R
                * K::wobble_amp
                * wob
                * ((2.0 * th + t * K::wobble2_speed).sin() * K::wobble2_mix
                    + (3.0 * th - t * K::wobble3_speed).sin() * K::wobble3_mix);
            if drag_amt != 0.0 {
                let cs = (th - drag_a).cos();
                target += drag_amt * (if cs > 0.0 { cs.powf(3.0) } else { 0.0 })
                    - drag_amt * K::drag_side * (th - drag_a).sin().powf(2.0);
            }
            if s_amt > K::speed_min {
                target += s_amt * (2.0 * (th - sa)).cos();
            }
            // @source slint-softbody-listen
            // listening: the bands become lumps travelling round the rim (low = 2 broad lobes, mid = 3, high = 5 small ones)
            if let Some(vb) = self.voice {
                target += R
                    * (K::voice_low * vb.low * (2.0 * th + vb.phase).sin()
                        + K::voice_mid
                            * vb.mid
                            * (3.0 * th - vb.phase * K::voice_mid_speed + K::voice_mid_phase)
                                .sin()
                        + K::voice_high
                            * vb.high
                            * (5.0 * th + vb.phase * K::voice_high_speed + K::voice_high_phase)
                                .sin());
            }
            let prev = self.ring[(i + RING_N - 1) % RING_N] as f64;
            let next = self.ring[(i + 1) % RING_N] as f64;
            let ri = self.ring[i] as f64;
            self.acc[i] = (-K::ring_k * (ri - target) - K::ring_c * self.vel[i] as f64
                + K::ring_tension * (prev + next - 2.0 * ri)
                - K::ring_volume * mean) as f32;
        }
        for i in 0..RING_N {
            self.vel[i] = (self.vel[i] as f64 + self.acc[i] as f64 * dt) as f32;
            self.ring[i] = clamp(
                self.ring[i] as f64 + self.vel[i] as f64 * dt,
                -R * K::ring_min,
                R * K::ring_max,
            ) as f32;
        }
        // W: squash springs, lean toward the pointer, a little bob
        step_raw(&mut self.wsx, 1.0, K::w_squash_k, K::w_squash_c, dt);
        step_raw(&mut self.wsy, 1.0, K::w_squash_k, K::w_squash_c, dt);
        step_raw(&mut self.wr, 0.0, K::w_turn_k, K::w_turn_c, dt);
        let (mut wtx, mut wty) = (0.0, 0.0);
        if let Some(p) = ptr {
            let c = self.center();
            wtx = clamp((p.0 - c.0) * K::w_lean, -K::w_lean_max, K::w_lean_max);
            wty = clamp((p.1 - c.1) * K::w_lean, -K::w_lean_max, K::w_lean_max);
        }
        if drag.is_some() {
            wtx = -self.cx.x * K::w_drag;
            wty = -self.cy.x * K::w_drag;
        }
        step_raw(&mut self.wx, wtx, K::w_k, K::w_c, dt);
        step_raw(
            &mut self.wy,
            wty + (t * K::w_bob_freq).sin() * K::w_bob_amp * wob,
            K::w_k,
            K::w_c,
            dt,
        );
        // @source slint-softbody-face
        // the face turns toward the cursor: up to ±0.45 rad, saturating smoothly with distance, on a quick
        // spring with a touch of lag (k 170, c 17 ≈ ζ 0.65: one small overshoot as it arrives)
        let (mut fy, mut fp);
        if let Some(p) = ptr {
            let c = self.center();
            let dx = (p.0 - c.0) / (R * K::face_reach);
            let dy = (p.1 - c.1) / (R * K::face_reach);
            fy = dx.tanh() * K::face_max;
            fp = dy.tanh() * K::face_max;
        } else {
            fy = (t * K::face_idle_yaw_freq).sin() * K::face_idle_yaw * wob;
            fp = (t * K::face_idle_pitch_freq + K::face_idle_pitch_phase).sin()
                * K::face_idle_pitch
                * wob;
        }
        if drag.is_some() {
            // it looks back as it's dragged
            fy = -(self.cx.v / K::face_drag_speed).tanh() * K::face_drag;
            fp = -(self.cy.v / K::face_drag_speed).tanh() * K::face_drag;
        }
        step_raw(&mut self.yaw, fy, K::face_k, K::face_c, dt);
        step_raw(&mut self.pitch, fp, K::face_k, K::face_c, dt);
        // swirl decays, the film keeps a slow drift
        self.swirl.v *= (-dt * K::swirl_decay).exp();
        self.swirl.x += self.swirl.v * dt;

        // out to the renderer
        let c = self.center();
        let s = &mut self.state;
        s.x = c.0;
        s.y = c.1;
        s.r = R;
        s.swirl = self.swirl.x;
        s.face = [self.yaw.x, self.pitch.x];
        s.ring = Some(self.ring);
        let rot = (self.wr.x * K::w_turn * PI) / 180.0;
        let (cs, sn) = (rot.cos(), rot.sin());
        let (sx, sy) = (self.wsx.x, self.wsy.x);
        // forward M = R(rot)·S(sx, sy); inverse = S⁻¹·R(−rot); column-major for the shader
        let (i00, i01, i10, i11) = (cs / sx, sn / sx, -sn / sy, cs / sy);
        s.w = Some(WState {
            inv: [i00, i10, i01, i11],
            off: [K::w_off_x + self.wx.x, K::w_off_y + self.wy.x],
            scale: R / K::w_units,
        });
    }
}
