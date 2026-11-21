//! A query longer than the search capsule stays inside it: the input's cell keeps the layout's
//! width, the text slides under the caret, and the clear button stays in the capsule (the start
//! page showed the text running past the pill and the button outside it). Mock time only; one
//! test (one platform per process).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::settings_page;
use wubble_ui::driver::Wired;
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, InternalPageKind};

fn frames(app: &AppWindow, wired: &Wired, n: usize) {
    for _ in 0..n {
        mock_elapsed_time(Duration::from_millis(16));
        wired.frame(app, Instant::now());
    }
}

/// The left and right edge of the first shown element with `id`.
fn span(app: &AppWindow, id: &str) -> (f32, f32) {
    let e = ElementHandle::find_by_element_id(app, id)
        .find(|e| e.size().width > 0.0)
        .unwrap_or_else(|| panic!("no {id}"));
    let x = e.absolute_position().x;
    (x, x + e.size().width)
}

#[test]
fn a_long_query_stays_in_the_capsule() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 880.0));
    let wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.show().unwrap();
    app.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(setting_items(
        &settings_page(&HashMap::new()),
    )))));
    frames(&app, &wired, 10);
    // Settings opens with the keys in its search (tests/settings_focus.rs)
    app.set_page(InternalPageKind::Settings);
    frames(&app, &wired, 30);

    let (cell_left, cell_right) = span(&app, "SearchCapsule::input");
    for c in "nvidia driver bug, what does it affect and how long has it been around?".chars() {
        for event in [
            WindowEvent::KeyPressed {
                text: c.to_string().into(),
            },
            WindowEvent::KeyReleased {
                text: c.to_string().into(),
            },
        ] {
            app.window().dispatch_event(event);
        }
    }
    frames(&app, &wired, 30);

    let (cap_left, cap_right) = span(&app, "SearchCapsule::clear");
    let (text_left, text_right) = span(&app, "SearchCapsule::input");
    assert!(
        text_right - text_left > cell_right - cell_left,
        "the query is longer than the field ({text_left}..{text_right})"
    );
    assert!(
        text_right <= cell_right + 1.0,
        "the text ends at the caret inside the cell: {text_right} > {cell_right}"
    );
    assert!(
        text_left < cell_left,
        "the text slid left under the caret: {text_left}"
    );
    assert!(
        cap_left >= cell_right - 1.0 && cap_right - cap_left > 0.0,
        "the clear button stays after the cell: {cap_left}..{cap_right}, cell ends {cell_right}"
    );
}
