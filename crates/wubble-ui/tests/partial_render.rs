//! Partial rendering in the shell's renderer (Slint's Skia on wgpu, vendored: see
//! `vendor/i-slint-renderer-skia/WUBBLE_CHANGES.md`), checked by state, never by time: a new
//! page frame alone redraws its pane and none of the sidebar's items; a change in one row
//! redraws that row; a frame with nothing changed draws nothing; a page hole clears its pane;
//! and every partial frame holds the same pixels as the same state drawn whole. Rendered offscreen into a kept texture
//! (`SkiaWGPU30Renderer::render_to_kept_texture`, the path the window's surface takes).
//! Needs a GPU adapter; without one the test returns early.
//! Pattern: `crisp_text.rs` (offscreen platform).

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_renderer_skia::SkiaWGPU30Renderer;
use slint::ComponentHandle;
use slint::platform::{Platform, Renderer, WindowAdapter};
use slint::{PhysicalSize, PlatformError};

slint::slint! {
    #[edition = "1.19"]
    export component Shell inherits Window {
        width: 400px;
        height: 300px;
        background: #202124;
        in property <image> frame;
        in property <color> row-ink: #e8eaed;
        // the sidebar: rows of a pill, a label and a dot, like the tab list
        for i in 12: Rectangle {
            x: 8px;
            y: 8px + i * 24px;
            width: 104px;
            height: 20px;
            border-radius: 6px;
            background: #303134;
            Text {
                x: 8px;
                text: "Tab " + i;
                color: i == 3 ? row-ink : #e8eaed;
                font-size: 11px;
                vertical-alignment: center;
            }
            Rectangle {
                x: 88px;
                y: 6px;
                width: 8px;
                height: 8px;
                border-radius: 4px;
                background: #8ab4f8;
            }
        }
        // the page card and its frame
        Rectangle {
            x: 128px;
            y: 8px;
            width: 264px;
            height: 284px;
            border-radius: 10px;
            clip: true;
            background: white;
            Image {
                width: parent.width;
                height: parent.height;
                source: frame;
                image-fit: fill;
            }
        }
    }
}

const SIZE: (u32, u32) = (400, 300);
/// Items the sidebar draws: three per row.
const SIDEBAR_ITEMS: u32 = 12 * 3;

fn poll_once<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut ctx = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut ctx) {
        std::task::Poll::Ready(result) => Some(result),
        std::task::Poll::Pending => None,
    }
}

#[derive(Clone)]
struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

thread_local! {
    /// The windows made, in order.
    static WINDOWS: RefCell<Vec<Rc<OffscreenWindow>>> = const { RefCell::new(Vec::new()) };
}

struct Offscreen(Gpu);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let g = &self.0;
        let renderer = SkiaWGPU30Renderer::new(
            g.instance.clone(),
            g.adapter.clone(),
            g.device.clone(),
            g.queue.clone(),
        )?;
        let window = Rc::new_cyclic(|weak: &std::rc::Weak<OffscreenWindow>| OffscreenWindow {
            window: slint::Window::new(weak.clone() as _),
            renderer,
        });
        WINDOWS.with(|w| w.borrow_mut().push(window.clone()));
        Ok(window)
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
        PhysicalSize::new(SIZE.0, SIZE.1)
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
}

/// The offscreen platform's device, or `None` when there is no wgpu adapter Skia can draw on.
fn init() -> Option<Gpu> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Some(Ok(adapter)) =
        poll_once(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        return None;
    };
    if adapter.get_info().backend == wgpu::Backend::Gl {
        return None;
    }
    let Some(Ok((device, queue))) =
        poll_once(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        return None;
    };
    let gpu = Gpu {
        instance,
        adapter,
        device,
        queue,
    };
    slint::platform::set_platform(Box::new(Offscreen(gpu.clone()))).ok()?;
    Some(gpu)
}

fn texture(gpu: &Gpu, size: (u32, u32), usage: wgpu::TextureUsages) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("partial_render"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    })
}

/// A page frame: a texture of stripes in `rgb`, as the frame ring hands Slint.
fn frame(gpu: &Gpu, rgb: [u8; 3]) -> slint::Image {
    let (w, h) = (64u32, 64u32);
    let t = texture(
        gpu,
        (w, h),
        wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
    );
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for _ in 0..w {
            let k = if (y / 8) % 2 == 0 { 1 } else { 2 };
            px.extend_from_slice(&[rgb[0] / k, rgb[1] / k, rgb[2] / k, 255]);
        }
    }
    gpu.queue.write_texture(
        t.as_image_copy(),
        &px,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    slint::Image::try_from(t).unwrap()
}

/// What Skia draws into: the kept frame, or a fresh one for a whole redraw. Cleared through
/// wgpu first: wgpu does not see Skia's writes, and would clear a texture it thinks was never
/// written before copying from it (the readback here, as the window's copy to the swapchain).
fn target(gpu: &Gpu) -> wgpu::Texture {
    let t = texture(
        gpu,
        SIZE,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let view = t.create_view(&Default::default());
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("partial_render init"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    gpu.queue.submit(Some(encoder.finish()));
    t
}

fn read(gpu: &Gpu, t: &wgpu::Texture) -> Vec<u8> {
    let row = (SIZE.0 * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("partial_render readback"),
        size: u64::from(row * SIZE.1),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        t.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(SIZE.1),
            },
        },
        wgpu::Extent3d {
            width: SIZE.0,
            height: SIZE.1,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = slice.get_mapped_range().unwrap();
    (0..SIZE.1 as usize)
        .flat_map(|y| mapped[y * row as usize..][..SIZE.0 as usize * 4].to_vec())
        .collect()
}

/// Pixels whose channels differ by more than 2.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 2))
        .count()
}

#[test]
fn a_page_frame_redraws_its_pane_and_nothing_of_the_sidebar() {
    let Some(gpu) = init() else {
        eprintln!("partial_render: no wgpu adapter for Skia, skipped");
        return;
    };
    let shell = Shell::new().unwrap();
    shell.show().unwrap();
    // a second window in the same state, drawn whole into a fresh texture each time: what the
    // kept texture must hold
    let reference = Shell::new().unwrap();
    reference.show().unwrap();
    let [window, other] = WINDOWS.with(|w| [w.borrow()[0].clone(), w.borrow()[1].clone()]);
    let kept = target(&gpu);
    let whole = |gpu: &Gpu| {
        reference.set_frame(shell.get_frame());
        reference.set_row_ink(shell.get_row_ink());
        let fresh = target(gpu);
        other.renderer.render_to_texture(&fresh).unwrap();
        read(gpu, &fresh)
    };

    shell.set_frame(frame(&gpu, [200, 40, 40]));
    window.renderer.render_to_kept_texture(&kept).unwrap();
    let first = window.renderer.last_frame();
    assert_eq!(first.dirty_share, 1.0, "the first frame is drawn whole");
    assert!(first.items_drawn > SIDEBAR_ITEMS, "{first:?}");
    assert_eq!(differing(&read(&gpu, &kept), &whole(&gpu)), 0);

    // a new page frame alone: the pane, and nothing of the sidebar
    shell.set_frame(frame(&gpu, [40, 160, 40]));
    window.renderer.render_to_kept_texture(&kept).unwrap();
    let page = window.renderer.last_frame();
    let pane_share = (264.0 * 284.0) / (400.0 * 300.0);
    assert!(
        (page.dirty_share - pane_share).abs() < 0.02,
        "a page frame redraws its pane ({pane_share:.3} of the window): {page:?}"
    );
    // the window background, the card, the image: nothing of the sidebar's 36
    assert!(
        page.items_drawn <= 3,
        "a page frame draws none of the sidebar's items: {page:?}"
    );
    let shown = read(&gpu, &kept);
    assert_eq!(
        differing(&shown, &whole(&gpu)),
        0,
        "a partial frame holds what a whole redraw does"
    );

    // nothing changed: nothing drawn
    window.renderer.render_to_kept_texture(&kept).unwrap();
    let still = window.renderer.last_frame();
    assert_eq!(
        (still.dirty_share, still.items_drawn),
        (0.0, 0),
        "{still:?}"
    );
    assert_eq!(differing(&read(&gpu, &kept), &shown), 0);

    // one row's ink: that row, not the pane
    shell.set_row_ink(slint::Color::from_rgb_u8(255, 128, 0));
    window.renderer.render_to_kept_texture(&kept).unwrap();
    let row = window.renderer.last_frame();
    assert!(
        row.dirty_share > 0.0 && row.dirty_share < 0.01,
        "a row's change redraws about that row: {row:?}"
    );
    assert!(row.items_drawn < 6, "{row:?}");
    assert_eq!(differing(&read(&gpu, &kept), &whole(&gpu)), 0);

    // the page hole (vendored change 3): the pane cleared to transparent inside its rounded
    // clip, where the page's OS layer shows, the window around it as it was
    shell.set_frame(i_slint_renderer_skia::hole::image());
    window.renderer.render_to_kept_texture(&kept).unwrap();
    let holed = read(&gpu, &kept);
    let at = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * SIZE.0 as usize + x) * 4;
        [holed[i], holed[i + 1], holed[i + 2], holed[i + 3]]
    };
    let near = |p: [u8; 4], q: [u8; 4]| p.iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 2);
    let ground = [0x20, 0x21, 0x24, 0xff];
    #[rustfmt::skip]
    let points: &[(&str, (usize, usize), [u8; 4])] = &[
        ("the pane's middle is a hole", (260, 150), [0; 4]),
        ("to its left edge", (129, 150), [0; 4]),
        ("to its bottom edge", (260, 290), [0; 4]),
        ("outside its rounded corner the window stays", (128, 8), ground),
        ("the window's ground", (4, 4), ground),
    ];
    for &(name, (x, y), want) in points {
        assert!(near(at(x, y), want), "{name}: {:?}", at(x, y));
    }
    assert_eq!(
        differing(&holed, &whole(&gpu)),
        0,
        "a partial frame with a hole holds what a whole redraw does"
    );
}
