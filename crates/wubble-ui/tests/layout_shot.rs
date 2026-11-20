//! The shell's layout cases drawn by its own renderer (Slint's Skia on wgpu, offscreen), for the
//! eye: a tool, not an assertion. Without `WUBBLE_LAYOUT_SHOT=DIR` (or a wgpu adapter) it returns
//! at once; with it, it writes `DIR/<name>.rgba` (u32 width, u32 height, little endian, then
//! RGBA) of the whole window: the address pill with a long address and every chip, twenty tabs,
//! menus with long rows (Slint's glass: no scene here), the sidebar mid-way through closing (to
//! hidden, then to the rail) at three points of the seam's spring, stepped by the real driver at
//! a fixed frame delta, and the crowded rail with a tab hovered. The headless gallery cannot show
//! the collapse: its seam only moves on presented frames.
//! Pattern: tests/status_bubble_shot.rs (the offscreen platform).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use i_slint_renderer_skia::SkiaWGPU30Renderer;
use slint::platform::{Platform, Renderer, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError};
use wubble_ui::driver;
use wubble_ui::{AppWindow, MenuEntry, MenuView, SidebarMode, SpaceItem, TabItem};

const SIZE: (u32, u32) = (1360, 880);
const DT: f32 = 1.0 / 120.0;

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
        PhysicalSize::new(SIZE.0, SIZE.1)
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

/// Lets Slint run its timers and tweens for `ms` of real time (the offscreen platform's clock).
fn pump(ms: u64) {
    for _ in 0..ms / 20 {
        std::thread::sleep(Duration::from_millis(20));
        slint::platform::update_timers_and_animations();
    }
}

fn shoot(app: &AppWindow, dir: &Path, name: &str) {
    let image = app.window().take_snapshot().unwrap();
    let mut out = Vec::new();
    out.extend_from_slice(&image.width().to_le_bytes());
    out.extend_from_slice(&image.height().to_le_bytes());
    out.extend_from_slice(image.as_bytes());
    std::fs::write(dir.join(format!("{name}.rgba")), out).unwrap();
}

fn tab(id: i32, title: &str) -> TabItem {
    TabItem {
        id,
        title: title.into(),
        url: "https://example.com/".into(),
        ..Default::default()
    }
}

/// A menu at `(x, y)` with these rows (label, icon, shortcut); an empty label is a separator.
fn menu(x: f32, y: f32, rows: &[(&str, &str, &str)]) -> MenuView {
    let entries: Vec<MenuEntry> = rows
        .iter()
        .map(|&(label, icon, shortcut)| MenuEntry {
            label: label.into(),
            enabled: !label.is_empty(),
            separator: label.is_empty(),
            icon: icon.into(),
            shortcut: shortcut.into(),
            danger: false,
        })
        .collect();
    MenuView {
        open: true,
        x,
        y,
        entries: slint::ModelRc::from(Rc::new(slint::VecModel::from(entries))),
        icons: true,
    }
}

#[test]
fn draw_the_layout_cases() {
    let Some(dir) = std::env::var_os("WUBBLE_LAYOUT_SHOT").map(PathBuf::from) else {
        return;
    };
    if !init() {
        eprintln!("layout_shot: no wgpu adapter for Skia, skipped");
        return;
    }
    let app = AppWindow::new().unwrap();
    let driver = driver::wire(&app, wubble_core::seam::Seam::new()).driver;
    let titles = [
        "List of longest words in the English language - Wikipedia",
        "Example Domain",
        "Settings",
        "History",
    ];
    let tabs: Vec<TabItem> = (0..20)
        .map(|i| {
            tab(
                i + 1,
                titles.get(i as usize).copied().unwrap_or("Background tab"),
            )
        })
        .collect();
    app.set_tabs(slint::ModelRc::from(Rc::new(slint::VecModel::from(tabs))));
    app.set_active_tab_id(1);
    app.set_spaces(slint::ModelRc::from(Rc::new(slint::VecModel::from(vec![
        SpaceItem {
            id: 1,
            name: "Space 1".into(),
            tint: slint::Color::from_rgb_u8(0x7c, 0xc8, 0xff),
            icon: "personal".into(),
            ..Default::default()
        },
        SpaceItem {
            id: 2,
            name: "Work".into(),
            tint: slint::Color::from_rgb_u8(0xff, 0xb8, 0x6b),
            icon: "work".into(),
            ..Default::default()
        },
    ]))));
    app.set_active_space_id(1);
    // as the host does: the pill first shows the bundled page's address, the long one later
    app.set_address_host("data".into());
    app.set_address_secure(true);
    app.set_address_bookmarkable(true);
    app.set_address_blocker_visible(true);
    app.set_address_blocked(14);
    app.set_reader_available(true);
    app.show().unwrap();
    let step = |n: usize| {
        for _ in 0..n {
            driver.borrow_mut().advance(&app, DT);
        }
    };
    step(600);
    pump(100);
    let _ = app.window().take_snapshot();
    app.set_address_host("en.wikipedia.org".into());
    app.set_address_path("/wiki/List_of_longest_words_in_the_English_language".into());
    pump(300);
    shoot(&app, &dir, "pill-and-crowd");

    // menus whose rows name a Space and a site: the panel fits its widest row, up to its
    // maximum, then a row is cut on one line
    for (name, view) in [
        (
            "menu-tab-rows",
            menu(
                120.0,
                150.0,
                &[
                    ("Reload", "", "Ctrl+R"),
                    ("Pin Tab", "pin", "Ctrl+Shift+D"),
                    ("New Folder with Tab", "", ""),
                    ("", "", ""),
                    ("Never Sleep", "awake", ""),
                    ("Never Sleep Tabs in This Space", "awake", ""),
                    ("Never Sleep en.wikipedia.org", "awake", ""),
                    ("Never Sleep a-very-lon\u{2026}ample.com", "awake", ""),
                ],
            ),
        ),
        (
            "menu-site-long-host",
            menu(
                190.0,
                92.0,
                &[
                    ("Blocked 14 requests", "", ""),
                    ("Turn off blocking on www.youtube.com", "shield", ""),
                    ("Page theme on a-very-lon\u{2026}ample.com", "", ""),
                    (
                        "A row far too long for any menu to show whole, cut on one line",
                        "",
                        "",
                    ),
                ],
            ),
        ),
        (
            "menu-sidebar",
            menu(
                90.0,
                820.0,
                &[
                    ("New Tab", "", "Ctrl+T"),
                    ("New Space", "", "Ctrl+Shift+S"),
                    ("New Folder", "", ""),
                ],
            ),
        ),
    ] {
        app.set_menu(view);
        pump(600);
        shoot(&app, &dir, name);
        app.set_menu(MenuView::default());
        pump(400);
    }

    // the sidebar closing, then closing to the rail: three points of the seam's way each
    for (mode, name) in [
        (SidebarMode::Hidden, "collapse-hidden"),
        (SidebarMode::Rail, "collapse-rail"),
    ] {
        app.invoke_sidebar_mode_requested(SidebarMode::Extended);
        step(600);
        pump(100);
        let from = app.get_sidebar_width();
        app.invoke_sidebar_mode_requested(mode);
        let to = if mode == SidebarMode::Hidden {
            0.0
        } else {
            wubble_core::seam::SEAM::rail as f32
        };
        for (k, at) in [0.25f32, 0.5, 0.75].into_iter().enumerate() {
            let mark = from + (to - from) * at;
            for _ in 0..600 {
                if app.get_sidebar_width() <= mark {
                    break;
                }
                step(1);
            }
            slint::platform::update_timers_and_animations();
            shoot(&app, &dir, &format!("{name}-{k}"));
            eprintln!(
                "{name}-{k}: width {} contents {} opacity {} rail {}",
                app.get_sidebar_width(),
                app.get_sidebar_contents(),
                app.get_sidebar_opacity(),
                app.get_sidebar_rail()
            );
        }
        step(600);
    }
    // the crowded rail, its first tab hovered: the close badge on the icon's corner
    app.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(36.0, 122.0),
        });
    step(60);
    pump(400);
    shoot(&app, &dir, "rail-crowd-hover");
}
