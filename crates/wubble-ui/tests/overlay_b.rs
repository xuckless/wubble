//! Overlays B on the testing backend (kit/overlay/b): the Boost editor's tint shows "no tint"
//! (no swatch checked while it is off, the Tint toggle asks for the shown colour), and the print
//! sheet keeps its body while it plays its exit after the host's empty closing view. Mock time
//! only, no timing asserted: time is run past every entrance so the panels are open.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, PointerEventButton};
use slint::{ComponentHandle, LogicalSize, ModelRc, SharedString, VecModel};
use wubble_ui::{AppWindow, BoostView, PrintSheetView};

const SWATCHES: [&str; 6] = ["Sky", "Lilac", "Blush", "Mint", "Amber", "Abyss"];

fn settle() {
    mock_elapsed_time(Duration::from_millis(600));
}

fn shown(app: &AppWindow, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.computed_opacity() > 0.0)
        .collect()
}

fn checked_swatches(app: &AppWindow) -> Vec<&'static str> {
    SWATCHES
        .into_iter()
        .filter(|name| {
            shown(app, name)
                .iter()
                .any(|e| e.accessible_checked() == Some(true))
        })
        .collect()
}

// the Tint row's toggle (the row's title says Tint too, and is not checkable)
fn tint_toggle(app: &AppWindow) -> Vec<ElementHandle> {
    let toggles: Vec<_> = shown(app, "Tint")
        .into_iter()
        .filter(|e| e.accessible_checked().is_some())
        .collect();
    assert_eq!(toggles.len(), 1, "one Tint toggle");
    toggles
}

fn boost_tint(app: &AppWindow) {
    let asked = Rc::new(RefCell::new(Vec::<slint::Color>::new()));
    let a = asked.clone();
    app.on_boost_tint(move |c| a.borrow_mut().push(c));
    let colour = slint::Color::from_rgb_u8(0x7c, 0xc4, 0xff);
    // off: a new Boost (the host's default colour), and one whose tint was turned off
    app.set_boost(BoostView {
        open: true,
        serial: 1,
        is_new: true,
        tint_on: false,
        tint: colour,
        ..Default::default()
    });
    settle();
    assert_eq!(
        checked_swatches(app),
        Vec::<&str>::new(),
        "off: no swatch is checked"
    );
    let toggle = tint_toggle(app);
    assert_eq!(toggle[0].accessible_checked(), Some(false));
    toggle[0].invoke_accessible_default_action();
    assert_eq!(*asked.borrow(), [colour], "on asks for the shown colour");

    assert!(
        shown(app, "Tint colour").is_empty(),
        "no swatches while off"
    );

    // on, at one of the swatches: that one is checked, and the toggle is on
    app.set_boost(BoostView {
        open: true,
        serial: 1,
        tint_on: true,
        tint: slint::Color::from_argb_u8(0, 0, 0, 0),
        strength: 20.0,
        ..Default::default()
    });
    settle();
    shown(app, "Lilac")[0].invoke_accessible_default_action();
    let picked = *asked.borrow().last().unwrap();
    app.set_boost(BoostView {
        open: true,
        serial: 1,
        tint_on: true,
        tint: picked,
        strength: 20.0,
        ..Default::default()
    });
    settle();
    assert_eq!(checked_swatches(app), ["Lilac"]);
    assert_eq!(tint_toggle(app)[0].accessible_checked(), Some(true));

    // the chosen swatch again turns the tint off (the host's rule): nothing stays checked
    app.set_boost(BoostView {
        open: true,
        serial: 1,
        tint_on: false,
        tint: picked,
        strength: 20.0,
        ..Default::default()
    });
    settle();
    assert_eq!(
        checked_swatches(app),
        Vec::<&str>::new(),
        "off again: no swatch is checked"
    );
    app.set_boost(BoostView::default());
    settle();
}

fn key(app: &AppWindow, text: SharedString) {
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    mock_elapsed_time(Duration::from_millis(16));
}

/// Escape in a Boost field leaves the field (keys typed after it edit nothing) and asks for no
/// action: closing would discard the unsaved draft.
fn boost_escape_keeps_the_draft(app: &AppWindow) {
    let acts = Rc::new(RefCell::new(Vec::<String>::new()));
    let a = acts.clone();
    app.on_boost_act(move |name| a.borrow_mut().push(name.into()));
    let edits = Rc::new(RefCell::new(Vec::<String>::new()));
    let e = edits.clone();
    app.on_boost_text(move |field, _| e.borrow_mut().push(field.into()));
    app.set_boost(BoostView {
        open: true,
        serial: 2,
        is_new: true,
        ..Default::default()
    });
    settle();
    for (label, field) in [("Name", "name"), ("Sites", "site"), ("Font", "font")] {
        let input = shown(app, label)
            .into_iter()
            .find(|e| e.accessible_role() == Some(AccessibleRole::TextInput))
            .unwrap_or_else(|| panic!("the {label} field"));
        input.mock_single_click(PointerEventButton::Left);
        mock_elapsed_time(Duration::from_millis(16));
        key(app, "x".into());
        assert_eq!(
            edits.borrow().last().map(String::as_str),
            Some(field),
            "{label}: typed into"
        );
        let typed = edits.borrow().len();
        key(app, Key::Escape.into());
        assert!(
            acts.borrow().is_empty(),
            "{label}: Escape asked for {:?}",
            acts.borrow()
        );
        key(app, "y".into());
        assert_eq!(
            edits.borrow().len(),
            typed,
            "{label}: Escape left the field"
        );
    }
    app.set_boost(BoostView::default());
    settle();
}

fn print_sheet_exit(app: &AppWindow) {
    let names: Vec<SharedString> = vec!["Save as PDF".into(), "Office".into()];
    app.set_print_sheet_view(PrintSheetView {
        open: true,
        serial: 1,
        destinations: ModelRc::new(VecModel::from(names)),
        destination: 1,
        pages: "1-3".into(),
        paper: 1,
        scale: 100,
        copies: 2,
        ..Default::default()
    });
    settle();
    assert!(!shown(app, "Pages").is_empty(), "the settings card is up");
    assert!(!shown(app, "Copies").is_empty());
    // the host's closing view is empty; one frame into the exit the body is still the last one
    app.set_print_sheet_view(PrintSheetView::default());
    mock_elapsed_time(Duration::from_millis(16));
    assert!(
        !shown(app, "Pages").is_empty(),
        "the card stays through the exit"
    );
    assert!(
        !shown(app, "Copies").is_empty(),
        "with the last view's rows"
    );
    assert!(!shown(app, "Office").is_empty(), "and its destinations");
    settle();
}

#[test]
fn overlays_b_keep_their_states() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    // tall enough for the whole Boost column (the finder skips what a Flickable clips away)
    app.window().set_size(LogicalSize::new(1280.0, 1800.0));
    settle();
    boost_tint(&app);
    boost_escape_keeps_the_draft(&app);
    print_sheet_exit(&app);
}
