//! Overlays B as the shell draws them (Slint's Skia on wgpu, offscreen), one picture per state,
//! for review by eye: the surfaces that need a live page in the app (the video strip and its
//! speed menu, the picture-in-picture strip and card, reader controls, the Boost editor, the
//! screenshot picker, the swipe chip, the autoscroll pointer) are put on the shell from
//! properties here. A review tool, run by hand
//! (`cargo run -p wubble-ui --example overlay_b_shots -- DIR`); without a wgpu adapter it
//! returns at once. It writes `DIR/NN-name.rgba` (u32 width, u32 height, little
//! endian, then RGBA) at 1x. The clock is the tool's own: each state is shown, the clock moved
//! on until its motion settled, then drawn (no wall clock). The glass is Slint's fallback (no
//! scene under it). Pattern: tests/icon_sheet.rs (the offscreen platform).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_renderer_skia::SkiaWGPU30Renderer;
use slint::platform::{Platform, PointerEventButton, Renderer, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, PlatformError, VecModel};
use wubble_ui::{AppWindow, BoostView, VideoMark, VideoSave, VideoView};

const LOGICAL: (u32, u32) = (1360, 880);

thread_local! {
    static NOW: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

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
    fn duration_since_start(&self) -> Duration {
        NOW.with(Cell::get)
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
        PhysicalSize::new(LOGICAL.0, LOGICAL.1)
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

/// Moves the test's clock on by `ms` in frames of 8 ms, stepping the controls registry too.
fn settle(controls: &std::cell::RefCell<wubble_ui::controls::Controls>, base: Instant, ms: u64) {
    for _ in 0..ms / 8 {
        let now = NOW.with(|n| {
            let t = n.get() + Duration::from_millis(8);
            n.set(t);
            t
        });
        controls.borrow_mut().frame(base + now);
        slint::platform::update_timers_and_animations();
    }
}

fn shoot(app: &AppWindow, dir: &Path, name: &str) {
    let shot = app.window().take_snapshot().unwrap();
    let mut out = Vec::new();
    out.extend_from_slice(&shot.width().to_le_bytes());
    out.extend_from_slice(&shot.height().to_le_bytes());
    out.extend_from_slice(shot.as_bytes());
    std::fs::write(dir.join(format!("{name}.rgba")), out).unwrap();
}

fn video() -> VideoView {
    VideoView {
        open: true,
        x: 120.0,
        y: 60.0,
        w: 720.0,
        h: 405.0,
        pane_right: 1080.0,
        pane_bottom: 860.0,
        rate: "1.5\u{d7}".into(),
        rate_index: 4,
        progress: 0.42,
        looped: true,
        can_pip: true,
        save: VideoSave::Offered,
        save_fraction: -1.0,
        save_label: "Save video".into(),
        marks: ModelRc::new(VecModel::from(vec![VideoMark {
            from: 0.6,
            to: 0.7,
            colour: slint::Color::from_rgb_u8(0x8A, 0xF2, 0xD0),
        }])),
        ..VideoView::default()
    }
}

fn main() {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: overlay_b_shots DIR");
        std::process::exit(2);
    };
    if !init() {
        eprintln!("overlay_b_shots: no wgpu adapter for Skia, skipped");
        return;
    }
    let base = Instant::now();
    let app = AppWindow::new().unwrap();
    let controls = wubble_ui::controls::wire(&app);
    app.show().unwrap();
    let rates: Vec<slint::SharedString> = ["0.25", "0.5", "0.75", "1", "1.5", "2", "3", "4"]
        .iter()
        .map(|r| format!("{r}\u{d7}").into())
        .collect();
    app.set_video_rates(ModelRc::new(VecModel::from(rates)));
    let step = |ms| settle(&controls, base, ms);
    let pointer = |x: f32, y: f32| {
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(x, y),
        })
    };

    // the video strip (the pointer on its frame button), then its speed menu, then collapsed
    // to its tab
    app.set_video(video());
    step(600);
    if let Some(frame) =
        i_slint_backend_testing::ElementHandle::find_by_element_id(&app, "VideoControls::frame")
            .next()
    {
        let (p, size) = (frame.absolute_position(), frame.size());
        pointer(p.x + size.width / 2.0, p.y + size.height / 2.0);
    }
    step(400);
    shoot(&app, &dir, "01-video-strip");
    app.set_video(VideoView {
        speeds: true,
        ..video()
    });
    step(600);
    shoot(&app, &dir, "02-video-speeds");
    app.set_video(VideoView {
        open: false,
        collapsed: true,
        ..video()
    });
    step(600);
    shoot(&app, &dir, "03-video-tab");
    app.set_video(VideoView::default());

    // the picture-in-picture card
    app.set_pip_title("A video".into());
    app.set_pip_card(true);
    step(800);
    shoot(&app, &dir, "04-pip-card");
    app.set_pip_card(false);

    // reader controls
    app.set_shot_pane_x(0.0);
    app.set_shot_pane_width(1084.0);
    app.set_reader_size(18);
    app.set_reader_on(true);
    step(600);
    shoot(&app, &dir, "05-reader-controls");
    app.set_reader_on(false);

    // the Boost editor
    app.set_boost(BoostView {
        open: true,
        serial: 1,
        name: "Bigger text".into(),
        site: "example.com".into(),
        css: "body { font-size: 18px }".into(),
        tint_on: true,
        tint: slint::Color::from_rgb_u8(0xB8, 0xA6, 0xFF),
        strength: 30.0,
        enabled: true,
        error: "".into(),
        ..BoostView::default()
    });
    step(800);
    shoot(&app, &dir, "06-boost-panel");
    app.set_boost(BoostView::default());
    step(600);

    // the screenshot picker at rest, then dragging
    app.set_shot_picking(true);
    step(600);
    shoot(&app, &dir, "07-shot-rest");
    let (x0, y0) = (500.0, 200.0);
    pointer(x0, y0);
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(x0, y0),
        button: PointerEventButton::Left,
    });
    pointer(820.0, 420.0);
    step(600);
    shoot(&app, &dir, "08-shot-drag");
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: LogicalPosition::new(820.0, 420.0),
        button: PointerEventButton::Left,
    });
    app.set_shot_picking(false);
    step(600);

    // the swipe chip pulled half way, then ready
    app.set_swipe_shift(60.0);
    app.set_swipe_progress(0.5);
    step(300);
    shoot(&app, &dir, "09-swipe-pull");
    app.set_swipe_progress(1.0);
    app.set_swipe_shift(80.0);
    step(600);
    shoot(&app, &dir, "10-swipe-ready");
    app.set_swipe_progress(0.0);

    // the autoscroll pointer still, then moving
    app.set_autoscroll_anchor_x(700.0);
    app.set_autoscroll_anchor_y(400.0);
    app.set_autoscroll_x(700.0);
    app.set_autoscroll_y(400.0);
    app.set_autoscroll_on(true);
    step(100);
    shoot(&app, &dir, "11-autoscroll-still");
    app.set_autoscroll_x(700.0);
    app.set_autoscroll_y(520.0);
    app.set_autoscroll_moving(true);
    app.set_autoscroll_stretch(1.3);
    app.set_autoscroll_squash(1.0 / 1.3_f32.sqrt());
    step(100);
    shoot(&app, &dir, "12-autoscroll-moving");
    app.set_autoscroll_on(false);

    // the strip's save button: hovered, saving (40 %), saved; then the strip on a 320 x 240
    // player (the video smoke's) and a 240 x 135 one
    let hover = |id: &str| {
        let e = i_slint_backend_testing::ElementHandle::find_by_element_id(
            &app,
            &format!("VideoControls::{id}"),
        )
        .next();
        if let Some(e) = e {
            let (p, size) = (e.absolute_position(), e.size());
            pointer(p.x + size.width / 2.0, p.y + size.height / 2.0);
        }
    };
    app.set_video(video());
    step(600);
    hover("download");
    step(400);
    shoot(&app, &dir, "13-video-save-hover");
    pointer(10.0, 10.0);
    app.set_video(VideoView {
        save: VideoSave::Saving,
        save_fraction: 0.4,
        ..video()
    });
    step(600);
    shoot(&app, &dir, "14-video-saving");
    app.set_video(VideoView {
        save: VideoSave::Saved,
        ..video()
    });
    step(600);
    shoot(&app, &dir, "15-video-saved");
    for (name, w, h) in [
        ("16-video-small", 320.0, 240.0),
        ("17-video-tiny", 240.0, 135.0),
    ] {
        app.set_video(VideoView { w, h, ..video() });
        step(600);
        shoot(&app, &dir, name);
    }
    app.set_video(VideoView::default());
    app.hide().unwrap();
}
