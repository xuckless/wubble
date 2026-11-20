//! The thinking orb's painter: one orb's frame (`wubble_core::orb`, a list of dots and lines in
//! draw order with their inks) drawn into a texture of its own, which a kit `ThinkingOrb` shows
//! 1:1. The design paints the same list on a 2D canvas (Ultra Reactivate's
//! `vendor/thinking-orbs/engine/core.ts` `paintFrame`: lines stroked, then dots filled as arcs,
//! source-over on a transparent canvas); `orb.wgsl` does that with one instanced quad per mark
//! and 4×4 coverage samples per pixel, premultiplied into a float target, then resolves to the
//! straight alpha Slint's Skia renderer expects of a wgpu texture (as [`crate::mask`]).
//!
//! Cost: one small render pass and a resolve per drawn frame, only for an orb that moves or has
//! just changed; nothing for an orb at rest or off screen (the host's `wubble_core::orb::Life`
//! says which). The output is a ring of two textures ([`crate::import`]'s), so the image property
//! changes every frame that draws and Slint shows the new one.
//!
//! This crate does not name wubble-core: the host converts the core's marks into [`OrbMark`]s.

use std::num::NonZeroU64;

use welding::wgpu;

use crate::import::Ring;
use crate::scene::with_prelude;
use crate::{FrameSize, GfxError, GpuContext, bytes};

const ORB_WGSL: &str = include_str!("orb.wgsl");
/// The premultiplied sum (a float target: no banding when the alpha is divided back out).
const ACCUM: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// One mark as the painter draws it, in frame units (CSS px of the orb's tuned size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OrbMark {
    /// A filled circle.
    Dot {
        x: f32,
        y: f32,
        r: f32,
        ink: [f32; 4],
    },
    /// A stroked segment, `w` wide, butt caps.
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        w: f32,
        ink: [f32; 4],
    },
}

/// `ink`: r, g, b 0..255, alpha 0..1 (the canvas's `rgba()`).
#[repr(C)]
#[derive(Clone, Copy)]
struct MarkI {
    geo: [f32; 4],
    kind: [f32; 4],
    ink: [f32; 4],
}

impl From<OrbMark> for MarkI {
    fn from(m: OrbMark) -> Self {
        match m {
            OrbMark::Dot { x, y, r, ink } => Self {
                geo: [x, y, r, 0.0],
                kind: [0.0; 4],
                ink,
            },
            OrbMark::Line {
                x1,
                y1,
                x2,
                y2,
                w,
                ink,
            } => Self {
                geo: [x1, y1, x2, y2],
                kind: [1.0, w, 0.0, 0.0],
                ink,
            },
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OrbU {
    view: [f32; 2],
    scale: f32,
    pad: f32,
}

const _: () = assert!(std::mem::size_of::<OrbU>() == 16);
const _: () = assert!(std::mem::size_of::<MarkI>() == 48);

/// One orb's textures: the float target and the output ring, `size` physical px square.
pub struct OrbCanvas {
    size: u32,
    accum_view: wgpu::TextureView,
    ring: Ring,
}

impl OrbCanvas {
    /// The side, physical px.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The texture drawn last (tests read it back).
    pub fn latest(&self) -> &wgpu::Texture {
        &self.ring.latest().texture
    }
}

/// The orb passes, shared by every orb of a window.
pub struct OrbPainter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    marks: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: u64,
}

impl OrbPainter {
    /// On the device Slint renders with.
    pub fn try_new(gpu: &GpuContext) -> Self {
        Self::from_device(gpu.device().clone(), gpu.queue().clone())
    }

    /// On a given device (tests).
    pub fn from_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wubble orb"),
            source: wgpu::ShaderSource::Wgsl(with_prelude(ORB_WGSL).into()),
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
        let marks = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wubble orb marks"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mark"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MarkI>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mark"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ACCUM,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
        });
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wubble orb resolve"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_resolve"),
                compilation_options: Default::default(),
                targets: &[Some(crate::import::TARGET_FORMAT.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wubble orb"),
            size: std::mem::size_of::<OrbU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (instances, capacity) = Self::instance_buffer(&device, 1024);
        Self {
            device,
            queue,
            marks,
            resolve,
            uniform,
            instances,
            capacity,
        }
    }

    fn instance_buffer(device: &wgpu::Device, marks: u64) -> (wgpu::Buffer, u64) {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wubble orb marks"),
            size: marks * std::mem::size_of::<MarkI>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        (buf, marks)
    }

    /// The textures for an orb `size` physical px square.
    pub fn canvas(&self, size: u32) -> Result<OrbCanvas, GfxError> {
        let size = size.max(1);
        let accum = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wubble orb accum"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ACCUM,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let accum_view = accum.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(OrbCanvas {
            size,
            accum_view,
            ring: Ring::new(&self.device, FrameSize::new(size, size))?,
        })
    }

    /// Draws `marks` (in order) at `scale` physical px per frame unit into the canvas's next
    /// output texture, and returns its image.
    pub fn paint(&mut self, canvas: &mut OrbCanvas, scale: f32, marks: &[OrbMark]) -> slint::Image {
        let n = marks.len() as u64;
        if n > self.capacity {
            (self.instances, self.capacity) =
                Self::instance_buffer(&self.device, n.next_power_of_two());
        }
        let data: Vec<MarkI> = marks.iter().map(|&m| m.into()).collect();
        if !data.is_empty() {
            // SAFETY: `MarkI` is #[repr(C)], Copy and all f32 (no padding).
            let raw = unsafe {
                std::slice::from_raw_parts(
                    data.as_ptr() as *const u8,
                    data.len() * std::mem::size_of::<MarkI>(),
                )
            };
            self.queue.write_buffer(&self.instances, 0, raw);
        }
        let side = canvas.size as f32;
        let u = OrbU {
            view: [side, side],
            scale,
            pad: 0.0,
        };
        self.queue.write_buffer(&self.uniform, 0, bytes(&u));
        let target = canvas.ring.advance();
        let marks_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wubble orb marks"),
            layout: &self.marks.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &self.uniform,
                    offset: 0,
                    size: NonZeroU64::new(std::mem::size_of::<OrbU>() as u64),
                }),
            }],
        });
        let resolve_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wubble orb resolve"),
            layout: &self.resolve.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&canvas.accum_view),
            }],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("wubble orb"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wubble orb marks"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &canvas.accum_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if n > 0 {
                pass.set_pipeline(&self.marks);
                pass.set_bind_group(0, &marks_group, &[]);
                pass.set_vertex_buffer(0, self.instances.slice(..));
                pass.draw(0..4, 0..n as u32);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wubble orb resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &resolve_group, &[]);
            pass.draw(0..4, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        target.image.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_validates_and_its_mirrors_match() {
        let src = with_prelude(ORB_WGSL);
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        for entry in ["vs_mark", "fs_mark", "fs_resolve"] {
            assert!(
                module.entry_points.iter().any(|p| p.name == entry),
                "{entry}"
            );
        }
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).expect("layout");
        let (h, _) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("OrbU"))
            .expect("OrbU");
        assert_eq!(layouter[h].size as usize, std::mem::size_of::<OrbU>());
    }
}
