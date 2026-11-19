//! The icon sheet (tools/fidelity/icon-sheet.mjs): every kit glyph at the design's sizes on the
//! design's ground, drawn by the shell's renderer (Slint's Skia on wgpu, offscreen) and dumped as
//! raw RGBA for the per-cell SSIM against the web sheet. A tool, not an assertion: without
//! `WUBBLE_ICON_SHEET=DIR` (or a wgpu adapter) it returns at once. It writes
//! `DIR/wubble-{1,2}x.rgba` (u32 width, u32 height, little endian, then RGBA).
//! Pattern: tests/crisp_text.rs (the offscreen platform).

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use i_slint_renderer_skia::SkiaWGPU30Renderer;
use slint::platform::{Platform, Renderer, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

slint::slint! {
    #[edition = "1.19"]
    import { IconSheet } from "tests/icon-sheet.slint";
    export { IconSheet }
}

static SCALE: AtomicU32 = AtomicU32::new(1);
const LOGICAL: (u32, u32) = (512, 896);

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
fn draw_the_icon_sheet() {
    let Some(dir) = std::env::var_os("WUBBLE_ICON_SHEET").map(PathBuf::from) else {
        return;
    };
    if !init() {
        eprintln!("icon_sheet: no wgpu adapter for Skia, skipped");
        return;
    }
    // one scale per process (the window adapter reads it at creation): 1x here, 2x by the
    // second test run of the same binary is not possible, so both are drawn by one window that
    // is told its scale factor.
    for scale in [1u32, 2] {
        SCALE.store(scale, Ordering::Relaxed);
        let sheet = IconSheet::new().unwrap();
        sheet
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale as f32,
            });
        sheet.show().unwrap();
        let shot = sheet.window().take_snapshot().unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(&shot.width().to_le_bytes());
        out.extend_from_slice(&shot.height().to_le_bytes());
        out.extend_from_slice(shot.as_bytes());
        std::fs::write(dir.join(format!("wubble-{scale}x.rgba")), out).unwrap();
        sheet.hide().unwrap();
    }
}
