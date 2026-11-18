//! The autoscroll arrow on the testing backend: its drawn length grows with the stretch the
//! host sets, so a fast scroll shows a longer arrow than the page at rest.

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize};
use std::time::Duration;
use wubble_ui::AppWindow;

fn arrow_length(app: &AppWindow) -> f32 {
    mock_elapsed_time(Duration::from_millis(16));
    let path = ElementHandle::find_by_element_id(app, "AutoscrollCursor::arrow")
        .next()
        .expect("the arrow is drawn while moving");
    path.size().height
}

#[test]
fn the_arrow_is_longer_at_speed() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    app.set_autoscroll_on(true);
    app.set_autoscroll_moving(true);
    app.set_autoscroll_x(400.0);
    app.set_autoscroll_y(400.0);
    app.set_autoscroll_stretch(1.0);
    app.set_autoscroll_squash(1.0);
    let rest = arrow_length(&app);
    assert!(rest > 0.0);
    app.set_autoscroll_stretch(1.35);
    app.set_autoscroll_squash(1.0 / 1.35_f32.sqrt());
    let fast = arrow_length(&app);
    assert!(fast > rest * 1.3, "{fast} vs {rest}");
}
