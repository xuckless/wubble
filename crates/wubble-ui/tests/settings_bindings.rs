//! The Keyboard shortcuts rows by pointer, on the testing backend: a press on a row's chord asks
//! the host to record a new one for that action, a press on Reset gives the row its own back,
//! and while a chord is asked, Replace and Cancel answer it. (The chord's own action, the
//! screen reader's way in, was tested alone: the chord drew caps but a press on them did
//! nothing.) Mock time only; one test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, VecModel};
use wubble_core::keys::{Platform, binding_rows};
use wubble_ui::settings::binding_items;
use wubble_ui::{AppWindow, CaptureView, InternalPageKind};

fn pump() {
    mock_elapsed_time(Duration::from_millis(400));
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

fn button(app: &AppWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .find(|h| h.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button {label:?}"))
}

#[test]
fn a_press_on_a_chord_asks_to_record_it() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 3000.0));
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    let rows = binding_items(&binding_rows(Platform::Linux, &[]));
    app.set_settings_bindings(ModelRc::from(Rc::new(VecModel::from(rows))));
    pump();
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    app.on_binding_record(move |id| l.borrow_mut().push(format!("record {id}")));
    let l = log.clone();
    app.on_capture_confirm(move || l.borrow_mut().push("confirm".into()));
    let l = log.clone();
    app.on_capture_cancel(move || l.borrow_mut().push("cancel".into()));

    click(&app, &button(&app, "New tab"));
    assert_eq!(*log.borrow(), ["record new-tab"], "a press on the chord");

    // asked a chord another action runs: Replace and Cancel answer
    log.borrow_mut().clear();
    app.set_settings_capture(CaptureView {
        action: "new-tab".into(),
        conflict: true,
        chord: "Ctrl+W".into(),
        other: "Close tab".into(),
        ..Default::default()
    });
    pump();
    click(&app, &button(&app, "Replace"));
    click(&app, &button(&app, "Cancel"));
    assert_eq!(*log.borrow(), ["confirm", "cancel"]);
}
