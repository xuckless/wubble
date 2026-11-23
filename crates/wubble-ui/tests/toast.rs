//! The toast on the testing backend: its glass sizes to its text, a short notice and a long one
//! alike, and the text never runs outside it. Mock time only; one test (one platform per
//! process).

use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize};
use wubble_ui::AppWindow;

fn element(app: &AppWindow, id: &str) -> ElementHandle {
    ElementHandle::find_by_element_id(app, id)
        .next()
        .unwrap_or_else(|| panic!("no {id}"))
}

/// (pill x, pill right, text x, text right, pill height, text height)
fn shown(app: &AppWindow, text: &str) -> [f32; 6] {
    app.set_toast_text(text.into());
    app.set_toast_shown(true);
    mock_elapsed_time(Duration::from_millis(600));
    let pill = element(app, "AToast::pill");
    let label = element(app, "AToast::label");
    let (p, ps) = (pill.absolute_position(), pill.size());
    let (l, ls) = (label.absolute_position(), label.size());
    [
        p.x,
        p.x + ps.width,
        l.x,
        l.x + ls.width,
        ps.height,
        ls.height,
    ]
}

#[test]
fn the_toast_holds_its_text() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let window_width = 1280.0;
    for text in [
        "125%",
        "Screenshot saved to Downloads and copied",
        "Video controls hidden on www.example.com. Show them again in Settings, under \
         \"Sites without video controls\".",
    ] {
        let [px, pr, lx, lr, ph, lh] = shown(&app, text);
        assert!(lr - lx > 0.0, "{text:?}: the text has no width");
        assert!(
            px <= lx && lr <= pr,
            "{text:?}: text {lx}..{lr} outside its glass {px}..{pr}"
        );
        assert!(lh <= ph, "{text:?}: text {lh} high in glass {ph}");
        assert!(px >= 0.0 && pr <= window_width, "{text:?}: off the window");
    }
}
