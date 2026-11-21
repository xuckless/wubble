//! The settings page's Select (the search engine) picked by the pointer and by the keyboard, on
//! the testing backend, in the order the app does it: the window is built, the controls are
//! wired, the page is opened and its rows set. A row of the open menu is pressed where it is
//! drawn; the keys go to the window. Mock time only; one test (one platform per process).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, ElementQuery, init_integration_test_with_mock_time,
    mock_elapsed_time,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::settings_page;
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, InternalPageKind};

fn pump(ms: u64) {
    mock_elapsed_time(Duration::from_millis(ms));
}

fn click(app: &AppWindow, h: &ElementHandle) {
    let (p, s) = (h.absolute_position(), h.size());
    let position = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    let button = PointerEventButton::Left;
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    pump(1);
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    pump(1);
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
    pump(400);
}

fn press(app: &AppWindow, key: Key) {
    let text: slint::SharedString = key.into();
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    pump(400);
}

fn trigger(app: &AppWindow) -> ElementHandle {
    ElementQuery::from_root(app)
        .match_descendants()
        .match_accessible_role(AccessibleRole::Combobox)
        .find_first()
        .expect("a Select on the settings page")
}

fn row(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .find(|h| h.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap_or_else(|| panic!("no menu row {label}"))
}

/// What the host does with a change: stores it and draws the page again from the store.
fn render(app: &AppWindow, stored: &HashMap<String, String>) {
    let rows = VecModel::from(setting_items(&settings_page(stored)));
    app.set_settings_rows(ModelRc::from(Rc::new(rows)));
}

/// The label the trigger shows.
fn shown(app: &AppWindow) -> String {
    trigger(app).accessible_value().unwrap().to_string()
}

#[test]
fn the_search_engine_select_is_picked_by_pointer_and_by_key() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 880.0));
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    let stored = Rc::new(RefCell::new(HashMap::new()));
    render(&app, &stored.borrow());
    app.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    pump(600);
    let changed = Rc::new(RefCell::new(vec![]));
    let (c, st, weak) = (changed.clone(), stored.clone(), app.as_weak());
    app.on_setting_changed(move |key, value| {
        c.borrow_mut().push((key.to_string(), value.to_string()));
        st.borrow_mut().insert(key.to_string(), value.to_string());
        render(&weak.upgrade().unwrap(), &st.borrow());
    });
    let open = || trigger(&app).accessible_expanded() == Some(true);

    // the pointer: press the trigger, the list opens, press a row
    assert!(!open());
    click(&app, &trigger(&app));
    assert!(open(), "a press on the trigger opens the list");
    click(&app, &row(&app, "DuckDuckGo"));
    assert_eq!(
        *changed.borrow(),
        [("search.engine".to_string(), "duckduckgo".to_string())],
        "a press on a row picks it"
    );
    assert!(!open(), "a pick closes the list");
    assert_eq!(shown(&app), "DuckDuckGo", "the trigger shows the pick");

    // the keyboard: the trigger's action (Enter on it) opens the list with the focus in it;
    // Escape closes it and picks nothing
    changed.borrow_mut().clear();
    trigger(&app).invoke_accessible_default_action();
    pump(400);
    assert!(open(), "the trigger's action opens the list");
    press(&app, Key::Escape);
    assert!(!open(), "Escape closes a list opened by key");
    assert!(changed.borrow().is_empty(), "and picks nothing");

    // Down moves off the current row (the last: it wraps) and Enter picks the one it lands on
    trigger(&app).invoke_accessible_default_action();
    pump(400);
    press(&app, Key::DownArrow);
    press(&app, Key::Return);
    assert_eq!(
        *changed.borrow(),
        [("search.engine".to_string(), "google".to_string())]
    );
    assert!(!open(), "a pick by key closes the list");
    assert_eq!(shown(&app), "Google");

    // a list a click opened answers the keys too: Escape closes it
    changed.borrow_mut().clear();
    click(&app, &trigger(&app));
    assert!(open());
    press(&app, Key::Escape);
    assert!(!open(), "Escape closes a list opened by a click");
    assert!(changed.borrow().is_empty());

    // a press outside closes it and picks nothing
    click(&app, &trigger(&app));
    assert!(open());
    let t = trigger(&app);
    let (p, s) = (t.absolute_position(), t.size());
    let position = LogicalPosition::new(p.x - 300.0, p.y + s.height / 2.0);
    let button = PointerEventButton::Left;
    app.window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    app.window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    pump(400);
    assert!(!open(), "a press outside closes the list");
    assert!(changed.borrow().is_empty());
}
