//! The settings page's side column (title, search, section list, pages) at a window shorter than
//! the list: the wheel scrolls the column under the pointer, and only that one; hovering moves no
//! focus and changes no section; the last row is reachable. Mock time only, no timing asserted.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::{PAGE_LINKS, settings_page};
use wubble_ui::settings::{page_link_items, setting_items};
use wubble_ui::{AppWindow, InternalPageKind, NavScroll, PageScroll};

const HEIGHT: f32 = 480.0;

fn nav_rows(app: &AppWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(app, "NavRow").collect()
}

/// The titles of the current section's rows among those on screen (a row scrolled out of the
/// list is not queried).
fn current(app: &AppWindow) -> Vec<String> {
    nav_rows(app)
        .iter()
        .filter(|h| h.accessible_item_selected() == Some(true))
        .filter_map(|h| h.accessible_label().map(|l| l.to_string()))
        .collect()
}

fn search_box(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, "Search settings")
        .find(|h| h.accessible_role() == Some(AccessibleRole::TextInput))
        .unwrap()
}

/// A key typed now goes to the search box if it has the focus: what its query reports.
fn typed_into_search(app: &AppWindow, queries: &Rc<RefCell<Vec<String>>>, key: &str) -> bool {
    let before = queries.borrow().len();
    for event in [
        WindowEvent::KeyPressed { text: key.into() },
        WindowEvent::KeyReleased { text: key.into() },
    ] {
        app.window().dispatch_event(event);
    }
    queries.borrow().len() > before
}

#[test]
fn the_wheel_scrolls_the_column_under_the_pointer() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, HEIGHT));
    let wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    let page = settings_page(&HashMap::new());
    app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(setting_items(&page)))));
    app.set_settings_pages(ModelRc::from(Rc::new(VecModel::from(page_link_items(
        &PAGE_LINKS,
    )))));
    let frames = |n: usize| {
        for _ in 0..n {
            mock_elapsed_time(Duration::from_millis(16));
            wired.driver.borrow_mut().advance(&app, 0.016);
        }
    };
    frames(30);

    let nav = app.global::<NavScroll>();
    let rows = app.global::<PageScroll>();
    assert!(
        nav.get_content() > nav.get_viewport(),
        "the list is taller than the window"
    );
    assert!(rows.get_content() > rows.get_viewport());
    assert_eq!(nav.get_y(), 0.0);
    assert_eq!(rows.get_y(), 0.0);

    let first = nav_rows(&app).remove(0);
    let at_nav = LogicalPosition::new(
        first.absolute_position().x + 40.0,
        first.absolute_position().y + 10.0,
    );
    let at_rows = LogicalPosition::new(at_nav.x + 232.0 + 48.0 + 60.0, 300.0);
    let wheel = |position: LogicalPosition, delta_y: f32| {
        app.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        app.window().dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y,
        });
        frames(90);
    };
    assert_eq!(current(&app), ["General"]);
    // the search box has the focus (a click); a pointer that only passes over takes none of it
    let queries = Rc::new(RefCell::new(vec![]));
    let q = queries.clone();
    app.on_settings_query_changed(move |text| q.borrow_mut().push(text.to_string()));
    app.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    search_box(&app).mock_single_click(PointerEventButton::Left);
    frames(2);
    // over the rows: the rows move, the list stays
    wheel(at_rows, -60.0);
    let kept = rows.get_y();
    assert!(kept > 0.0, "the rows scrolled: {kept}");
    assert_eq!(nav.get_y(), 0.0, "the list did not");
    assert_eq!(current(&app), ["General"]);

    // over the list: the list moves, the rows stay
    wheel(at_nav, -60.0);
    assert!(nav.get_y() > 0.0, "the list scrolled: {}", nav.get_y());
    assert_eq!(rows.get_y(), kept, "the rows did not");
    assert!(current(&app).iter().all(|c| c == "General"));

    // as far as it goes (the viewport grew as the page scrolled): the last row is on screen
    wheel(at_nav, -2000.0);
    let (view, content) = (nav.get_viewport(), nav.get_content());
    assert!(
        (nav.get_y() - (content - view)).abs() < 0.5,
        "{}",
        nav.get_y()
    );
    let last = nav_rows(&app).pop().unwrap();
    assert_eq!(
        last.accessible_label().unwrap(),
        PAGE_LINKS.last().unwrap().title()
    );
    let bottom = last.absolute_position().y + last.size().height;
    assert!(
        bottom <= HEIGHT,
        "the last row ends at {bottom} in {HEIGHT}"
    );
    assert_eq!(rows.get_y(), kept);
    assert!(current(&app).iter().all(|c| c == "General"));

    // the list back at its top, then the page to its end: the last section becomes current and
    // its row is brought into view
    wheel(at_nav, 2000.0);
    assert_eq!(nav.get_y(), 0.0);
    wheel(at_rows, -20000.0);
    assert_eq!(current(&app), ["About"]);
    let about = nav_rows(&app)
        .into_iter()
        .find(|h| h.accessible_label().as_deref() == Some("About"))
        .unwrap();
    assert!(about.absolute_position().y + about.size().height <= HEIGHT);

    // the wheel moved no focus: a key still goes to the search box
    assert!(
        typed_into_search(&app, &queries, "q"),
        "the search box kept the focus"
    );
    assert_eq!(queries.borrow().last().unwrap(), "q");
}
