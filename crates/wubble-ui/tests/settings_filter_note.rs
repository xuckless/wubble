//! The Filter lists field on the Settings page: a refused address stays in the field with the
//! host's note under it (the setting it goes into has no row of its own to show the note); a
//! taken one empties the field and the note goes. Mock time only; one test (one platform per
//! process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedString, VecModel};
use wubble_core::pages::settings_page;
use wubble_ui::settings::setting_items;
use wubble_ui::{AppWindow, InternalPageKind};

const FIELD: &str = "Add a list by its https:// address";
const REFUSED: &str = "Needs an address that starts with https://";

fn pump() {
    mock_elapsed_time(Duration::from_millis(400));
}

fn field(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, FIELD)
        .find(|h| h.accessible_role() == Some(AccessibleRole::TextInput))
        .expect("the Filter lists field")
}

fn note_shown(app: &AppWindow) -> bool {
    ElementHandle::find_by_accessible_label(app, REFUSED).any(|h| h.computed_opacity() > 0.0)
        && app.get_settings_filter_note() == REFUSED
}

fn submit(app: &AppWindow, text: &str) {
    let f = field(app);
    f.mock_single_click(PointerEventButton::Left);
    f.set_accessible_value(text);
    let enter = SharedString::from(Key::Return);
    let w = app.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: enter.clone(),
    });
    w.dispatch_event(WindowEvent::KeyReleased { text: enter });
    pump();
}

#[test]
fn a_refused_list_address_stays_with_its_note() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 9000.0));
    let _wired = wubble_ui::driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_page(InternalPageKind::Settings);
    app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(setting_items(
        &settings_page(&Default::default()),
    )))));
    // the host's side: an https:// address is taken, anything else refused with its note
    let sent = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let s = sent.clone();
    let w = app.as_weak();
    app.on_setting_entry_added(move |key, entry| {
        s.borrow_mut().push((key.into(), entry.to_string()));
        let note = if entry.starts_with("https://") {
            ""
        } else {
            REFUSED
        };
        w.unwrap().set_settings_filter_note(note.into());
    });
    pump();

    submit(&app, "http://lists.test/mine.txt");
    assert_eq!(
        *sent.borrow(),
        [(
            "blocker.custom_lists".to_owned(),
            "http://lists.test/mine.txt".to_owned()
        )]
    );
    assert_eq!(
        field(&app).accessible_value().as_deref(),
        Some("http://lists.test/mine.txt"),
        "a refused address stays to be fixed"
    );
    assert!(note_shown(&app), "the refusal is said under the field");

    submit(&app, "https://lists.test/mine.txt");
    assert_eq!(sent.borrow().len(), 2);
    assert_eq!(
        field(&app).accessible_value().as_deref(),
        Some(""),
        "a taken one empties the field"
    );
    assert!(!note_shown(&app));
}
