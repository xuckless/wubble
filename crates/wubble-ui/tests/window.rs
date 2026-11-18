//! The frameless window on the testing backend: the lights, the resize zones, the corner radius
//! and the loading sheen. Mock time only. The platform can be installed once per process, so the
//! scenarios run in order inside one test.

use std::{cell::RefCell, rc::Rc, time::Duration};

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, Global, LogicalPosition,
    platform::{PointerEventButton, WindowEvent},
};
use wubble_ui::{AppWindow, WindowEdge};

fn click(app: &AppWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    let w = app.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    mock_elapsed_time(Duration::from_millis(1));
    let button = PointerEventButton::Left;
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    mock_elapsed_time(Duration::from_millis(1));
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
    mock_elapsed_time(Duration::from_millis(1));
}

fn lights(app: &AppWindow) {
    // inset 8 + sidebar padding 6 + 10, lights 12 wide with gap 8; row 30 high.
    let y = 8.0 + 6.0 + 15.0;
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    app.on_window_close_requested(move || l.borrow_mut().push("close"));
    let l = log.clone();
    app.on_window_minimise_requested(move || l.borrow_mut().push("minimise"));
    let l = log.clone();
    app.on_window_toggle_fullscreen_requested(move || l.borrow_mut().push("fullscreen"));
    for x in [30.0, 50.0, 70.0] {
        click(app, x, y);
    }
    assert_eq!(*log.borrow(), ["close", "minimise", "fullscreen"]);
}

fn edges(app: &AppWindow) {
    let size = app.window().size().to_logical(app.window().scale_factor());
    let (w, h) = (size.width, size.height);
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    app.on_window_begin_resize(move |edge| l.borrow_mut().push(edge));
    let table = [
        (3.0, h / 2.0, WindowEdge::W),
        (w - 3.0, h / 2.0, WindowEdge::E),
        (w / 2.0, 3.0, WindowEdge::N),
        (w / 2.0, h - 3.0, WindowEdge::S),
        // A press in the 10 px corner square gives the corner, not the strip under it.
        (3.0, 3.0, WindowEdge::Nw),
        (8.0, 3.0, WindowEdge::Nw),
        (w - 3.0, 3.0, WindowEdge::Ne),
        (3.0, h - 3.0, WindowEdge::Sw),
        (w - 3.0, h - 3.0, WindowEdge::Se),
        (12.0, 3.0, WindowEdge::N),
        (w / 2.0, 2.0, WindowEdge::N),
    ];
    for (x, y, _) in table {
        click(app, x, y);
    }
    let want: Vec<_> = table.iter().map(|t| t.2).collect();
    assert_eq!(*log.borrow(), want);

    // Square until the host says the compositor blends.
    assert_eq!(app.get_window_radius(), 0.0);
    app.set_rounded_corners(true);
    assert!(app.get_window_resizable());
    assert_eq!(
        app.get_window_radius(),
        wubble_ui::Radius::get(app).get_window()
    );
    log.borrow_mut().clear();
    app.window().set_fullscreen(true);
    mock_elapsed_time(Duration::from_millis(1));
    assert!(!app.get_window_resizable());
    assert_eq!(app.get_window_radius(), 0.0);
    click(app, 3.0, h / 2.0);
    assert!(log.borrow().is_empty());
    app.window().set_fullscreen(false);
    mock_elapsed_time(Duration::from_millis(1));

    // Maximised: square, no resize zones.
    app.window().set_maximized(true);
    mock_elapsed_time(Duration::from_millis(1));
    assert!(!app.get_window_resizable());
    assert_eq!(app.get_window_radius(), 0.0);
    click(app, 3.0, h / 2.0);
    assert!(log.borrow().is_empty());
    app.window().set_maximized(false);
    mock_elapsed_time(Duration::from_millis(1));
    assert!(app.get_window_resizable());
}

fn drag(app: &AppWindow, x: f32, y: f32, dx: f32) {
    let w = app.window();
    let button = PointerEventButton::Left;
    let at = |x: f32| LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position: at(x) });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at(x),
        button,
    });
    mock_elapsed_time(Duration::from_millis(1));
    w.dispatch_event(WindowEvent::PointerMoved {
        position: at(x + dx),
    });
    mock_elapsed_time(Duration::from_millis(1));
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at(x + dx),
        button,
    });
    // Far from the last press, so the next one is no double-click.
    mock_elapsed_time(Duration::from_millis(1000));
}

/// Bare chrome moves the window once a press travels; the thin top strip still resizes.
fn moving(app: &AppWindow) {
    let size = app.window().size().to_logical(app.window().scale_factor());
    let log = Rc::new(RefCell::new(Vec::new()));
    let l = log.clone();
    app.on_window_begin_move(move || l.borrow_mut().push("move"));
    let l = log.clone();
    app.on_window_begin_resize(move |_| l.borrow_mut().push("resize"));
    let l = log.clone();
    app.on_window_toggle_maximise_requested(move || l.borrow_mut().push("maximise"));
    // The top gutter over the page, below the 5 px resize strip.
    drag(app, size.width / 2.0, 6.0, 20.0);
    assert_eq!(*log.borrow(), ["move"]);
    // The sidebar's glass beside the lights, inside its padding.
    log.borrow_mut().clear();
    drag(app, 120.0, 10.0, 20.0);
    assert_eq!(*log.borrow(), ["move"]);
    // A press that does not travel is a click, not a move.
    log.borrow_mut().clear();
    drag(app, 120.0, 10.0, 2.0);
    assert!(log.borrow().is_empty());
    // At the very top it resizes.
    drag(app, size.width / 2.0, 2.0, 20.0);
    assert_eq!(*log.borrow(), ["resize"]);
    // A double-click on bare chrome maximises.
    log.borrow_mut().clear();
    click(app, size.width / 2.0, 6.0);
    click(app, size.width / 2.0, 6.0);
    assert_eq!(*log.borrow(), ["maximise"]);
}

fn loading(app: &AppWindow) {
    assert_eq!(app.get_address_sheen(), 0.0);
    app.set_address_loading(true);
    mock_elapsed_time(Duration::from_millis(200));
    assert!(app.get_address_sheen() > 0.99);
    app.set_address_loading(false);
    mock_elapsed_time(Duration::from_millis(750));
    let mid = app.get_address_sheen();
    assert!(mid > 0.0 && mid < 1.0);
    mock_elapsed_time(Duration::from_millis(1500));
    assert_eq!(app.get_address_sheen(), 0.0);
}

#[test]
fn window_frame() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    mock_elapsed_time(Duration::from_millis(1));
    lights(&app);
    edges(&app);
    moving(&app);
    loading(&app);
}
