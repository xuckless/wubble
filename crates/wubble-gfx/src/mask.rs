//! The page's bowed edge (owner rule "Resize feel", TODO.md): while a seam's string is bent, the
//! page's edge follows it like rubber. Slint cannot clip an image by a path, so this redraws the
//! pane's last frame into a texture of its own with that edge cut along the string, and Slint
//! shows that texture 1:1 instead of the frame. One draw per bowed pane per frame; never at rest.
//!
//! The page is filled and stretched here exactly as `SplitPane` draws it at rest (the frame fills
//! the pane times the stretch, anchored top left), so switching between the two shows no step.
//! The output is wider than the pane by a margin on each side, room for the edge to bulge out
//! (`SPLIT.margin` in the design, 72 px); the bulge shows the design's page colour where the frame
//! has no pixels. Its alpha is straight: femtovg does not treat a wgpu texture as premultiplied
//! (i-slint-renderer-femtovg 1.19 `images.rs`: `PREMULTIPLIED` only for CPU buffers).
//!
//! Plumbing as the [`crate::import`] blit (wgpu examples, MIT OR Apache-2.0) and the scene's glass
//! pass; the SDF is the scene's `sdRound` (owner's design repo). No third-party code is new here.

use std::num::NonZeroU64;

use welding::wgpu;

use crate::import::Ring;
use crate::scene::with_prelude;
use crate::{Edge, FrameSize, GfxError, pack_edge};

const MASK_WGSL: &str = include_str!("mask.wgsl");

/// What one pane's mask draws.
#[derive(Clone, Copy, Debug)]
pub struct MaskJob<'a> {
    /// The pane's size, logical px.
    pub pane: (f32, f32),
    pub scale: f32,
    /// The frame's extra stretch per axis, 0 at rest (`SplitPane.stretch-x/-y`).
    pub stretch: (f32, f32),
    /// The part of the frame shown, frame px from its top left (`SplitPane.clip-*`).
    pub clip: FrameSize,
    /// Corner radius, logical px.
    pub radius: f32,
    /// Room each side for a bulge, logical px (rounded up to whole physical px).
    pub margin: f32,
    /// The left and right edges' displacement, pane-local heights; `None`: straight.
    pub left: Option<Edge<'a>>,
    pub right: Option<Edge<'a>>,
    /// Where an edge bulges past the frame, display-encoded RGB 0..1.
    pub fill: [f32; 3],
}

impl MaskJob<'_> {
    /// The margin in whole physical px.
    fn margin_px(&self) -> u32 {
        (self.margin * self.scale).ceil().max(0.0) as u32
    }

    /// The output's size: the pane plus the margin each side, physical px.
    pub fn output_size(&self) -> FrameSize {
        let px = |l: f32| (l * self.scale).round().max(1.0) as u32;
        FrameSize::new(px(self.pane.0) + 2 * self.margin_px(), px(self.pane.1))
    }
}

/// The masked pane: its image, and how far left of the pane it starts (logical px; Slint draws
/// it there at `width = pane + 2 · margin`, 1:1 on whole physical pixels).
#[derive(Clone, Debug)]
pub struct Masked {
    pub image: slint::Image,
    pub margin: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MaskU {
    pane: [f32; 2],
    margin: f32,
    scale: f32,
    stretch: [f32; 2],
    clip_uv: [f32; 2],
    fill: [f32; 4],
    radius: f32,
    _p: [f32; 3],
    l_tab: [f32; 4],
    r_tab: [f32; 4],
    left: [[f32; 4]; 64],
    right: [[f32; 4]; 64],
} // 2144 bytes

const _: () = assert!(std::mem::size_of::<MaskU>() == 2144);

fn bytes(u: &MaskU) -> &[u8] {
    // SAFETY: MaskU is #[repr(C)], Copy and made only of f32 with every gap an explicit field.
    unsafe {
        std::slice::from_raw_parts(
            (u as *const MaskU).cast::<u8>(),
            std::mem::size_of::<MaskU>(),
        )
    }
}

/// The mask pass and its own ring of two output textures (as the frame ring: consecutive
/// frames are different images, so setting one always changes the property).
pub struct EdgeMask {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    ring: Option<Ring>,
}

impl EdgeMask {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("wubble edge mask"),
            source: wgpu::ShaderSource::Wgsl(with_prelude(MASK_WGSL).into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wubble edge mask"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mask"),
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
        // linear: a stale frame is scaled to the pane, as femtovg's `image-fit: fill` does
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("wubble edge mask"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("wubble edge mask"),
            size: std::mem::size_of::<MaskU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            sampler,
            uniform,
            ring: None,
        }
    }

    /// Draws `job` from `source` (the pane's frame texture) into the next output texture.
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: &wgpu::Texture,
        job: &MaskJob,
    ) -> Result<Masked, GfxError> {
        let size = job.output_size();
        let ring = match self.ring.take() {
            Some(ring) if ring.size == size => ring,
            _ => Ring::new(device, size)?,
        };
        let target = self.ring.insert(ring).advance();
        let (l_tab, left) = pack_edge(job.left);
        let (r_tab, right) = pack_edge(job.right);
        let margin = job.margin_px();
        let u = MaskU {
            pane: [(size.width - 2 * margin) as f32, size.height as f32],
            margin: margin as f32,
            scale: job.scale,
            stretch: [job.stretch.0, job.stretch.1],
            clip_uv: [
                job.clip.width as f32 / source.width().max(1) as f32,
                job.clip.height as f32 / source.height().max(1) as f32,
            ],
            fill: [job.fill[0], job.fill[1], job.fill[2], 1.0],
            radius: job.radius * job.scale,
            _p: [0.0; 3],
            l_tab: [l_tab[0], l_tab[1], l_tab[2], 0.0],
            r_tab: [r_tab[0], r_tab[1], r_tab[2], 0.0],
            left,
            right,
        };
        queue.write_buffer(&self.uniform, 0, bytes(&u));
        let view = source.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("wubble edge mask"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform,
                        offset: 0,
                        size: NonZeroU64::new(std::mem::size_of::<MaskU>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("wubble edge mask"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("wubble edge mask"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        queue.submit([encoder.finish()]);
        Ok(Masked {
            image: target.image.clone(),
            margin: margin as f32 / job.scale,
        })
    }

    /// The texture `run` drew last (tests read it back).
    pub fn latest(&self) -> Option<&wgpu::Texture> {
        self.ring.as_ref().map(|r| &r.latest().texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_validates_and_its_uniform_mirror_matches() {
        let src = with_prelude(MASK_WGSL);
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        assert!(module.entry_points.iter().any(|p| p.name == "fs_mask"));
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).expect("layout");
        let (h, _) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("MaskU"))
            .expect("MaskU");
        assert_eq!(layouter[h].size as usize, std::mem::size_of::<MaskU>());
    }

    #[test]
    fn the_output_is_the_pane_plus_whole_pixel_margins() {
        let job = MaskJob {
            pane: (1001.0 * 0.8, 600.0),
            scale: 1.25,
            stretch: (0.0, 0.0),
            clip: FrameSize::new(1001, 750),
            radius: 14.0,
            margin: 72.0,
            left: None,
            right: None,
            fill: [1.0; 3],
        };
        // 800.8 · 1.25 = 1001; 72 · 1.25 = 90
        assert_eq!(job.output_size(), FrameSize::new(1001 + 180, 750));
        let odd = MaskJob { scale: 1.1, ..job };
        assert_eq!(odd.margin_px(), 80); // 79.2 up
    }
}
