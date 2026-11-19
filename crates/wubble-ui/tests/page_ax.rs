//! The visible page for screen readers (`page-ax.slint`) on the testing backend: each item of
//! `ax-items` is an accessible element with the role and label its kind gives (a link a button
//! called "…, link", a heading text called "Heading level N, …"), a field's value, a check
//! box's state, and its default action and set-value reach `ax-action` with the item's id. One
//! test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};
use wubble_ui::{AppWindow, AxItemView};

fn one(app: &AppWindow, label: &str) -> ElementHandle {
    let mut found: Vec<_> = ElementHandle::find_by_accessible_label(app, label).collect();
    assert_eq!(found.len(), 1, "one element labelled {label:?}");
    found.remove(0)
}

fn item(id: i32, kind: i32, label: &str) -> AxItemView {
    AxItemView {
        id,
        kind,
        label: label.into(),
        x: 10.0,
        y: 20.0 * id as f32,
        w: 100.0,
        h: 18.0,
        ..AxItemView::default()
    }
}

#[test]
fn page_items_are_accessible_elements_with_their_actions() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let log: Rc<RefCell<Vec<(i32, String, String)>>> = Rc::default();
    let l = log.clone();
    app.on_ax_action(move |id, name, value| {
        l.borrow_mut().push((id, name.to_string(), value.to_string()))
    });
    let items = vec![
        AxItemView {
            level: 1,
            ..item(0, 0, "Welcome")
        },
        item(1, 7, "Read the"),
        item(2, 1, "docs"),
        AxItemView {
            value: "a@b".into(),
            ..item(3, 3, "Email")
        },
        AxItemView {
            checked: true,
            ..item(4, 4, "Remember me")
        },
        item(5, 2, "Send"),
    ];
    app.set_ax_items(ModelRc::new(VecModel::from(items)));
    mock_elapsed_time(Duration::from_millis(16));

    let heading = one(&app, "Heading level 1, Welcome");
    assert_eq!(heading.accessible_role(), Some(AccessibleRole::Text));
    assert_eq!(
        one(&app, "Read the").accessible_role(),
        Some(AccessibleRole::Text)
    );
    let link = one(&app, "docs, link");
    assert_eq!(link.accessible_role(), Some(AccessibleRole::Button));
    let field = one(&app, "Email");
    assert_eq!(field.accessible_role(), Some(AccessibleRole::TextInput));
    assert_eq!(field.accessible_value().as_deref(), Some("a@b"));
    let check = one(&app, "Remember me");
    assert_eq!(check.accessible_role(), Some(AccessibleRole::Checkbox));
    assert_eq!(check.accessible_checked(), Some(true));
    // At the item's place over the page.
    assert_eq!(one(&app, "Send").size(), LogicalSize::new(100.0, 18.0));

    link.invoke_accessible_default_action();
    one(&app, "Send").invoke_accessible_default_action();
    field.set_accessible_value("typed");
    mock_elapsed_time(Duration::from_millis(16));
    assert_eq!(
        *log.borrow(),
        [
            (2, "click".to_string(), String::new()),
            (5, "click".to_string(), String::new()),
            (3, "set-value".to_string(), "typed".to_string()),
        ]
    );

    // Off: no items, no elements.
    app.set_ax_items(ModelRc::default());
    mock_elapsed_time(Duration::from_millis(16));
    assert_eq!(
        ElementHandle::find_by_accessible_label(&app, "docs, link").count(),
        0
    );
}
