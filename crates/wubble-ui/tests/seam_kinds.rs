//! The two seams of the web design on the testing backend: the sidebar's string is always drawn
//! (`.wb-seam path`), a split seam draws nothing at rest and only its held line
//! (`.wb-split-line`), which fades in while held and out on release (browser.css:114-115).
//! Mock time; the tweens are let run out, nothing is timed against the wall clock.

use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::ComponentHandle;

slint::slint! {
    #[edition = "1.19"]
    import { SeamStrip, SeamKind } from "ui/kit/controls/seam.slint";
    export component Probe inherits Window {
        width: 400px;
        height: 300px;
        in property <bool> held;
        side := SeamStrip { kind: SeamKind.sidebar; seam-x: 100px; held: root.held; }
        split := SeamStrip { kind: SeamKind.split; seam-x: 200px; held: root.held; }
        out property <bool> side-rest: side.rest-string;
        out property <float> side-line: side.line-opacity;
        out property <color> side-colour: side.string-colour;
        out property <bool> split-rest: split.rest-string;
        out property <float> split-line: split.line-opacity;
    }
}

/// Past the longest tween (.2 s), so every value is at its target.
fn run_out() {
    mock_elapsed_time(Duration::from_millis(400));
}

#[test]
fn each_seam_does_what_the_web_does_for_it() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    p.show().unwrap();
    run_out();

    // at rest: the sidebar's string white .1, no line; the split seam nothing at all
    assert!(p.get_side_rest());
    assert_eq!(p.get_side_line(), 0.0);
    assert_eq!(
        p.get_side_colour(),
        slint::Color::from_argb_f32(0.1, 1.0, 1.0, 1.0)
    );
    assert!(!p.get_split_rest());
    assert_eq!(p.get_split_line(), 0.0);

    // held: the split seam's line comes in; the sidebar keeps its string (sky now) and no line
    p.set_held(true);
    run_out();
    assert_eq!(p.get_split_line(), 1.0);
    assert!(!p.get_split_rest());
    assert!(p.get_side_rest());
    assert_eq!(p.get_side_line(), 0.0);
    assert_eq!(
        p.get_side_colour(),
        slint::Color::from_rgb_u8(0x7C, 0xC8, 0xFF)
    );

    // released: the line goes again
    p.set_held(false);
    run_out();
    assert_eq!(p.get_split_line(), 0.0);
}
