//! The shell's accessibility on the testing backend: what a screen reader is told (role, label,
//! selection) about the tab list, the sidebar's buttons, the Spaces and the context menu, and
//! that each one's default action (what an assistive technology's "click" runs) reaches the
//! same callback as a pointer click. Mock time only; one test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedString, VecModel};
use wubble_ui::{AppWindow, CommandBarState, FocusRegion, MenuEntry, MenuView, SpaceItem, TabItem};

/// The one element `label` names (role and all), failing with the label when there is none.
fn one(app: &AppWindow, label: &str) -> ElementHandle {
    let mut found: Vec<_> = ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.accessible_role() != Some(AccessibleRole::Text))
        .collect();
    assert_eq!(found.len(), 1, "one element labelled {label:?}");
    found.remove(0)
}

fn key(app: &AppWindow, text: SharedString) {
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    pump();
}

fn pump() {
    mock_elapsed_time(Duration::from_millis(16));
}

#[test]
fn the_shell_names_its_controls_and_runs_their_default_action() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    // the kit menu keeps its highlight and keys in the controls registry
    let _controls = wubble_ui::controls::wire(&app);
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let tab = |id: i32, title: &str| TabItem {
        id,
        title: title.into(),
        ..Default::default()
    };
    app.set_tabs(ModelRc::new(VecModel::from(vec![
        tab(1, "Alpha"),
        tab(2, "Beta"),
    ])));
    app.set_active_tab_id(2);
    app.set_selected_row(1);
    let space = |id: i32, name: &str| SpaceItem {
        id,
        name: name.into(),
        icon: "personal".into(),
        ..Default::default()
    };
    app.set_spaces(ModelRc::new(VecModel::from(vec![
        space(10, "Home"),
        space(11, "Work"),
    ])));
    app.set_active_space_id(11);
    // the command bar's machine runs in the driver (which reads the active Space's row)
    let _driver = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    pump();

    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let push = |what: String| {
        let log = log.clone();
        move || log.borrow_mut().push(what.clone())
    };
    {
        let l = log.clone();
        app.on_tab_activated(move |id| l.borrow_mut().push(format!("tab {id}")));
        let l = log.clone();
        app.on_shell_action(move |id| l.borrow_mut().push(format!("action {id}")));
        let l = log.clone();
        app.on_space_switched(move |id| l.borrow_mut().push(format!("space {id}")));
        let l = log.clone();
        app.on_menu_chosen(move |i| l.borrow_mut().push(format!("menu {i}")));
    }
    app.on_settings_requested(push("settings".into()));
    app.on_new_tab_requested(push("new tab".into()));
    {
        let l = log.clone();
        app.on_tab_closed(move |id| l.borrow_mut().push(format!("close {id}")));
    }

    // F6 from the page: the address (the bar opens and takes the keys), F6 again: the sidebar.
    // The bar's closing does not take the keys back to the page once it settles.
    assert!(!app.get_sidebar_focused());
    app.invoke_focus_region(FocusRegion::Address);
    pump();
    app.invoke_focus_region(FocusRegion::Sidebar);
    assert_eq!(app.get_command_bar_state(), CommandBarState::Closing);
    // the close timer (the dropdown's out duration, 176 ms) starts on the next turn
    pump();
    mock_elapsed_time(Duration::from_millis(200));
    assert_eq!(app.get_command_bar_state(), CommandBarState::Closed);
    pump();
    assert!(
        app.get_sidebar_focused(),
        "F6 counts the sidebar as holding the keys"
    );
    assert!(!app.get_web_focused());
    // On the sidebar's active row: Enter picks it, Delete closes it, Tab goes on to the
    // new-tab row and Space presses it.
    key(&app, Key::Return.into());
    key(&app, Key::Delete.into());
    key(&app, Key::Tab.into());
    key(&app, " ".into());
    assert_eq!(
        log.borrow_mut().drain(..).collect::<Vec<_>>(),
        ["tab 2", "close 2", "new tab"]
    );

    // The tab list: a tab list of tabs, the active one selected; the default action picks one.
    let list = one(&app, "Tabs");
    assert_eq!(list.accessible_role(), Some(AccessibleRole::TabList));
    let alpha = one(&app, "Alpha");
    let beta = one(&app, "Beta");
    assert_eq!(alpha.accessible_role(), Some(AccessibleRole::Tab));
    assert_eq!(alpha.accessible_item_selected(), Some(false));
    assert_eq!(beta.accessible_item_selected(), Some(true));
    alpha.invoke_accessible_default_action();

    // The header's icon buttons are named by what they do; back is disabled without history.
    for (label, enabled) in [
        ("Toggle sidebar", true),
        ("Back", false),
        ("Forward", false),
        ("Reload", true),
        ("Settings", true),
        ("New tab", true),
    ] {
        let b = one(&app, label);
        assert_eq!(b.accessible_role(), Some(AccessibleRole::Button), "{label}");
        if label != "New tab" {
            assert_eq!(b.accessible_enabled(), Some(enabled), "{label}");
        }
    }
    one(&app, "Reload").invoke_accessible_default_action();
    // A disabled button ignores the action, as it ignores a click.
    one(&app, "Back").invoke_accessible_default_action();
    one(&app, "Settings").invoke_accessible_default_action();
    one(&app, "New tab").invoke_accessible_default_action();

    // The Spaces: tabs of their own list, the active one selected.
    assert_eq!(
        one(&app, "Spaces").accessible_role(),
        Some(AccessibleRole::TabList)
    );
    let home = one(&app, "Home");
    assert_eq!(home.accessible_role(), Some(AccessibleRole::Tab));
    assert_eq!(home.accessible_item_selected(), Some(false));
    assert_eq!(one(&app, "Work").accessible_item_selected(), Some(true));
    home.invoke_accessible_default_action();

    // The context menu: a list of items; a disabled one does nothing.
    let entry = |label: &str, enabled: bool| MenuEntry {
        label: label.into(),
        enabled,
        ..Default::default()
    };
    app.set_menu(MenuView {
        open: true,
        x: 200.0,
        y: 200.0,
        entries: ModelRc::new(VecModel::from(vec![
            entry("Copy", true),
            entry("Paste", false),
        ])),
    });
    pump();
    assert_eq!(
        one(&app, "Menu").accessible_role(),
        Some(AccessibleRole::List)
    );
    let copy = one(&app, "Copy");
    assert_eq!(copy.accessible_role(), Some(AccessibleRole::ListItem));
    assert_eq!(one(&app, "Paste").accessible_enabled(), Some(false));
    one(&app, "Paste").invoke_accessible_default_action();
    copy.invoke_accessible_default_action();

    let expected: Vec<SharedString> = [
        "tab 1",
        "action reload",
        "settings",
        "new tab",
        "space 10",
        "menu 0",
    ]
    .map(SharedString::from)
    .to_vec();
    let got: Vec<SharedString> = log.borrow().iter().map(SharedString::from).collect();
    assert_eq!(got, expected);
    log.borrow_mut().clear();

    // The active tab is no row (a favourite): F6 to the sidebar lands on the new-tab row.
    app.set_menu(MenuView::default());
    app.set_active_tab_id(99);
    app.set_selected_row(-1);
    pump();
    app.invoke_focus_region(FocusRegion::Sidebar);
    pump();
    key(&app, Key::Return.into());
    assert_eq!(log.borrow().as_slice(), ["new tab"]);
    // F6 to the page: the sidebar no longer holds the keys.
    app.invoke_focus_region(FocusRegion::Page);
    pump();
    assert!(!app.get_sidebar_focused());
}
