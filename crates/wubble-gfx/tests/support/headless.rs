//! A fixed scene rendered on a real wgpu device with no window, shared by the
//! `scene_headless` test and the `headless` example.
//! Source: the-wallpaper-project `src/wubble/slint/crate/examples/headless.rs`
//! (owner's code), without the bubble, minis, pop and voice glow.

use std::future::Future;
use std::task::{Context, Poll, Waker};

use wubble_gfx::scene::{BeamRect, GlassRect, Quality, Rect, Renderer, SceneOptions, Tier};

/// Logical view size and scale of every run.
pub const VIEW: (f32, f32, f32) = (320.0, 200.0, 2.0);
/// The thick panel; nothing else is drawn near the top right corner.
pub const THICK: Rect = Rect::new(16.0, 16.0, 100.0, 80.0);

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

/// Any adapter (hardware or a CPU rasteriser), or `None` when there is none.
pub fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::default();
    let adapter =
        block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    let name = adapter.get_info().name;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("an adapter was found, so a default device must be created");
    device.on_uncaptured_error(std::sync::Arc::new(|e| panic!("wgpu error: {e}")));
    Some((device, queue, name))
}

/// What a run draws.
#[derive(Clone, Copy)]
pub struct Run {
    pub glass: bool,
    pub quality: Quality,
    pub frames: u32,
}

/// Renders `run` at [`VIEW`], 1/60 s per frame, and reads the output back:
/// (width, height, tightly packed RGBA).
pub fn render(device: &wgpu::Device, queue: &wgpu::Queue, run: Run) -> (u32, u32, Vec<u8>) {
    let mut r = Renderer::from_device(device.clone(), queue.clone(), SceneOptions::default());
    r.set_quality(run.quality);
    r.scene.pointer_move(150.0, 90.0);
    let (w, h, dpr) = VIEW;
    let dt = 1.0 / 60.0;
    for i in 0..run.frames {
        if i == 1 {
            // after the first frame seeded the liquid, so the beam's random hue
            // leaves the liquid the same with and without glass
            if run.glass {
                let panel = |rect, radius, tier, hover| GlassRect {
                    rect,
                    radius,
                    tier,
                    pressed: false,
                    hover,
                    focus: false,
                };
                let bar = Rect::new(16.0, 110.0, 100.0, 30.0);
                r.scene.set_glass(&[
                    panel(THICK, 20.0, Tier::Thick, false),
                    panel(bar, 15.0, Tier::Thin, true),
                ]);
                r.scene.set_beam(Some(BeamRect {
                    rect: bar,
                    radius: 15.0,
                }));
                r.scene.lap_beam();
            }
            // a ripple through the liquid and a press wave through the thick glass
            r.scene.tap(60.0, 50.0);
        }
        r.scene.pointer_move(150.0 + i as f32, 90.0);
        r.render_under_ui(w, h, dpr, dt);
    }
    let tex = r.output().expect("rendered at least one frame");
    (
        tex.width(),
        tex.height(),
        wubble_gfx::scene::readback_rgba8(device, queue, tex),
    )
}
