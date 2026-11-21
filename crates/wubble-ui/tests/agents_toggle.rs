//! The Agents (MCP) switch draws the hosting state it is given: switched on after the page is
//! up, its knob springs to the on side and rests there. Mock time only.

use std::time::{Duration, Instant};

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, LogicalSize, Model};
use wubble_ui::{AgentsView, AppWindow, Controls, InternalPageKind, driver};

#[test]
fn the_switch_follows_the_hosting_state() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 4000.0));
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    app.set_settings_agents(AgentsView {
        on: false,
        status: "Off".into(),
        ..Default::default()
    });
    mock_elapsed_time(Duration::from_millis(100));
    let toggles = |app: &AppWindow| -> Vec<(f32, f32)> {
        app.global::<Controls>()
            .get_views()
            .iter()
            .map(|v| (v.value, v.x))
            .collect()
    };
    assert_eq!(toggles(&app), [(0.0, 0.0)], "drawn off");
    app.set_settings_agents(AgentsView {
        on: true,
        status: "Hosting".into(),
        ..Default::default()
    });
    mock_elapsed_time(Duration::from_millis(16));
    let mut now = Instant::now();
    let mut frames = 0;
    while wired.frame(&app, now).0 && frames < 2000 {
        now += Duration::from_millis(8);
        frames += 1;
    }
    let after = toggles(&app);
    assert!(
        frames > 1 && frames < 2000,
        "it sprang over frames and came to rest"
    );
    assert!(
        after.iter().any(|(value, x)| *value == 1.0 && *x > 0.0),
        "the switch rests on, its knob at the far end: {after:?}"
    );
}
