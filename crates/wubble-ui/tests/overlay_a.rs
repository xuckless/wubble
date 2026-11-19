//! Overlays A on the testing backend (kit/overlay/a): the find bar, the permission prompt, the
//! autofill chip, menu and save prompt, the downloads popover and the error page answer through
//! their kit buttons (found by accessible label) and keys, with the intents the host expects.
//! Mock time only, no timing asserted: time is run past every entrance so the panels are open.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString, VecModel};
use wubble_ui::{
    AppWindow, AutofillAnswer, AutofillView, DownloadAction, DownloadItem, DownloadStatus,
    ErrorView, FindView, PermissionView,
};

fn settle() {
    mock_elapsed_time(Duration::from_millis(600));
}

fn click(app: &AppWindow, label: &str) {
    ElementHandle::find_by_accessible_label(app, label)
        .find(|e| e.computed_opacity() > 0.0)
        .unwrap_or_else(|| panic!("no visible {label}"))
        .mock_single_click(PointerEventButton::Left);
}

fn has(app: &AppWindow, label: &str) -> bool {
    ElementHandle::find_by_accessible_label(app, label).any(|e| e.computed_opacity() > 0.0)
}

fn key(app: &AppWindow, k: impl Into<SharedString> + Clone) {
    let text: SharedString = k.into();
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn strings(v: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        v.iter().map(|s| SharedString::from(*s)).collect::<Vec<_>>(),
    ))
}

fn find_bar(app: &AppWindow) {
    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    app.on_find_step(move |f| l.borrow_mut().push(format!("step {f}")));
    let l = log.clone();
    app.on_find_closed(move || l.borrow_mut().push("closed".into()));
    let l = log.clone();
    app.on_find_edited(move |q| l.borrow_mut().push(format!("edit {q}")));
    app.set_find(FindView {
        open: true,
        query: "wubble".into(),
        counter: "0 of 0".into(),
        focus_serial: 1,
    });
    settle();
    assert!(has(app, "0 of 0"), "the count is the host's counter");
    click(app, "Next match");
    click(app, "Previous match");
    click(app, "Close find");
    assert_eq!(*log.borrow(), ["step true", "step false", "closed"]);
    log.borrow_mut().clear();
    // keys in the field: Return, Shift+Return, Escape
    click(app, "Find in page");
    key(app, Key::Return);
    app.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    key(app, Key::Return);
    app.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    key(app, Key::Escape);
    assert_eq!(*log.borrow(), ["step true", "step false", "closed"]);
    app.set_find(FindView::default());
    settle();
    assert!(!has(app, "Next match"), "closed, the bar is gone");
}

fn permission(app: &AppWindow) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    app.on_permission_answered(move |allow, remember| l.borrow_mut().push((allow, remember)));
    let ask = |serial| PermissionView {
        open: true,
        title: "example.com wants to".into(),
        lines: strings(&["Use your camera"]),
        remember: "Remember for this site".into(),
        allow: "Allow".into(),
        block: "Block".into(),
        serial,
    };
    app.set_permission(ask(1));
    settle();
    click(app, "Remember for this site");
    click(app, "Allow");
    app.set_permission(ask(2));
    settle();
    click(app, "Block");
    assert_eq!(*log.borrow(), [(true, true), (false, false)]);
    app.set_permission(PermissionView::default());
    settle();
    assert!(!has(app, "Allow"));
}

fn autofill(app: &AppWindow) {
    let menu = Rc::new(RefCell::new(Vec::new()));
    let m = menu.clone();
    app.on_autofill_menu(move |i| m.borrow_mut().push(i));
    let answers = Rc::new(RefCell::new(Vec::new()));
    let a = answers.clone();
    app.on_autofill_answer(move |x| a.borrow_mut().push(x));
    let view = |open: bool| AutofillView {
        chip: true,
        menu_open: open,
        rows: strings(&["alice@example.com"]),
        ..Default::default()
    };
    app.set_autofill(view(false));
    settle();
    click(app, "Passwords");
    app.set_autofill(view(true));
    settle();
    click(app, "alice@example.com");
    click(app, "Passwords");
    assert_eq!(*menu.borrow(), [-1, 0, -2]);
    app.set_autofill(AutofillView {
        prompt: true,
        title: "Save password?".into(),
        detail: "alice@example.com on example.com".into(),
        accept: "Save".into(),
        ..Default::default()
    });
    settle();
    for label in ["Never", "Not now", "Save"] {
        click(app, label);
    }
    assert_eq!(
        *answers.borrow(),
        [
            AutofillAnswer::Never,
            AutofillAnswer::NotNow,
            AutofillAnswer::Save
        ]
    );
    app.set_autofill(AutofillView::default());
    settle();
}

fn downloads(app: &AppWindow) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    app.on_download_action(move |id, a| l.borrow_mut().push((id, a)));
    app.set_downloads(ModelRc::new(VecModel::from(vec![
        DownloadItem {
            id: 1,
            name: "big.iso".into(),
            detail: "12 MB of 40 MB".into(),
            progress: 0.3,
            status: DownloadStatus::Running,
        },
        DownloadItem {
            id: 2,
            name: "notes.pdf".into(),
            detail: "Done".into(),
            progress: 1.0,
            status: DownloadStatus::Complete,
        },
    ])));
    app.set_downloads_open(true);
    settle();
    click(app, "Pause");
    click(app, "Cancel");
    click(app, "Show in folder");
    click(app, "Remove from list");
    assert_eq!(
        *log.borrow(),
        [
            (1, DownloadAction::Pause),
            (1, DownloadAction::Cancel),
            (2, DownloadAction::Reveal),
            (2, DownloadAction::Remove)
        ]
    );
    // a press outside the panel closes it
    let corner = LogicalPosition::new(1270.0, 790.0);
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: corner,
        button: PointerEventButton::Left,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: corner,
        button: PointerEventButton::Left,
    });
    assert!(!app.get_downloads_open());
    settle();
}

fn error_page(app: &AppWindow) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    app.on_error_primary(move || l.borrow_mut().push("primary"));
    let l = log.clone();
    app.on_error_secondary(move || l.borrow_mut().push("secondary"));
    app.set_page_error(ErrorView {
        open: true,
        title: "This site can't be reached".into(),
        summary: "example.invalid's server could not be found.".into(),
        detail: "ERR_NAME_NOT_RESOLVED".into(),
        primary: "Reload".into(),
        secondary: "Go back".into(),
        cert: false,
    });
    settle();
    click(app, "Reload");
    click(app, "Go back");
    // the detail folds under Details (closed for a load error)
    click(app, "Details");
    settle();
    assert!(has(app, "Hide details"));
    assert_eq!(*log.borrow(), ["primary", "secondary"]);
    app.set_page_error(ErrorView::default());
    settle();
}

#[test]
fn overlays_a_answer_through_the_kit() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    settle();
    find_bar(&app);
    permission(&app);
    autofill(&app);
    downloads(&app);
    error_page(&app);
}
