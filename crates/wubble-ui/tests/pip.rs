//! The picture-in-picture window's controls and the Main window's card on the testing backend:
//! what a screen reader is told about their buttons and that each button's default action (and
//! a pointer click) reaches `pip-command` with the host's names. One test (one platform per
//! process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use wubble_ui::{AppWindow, SidebarMode, VideoView};

fn button(app: &AppWindow, label: &str) -> ElementHandle {
    let mut found: Vec<_> = ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.accessible_role() == Some(AccessibleRole::Button))
        .collect();
    assert_eq!(found.len(), 1, "one button labelled {label:?}");
    found.remove(0)
}

fn pump() {
    mock_elapsed_time(Duration::from_millis(400));
}

fn logged(app: &AppWindow) -> Rc<RefCell<Vec<(String, f32)>>> {
    let log: Rc<RefCell<Vec<(String, f32)>>> = Rc::default();
    let l = log.clone();
    app.on_pip_command(move |name, value| l.borrow_mut().push((name.to_string(), value)));
    log
}

#[test]
fn the_pip_controls_and_card_name_their_buttons_and_send_intents() {
    init_integration_test_with_mock_time();

    // The Pip window: the controls show while the pointer is over the window.
    let pip = AppWindow::new().unwrap();
    pip.set_window_kind(3);
    // as the host opens one (`WindowFactory::open`: Little, Popup and Pip windows show their
    // page alone, the sidebar hidden, so no seam lies over the controls)
    pip.set_sidebar_mode(SidebarMode::Hidden);
    pip.window().set_size(LogicalSize::new(400.0, 225.0));
    assert!(pip.get_pip());
    pip.set_video(VideoView {
        paused: true,
        ..VideoView::default()
    });
    let log = logged(&pip);
    pip.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(200.0, 100.0),
    });
    pump();
    for (label, name, value) in [
        ("Back 10 seconds", "seek-by", -10.0),
        ("Play", "play", 0.0),
        ("Forward 10 seconds", "seek-by", 10.0),
        ("Back to tab", "return", 0.0),
        ("Close picture-in-picture", "close", 0.0),
    ] {
        button(&pip, label).invoke_accessible_default_action();
        pump();
        assert_eq!(
            log.borrow_mut().pop(),
            Some((name.to_string(), value)),
            "{label}"
        );
    }
    // A pointer click on the strip's button does the same.
    let close = button(&pip, "Close picture-in-picture");
    let (p, s) = (close.absolute_position(), close.size());
    let at = LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0);
    pip.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    for e in [
        WindowEvent::PointerPressed {
            position: at,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position: at,
            button: PointerEventButton::Left,
        },
    ] {
        pip.window().dispatch_event(e);
    }
    pump();
    assert_eq!(log.borrow_mut().pop(), Some(("close".to_string(), 0.0)));
    // No card in the Pip window, no video strip either.
    assert_eq!(
        ElementHandle::find_by_accessible_label(&pip, "Picture in picture").count(),
        0
    );

    // A Main window: no controls; the card only while its tab is out.
    let main = AppWindow::new().unwrap();
    main.window().set_size(LogicalSize::new(1280.0, 800.0));
    assert!(!main.get_pip());
    let log = logged(&main);
    pump();
    assert_eq!(
        ElementHandle::find_by_accessible_label(&main, "Back to tab").count(),
        0
    );
    main.set_pip_card(true);
    main.set_pip_title("A video".into());
    pump();
    button(&main, "Back to tab").invoke_accessible_default_action();
    pump();
    assert_eq!(log.borrow_mut().pop(), Some(("return".to_string(), 0.0)));
    assert_eq!(
        ElementHandle::find_by_accessible_label(&main, "Close picture-in-picture").count(),
        0
    );
}
