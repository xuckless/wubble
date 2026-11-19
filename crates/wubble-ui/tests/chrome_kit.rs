//! The chrome's kit parts (`ui/kit/chrome/`) on the testing backend, against the web design's
//! rules: the address pill's press and its open origin. Mock time; tweens are let run out,
//! nothing is timed against the wall clock. One test (one platform per process).

use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{PointerEventButton, WindowEvent},
};

slint::slint! {
    #[edition = "1.19"]
    import { AddressPill } from "ui/kit/chrome/address-pill.slint";
    export component Probe inherits Window {
        width: 400px;
        height: 300px;
        in property <bool> compact;
        pill := AddressPill { x: 20px; y: 50px; width: 240px; compact: root.compact; host: "example.com"; path: "/a"; }
        out property <float> pill-press: pill.press-scale;
        out property <length> origin-x: pill.origin.x;
        out property <length> origin-y: pill.origin.y;
    }
}

/// Past the longest tween, so every value is at its target.
fn run_out() {
    mock_elapsed_time(Duration::from_millis(600));
}

fn at(p: &Probe, event: WindowEvent) {
    p.window().dispatch_event(event);
    mock_elapsed_time(Duration::from_millis(1));
}

/// `.wb-address:active { transform: scale(.985) }` and `onOpen({ x: r.left + 40, y: mid })`.
fn address_pill(p: &Probe) {
    let position = LogicalPosition::new(140.0, 68.0);
    at(p, WindowEvent::PointerMoved { position });
    at(
        p,
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
    );
    run_out();
    assert!(
        (p.get_pill_press() - 0.985).abs() < 1e-6,
        "{}",
        p.get_pill_press()
    );
    at(
        p,
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    );
    run_out();
    assert_eq!(p.get_pill_press(), 1.0);
    assert_eq!((p.get_origin_x(), p.get_origin_y()), (60.0, 68.0));
}

#[test]
fn chrome_kit_parts_follow_the_design() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    p.show().unwrap();
    run_out();
    address_pill(&p);
}
