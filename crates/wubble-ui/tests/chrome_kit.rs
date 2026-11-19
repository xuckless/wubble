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
    import { Favs } from "ui/kit/chrome/favs.slint";
    import { SpaceTint, SpaceSwitcher } from "ui/kit/chrome/space-switcher.slint";
    export component Probe inherits Window {
        width: 400px;
        height: 300px;
        in property <bool> compact;
        pill := AddressPill { x: 20px; y: 50px; width: 240px; compact: root.compact; host: "example.com"; path: "/a"; }
        out property <float> pill-press: pill.press-scale;
        favs := Favs {
            x: 20px; y: 120px; width: 240px; compact: root.compact;
            tabs: [{ id: 7, title: "a", url: "https://a.test" }, { id: 8, title: "b", url: "https://b.test" }];
        }
        out property <int> fav-hover: favs.hovered-id;
        out property <length> favs-height: favs.height;
        // the Space glyph's colour: rest, hovered, active
        out property <[color]> space-tints: [SpaceTint.of(false, false), SpaceTint.of(true, false), SpaceTint.of(false, true)];
        spaces := SpaceSwitcher {
            x: 20px; y: 200px; vertical: root.compact;
            spaces: [{ id: 1, name: "a" }, { id: 2, name: "b" }, { id: 3, name: "c" }];
        }
        out property <length> spaces-w: spaces.width;
        out property <length> spaces-h: spaces.height;
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

/// `.wb-favs`: four columns 8 apart, 44 high; the rail: one 40 px column 6 apart. The tile
/// under the pointer is the one a press waves.
fn favourites(p: &Probe) {
    assert_eq!(p.get_favs_height(), 44.0);
    // the second tile: x 20 + (240 - 24) / 4 + 8 = 82 .. 136
    at(
        p,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(100.0, 140.0),
        },
    );
    assert_eq!(p.get_fav_hover(), 8);
    at(p, WindowEvent::PointerExited);
    assert_eq!(p.get_fav_hover(), -1);
    p.set_compact(true);
    mock_elapsed_time(Duration::from_millis(1));
    assert_eq!(p.get_favs_height(), 40.0 + 6.0 + 40.0);
    p.set_compact(false);
}

/// The Space glyph's computed colour: ink at rest, hovered and active (`.wb button { color:
/// inherit }` outranks `.wb-space`'s haze); `.wb-spaces` 36 tiles 2 apart, a column on the rail.
fn spaces(p: &Probe) {
    use slint::Model;
    let tints: Vec<slint::Color> = p.get_space_tints().iter().collect();
    let (haze, ink) = (
        slint::Color::from_rgb_u8(0x7E, 0x84, 0x96),
        slint::Color::from_rgb_u8(0xF2, 0xF4, 0xFA),
    );
    assert_eq!(tints, [ink, ink, ink]);
    assert_ne!(tints[0], haze, "the web renders no haze glyph");
    assert_eq!(
        (p.get_spaces_w(), p.get_spaces_h()),
        (3.0 * 38.0 - 2.0, 36.0)
    );
    p.set_compact(true);
    mock_elapsed_time(Duration::from_millis(1));
    assert_eq!(
        (p.get_spaces_w(), p.get_spaces_h()),
        (36.0, 3.0 * 38.0 - 2.0)
    );
    p.set_compact(false);
}

#[test]
fn chrome_kit_parts_follow_the_design() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    p.show().unwrap();
    run_out();
    address_pill(&p);
    favourites(&p);
    spaces(&p);
}
