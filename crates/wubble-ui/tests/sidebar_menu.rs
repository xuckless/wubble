//! A right click on the sidebar's empty glass asks for the sidebar's menu at the pointer (QA
//! LOW-02); one on a tab row stays the row's.

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::init_integration_test_with_mock_time;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use wubble_ui::{AppWindow, TabItem, driver};

fn right_click(app: &AppWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Right,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Right,
    });
}

#[test]
fn the_empty_sidebar_asks_for_its_menu() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let driver = driver::wire(&app, wubble_core::seam::Seam::new()).driver;
    let tab = TabItem {
        id: 1,
        title: "Example".into(),
        ..Default::default()
    };
    app.set_tabs(slint::ModelRc::from(Rc::new(slint::VecModel::from(vec![
        tab,
    ]))));
    for _ in 0..600 {
        if !driver.borrow_mut().advance(&app, 0.016) {
            break;
        }
    }
    let asked = Rc::new(RefCell::new(vec![]));
    let rows = Rc::new(RefCell::new(vec![]));
    let a = Rc::clone(&asked);
    app.on_sidebar_context(move |x, y| a.borrow_mut().push((x, y)));
    let r = Rc::clone(&rows);
    app.on_tab_context(move |id, _, _| r.borrow_mut().push(id));

    // below the rows and the new-tab row, above the footer: the sidebar's own glass
    right_click(&app, 130.0, 600.0);
    assert_eq!(*asked.borrow(), [(130.0, 600.0)]);
    assert!(rows.borrow().is_empty());

    // the first tab row: the row's menu, not the sidebar's
    let y = app.get_side_edges().pill_y + 48.0;
    right_click(&app, 130.0, y);
    assert_eq!(asked.borrow().len(), 1, "a row's click is the row's");
    assert_eq!(*rows.borrow(), [1]);
}
