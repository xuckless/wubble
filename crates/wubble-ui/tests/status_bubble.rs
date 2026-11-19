//! The link status bubble on the testing backend: its glass capsule holds its text. A long
//! address is cut inside the capsule, never drawn past its end, and the capsule is the bubble's
//! own box. Mock time only; one test (one platform per process).

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
        width: 900px;
        height: 600px;
        bubble := AStatusBubble {
            shown: true;
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
            && (panel.w - bubble.w).abs() < eps
            && (panel.h - bubble.h).abs() < eps,
        "{host}{rest}: the capsule is the bubble's box: panel {},{} {}x{} bubble {},{} {}x{}",
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
        bubble.w <= 900.0 / 3.0 + 20.0 + eps,
        "at most a third of the pane"
    );
}

#[test]
fn the_capsule_holds_its_text() {
    init_integration_test_with_mock_time();
    let app = BubbleSpecimen::new().unwrap();
    app.window().set_size(LogicalSize::new(900.0, 600.0));
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
}
