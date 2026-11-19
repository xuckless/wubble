//! Exits on the testing backend: a sheet and the error panel keep their view while they leave
//! (the host has already cleared it) and answer nothing then; the outgoing page's copy plays
//! under the incoming one and is dropped. Mock time and explicit frame deltas; one test (one
//! platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, Image, LogicalSize, Rgba8Pixel, SharedPixelBuffer};
use wubble_ui::{AppWindow, DialogView, ErrorView, GlanceView, driver};

fn opacity_of(app: &AppWindow, label: &str) -> Option<f32> {
    ElementHandle::find_by_accessible_label(app, label)
        .next()
        .map(|e| e.computed_opacity())
}

fn sheet_leaves_with_its_view(app: &AppWindow) {
    let answered = Rc::new(RefCell::new(0));
    let a = answered.clone();
    app.on_dialog_answered(move |_, _, _| *a.borrow_mut() += 1);
    app.set_dialog(DialogView {
        open: true,
        title: "Leave site?".into(),
        accept: "Leave".into(),
        cancel: "Stay".into(),
        serial: 1,
        ..Default::default()
    });
    mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(opacity_of(app, "Leave site?"), Some(1.0));
    // the host closes it and clears the view
    app.set_dialog(DialogView {
        serial: 1,
        ..Default::default()
    });
    mock_elapsed_time(Duration::from_millis(60));
    let leaving = opacity_of(app, "Leave site?").expect("the title is still drawn");
    assert!(leaving > 0.0 && leaving < 1.0, "{leaving}");
    // a click on its button while it leaves answers nothing
    if let Some(button) = ElementHandle::find_by_accessible_label(app, "Leave").next() {
        button.mock_single_click(PointerEventButton::Left);
    }
    assert_eq!(*answered.borrow(), 0);
    mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(opacity_of(app, "Leave site?").unwrap_or(0.0), 0.0);
}

fn error_panel_leaves_with_its_view(app: &AppWindow) {
    app.set_page_error(ErrorView {
        open: true,
        title: "This site can't be reached".into(),
        primary: "Reload".into(),
        ..Default::default()
    });
    mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(opacity_of(app, "This site can't be reached"), Some(1.0));
    app.set_page_error(ErrorView::default());
    mock_elapsed_time(Duration::from_millis(60));
    let leaving = opacity_of(app, "This site can't be reached").expect("still drawn");
    assert!(leaving > 0.0 && leaving < 1.0, "{leaving}");
    mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(
        opacity_of(app, "This site can't be reached").unwrap_or(0.0),
        0.0
    );
}

fn outgoing_page_plays_and_is_dropped(app: &AppWindow) {
    let wired = driver::wire(app, wubble_core::seam::Seam::new());
    let driver = wired.driver;
    app.set_active_tab_id(1);
    driver.borrow_mut().advance(app, 0.016);
    let pixels = SharedPixelBuffer::<Rgba8Pixel>::new(4, 4);
    app.set_exit_page(Image::from_rgba8(pixels));
    app.set_active_tab_id(2);
    driver.borrow_mut().advance(app, 0.016);
    assert_eq!(app.get_exit_t(), 0.0, "the switch starts it");
    driver.borrow_mut().advance(app, 0.016);
    let t = app.get_exit_t();
    assert!(t > 0.0 && t < 1.0, "{t}");
    assert!(app.get_exit_page().size().width > 0);
    for _ in 0..30 {
        driver.borrow_mut().advance(app, 0.016);
    }
    assert_eq!(app.get_exit_t(), 1.0);
    assert_eq!(app.get_exit_page().size().width, 0, "dropped once gone");
}

fn glance_card_promotes_closes_and_leaves(app: &AppWindow) {
    let log = Rc::new(RefCell::new(vec![]));
    let l = log.clone();
    app.on_glance_to_tab(move || l.borrow_mut().push("tab"));
    let l = log.clone();
    app.on_glance_to_split(move || l.borrow_mut().push("split"));
    let l = log.clone();
    app.on_glance_closed(move || l.borrow_mut().push("closed"));
    let pressed = Rc::new(RefCell::new(vec![]));
    let p = pressed.clone();
    app.on_web_pointer_pressed(move |pane, _, _, _| p.borrow_mut().push(pane));
    app.set_glance(GlanceView {
        open: true,
        id: 7,
        title: "Peeked page".into(),
        x: 100.0,
        card_y: 40.0,
        y: 84.0,
        width: 800.0,
        height: 500.0,
        ..Default::default()
    });
    mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(opacity_of(app, "Peeked page"), Some(1.0));
    for label in ["Open as tab", "Open in split"] {
        ElementHandle::find_by_accessible_label(app, label)
            .next()
            .unwrap_or_else(|| panic!("{label}"))
            .mock_single_click(PointerEventButton::Left);
    }
    assert_eq!(*log.borrow(), ["tab", "split"]);
    // a press on its page is the peek's page's own (its pane id)
    let title = ElementHandle::find_by_accessible_label(app, "Peeked page")
        .next()
        .unwrap();
    let at = title.absolute_position();
    app.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(at.x + 50.0, at.y + 200.0),
            button: PointerEventButton::Left,
        });
    app.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(at.x + 50.0, at.y + 200.0),
            button: PointerEventButton::Left,
        });
    assert_eq!(*pressed.borrow(), [7]);
    // the host closes it: it leaves with its view
    app.set_glance(GlanceView::default());
    mock_elapsed_time(Duration::from_millis(60));
    let leaving = opacity_of(app, "Peeked page").expect("still drawn");
    assert!(leaving > 0.0 && leaving < 1.0, "{leaving}");
    mock_elapsed_time(Duration::from_millis(400));
    assert_eq!(opacity_of(app, "Peeked page").unwrap_or(0.0), 0.0);
}

#[test]
fn exits_keep_their_view() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    sheet_leaves_with_its_view(&app);
    error_panel_leaves_with_its_view(&app);
    glance_card_promotes_closes_and_leaves(&app);
    outgoing_page_plays_and_is_dropped(&app);
}
