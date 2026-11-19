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
use wubble_ui::{AppWindow, DialogView, ErrorView, driver};

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

#[test]
fn exits_keep_their_view() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    sheet_leaves_with_its_view(&app);
    error_panel_leaves_with_its_view(&app);
    outgoing_page_plays_and_is_dropped(&app);
}
