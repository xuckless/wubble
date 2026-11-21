//! The scene's glass under an overlay follows the overlay's entrance: mid-way through the menu's
//! grow (`OverlapLook`, scaled about the point it grows from), the glass rect the frame delivers
//! is where Slint draws the panel. The expected rect is worked out from the panel's rect at
//! rest, the point it grows from and the scale (the CSS transform: `x0 + origin * (1 - zoom)`),
//! not from Slint's own positions. Testing backend, mock time; one test (one platform per
//! process).

use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};
use wubble_ui::driver::{self, Wired};
use wubble_ui::{AppWindow, GlassItem, GlassKeys, GlassLayer, MenuEntry, MenuView};

const W: f32 = 1280.0;
const H: f32 = 800.0;

fn menu(open: bool, x: f32, y: f32) -> MenuView {
    let entries: Vec<MenuEntry> = ["Back", "Forward", "Reload", "Save page as"]
        .into_iter()
        .map(|label| MenuEntry {
            label: label.into(),
            enabled: true,
            ..MenuEntry::default()
        })
        .collect();
    MenuView {
        open,
        x,
        y,
        entries: ModelRc::from(Rc::new(VecModel::from(entries))),
        icons: false,
    }
}

/// The menu's glass after one frame `ms` of mock time later.
fn frame(app: &AppWindow, wired: &Wired, now: &mut Instant, ms: u64) -> Option<GlassItem> {
    mock_elapsed_time(Duration::from_millis(ms));
    *now += Duration::from_millis(ms);
    let (_, glass) = wired.frame(app, *now);
    let key = app.global::<GlassKeys>().get_menu();
    glass?.into_iter().find(|g| g.key == key && g.present)
}

/// Opens the menu at `at`, records its glass frame by frame through the entrance, and checks
/// every frame mid-grow against the rect worked out from the rest rect.
fn grows_from(app: &AppWindow, wired: &Wired, now: &mut Instant, at: (f32, f32)) {
    app.set_menu(menu(true, at.0, at.1));
    let mut seen = vec![];
    for _ in 0..40 {
        if let Some(g) = frame(app, wired, now, 8) {
            seen.push(g);
        }
    }
    let rest = seen.last().cloned().expect("the menu's glass at rest");
    let (ox, oy) = (
        (at.0 - rest.x).clamp(0.0, rest.width),
        (at.1 - rest.y).clamp(0.0, rest.height),
    );
    let mid: Vec<&GlassItem> = seen.iter().filter(|g| g.width < rest.width - 1.0).collect();
    assert!(!mid.is_empty(), "{at:?}: no frame caught mid-entrance");
    for g in mid {
        let zoom = g.width / rest.width;
        let want = (rest.x + ox * (1.0 - zoom), rest.y + oy * (1.0 - zoom));
        assert!(
            (g.x - want.0).abs() < 0.5 && (g.y - want.1).abs() < 0.5,
            "{at:?}: at zoom {zoom} the glass is at ({}, {}), the panel at {want:?} (rest {rest:?})",
            g.x,
            g.y
        );
        assert!(
            (g.height - rest.height * zoom).abs() < 0.5,
            "{at:?}: height {} at zoom {zoom}",
            g.height
        );
    }
    app.set_menu(menu(false, at.0, at.1));
    for _ in 0..60 {
        frame(app, wired, now, 8);
    }
}

#[test]
fn menu_glass_tracks_its_entrance() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(W, H));
    app.global::<GlassLayer>().set_scene_active(true);
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    let mut now = Instant::now();
    for _ in 0..30 {
        frame(&app, &wired, &mut now, 16);
    }
    // grows from its top left (room below and right of the point)
    grows_from(&app, &wired, &mut now, (400.0, 300.0));
    // flipped up and left at the window's corner: grows from its bottom right
    grows_from(&app, &wired, &mut now, (W - 20.0, H - 20.0));
}
