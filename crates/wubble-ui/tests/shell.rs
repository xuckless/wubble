//! The spring-driven shell on the testing backend: mock time only, explicit
//! frame deltas, no clocks. The platform can be installed once per process,
//! so the scenarios run in order inside one test.

use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString,
    platform::{Key, PointerEventButton, WindowEvent},
};
use wubble_core::seam::{INSET, SEAM, SIDEBAR_DEFAULT};
use wubble_core::tabs::SpaceSwitcher;
use wubble_ui::driver::{self, Driver};
use wubble_ui::{AppWindow, SidebarMode, SpaceItem};

const DT: f32 = 0.016;

type Shared = std::rc::Rc<std::cell::RefCell<Driver>>;

/// Frame deltas until everything rests. Count-bounded, not time-bounded.
fn settle(app: &AppWindow, driver: &Shared) {
    for _ in 0..4000 {
        if !driver.borrow_mut().advance(app, DT) {
            return;
        }
    }
    panic!("springs never settled");
}

/// A fixed number of frames, for a held seam (it moves for as long as it is held).
fn frames(app: &AppWindow, driver: &Shared, n: usize) {
    for _ in 0..n {
        driver.borrow_mut().advance(app, DT);
    }
}

/// Lets Slint run its `changed` handlers, timers and tweens for `ms` of mock time.
fn pump(ms: u64) {
    mock_elapsed_time(Duration::from_millis(ms));
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

fn pointer(app: &AppWindow, x: f32, y: f32) {
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    pump(1);
}

fn button(app: &AppWindow, x: f32, y: f32, down: bool) {
    let (position, button) = (at(x, y), PointerEventButton::Left);
    app.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
    pump(1);
}

/// Modifiers held down around one key, as a keyboard sends them.
fn chord(app: &AppWindow, held: &[SharedString], key: &str) {
    let w = app.window();
    for m in held {
        w.dispatch_event(WindowEvent::KeyPressed { text: m.clone() });
    }
    w.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    w.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    for m in held.iter().rev() {
        w.dispatch_event(WindowEvent::KeyReleased { text: m.clone() });
    }
    pump(1);
}

/// The seam's centre: the sidebar's edge plus half the inset.
fn seam_x(app: &AppWindow) -> f32 {
    INSET as f32 + app.get_sidebar_width() + INSET as f32 / 2.0
}

fn three_modes(app: &AppWindow, driver: &Shared) {
    settle(app, driver);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    assert_eq!(app.get_sidebar_width(), SIDEBAR_DEFAULT as f32);
    assert_eq!(app.get_content_x(), 268.0);

    app.invoke_toggle_sidebar();
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Hidden);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), 0.0);
    assert_eq!(app.get_sidebar_opacity(), 0.0);
    assert_eq!(app.get_content_x(), 8.0);
    assert_eq!(app.get_content_width(), 1280.0 - 16.0);

    // The frame hook asks for no redraw once idle, and stays idle.
    let base = std::time::Instant::now();
    assert!(!driver.borrow_mut().frame(app, base));
    assert!(
        !driver
            .borrow_mut()
            .frame(app, base + Duration::from_millis(16))
    );

    app.invoke_toggle_sidebar();
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), SIDEBAR_DEFAULT as f32);
    assert_eq!(app.get_content_x(), 268.0);
    assert!(!app.get_sidebar_rail());
}

fn peek(app: &AppWindow, driver: &Shared) {
    app.invoke_toggle_sidebar();
    settle(app, driver);
    pump(1);
    assert!(!app.get_peeking());

    // Into the edge strip: it peeks 60 ms later.
    pointer(app, 10.0, 300.0);
    pump(30);
    assert!(!app.get_peeking());
    pump(40);
    assert!(app.get_peeking());
    // The slide (340 ms), then over a child row: still peeking.
    pump(400);
    pointer(app, 100.0, 80.0);
    pump(400);
    assert!(app.get_peeking());
    // Off the sidebar: it leaves 260 ms later.
    pointer(app, 900.0, 300.0);
    pump(200);
    assert!(app.get_peeking());
    pump(100);
    assert!(!app.get_peeking());

    // A peek never outlives the hidden mode (once the sidebar has slid away from the edge).
    pump(400);
    pointer(app, 10.0, 300.0);
    pump(100);
    assert!(app.get_peeking());
    app.invoke_toggle_sidebar();
    pump(1);
    assert!(!app.get_peek());
    settle(app, driver);
    pointer(app, 900.0, 300.0);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
}

fn seam_drag(app: &AppWindow, driver: &Shared) {
    // Inside the limits the width follows the pointer one to one (the pointer includes the inset).
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, x + 30.0, 300.0);
    frames(app, driver, 600);
    assert_eq!(app.get_sidebar_width(), x + 30.0 - INSET as f32);
    assert!(app.get_seam_bowed());
    assert!(!app.get_seam_path().is_empty());
    // Far past the maximum it is banded, never beyond max + 34.
    pointer(app, 900.0, 300.0);
    frames(app, driver, 600);
    let held = app.get_sidebar_width();
    let max = SEAM::max as f32;
    assert!(held > max && held < max + SEAM::bandK as f32, "{held}");
    button(app, 900.0, 300.0, false);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), max);
    assert!(!app.get_seam_bowed());

    // Pulled under the rail snap point (128): the rail.
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, INSET as f32 + 100.0, 300.0);
    frames(app, driver, 60);
    button(app, INSET as f32 + 100.0, 300.0, false);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Rail);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), SEAM::rail as f32);
    assert!(app.get_sidebar_rail());

    // From the rail, under 22: hidden.
    let x = seam_x(app);
    pointer(app, x, 300.0);
    button(app, x, 300.0, true);
    pointer(app, INSET as f32 + 10.0, 300.0);
    frames(app, driver, 10);
    button(app, INSET as f32 + 10.0, 300.0, false);
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Hidden);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), 0.0);

    // The seam strip is gone; the sidebar shortcut brings it back at the saved width.
    chord(app, &[Key::Control.into(), Key::Alt.into()], "z");
    assert_eq!(app.get_sidebar_mode(), SidebarMode::Extended);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), max);
    assert_eq!(app.get_sidebar_saved(), max);
    pointer(app, 900.0, 300.0);
}

fn space_capsule(app: &AppWindow, driver: &Shared) {
    let spaces: Vec<SpaceItem> = (1..=3)
        .map(|id| SpaceItem {
            id,
            name: "".into(),
            tint: Default::default(),
        })
        .collect();
    app.set_spaces(ModelRc::from(spaces.as_slice()));
    app.set_active_space_id(3);
    pump(1);
    settle(app, driver);
    assert_eq!(app.get_space_index(), 2);
    assert_eq!(app.get_capsule_offset(), 2.0 * SpaceSwitcher::PITCH as f32);
    app.set_active_space_id(1);
    pump(1);
    settle(app, driver);
    assert_eq!(app.get_capsule_offset(), 0.0);

    // Id set before the model: the index resolves when the model arrives.
    app.set_spaces(ModelRc::default());
    app.set_active_space_id(3);
    pump(1);
    assert_eq!(app.get_space_index(), -1);
    app.set_spaces(ModelRc::from(spaces.as_slice()));
    pump(1);
    settle(app, driver);
    assert_eq!(app.get_capsule_offset(), 2.0 * SpaceSwitcher::PITCH as f32);
}

#[test]
fn shell_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let driver = driver::connect(&app);
    three_modes(&app, &driver);
    peek(&app, &driver);
    seam_drag(&app, &driver);
    space_capsule(&app, &driver);
}
