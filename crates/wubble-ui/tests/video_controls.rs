//! The video controls on the testing backend: the pointer on any of the strip's buttons is on
//! the strip (so the strip stays); the collapsed strip's tab is docked to the page pane's right
//! edge where the strip was, and the strip's close button dismisses on a click and hides the
//! site on a right click. One test (one platform per process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use wubble_ui::{AppWindow, VideoView};

/// `Spacing.s2`: where the strip and the tab sit below the video's top.
const SPACE_S2: f32 = 8.0;

fn view(open: bool, collapsed: bool) -> VideoView {
    VideoView {
        open,
        collapsed,
        x: 100.0,
        y: 80.0,
        w: 500.0,
        h: 300.0,
        // the pane runs on past the video
        pane_right: 900.0,
        pane_bottom: 700.0,
        rate: "1\u{d7}".into(),
        ..VideoView::default()
    }
}

fn element(app: &AppWindow, id: &str) -> ElementHandle {
    ElementHandle::find_by_element_id(app, id)
        .next()
        .unwrap_or_else(|| panic!("no {id}"))
}

fn centre(e: &ElementHandle) -> LogicalPosition {
    let (p, s) = (e.absolute_position(), e.size());
    LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
}

fn click(app: &AppWindow, at: LogicalPosition, button: PointerEventButton) {
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    mock_elapsed_time(Duration::from_millis(16));
}

#[test]
fn the_tab_docks_to_the_pane_edge_and_the_close_button_dismisses() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    let commands = Rc::new(RefCell::new(vec![]));
    let log = commands.clone();
    app.on_video_command(move |name, _| log.borrow_mut().push(name.to_string()));
    let over = Rc::new(RefCell::new(vec![]));
    let hovers = over.clone();
    app.on_video_hovered(move |on| hovers.borrow_mut().push(on));

    // The pointer onto the strip, then over each of its buttons: it stays on the strip
    // throughout (no `hovered(false)` between them). Under the buttons' touch areas the strip's
    // own lost its hover to each of them, and the strip went with the pointer still on it.
    app.set_video(view(true, false));
    mock_elapsed_time(Duration::from_millis(400));
    for id in [
        "play",
        "bar",
        "rate-button",
        "loop",
        "frame",
        "mute",
        "close",
    ] {
        let e = element(&app, &format!("VideoControls::{id}"));
        app.window().dispatch_event(WindowEvent::PointerMoved {
            position: centre(&e),
        });
        mock_elapsed_time(Duration::from_millis(200));
        assert_eq!(*over.borrow(), [true], "over {id}");
    }
    // Off the strip, beside it on the video: no longer over it.
    let video = element(&app, "VideoControls::root").absolute_position();
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(video.x + 50.0, video.y + 250.0),
    });
    mock_elapsed_time(Duration::from_millis(16));
    assert_eq!(*over.borrow(), [true, false]);

    // The strip open: its close button dismisses, a right click hides the site.
    app.set_video(view(true, false));
    mock_elapsed_time(Duration::from_millis(400));
    let close = element(&app, "VideoControls::close");
    click(&app, centre(&close), PointerButton::Left);
    click(&app, centre(&close), PointerButton::Right);
    assert_eq!(*commands.borrow(), ["dismiss", "hide-site"]);
    commands.borrow_mut().clear();

    // Collapsed: only the tab, its right edge on the pane's, level with the strip.
    app.set_video(view(false, true));
    mock_elapsed_time(Duration::from_millis(600));
    let tab = element(&app, "VideoControls::tab");
    let strip = element(&app, "VideoControls::strip");
    let controls = element(&app, "VideoControls::root");
    let origin = controls.absolute_position().x - 100.0;
    assert_eq!(
        tab.absolute_position().x + tab.size().width,
        origin + 900.0,
        "docked to the pane's right edge"
    );
    assert_eq!(
        tab.absolute_position().y,
        controls.absolute_position().y + SPACE_S2
    );
    assert_eq!(tab.size().height, strip.size().height);
    // A click on the tab (outside the video's own rectangle) opens the strip again.
    click(&app, centre(&tab), PointerButton::Left);
    assert!(commands.borrow().iter().any(|c| c == "expand"));
}

use PointerEventButton as PointerButton;
