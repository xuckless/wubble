//! The scene on wgpu: fluid backdrop, blur pyramid, glass quads.
//!
//! Source: the-wallpaper-project `src/wubble/slint/render.rs` (`Renderer`,
//! uniform mirrors, `Arena`, `Pipelines`, `fluid_op`, `fluid_step`, `blur`,
//! `frame`) and `crate/examples/headless.rs` (the readback). Owner's code.
//! Changed: built on the shared `GpuContext` device; no bubble, pop or
//! W-field passes and no overlay mode; the backdrop work is skipped on the
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
//! `out` is Rgba8Unorm (not sRGB) on purpose: WebGL writes shader values
//! straight to the canvas with no linear→sRGB encode, so a non-sRGB target
//! reproduces the design's colours exactly.
//!
//! Uniforms: each draw's uniforms go into a CPU arena at aligned offsets, the
//! arena is uploaded once per frame, and each draw binds its own slice.

use std::num::NonZeroU64;
use std::time::{Duration, Instant};

use wgpu_profiler::{GpuProfiler, GpuProfilerSettings};

use super::quality::{BackdropClock, MAX_DT, Quality};
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
    _p: [f32; 3],
    u_beam: [f32; 4],
    u_voice: [f32; 4],
    u_wave: [[f32; 4]; 4],
} // 208 bytes

const _: () = {
    assert!(std::mem::size_of::<FluidU>() == 64);
    assert!(std::mem::size_of::<CompositeU>() == 160);
    assert!(std::mem::size_of::<BlurU>() == 16);
    assert!(std::mem::size_of::<GlassU>() == 208);
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
        // needs on `out`; COPY_SRC for readbacks (tests, screenshots)
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
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
    out: Target,
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
}

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
        })
    }

    /// How many backdrops (composite and blur pyramid) have been rendered: a still backdrop
    /// renders one only when its dye, its ripples or its size changed.
    pub fn backdrops(&self) -> u64 {
        self.backdrops
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

    /// Whether the scene moves on its own beyond the ambient drift ([`Scene::animating`]).
    pub fn animating(&self) -> bool {
        self.scene.animating()
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
    /// the back of the window when the output texture is new (first frame, resize);
    /// `Ok(None)` means the image already set is still the one, as in the design.
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
    ) -> Result<Option<slint::Image>, GfxError> {
        let dpr = window.scale_factor().min(DPR_MAX);
        let size = window.size().to_logical(dpr);
        let dt = self
            .last_frame
            .replace(now)
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        let tex = self.render_paced(size.width, size.height, dpr, dt.min(MAX_DT), pace);
        tex.map(|t| slint::Image::try_from(t).map_err(|e| GfxError::Image(e.to_string())))
            .transpose()
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
        self.frame(dt, pace.clamp(0.0, 1.0));
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
            }
            return None;
        }
        // keep the liquid unless the scene asked for new grids
        let fluid = match self.sized.take() {
            Some(s) if !rs.fluid => s.fluid,
            _ => fluid_targets(d, w, h, &self.scene.opts.fluid),
        };
        let out = target(
            d,
            ((w * dpr).round() as u32).max(1),
            ((h * dpr).round() as u32).max(1),
            OUT,
            "wubble scene",
        );
        let tex = out.tex.clone();
        self.sized = Some(Sized {
            fluid,
            pyramid: Pyramid::new(d, w, h, scale),
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
    /// Advance the scene by `dt` seconds and render into `out`.
    fn frame(&mut self, dt: f32, pace: f32) {
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
        let (vw, vh, t) = (self.scene.w, self.scene.h, self.scene.time);

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

            // 2. composite the backdrop at half resolution
            let mut cu = CompositeU {
                u_ground: self.scene.ground,
                u_time: t,
                u_view: [vw, vh],
                u_mute: self.scene.opts.intensity,
                ..Default::default()
            };
            cu.u_rip_n = self.scene.ripples.len() as i32;
            for (i, r) in self.scene.ripples.iter().enumerate() {
                cu.u_rip[i] = [r.x, r.y, r.age / r.life, r.reach];
            }
            let u = self.arena.push(&cu);
            let py = &mut sz.pyramid;
            let b = self.bind(
                &self.p.comp,
                &[Res::U(u.0, u.1), Res::Tex(2, &f.dye.read.view)],
            );
            self.run(&mut enc, &self.p.comp, &b, &py.bg.view);

            // 3. blur pyramid for the glass: σ in texels, scaled so its CSS px stay the design's
            let [s1, s2, s3] = BLUR_SIGMA.map(|s| s * py.scale);
            self.blur(&mut enc, &py.bg, &py.b1t, &py.b1, s1);
            self.blur(&mut enc, &py.b1, &py.b2t, &py.b2, s2);
            self.blur(&mut enc, &py.b2, &py.b3t, &py.b3, s3); // the frost: ≈44 CSS px σ in all
            py.drawn = true;
            self.backdrops += 1;
        }

        // 4. one pass on the output: backdrop, then the glass (the beam inside its panel's)
        let py = &sz.pyramid;
        let blit = self.bind(&self.p.blit, &[Res::Tex(2, &py.bg.view)]);
        let surfaces = self.scene.surfaces();
        let mut glass = Vec::with_capacity(surfaces.len());
        for s in &surfaces {
            let gu = self.glass_u(s);
            let u = self.arena.push(&gu);
            glass.push(self.bind(
                &self.p.glass,
                &[
                    Res::U(u.0, u.1),
                    Res::Tex(3, &py.b2.view),
                    Res::Tex(4, &py.b3.view),
                ],
            ));
        }
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wubble scene out"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &sz.out.view,
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
            rp.set_pipeline(&self.p.blit);
            rp.set_bind_group(0, &blit, &[]);
            rp.draw(0..4, 0..1);
            rp.set_pipeline(&self.p.glass);
            for b in &glass {
                rp.set_bind_group(0, b, &[]);
                rp.draw(0..4, 0..1);
            }
        }

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
            _p: [0.0; 3],
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
    fn every_shader_validates_with_its_entry_points() {
        let modules: [(&str, &[&str]); 5] = [
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
        // `frame` binds b2 at 3 and b3 at 4
        assert_eq!((binding("uB2"), binding("uB3")), (Some(3), Some(4)));
    }

    #[test]
    fn uniform_mirrors_match_the_wgsl_layout() {
        let size = |body, name| struct_size(&validated(body), name) as usize;
        assert_eq!(size(FLUID_WGSL, "FluidU"), size_of::<FluidU>());
        assert_eq!(size(COMPOSITE_WGSL, "CompositeU"), size_of::<CompositeU>());
        assert_eq!(size(BLUR_WGSL, "BlurU"), size_of::<BlurU>());
        assert_eq!(size(GLASS_WGSL, "GlassU"), size_of::<GlassU>());
    }
}
