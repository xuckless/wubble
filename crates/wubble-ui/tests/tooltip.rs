//! The window's tooltip on the testing backend: it hangs below what it is for, and sits above
//! it when the window has no room below (a sidebar-bottom button, a pointer near the bottom
//! edge). Mock time only; one test (one platform per process).

use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize};
use wubble_ui::{AppWindow, TipView};

fn tip_top(app: &AppWindow, view: TipView) -> (f32, f32) {
    app.set_tip(view);
    mock_elapsed_time(Duration::from_millis(600));
    let tip = ElementHandle::find_by_element_id(app, "Tooltip::tip")
        .next()
        .expect("the tooltip");
    (tip.absolute_position().y, tip.size().height)
}

#[test]
fn a_tooltip_without_room_below_sits_above() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(800.0, 600.0));
    let button = |top: f32, bottom: f32| TipView {
        shown: true,
        text: "Settings".into(),
        shortcut: "".into(),
        anchor_x: 100.0,
        anchor_y: bottom,
        anchor_top: top,
        centered: true,
        jump: true,
    };

    let (y, h) = tip_top(&app, button(100.0, 130.0));
    assert!(y > 130.0, "below a button with room: {y}");
    assert!(h > 0.0);

    let (y, h) = tip_top(&app, button(564.0, 594.0));
    assert!(
        y + h < 564.0,
        "above a button at the bottom edge: {y} + {h}"
    );

    let pointer = TipView {
        centered: false,
        anchor_x: 300.0,
        anchor_y: 590.0,
        anchor_top: 570.0,
        ..button(0.0, 0.0)
    };
    let (y, h) = tip_top(&app, pointer);
    assert!(y + h < 570.0, "above a pointer near the bottom: {y} + {h}");
    assert!(y + h <= 600.0);
}
