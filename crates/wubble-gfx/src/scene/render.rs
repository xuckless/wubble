//! The scene on wgpu: fluid backdrop, blur pyramid, glass quads.
//!
//! Source: the-wallpaper-project `src/wubble/slint/render.rs` (`Renderer`,
//! uniform mirrors, `Arena`, `Pipelines`, `fluid_op`, `fluid_step`, `blur`,
//! `frame`) and `crate/examples/headless.rs` (the readback). Owner's code.
//! Changed: built on the shared `GpuContext` device; the bubble, pop and W-field
//! passes only on the start page's own layer ([`super::start`]), no overlay mode; the backdrop work is skipped on the
//! frames the quality level says (`Quality`), and its targets shrink with it;
//! the still backdrop (`Backdrop::Still`, the browser window's).
//!
//! Per frame, back to front:
//!   0. queued fluid ops (taps, resize seeds), then the ambient emitters and
//!      the pointer push (splats); a still backdrop instead splats its dye table
//!      afresh into a cleared dye when the palette or the shape changed
//!   1. fluid step: curl → vorticity → divergence → pressure warm start (×0.8)
//!      → Jacobi iterations → gradient subtract → advect velocity → advect dye
//!      (not for a still backdrop)
//!   2. composite the backdrop at half resolution into `bg`
//!   3. blur pyramid: bg → b1 (¼ res, σ 3.5) → b2 (⅛ res, σ 4.5) → b3 (1/16 res,
//!      σ 5, the frost), σ in texels of the level read ([`BLUR_SIGMA`])
//!   4. blit bg to `out`, then in the same render pass the glass quads (the beam
//!      inside its panel's glass)
//!
//! wubble, the page in the scene ([`super::pages`]): between 3 and 4, when [`page_blur`] says
//! so, `bg` is copied into a second pyramid `P`, every pane's latest frame drawn on it as its
//! rounded card, and `P` blurred like the first. Overlay glass over a pane reads `P` instead of
//! the backdrop's levels, the glass under the page mixes a little of it in (the ambience). Overlay
//! glass is not drawn in `out` (Slint draws the page over that) but in one window-sized image per
//! [`OverLayer`], above the page: premultiplied into a scratch target, then resolved to straight
//! alpha (Skia draws a wgpu image as `AlphaType::Unpremul`).
//!
//! Steps 0-3 are the backdrop: a living liquid runs them when `BackdropClock::tick`
//! says so (every frame at `Full`/`HalfRes`), or when the pyramid is new, and steps
//! the time of the frames they skipped; a still one only when its dye or ripples
//! changed or the pyramid is new.
//!
//! Targets (logical-px based, so they don't grow with DPR):
//!   vel / pres ping-pong, div, curl   RGBA16Float  sim res 96 on the short side
//!   dye ping-pong                     RGBA16Float  dye res 256 on the short side
//!   bg                                RGBA16Float  round(w/2 · scale) × round(h/2 · scale)
//!   b1, b1t / b2, b2t / b3, b3t       RGBA16Float  ceil(bg/2) / ceil(bg/4) / ceil(bg/8)
//!   out (shown by Slint)              RGBA8Unorm   w·min(dpr, 2) × h·min(dpr, 2)
//!   P: bg and its levels, as above    RGBA16Float  only while a page is under glass or ambience
//!   over (one per layer) + scratch    RGBA8Unorm   as `out`, only while the layer has glass
//! `out` is Rgba8Unorm (not sRGB) on purpose: WebGL writes shader values
//! straight to the canvas with no linear→sRGB encode, so a non-sRGB target
//! reproduces the design's colours exactly.
//!
//! Uniforms: each draw's uniforms go into a CPU arena at aligned offsets, the
//! arena is uploaded once per frame, and each draw binds its own slice.

use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use wgpu_profiler::{GpuProfiler, GpuProfilerSettings};

use super::pages::{
    OverLayer, PageBlur, PageBlurIn, PageRect, by_layer, overlaps, page_blur, rect_of,
};
use super::quality::{BackdropClock, MAX_DT, Quality};
use super::start::{self, Globals, StartCard, StartDraw, StartFx, bubble_u, pop_u};
use super::state::{
    Backdrop, FluidOp, FluidOptions, GlassSurface, Scene, SceneOptions, wave_uniforms,
};
use crate::import::poll_once;
use crate::{GfxError, GpuContext, bytes};

// @source slint-render-shaders
pub(super) const PRELUDE: &str = concat!(
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/film.wgsl"),
    include_str!("shaders/wave.wgsl"),
    include_str!("shaders/beam.wgsl")
);
pub(super) const FLUID_WGSL: &str = include_str!("shaders/fluid.wgsl");
pub(super) const COMPOSITE_WGSL: &str = include_str!("shaders/composite.wgsl");
pub(super) const BLUR_WGSL: &str = include_str!("shaders/blur.wgsl");
pub(super) const BLIT_WGSL: &str = include_str!("shaders/blit.wgsl");
pub(super) const GLASS_WGSL: &str = include_str!("shaders/glass.wgsl");
pub(super) const PAGE_WGSL: &str = include_str!("shaders/page.wgsl");
pub(super) const BUBBLE_WGSL: &str = include_str!("shaders/bubble.wgsl");
pub(super) const POP_WGSL: &str = include_str!("shaders/pop.wgsl");

/// The blur pyramid's σ per level, in texels of the level each blur reads (scene.ts `frame`,
/// `blur(this.bg, ..., 3.5)`, `blur(this.b1, ..., 4.5)`, `blur(this.b2, ..., 5.0)`): about 7, 19 and
/// 44 CSS px in all. Below `Quality::Full` the levels are `scale` as big, so each σ is multiplied
/// by `scale` to keep the design's blur in CSS px (wubble: map.md W-01).
pub const BLUR_SIGMA: [f32; 3] = [3.5, 4.5, 5.0];
/// Each pyramid level's divisor of the composite's size (scene.ts `resize`: `hw / 2`, `/ 4`, `/ 8`).
pub const PYRAMID_DIV: [u32; 3] = [2, 4, 8];
/// Px around each glass quad for its shadow and bloom (scene.ts `drawGlass`, `const pad = 70`).
pub const GLASS_PAD: f64 = 70.0;
/// The highest device pixel ratio drawn at (scene.ts `resize`, `Math.min(devicePixelRatio, 2)`).
pub const DPR_MAX: f32 = 2.0;

// The ambience: the page's light leaking into the glass under the page next to it (the sidebar).
// Owner-requested (2026-11-19, "a slight ambience mode where the light from the webpage leaks
// into the navbar"), no web counterpart: wubble's own values, started subtle, judged by eye.
// It costs no pass: the glass mixes in the page-inclusive pyramid `P`, whose blur (σ ≈ 44 CSS px
// at its 1/16 level) is the falloff: about 40 % of the page's colour 8 px past its edge, 15 % at
// 44, 2 % at 90 (TODO.md Decided, "Overlay glass blurs the page").
/// How much of `P` the glass under the page mixes in (`uAmb`).
pub const AMBIENCE_STRENGTH: f32 = 0.20;
/// Which of `P`'s levels: 0 its ⅛ level (σ ≈ 19 CSS px, a tighter glow), 1 its 1/16 (σ ≈ 44).
pub const AMBIENCE_LEVEL: f32 = 1.0;
/// How often a changing page (scroll, video) updates the ambience, per second, when nothing
/// else needs `P`: a still page costs nothing.
pub const AMBIENCE_HZ: f64 = 8.0;

/// wubble: the glass's material over pages, handed in by the host (`wubble_core::glass`): over a
/// page the glass is the design's CSS tier (core/wubble.css), and no glass gets brighter than
/// the contrast ceiling. The default is no CSS filter and no ceiling (the scene's own tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Thin, regular, thick: `[saturate, brightness, fill r, g, b (0-1), fill alpha]`.
    pub css: [[f32; 6]; 3],
    /// The brightest relative luminance the glass may have, and where easing toward it starts.
    pub ceiling: f32,
    pub knee: f32,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            css: [[1.0, 1.0, 0.0, 0.0, 0.0, 0.0]; 3],
            ceiling: 1.0,
            knee: 1.0,
        }
    }
}

/// The ambience's update period, [`AMBIENCE_HZ`].
pub fn ambience_every() -> Duration {
    Duration::from_secs_f64(1.0 / AMBIENCE_HZ)
}

/// The scene's view of a window `width`×`height` physical px at `scale`: its logical size
/// (the window's, whatever the scale) and the pixel ratio it draws at (at most [`DPR_MAX`]).
/// A window above [`DPR_MAX`] gets a smaller texture, which Slint stretches to the window.
pub fn view_of(width: u32, height: u32, scale: f32) -> (f32, f32, f32) {
    let scale = scale.max(f32::MIN_POSITIVE);
    (
        width as f32 / scale,
        height as f32 / scale,
        scale.min(DPR_MAX),
    )
}

/// The output texture's size, physical px, for a view of `w`×`h` logical px at `dpr`.
pub fn out_size(w: f32, h: f32, dpr: f32) -> (u32, u32) {
    (
        ((w * dpr).round() as u32).max(1),
        ((h * dpr).round() as u32).max(1),
    )
}

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const OUT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// @source slint-render-uniforms
// #[repr(C)] mirrors of the WGSL uniform structs. Field order, padding and sizes match the WGSL
// layout rules (vec3 aligns to 16, arrays of vec4 stride 16). Checked against naga's layout in
// the tests below.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct FluidU {
    u_texel: [f32; 2],
    u_point: [f32; 2],
    u_value: [f32; 3],
    u_dt: f32,
    u_diss: f32,
    u_aspect: f32,
    u_radius: f32,
    u_curl_amt: f32,
    u_k: f32,
    u_ring: i32,
    _pad: [f32; 2],
} // 64 bytes

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct CompositeU {
    u_ground: [f32; 3],
    u_time: f32,
    u_view: [f32; 2],
    u_rip_n: i32,
    u_mute: f32,
    u_rip: [[f32; 4]; 8],
} // 160 bytes

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct BlurU {
    u_step: [f32; 2],
    u_sigma: f32,
    _pad: f32,
} // 16 bytes

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct GlassU {
    u_quad: [f32; 4],
    u_rect: [f32; 4],
    u_view: [f32; 2],
    u_light: [f32; 2],
    u_tint: [f32; 3],
    u_r: f32,
    u_tier: f32,
    u_film: f32,
    u_alpha: f32,
    u_shadow: f32,
    u_time: f32,
    u_dpr: f32,
    u_wave_n: i32,
    u_voice_amt: f32,
    u_material: f32,
    u_amb: f32,
    u_amb_level: f32,
    u_page: f32,
    u_beam: [f32; 4],
    u_voice: [f32; 4],
    u_wave: [[f32; 4]; 4],
    u_css: [f32; 4],
    u_css_f: [f32; 4],
} // 240 bytes

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct PageU {
    u_quad: [f32; 4],
    u_view: [f32; 2],
    u_uv: [f32; 2],
    u_r: f32,
    _p: [f32; 3],
} // 48 bytes

const _: () = {
    assert!(std::mem::size_of::<PageU>() == 48);
    assert!(std::mem::size_of::<FluidU>() == 64);
    assert!(std::mem::size_of::<CompositeU>() == 160);
    assert!(std::mem::size_of::<BlurU>() == 16);
    assert!(std::mem::size_of::<GlassU>() == 240);
};

// @source slint-render-targets
struct Target {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

fn target(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> Target {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // TEXTURE_BINDING | RENDER_ATTACHMENT is what Slint's `Image::try_from`
        // needs on `out`; COPY_SRC for readbacks (tests, screenshots) and `bg`'s copy into `P`,
        // COPY_DST for that copy
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    Target {
        tex,
        view,
        w: w.max(1),
        h: h.max(1),
    }
}

struct Double {
    read: Target,
    write: Target,
}

impl Double {
    fn new(device: &wgpu::Device, w: u32, h: u32, label: &str) -> Self {
        Double {
            read: target(device, w, h, HDR, label),
            write: target(device, w, h, HDR, label),
        }
    }
    fn swap(&mut self) {
        std::mem::swap(&mut self.read, &mut self.write);
    }
}

struct FluidTargets {
    vel: Double,
    dye: Double,
    pres: Double,
    div: Target,
    curl: Target,
    aspect: f32,
}

/// `Fluid.resize`: sim and dye grids keep the view's aspect, `res` on the short side.
fn fluid_targets(device: &wgpu::Device, w: f32, h: f32, o: &FluidOptions) -> FluidTargets {
    let aspect = w / h.max(1.0);
    let dims = |res: u32| -> (u32, u32) {
        if aspect >= 1.0 {
            ((res as f32 * aspect).round() as u32, res)
        } else {
            (res, (res as f32 / aspect).round() as u32)
        }
    };
    let (sw, sh) = dims(o.sim_res);
    let (dw, dh) = dims(o.dye_res);
    FluidTargets {
        vel: Double::new(device, sw, sh, "vel"),
        dye: Double::new(device, dw, dh, "dye"),
        pres: Double::new(device, sw, sh, "pres"),
        div: target(device, sw, sh, HDR, "div"),
        curl: target(device, sw, sh, HDR, "curl"),
        aspect,
    }
}

/// The composited backdrop and its blur pyramid, at one quality scale.
struct Pyramid {
    bg: Target,
    b1: Target,
    b1t: Target,
    b2: Target,
    b2t: Target,
    b3: Target,
    b3t: Target,
    scale: f32,
    /// Rendered at least once since allocation.
    drawn: bool,
}

impl Pyramid {
    fn new(d: &wgpu::Device, w: f32, h: f32, scale: f32) -> Self {
        let hw = ((w / 2.0 * scale).round() as u32).max(1);
        let hh = ((h / 2.0 * scale).round() as u32).max(1);
        let [q1, q2, q3] = PYRAMID_DIV.map(|n| (hw.div_ceil(n), hh.div_ceil(n)));
        Pyramid {
            bg: target(d, hw, hh, HDR, "bg"),
            b1: target(d, q1.0, q1.1, HDR, "b1"),
            b1t: target(d, q1.0, q1.1, HDR, "b1t"),
            b2: target(d, q2.0, q2.1, HDR, "b2"),
            b2t: target(d, q2.0, q2.1, HDR, "b2t"),
            b3: target(d, q3.0, q3.1, HDR, "b3"),
            b3t: target(d, q3.0, q3.1, HDR, "b3t"),
            scale,
            drawn: false,
        }
    }
}

struct Sized {
    fluid: FluidTargets,
    pyramid: Pyramid,
    /// wubble: `P`, the backdrop with the pages on it, blurred the same way; at the pyramid's
    /// scale, allocated on the first build ([`page_blur`]).
    page: Option<Pyramid>,
    out: Target,
}

/// `P` exists and has been drawn since it was allocated.
fn page_drawn(sz: Option<&Sized>) -> bool {
    sz.and_then(|s| s.page.as_ref()).is_some_and(|p| p.drawn)
}

/// One pane for the page-inclusive blur: its card and its latest frame, a texture on the scene's
/// device (the presenter's ring target, never copied).
#[derive(Clone, Debug, PartialEq)]
pub struct PagePane {
    pub rect: PageRect,
    pub texture: wgpu::Texture,
    /// The shown part of the frame, physical px from its top left (`PaneFrame.clip`).
    pub clip: (u32, u32),
}

/// What [`Renderer::before_rendering`] hands Slint: each image only when it is new.
#[derive(Default)]
pub struct Frame {
    /// The back-most image (first frame, resize).
    pub scene: Option<slint::Image>,
    /// Each [`OverLayer`]'s image by [`OverLayer::index`]: a new texture, or an empty image
    /// when the layer has no glass any more.
    pub over: [Option<slint::Image>; 3],
}

// @source slint-render-arena
/// Per-frame uniform arena: every draw gets its own aligned slice of one buffer.
struct Arena {
    buf: wgpu::Buffer,
    cpu: Vec<u8>,
    align: usize,
}

impl Arena {
    const CAP: usize = 256 * 1024;
    fn new(device: &wgpu::Device) -> Self {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniform arena"),
            size: Self::CAP as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let align = device.limits().min_uniform_buffer_offset_alignment as usize;
        Arena {
            buf,
            cpu: Vec::with_capacity(Self::CAP),
            align,
        }
    }
    /// Returns (offset, size) of the pushed slice.
    fn push<T: Copy>(&mut self, v: &T) -> (u64, u64) {
        let off = self.cpu.len().div_ceil(self.align) * self.align;
        self.cpu.resize(off, 0);
        self.cpu.extend_from_slice(bytes(v));
        assert!(
            self.cpu.len() <= Self::CAP,
            "uniform arena full: raise Arena::CAP"
        );
        (off as u64, std::mem::size_of::<T>() as u64)
    }
}

// @source slint-render-pipelines
struct Pipelines {
    advect: wgpu::RenderPipeline,
    splat: wgpu::RenderPipeline,
    curl: wgpu::RenderPipeline,
    vort: wgpu::RenderPipeline,
    div: wgpu::RenderPipeline,
    pres: wgpu::RenderPipeline,
    grad: wgpu::RenderPipeline,
    scale: wgpu::RenderPipeline,
    comp: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    page: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    /// wubble: the start page's layer (start.rs): bubbles and pops into its card, the card
    /// into the output
    bubble: wgpu::RenderPipeline,
    pop: wgpu::RenderPipeline,
    card: wgpu::RenderPipeline,
}

/// A shader body with the shared prelude, as every scene module is compiled.
pub(crate) fn with_prelude(body: &str) -> String {
    format!("{PRELUDE}\n{body}")
}

fn module(device: &wgpu::Device, label: &str, body: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(with_prelude(body).into()),
    })
}

/// `layout: None`: wgpu derives the bind group layout from what each entry point uses, so a
/// fluid pass only binds its own textures (see the binding table in fluid.wgsl).
fn pipeline(
    device: &wgpu::Device,
    m: &wgpu::ShaderModule,
    vs: &str,
    fs: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fs),
        layout: None,
        vertex: wgpu::VertexState {
            module: m,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: m,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let fluid = module(device, "fluid", FLUID_WGSL);
        let comp = module(device, "composite", COMPOSITE_WGSL);
        let blur = module(device, "blur", BLUR_WGSL);
        let blit = module(device, "blit", BLIT_WGSL);
        let glass = module(device, "glass", GLASS_WGSL);
        let page = module(device, "page", PAGE_WGSL);
        let bubble = module(device, "bubble", BUBBLE_WGSL);
        let pop = module(device, "pop", POP_WGSL);
        // premultiplied: gl.blendFunc(ONE, ONE_MINUS_SRC_ALPHA)
        let pm = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let fs = |name: &str| pipeline(device, &fluid, "vs_fullscreen", name, HDR, None);
        Pipelines {
            advect: fs("fs_advect"),
            splat: fs("fs_splat"),
            curl: fs("fs_curl"),
            vort: fs("fs_vorticity"),
            div: fs("fs_divergence"),
            pres: fs("fs_pressure"),
            grad: fs("fs_gradient"),
            scale: fs("fs_scale"),
            comp: pipeline(device, &comp, "vs_fullscreen", "fs_composite", HDR, None),
            blur: pipeline(device, &blur, "vs_fullscreen", "fs_blur", HDR, None),
            blit: pipeline(device, &blit, "vs_fullscreen", "fs_blit", OUT, None),
            glass: pipeline(device, &glass, "vs_glass", "fs_glass", OUT, pm),
            page: pipeline(device, &page, "vs_page", "fs_page", HDR, pm),
            resolve: pipeline(device, &page, "vs_fullscreen", "fs_resolve", OUT, None),
            bubble: pipeline(device, &bubble, "vs_bubble", "fs_bubble", OUT, pm),
            pop: pipeline(device, &pop, "vs_pop", "fs_pop", OUT, pm),
            card: pipeline(device, &page, "vs_card", "fs_page", OUT, pm),
        }
    }
}

/// What a draw binds, by WGSL binding number.
enum Res<'a> {
    U(u64, u64),
    Tex(u32, &'a wgpu::TextureView),
}

// @source slint-render-renderer
/// The scene renderer. Owns the GPU targets and the CPU [`Scene`], which the
/// host drives through [`Renderer::scene`].
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    p: Pipelines,
    sampler: wgpu::Sampler,
    arena: Arena,
    sized: Option<Sized>,
    /// Glass, beam, palette and pointer input: see [`Scene`].
    pub scene: Scene,
    quality: Quality,
    /// When the backdrop moves, and the time of the frames it skipped.
    clock: BackdropClock,
    last_frame: Option<Instant>,
    /// GPU timestamps around each frame's work, when the device has them
    /// (`TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_ENCODERS`).
    profiler: Option<GpuProfiler>,
    /// The GPU time of the latest frame whose timestamps came back, until taken.
    cost: Option<Duration>,
    /// Backdrops (composite and pyramid) rendered so far: a count for tests and `--measure`.
    backdrops: u64,
    /// wubble: the panes under the scene's glass, as the host last set them.
    pages: Vec<PagePane>,
    /// A pane's frame or place changed since `P` was built.
    pages_stale: bool,
    /// When `P` was last built, and how many times so far (a count for the overlay smoke).
    page_built: Option<Instant>,
    pages_built: u64,
    /// How much of `P` the glass under the page mixes in ([`AMBIENCE_STRENGTH`]; 0 is off).
    ambience: f32,
    /// The glass over pages and its contrast ceiling ([`Material`]).
    material: Material,
    /// Each overlay layer's image (straight alpha), and the premultiplied target it is drawn in.
    over: [Option<Target>; 3],
    scratch: Option<Target>,
    /// Layers whose image is new or gone since the host last took them.
    over_new: [bool; 3],
    /// wubble: the start page's living layer while a start tab shows ([`super::start`]).
    start: Option<StartLayer>,
    /// The Space's dye for the start page's liquid ([`Renderer::set_start_palette`]).
    start_palette: [[f32; 3]; 3],
    /// The W distance field (`uW`), once the host hands it over ([`Renderer::set_wfield`]).
    w_tex: Option<Target>,
}

/// wubble: the start page's living layer: its own liquid (a second [`Scene`], `Liquid`) and
/// clock, the frame to draw, and its GPU targets at the page card's size.
struct StartLayer {
    scene: Scene,
    clock: BackdropClock,
    draw: StartDraw,
    sized: Option<StartSized>,
}

/// The layer's targets: its fluid grids and pyramid at the card's logical size, and the card's
/// image (`Rgba8Unorm`, top-first, physical px at the scene's pixel ratio).
struct StartSized {
    fluid: FluidTargets,
    pyramid: Pyramid,
    card: Target,
}

/// The start page's liquid is seeded the same on every start page.
const START_SEED: u64 = 0x057a_77ed;

impl Renderer {
    /// The scene on the one device that Slint and the frame importer share.
    /// `Err` when the device refuses the scene's shaders or pipelines (a
    /// validation error, captured instead of panicking), so the host can fall
    /// back to Slint-only glass. The formats need no check: `Rgba16Float` and
    /// `Rgba8Unorm` are renderable, blendable and filterable on every WebGPU
    /// device.
    pub fn try_new(gpu: &GpuContext, opts: SceneOptions) -> Result<Self, GfxError> {
        Self::from_device(gpu.device().clone(), gpu.queue().clone(), opts)
    }

    /// The scene on a given device. Only for headless runs (tests, the
    /// `headless` example): the app uses [`Renderer::try_new`], so the scene's
    /// textures live on the device Slint renders with.
    pub fn from_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        opts: SceneOptions,
    ) -> Result<Self, GfxError> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let p = Pipelines::new(&device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let arena = Arena::new(&device);
        // native wgpu resolves an error scope at once
        match poll_once(scope.pop()) {
            Some(None) => {}
            Some(Some(e)) => return Err(GfxError::Device(format!("scene pipelines: {e}"))),
            None => return Err(GfxError::Device("scene error scope did not resolve".into())),
        }
        let timers =
            GpuProfiler::ALL_WGPU_TIMER_FEATURES - wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        let profiler = device
            .features()
            .contains(timers)
            .then(|| {
                GpuProfiler::new(
                    &device,
                    GpuProfilerSettings {
                        enable_timer_queries: true,
                        enable_debug_groups: false,
                        max_num_pending_frames: 3,
                    },
                )
                .ok()
            })
            .flatten();
        Ok(Renderer {
            device,
            queue,
            p,
            sampler,
            arena,
            sized: None,
            scene: Scene::new(opts, 0x5eed),
            quality: Quality::Full,
            clock: BackdropClock::new(),
            last_frame: None,
            profiler,
            cost: None,
            backdrops: 0,
            pages: vec![],
            pages_stale: false,
            page_built: None,
            pages_built: 0,
            ambience: AMBIENCE_STRENGTH,
            material: Material::default(),
            over: [None, None, None],
            scratch: None,
            over_new: [false; 3],
            start: None,
            start_palette: SceneOptions::default().palette,
            w_tex: None,
        })
    }

    /// How many backdrops (composite and blur pyramid) have been rendered: a still backdrop
    /// renders one only when its dye, its ripples or its size changed.
    pub fn backdrops(&self) -> u64 {
        self.backdrops
    }

    /// How many times the page-inclusive blur `P` has been built ([`page_blur`]).
    pub fn pages_built(&self) -> u64 {
        self.pages_built
    }

    /// The ambience's strength: [`AMBIENCE_STRENGTH`], 0 for none (the overlay smoke's control).
    pub fn set_ambience(&mut self, strength: f32) {
        self.ambience = strength.max(0.0);
    }

    /// The glass's material over pages and its contrast ceiling ([`Material`]).
    pub fn set_material(&mut self, material: Material) {
        self.material = material;
    }

    /// The panes under the glass, left to right: each card and its latest frame. A changed
    /// frame or place makes `P` stale; it is rebuilt when [`page_blur`] says so. No panes (an
    /// internal page, no frame yet) drop `P`.
    pub fn set_pages(&mut self, pages: Vec<PagePane>) {
        if pages == self.pages {
            return;
        }
        if pages.is_empty()
            && let Some(sz) = &mut self.sized
        {
            sz.page = None;
        }
        self.pages = pages;
        self.pages_stale = true;
    }

    /// A pane's texture was written in place (its page's damage): the same textures, new
    /// pixels, so `P` is out of date as after [`Renderer::set_pages`] with new ones.
    pub fn pages_written(&mut self) {
        if !self.pages.is_empty() {
            self.pages_stale = true;
        }
    }

    /// Whether to build `P` on the next frame, from the glass and panes as they are now and no
    /// new backdrop: the host's `scene_due` input (`Build`) and its wake-up (`After`).
    pub fn page_verdict(&self, now: Instant) -> PageBlur {
        let built = page_drawn(self.sized.as_ref());
        page_blur(&self.page_in(&self.scene.surfaces(), now, false, built))
    }

    fn page_rects(&self) -> Vec<PageRect> {
        self.pages.iter().map(|p| p.rect).collect()
    }

    /// The rule's inputs; `built`: `P` exists and is drawn ([`page_drawn`]).
    fn page_in(
        &self,
        surfaces: &[GlassSurface],
        now: Instant,
        backdrop_new: bool,
        built: bool,
    ) -> PageBlurIn {
        let rects = self.page_rects();
        PageBlurIn {
            pages: !self.pages.is_empty(),
            overlay: surfaces
                .iter()
                .any(|s| s.layer.is_some() && overlaps(&rect_of(s), &rects)),
            ambience: self.ambience > 0.0 && surfaces.iter().any(|s| s.layer.is_none()),
            stale: self.pages_stale,
            backdrop_new,
            built: built && !self.pages.is_empty(),
            flat: self.quality == Quality::Flat,
            since_build: self
                .page_built
                .map_or(Duration::MAX, |t| now.saturating_duration_since(t)),
            ambience_every: ambience_every(),
        }
    }

    /// The overlay layers' images that are new or gone since the last call: by
    /// [`OverLayer::index`], `Some(Some(texture))` new, `Some(None)` gone, `None` unchanged.
    pub fn take_over_changes(&mut self) -> [Option<Option<wgpu::Texture>>; 3] {
        let mut out = [None, None, None];
        for l in OverLayer::ALL {
            let i = l.index();
            if std::mem::take(&mut self.over_new[i]) {
                out[i] = Some(self.over[i].as_ref().map(|t| t.tex.clone()));
            }
        }
        out
    }

    /// An overlay layer's image as last drawn, for checks.
    pub fn over_texture(&self, layer: OverLayer) -> Option<&wgpu::Texture> {
        self.over[layer.index()].as_ref().map(|t| &t.tex)
    }

    /// Whether frame costs are measured: the device has GPU timestamps.
    pub fn measures_cost(&self) -> bool {
        self.profiler.is_some()
    }

    /// The device's timestamp features, for the log line that says why there are no costs.
    pub fn timestamp_features(&self) -> String {
        let f = self.device.features();
        format!(
            "TIMESTAMP_QUERY={} INSIDE_ENCODERS={} INSIDE_PASSES={}",
            f.contains(wgpu::Features::TIMESTAMP_QUERY),
            f.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
            f.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
        )
    }

    /// The GPU time of the scene's work in the latest frame whose
    /// timestamps have come back (one to three frames late), once: `None`
    /// until another frame's come back, and always when the device has no
    /// timestamp queries. This is the cost the host feeds to the `Governor`.
    pub fn take_cost(&mut self) -> Option<Duration> {
        self.cost.take()
    }

    /// When [`Renderer::before_rendering`] last ran (the host's frame interval).
    pub fn last_frame(&self) -> Option<Instant> {
        self.last_frame
    }

    /// The level the host's `Governor` chose. Takes effect on the next frame.
    pub fn set_quality(&mut self, quality: Quality) {
        self.quality = quality;
    }

    /// The output texture read back as RGBA8 (width, height, pixels), for checks: it waits for
    /// the GPU, so never on the frame path.
    pub fn readback(&self) -> Option<(u32, u32, Vec<u8>)> {
        let t = self.output()?;
        Some((
            t.width(),
            t.height(),
            readback_rgba8(&self.device, &self.queue, t),
        ))
    }

    /// Whether the scene moves on its own beyond the ambient drift ([`Scene::animating`]); a
    /// shown start page always does (its liquid and bubble live).
    pub fn animating(&self) -> bool {
        self.scene.animating() || self.start.is_some()
    }

    /// The texture Slint shows, once the first frame has a size.
    pub fn output(&self) -> Option<&wgpu::Texture> {
        self.sized.as_ref().map(|s| &s.out.tex)
    }

    /// Call from the window's rendering notifier on `BeforeRendering`, before
    /// Slint draws: sizes the scene to the window (at most [`DPR_MAX`]), advances it by the
    /// time since the last call (capped at `MAX_DT`) and renders it. `pace` is 1, or 0 to hold
    /// the scene's clock (`--frozen-backdrop`, the fidelity harness). wubble change: it no longer asks for the next frame; the host
    /// does, by the run state (`run.rs`, [`Renderer::animating`]). Returns the image to show at
    /// the back of the window when the output texture is new (first frame, resize), and each
    /// overlay layer's when it is new or gone ([`Frame`]); `None` means the image already set is
    /// still the one, as in the design.
    /// Setting a property during rendering asks Slint for another frame, so an image set every
    /// frame kept the window rendering at the display rate even while the run state was Idle.
    /// (An earlier wubble change set it every frame: femtovg cached the window's rounded clip
    /// as a layer and showed a frozen scene. Slint's Skia renderer draws the whole window on
    /// every render while a rendering notifier is set (`partial_rendering_state` is `None`
    /// then) and caches no layer for a clip, so the texture drawn in place shows; `--smoke
    /// frames` still checks the window shows the scene's current frame.)
    pub fn before_rendering(
        &mut self,
        window: &slint::Window,
        now: Instant,
        pace: f32,
    ) -> Result<Frame, GfxError> {
        let size = window.size();
        let (w, h, dpr) = view_of(size.width, size.height, window.scale_factor());
        let dt = self
            .last_frame
            .replace(now)
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        let tex = self.render_paced(w, h, dpr, dt.min(MAX_DT), pace);
        let image = |t: wgpu::Texture| {
            slint::Image::try_from(t).map_err(|e| GfxError::Image(e.to_string()))
        };
        let mut frame = Frame {
            scene: tex.map(image).transpose()?,
            ..Frame::default()
        };
        for (slot, change) in frame.over.iter_mut().zip(self.take_over_changes()) {
            *slot = match change {
                Some(Some(t)) => Some(image(t)?),
                Some(None) => Some(slint::Image::default()),
                None => None,
            };
        }
        Ok(frame)
    }

    /// Size the scene to a `w`×`h` logical px view at `dpr`, advance it by
    /// `dt` seconds and render it into the output texture. Returns the output
    /// texture when it is new (hand it to Slint with `slint::Image::try_from`).
    pub fn render_under_ui(&mut self, w: f32, h: f32, dpr: f32, dt: f32) -> Option<wgpu::Texture> {
        self.render_paced(w, h, dpr, dt, 1.0)
    }

    /// [`Renderer::render_under_ui`] with the scene's clock running at `pace` (0 to 1) of its
    /// speed: its clock, its ambient emitters and its fluid step get `dt * pace`, while the rest
    /// (glass waves, ripples, the palette crossfade, the light) gets the whole `dt`. The fidelity
    /// harness holds it at 0.
    pub fn render_paced(
        &mut self,
        w: f32,
        h: f32,
        dpr: f32,
        dt: f32,
        pace: f32,
    ) -> Option<wgpu::Texture> {
        let tex = self.resize(w, h, dpr);
        let now = self.last_frame.unwrap_or_else(Instant::now);
        self.frame(dt, pace.clamp(0.0, 1.0), now);
        tex
    }

    /// (Re)allocate for a view of w×h logical px at `dpr` and the current
    /// quality. Returns the new output texture when it changed.
    fn resize(&mut self, w: f32, h: f32, dpr: f32) -> Option<wgpu::Texture> {
        let rs = self.scene.resize(w, h, dpr);
        let scale = self.quality.scale();
        let d = &self.device;
        if !rs.targets
            && let Some(sz) = &mut self.sized
        {
            if sz.pyramid.scale != scale {
                sz.pyramid = Pyramid::new(d, w, h, scale);
                sz.page = None;
            }
            return None;
        }
        // keep the liquid unless the scene asked for new grids
        let fluid = match self.sized.take() {
            Some(s) if !rs.fluid => s.fluid,
            _ => fluid_targets(d, w, h, &self.scene.opts.fluid),
        };
        let (ow, oh) = out_size(w, h, dpr);
        let out = target(d, ow, oh, OUT, "wubble scene");
        let tex = out.tex.clone();
        self.sized = Some(Sized {
            fluid,
            pyramid: Pyramid::new(d, w, h, scale),
            page: None,
            out,
        });
        Some(tex)
    }

    fn bind(&self, pl: &wgpu::RenderPipeline, res: &[Res]) -> wgpu::BindGroup {
        // every pass that reads a texture reads it through `samp` (binding 1); an auto layout
        // only contains what the shader uses, so the sampler is left out when nothing samples
        let mut entries = vec![];
        if res.iter().any(|r| matches!(r, Res::Tex(..))) {
            entries.push(wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            });
        }
        for r in res {
            entries.push(match r {
                Res::U(off, size) => wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.arena.buf,
                        offset: *off,
                        size: NonZeroU64::new(*size),
                    }),
                },
                Res::Tex(b, v) => wgpu::BindGroupEntry {
                    binding: *b,
                    resource: wgpu::BindingResource::TextureView(v),
                },
            });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pl.get_bind_group_layout(0),
            entries: &entries,
        })
    }

    /// One full-screen pass into `out` (the GL `run(program, target)`).
    fn run(
        &self,
        enc: &mut wgpu::CommandEncoder,
        pl: &wgpu::RenderPipeline,
        bg: &wgpu::BindGroup,
        out: &wgpu::TextureView,
    ) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pl);
        rp.set_bind_group(0, bg, &[]);
        rp.draw(0..4, 0..1);
    }

    // @source slint-render-frame
    /// Advance the scene by `dt` seconds and render into `out` (and the overlay layers' images).
    fn frame(&mut self, dt: f32, pace: f32, now: Instant) {
        let Some(mut sz) = self.sized.take() else {
            return;
        };
        if self.scene.w == 0.0 {
            self.sized = Some(sz);
            return;
        }
        self.arena.cpu.clear();
        self.scene.advance(dt, pace);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wubble scene"),
            });
        let timer = self
            .profiler
            .as_ref()
            .map(|p| p.begin_query("wubble scene", &mut enc));
        let (dye, still) = self.scene.take_still_work();
        let backdrop = match self.scene.opts.backdrop {
            Backdrop::Liquid => self.clock.tick(self.quality, dt * pace, !sz.pyramid.drawn),
            Backdrop::Still => (still || !sz.pyramid.drawn).then_some(0.0),
        };
        if let Some(dt) = backdrop {
            let f = &mut sz.fluid;
            if self.scene.opts.backdrop == Backdrop::Liquid {
                // 0. splats (queued taps first, then emitters and the pointer: the TS order)
                for op in self.scene.fluid_ops(dt) {
                    self.fluid_op(&mut enc, f, op);
                }
                // 1. the fluid step
                self.fluid_step(&mut enc, f, dt);
            } else if dye {
                // 0. the still dye, afresh in the current palette
                self.clear(&mut enc, &f.dye.read.view);
                for op in self.scene.still_ops() {
                    self.fluid_op(&mut enc, f, op);
                }
            }

            // 2. composite the backdrop at half resolution, 3. its blur pyramid
            let cu = composite_u(&self.scene);
            self.backdrop_passes(&mut enc, &cu, &f.dye.read.view, &mut sz.pyramid);
            self.backdrops += 1;
        }

        // wubble: the page-inclusive pyramid `P`, when the page blur's rule says so
        let surfaces = self.scene.surfaces();
        let built = page_drawn(Some(&sz));
        if page_blur(&self.page_in(&surfaces, now, backdrop.is_some(), built)) == PageBlur::Build {
            self.build_page(&mut enc, &mut sz);
            self.page_built = Some(now);
        }

        // wubble: the start page's living layer, into its card's image (start.rs)
        let card = self.start_passes(&mut enc, dt, pace);

        // 4. one pass on the output: backdrop, then the glass under the page (the beam inside
        // its panel's); then each overlay layer's glass in its own image
        let py = &sz.pyramid;
        let page = sz
            .page
            .as_ref()
            .filter(|p| p.drawn && !self.pages.is_empty());
        let rects = self.page_rects();
        let blit = self.bind(&self.p.blit, &[Res::Tex(2, &py.bg.view)]);
        let mut glass = Vec::with_capacity(surfaces.len());
        for s in &surfaces {
            // overlay glass over a pane frosts the page; the glass under the page mixes it in
            let over_page = s.layer.is_some() && overlaps(&rect_of(s), &rects);
            let levels = match page {
                Some(pg) if over_page => pg,
                _ => py,
            };
            let ambient = page.filter(|_| s.layer.is_none());
            let mut gu = self.glass_u(s);
            if over_page && page.is_some() {
                gu.u_page = 1.0;
            }
            if ambient.is_some() {
                gu.u_amb = self.ambience;
                gu.u_amb_level = AMBIENCE_LEVEL;
            }
            let amb = ambient.unwrap_or(levels);
            let u = self.arena.push(&gu);
            glass.push(self.bind(
                &self.p.glass,
                &[
                    Res::U(u.0, u.1),
                    Res::Tex(3, &levels.b2.view),
                    Res::Tex(4, &levels.b3.view),
                    Res::Tex(5, &amb.b2.view),
                    Res::Tex(6, &amb.b3.view),
                ],
            ));
        }
        let layers = by_layer(&surfaces);
        {
            let mut rp = self.pass(&mut enc, "wubble scene out", &sz.out.view);
            rp.set_pipeline(&self.p.blit);
            rp.set_bind_group(0, &blit, &[]);
            rp.draw(0..4, 0..1);
            rp.set_pipeline(&self.p.glass);
            for &i in &layers[0] {
                rp.set_bind_group(0, &glass[i], &[]);
                rp.draw(0..4, 0..1);
            }
            // the start page's card over it: the living liquid only inside the page card
            if let Some(b) = &card {
                rp.set_pipeline(&self.p.card);
                rp.set_bind_group(0, b, &[]);
                rp.draw(0..4, 0..1);
            }
        }
        self.draw_over(&mut enc, &sz.out, &layers, &glass);

        if let (Some(p), Some(q)) = (self.profiler.as_mut(), timer) {
            p.end_query(&mut enc, q);
            p.resolve_queries(&mut enc);
        }
        // all uniforms of this frame, then the commands that read them
        self.queue.write_buffer(&self.arena.buf, 0, &self.arena.cpu);
        self.queue.submit([enc.finish()]);
        self.sized = Some(sz);
        self.collect_cost();
    }

    /// A render pass on `view` that clears it to transparent.
    fn pass<'e>(
        &self,
        enc: &'e mut wgpu::CommandEncoder,
        label: &str,
        view: &wgpu::TextureView,
    ) -> wgpu::RenderPass<'e> {
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }

    /// wubble: builds `P`: the backdrop's `bg` copied, every pane's latest frame drawn on it as
    /// its rounded card, then blurred as the backdrop's pyramid is. Half resolution, no readback.
    fn build_page(&mut self, enc: &mut wgpu::CommandEncoder, sz: &mut Sized) {
        let py = &sz.pyramid;
        let mut page = match sz.page.take() {
            Some(p) => p,
            None => Pyramid::new(&self.device, self.scene.w, self.scene.h, py.scale),
        };
        enc.copy_texture_to_texture(
            py.bg.tex.as_image_copy(),
            page.bg.tex.as_image_copy(),
            py.bg.tex.size(),
        );
        // moved out while `self` binds, put back below
        let pages = std::mem::take(&mut self.pages);
        let binds: Vec<wgpu::BindGroup> = pages
            .iter()
            .map(|p| {
                let (tw, th) = (p.texture.width().max(1), p.texture.height().max(1));
                let u = self.arena.push(&PageU {
                    u_quad: [p.rect.rect.x, p.rect.rect.y, p.rect.rect.w, p.rect.rect.h],
                    u_view: [self.scene.w, self.scene.h],
                    u_uv: [
                        p.clip.0.min(tw) as f32 / tw as f32,
                        p.clip.1.min(th) as f32 / th as f32,
                    ],
                    u_r: p.rect.radius,
                    _p: [0.0; 3],
                });
                let view = p
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                self.bind(&self.p.page, &[Res::U(u.0, u.1), Res::Tex(2, &view)])
            })
            .collect();
        self.pages = pages;
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wubble scene pages"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &page.bg.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.p.page);
            for b in &binds {
                rp.set_bind_group(0, b, &[]);
                rp.draw(0..4, 0..1);
            }
        }
        let [s1, s2, s3] = BLUR_SIGMA.map(|s| s * page.scale);
        self.blur(enc, &page.bg, &page.b1t, &page.b1, s1);
        self.blur(enc, &page.b1, &page.b2t, &page.b2, s2);
        self.blur(enc, &page.b2, &page.b3t, &page.b3, s3);
        page.drawn = true;
        sz.page = Some(page);
        self.pages_stale = false;
        self.pages_built += 1;
    }

    /// wubble: each overlay layer's glass into its own image, the size of `out`: premultiplied
    /// into the scratch target, then resolved to straight alpha. A layer without glass drops its
    /// image (the host clears Slint's); the scratch goes with the last one.
    fn draw_over(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        out: &Target,
        layers: &[Vec<usize>; 4],
        glass: &[wgpu::BindGroup],
    ) {
        for l in OverLayer::ALL {
            let i = l.index();
            let items = &layers[i + 1];
            if items.is_empty() {
                if self.over[i].take().is_some() {
                    self.over_new[i] = true;
                }
                continue;
            }
            if self.over[i]
                .as_ref()
                .is_none_or(|t| (t.w, t.h) != (out.w, out.h))
            {
                self.over[i] = Some(target(&self.device, out.w, out.h, OUT, "wubble over"));
                self.over_new[i] = true;
            }
            if self
                .scratch
                .as_ref()
                .is_none_or(|t| (t.w, t.h) != (out.w, out.h))
            {
                self.scratch = Some(target(&self.device, out.w, out.h, OUT, "wubble over pm"));
            }
            let (Some(scratch), Some(shown)) = (&self.scratch, &self.over[i]) else {
                continue;
            };
            {
                let mut rp = self.pass(enc, "wubble over glass", &scratch.view);
                rp.set_pipeline(&self.p.glass);
                for &g in items {
                    rp.set_bind_group(0, &glass[g], &[]);
                    rp.draw(0..4, 0..1);
                }
            }
            let b = self.bind(&self.p.resolve, &[Res::Tex(2, &scratch.view)]);
            self.run(enc, &self.p.resolve, &b, &shown.view);
        }
        if self.over.iter().all(Option::is_none) {
            self.scratch = None;
        }
    }

    /// Closes the profiler frame and picks up any finished one. The poll does
    /// not block: it only runs the map callbacks of work already done.
    fn collect_cost(&mut self) {
        let Some(p) = self.profiler.as_mut() else {
            return;
        };
        if p.end_frame().is_err() {
            return;
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
        while let Some(results) = p.process_finished_frame(self.queue.get_timestamp_period()) {
            if let Some(t) = results.first().and_then(|r| r.time.clone()) {
                self.cost = Some(Duration::from_secs_f64((t.end - t.start).max(0.0)));
            }
        }
    }

    /// The glass uniforms of one surface. The voice glow is the deferred start
    /// page's: its fields stay zero. Under `Quality::Flat` full glass is sent
    /// as `uMaterial` 2, the flat fill (the one change to glass.wgsl).
    fn glass_u(&self, s: &GlassSurface) -> GlassU {
        let pad = GLASS_PAD;
        let (u_wave, u_wave_n) = wave_uniforms(&s.waves);
        let m = &self.material;
        let t = m.css[(s.tier.round() as usize).min(2)];
        GlassU {
            u_quad: [
                (s.x - pad) as f32,
                (s.y - pad) as f32,
                (s.w + pad * 2.0) as f32,
                (s.h + pad * 2.0) as f32,
            ],
            u_rect: [s.x as f32, s.y as f32, s.w as f32, s.h as f32],
            u_view: [self.scene.w, self.scene.h],
            u_light: self.scene.light,
            u_tint: s.tint,
            u_r: s.r as f32,
            u_tier: s.tier as f32,
            u_film: s.film as f32,
            u_alpha: s.alpha as f32,
            u_shadow: s.shadow as f32,
            u_time: self.scene.time,
            u_dpr: self.scene.dpr,
            u_wave_n,
            u_voice_amt: 0.0,
            // Flat draws full glass as a flat fill (glass.wgsl `uMaterial` 2)
            u_material: if self.quality == Quality::Flat && s.material > 0.5 {
                2.0
            } else {
                s.material as f32
            },
            u_amb: 0.0,
            u_amb_level: 0.0,
            u_page: 0.0,
            u_beam: s
                .beam
                .map(|b| {
                    [
                        b.head as f32,
                        b.length as f32,
                        b.strength as f32,
                        b.hue as f32,
                    ]
                })
                .unwrap_or([0.0, 0.25, 0.0, 0.0]),
            u_voice: [0.0; 4],
            u_wave,
            u_css: [t[2], t[3], t[4], t[5]],
            u_css_f: [t[0], t[1], m.ceiling, m.knee],
        }
    }

    fn fluid_u(&self, f: &FluidTargets) -> FluidU {
        FluidU {
            u_texel: [1.0 / f.vel.read.w as f32, 1.0 / f.vel.read.h as f32],
            u_aspect: f.aspect,
            ..Default::default()
        }
    }

    /// Clears a target to transparent black (a still dye before its splats).
    fn clear(&self, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView) {
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// The dye half of `Fluid.splat`: `value` added to the dye around (x, y).
    fn dye_splat(&mut self, enc: &mut wgpu::CommandEncoder, f: &mut FluidTargets, u: FluidU) {
        let s = self.arena.push(&u);
        let b = self.bind(
            &self.p.splat,
            &[Res::U(s.0, s.1), Res::Tex(3, &f.dye.read.view)],
        );
        self.run(enc, &self.p.splat, &b, &f.dye.write.view);
        f.dye.swap();
    }

    // @source slint-render-fluid-ops
    /// `Fluid.splat` and `Fluid.ripple`; wubble: `FluidOp::Dye`, a splat of dye only.
    fn fluid_op(&mut self, enc: &mut wgpu::CommandEncoder, f: &mut FluidTargets, op: FluidOp) {
        match op {
            FluidOp::Dye {
                x,
                y,
                color,
                radius,
            } => {
                let u = FluidU {
                    u_point: [x, y],
                    u_radius: radius,
                    u_value: color,
                    ..self.fluid_u(f)
                };
                self.dye_splat(enc, f, u);
            }
            FluidOp::Splat {
                x,
                y,
                dx,
                dy,
                color,
                radius,
            } => {
                let mut u = FluidU {
                    u_point: [x, y],
                    u_radius: radius,
                    u_ring: 0,
                    u_value: [dx, dy, 0.0],
                    ..self.fluid_u(f)
                };
                let s = self.arena.push(&u);
                let b = self.bind(
                    &self.p.splat,
                    &[Res::U(s.0, s.1), Res::Tex(3, &f.vel.read.view)],
                );
                self.run(enc, &self.p.splat, &b, &f.vel.write.view);
                f.vel.swap();
                if let Some(c) = color {
                    u.u_value = c;
                    self.dye_splat(enc, f, u);
                }
            }
            FluidOp::Ripple {
                x,
                y,
                strength,
                radius,
            } => {
                let u = FluidU {
                    u_point: [x, y],
                    u_radius: radius,
                    u_ring: 1,
                    u_value: [strength, 0.0, 0.0],
                    ..self.fluid_u(f)
                };
                let s = self.arena.push(&u);
                let b = self.bind(
                    &self.p.splat,
                    &[Res::U(s.0, s.1), Res::Tex(3, &f.vel.read.view)],
                );
                self.run(enc, &self.p.splat, &b, &f.vel.write.view);
                f.vel.swap();
            }
        }
    }

    // @source slint-render-fluid-step
    /// `Fluid.step`: same passes, same order, same constants as fluid.ts; the
    /// Jacobi iteration count follows the quality level.
    fn fluid_step(&mut self, enc: &mut wgpu::CommandEncoder, f: &mut FluidTargets, dt: f32) {
        let o = self.scene.opts.fluid;
        let base = self.fluid_u(f);

        let u = self.arena.push(&base);
        let b = self.bind(
            &self.p.curl,
            &[Res::U(u.0, u.1), Res::Tex(2, &f.vel.read.view)],
        );
        self.run(enc, &self.p.curl, &b, &f.curl.view);

        let u = self.arena.push(&FluidU {
            u_curl_amt: o.curl,
            u_dt: dt,
            ..base
        });
        let b = self.bind(
            &self.p.vort,
            &[
                Res::U(u.0, u.1),
                Res::Tex(2, &f.vel.read.view),
                Res::Tex(4, &f.curl.view),
            ],
        );
        self.run(enc, &self.p.vort, &b, &f.vel.write.view);
        f.vel.swap();

        let u = self.arena.push(&base);
        let b = self.bind(
            &self.p.div,
            &[Res::U(u.0, u.1), Res::Tex(2, &f.vel.read.view)],
        );
        self.run(enc, &self.p.div, &b, &f.div.view);

        // warm start: keep 80% of last frame's pressure
        let u = self.arena.push(&FluidU { u_k: 0.8, ..base });
        let b = self.bind(
            &self.p.scale,
            &[Res::U(u.0, u.1), Res::Tex(3, &f.pres.read.view)],
        );
        self.run(enc, &self.p.scale, &b, &f.pres.write.view);
        f.pres.swap();

        let u = self.arena.push(&base);
        for _ in 0..self.quality.pressure_iters(o.pressure_iters) {
            let b = self.bind(
                &self.p.pres,
                &[
                    Res::U(u.0, u.1),
                    Res::Tex(5, &f.pres.read.view),
                    Res::Tex(6, &f.div.view),
                ],
            );
            self.run(enc, &self.p.pres, &b, &f.pres.write.view);
            f.pres.swap();
        }

        let b = self.bind(
            &self.p.grad,
            &[
                Res::U(u.0, u.1),
                Res::Tex(5, &f.pres.read.view),
                Res::Tex(2, &f.vel.read.view),
            ],
        );
        self.run(enc, &self.p.grad, &b, &f.vel.write.view);
        f.vel.swap();

        let u = self.arena.push(&FluidU {
            u_dt: dt,
            u_diss: o.vel_diss,
            ..base
        });
        let b = self.bind(
            &self.p.advect,
            &[
                Res::U(u.0, u.1),
                Res::Tex(2, &f.vel.read.view),
                Res::Tex(3, &f.vel.read.view),
            ],
        );
        self.run(enc, &self.p.advect, &b, &f.vel.write.view);
        f.vel.swap();

        // dye advects with the velocity's texel size, as in the TS
        let u = self.arena.push(&FluidU {
            u_dt: dt,
            u_diss: o.dye_diss,
            ..base
        });
        let b = self.bind(
            &self.p.advect,
            &[
                Res::U(u.0, u.1),
                Res::Tex(2, &f.vel.read.view),
                Res::Tex(3, &f.dye.read.view),
            ],
        );
        self.run(enc, &self.p.advect, &b, &f.dye.write.view);
        f.dye.swap();
    }

    fn blur(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        src: &Target,
        tmp: &Target,
        out: &Target,
        sigma: f32,
    ) {
        let u = self.arena.push(&BlurU {
            u_step: [1.0 / src.w as f32, 0.0],
            u_sigma: sigma,
            _pad: 0.0,
        });
        let b = self.bind(&self.p.blur, &[Res::U(u.0, u.1), Res::Tex(2, &src.view)]);
        self.run(enc, &self.p.blur, &b, &tmp.view);
        let u = self.arena.push(&BlurU {
            u_step: [0.0, 1.0 / tmp.h as f32],
            u_sigma: sigma,
            _pad: 0.0,
        });
        let b = self.bind(&self.p.blur, &[Res::U(u.0, u.1), Res::Tex(2, &tmp.view)]);
        self.run(enc, &self.p.blur, &b, &out.view);
    }
}

/// Step 2's uniforms for a scene: its ground, time, size, mute and running ripples.
fn composite_u(scene: &Scene) -> CompositeU {
    let mut cu = CompositeU {
        u_ground: scene.ground,
        u_time: scene.time,
        u_view: [scene.w, scene.h],
        u_mute: scene.opts.intensity,
        ..Default::default()
    };
    cu.u_rip_n = scene.ripples.len() as i32;
    for (i, r) in scene.ripples.iter().enumerate() {
        cu.u_rip[i] = [r.x, r.y, r.age / r.life, r.reach];
    }
    cu
}

impl Renderer {
    /// Steps 2 and 3 for one scene: the dye composited at half resolution into the pyramid's
    /// `bg`, then its blur levels. wubble: shared by the window's backdrop and the start page's.
    fn backdrop_passes(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        cu: &CompositeU,
        dye: &wgpu::TextureView,
        py: &mut Pyramid,
    ) {
        let u = self.arena.push(cu);
        let b = self.bind(&self.p.comp, &[Res::U(u.0, u.1), Res::Tex(2, dye)]);
        self.run(enc, &self.p.comp, &b, &py.bg.view);

        // 3. blur pyramid for the glass: σ in texels, scaled so its CSS px stay the design's
        let [s1, s2, s3] = BLUR_SIGMA.map(|s| s * py.scale);
        self.blur(enc, &py.bg, &py.b1t, &py.b1, s1);
        self.blur(enc, &py.b1, &py.b2t, &py.b2, s2);
        self.blur(enc, &py.b2, &py.b3t, &py.b3, s3); // the frost: ≈44 CSS px σ in all
        py.drawn = true;
    }

    // ----- wubble: the start page's living layer ([`super::start`])

    /// The start page's frame: where its card is and what it draws. The first call makes the
    /// layer (its liquid in the start palette); every frame after it, the layer's liquid steps
    /// and the card is drawn over the window's backdrop and glass until [`Renderer::start_end`].
    pub fn start_frame(&mut self, draw: StartDraw) {
        let palette = self.start_palette;
        let layer = self.start.get_or_insert_with(|| StartLayer {
            scene: Scene::new(
                SceneOptions {
                    palette,
                    ..SceneOptions::default()
                },
                START_SEED,
            ),
            clock: BackdropClock::new(),
            draw: StartDraw::default(),
            sized: None,
        });
        layer.draw = draw;
    }

    /// No start page is shown: the layer and its targets go, and nothing of it runs.
    pub fn start_end(&mut self) {
        self.start = None;
    }

    /// Whether the start page's layer is up.
    pub fn start_shown(&self) -> bool {
        self.start.is_some()
    }

    /// What the start page did to its liquid (page-local px). Nothing without a layer.
    pub fn start_effect(&mut self, fx: StartFx) {
        if let Some(l) = &mut self.start {
            start::apply(&mut l.scene, fx);
        }
    }

    /// The Space's three dye colours for the start page's liquid (the window's own crossfade:
    /// about 600 ms); kept for the next layer.
    pub fn set_start_palette(&mut self, colors: [[f32; 3]; 3]) {
        self.start_palette = colors;
        if let Some(l) = &mut self.start {
            l.scene.set_palette(colors);
        }
    }

    /// The W distance field, `size`² RGBA8 rows as they are (`wubble_core::start::wfield`):
    /// uploaded once. Until it is here the start page draws its liquid without bubbles.
    pub fn set_wfield(&mut self, rgba: &[u8], size: u32) {
        let t = target(
            &self.device,
            size,
            size,
            wgpu::TextureFormat::Rgba8Unorm,
            "w field",
        );
        self.queue.write_texture(
            t.tex.as_image_copy(),
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: Some(size),
            },
            t.tex.size(),
        );
        self.w_tex = Some(t);
    }

    /// Whether the W field has been handed over.
    pub fn has_wfield(&self) -> bool {
        self.w_tex.is_some()
    }

    /// The start page's card image as last drawn, for checks.
    pub fn start_texture(&self) -> Option<&wgpu::Texture> {
        self.start.as_ref()?.sized.as_ref().map(|s| &s.card.tex)
    }

    /// The layer's frame: its liquid by its own clock (at the page's pace), then into the card's
    /// image its backdrop, the bubbles and the pops. Returns the output pass's card bind group;
    /// `None` without a layer or with no card yet.
    fn start_passes(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        dt: f32,
        pace: f32,
    ) -> Option<wgpu::BindGroup> {
        let mut l = self.start.take()?;
        let out = self.start_layer_passes(enc, &mut l, dt, pace);
        self.start = Some(l);
        out
    }

    fn start_layer_passes(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        l: &mut StartLayer,
        dt: f32,
        pace: f32,
    ) -> Option<wgpu::BindGroup> {
        let StartCard { rect, radius } = l.draw.card;
        if rect.w < 1.0 || rect.h < 1.0 {
            return None;
        }
        let (w, h, dpr) = (rect.w, rect.h, self.scene.dpr);
        let rs = l.scene.resize(w, h, dpr);
        let scale = self.quality.scale();
        let (cw, ch) = out_size(w, h, dpr);
        let mut sz = match l.sized.take() {
            Some(s) if !rs.targets && s.pyramid.scale == scale => s,
            old => {
                let d = &self.device;
                StartSized {
                    fluid: match old {
                        Some(s) if !rs.fluid => s.fluid,
                        _ => fluid_targets(d, w, h, &l.scene.opts.fluid),
                    },
                    pyramid: Pyramid::new(d, w, h, scale),
                    card: target(d, cw, ch, OUT, "wubble start card"),
                }
            }
        };
        let pace = pace * l.draw.pace.clamp(0.0, 1.0);
        l.scene.advance(dt, pace);
        if let Some(dt) = l.clock.tick(self.quality, dt * pace, !sz.pyramid.drawn) {
            let f = &mut sz.fluid;
            for op in l.scene.fluid_ops(dt) {
                self.fluid_op(enc, f, op);
            }
            self.fluid_step(enc, f, dt);
            let cu = composite_u(&l.scene);
            self.backdrop_passes(enc, &cu, &f.dye.read.view, &mut sz.pyramid);
        }

        // the card: the layer's backdrop, the bubbles (body first), the pops
        let py = &sz.pyramid;
        let g = Globals {
            view: [w, h],
            dpr,
            light: l.scene.light,
            time: l.draw.time,
        };
        let blit = self.bind(&self.p.blit, &[Res::Tex(2, &py.bg.view)]);
        // no bubble before the W field is here (a missing texture never draws a wrong frame)
        let w_view = self.w_tex.as_ref().map(|t| t.view.clone());
        let mut bubbles = Vec::with_capacity(l.draw.bubbles.len());
        for (wv, b) in w_view
            .iter()
            .flat_map(|w| l.draw.bubbles.iter().map(move |b| (w, b)))
        {
            let u = self.arena.push(&bubble_u(b, g));
            bubbles.push(self.bind(
                &self.p.bubble,
                &[
                    Res::U(u.0, u.1),
                    Res::Tex(2, &py.bg.view),
                    Res::Tex(3, wv),
                    Res::Tex(4, &py.b1.view),
                    Res::Tex(5, &py.b2.view),
                ],
            ));
        }
        let pops: Vec<wgpu::BindGroup> = l
            .draw
            .pops
            .iter()
            .map(|p| {
                let u = self.arena.push(&pop_u(p, g));
                self.bind(&self.p.pop, &[Res::U(u.0, u.1)])
            })
            .collect();
        {
            let mut rp = self.pass(enc, "wubble start card", &sz.card.view);
            rp.set_pipeline(&self.p.blit);
            rp.set_bind_group(0, &blit, &[]);
            rp.draw(0..4, 0..1);
            rp.set_pipeline(&self.p.bubble);
            for b in &bubbles {
                rp.set_bind_group(0, b, &[]);
                rp.draw(0..4, 0..1);
            }
            rp.set_pipeline(&self.p.pop);
            for b in &pops {
                rp.set_bind_group(0, b, &[]);
                rp.draw(0..4, 0..1);
            }
        }
        // the card in the output, rounded like the page card
        let u = self.arena.push(&PageU {
            u_quad: [rect.x, rect.y, rect.w, rect.h],
            u_view: [self.scene.w, self.scene.h],
            u_uv: [1.0, 1.0],
            u_r: radius,
            _p: [0.0; 3],
        });
        let card = self.bind(
            &self.p.card,
            &[Res::U(u.0, u.1), Res::Tex(2, &sz.card.view)],
        );
        l.sized = Some(sz);
        Some(card)
    }
}

// @source headless readback (crate/examples/headless.rs)
/// Copies an `Rgba8Unorm` texture to the CPU as tightly packed RGBA rows and
/// blocks until it is done. For tests and screenshots only: never on the frame
/// path.
pub fn readback_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (tex.width(), tex.height());
    let bpr =
        (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scene readback"),
        size: u64::from(bpr * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bpr),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([enc.finish()]);
    buf.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("wait for readback");
    let mapped = buf.slice(..).get_mapped_range().expect("mapped readback");
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in mapped.chunks(bpr as usize) {
        out.extend_from_slice(&row[..(w * 4) as usize]);
    }
    out
}

#[cfg(test)]
mod tests {
    //! Shader checks with naga, no GPU: every module the renderer builds
    //! parses and validates, has its entry points, and its uniform structs
    //! have the size of their Rust mirrors.
    use super::*;

    fn validated(body: &str) -> naga::Module {
        let src = with_prelude(body);
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        module
    }

    fn struct_size(module: &naga::Module, name: &str) -> u32 {
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).expect("layout");
        let (h, _) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no struct {name}"));
        layouter[h].size
    }

    #[test]
    fn a_window_above_the_dpr_cap_keeps_its_logical_size_and_gets_a_smaller_texture() {
        // 1000x600 logical at 3x: the scene's view is the window's, drawn at 2x
        let (w, h, dpr) = view_of(3000, 1800, 3.0);
        assert_eq!((w, h, dpr), (1000.0, 600.0, 2.0));
        assert_eq!(out_size(w, h, dpr), (2000, 1200));
        // at or below the cap the texture is the window's
        let (w, h, dpr) = view_of(2720, 1760, 2.0);
        assert_eq!(out_size(w, h, dpr), (2720, 1760));
        let (w, h, dpr) = view_of(1700, 1100, 1.25);
        assert_eq!(out_size(w, h, dpr), (1700, 1100));
    }

    #[test]
    fn the_ambience_updates_at_its_rate() {
        assert_eq!(ambience_every(), Duration::from_millis(125));
    }

    // the ambience is subtle, and reads one of `P`'s two levels or a mix of them
    const _: () = assert!(AMBIENCE_STRENGTH > 0.0 && AMBIENCE_STRENGTH < 0.5);
    const _: () = assert!(AMBIENCE_LEVEL >= 0.0 && AMBIENCE_LEVEL <= 1.0);

    #[test]
    fn every_shader_validates_with_its_entry_points() {
        let modules: [(&str, &[&str]); 8] = [
            (
                FLUID_WGSL,
                &[
                    "vs_fullscreen",
                    "fs_advect",
                    "fs_splat",
                    "fs_curl",
                    "fs_vorticity",
                    "fs_divergence",
                    "fs_pressure",
                    "fs_gradient",
                    "fs_scale",
                ],
            ),
            (COMPOSITE_WGSL, &["vs_fullscreen", "fs_composite"]),
            (BLUR_WGSL, &["vs_fullscreen", "fs_blur"]),
            (BLIT_WGSL, &["vs_fullscreen", "fs_blit"]),
            (GLASS_WGSL, &["vs_glass", "fs_glass"]),
            (PAGE_WGSL, &["vs_page", "vs_card", "fs_page", "fs_resolve"]),
            (BUBBLE_WGSL, &["vs_bubble", "fs_bubble"]),
            (POP_WGSL, &["vs_pop", "fs_pop"]),
        ];
        for (body, entries) in modules {
            let m = validated(body);
            for e in entries {
                assert!(
                    m.entry_points.iter().any(|p| p.name == *e),
                    "missing entry point {e}"
                );
            }
        }
    }

    #[test]
    fn the_glass_reads_the_two_deepest_levels_where_the_frame_binds_them() {
        let m = validated(GLASS_WGSL);
        let binding = |name: &str| {
            m.global_variables
                .iter()
                .find(|(_, v)| v.name.as_deref() == Some(name))
                .and_then(|(_, v)| v.binding.as_ref())
                .map(|b| b.binding)
        };
        // `frame` binds b2 at 3 and b3 at 4, and the page pyramid's at 5 and 6
        assert_eq!((binding("uB2"), binding("uB3")), (Some(3), Some(4)));
        assert_eq!((binding("uP2"), binding("uP3")), (Some(5), Some(6)));
    }

    #[test]
    fn uniform_mirrors_match_the_wgsl_layout() {
        let size = |body, name| struct_size(&validated(body), name) as usize;
        assert_eq!(size(FLUID_WGSL, "FluidU"), size_of::<FluidU>());
        assert_eq!(size(COMPOSITE_WGSL, "CompositeU"), size_of::<CompositeU>());
        assert_eq!(size(BLUR_WGSL, "BlurU"), size_of::<BlurU>());
        assert_eq!(size(GLASS_WGSL, "GlassU"), size_of::<GlassU>());
        assert_eq!(size(PAGE_WGSL, "PageU"), size_of::<PageU>());
        assert_eq!(size(BUBBLE_WGSL, "BubbleU"), size_of::<start::BubbleU>());
        assert_eq!(size(POP_WGSL, "PopU"), size_of::<start::PopU>());
    }
}
