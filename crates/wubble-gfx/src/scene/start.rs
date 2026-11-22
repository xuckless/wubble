//! wubble: the start page's living layer (`wubble://start`, the new-tab page), as plain data.
//!
//! The browser window's backdrop is the still dye (`Backdrop::Still`, owner override
//! 2026-11-19); the start page is where the design's living liquid and its bubble live. So the
//! renderer keeps a second CPU [`Scene`] (`Backdrop::Liquid`, the design's: emitters, the
//! pointer's push, ripples, the light's lag, the palette crossfade) with its own fluid grids and
//! blur pyramid sized to the page card, and only while a start tab is shown
//! (`Renderer::start_frame` / `Renderer::start_end`). The liquid is simulated for the page
//! region alone (its own grids at the card's aspect), never for the window: the sidebar's and
//! the chrome's glass keep sampling the still backdrop, and nothing of this runs without a
//! shown start tab.
//!
//! Each frame with a layer (`render.rs` `start_passes`): the layer's liquid steps by its own
//! `BackdropClock`, is composited and blurred (b1, b2 for the bubble), then the card's image is
//! drawn: the layer's backdrop (`blit`), the bubbles (`bubble.wgsl`, body first then the minis)
//! and the pops (`pop.wgsl`), with `uView` the card's size in logical px, so the page-local
//! coordinates of `wubble_core::start::Living` are used as they are. The output pass then lays
//! the card over the still backdrop and the window's glass, rounded like the page card (page.wgsl
//! `vs_card` + `fs_page`): the mask. Everything outside the card is untouched.
//!
//! The types here are the scene's own (this crate does not depend on `wubble-core`): the host
//! converts `wubble_core::start::frame::{BubbleUniforms, PopUniforms}` and `living::Effect` field
//! by field.
//!
//! Source of the uniform mirrors: the-wallpaper-project `src/wubble/slint/render.rs` (`BubbleU`,
//! `PopU` and their fill in `frame`), owner's code; the pop's push is `scene.pop` in
//! `core/gl/scene.ts`, its numbers handed in by the host (`kit_gen::scene::pop_push*`).

use super::state::{Rect, Scene};

/// Where the start page's card is: logical window px and its corner radius.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StartCard {
    pub rect: Rect,
    pub radius: f32,
}

/// One bubble as the bubble pass draws it (`BubbleUniforms` in wubble-core): page-local logical
/// px. `ring` is the soft body's rim offsets (zeros for a circle), `wave` the newest `wave_n`
/// waves oldest first (x, y, age / life, reach), `w_inv` column-major.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BubbleDraw {
    pub quad: [f32; 4],
    pub c: [f32; 2],
    pub r: f32,
    pub ring: [f32; RING_N],
    pub body: f32,
    pub alpha: f32,
    pub swirl: f32,
    pub seed: f32,
    pub has_w: bool,
    pub face: [f32; 2],
    pub wave: [[f32; 4]; 4],
    pub wave_n: u32,
    pub w_inv: [f32; 4],
    pub w_off: [f32; 2],
    pub w_scale: f32,
}

/// One pop as the pop pass draws it (`PopUniforms` in wubble-core): `k` is age / life.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PopDraw {
    pub quad: [f32; 4],
    pub c: [f32; 2],
    pub r: f32,
    pub k: f32,
    pub ang: f32,
    pub seed: f32,
}

/// One frame of the start page: where its card is, what it draws, and the scene's pace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartDraw {
    pub card: StartCard,
    /// in draw order: the body first, then the minis in spawn order
    pub bubbles: Vec<BubbleDraw>,
    pub pops: Vec<PopDraw>,
    /// `scene.time` of the living bubble, seconds (`uTime`)
    pub time: f32,
    /// the layer's liquid runs at `pace` of real time: living.ts `scene.frame(still ? dt * .3 : dt)`
    /// (wubble: times the settle's scale, `wubble_core::start::calm`)
    pub pace: f32,
    /// The page has settled (wubble's, the owner's): nothing of it moves, so it asks for no
    /// frames ([`super::Renderer::animating`]) once its liquid's own ripples and light are done.
    pub rest: bool,
}

/// What the start page does to its liquid (`wubble_core::start::living::Effect`), page-local
/// logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StartFx {
    /// `scene.ripple(x, y, reach, strength)`: a ring through the backdrop and a push in the liquid.
    Ripple {
        x: f32,
        y: f32,
        reach: f32,
        strength: f32,
    },
    /// `scene.move(x, y)`: the pointer pushes the liquid and the light follows it.
    Light { x: f32, y: f32 },
    /// `scene.leave()`.
    LightLeave,
    /// `scene.pop`'s push in the liquid for a pop of radius `r` at (x, y): `fluid.ripple(x / w,
    /// 1 - y / h, strength, max(min, r / w * k))`.
    Pop {
        x: f32,
        y: f32,
        r: f32,
        strength: f32,
        k: f32,
        min: f32,
    },
}

/// The ring's sample count (scene.ts `RING_N`, bubble.wgsl `RING_N`).
pub const RING_N: usize = 24;

/// Applies one effect to the layer's scene.
pub(super) fn apply(scene: &mut Scene, fx: StartFx) {
    match fx {
        StartFx::Ripple {
            x,
            y,
            reach,
            strength,
        } => scene.ripple(x, y, reach, strength),
        StartFx::Light { x, y } => scene.pointer_move(x, y),
        StartFx::LightLeave => scene.pointer_leave(),
        StartFx::Pop {
            x,
            y,
            r,
            strength,
            k,
            min,
        } => scene.pop_push(x, y, r, strength, k, min),
    }
}

// @source slint-render-uniforms (render.rs BubbleU, PopU)
// #[repr(C)] mirrors of bubble.wgsl's and pop.wgsl's uniforms (sizes checked against naga's
// layout in render.rs's tests).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct BubbleU {
    u_quad: [f32; 4],
    u_view: [f32; 2],
    u_c: [f32; 2],
    u_light: [f32; 2],
    u_w_off: [f32; 2],
    u_w_inv: [f32; 4],
    u_r: f32,
    u_body: f32,
    u_alpha: f32,
    u_time: f32,
    u_swirl: f32,
    u_seed: f32,
    u_dpr: f32,
    u_has_w: f32,
    u_overlay: f32,
    u_w_scale: f32,
    u_face: [f32; 2],
    u_wave_n: i32,
    _p: [f32; 3],
    u_ring: [f32; RING_N],
    u_wave: [[f32; 4]; 4],
} // 288 bytes

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct PopU {
    u_quad: [f32; 4],
    u_view: [f32; 2],
    u_c: [f32; 2],
    u_r: f32,
    u_k: f32,
    u_ang: f32,
    u_seed: f32,
    u_dpr: f32,
    _p: [f32; 3],
} // 64 bytes

const _: () = {
    assert!(std::mem::size_of::<BubbleU>() == 288);
    assert!(std::mem::size_of::<PopU>() == 64);
};

/// The frame-wide values both passes read: the card's size in logical px (`uView`), the scene's
/// pixel ratio, the light and the living bubble's time.
#[derive(Clone, Copy, Debug)]
pub(super) struct Globals {
    pub view: [f32; 2],
    pub dpr: f32,
    pub light: [f32; 2],
    pub time: f32,
}

/// One bubble's uniforms (render.rs `frame`'s bubble loop). `uOverlay` is 0: the start page
/// draws the bubble over its own backdrop copy.
pub(super) fn bubble_u(b: &BubbleDraw, g: Globals) -> BubbleU {
    BubbleU {
        u_quad: b.quad,
        u_view: g.view,
        u_c: b.c,
        u_light: g.light,
        u_w_off: b.w_off,
        u_w_inv: b.w_inv,
        u_r: b.r,
        u_body: b.body,
        u_alpha: b.alpha,
        u_time: g.time,
        u_swirl: b.swirl,
        u_seed: b.seed,
        u_dpr: g.dpr,
        u_has_w: if b.has_w { 1.0 } else { 0.0 },
        u_overlay: 0.0,
        u_w_scale: b.w_scale,
        u_face: b.face,
        u_wave_n: b.wave_n.min(4) as i32,
        _p: [0.0; 3],
        u_ring: b.ring,
        u_wave: b.wave,
    }
}

/// One pop's uniforms.
pub(super) fn pop_u(p: &PopDraw, g: Globals) -> PopU {
    PopU {
        u_quad: p.quad,
        u_view: g.view,
        u_c: p.c,
        u_r: p.r,
        u_k: p.k,
        u_ang: p.ang,
        u_seed: p.seed,
        u_dpr: g.dpr,
        _p: [0.0; 3],
    }
}

/// What one scene frame changed in the window's image (`out`) and the overlay layers' images,
/// for the shell's partial rendering: Slint cannot see a texture written in place, so the host
/// marks this region dirty ([`super::Frame::damage`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Damage {
    /// Everything may have changed: Slint redraws the whole window.
    Whole,
    /// Only the start page's card, logical window px: its liquid and bubble moved, nothing under
    /// or around it did.
    Card(Rect),
    /// Nothing changed.
    Nothing,
}

impl Damage {
    /// Two frames' damage drawn before Slint shows either (headless renders, tests).
    pub fn and(self, later: Damage) -> Damage {
        match (self, later) {
            (Damage::Nothing, d) | (d, Damage::Nothing) => d,
            (Damage::Card(a), Damage::Card(b)) if a == b => Damage::Card(a),
            _ => Damage::Whole,
        }
    }
}

/// A frame's image with no damage told: all of it.
impl Default for Damage {
    fn default() -> Self {
        Damage::Whole
    }
}

/// What [`damage`] decides from: this frame against the last one drawn.
#[derive(Clone, Copy, Debug)]
pub struct DamageIn<'a> {
    /// The output or an overlay layer's texture is new or gone (first frame, resize, a layer's
    /// glass appearing or leaving).
    pub new_targets: bool,
    /// The window's backdrop (its `bg` and blur levels) was rendered again.
    pub backdrop: bool,
    /// The page-inclusive pyramid was built again (the ambience, overlay glass over a pane).
    pub page_built: bool,
    /// Everything the glass draws reads (each surface's uniforms and inputs: render.rs `glass_key`), this frame and the
    /// last (`None`: no frame drawn yet).
    pub glass: &'a [u8],
    pub last_glass: Option<&'a [u8]>,
    /// The start page's card this frame and the last (`None`: no start page drawn).
    pub card: Option<StartCard>,
    pub last_card: Option<StartCard>,
}

/// The region a scene frame changed. Outside the start page's card the output is the window's
/// backdrop and its glass: drawn again from the same textures and the same uniforms it is the
/// same image, so only the card, drawn anew every frame, changed. Anything else that moved (the
/// backdrop, the page blur, a glass rect, its light, wave, film, beam or alpha, a texture, the
/// card's own place) changes all of it.
pub fn damage(d: &DamageIn) -> Damage {
    let same = !d.new_targets
        && !d.backdrop
        && !d.page_built
        && d.last_glass == Some(d.glass)
        && d.card == d.last_card;
    match d.card {
        _ if !same => Damage::Whole,
        Some(c) => Damage::Card(c.rect),
        None => Damage::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SceneOptions;

    #[test]
    fn a_start_page_frame_changes_only_its_card_while_nothing_else_moves() {
        let card = StartCard {
            rect: Rect::new(268.0, 8.0, 1084.0, 864.0),
            radius: 12.0,
        };
        let glass = [1u8, 2, 3];
        let still = DamageIn {
            new_targets: false,
            backdrop: false,
            page_built: false,
            glass: &glass,
            last_glass: Some(&glass),
            card: Some(card),
            last_card: Some(card),
        };
        assert_eq!(damage(&still), Damage::Card(card.rect));
        // no start page and nothing moved: nothing to redraw
        let none = DamageIn {
            card: None,
            last_card: None,
            ..still
        };
        assert_eq!(damage(&none), Damage::Nothing);
        // anything else that moved: the whole window
        let moved = [9u8, 2, 3];
        let cases = [
            DamageIn {
                new_targets: true,
                ..still
            },
            DamageIn {
                backdrop: true,
                ..still
            },
            DamageIn {
                page_built: true,
                ..still
            },
            DamageIn {
                glass: &moved,
                ..still
            },
            DamageIn {
                last_glass: None,
                ..still
            },
            DamageIn {
                last_card: None,
                ..still
            },
            DamageIn {
                card: None,
                ..still
            },
            DamageIn {
                last_card: Some(StartCard {
                    radius: 0.0,
                    ..card
                }),
                ..still
            },
        ];
        for c in cases {
            assert_eq!(damage(&c), Damage::Whole, "{c:?}");
        }
    }

    fn sized() -> Scene {
        let mut s = Scene::new(SceneOptions::default(), 7);
        s.resize(800.0, 600.0, 2.0);
        // the resize's seeding splats
        let _ = s.fluid_ops(0.0);
        s
    }

    #[test]
    fn a_pop_pushes_the_liquid_as_scene_pop_does() {
        let mut s = sized();
        apply(
            &mut s,
            StartFx::Pop {
                x: 200.0,
                y: 150.0,
                r: 10.0,
                strength: 500.0,
                k: 0.6,
                min: 0.002,
            },
        );
        let ops = s.fluid_ops(0.0);
        assert_eq!(
            ops[0],
            super::super::state::FluidOp::Ripple {
                x: 0.25,
                y: 0.75,
                strength: 500.0,
                radius: (10.0_f32 / 800.0 * 0.6).max(0.002),
            }
        );
        // a tiny pop's push keeps the floor
        apply(
            &mut s,
            StartFx::Pop {
                x: 0.0,
                y: 0.0,
                r: 0.1,
                strength: 500.0,
                k: 0.6,
                min: 0.002,
            },
        );
        assert!(matches!(
            s.fluid_ops(0.0)[0],
            super::super::state::FluidOp::Ripple { radius, .. } if radius == 0.002
        ));
    }

    #[test]
    fn a_ripple_rings_the_backdrop_and_pushes_the_liquid() {
        let mut s = sized();
        apply(
            &mut s,
            StartFx::Ripple {
                x: 400.0,
                y: 300.0,
                reach: 300.0,
                strength: 1.0,
            },
        );
        assert_eq!(s.ripples.len(), 1);
        assert!(matches!(
            s.fluid_ops(0.0)[0],
            super::super::state::FluidOp::Ripple { strength, .. } if strength == 1100.0
        ));
    }

    #[test]
    fn the_light_follows_the_pointer_and_leaves_with_it() {
        let mut s = sized();
        apply(&mut s, StartFx::Light { x: 100.0, y: 50.0 });
        s.advance(1.0, 1.0);
        assert!((s.light[0] - 100.0).abs() < 1.0 && (s.light[1] - 50.0).abs() < 1.0);
        // a move pushes the liquid (the last of the frame's splats: emitters, then the pointer)
        apply(&mut s, StartFx::Light { x: 130.0, y: 50.0 });
        let pushes = |s: &mut Scene| {
            s.fluid_ops(0.0)
                .iter()
                .filter(|op| matches!(op, super::super::state::FluidOp::Splat { color: None, .. }))
                .count()
        };
        assert_eq!(pushes(&mut s), 1);
        // gone: no push, even when it comes back somewhere else (no jump)
        apply(&mut s, StartFx::LightLeave);
        assert_eq!(pushes(&mut s), 0);
        apply(&mut s, StartFx::Light { x: 400.0, y: 300.0 });
        assert_eq!(pushes(&mut s), 0);
    }

    #[test]
    fn a_bubble_s_uniforms_carry_its_fields_and_the_frame_s() {
        let b = BubbleDraw {
            quad: [1.0, 2.0, 3.0, 4.0],
            c: [5.0, 6.0],
            r: 7.0,
            has_w: true,
            wave_n: 9,
            ..Default::default()
        };
        let g = Globals {
            view: [800.0, 600.0],
            dpr: 2.0,
            light: [10.0, 20.0],
            time: 3.0,
        };
        let u = bubble_u(&b, g);
        assert_eq!((u.u_quad, u.u_c, u.u_r), (b.quad, b.c, b.r));
        assert_eq!(
            (u.u_view, u.u_dpr, u.u_light, u.u_time),
            (g.view, 2.0, g.light, 3.0)
        );
        assert_eq!((u.u_has_w, u.u_overlay, u.u_wave_n), (1.0, 0.0, 4));
        let p = pop_u(
            &PopDraw {
                k: 0.5,
                ..Default::default()
            },
            g,
        );
        assert_eq!((p.u_k, p.u_view, p.u_dpr), (0.5, g.view, 2.0));
    }
}
