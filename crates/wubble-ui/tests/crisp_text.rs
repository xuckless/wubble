//! Text drawn by the shell's renderer (Slint's femtovg on wgpu, offscreen) keeps every glyph
//! whole and on one baseline: a centred text in an odd-sized box at a fractional position (as
//! a segmented control's label, a key cap or a settings row in the scroller can be) draws each
//! stem of "l l l …" with the same ink over the same rows. Before
//! the vendored renderer's change (vendor/i-slint-renderer-femtovg/WUBBLE_CHANGES.md) the text's
//! origin landed on a half pixel and femtovg's nearest-sampled glyph atlas dropped or doubled
//! columns ('Full' drawn as 'Fuli'). Needs a GPU adapter; without one the test returns early.
//! Pattern: the renderer crate's own `tests/image_source.rs` (offscreen platform).

use std::rc::Rc;

use i_slint_renderer_femtovg::FemtoVGWGPURenderer;
use slint::ComponentHandle;
use slint::platform::{Platform, Renderer, WindowAdapter};
use slint::{PhysicalSize, PlatformError};

slint::slint! {
    #[edition = "1.19"]
    import "ui/fonts/Geist-Variable.ttf";
    export component Crisp inherits Window {
        width: 200px;
        height: 160px;
        background: black;
        // Odd-sized boxes, centred both ways, at the fractional positions layout gives (a card at
        // x 513.04 in a column at 347.48, a scroller at 2181.9): the centre and the origin are
        // near half pixels, where a nearest-sampled glyph lands on either side.
        for at[i] in [
            { x: 10.52px, y: 2.45px },
            { x: 10.48px, y: 22.9px },
            { x: 11.02px, y: 42.5px },
            { x: 10.5px, y: 62.55px },
            { x: 10.73px, y: 82.2px },
            { x: 10.25px, y: 102.52px },
            { x: 10px, y: 122.5px },
        ]: Text {
            x: at.x;
            y: at.y;
            width: 121px;
            height: 17px;
            text: "l l l l l l l l l l l l";
            color: white;
            font-family: "Geist";
            font-size: 13px;
            font-weight: 500;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
    }
}

const SIZE: (u32, u32) = (200, 160);
const ROWS: u32 = 7;

fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut ctx) {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    }
}

struct Offscreen {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let renderer = FemtoVGWGPURenderer::new(
            self.instance.clone(),
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
    renderer: FemtoVGWGPURenderer,
}

impl WindowAdapter for OffscreenWindow {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        PhysicalSize::new(SIZE.0, SIZE.1)
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

/// The offscreen platform, or `false` when there is no wgpu adapter.
fn init() -> bool {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Some(Ok(adapter)) =
        poll_once(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        return false;
    };
    let Some(Ok((device, queue))) =
        poll_once(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        return false;
    };
    slint::platform::set_platform(Box::new(Offscreen {
        instance,
        device,
        queue,
    }))
    .is_ok()
}

/// One glyph: its ink and the first and last rows it inks.
#[derive(Debug)]
struct Stem {
    ink: u32,
    top: usize,
    bottom: usize,
}

/// The stems in rows `top..top + 20`: runs of columns with ink, split at empty ones.
fn stems(shot: &slint::SharedPixelBuffer<slint::Rgba8Pixel>, top: usize) -> Vec<Stem> {
    let width = shot.width() as usize;
    let pixels = shot.as_slice();
    let ink = |x: usize, y: usize| u32::from(pixels[y * width + x].r);
    let rows = top..top + 20;
    let mut stems = Vec::new();
    let mut current: Option<Stem> = None;
    for x in 0..width {
        let inked: Vec<usize> = rows.clone().filter(|&y| ink(x, y) > 0).collect();
        match (inked.first(), inked.last(), current.as_mut()) {
            (Some(&t), Some(&b), Some(stem)) => {
                stem.ink += rows.clone().map(|y| ink(x, y)).sum::<u32>();
                stem.top = stem.top.min(t);
                stem.bottom = stem.bottom.max(b);
            }
            (Some(&t), Some(&b), None) => {
                current = Some(Stem {
                    ink: rows.clone().map(|y| ink(x, y)).sum(),
                    top: t,
                    bottom: b,
                });
            }
            _ => stems.extend(current.take()),
        }
    }
    stems.extend(current);
    stems
}

#[test]
fn centred_text_keeps_every_glyph_whole() {
    if !init() {
        eprintln!("crisp_text: no wgpu adapter, skipped");
        return;
    }
    let window = Crisp::new().unwrap();
    window.show().unwrap();
    let shot = window.window().take_snapshot().unwrap();
    for row in 0..ROWS {
        let stems = stems(&shot, row as usize * 20);
        assert_eq!(stems.len(), 12, "row {row}: one stem per 'l': {stems:?}");
        let inks = stems.iter().map(|s| s.ink);
        let (min, max) = (inks.clone().min().unwrap(), inks.max().unwrap());
        // Subpixel variants of one glyph differ by antialiasing only; a dropped or doubled
        // column changes a stem's ink by a third or more.
        assert!(
            max * 100 <= min * 115,
            "row {row}: stems differ in ink (a glyph column dropped or doubled): {stems:?}"
        );
        // One baseline: every 'l' spans the same rows (no glyph a pixel up or down).
        assert!(
            stems
                .iter()
                .all(|s| (s.top, s.bottom) == (stems[0].top, stems[0].bottom)),
            "row {row}: glyphs at different heights: {stems:?}"
        );
    }
}
