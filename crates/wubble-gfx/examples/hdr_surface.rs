//! HDR presentation spike: which (format, colour space) pairs does this
//! machine's Vulkan/Wayland surface offer, and does an HDR one present?
//!
//! Run by hand (opens a window for a few dozen frames, then exits by itself):
//!
//!     .local/bin/cargo.sh run -p wubble-gfx --example hdr_surface [-- hdr10|hlg|scrgb|extended-srgb|srgb]
//!
//! It prints the surface capabilities, the display info wgpu reports and the
//! choice it made. With a working HDR output the top row of the pattern shows
//! six greys at 50 / 100 / 203 / 400 / 1000 / 10000 nits, each visibly brighter
//! than the last; on an SDR output everything from 100 nits up is the same
//! white. No timing is asserted: the exit is a frame count.
//!
//! Ported from gfx-rs/wgpu `examples/standalone/03_hdr_surface/src/main.rs`
//! (trunk, wgpu 30; MIT OR Apache-2.0), trimmed to native, without the macOS
//! screen observer or the web build, and with the window closing after
//! `FRAMES` redraws. The shader is `hdr_surface.wgsl`, unchanged.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Redraws before the window closes itself. Not a duration.
const FRAMES: u32 = 90;

struct ModeChoice {
    format: wgpu::TextureFormat,
    color_space: wgpu::SurfaceColorSpace,
    shader_mode: u32,
}

/// The most capable (format, colour space) the surface offers: HDR10, HLG,
/// scRGB, encoded extended sRGB, else SDR. `forced` limits it to one.
fn pick_mode(caps: &wgpu::SurfaceCapabilities, forced: Option<&str>) -> ModeChoice {
    use wgpu::{SurfaceColorSpace as Cs, SurfaceColorSpaces as Csf, TextureFormat as Tf};

    const PREFERENCES: &[(&str, Cs, Csf, u32, &[Tf])] = &[
        (
            "hdr10",
            Cs::Bt2100Pq,
            Csf::BT2100_PQ,
            2,
            &[Tf::Rgb10a2Unorm, Tf::Rgba16Float],
        ),
        (
            "hlg",
            Cs::Bt2100Hlg,
            Csf::BT2100_HLG,
            3,
            &[Tf::Rgb10a2Unorm, Tf::Rgba16Float],
        ),
        (
            "scrgb",
            Cs::ExtendedSrgbLinear,
            Csf::EXTENDED_SRGB_LINEAR,
            1,
            &[Tf::Rgba16Float],
        ),
        (
            "extended-srgb",
            Cs::ExtendedSrgb,
            Csf::EXTENDED_SRGB,
            4,
            &[Tf::Rgba16Float],
        ),
    ];

    for &(name, cs, flag, shader_mode, preferred) in PREFERENCES {
        if forced.is_some_and(|f| f != name) {
            continue;
        }
        let first = preferred
            .iter()
            .copied()
            .filter(|&f| caps.color_spaces(f).contains(flag))
            .chain(
                caps.format_capabilities
                    .iter()
                    .filter(|fc| fc.color_spaces.contains(flag))
                    .map(|fc| fc.format),
            )
            .next();
        if let Some(format) = first {
            return ModeChoice {
                format,
                color_space: cs,
                shader_mode,
            };
        }
    }
    ModeChoice {
        format: caps.formats[0],
        color_space: wgpu::SurfaceColorSpace::Auto,
        shader_mode: 0,
    }
}

struct State {
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    /// Redraws attempted; the exit counts these, so a window that never gets
    /// a frame (minimised, other workspace) still ends.
    attempts: u32,
    presented: u32,
}

impl State {
    fn new(
        window: Arc<Window>,
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
    ) -> Self {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("no device");
        let info = adapter.get_info();
        println!(
            "adapter: {} ({:?}), driver {}",
            info.name, info.backend, info.driver_info
        );

        let caps = surface.get_capabilities(&adapter);
        println!("surface formats and colour spaces:");
        for fc in &caps.format_capabilities {
            println!("  {:?}: {:?}", fc.format, fc.color_spaces);
        }
        let hdr = surface.display_hdr_info(&adapter);
        println!("display info: {hdr:?}");
        println!("tone_map_headroom: {:?}", hdr.tone_map_headroom());

        let forced = std::env::args().nth(1);
        let choice = pick_mode(&caps, forced.as_deref());
        println!(
            "configuring {:?} + {:?} (hdr: {})",
            choice.format,
            choice.color_space,
            choice.color_space.is_hdr()
        );
        window.set_title(&format!(
            "HDR spike {:?} {:?}",
            choice.format, choice.color_space
        ));

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: choice.format,
            color_space: choice.color_space,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::include_wgsl!("hdr_surface.wgsl"));
        let params: [u32; 2] = [
            choice.shader_mode,
            u32::from(choice.shader_mode == 0 && !choice.format.is_srgb()),
        ];
        let params_buffer = wgpu::util::DeviceExt::create_buffer_init(
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("params"),
                contents: params.map(u32::to_ne_bytes).as_flattened(),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hdr test pattern"),
            layout: Some(&pipeline_layout),
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
                targets: &[Some(choice.format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            window,
            device,
            queue,
            surface,
            config,
            pipeline,
            bind_group,
            attempts: 0,
            presented: 0,
        }
    }

    /// Draws one frame; false when the surface is lost and the spike must end.
    fn render(&mut self) -> bool {
        self.attempts += 1;
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return true;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                eprintln!("hdr surface: surface lost");
                return false;
            }
            _ => return true,
        };
        let view = texture.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(texture);
        self.presented += 1;
        true
    }
}

#[derive(Default)]
struct App {
    state: Option<State>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("HDR spike"))
                .expect("window"),
        );
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone()).expect("surface");
        let state = State::new(window, &instance, surface);
        state.window.request_redraw();
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.config.width = size.width.max(1);
                state.config.height = size.height.max(1);
                state.surface.configure(&state.device, &state.config);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if !state.render() || state.attempts >= FRAMES {
                    println!(
                        "hdr surface: presented {} of {} frames",
                        state.presented, state.attempts
                    );
                    event_loop.exit();
                } else {
                    state.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut App::default()).expect("run");
}
