//! The GPU side of glass: one shared dual-Kawase down chain from the web
//! frame, one up chain per tier in use whose last pass saturates, output as
//! Slint images. Same queue as the frame blit and Slint, no readback.

use welding::wgpu;
use wgpu::util::DeviceExt;

use super::plan::{KawaseParams, Tier, TierSet, kawase_level_size, kawase_plan};
use super::quality::{CACHED_REFRESH_CALLS, Quality, should_render};
use crate::FrameSize;
use crate::import::{ENCODED_VIEW_FORMAT, FULLSCREEN_VS_WGSL, GfxError, GpuContext, TARGET_FORMAT};

const KAWASE_WGSL: &str = include_str!("kawase.wgsl");

/// Blurred, saturated backdrops of one web frame, one per tier in use.
/// `px_per_frame_px` is backdrop pixels per frame pixel (0.5 at `Full`,
/// 0.25 at `HalfRes`), for the Glass component's source clip.
#[derive(Clone, Debug, Default)]
pub struct Backdrops {
    /// Indexed by `Tier as usize`; `None` for tiers not in use.
    images: [Option<slint::Image>; Tier::ALL.len()],
    pub px_per_frame_px: f32,
}

impl Backdrops {
    pub fn get(&self, tier: Tier) -> Option<&slint::Image> {
        self.images[tier as usize].as_ref()
    }
}

/// What a chain was built for; any change rebuilds it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ChainKey {
    frame: FrameSize,
    /// Where the up chains stop: level 1 (half size, `Full`) or level 2
    /// (quarter size after a 1-tap first level, otherwise).
    out_level: u32,
    tiers: TierSet,
    scale_factor: f32,
}

struct Pipelines {
    layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

/// One fullscreen pass: `pipeline` reading `bind_group` into `target`.
struct Pass {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    target: wgpu::TextureView,
}

/// The first down pass reads the frame, which changes every call, so only
/// its params buffer is kept and its bind group is made per call.
struct FirstPass {
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    target: wgpu::TextureView,
}

struct TierChain {
    tier: Tier,
    /// All up passes but the last.
    passes: Vec<Pass>,
    /// The last up pass reads this; it writes `outputs[next]`.
    last: wgpu::BindGroup,
    outputs: [(wgpu::TextureView, slint::Image); 2],
}

struct Chain {
    key: ChainKey,
    first: FirstPass,
    down: Vec<Pass>,
    tiers: Vec<TierChain>,
    /// What the last `encode` returned, for `Cached` calls that skip.
    last: Backdrops,
    next: usize,
}

/// Renders glass backdrops from web frames. One per window, on the shared
/// device (`GlassBackdrop::new(&gpu)`).
pub struct GlassBackdrop {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Pipelines,
    chain: Option<Chain>,
    skipped: u32,
}

impl GlassBackdrop {
    pub fn new(gpu: &GpuContext) -> Self {
        let device = gpu.device().clone();
        Self {
            pipelines: Pipelines::new(&device),
            queue: gpu.queue().clone(),
            device,
            chain: None,
            // So the first `Cached` call renders.
            skipped: CACHED_REFRESH_CALLS - 1,
        }
    }

    /// Backdrops of `frame` (a `FramePresenter::current_texture`) for the
    /// tiers in `tiers` at `quality`. `scale_factor` is frame pixels per
    /// logical pixel (the window scale factor). `Flat` or an empty tier set
    /// returns no images and frees the textures; `Cached` returns the last
    /// images on all but every `CACHED_REFRESH_CALLS`-th call. `frame` must
    /// allow `Rgba8Unorm` views (`FramePresenter`'s textures do).
    pub fn render(
        &mut self,
        frame: &wgpu::Texture,
        quality: Quality,
        tiers: TierSet,
        scale_factor: f32,
    ) -> Result<Backdrops, GfxError> {
        if quality == Quality::Flat || tiers.is_empty() {
            self.chain = None;
            return Ok(Backdrops::default());
        }
        let size = frame.size();
        let key = ChainKey {
            frame: FrameSize::new(size.width, size.height),
            out_level: if quality == Quality::Full { 1 } else { 2 },
            tiers,
            scale_factor,
        };
        // A cached image of another frame size or tier set would be wrong.
        if !should_render(quality, self.skipped)
            && let Some(chain) = self
                .chain
                .as_ref()
                .filter(|c| c.key.frame == key.frame && c.key.tiers == key.tiers)
        {
            self.skipped += 1;
            return Ok(chain.last.clone());
        }
        self.skipped = 0;
        let chain = match self.chain.take() {
            Some(chain) if chain.key == key => chain,
            _ => Chain::new(&self.device, &self.pipelines, key)?,
        };
        let chain = self.chain.insert(chain);
        chain.encode(&self.device, &self.queue, &self.pipelines, frame);
        Ok(chain.last.clone())
    }
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wubble glass"),
            source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_VS_WGSL}{KAWASE_WGSL}").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("wubble glass"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(
                    2,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("wubble glass"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // Same descriptor shape as `Blitter::new` in import.rs.
        let pipeline = |fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(ENCODED_VIEW_FORMAT.into())],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wubble glass"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            down: pipeline("fs_down"),
            copy: pipeline("fs_copy"),
            up: pipeline("fs_up"),
            layout,
            sampler,
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wubble glass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        })
    }
}

fn params_buffer(device: &wgpu::Device, params: KawaseParams) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("wubble glass"),
        contents: &params.to_bytes(),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

/// A pyramid level texture, sRGB for Slint, viewed encoded by the passes.
/// `COPY_SRC` only so tests can read outputs back.
fn level_texture(device: &wgpu::Device, size: FrameSize) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wubble glass"),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[ENCODED_VIEW_FORMAT],
    })
}

/// The view every pass reads and writes: gamma-encoded values, no sRGB
/// decode, so blur and saturate run on encoded sRGB as CSS filters do in
/// Chromium (and as KWin blurs, whose offsets are tuned for it).
fn view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(ENCODED_VIEW_FORMAT),
        ..Default::default()
    })
}

impl Chain {
    /// Allocates every texture, uniform and bind group for `key`.
    ///
    /// Level `k` is `frame / 2^k`. The shared down chain writes levels
    /// `1..=depth`. A tier with plan depth `n` (at least the output level
    /// `o`) runs up passes reading levels `n, n-1, ..., o`, each writing
    /// level `max(L - 1, o)`; the last one saturates into the output.
    fn new(device: &wgpu::Device, pipelines: &Pipelines, key: ChainKey) -> Result<Self, GfxError> {
        let out_level = key.out_level;
        let plans: Vec<_> = key
            .tiers
            .iter()
            .map(|tier| {
                let plan = kawase_plan(tier.sigma_px(), key.scale_factor);
                (tier, plan.levels.max(out_level), plan.offset)
            })
            .collect();
        let depth = plans.iter().map(|p| p.1).max().unwrap_or(out_level);
        // KWin uses one offset for down and up; tiers share the down chain,
        // so it takes the smallest tier offset and each tier's up chain its own.
        let down_offset = plans.iter().map(|p| p.2).fold(f32::INFINITY, f32::min);
        let size = |level| kawase_level_size(key.frame, level);

        // levels[k - 1] is level k.
        let levels: Vec<wgpu::TextureView> = (1..=depth)
            .map(|k| view(&level_texture(device, size(k))))
            .collect();
        let first = FirstPass {
            pipeline: if out_level == 1 {
                pipelines.down.clone()
            } else {
                pipelines.copy.clone()
            },
            params: params_buffer(device, KawaseParams::new(key.frame, down_offset, 1.0)),
            target: levels[0].clone(),
        };
        let down = (2..=depth)
            .map(|k| {
                let params = KawaseParams::new(size(k - 1), down_offset, 1.0);
                Pass {
                    pipeline: pipelines.down.clone(),
                    bind_group: pipelines.bind_group(
                        device,
                        &levels[k as usize - 2],
                        &params_buffer(device, params),
                    ),
                    target: levels[k as usize - 1].clone(),
                }
            })
            .collect();

        let mut tiers = Vec::new();
        for (tier, n, offset) in plans {
            // Intermediate up results: scratch[k] for levels o..n-1.
            let scratch: Vec<(u32, wgpu::TextureView)> = (out_level..n)
                .map(|k| (k, view(&level_texture(device, size(k)))))
                .collect();
            let scratch_at = |k: u32| &scratch[(k - out_level) as usize].1;
            let source_at = |l: u32| {
                if l == n {
                    &levels[l as usize - 1]
                } else {
                    scratch_at(l)
                }
            };
            let passes = (out_level + 1..=n)
                .rev()
                .map(|l| Pass {
                    pipeline: pipelines.up.clone(),
                    bind_group: pipelines.bind_group(
                        device,
                        source_at(l),
                        &params_buffer(device, KawaseParams::new(size(l), offset, 1.0)),
                    ),
                    target: scratch_at(l - 1).clone(),
                })
                .collect();
            let last_params = KawaseParams::new(size(out_level), offset, tier.saturation());
            let last = pipelines.bind_group(
                device,
                source_at(out_level),
                &params_buffer(device, last_params),
            );
            let output = || -> Result<_, GfxError> {
                let texture = level_texture(device, size(out_level));
                let image = slint::Image::try_from(texture.clone())
                    .map_err(|e| GfxError::Image(e.to_string()))?;
                Ok((view(&texture), image))
            };
            tiers.push(TierChain {
                tier,
                passes,
                last,
                outputs: [output()?, output()?],
            });
        }
        Ok(Self {
            key,
            first,
            down,
            tiers,
            last: Backdrops {
                px_per_frame_px: size(out_level).width as f32 / key.frame.width as f32,
                ..Backdrops::default()
            },
            next: 0,
        })
    }

    /// Records and submits every pass for `frame`; `last` gets the fresh images.
    fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        frame: &wgpu::Texture,
    ) {
        let slot = self.next;
        self.next = (slot + 1) % 2;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("wubble glass"),
        });
        let first_bind = pipelines.bind_group(device, &view(frame), &self.first.params);
        run(
            &mut encoder,
            &self.first.pipeline,
            &first_bind,
            &self.first.target,
        );
        for pass in &self.down {
            run(&mut encoder, &pass.pipeline, &pass.bind_group, &pass.target);
        }
        for tier in &self.tiers {
            for pass in &tier.passes {
                run(&mut encoder, &pass.pipeline, &pass.bind_group, &pass.target);
            }
            let (target, image) = &tier.outputs[slot];
            run(&mut encoder, &pipelines.up, &tier.last, target);
            self.last.images[tier.tier as usize] = Some(image.clone());
        }
        queue.submit([encoder.finish()]);
    }
}

/// One fullscreen-triangle pass, as in `Blitter::run` (import.rs).
fn run(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("wubble glass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}
