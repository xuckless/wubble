//! Native frame -> `wgpu::Texture` on Slint's device -> `slint::Image`.
//!
//! [`GpuContext`] creates the one wgpu device that both Slint and the frame
//! importer use (Slint gets it through `WGPUConfiguration::Manual`). The device
//! carries the dmabuf import extensions, which Slint's own device would not.
//!
//! [`FramePresenter`] imports each CEF frame with welding and copies it, GPU to
//! GPU, into a Slint-compatible target. The copy exists because Slint only
//! accepts `Rgba8Unorm(Srgb)` textures with `TEXTURE_BINDING | RENDER_ATTACHMENT`
//! (i-slint-core `graphics/wgpu_30.rs`, `TryFrom<wgpu::Texture> for Image`),
//! while welding/grafting import CEF's BGRA dmabuf as `Bgra8UnormSrgb` with
//! `TEXTURE_BINDING | COPY_SRC`. `copy_texture_to_texture` cannot swizzle, so a
//! fullscreen-triangle blit does it. Nothing is read back to the CPU.

use std::fmt;

use slint::wgpu_30::WGPUConfiguration;
use welding::wgpu;
use welding::{HostWgpuContext, WgpuTextureImporter};

use crate::FrameSize;

/// What went wrong on the GPU side. The text is the underlying error verbatim.
#[derive(Debug)]
pub enum GfxError {
    /// No hardware Vulkan adapter, or this platform has no import path yet.
    Adapter(String),
    /// The dmabuf-capable device could not be created.
    Device(String),
    /// welding/grafting could not import a frame.
    Import(String),
    /// Slint refused the blit target.
    Image(String),
}

impl fmt::Display for GfxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(e) => write!(f, "no usable GPU adapter: {e}"),
            Self::Device(e) => write!(f, "cannot create the GPU device: {e}"),
            Self::Import(e) => write!(f, "frame import failed: {e}"),
            Self::Image(e) => write!(f, "Slint rejected the frame texture: {e}"),
        }
    }
}

impl std::error::Error for GfxError {}

/// One CEF frame as an owned native handle (a dmabuf on Linux). Only this
/// crate opens it; `wubble-engine` hands it over unopened.
pub struct NativeFrame(welding::NativeFrame);

impl NativeFrame {
    pub fn size(&self) -> FrameSize {
        let size = self.0.size();
        FrameSize::new(size.width, size.height)
    }
}

impl From<welding::NativeFrame> for NativeFrame {
    fn from(frame: welding::NativeFrame) -> Self {
        Self(frame)
    }
}

/// The wgpu instance, adapter, device and queue shared by Slint and the importer.
#[derive(Clone)]
pub struct GpuContext {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuContext {
    /// A Vulkan device on the high-performance hardware adapter (never a CPU
    /// rasteriser such as llvmpipe), with the dmabuf import extensions.
    pub fn new() -> Result<Self, GfxError> {
        // Same instance settings as Slint's Automatic path (i-slint-core
        // graphics/wgpu_30.rs), restricted to Vulkan, the only Linux import path.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
            display: None,
        });
        let adapter = poll_once(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok_or_else(|| GfxError::Adapter("adapter request did not complete".into()))?
        .map_err(|e| GfxError::Adapter(e.to_string()))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err(GfxError::Adapter(format!(
                "only a CPU adapter is available ({})",
                info.name
            )));
        }
        let (device, queue) = dmabuf_device(&adapter)?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    /// Hands this device to Slint; pass it to `BackendSelector::require_wgpu_30`.
    pub fn slint_configuration(&self) -> WGPUConfiguration {
        WGPUConfiguration::Manual {
            instance: self.instance.clone(),
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
        }
    }

    /// Adapter name and driver, for logs and `blocked` reports.
    pub fn describe(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({}, {})", info.name, info.driver, info.driver_info)
    }

    pub fn presenter(&self) -> FramePresenter {
        FramePresenter {
            ctx: HostWgpuContext::new(self.device.clone(), self.queue.clone()),
            blit: Blitter::new(&self.device),
            ring: None,
        }
    }
}

#[cfg(target_os = "linux")]
fn dmabuf_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GfxError> {
    welding::build_dmabuf_capable_device(adapter, &slint_device_descriptor(adapter))
        .map_err(|e| GfxError::Device(e.to_string()))
}

/// Placeholder until macOS (IOSurface) and Windows (D3D11) get their import path.
#[cfg(not(target_os = "linux"))]
fn dmabuf_device(_adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GfxError> {
    Err(GfxError::Adapter(
        "zero-copy import is Linux-only so far".into(),
    ))
}

/// The descriptor Slint uses for its own device, so femtovg sees the same
/// features and limits under `Manual` as under `Automatic`.
/// Ported from Slint 1.19 i-slint-core `graphics/wgpu_30.rs`
/// `default_device_descriptor` (GPL-3.0-only option of Slint's licence).
#[cfg(target_os = "linux")]
fn slint_device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: None,
        required_features: adapter.features() - wgpu::Features::all_experimental_mask(),
        required_limits: adapter.limits(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::default(),
    }
}

/// Polls a wgpu future once; native wgpu resolves adapter requests immediately.
/// Copied from Slint 1.19 i-slint-core `graphics/wgpu_30.rs` `poll_once`
/// (GPL-3.0-only option of Slint's licence).
fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut ctx) {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    }
}

/// The blit target format: what Slint accepts, sRGB like CEF's output.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Number of blit targets. Two, so consecutive frames are different images
/// and setting one on the window always changes the property.
const RING_LEN: usize = 2;

/// One Slint-compatible texture and its `Image`, wrapped once.
struct Target {
    view: wgpu::TextureView,
    image: slint::Image,
}

struct Ring {
    size: FrameSize,
    targets: [Target; RING_LEN],
    next: usize,
}

impl Ring {
    fn new(device: &wgpu::Device, size: FrameSize) -> Result<Self, GfxError> {
        let target = || -> Result<Target, GfxError> {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("wubble web frame"),
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
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let image =
                slint::Image::try_from(texture).map_err(|e| GfxError::Image(e.to_string()))?;
            Ok(Target { view, image })
        };
        Ok(Self {
            size,
            targets: [target()?, target()?],
            next: 0,
        })
    }

    /// The target for the next frame; advances the ring.
    fn advance(&mut self) -> &Target {
        let slot = self.next;
        self.next = (slot + 1) % RING_LEN;
        &self.targets[slot]
    }
}

/// Turns CEF frames into `slint::Image`s on the shared device.
pub struct FramePresenter {
    ctx: HostWgpuContext,
    blit: Blitter,
    ring: Option<Ring>,
}

impl FramePresenter {
    /// Imports `frame`, blits it into the next ring target and returns that
    /// target's image. The blit is submitted on the queue Slint renders with,
    /// so it completes before Slint samples the image.
    pub fn present(&mut self, frame: NativeFrame) -> Result<slint::Image, GfxError> {
        let size = frame.size();
        let imported = WgpuTextureImporter::import(frame.0, &self.ctx)
            .map_err(|e| GfxError::Import(e.to_string()))?;
        let ring = match self.ring.take() {
            Some(ring) if ring.size == size => ring,
            _ => Ring::new(&self.ctx.device, size)?,
        };
        let target = self.ring.insert(ring).advance();
        self.blit.run(
            &self.ctx.device,
            &self.ctx.queue,
            &imported.view,
            &target.view,
        );
        Ok(target.image.clone())
    }
}

/// Fullscreen-triangle texture copy.
/// Ported from wgpu `examples/features/src/mipmap/blit.wgsl` and the blit
/// pipeline in `examples/features/src/mipmap/mod.rs` (gfx-rs/wgpu,
/// MIT OR Apache-2.0). Changed: nearest sampling (source and target are the
/// same size) and the target format.
const BLIT_WGSL: &str = r"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = i32(vertex_index) / 2;
    let y = i32(vertex_index) & 1;
    let tc = vec2<f32>(f32(x) * 2.0, f32(y) * 2.0);
    out.position = vec4<f32>(tc.x * 2.0 - 1.0, 1.0 - tc.y * 2.0, 0.0, 1.0);
    out.tex_coords = tc;
    return out;
}

@group(0) @binding(0)
var r_color: texture_2d<f32>;
@group(0) @binding(1)
var r_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(r_color, r_sampler, in.tex_coords);
}
";

struct Blitter {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl Blitter {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wubble blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wubble blit"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(TARGET_FORMAT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wubble blit"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self { pipeline, sampler }
    }

    fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wubble blit"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("wubble blit"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wubble blit"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}
