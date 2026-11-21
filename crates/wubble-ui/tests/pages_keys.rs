//! The list pages by keyboard on the testing backend: Tab reaches each row's Delete button and
//! Enter presses it. Mock time only; one test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedString, VecModel};
use wubble_ui::{AppWindow, HistoryLineItem, InternalPageKind};

fn pump() {
    mock_elapsed_time(Duration::from_millis(500));
}

fn key(app: &AppWindow, text: SharedString) {
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    pump();
}

fn visit(key: i32, url: &str) -> HistoryLineItem {
    HistoryLineItem {
        header: false,
        key,
        label: Default::default(),
        title: url.into(),
        url: url.into(),
        favicon: Default::default(),
    }
}

/// A history page with a header and two visits, its callbacks logging into the returned list.
fn page() -> (AppWindow, Rc<RefCell<Vec<String>>>) {
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 900.0));
    app.set_page(InternalPageKind::History);
    app.set_history_lines(ModelRc::new(VecModel::from(vec![
        HistoryLineItem {
            header: true,
            key: -1,
            label: "Today".into(),
            title: Default::default(),
            url: Default::default(),
            favicon: Default::default(),
        },
        visit(1, "https://a.test/"),
        visit(2, "https://b.test/"),
    ])));
    pump();
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = log.clone();
    app.on_history_deleted(move |u| l.borrow_mut().push(format!("delete {u}")));
    let l = log.clone();
    app.on_page_opened(move |u| l.borrow_mut().push(format!("open {u}")));
    let l = log.clone();
    app.on_page_opened_new_tab(move |u| l.borrow_mut().push(format!("tab {u}")));
    (app, log)
}

#[test]
fn history_rows_can_be_deleted_from_the_keyboard() {
    init_integration_test_with_mock_time();
    // Tab n times from the page's first focus, then Enter: what that stop does
    let mut by_stop = vec![];
    for stops in 1..=14 {
        let (app, log) = page();
        for _ in 0..stops {
            key(&app, Key::Tab.into());
        }
        key(&app, Key::Return.into());
        by_stop.push(log.borrow().clone());
    }
    let flat: Vec<&String> = by_stop.iter().flatten().collect();
    assert!(
        flat.iter().any(|l| l.as_str() == "delete https://a.test/"),
        "the first row's Delete is a Tab stop: {by_stop:?}"
    );
    assert!(
        flat.iter().any(|l| l.as_str() == "delete https://b.test/"),
        "the second row's Delete is a Tab stop: {by_stop:?}"
    );
}
