//! Settings takes the keys into its search as it opens (as `settle-focus` asks every page),
//! through the wide column's search called at once. Mock time only; one test (one platform per
//! process).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
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

/// A key typed now: whether the settings search took it.
fn typed_into_search(app: &AppWindow, queries: &Rc<RefCell<Vec<String>>>) -> bool {
    let before = queries.borrow().len();
    for event in [
        WindowEvent::KeyPressed { text: "x".into() },
        WindowEvent::KeyReleased { text: "x".into() },
    ] {
        app.window().dispatch_event(event);
    }
    queries.borrow().len() > before
}

#[test]
fn opening_settings_gives_its_search_the_keys() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 880.0));
    let wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.show().unwrap();
    app.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let queries = Rc::new(RefCell::new(vec![]));
    let q = queries.clone();
    app.on_settings_query_changed(move |text| q.borrow_mut().push(text.to_string()));
    app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(setting_items(
        &settings_page(&HashMap::new()),
    )))));
    frames(&app, &wired, 10);

    app.set_page(InternalPageKind::Settings);
    frames(&app, &wired, 30);
    assert!(
        typed_into_search(&app, &queries),
        "opened: the search has the keys"
    );
}
