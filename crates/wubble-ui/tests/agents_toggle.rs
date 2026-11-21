//! The Agents (MCP) switch draws the hosting state it is given: on when the Settings page (built
//! with the window, before the controls registry is wired) shows a running server, and
//! springing to on when hosting starts while the page is up. Mock time only.

use std::time::{Duration, Instant};

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, Model};
use wubble_ui::{AgentsView, AppWindow, Controls, InternalPageKind, driver};

fn agents(on: bool) -> AgentsView {
    AgentsView {
        on,
        status: if on { "Hosting" } else { "Off" }.into(),
        ..Default::default()
    }
}

/// Every toggle's (value, knob x, fill) after the springs came to rest.
fn at_rest(app: &AppWindow, wired: &driver::Wired) -> Vec<(f32, f32, f32)> {
    mock_elapsed_time(Duration::from_millis(16));
    let mut now = Instant::now();
    let mut frames = 0;
    while wired.frame(app, now).0 && frames < 2000 {
        now += Duration::from_millis(8);
        frames += 1;
    }
    assert!(frames < 2000, "the springs came to rest");
    app.global::<Controls>()
        .get_views()
        .iter()
        .map(|v| (v.value, v.x, v.fill))
        .collect()
}

fn drawn_on(views: &[(f32, f32, f32)]) -> bool {
    views
        .iter()
        .any(|(value, x, fill)| *value == 1.0 && *x > 0.0 && *fill == 1.0)
}

#[test]
fn the_switch_draws_the_hosting_state() {
    init_integration_test_with_mock_time();

    // hosting already on as the page opens (the setting on at start)
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 4000.0));
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_settings_agents(agents(true));
    app.set_page(InternalPageKind::Settings);
    let views = at_rest(&app, &wired);
    assert!(drawn_on(&views), "drawn on as the page opens: {views:?}");

    // off, then turned on while the page is up
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 4000.0));
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_settings_agents(agents(false));
    app.set_page(InternalPageKind::Settings);
    let views = at_rest(&app, &wired);
    assert!(!drawn_on(&views), "drawn off: {views:?}");
    app.set_settings_agents(agents(true));
    let views = at_rest(&app, &wired);
    assert!(drawn_on(&views), "springs on and rests there: {views:?}");
}
