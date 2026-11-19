//! The CPU half of the scene: glass surfaces, the beam, palette, pointer light,
//! ripples, ambient emitters and the queue of fluid ops. Pure: no GPU, no clock
//! (the caller passes `dt`).
//!
//! Source: the-wallpaper-project `src/wubble/slint/render.rs` (`Scene`,
//! `GlassSurface`, `Beam`, `FluidOptions`, `SceneOptions`, `wave_uniforms`),
//! `softbody.rs` (`Wave`, `push_wave`, `age_waves`), `minis.rs` (`Rng`) and
//! `browser/chrome.tsx` (the address bar's loading beam curve). Owner's code.
//! Left out: bubbles, pops, `stir`, the voice glow and overlay mode (the
//! deferred start page and web overlay layer).

use super::pages::OverLayer;

// @source slint-rng (minis.rs)
/// xorshift64* in [0, 1), standing in for `Math.random()`.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn random(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

// @source slint-wave (softbody.rs)
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
    if waves.len() > 4 {
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

/// A rectangle in logical px, top-left origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// Glass thickness. Mirrored by the Slint `GlassTier` (checked in
/// `crates/wubble/src/backdrop.rs`, test `slint_glass_tier_mirrors_the_scene`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    Thin,
    Regular,
    Thick,
}

impl Tier {
    pub const ALL: [Tier; 3] = [Tier::Thin, Tier::Regular, Tier::Thick];

    /// `uTier` in glass.wgsl: 0 thin, 1 regular, 2 thick.
    pub const fn shader_value(self) -> f64 {
        match self {
            Tier::Thin => 0.0,
            Tier::Regular => 1.0,
            Tier::Thick => 2.0,
        }
    }
}

/// One glass panel of the chrome layout, as the host knows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassRect {
    pub rect: Rect,
    /// Corner radius, logical px.
    pub radius: f32,
    pub tier: Tier,
    pub pressed: bool,
    pub hover: bool,
    pub focus: bool,
    /// wubble: RGB 0..1 mixed into the glass (`GlassSurface::tint`); black = none. The
    /// window's glass carries the Space colour (design desktop.tsx `glass.tint = accent`).
    pub tint: [f32; 3],
    /// The drop shadow's weight (`GlassSurface::shadow`): the design's per surface, set by the
    /// host (window 1, address 0, favourites 0.3, else `useGlass`'s 0.6).
    pub shadow: f32,
    /// This panel carries the border beam (the address bar): the beam is drawn in its own glass
    /// pass, `col += fx` (scene.ts GLASS), as the design does.
    pub beam: bool,
    /// wubble: an overlay's glass, drawn above the page in that layer's image; `None` is under
    /// the page (the window's glass, the address bar, favourites).
    pub layer: Option<OverLayer>,
    /// The panel's own opacity (its entrance fade): `GlassSurface::alpha`.
    pub alpha: f32,
}

// @source slint-render-scene-types (render.rs)
/// Mirrors `GlassSurface` in scene.ts. Logical px, top-left origin.
#[derive(Clone, Debug, PartialEq)]
pub struct GlassSurface {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub r: f64,
    /// 0 thin, 1 regular, 2 thick
    pub tier: f64,
    /// 0..1, the touched film rim
    pub film: f64,
    pub alpha: f64,
    /// RGB 0..1 tint (`tint * .55 + .1` mixed in at 7 %); black = none
    pub tint: [f32; 3],
    pub shadow: f64,
    /// waves running through the glass (presses)
    pub waves: Vec<Wave>,
    /// a beam of light running round the rim
    pub beam: Option<Beam>,
    /// 1 = full glass; 0 = effects only (beam, glow) for surfaces drawn over live content
    pub material: f64,
    /// wubble: the overlay layer it is drawn in (`GlassRect::layer`); `None` under the page.
    pub layer: Option<OverLayer>,
}

impl Default for GlassSurface {
    /// The TS defaults for omitted fields: alpha 1, shadow 1, material 1, no tint.
    fn default() -> Self {
        GlassSurface {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            r: 0.0,
            tier: 0.0,
            film: 0.0,
            alpha: 1.0,
            tint: [0.0; 3],
            shadow: 1.0,
            waves: vec![],
            beam: None,
            material: 1.0,
            layer: None,
        }
    }
}

/// Border beam: head and tail length as fractions of the perimeter, strength 0..1, hue offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    pub head: f64,
    pub length: f64,
    pub strength: f64,
    pub hue: f64,
}

/// The glass surface for one panel of the chrome: the panel's shadow weight, the film rim on
/// while the panel is hovered, pressed or focused (browser/chrome.tsx sets `film = 1` on enter).
pub fn surface_of(g: &GlassRect) -> GlassSurface {
    let touched = g.pressed || g.hover || g.focus;
    GlassSurface {
        x: g.rect.x.into(),
        y: g.rect.y.into(),
        w: g.rect.w.into(),
        h: g.rect.h.into(),
        r: g.radius.into(),
        tier: g.tier.shader_value(),
        film: if touched { 1.0 } else { 0.0 },
        shadow: g.shadow.into(),
        tint: g.tint,
        alpha: g.alpha.clamp(0.0, 1.0).into(),
        layer: g.layer,
        ..Default::default()
    }
}

/// Film rim fade in and out, seconds (the design's `wubble.css` `.wb-film`: 200 ms in,
/// 150 ms out; the Slint fallback `GlassPanel` uses the same).
pub const FILM_IN: f64 = 0.2;
pub const FILM_OUT: f64 = 0.15;

/// wubble: the film rim's level `dt` seconds after `prev`, easing linearly toward 1 while
/// `touched` and toward 0 otherwise, as the CSS opacity transition does. Never at rest unless
/// touched.
pub fn film_level(prev: f64, touched: bool, dt: f64) -> f64 {
    if touched {
        (prev + dt / FILM_IN).min(1.0)
    } else {
        (prev - dt / FILM_OUT).max(0.0)
    }
}

/// `waveUniforms`: the newest (at most 4) waves as (x, y, age/life, reach).
pub(super) fn wave_uniforms(waves: &[Wave]) -> ([[f32; 4]; 4], i32) {
    let n = waves.len().min(4);
    let mut out = [[0.0; 4]; 4];
    for (i, o) in out.iter_mut().enumerate().take(n) {
        let w = waves[waves.len() - n + i];
        *o = [
            w.x as f32,
            w.y as f32,
            (w.age / w.life) as f32,
            w.reach as f32,
        ];
    }
    (out, n as i32)
}

#[derive(Clone, Copy, Debug)]
pub struct FluidOptions {
    pub sim_res: u32,
    pub dye_res: u32,
    /// Jacobi iterations at `Quality::Full`.
    pub pressure_iters: u32,
    pub curl: f32,
    pub vel_diss: f32,
    pub dye_diss: f32,
}

impl Default for FluidOptions {
    /// The calm defaults in fluid.ts: curl 2.5, velDiss 0.8, dyeDiss 0.12.
    fn default() -> Self {
        FluidOptions {
            sim_res: 96,
            dye_res: 256,
            pressure_iters: 18,
            curl: 2.5,
            vel_diss: 0.8,
            dye_diss: 0.12,
        }
    }
}

/// What the backdrop under the glass is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backdrop {
    /// The design's living liquid: emitters, the pointer's push and the fluid step every frame.
    #[default]
    Liquid,
    /// wubble (owner override, 2026-11-19: "we can stop the wave effect in the side navbar and
    /// make it still frosted translucent glass"): the Space-tinted dye held still. The dye is the
    /// table in `still_dye.txt` ([`still_blobs`]), splatted afresh when the palette or the
    /// window's shape changes; no fluid step, emitters or pushes. Tap ripples still run through
    /// it (they are drawn by the composite, not the fluid).
    Still,
}

/// One blob of the still dye, as `still_dye.txt` gives it: centre as fractions of the window
/// (top-left origin), which of the Space's three colours, `Fluid.splat`'s radius for a canvas as
/// tall as the window, and the colour's scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blob {
    pub x: f32,
    pub y: f32,
    pub color: usize,
    pub radius: f32,
    pub strength: f32,
}

/// The still dye's table, one copy for wubble and the fidelity harness's frozen web liquid
/// (`tools/fidelity/vite.config.mjs` reads the same file).
pub const STILL_DYE: &str = include_str!("still_dye.txt");

/// The blobs of [`STILL_DYE`]. Lines are `fx fy colour radius strength`; `#` starts a comment.
/// Panics on a malformed line: the table is compiled in and checked by a test.
pub fn still_blobs() -> Vec<Blob> {
    STILL_DYE
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v: Vec<f32> = l
                .split_whitespace()
                .map(|x| x.parse().unwrap_or_else(|_| panic!("still_dye.txt: {l}")))
                .collect();
            assert_eq!(v.len(), 5, "still_dye.txt: {l}");
            Blob {
                x: v[0],
                y: v[1],
                color: v[2] as usize,
                radius: v[3],
                strength: v[4],
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct SceneOptions {
    /// The Space's three dye colours, RGB 0..1.
    pub palette: [[f32; 3]; 3],
    pub ground: [f32; 3],
    pub fluid: FluidOptions,
    pub ambient: f32,
    pub pointer_force: f32,
    /// how strongly the liquid shows over the ground, 0..1 (0.3 = muted, the default)
    pub intensity: f32,
    /// wubble: a living liquid or the still dye.
    pub backdrop: Backdrop,
}

/// `0xRRGGBB` to RGB 0..1.
pub fn hex(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}

impl Default for SceneOptions {
    fn default() -> Self {
        SceneOptions {
            palette: [hex(0x2F5FC4), hex(0xA84886), hex(0x237E69)],
            ground: hex(0x08090D),
            fluid: FluidOptions::default(),
            ambient: 1.0,
            pointer_force: 1.0,
            intensity: 0.3,
            backdrop: Backdrop::Liquid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Ripple {
    pub x: f32,
    pub y: f32,
    pub age: f32,
    pub life: f32,
    pub reach: f32,
}

struct Emitter {
    phase: f32,
    speed: f32,
    ax: f32,
    ay: f32,
    fx: f32,
    fy: f32,
    color: usize,
}

/// What `Scene::resize` asks the renderer to do.
/// `targets`: reallocate the screen-sized targets; `fluid`: reallocate the fluid grids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resized {
    pub targets: bool,
    pub fluid: bool,
}

/// A fluid splat in sim coordinates (0..1, y up), as `Fluid.splat` / `Fluid.ripple` take them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FluidOp {
    Splat {
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        color: Option<[f32; 3]>,
        radius: f32,
    },
    Ripple {
        x: f32,
        y: f32,
        strength: f32,
        radius: f32,
    },
    /// wubble: a splat of dye only (the still dye): `Splat` without its velocity pass.
    Dye {
        x: f32,
        y: f32,
        color: [f32; 3],
        radius: f32,
    },
}

/// Most glass surfaces drawn (each takes a 256-byte uniform slice).
pub const MAX_GLASS: usize = 64;

/// wubble: the panels [`Scene::set_glass`] keeps, in order: all of them up to [`MAX_GLASS`];
/// past it every overlay's first (an overlay has no Slint fill of its own while the scene draws
/// its glass), then the ones under the page back to front (the window's glass leads them).
fn kept(rects: &[GlassRect]) -> Vec<&GlassRect> {
    let mut over = rects
        .iter()
        .filter(|g| g.layer.is_some())
        .count()
        .min(MAX_GLASS);
    let mut under = MAX_GLASS - over;
    rects
        .iter()
        .filter(|g| {
            let left = if g.layer.is_some() {
                &mut over
            } else {
                &mut under
            };
            let keep = *left > 0;
            *left = left.saturating_sub(1);
            keep
        })
        .collect()
}
/// Most queued fluid ops kept while the backdrop is not rendered (Cached or
/// Flat quality); the oldest go first.
const MAX_OPS: usize = 64;

// @source slint-render-scene (render.rs)
/// The CPU half of `Scene`: everything except the GPU calls. Methods match scene.ts.
pub struct Scene {
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) dpr: f32,
    fluid_aspect: f32,
    pub(super) time: f32,
    pub(super) glass: Vec<GlassSurface>,
    /// wubble: whether each surface is touched; its `film` eases toward it ([`film_level`]).
    touched: Vec<bool>,
    /// The surface that carries the border beam ([`GlassRect::beam`]), and the beam's shape this
    /// frame (the shell owns its lifecycle: `wubble_core::beam`).
    beam_at: Option<usize>,
    beam_shape: Option<Beam>,
    /// [`Backdrop::Still`]: the dye must be splatted afresh (palette, shape), and the backdrop
    /// composited again (the dye changed, ripples moved or just ended). Taken by the renderer.
    dye_stale: bool,
    backdrop_stale: bool,
    pub(super) ripples: Vec<Ripple>,
    pub(super) light: [f32; 2],
    light_target: [f32; 2],
    pointer: (f32, f32, f32, f32, bool), // x, y, px, py, has
    palette: [[f32; 3]; 3],
    palette_target: [[f32; 3]; 3],
    pub(super) ground: [f32; 3],
    emitters: Vec<Emitter>,
    pub(super) opts: SceneOptions,
    ops: Vec<FluidOp>,
    rng: Rng,
}

impl Scene {
    pub fn new(o: SceneOptions, seed: u64) -> Self {
        let emitters = (0..5)
            .map(|i| {
                let f = i as f32;
                Emitter {
                    phase: f * 1.7,
                    speed: 0.018 + f * 0.005,
                    ax: 0.32 + 0.06 * f,
                    ay: 0.28 + 0.04 * (4.0 - f),
                    fx: 1.0 + (i % 2) as f32,
                    fy: 2.0 - (i % 2) as f32 * 1.0,
                    color: i % 3,
                }
            })
            .collect();
        Scene {
            w: 0.0,
            h: 0.0,
            dpr: 1.0,
            fluid_aspect: 0.0,
            time: 0.0,
            glass: vec![],
            touched: vec![],
            beam_at: None,
            beam_shape: None,
            dye_stale: false,
            backdrop_stale: false,
            ripples: vec![],
            light: [0.0; 2],
            light_target: [0.0; 2],
            pointer: (0.0, 0.0, 0.0, 0.0, false),
            palette: o.palette,
            palette_target: o.palette,
            ground: o.ground,
            emitters,
            opts: o,
            ops: vec![],
            rng: Rng::new(seed),
        }
    }

    /// The Space's dye colours; the liquid crossfades to them in about 600 ms.
    pub fn set_palette(&mut self, colors: [[f32; 3]; 3]) {
        self.palette_target = colors;
        self.dye_stale = true;
    }

    fn still(&self) -> bool {
        self.opts.backdrop == Backdrop::Still
    }

    /// [`Backdrop::Still`]: what the backdrop needs this frame, once: (splat the dye afresh,
    /// composite the backdrop again). Always `(false, false)` for a living liquid, whose backdrop
    /// renders by the quality's clock.
    pub(super) fn take_still_work(&mut self) -> (bool, bool) {
        let dye = std::mem::take(&mut self.dye_stale);
        let backdrop = std::mem::take(&mut self.backdrop_stale);
        if self.still() {
            (dye, dye || backdrop)
        } else {
            (false, false)
        }
    }

    /// The still dye's splats for the current palette ([`still_blobs`]), in sim coordinates.
    pub(super) fn still_ops(&self) -> Vec<FluidOp> {
        still_blobs()
            .into_iter()
            .map(|b| {
                let c = self.palette[b.color % 3];
                FluidOp::Dye {
                    x: b.x,
                    y: 1.0 - b.y,
                    color: c.map(|v| v * b.strength),
                    radius: b.radius,
                }
            })
            .collect()
    }

    /// The chrome's glass panels, back to front; at most [`MAX_GLASS`] are
    /// kept. Call on every layout or hover/press/focus change. Surface `i`
    /// keeps the press waves still running in the old surface `i`, and (wubble) its film
    /// level, which then eases toward the new touch state in [`Scene::advance`].
    pub fn set_glass(&mut self, rects: &[GlassRect]) {
        let mut old = std::mem::take(&mut self.glass).into_iter();
        self.touched.clear();
        let kept = kept(rects);
        self.beam_at = kept.iter().position(|g| g.beam);
        self.glass = kept
            .into_iter()
            .map(|g| {
                let new = surface_of(g);
                self.touched.push(new.film > 0.0);
                let (waves, film) = old.next().map_or((vec![], 0.0), |s| (s.waves, s.film));
                GlassSurface { waves, film, ..new }
            })
            .collect();
    }

    /// The palette is still crossfading to the Space's colours.
    fn palette_moving(&self) -> bool {
        self.palette
            .iter()
            .zip(&self.palette_target)
            .any(|(a, b)| a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-3))
    }

    /// wubble: whether anything in the scene still moves on its own besides the ambient
    /// drift: tap waves, ripples, the beam lap, the palette crossfade, the light catching up
    /// with the pointer, a film rim easing. The run state (`run.rs`) keeps full rate while it does.
    pub fn animating(&self) -> bool {
        let film = self
            .glass
            .iter()
            .zip(&self.touched)
            .any(|(g, &t)| g.film != if t { 1.0 } else { 0.0 });
        let palette = self.palette_moving();
        let light = (self.light[0] - self.light_target[0]).abs() > 0.5
            || (self.light[1] - self.light_target[1]).abs() > 0.5;
        film || palette
            || light
            || !self.ripples.is_empty()
            || !self.ops.is_empty()
            || self.glass.iter().any(|g| !g.waves.is_empty())
            || self.beam_shape.is_some()
            || self.dye_stale
            || self.backdrop_stale
    }

    /// The beam this frame (`None`: nothing drawn), drawn in the glass of the panel that carries
    /// it ([`GlassRect::beam`]). wubble: the design times its lap in chrome.tsx `Address`; here
    /// the shell's `wubble_core::beam` does, and hands the shape over every frame. Kept without
    /// such a panel, for when one arrives.
    pub fn set_beam_shape(&mut self, shape: Option<Beam>) {
        self.beam_shape = shape;
    }

    /// The surfaces drawn this frame, back to front: the glass list with the beam on its panel,
    /// without the ones the design skips (`alpha <= .001`, under 1 px; scene.ts `drawGlass`).
    pub(super) fn surfaces(&self) -> Vec<GlassSurface> {
        let beam = self.beam_shape.filter(|s| s.strength > 0.001);
        self.glass
            .iter()
            .enumerate()
            .filter(|(_, s)| s.alpha > 0.001 && s.w >= 1.0 && s.h >= 1.0)
            .map(|(i, s)| GlassSurface {
                beam: beam.filter(|_| self.beam_at == Some(i)),
                ..s.clone()
            })
            .collect()
    }

    /// Reallocation the renderer must do for a view of `w`×`h` logical px.
    pub fn resize(&mut self, w: f32, h: f32, dpr: f32) -> Resized {
        if w == self.w && h == self.h && dpr == self.dpr {
            return Resized {
                targets: false,
                fluid: false,
            };
        }
        // a still dye is splatted afresh for any new size, as the harness's web side does: kept,
        // it would stretch with the window (a living liquid is sampled by uv and survives)
        let still_reshaped = self.still() && (w, h) != (self.w, self.h);
        self.w = w;
        self.h = h;
        self.dpr = dpr;
        // the liquid survives resizes: it's sampled by uv, so only a big change of shape reallocates it
        let aspect = w / h.max(1.0);
        if !still_reshaped
            && self.fluid_aspect != 0.0
            && (aspect / self.fluid_aspect).ln().abs() < 0.35
        {
            return Resized {
                targets: true,
                fluid: false,
            };
        }
        let fresh = self.fluid_aspect == 0.0;
        self.fluid_aspect = aspect;
        self.ops.clear();
        if self.still() {
            // new grids, an empty dye: the still dye is splatted afresh
            self.dye_stale = true;
            return Resized {
                targets: true,
                fluid: true,
            };
        }
        if !fresh {
            for i in 0..6 {
                let c = self.palette[i % 3];
                let r = &mut self.rng;
                let (x, y) = (
                    0.15 + r.random() as f32 * 0.7,
                    0.15 + r.random() as f32 * 0.7,
                );
                self.ops.push(FluidOp::Splat {
                    x,
                    y,
                    dx: 0.0,
                    dy: 0.0,
                    color: Some([c[0] * 0.9, c[1] * 0.9, c[2] * 0.9]),
                    radius: 0.03,
                });
            }
            return Resized {
                targets: true,
                fluid: true,
            };
        }
        // seed the liquid so the first frame isn't empty
        for i in 0..9 {
            let c = self.palette[i % 3];
            let r = &mut self.rng;
            let (x, y) = (
                0.15 + r.random() as f32 * 0.7,
                0.15 + r.random() as f32 * 0.7,
            );
            let (dx, dy) = (
                (r.random() as f32 - 0.5) * 80.0,
                (r.random() as f32 - 0.5) * 80.0,
            );
            self.ops.push(FluidOp::Splat {
                x,
                y,
                dx,
                dy,
                color: Some([c[0] * 0.9, c[1] * 0.9, c[2] * 0.9]),
                radius: 0.02,
            });
        }
        Resized {
            targets: true,
            fluid: true,
        }
    }

    fn push_op(&mut self, op: FluidOp) {
        if self.ops.len() >= MAX_OPS {
            self.ops.remove(0);
        }
        self.ops.push(op);
    }

    /// Pointer in logical px. Pushes the liquid along its motion and moves the light.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        let p = &mut self.pointer;
        if !p.4 {
            p.2 = x;
            p.3 = y;
        }
        p.0 = x;
        p.1 = y;
        p.4 = true;
        self.light_target = [x, y];
    }

    pub fn pointer_leave(&mut self) {
        self.pointer.4 = false;
    }

    /// A tap or press at (x, y), logical px. On glass, a press wave through the front-most
    /// glass under the point (chrome.tsx `Fav`: reach 90, life 0.7); on the bare backdrop, a
    /// ring wave through it and an outward push in the liquid (desktop.tsx `deskDown`:
    /// `ripple(x, y, 240, 0.6)`). README › Shaders: "presses send one through glass; taps send
    /// one through the liquid".
    pub fn tap(&mut self, x: f32, y: f32) {
        match self
            .glass
            .iter_mut()
            .rev()
            .find(|g| Rect::new(g.x as f32, g.y as f32, g.w as f32, g.h as f32).contains(x, y))
        {
            Some(g) => push_wave(&mut g.waves, x.into(), y.into(), 90.0, 0.7),
            None => self.ripple(x, y, 240.0, 0.6),
        }
    }

    /// `Scene.ripple`: a wave ring through the backdrop and an outward push in the liquid.
    fn ripple(&mut self, x: f32, y: f32, reach: f32, strength: f32) {
        self.ripples.push(Ripple {
            x,
            y,
            age: 0.0,
            life: 1.15,
            reach,
        });
        if self.ripples.len() > 8 {
            self.ripples.remove(0);
        }
        if self.w > 0.0 && !self.still() {
            self.push_op(FluidOp::Ripple {
                x: x / self.w,
                y: 1.0 - y / self.h,
                strength: 1100.0 * strength,
                radius: 0.01,
            });
        }
    }

    // @source slint-render-emitters (render.rs `advance`, split in two)
    /// The per-frame CPU part of `Scene.frame` that runs whether or not the
    /// backdrop is rendered: time, glass waves, palette crossfade, light lag,
    /// and ripple ageing. `pace` (0 or 1: the fidelity harness's frozen clock) scales only the
    /// scene's clock.
    pub(super) fn advance(&mut self, dt: f32, pace: f32) {
        // a still dye follows the crossfade by being splatted afresh while it moves, and the
        // backdrop is composited while ripples run and once more when the last one ends
        self.dye_stale |= self.palette_moving();
        self.backdrop_stale |= !self.ripples.is_empty();
        self.time += dt * pace;
        for g in &mut self.glass {
            age_waves(&mut g.waves, dt.into());
        }
        for (g, &t) in self.glass.iter_mut().zip(&self.touched) {
            g.film = film_level(g.film, t, dt.into());
        }
        // palette crossfade (Space switch): ~600 ms
        let k = 1.0 - (-dt * 5.0).exp();
        for (i, c) in self.palette.iter_mut().enumerate() {
            let g = self.palette_target[i % 3];
            for j in 0..3 {
                c[j] += (g[j] - c[j]) * k;
            }
        }
        // light lags the pointer slightly, like a reflection
        let lk = 1.0 - (-dt * 18.0).exp();
        self.light[0] += (self.light_target[0] - self.light[0]) * lk;
        self.light[1] += (self.light_target[1] - self.light[1]) * lk;
        // ripples age here (the TS filters them while building the uniform array)
        self.ripples.retain_mut(|r| {
            r.age += dt;
            r.age < r.life
        });
    }

    /// The fluid ops of a frame whose backdrop is rendered, in order: the
    /// queued ones (resize seeds, taps), then the ambient emitters and the
    /// pointer push for `dt`.
    pub(super) fn fluid_ops(&mut self, dt: f32) -> Vec<FluidOp> {
        let mut ops = std::mem::take(&mut self.ops);
        // ambient emitters: slow Lissajous paths, each pushing a little dye along its tangent
        let amb = self.opts.ambient;
        if amb > 0.0 {
            for e in &mut self.emitters {
                e.phase += dt * e.speed;
                let x = 0.5 + e.ax * (e.phase * e.fx * 2.1).sin();
                let y = 0.5 + e.ay * (e.phase * e.fy * 1.7 + 1.3).sin();
                let dx = e.ax * (e.phase * e.fx * 2.1).cos() * e.fx * 2.1;
                let dy = e.ay * (e.phase * e.fy * 1.7 + 1.3).cos() * e.fy * 1.7;
                let c = self.palette[e.color];
                let a = 0.5 * dt * amb;
                ops.push(FluidOp::Splat {
                    x,
                    y,
                    dx: dx * 14.0 * amb,
                    dy: dy * 14.0 * amb,
                    color: Some([c[0] * a, c[1] * a, c[2] * a]),
                    radius: 0.012,
                });
            }
        }
        let pf = self.opts.pointer_force;
        let p = &mut self.pointer;
        if p.4 && pf > 0.0 {
            let (dx, dy) = (p.0 - p.2, p.1 - p.3);
            if dx * dx + dy * dy > 0.25 {
                ops.push(FluidOp::Splat {
                    x: p.0 / self.w,
                    y: 1.0 - p.1 / self.h,
                    dx: dx * 4.0 * pf,
                    dy: -dy * 4.0 * pf,
                    color: None,
                    radius: 0.0016,
                });
            }
            p.2 = p.0;
            p.3 = p.1;
        }
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> GlassRect {
        GlassRect {
            rect: Rect::new(x, y, w, h),
            radius: 12.0,
            tier: Tier::Regular,
            pressed: false,
            hover: false,
            focus: false,
            tint: [0.0; 3],
            shadow: 0.6,
            beam: false,
            layer: None,
            alpha: 1.0,
        }
    }

    fn sized() -> Scene {
        let mut s = Scene::new(SceneOptions::default(), 0x5eed);
        s.resize(800.0, 600.0, 2.0);
        s
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1000 {
            let x = a.random();
            assert_eq!(x, b.random());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(7).random(), Rng::new(8).random());
        // a zero seed would stick at zero
        assert!(Rng::new(0).random() > 0.0);
    }

    #[test]
    fn resize_seeds_once_and_keeps_the_fluid_across_small_shape_changes() {
        let mut s = Scene::new(SceneOptions::default(), 1);
        let r = s.resize(800.0, 600.0, 1.0);
        assert_eq!(
            r,
            Resized {
                targets: true,
                fluid: true
            }
        );
        assert_eq!(s.ops.len(), 9);
        assert_eq!(
            s.resize(800.0, 600.0, 1.0),
            Resized {
                targets: false,
                fluid: false
            }
        );
        // same aspect at another dpr, then a small change of shape: targets only
        assert_eq!(
            s.resize(800.0, 600.0, 2.0),
            Resized {
                targets: true,
                fluid: false
            }
        );
        assert_eq!(
            s.resize(900.0, 600.0, 2.0),
            Resized {
                targets: true,
                fluid: false
            }
        );
        // over e^0.35 of the first aspect: new grids, 6 calm seeds
        assert_eq!(
            s.resize(1600.0, 600.0, 2.0),
            Resized {
                targets: true,
                fluid: true
            }
        );
        assert_eq!(s.ops.len(), 6);
    }

    #[test]
    fn palette_crossfades_in_dt_steps() {
        let mut s = sized();
        let target = [hex(0xFFFFFF); 3];
        s.set_palette(target);
        s.advance(0.1, 1.0);
        assert!(s.palette[0][0] < 0.9, "no jump");
        for _ in 0..12 {
            s.advance(1.0 / 20.0, 1.0);
        }
        // 0.7 s at rate 5: within 3% of the target
        for c in s.palette {
            for v in c {
                assert!(v > 0.97, "{v}");
            }
        }
    }

    #[test]
    fn pace_scales_the_clock_and_nothing_else() {
        let mut s = sized();
        s.tap(1.0, 1.0);
        s.advance(0.5, 0.0);
        // a frozen liquid: the clock stands, the tap wave still ages at the real rate
        assert_eq!(s.time, 0.0);
        assert!((s.ripples[0].age - 0.5).abs() < 1e-6);
        s.advance(0.5, 0.5);
        assert!((s.time - 0.25).abs() < 1e-6, "{}", s.time);
        assert!(s.ripples.is_empty() || (s.ripples[0].age - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ripples_cap_at_eight_and_age_out() {
        let mut s = sized();
        for i in 0..10 {
            s.tap(i as f32, 0.0);
        }
        assert_eq!(s.ripples.len(), 8);
        assert_eq!(s.ripples[0].x, 2.0);
        s.advance(1.2, 1.0);
        assert!(s.ripples.is_empty());
    }

    #[test]
    fn waves_cap_at_four_and_age() {
        let mut w = vec![];
        for i in 0..6 {
            push_wave(&mut w, i as f64, 0.0, 90.0, 0.7);
        }
        assert_eq!(w.len(), 4);
        assert_eq!(w[0].x, 2.0);
        age_waves(&mut w, 0.5);
        assert_eq!(w.len(), 4);
        let (u, n) = wave_uniforms(&w);
        assert_eq!(n, 4);
        assert_eq!(u[3], [5.0, 0.0, (0.5f64 / 0.7) as f32, 90.0]);
        age_waves(&mut w, 0.3);
        assert!(w.is_empty());
    }

    #[test]
    fn surface_of_maps_tier_radius_and_touch() {
        let mut g = rect(10.0, 20.0, 30.0, 40.0);
        g.tier = Tier::Thick;
        let s = surface_of(&g);
        assert_eq!((s.x, s.y, s.w, s.h, s.r), (10.0, 20.0, 30.0, 40.0, 12.0));
        assert_eq!((s.tier, s.film, s.material), (2.0, 0.0, 1.0));
        for touch in [
            GlassRect { pressed: true, ..g },
            GlassRect { hover: true, ..g },
            GlassRect { focus: true, ..g },
        ] {
            assert_eq!(surface_of(&touch).film, 1.0);
        }
        let tiers: Vec<f64> = Tier::ALL.iter().map(|t| t.shader_value()).collect();
        assert_eq!(tiers, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn tap_waves_the_front_glass_and_set_glass_keeps_running_waves() {
        let mut s = sized();
        s.set_glass(&[rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 100.0, 100.0)]);
        s.tap(60.0, 60.0);
        assert!(s.glass[0].waves.is_empty());
        assert_eq!(s.glass[1].waves.len(), 1);
        // a hover on the same layout keeps the wave
        let mut hovered = rect(50.0, 50.0, 100.0, 100.0);
        hovered.hover = true;
        s.set_glass(&[rect(0.0, 0.0, 100.0, 100.0), hovered]);
        assert_eq!(s.glass[1].waves.len(), 1);
        // wubble: the film rim eases in from where it was (200 ms) instead of snapping
        assert_eq!(s.glass[1].film, 0.0);
        assert!(s.animating());
        s.advance(0.1, 1.0);
        assert!((s.glass[1].film - 0.5).abs() < 1e-6);
        s.advance(0.2, 1.0);
        assert_eq!(s.glass[1].film, 1.0);
        assert!(
            s.ripples.is_empty(),
            "a press on glass waves the glass only"
        );
        // outside every panel: backdrop only
        s.tap(500.0, 500.0);
        assert_eq!(s.glass[1].waves.len(), 1);
        assert_eq!((s.ripples.len(), s.ripples[0].reach), (1, 240.0));
        s.set_glass(&[]);
        assert!(s.glass.is_empty());
        let many = vec![rect(0.0, 0.0, 10.0, 10.0); MAX_GLASS + 5];
        s.set_glass(&many);
        assert_eq!(s.glass.len(), MAX_GLASS);
        // full of favourites: an overlay past the cap is kept, the last ones under the page go
        let mut crowded = many.clone();
        crowded.push(GlassRect {
            layer: Some(OverLayer::Top),
            ..rect(1.0, 2.0, 30.0, 40.0)
        });
        s.set_glass(&crowded);
        assert_eq!(s.glass.len(), MAX_GLASS);
        assert_eq!(s.glass[MAX_GLASS - 1].layer, Some(OverLayer::Top));
        assert!(s.glass[..MAX_GLASS - 1].iter().all(|g| g.layer.is_none()));
    }

    #[test]
    fn film_eases_in_and_out_and_clamps() {
        assert_eq!(film_level(0.0, true, 0.1), 0.5);
        assert_eq!(film_level(0.9, true, 1.0), 1.0);
        assert!((film_level(1.0, false, 0.075) - 0.5).abs() < 1e-12);
        assert_eq!(film_level(0.1, false, 1.0), 0.0);
        assert_eq!(film_level(0.0, false, 0.0), 0.0);
        // monotone toward the target
        let mut f = 0.0;
        for _ in 0..30 {
            let n = film_level(f, true, 1.0 / 120.0);
            assert!(n >= f);
            f = n;
        }
    }

    #[test]
    fn a_settled_scene_is_not_animating() {
        let mut s = sized();
        assert!(s.animating(), "the resize seeds are queued");
        s.fluid_ops(0.0);
        assert!(!s.animating());
        s.set_glass(&[rect(0.0, 0.0, 100.0, 100.0)]);
        assert!(!s.animating(), "untouched glass rests at film 0");
        s.tap(10.0, 10.0);
        assert!(s.animating());
    }

    #[test]
    fn the_beam_is_drawn_in_the_glass_that_carries_it() {
        let mut s = sized();
        let shape = Beam {
            head: 0.4,
            length: 0.35,
            strength: 0.8,
            hue: 0.1,
        };
        s.set_beam_shape(Some(shape));
        let mut address = rect(100.0, 10.0, 300.0, 32.0);
        s.set_glass(&[rect(0.0, 0.0, 800.0, 600.0), address]);
        assert!(
            s.surfaces().iter().all(|g| g.beam.is_none()),
            "no panel carries it"
        );
        address.beam = true;
        address.shadow = 0.0;
        s.set_glass(&[rect(0.0, 0.0, 800.0, 600.0), address]);
        let surfaces = s.surfaces();
        assert_eq!(surfaces.len(), 2, "no surface of its own");
        assert_eq!(surfaces[0].beam, None);
        assert_eq!(surfaces[1].beam, Some(shape));
        // full glass, with the panel's own shadow weight
        assert_eq!((surfaces[1].material, surfaces[1].shadow), (1.0, 0.0));
        s.set_beam_shape(None);
        assert!(s.surfaces()[1].beam.is_none(), "the lap ended");
        s.set_beam_shape(Some(Beam {
            strength: 0.0,
            ..shape
        }));
        assert!(s.surfaces()[1].beam.is_none(), "a faded beam draws nothing");
    }

    #[test]
    fn surfaces_skip_what_the_design_skips() {
        let mut s = sized();
        s.set_glass(&[
            rect(0.0, 0.0, 0.5, 100.0),
            rect(0.0, 0.0, 100.0, 0.9),
            rect(0.0, 0.0, 10.0, 10.0),
        ]);
        assert_eq!(s.surfaces().len(), 1);
    }

    #[test]
    fn a_panel_s_layer_and_opacity_reach_its_surface_and_opacity_0_draws_nothing() {
        let menu = GlassRect {
            layer: Some(OverLayer::Top),
            alpha: 0.4,
            ..rect(10.0, 10.0, 100.0, 50.0)
        };
        let g = surface_of(&menu);
        assert_eq!((g.layer, g.alpha), (Some(OverLayer::Top), 0.4f32 as f64));
        assert_eq!(surface_of(&rect(0.0, 0.0, 9.0, 9.0)).layer, None);
        // out of range opacities are held to 0..1
        assert_eq!(surface_of(&GlassRect { alpha: 3.0, ..menu }).alpha, 1.0);
        let mut s = sized();
        s.set_glass(&[GlassRect { alpha: 0.0, ..menu }, menu]);
        let drawn = s.surfaces();
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].layer, Some(OverLayer::Top));
    }

    fn still() -> Scene {
        let mut s = Scene::new(
            SceneOptions {
                backdrop: Backdrop::Still,
                ..SceneOptions::default()
            },
            1,
        );
        s.resize(800.0, 600.0, 1.0);
        s
    }

    #[test]
    fn the_still_dye_table_parses() {
        let blobs = still_blobs();
        assert!(blobs.len() >= 8, "{}", blobs.len());
        for b in &blobs {
            assert!(
                (-0.2..=1.2).contains(&b.x) && (-0.2..=1.2).contains(&b.y),
                "{b:?}"
            );
            assert!(b.color < 3 && b.radius > 0.0 && b.radius < 0.1, "{b:?}");
            assert!(b.strength > 0.0 && b.strength <= 1.0, "{b:?}");
        }
        // every colour is used
        for c in 0..3 {
            assert!(blobs.iter().any(|b| b.color == c));
        }
    }

    #[test]
    fn a_still_backdrop_pushes_no_liquid() {
        let mut s = still();
        assert!(s.ops.is_empty(), "no seeds");
        s.pointer_move(10.0, 10.0);
        s.pointer_move(100.0, 100.0);
        s.tap(500.0, 500.0);
        assert!(s.ops.is_empty(), "no ripple push");
        assert_eq!(s.ripples.len(), 1, "the ring wave still runs");
        // its dye: one dye-only splat per blob, y up, in the palette's colours
        let ops = s.still_ops();
        let blobs = still_blobs();
        assert_eq!(ops.len(), blobs.len());
        let pal = SceneOptions::default().palette;
        match ops[0] {
            FluidOp::Dye {
                x,
                y,
                color,
                radius,
            } => {
                assert_eq!(
                    (x, y, radius),
                    (blobs[0].x, 1.0 - blobs[0].y, blobs[0].radius)
                );
                assert_eq!(color, pal[blobs[0].color].map(|v| v * blobs[0].strength));
            }
            op => panic!("{op:?}"),
        }
    }

    #[test]
    fn a_still_dye_is_splatted_afresh_only_when_it_changed() {
        let mut s = still();
        // the first frame: dye and backdrop
        assert_eq!(s.take_still_work(), (true, true));
        assert_eq!(s.take_still_work(), (false, false));
        s.advance(1.0 / 60.0, 1.0);
        assert_eq!(s.take_still_work(), (false, false), "nothing moved");
        assert!(!s.animating());
        // any new size makes new grids and splats the dye afresh, a small change of shape too
        // (kept, the dye would stretch with the window)
        assert_eq!(
            s.resize(900.0, 600.0, 1.0),
            Resized {
                targets: true,
                fluid: true
            }
        );
        assert_eq!(s.take_still_work(), (true, true));
        s.resize(1800.0, 600.0, 1.0);
        assert_eq!(s.take_still_work(), (true, true));
        // another pixel ratio alone keeps it (the grids are in logical px)
        assert_eq!(
            s.resize(1800.0, 600.0, 2.0),
            Resized {
                targets: true,
                fluid: false
            }
        );
        assert_eq!(s.take_still_work(), (false, false));
        // a palette: afresh while it crossfades, then not
        s.set_palette([hex(0xFFFFFF); 3]);
        assert!(s.animating());
        let mut frames = 0;
        loop {
            s.advance(1.0 / 60.0, 1.0);
            if s.take_still_work() != (true, true) {
                break;
            }
            frames += 1;
            assert!(frames < 1000);
        }
        assert!(frames > 10, "{frames}");
        assert!(!s.animating());
    }

    #[test]
    fn ripples_keep_a_still_backdrop_composited_until_they_end() {
        let mut s = still();
        s.take_still_work();
        s.tap(400.0, 300.0);
        let mut composited = 0;
        while !s.ripples.is_empty() {
            s.advance(0.1, 1.0);
            assert_eq!(s.take_still_work(), (false, true));
            composited += 1;
        }
        // the frame the last one ended in drew it gone; nothing after
        assert!(composited >= 11, "{composited}");
        s.advance(0.1, 1.0);
        assert_eq!(s.take_still_work(), (false, false));
    }

    #[test]
    fn a_living_liquid_takes_no_still_work() {
        let mut s = sized();
        s.set_palette([hex(0xFFFFFF); 3]);
        s.tap(500.0, 500.0);
        s.advance(0.1, 1.0);
        assert_eq!(s.take_still_work(), (false, false));
        assert!(!s.dye_stale && !s.backdrop_stale);
    }

    #[test]
    fn queued_ops_are_kept_until_a_rendered_frame_and_capped() {
        let mut s = sized();
        assert_eq!(s.ops.len(), 9);
        for _ in 0..100 {
            s.tap(1.0, 1.0);
            s.advance(1.0 / 60.0, 1.0);
        }
        assert_eq!(s.ops.len(), MAX_OPS);
        // queued ops first, then 5 emitters; no pointer yet
        let ops = s.fluid_ops(1.0 / 60.0);
        assert_eq!(ops.len(), MAX_OPS + 5);
        assert!(matches!(ops[0], FluidOp::Ripple { .. }));
        assert_eq!(s.fluid_ops(1.0 / 60.0).len(), 5);
    }

    #[test]
    fn pointer_pushes_only_when_it_moves() {
        let mut s = sized();
        s.fluid_ops(0.0);
        s.pointer_move(100.0, 100.0);
        assert_eq!(s.fluid_ops(0.0).len(), 5, "first sample sets the origin");
        s.pointer_move(110.0, 100.0);
        let ops = s.fluid_ops(0.0);
        assert_eq!(ops.len(), 6);
        assert!(matches!(ops[5], FluidOp::Splat { color: None, .. }));
        s.pointer_move(130.0, 100.0);
        s.pointer_leave();
        assert_eq!(s.fluid_ops(0.0).len(), 5);
        // the light follows with a lag
        s.advance(1.0 / 60.0, 1.0);
        assert!(s.light[0] > 0.0 && s.light[0] < 130.0);
    }
}
