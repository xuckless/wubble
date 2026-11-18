//! The spring-driven shell on the testing backend: mock time only, explicit
//! frame deltas, no clocks. The platform can be installed once per process,
//! so the scenarios run in order inside one test.

use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, ModelRc,
    platform::{PointerEventButton, WindowEvent},
};
use wubble_ui::driver::{self, Driver};
use wubble_ui::{AppWindow, CompactMode, SpaceItem};

const DT: f32 = 0.016;

type Shared = std::rc::Rc<std::cell::RefCell<Driver>>;

/// Frame deltas until the springs rest. Count-bounded, not time-bounded.
fn settle(app: &AppWindow, driver: &Shared) {
    for _ in 0..4000 {
        if !driver.borrow_mut().advance(app, DT) {
            return;
        }
    }
    panic!("springs never settled");
}

/// Lets Slint run its `changed` handlers.
fn pump() {
    mock_elapsed_time(Duration::from_millis(1));
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

fn pointer(app: &AppWindow, x: f32, y: f32) {
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    pump();
}

fn advance_writes_properties_and_stops(app: &AppWindow, driver: &Shared) {
    assert_eq!(app.get_compact_mode(), CompactMode::Expanded);
    app.invoke_toggle_compact();
    assert_eq!(app.get_compact_mode(), CompactMode::Hidden);
    settle(app, driver);
    assert_eq!(app.get_sidebar_slide(), 0.0);
    assert_eq!(app.get_content_inset(), 0.0);

    // At rest, advancing writes nothing and asks for no frame.
    app.set_sidebar_slide(0.5);
    assert!(!driver.borrow_mut().advance(app, DT));
    assert_eq!(app.get_sidebar_slide(), 0.5);

    // toggle_compact_roundtrip
    app.invoke_toggle_compact();
    assert_eq!(app.get_compact_mode(), CompactMode::Expanded);
    settle(app, driver);
    assert_eq!(app.get_sidebar_slide(), 1.0);
    assert_eq!(app.get_content_inset(), 1.0);
}

fn hover_zone_survives_child_hover(app: &AppWindow, driver: &Shared) {
    app.invoke_toggle_compact();
    settle(app, driver);
    assert_eq!(app.get_compact_mode(), CompactMode::Hidden);

    pointer(app, 3.0, 300.0);
    assert_eq!(app.get_compact_mode(), CompactMode::Peeking);
    settle(app, driver);
    // Over a child row of the sidebar: the zone must keep its hover.
    pointer(app, 100.0, 40.0);
    assert_eq!(app.get_compact_mode(), CompactMode::Peeking);
    pointer(app, 900.0, 300.0);
    assert_eq!(app.get_compact_mode(), CompactMode::Hidden);
    settle(app, driver);

    app.invoke_toggle_compact();
    settle(app, driver);
    assert_eq!(app.get_compact_mode(), CompactMode::Expanded);
    pointer(app, 900.0, 300.0);
}

fn seam_drag_follows_pointer_and_rubber_bands(app: &AppWindow, driver: &Shared) {
    let max = 360.0;
    // The handle straddles the sidebar's right edge (8 + 260 + 4).
    let grab = at(272.0, 300.0);
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: grab });
    pump();
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: grab,
        button: PointerEventButton::Left,
    });
    // Inside the limits the width follows the pointer one to one.
    pointer(app, 302.0, 300.0);
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), 290.0);
    // Far past the maximum it is banded, never beyond max + 34.
    pointer(app, 900.0, 300.0);
    settle(app, driver);
    let held = app.get_sidebar_width();
    assert!(held > max && held < max + 34.0, "{held}");
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at(900.0, 300.0),
        button: PointerEventButton::Left,
    });
    settle(app, driver);
    assert_eq!(app.get_sidebar_width(), max);
}

fn space_indicator_follows_active(app: &AppWindow, driver: &Shared) {
    let spaces: Vec<SpaceItem> = (1..=3)
        .map(|id| SpaceItem {
            id,
            name: "".into(),
            tint: Default::default(),
        })
        .collect();
    app.set_spaces(ModelRc::from(spaces.as_slice()));
    app.set_active_space_id(3);
    pump();
    settle(app, driver);
    assert_eq!(app.get_space_indicator(), 2.0);
    app.set_active_space_id(1);
    pump();
    settle(app, driver);
    assert_eq!(app.get_space_indicator(), 0.0);
}

#[test]
fn shell_scenarios() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let driver = driver::connect(&app);
    advance_writes_properties_and_stops(&app, &driver);
    hover_zone_survives_child_hover(&app, &driver);
    seam_drag_follows_pointer_and_rubber_bands(&app, &driver);
    space_indicator_follows_active(&app, &driver);
}
