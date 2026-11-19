//! What the scene's bubble and pop passes draw each frame, as plain data: the interface between
//! this crate's `Living` and the renderer (`wubble-gfx`'s scene, which owns the passes, the
//! textures and the shaders). Taken from the design's `core/gl/scene.ts` (`drawBubbles` at
//! :753-809, `waveUniforms` at :812-818, `wTexture` at :482, the BUBBLE and POP GLSL).
//!
//! # Bubble pass (`bubble.wgsl`, a translation of scene.ts's BUBBLE program)
//!
//! Per frame (globals): `uView` = page size (logical px), `uTime` = `StartFrame::time`, `uDpr`,
//! `uLight` = the light, which lags the pointer by `1 - exp(-18 dt)` per frame (scene-owned, fed by
//! `Effect::Light` / `Effect::LightLeave`), textures `uBg` (the half-res backdrop), `uW` (the W
//! field, `wfield::w_field_rgba`, `Rgba8Unorm`, linear, clamp, uploaded once), `uB1`, `uB2` (the
//! blurred backdrops, or `uW` when absent). Blend premultiplied: `ONE, ONE_MINUS_SRC_ALPHA`.
//!
//! Per bubble, in `StartFrame::bubbles` order (the body first, then the minis in spawn order),
//! the uniforms are `BubbleUniforms` (`BubbleUniforms::of`, which also skips the invisible ones):
//! `uQuad`, `uC`, `uR`, `uRing[RING_N]`, `uBody`, `uAlpha`, `uSwirl`, `uSeed`, `uHasW`, `uFace`,
//! `uWave[4]` + `uWaveN`, `uOverlay` (0 on the start page) and, with a W, `uWInv` (mat2,
//! column-major), `uWOff`, `uWScale`.
//!
//! # Pop pass (`pop.wgsl`, scene.ts's POP program)
//!
//! Globals `uView`, `uDpr`; per pop `PopUniforms`: `uQuad`, `uC`, `uR`, `uK` (= age / life),
//! `uAng`, `uSeed`. Same blend. The pop's push in the liquid is `Effect::Pop`.

use super::living::Living;
use super::rand::Random;
use super::softbody::{BubbleState, PopState, RING_N, Wave};
use crate::kit_gen::scene as S;

/// One frame of the start page's bubbles and pops, borrowed from the `Living`.
#[derive(Clone, Debug)]
pub struct StartFrame<'a> {
    /// the body first, then the minis in spawn order (the draw order)
    pub bubbles: Vec<&'a BubbleState>,
    pub pops: &'a [PopState],
    /// `scene.time`, seconds (the bubble shader's `uTime`)
    pub time: f64,
    /// the page's size, logical px (`uView`)
    pub view: (f64, f64),
}

impl<R: Random> Living<R> {
    pub fn frame(&self) -> StartFrame<'_> {
        StartFrame {
            bubbles: std::iter::once(&self.body.state)
                .chain(self.minis.bubbles())
                .collect(),
            pops: &self.pops,
            time: self.time,
            view: self.size,
        }
    }
}

/// One bubble's uniforms, as scene.ts sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleUniforms {
    /// x, y, w, h of the quad drawn (logical px): the circle's box grown by `ext = r * 1.7 + 4`
    pub quad: [f32; 4],
    pub c: [f32; 2],
    pub r: f32,
    /// zeros for a perfect circle
    pub ring: [f32; RING_N],
    pub body: f32,
    pub alpha: f32,
    pub swirl: f32,
    pub seed: f32,
    pub has_w: bool,
    pub face: [f32; 2],
    /// the newest `wave_n` waves, oldest first: x, y, age / life, reach
    pub wave: [[f32; 4]; S::wave_max],
    pub wave_n: u32,
    /// column-major
    pub w_inv: [f32; 4],
    pub w_off: [f32; 2],
    pub w_scale: f32,
}

/// `waveUniforms`: the newest four, oldest first.
pub fn wave_uniforms(waves: &[Wave]) -> ([[f32; 4]; S::wave_max], u32) {
    let n = waves.len().min(S::wave_max);
    let mut out = [[0.0; 4]; S::wave_max];
    for (slot, w) in out.iter_mut().zip(&waves[waves.len() - n..]) {
        *slot = [
            w.x as f32,
            w.y as f32,
            (w.age / w.life) as f32,
            w.reach as f32,
        ];
    }
    (out, n as u32)
}

impl BubbleUniforms {
    /// `None` for a bubble the web skips (`alpha <= .001` or `r < .5`).
    pub fn of(b: &BubbleState) -> Option<Self> {
        if b.alpha <= S::bubble_min_alpha || b.r < S::bubble_min_r {
            return None;
        }
        let ext = b.r * S::bubble_ext_k + S::bubble_ext_pad;
        let (wave, wave_n) = wave_uniforms(&b.waves);
        let w = b.w;
        Some(BubbleUniforms {
            quad: [
                (b.x - ext) as f32,
                (b.y - ext) as f32,
                (ext * 2.0) as f32,
                (ext * 2.0) as f32,
            ],
            c: [b.x as f32, b.y as f32],
            r: b.r as f32,
            ring: b.ring.unwrap_or([0.0; RING_N]),
            body: b.body as f32,
            alpha: b.alpha as f32,
            swirl: b.swirl as f32,
            seed: b.seed as f32,
            has_w: w.is_some(),
            face: [b.face[0] as f32, b.face[1] as f32],
            wave,
            wave_n,
            w_inv: w.map_or([1.0, 0.0, 0.0, 1.0], |w| w.inv.map(|v| v as f32)),
            w_off: w.map_or([0.0; 2], |w| w.off.map(|v| v as f32)),
            w_scale: w.map_or(0.0, |w| w.scale as f32),
        })
    }
}

/// One pop's uniforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopUniforms {
    /// the pop's box grown by `ext = r * 3 + 10`
    pub quad: [f32; 4],
    pub c: [f32; 2],
    pub r: f32,
    /// age / life, 0..1
    pub k: f32,
    pub ang: f32,
    pub seed: f32,
}

impl PopUniforms {
    pub fn of(p: &PopState) -> Self {
        let ext = p.r * S::pop_ext_k + S::pop_ext_pad;
        PopUniforms {
            quad: [
                (p.x - ext) as f32,
                (p.y - ext) as f32,
                (ext * 2.0) as f32,
                (ext * 2.0) as f32,
            ],
            c: [p.x as f32, p.y as f32],
            r: p.r as f32,
            k: (p.age / p.life) as f32,
            ang: p.angle as f32,
            seed: p.seed as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::start::living::{Event, Options, transition};
    use crate::start::rand::Rng;

    #[test]
    fn the_frame_draws_the_body_first_then_the_minis() {
        let l = Living::new(Options::start_page(), Rng::new(9));
        let (l, _) = transition(l, Event::Resize { w: 800.0, h: 600.0 });
        let (l, _) = transition(
            l,
            Event::Down {
                x: 5.0,
                y: 5.0,
                t_ms: 0.0,
            },
        );
        let (l, _) = transition(l, Event::Tick { dt: 1.0 / 120.0 });
        let f = l.frame();
        assert_eq!(f.bubbles.len(), 1 + l.minis.list.len());
        assert_eq!(f.bubbles[0], &l.body.state);
        assert_eq!(f.bubbles[1], &l.minis.list[0].b);
        assert_eq!(f.view, (800.0, 600.0));
        let u = BubbleUniforms::of(f.bubbles[0]).unwrap();
        assert!(u.has_w && u.body == 1.0);
        let ext = l.body.R * S::bubble_ext_k + S::bubble_ext_pad;
        assert_eq!(u.quad[2], (ext * 2.0) as f32);
    }

    #[test]
    fn invisible_bubbles_are_skipped_and_waves_are_the_newest_four() {
        let b = BubbleState {
            r: 0.2,
            ..Default::default()
        };
        assert_eq!(BubbleUniforms::of(&b), None);
        let waves: Vec<Wave> = (0..6)
            .map(|i| Wave {
                x: i as f64,
                y: 0.0,
                age: 0.5,
                life: 1.0,
                reach: 10.0,
            })
            .collect();
        let (w, n) = wave_uniforms(&waves);
        assert_eq!(n, 4);
        assert_eq!(w[0], [2.0, 0.0, 0.5, 10.0]);
        assert_eq!(w[3][0], 5.0);
        let p = PopUniforms::of(&PopState {
            x: 10.0,
            y: 10.0,
            r: 4.0,
            age: 0.18,
            life: 0.36,
            angle: 1.0,
            seed: 3.0,
        });
        assert_eq!(p.k, 0.5);
        assert_eq!(
            p.quad[2],
            ((4.0 * S::pop_ext_k + S::pop_ext_pad) * 2.0) as f32
        );
    }
}
