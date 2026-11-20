//! The agent approval sheet on the testing backend, wired as the host wires a window (the
//! controls registry after the window is built), driven by pointer events through the window:
//! the Window choice shows the client's choice and follows a press; the Secrets list removes a
//! name, the Sites list removes, adds and edits a site (the host's answers are the callbacks
//! here); Allow and Deny answer with the choice shown, in a window of the least size too (the
//! fields scroll). Mock time only, nothing timed.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString, VecModel};
use wubble_ui::{AgentSheetView, AppWindow, EntryItem, driver};

fn settle() {
    mock_elapsed_time(Duration::from_millis(600));
}

fn rows(v: &[&str]) -> ModelRc<EntryItem> {
    let items: Vec<EntryItem> = v
        .iter()
        .map(|s| EntryItem {
            label: (*s).into(),
            key: (*s).into(),
            cells: ModelRc::new(VecModel::from(vec![SharedString::from(*s)])),
            ..Default::default()
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

fn root(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(app, "AppWindow::agent-sheet")
        .next()
        .expect("the agent sheet")
}

fn centre(e: &ElementHandle) -> LogicalPosition {
    let (p, s) = (e.absolute_position(), e.size());
    LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
}

fn click(app: &AppWindow, at: LogicalPosition) {
    let button = PointerEventButton::Left;
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    settle();
}

/// The sheet's elements with `role` and `label`, in tree order.
fn find(app: &AppWindow, role: AccessibleRole, label: &str) -> Vec<ElementHandle> {
    root(app)
        .query_descendants()
        .match_accessible_role(role)
        .match_accessible_label(label)
        .find_all()
}

fn press(app: &AppWindow, role: AccessibleRole, label: &str, nth: usize) {
    let e = find(app, role, label)
        .into_iter()
        .nth(nth)
        .unwrap_or_else(|| panic!("no {label} ({nth}) on the sheet"));
    click(app, centre(&e));
}

fn chosen(app: &AppWindow) -> Option<String> {
    root(app)
        .query_descendants()
        .match_accessible_role(AccessibleRole::Tab)
        .find_all()
        .into_iter()
        .find(|e| e.accessible_item_selected() == Some(true))
        .and_then(|e| e.accessible_label())
        .map(|l| l.to_string())
}

fn segment(app: &AppWindow, i: usize) -> LogicalPosition {
    let segs = root(app)
        .query_descendants()
        .match_id("SegRow::seg")
        .find_all();
    centre(&segs[i])
}

fn type_text(app: &AppWindow, text: &str) {
    for c in text.chars() {
        let text = SharedString::from(c.to_string());
        app.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        app.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
    let enter = SharedString::from(Key::Return);
    app.window().dispatch_event(WindowEvent::KeyPressed {
        text: enter.clone(),
    });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text: enter });
    settle();
}

#[derive(Debug, PartialEq)]
enum Said {
    Answered(bool, bool),
    Removed(bool, String),
    Site(String),
}

#[test]
fn the_sheet_answers_to_the_pointer() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    // wired after the window is built, as the host does
    let _wired = driver::wire(&app, wubble_core::seam::Seam::new());
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 1400.0));
    settle();

    let said = Rc::new(RefCell::new(Vec::<Said>::new()));
    let s = said.clone();
    app.on_agent_sheet_answered(move |allow, headless| {
        s.borrow_mut().push(Said::Answered(allow, headless))
    });
    let s = said.clone();
    app.on_agent_sheet_removed(move |site, key| {
        s.borrow_mut().push(Said::Removed(site, key.to_string()))
    });
    let s = said.clone();
    app.on_agent_sheet_site_submitted(move |text| {
        s.borrow_mut().push(Said::Site(text.to_string()))
    });

    let view = |serial, headless| AgentSheetView {
        open: true,
        serial,
        client: "Claude Code".into(),
        secrets: rows(&["GITHUB_TOKEN", "NPM_TOKEN"]),
        sites: rows(&["github.com"]),
        headless,
        ..Default::default()
    };
    app.set_agent_sheet_view(view(1, true));
    settle();

    // the client's choice shows, and a press on Windowed takes it
    assert_eq!(chosen(&app).as_deref(), Some("Headless"));
    click(&app, segment(&app, 0));
    assert_eq!(
        chosen(&app).as_deref(),
        Some("Windowed"),
        "a press on Windowed"
    );

    // a secret name: selected, then "−" (the Secrets editor's is the first)
    press(&app, AccessibleRole::ListItem, "NPM_TOKEN", 0);
    press(&app, AccessibleRole::Button, "Remove", 0);
    // a site: selected, then the Sites editor's "−"
    press(&app, AccessibleRole::ListItem, "github.com", 0);
    press(&app, AccessibleRole::Button, "Remove", 1);
    // a new site: the Sites editor's "+" (the last: the Secrets editor's adds nothing), typed,
    // Enter
    let adds = find(&app, AccessibleRole::Button, "Add").len();
    press(&app, AccessibleRole::Button, "Add", adds - 1);
    type_text(&app, "example.com");
    assert_eq!(
        said.borrow().as_slice(),
        [
            Said::Removed(false, "NPM_TOKEN".into()),
            Said::Removed(true, "github.com".into()),
            Said::Site("example.com".into()),
        ]
    );

    // the host's next view (same serial) keeps the window choice; Allow answers with it
    said.borrow_mut().clear();
    app.set_agent_sheet_view(AgentSheetView {
        sites: rows(&["example.com"]),
        secrets: rows(&["GITHUB_TOKEN"]),
        answer: 1,
        ..view(1, true)
    });
    settle();
    assert_eq!(chosen(&app).as_deref(), Some("Windowed"));
    press(&app, AccessibleRole::Button, "Allow", 0);
    assert_eq!(said.borrow().as_slice(), [Said::Answered(true, false)]);

    // the next sheet restarts from its client's choice; Deny refuses
    said.borrow_mut().clear();
    app.set_agent_sheet_view(AgentSheetView::default());
    settle();
    app.set_agent_sheet_view(view(2, true));
    settle();
    assert_eq!(chosen(&app).as_deref(), Some("Headless"));
    press(&app, AccessibleRole::Button, "Deny", 0);
    assert_eq!(said.borrow().as_slice(), [Said::Answered(false, true)]);

    // the window at its least size: the fields scroll, the buttons stay in reach
    said.borrow_mut().clear();
    app.set_agent_sheet_view(AgentSheetView::default());
    settle();
    app.window().set_size(LogicalSize::new(560.0, 380.0));
    app.set_agent_sheet_view(view(3, false));
    settle();
    press(&app, AccessibleRole::Button, "Allow", 0);
    assert_eq!(said.borrow().as_slice(), [Said::Answered(true, false)]);
}
