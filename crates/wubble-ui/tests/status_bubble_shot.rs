//! The link status bubble drawn by the shell's renderer (Slint's Skia on wgpu, offscreen), for
//! the eye: a tool, not an assertion. Without `WUBBLE_BUBBLE_SHOT=DIR` (or a wgpu adapter) it
//! returns at once; with it, it writes `DIR/status-bubble-{1,2}x.rgba` (u32 width, u32 height,
//! little endian, then RGBA) of a short and a long address over a dark and a light page.
//! Pattern: tests/icon_sheet.rs (the offscreen platform).

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use i_slint_renderer_skia::SkiaWGPU30Renderer;
use slint::platform::{Platform, Renderer, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

slint::slint! {
    #[edition = "1.19"]
    import "ui/fonts/Geist-Variable.ttf";
    import { AStatusBubble } from "ui/kit/overlay/a/a-status-bubble.slint";
    component Pane inherits Rectangle {
        in property <string> host;
        in property <string> rest;
        in property <bool> shown;
        clip: true;
        AStatusBubble {
            shown: root.shown;
            host: root.host;
            rest: root.rest;
            pane-width: root.width;
            pane-height: root.height;
        }
    }
    export component BubbleShot inherits Window {
        in property <bool> shown;
        width: 600px;
        height: 240px;
        background: #0d1117;
        Pane { x: 0; y: 0; width: 600px; height: 60px; background: #0d1117; shown: root.shown; host: "github.com"; rest: "/xuckless"; }
        Pane { x: 0; y: 60px; width: 600px; height: 60px; background: #0d1117; shown: root.shown; host: "github.com"; rest: "/xuckless/the-wallpaper-project/blob/main/src/wubble/pages/settings.tsx"; }
        Pane { x: 0; y: 120px; width: 600px; height: 60px; background: #ffffff; shown: root.shown; host: "github.com"; rest: "/xuckless/the-wallpaper-project/blob/main/src/wubble/pages/settings.tsx"; }
        Pane { x: 0; y: 180px; width: 600px; height: 60px; background: #ffffff; shown: root.shown; host: "a-very-long-host-name-that-does-not-fit.example.com"; rest: "/x"; }
    }
}

static SCALE: AtomicU32 = AtomicU32::new(1);
const LOGICAL: (u32, u32) = (600, 240);

fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut ctx) {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    }
}

struct Offscreen {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let renderer = SkiaWGPU30Renderer::new(
            self.instance.clone(),
            self.adapter.clone(),
            self.device.clone(),
            self.queue.clone(),
        )?;
        Ok(Rc::new_cyclic(|weak: &std::rc::Weak<OffscreenWindow>| {
            OffscreenWindow {
                window: slint::Window::new(weak.clone() as _),
                renderer,
            }
        }))
    }
}

struct OffscreenWindow {
    window: slint::Window,
    renderer: SkiaWGPU30Renderer,
}

impl WindowAdapter for OffscreenWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        let s = SCALE.load(Ordering::Relaxed);
        PhysicalSize::new(LOGICAL.0 * s, LOGICAL.1 * s)
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

fn init() -> bool {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Some(Ok(adapter)) =
        poll_once(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        return false;
    };
    if adapter.get_info().backend == wgpu::Backend::Gl {
        return false;
    }
    let Some(Ok((device, queue))) =
        poll_once(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        return false;
    };
    slint::platform::set_platform(Box::new(Offscreen {
        instance,
        adapter,
        device,
        queue,
    }))
    .is_ok()
}

#[test]
fn draw_the_status_bubble() {
    let Some(dir) = std::env::var_os("WUBBLE_BUBBLE_SHOT").map(PathBuf::from) else {
        return;
    };
    if !init() {
        eprintln!("status_bubble_shot: no wgpu adapter for Skia, skipped");
        return;
    }
    for scale in [1u32, 2] {
        SCALE.store(scale, Ordering::Relaxed);
        let shot = BubbleShot::new().unwrap();
        shot.window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale as f32,
            });
        shot.show().unwrap();
        shot.set_shown(true);
        // past the tooltip motion's entrance (real time: the offscreen platform's clock)
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(20));
            slint::platform::update_timers_and_animations();
        }
        let image = shot.window().take_snapshot().unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(&image.width().to_le_bytes());
        out.extend_from_slice(&image.height().to_le_bytes());
        out.extend_from_slice(image.as_bytes());
        std::fs::write(dir.join(format!("status-bubble-{scale}x.rgba")), out).unwrap();
        shot.hide().unwrap();
    }
}
