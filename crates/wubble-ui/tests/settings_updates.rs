//! The Updates and About groups of the Settings page on the testing backend: the switch reports
//! the check turned off or on, "Check now" asks for a check only while it can be asked for,
//! "Restart to update" shows once a build is ready, an inert build (a package, a development
//! run) has no switch and no button, and About names the version, the engine and the licence,
//! with the notices behind their button. Mock time only; one test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::settings_page;
use wubble_ui::settings::setting_items;
use wubble_ui::{AboutView, AppWindow, InternalPageKind, UpdatesView};

fn pump() {
    mock_elapsed_time(Duration::from_millis(400));
}

fn find(app: &AppWindow, label: &str, role: AccessibleRole) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|h| h.accessible_role() == Some(role))
        .collect()
}

fn click(app: &AppWindow, h: &ElementHandle) {
    let (p, s) = (h.absolute_position(), h.size());
    let position = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    let button = PointerEventButton::Left;
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
    pump();
}

fn updates(on: bool, can_check: bool, ready: bool, inert: bool) -> UpdatesView {
    UpdatesView {
        on,
        version: "0.1.0-preview.3".into(),
        status: "Up to date".into(),
        can_check,
        ready,
        inert,
    }
}

#[test]
fn the_updates_and_about_groups_do_what_they_say() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 9000.0));
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(setting_items(
        &settings_page(&Default::default()),
    )))));
    app.set_settings_about(AboutView {
        version: "0.1.0-preview.3".into(),
        cef: "156.0.30".into(),
        chromium: "156.0.7871.0".into(),
        licence: "GPL-3.0-only".into(),
        notices: "Third-party notices text".into(),
    });
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    app.on_updates_toggled(move |on| l.borrow_mut().push(format!("toggle {on}")));
    let l = log.clone();
    app.on_updates_check(move || l.borrow_mut().push("check".into()));
    let l = log.clone();
    app.on_updates_restart(move || l.borrow_mut().push("restart".into()));
    let switch = "Check for updates when wubble opens";

    // on, up to date: the switch turns the check off; "Check now" asks for one
    app.set_settings_updates(updates(true, true, false, false));
    pump();
    let toggle = find(&app, switch, AccessibleRole::Switch);
    assert_eq!(toggle.len(), 1);
    assert_eq!(toggle[0].accessible_checked(), Some(true));
    click(&app, &toggle[0]);
    click(&app, &find(&app, "Check now", AccessibleRole::Button)[0]);
    assert_eq!(*log.borrow(), ["toggle false", "check"]);

    // off: the switch shows it, and turns it on
    log.borrow_mut().clear();
    app.set_settings_updates(updates(false, true, false, false));
    pump();
    let toggle = find(&app, switch, AccessibleRole::Switch);
    assert_eq!(toggle[0].accessible_checked(), Some(false));
    click(&app, &toggle[0]);
    assert_eq!(*log.borrow(), ["toggle true"]);

    // a check under way cannot be asked for again
    log.borrow_mut().clear();
    app.set_settings_updates(updates(true, false, false, false));
    pump();
    click(&app, &find(&app, "Check now", AccessibleRole::Button)[0]);
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());

    // a build is ready: "Restart to update" instead of "Check now"
    app.set_settings_updates(updates(true, true, true, false));
    pump();
    assert!(find(&app, "Check now", AccessibleRole::Button).is_empty());
    click(
        &app,
        &find(&app, "Restart to update", AccessibleRole::Button)[0],
    );
    assert_eq!(*log.borrow(), ["restart"]);

    // updates come from elsewhere: no switch, no button
    app.set_settings_updates(updates(true, true, false, true));
    pump();
    assert!(find(&app, switch, AccessibleRole::Switch).is_empty());
    assert!(find(&app, "Check now", AccessibleRole::Button).is_empty());

    // About: the version and the engine on one row, the licence on the next, notices behind a button
    let text = |s: &str| ElementHandle::find_by_accessible_label(&app, s).count();
    assert_eq!(
        text("wubble 0.1.0-preview.3"),
        2,
        "the Updates row and the About row"
    );
    assert!(text("GPL-3.0-only") >= 1, "the licence is shown");
    assert_eq!(text("Third-party notices text"), 0, "behind the button");
    click(
        &app,
        &find(&app, "Third-party notices", AccessibleRole::Button)[0],
    );
    assert_eq!(text("Third-party notices text"), 1);
    click(&app, &find(&app, "Hide notices", AccessibleRole::Button)[0]);
    assert_eq!(text("Third-party notices text"), 0);
}
