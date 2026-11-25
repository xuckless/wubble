//! Native frame -> `wgpu::Texture` on Slint's device -> `slint::Image`.
//!
//! [`GpuContext`] creates the one wgpu device that both Slint and the frame
//! importer use (Slint gets it through `WGPUConfiguration::Manual`). On Linux
//! the device carries the dmabuf import extensions, which Slint's own device
//! would not; on macOS it is a plain Metal device (IOSurface import).
//!
//! [`FramePresenter`] imports each CEF frame with welding and copies it, GPU to
//! GPU, into a Slint-compatible target. The copy exists because Slint only
//! accepts `Rgba8Unorm(Srgb)` textures (we use `Rgba8Unorm`, see `TARGET_FORMAT`) with `TEXTURE_BINDING | RENDER_ATTACHMENT`
//! (i-slint-core `graphics/wgpu_30.rs`, `TryFrom<wgpu::Texture> for Image`),
//! while welding hands over CEF's BGRA frame (dmabuf or IOSurface), copied in
//! the paint callback, as `Bgra8UnormSrgb` with `TEXTURE_BINDING | COPY_SRC`. `copy_texture_to_texture` cannot swizzle, so a
//! fullscreen-triangle blit does it. Nothing is read back to the CPU.

use std::fmt;
use std::time::{Duration, Instant};

use slint::wgpu_30::WGPUConfiguration;
use welding::wgpu;
use welding::{HostWgpuContext, WgpuTextureImporter};

use crate::FrameSize;
use crate::mask::{EdgeMask, MaskJob, Masked};

/// What went wrong on the GPU side. The text is the underlying error verbatim.
#[derive(Debug)]
pub enum GfxError {
    /// No hardware adapter on the import backend (Vulkan on Linux, Metal on
    /// macOS), or this platform has no import path yet.
    Adapter(String),
    /// The import-capable device could not be created.
    Device(String),
    /// welding/grafting could not import a frame.
    Import(String),
    /// Slint refused the blit target.
    Image(String),
    /// The page's OS layer could not be made or refused a frame (`crate::layer`).
    Layer(String),
}

impl fmt::Display for GfxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(e) => write!(f, "no usable GPU adapter: {e}"),
            Self::Device(e) => write!(f, "cannot create the GPU device: {e}"),
            Self::Import(e) => write!(f, "frame import failed: {e}"),
            Self::Image(e) => write!(f, "Slint rejected the frame texture: {e}"),
            Self::Layer(e) => write!(f, "page layer: {e}"),
        }
    }
}

impl std::error::Error for GfxError {}

/// One CEF frame, copied on the GPU from its native handle (a dmabuf on
/// Linux, an IOSurface on macOS). Only this crate opens it; `wubble-engine`
/// hands it over unopened.
pub struct NativeFrame(welding::NativeFrame);

/// The part of a frame that changed since its view's previous frame, in the frame's pixels.
pub use welding::Damage;

/// When CEF painted a frame and what copying it cost the paint callback.
/// Only frames copied inside the callback (Linux, macOS) carry it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameTiming {
    /// Entry to `OnAcceleratedPaint`.
    pub arrived: Instant,
    /// Import, copy and wait, on the thread that ran the callback.
    pub copy: Duration,
    /// The wait for the copy's submission, part of `copy`.
    pub copy_wait: Duration,
}

impl NativeFrame {
    pub fn size(&self) -> FrameSize {
        let size = self.0.size();
        FrameSize::new(size.width, size.height)
    }

    /// CEF's paint count for this view when it painted the frame; a gap
    /// between two taken frames is frames replaced before they were taken.
    pub fn generation(&self) -> u64 {
        self.0.generation()
    }

    /// What changed since the view's previous frame, when CEF said (`None`: maybe all of it).
    pub fn damage(&self) -> Option<Damage> {
        match &self.0 {
            welding::NativeFrame::Copied(c) => c.damage,
            _ => None,
        }
    }

    /// Whether the frame was copied into an OS buffer (`crate::layer`): macOS and Linux.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn has_os_buffer(&self) -> bool {
        self.os_buffer().is_some()
    }

    /// Whether the frame was copied into an OS buffer: never here.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn has_os_buffer(&self) -> bool {
        false
    }

    /// The OS buffer the frame was copied into, when the view was asked for them.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn os_buffer(&self) -> Option<&welding::OsBuffer> {
        match &self.0 {
            welding::NativeFrame::Copied(c) => c.os.as_ref(),
            _ => None,
        }
    }

    pub fn timing(&self) -> Option<FrameTiming> {
        match &self.0 {
            welding::NativeFrame::Copied(c) => Some(FrameTiming {
                arrived: c.arrived,
                copy: c.copy,
                copy_wait: c.copy_wait,
            }),
            _ => None,
        }
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
    /// A device on the high-performance hardware adapter (never a CPU
    /// rasteriser such as llvmpipe) that can import CEF's frames: Vulkan with
    /// the dmabuf import extensions on Linux, Metal on macOS.
    pub fn new() -> Result<Self, GfxError> {
        // Same instance settings as Slint's Automatic path (i-slint-core
        // graphics/wgpu_30.rs), restricted to the backend this OS imports on.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: IMPORT_BACKENDS,
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
        let (device, queue) = import_device(&adapter)?;
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

    /// GPU memory wgpu's allocator holds on this device, in bytes: `(allocated, reserved)`, the
    /// live allocations and the blocks they come from (reserved stays when allocations go).
    /// Vulkan only (gpu-allocator); `None` on Metal, whose resources are counted in the
    /// process's footprint. Textures imported from CEF's frames are not in it.
    pub fn allocator_bytes(&self) -> Option<(u64, u64)> {
        self.device
            .generate_allocator_report()
            .map(|r| (r.total_allocated_bytes, r.total_reserved_bytes))
    }

    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn presenter(&self) -> FramePresenter {
        FramePresenter {
            ctx: HostWgpuContext::new(self.device.clone(), self.queue.clone()),
            blit: Blitter::new(&self.device, BLIT_FS_WGSL, wgpu::FilterMode::Nearest),
            shrink: Blitter::new(&self.device, SHRINK_FS_WGSL, wgpu::FilterMode::Linear),
            ring: None,
            mask: EdgeMask::new(&self.device),
            generation: None,
        }
    }
}

/// The wgpu backend CEF's shared textures import into on this OS.
#[cfg(target_os = "linux")]
const IMPORT_BACKENDS: wgpu::Backends = wgpu::Backends::VULKAN;
#[cfg(target_os = "macos")]
const IMPORT_BACKENDS: wgpu::Backends = wgpu::Backends::METAL;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const IMPORT_BACKENDS: wgpu::Backends = wgpu::Backends::all();

#[cfg(target_os = "linux")]
fn import_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GfxError> {
    welding::build_dmabuf_capable_device(adapter, &slint_device_descriptor(adapter))
        .map_err(|e| GfxError::Device(e.to_string()))
}

/// Metal imports an IOSurface on any device: no extra features.
#[cfg(target_os = "macos")]
fn import_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GfxError> {
    poll_once(adapter.request_device(&slint_device_descriptor(adapter)))
        .ok_or_else(|| GfxError::Device("device request did not complete".into()))?
        .map_err(|e| GfxError::Device(e.to_string()))
}

/// Placeholder until Windows (D3D11) gets its import path.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn import_device(_adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GfxError> {
    Err(GfxError::Adapter(
        "zero-copy import is Linux and macOS only so far".into(),
    ))
}

/// The descriptor Slint uses for its own device, so the renderer sees the same
/// features and limits under `Manual` as under `Automatic`.
/// Ported from Slint 1.19 i-slint-core `graphics/wgpu_30.rs`
/// `default_device_descriptor` (GPL-3.0-only option of Slint's licence).
#[cfg(any(target_os = "linux", target_os = "macos"))]
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
pub(crate) fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut ctx) {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    }
}

/// The blit target format. A UNORM texture's values are display-encoded sRGB
/// to Slint's Skia renderer, which draws them onto the UNORM surface unchanged
/// (i-slint-renderer-skia 1.19 `sampled_texture_color_space`). Not an sRGB
/// format: femtovg, the earlier renderer, showed one too dark (#808080 came out
/// as #373737), and UNORM needs no conversion anywhere. The blit re-encodes.
pub(crate) const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Number of blit targets. Two, so consecutive frames are different images
/// and setting one on the window always changes the property.
const RING_LEN: usize = 2;

/// One Slint-compatible texture and its `Image`, wrapped once.
pub(crate) struct Target {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) image: slint::Image,
}

/// Targets alive in every ring (the presenters', the edge masks', the orbs'; `--measure`'s
/// `gpu_targets`), so a run that
/// opens and closes tabs can tell a cache from a leak. A texture the shell still shows outlives
/// its ring until Slint lets the image go.
static LIVE_TARGETS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The frame targets alive now ([`LIVE_TARGETS`]).
pub fn live_targets() -> usize {
    LIVE_TARGETS.load(std::sync::atomic::Ordering::Relaxed)
}

impl Drop for Ring {
    fn drop(&mut self) {
        LIVE_TARGETS.fetch_sub(RING_LEN, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) struct Ring {
    pub(crate) size: FrameSize,
    targets: [Target; RING_LEN],
    next: usize,
}

impl Ring {
    pub(crate) fn new(device: &wgpu::Device, size: FrameSize) -> Result<Self, GfxError> {
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
                // COPY_SRC only for `FramePresenter::probe`.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let image = slint::Image::try_from(texture.clone())
                .map_err(|e| GfxError::Image(e.to_string()))?;
            Ok(Target {
                texture,
                view,
                image,
            })
        };
        let targets = [target()?, target()?];
        LIVE_TARGETS.fetch_add(RING_LEN, std::sync::atomic::Ordering::Relaxed);
        Ok(Self {
            size,
            targets,
            next: 0,
        })
    }

    /// The target for the next frame; advances the ring.
    pub(crate) fn advance(&mut self) -> &Target {
        let slot = self.next;
        self.next = (slot + 1) % RING_LEN;
        &self.targets[slot]
    }

    /// The target `advance` returned last.
    pub(crate) fn latest(&self) -> &Target {
        &self.targets[(self.next + RING_LEN - 1) % RING_LEN]
    }
}

/// The damage a frame may be written with in place, into the target on screen: `None` when it
/// must be presented whole. In place needs all of: the frame says what changed (`damage`), and
/// not everything; it is the generation after the one presented last (a frame replaced before
/// it was taken changed things this damage does not cover); the ring's targets are the frame's
/// size (a resized pane's first frame of the new size, or a frame of the old size after it,
/// gets a new ring and is presented whole); and the image on screen is the ring's latest target.
fn in_place(
    damage: Option<Damage>,
    last: Option<u64>,
    generation: u64,
    ring: Option<FrameSize>,
    size: FrameSize,
    shows_latest: bool,
) -> Option<Damage> {
    let damage = damage?;
    let next = last.map(|g| g + 1) == Some(generation);
    (next && ring == Some(size) && shows_latest && !damage.is_whole(size.width, size.height))
        .then_some(damage)
}

/// Turns CEF frames into `slint::Image`s on the shared device.
pub struct FramePresenter {
    ctx: HostWgpuContext,
    blit: Blitter,
    /// The snapshot pass: the latest frame into a small texture (`snapshot`).
    shrink: Blitter,
    ring: Option<Ring>,
    /// The bowed-edge pass over the latest frame (`crate::mask`).
    mask: EdgeMask,
    /// The generation (CEF's paint count) of the frame presented last: a frame's damage is
    /// relative to the one before it, so it holds only for the next generation.
    generation: Option<u64>,
}

impl FramePresenter {
    /// Imports `frame`, blits it into the next ring target and returns that
    /// target's image. The blit is submitted on the queue Slint renders with,
    /// so it completes before Slint samples the image.
    pub fn present(&mut self, frame: NativeFrame) -> Result<slint::Image, GfxError> {
        let size = frame.size();
        self.generation = Some(frame.generation());
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
            None,
        );
        Ok(target.image.clone())
    }

    /// As [`FramePresenter::present`], but when the frame says what changed and `shown` is the
    /// image this presenter returned last (still on screen, at the frame's size), only that part
    /// is written, in place: the same image comes back with the damage, which the caller marks
    /// dirty (Slint sees no new image). Otherwise the next ring target, whole, and `None`.
    pub fn present_damaged(
        &mut self,
        frame: NativeFrame,
        shown: &slint::Image,
    ) -> Result<(slint::Image, Option<Damage>), GfxError> {
        let shows_latest = self.ring.as_ref().is_some_and(|ring| {
            shown.to_wgpu_30_texture().as_ref() == Some(&ring.latest().texture)
        });
        let in_place = in_place(
            frame.damage(),
            self.generation,
            frame.generation(),
            self.ring.as_ref().map(|ring| ring.size),
            frame.size(),
            shows_latest,
        );
        let Some(damage) = in_place else {
            return self.present(frame).map(|image| (image, None));
        };
        self.generation = Some(frame.generation());
        let ring = self.ring.as_ref().expect("matched above");
        let target = ring.latest();
        if damage.width > 0 && damage.height > 0 {
            let imported = WgpuTextureImporter::import(frame.0, &self.ctx)
                .map_err(|e| GfxError::Import(e.to_string()))?;
            self.blit.run(
                &self.ctx.device,
                &self.ctx.queue,
                &imported.view,
                &target.view,
                Some(damage),
            );
        }
        Ok((target.image.clone(), Some(damage)))
    }

    /// `frame` written whole over the target shown last, without taking the frame (the page
    /// layer's snapshot: its layer keeps showing the frame and may hand it over later); into a new
    /// ring when there is none of the frame's size. The image comes back: the same one when
    /// written in place, which Slint sees no change of. Only a frame copied in CEF's callback can
    /// be read so (its copy is a texture of its own: on macOS the pool slot the layer shows).
    pub fn refresh(&mut self, frame: &NativeFrame) -> Result<slint::Image, GfxError> {
        let welding::NativeFrame::Copied(copied) = &frame.0 else {
            return Err(GfxError::Import(
                "only a copied frame can be read without taking it".into(),
            ));
        };
        let size = frame.size();
        if self.ring.as_ref().is_none_or(|ring| ring.size != size) {
            self.ring = Some(Ring::new(&self.ctx.device, size)?);
        }
        self.generation = Some(frame.generation());
        let ring = self.ring.as_ref().expect("made above");
        let target = ring.latest();
        self.blit.run(
            &self.ctx.device,
            &self.ctx.queue,
            &copied.texture.view,
            &target.view,
            None,
        );
        Ok(target.image.clone())
    }

    /// Lets the ring's targets go (its pane went: a split closed, the peek or DevTools closed);
    /// the next frame makes a new ring, as after a resize.
    pub fn release(&mut self) {
        self.ring = None;
        self.generation = None;
    }

    /// The RGBA pixel at (`x`, `y`) (physical, clamped to the frame) of the
    /// frame `present` returned last, `None` before the first. A CPU
    /// readback that waits for the GPU: for the hand-run `--measure` input
    /// latency probe only, never on the normal frame path.
    pub fn probe(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let ring = self.ring.as_ref()?;
        let target = ring.latest();
        let (device, queue) = (&self.ctx.device, &self.ctx.queue);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wubble probe"),
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("wubble probe"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: x.min(ring.size.width.saturating_sub(1)),
                    y: y.min(ring.size.height.saturating_sub(1)),
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..8);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .ok()?;
        let data = slice.get_mapped_range().ok()?;
        Some([data[0], data[1], data[2], data[3]])
    }

    /// A small copy of the latest frame, at most `max_width` wide (the aspect kept), in a
    /// texture of its own that is never overwritten: the snapshot a hidden or discarded tab
    /// shows (`wubble_core::snapshot`) and the tab preview card's thumbnail. GPU only, no
    /// readback: one bilinear pass into a new texture. Returns the image and its size in bytes;
    /// `None` before the first frame.
    pub fn snapshot(&self, max_width: u32) -> Result<Option<(slint::Image, u64)>, GfxError> {
        let Some(ring) = self.ring.as_ref() else {
            return Ok(None);
        };
        let size = shrunk(ring.size, max_width);
        let texture = self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wubble snapshot"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.shrink.run(
            &self.ctx.device,
            &self.ctx.queue,
            &ring.latest().view,
            &view,
            None,
        );
        let image = slint::Image::try_from(texture).map_err(|e| GfxError::Image(e.to_string()))?;
        let bytes = u64::from(size.width) * u64::from(size.height) * 4;
        Ok(Some((image, bytes)))
    }

    /// The latest frame with its bowed edges cut (`crate::mask`), `None` before the first frame.
    /// Needs no new frame: a bending string redraws the same one.
    pub fn mask(&mut self, job: &MaskJob) -> Result<Option<Masked>, GfxError> {
        let Some(ring) = self.ring.as_ref() else {
            return Ok(None);
        };
        let source = &ring.latest().texture;
        self.mask
            .run(&self.ctx.device, &self.ctx.queue, source, job)
            .map(Some)
    }

    /// The device frames are presented on, for welding's in-callback copy.
    pub fn context(&self) -> &HostWgpuContext {
        &self.ctx
    }
}

/// Fullscreen-triangle vertex stage of the blit.
/// Ported from wgpu `examples/features/src/mipmap/blit.wgsl` and the blit
/// pipeline in `examples/features/src/mipmap/mod.rs` (gfx-rs/wgpu,
/// MIT OR Apache-2.0).
const FULLSCREEN_VS_WGSL: &str = r"
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
";

/// Fullscreen-triangle texture copy, fragment stage (same source as
/// `FULLSCREEN_VS_WGSL`). Changed: nearest sampling (source and target are
/// the same size) and the target format.
const BLIT_FS_WGSL: &str = r"
@group(0) @binding(0)
var r_color: texture_2d<f32>;
@group(0) @binding(1)
var r_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The source is an sRGB view, so the sample is linear; the target stores
    // display-encoded bytes (see TARGET_FORMAT), which round-trips exactly.
    let c = textureSample(r_color, r_sampler, in.tex_coords);
    let lo = c.rgb * 12.92;
    let hi = 1.055 * pow(c.rgb, vec3<f32>(1.0 / 2.4)) - 0.055;
    return vec4<f32>(select(hi, lo, c.rgb <= vec3<f32>(0.0031308)), c.a);
}
";

/// The snapshot pass's fragment stage: a plain bilinear sample. Source and target both hold
/// display-encoded bytes (`TARGET_FORMAT`), so nothing is re-encoded; filtering encoded values
/// is not gamma-correct, which a thumbnail does not show.
const SHRINK_FS_WGSL: &str = r"
@group(0) @binding(0)
var r_color: texture_2d<f32>;
@group(0) @binding(1)
var r_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(r_color, r_sampler, in.tex_coords);
}
";

/// `size` scaled down to at most `max_width` wide, the aspect kept, at least 1 x 1.
fn shrunk(size: FrameSize, max_width: u32) -> FrameSize {
    let width = size.width.min(max_width).max(1);
    let height = (u64::from(size.height) * u64::from(width) / u64::from(size.width.max(1))) as u32;
    FrameSize::new(width, height.max(1))
}

/// A fullscreen-triangle pass from one texture into another with one fragment stage.
struct Blitter {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl Blitter {
    fn new(device: &wgpu::Device, fragment: &str, filter: wgpu::FilterMode) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wubble blit"),
            source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_VS_WGSL}{fragment}").into()),
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
            mag_filter: filter,
            min_filter: filter,
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
        // only this part, over what the target holds (source and target one size)
        region: Option<Damage>,
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
                        load: match region {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            if let Some(r) = region {
                pass.set_scissor_rect(r.x, r.y, r.width, r.height);
            }
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_shader_validates() {
        let src = format!("{FULLSCREEN_VS_WGSL}{SHRINK_FS_WGSL}");
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        assert!(module.entry_points.iter().any(|p| p.name == "fs_main"));
    }

    /// A pane resize: the in-place path stays closed from the first frame of the new size (the
    /// ring is the old size) until a frame of the new size has been presented whole, and a late
    /// frame of the old size never writes into targets of the new one.
    #[test]
    fn a_pane_resize_presents_whole_until_a_frame_of_the_new_size_is_on_screen() {
        let small = FrameSize::new(1080, 780);
        let large = FrameSize::new(1280, 780);
        let damage = Some(Damage {
            x: 10,
            y: 10,
            width: 20,
            height: 20,
        });
        // steady at the old size: the next generation goes in place
        assert_eq!(
            in_place(damage, Some(4), 5, Some(small), small, true),
            damage
        );
        // the pane grew: the first frame of the new size meets the old ring, whole
        assert_eq!(in_place(damage, Some(5), 6, Some(small), large, true), None);
        // presented whole, the ring is the new size: in place again
        assert_eq!(
            in_place(damage, Some(6), 7, Some(large), large, true),
            damage
        );
        // a frame of the old size arriving late: whole (the ring is remade at its size)
        assert_eq!(in_place(damage, Some(7), 8, Some(large), small, true), None);
    }

    #[test]
    fn a_frame_is_written_in_place_only_after_the_one_on_screen() {
        let size = FrameSize::new(800, 600);
        let some = Some(Damage {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        });
        let whole = Some(Damage {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        // no damage, a whole damage, a skipped generation, nothing presented yet, or another
        // image on screen: whole
        assert_eq!(in_place(None, Some(1), 2, Some(size), size, true), None);
        assert_eq!(in_place(whole, Some(1), 2, Some(size), size, true), None);
        assert_eq!(in_place(some, Some(1), 3, Some(size), size, true), None);
        assert_eq!(in_place(some, None, 1, None, size, false), None);
        assert_eq!(in_place(some, Some(1), 2, Some(size), size, false), None);
    }

    #[test]
    fn snapshots_shrink_to_the_width_keeping_the_aspect() {
        assert_eq!(
            shrunk(FrameSize::new(2560, 1600), 384),
            FrameSize::new(384, 240)
        );
        // Smaller than the cap: kept.
        assert_eq!(
            shrunk(FrameSize::new(200, 100), 384),
            FrameSize::new(200, 100)
        );
        // Degenerate sizes stay at least one pixel.
        assert_eq!(shrunk(FrameSize::new(4000, 1), 384), FrameSize::new(384, 1));
        assert_eq!(shrunk(FrameSize::new(0, 0), 384), FrameSize::new(1, 1));
    }
}
