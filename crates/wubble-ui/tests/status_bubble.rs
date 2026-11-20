//! The link status bubble on the testing backend: its glass capsule holds its text. A long
//! address is cut inside the capsule, never drawn past its end, and a short one gets a capsule
//! its own width, also in the shell's pane where the address arrives after the bubble was made. Mock time only; one test (one platform per process).

use std::time::Duration;

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize};

slint::slint! {
    #[edition = "1.19"]
    import { AStatusBubble } from "ui/kit/overlay/a/a-status-bubble.slint";
    export component BubbleSpecimen inherits Window {
        in property <string> host;
        in property <string> rest;
        in property <bool> shown;
        width: 900px;
        height: 600px;
        bubble := AStatusBubble {
            shown: root.shown;
            host: root.host;
            rest: root.rest;
            pane-width: root.width;
            pane-height: root.height;
        }
    }
}

struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn rect(e: &ElementHandle) -> Rect {
    let p = e.absolute_position();
    let s = e.size();
    Rect {
        x: p.x,
        y: p.y,
        w: s.width,
        h: s.height,
    }
}

fn one(app: &BubbleSpecimen, id: &str) -> Rect {
    rect(
        &ElementHandle::find_by_element_id(app, id)
            .next()
            .unwrap_or_else(|| panic!("no {id}")),
    )
}

fn check(app: &BubbleSpecimen, host: &str, rest: &str) {
    app.set_host(host.into());
    app.set_rest(rest.into());
    mock_elapsed_time(Duration::from_millis(600));
    let bubble = one(app, "BubbleSpecimen::bubble");
    let panel = one(app, "AStatusBubble::panel");
    let row = one(app, "AStatusBubble::row");
    let eps = 0.5;
    assert!(
        (panel.x - bubble.x).abs() < eps
            && panel.w <= bubble.w + eps
            && (panel.h - bubble.h).abs() < eps,
        "{host}{rest}: the capsule starts the bubble's box and fits in it: panel {},{} {}x{} bubble {},{} {}x{}",
        panel.x,
        panel.y,
        panel.w,
        panel.h,
        bubble.x,
        bubble.y,
        bubble.w,
        bubble.h
    );
    assert!(
        row.x >= panel.x - eps && row.x + row.w <= panel.x + panel.w + eps,
        "{host}{rest}: the text row lies in the capsule: row {} +{} panel {} +{}",
        row.x,
        row.w,
        panel.x,
        panel.w
    );
    for id in ["AStatusBubble::host-text", "AStatusBubble::rest-text"] {
        let t = one(app, id);
        assert!(
            t.x >= panel.x - eps && t.x + t.w <= panel.x + panel.w + eps,
            "{host}{rest}: {id} lies in the capsule: {} +{} in {} +{}",
            t.x,
            t.w,
            panel.x,
            panel.w
        );
        assert!(t.y >= panel.y - eps && t.y + t.h <= panel.y + panel.h + eps);
    }
    assert!(
        panel.w <= 900.0 / 3.0 + 20.0 + eps,
        "at most a third of the pane"
    );
    let host_text = one(app, "AStatusBubble::host-text");
    assert!(
        host_text.w > 20.0,
        "{host}{rest}: the host shows, not cut to nothing: {}",
        host_text.w
    );
}

#[test]
fn the_capsule_holds_its_text() {
    init_integration_test_with_mock_time();
    let app = BubbleSpecimen::new().unwrap();
    app.window().set_size(LogicalSize::new(900.0, 600.0));
    // as the host does: drawn hidden and empty first, then an address arrives and shows
    mock_elapsed_time(Duration::from_millis(600));
    app.set_shown(true);
    check(&app, "github.com", "/xuckless");
    check(
        &app,
        "github.com",
        "/xuckless/the-wallpaper-project/blob/main/src/wubble/pages/settings.tsx",
    );
    check(
        &app,
        "a-very-long-host-name-that-does-not-fit-anywhere.example.com",
        "/x",
    );
    in_the_shell();
}

/// The bubble as the shell places it: in a page pane filling the content area.
fn in_the_shell() {
    let app = wubble_ui::AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1360.0, 880.0));
    let pane = wubble_ui::SplitPane {
        id: 1,
        x0: 0.0,
        x1: 1084.0,
        focused: true,
        status_shown: true,
        status_host: "github.com".into(),
        status_rest: "/xuckless/the-wallpaper-project".into(),
        ..Default::default()
    };
    app.set_panes(slint::ModelRc::new(slint::VecModel::from(vec![pane])));
    mock_elapsed_time(Duration::from_millis(600));
    let panel = ElementHandle::find_by_element_id(&app, "AStatusBubble::panel")
        .next()
        .expect("the bubble");
    let host = ElementHandle::find_by_element_id(&app, "AStatusBubble::host-text")
        .next()
        .unwrap();
    assert!(
        panel.size().width > 100.0 && host.size().width > 20.0,
        "the bubble in a pane: {:?}, host {:?}",
        panel.size(),
        host.size()
    );
}
