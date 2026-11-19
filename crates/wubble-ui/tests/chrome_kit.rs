//! The chrome's kit parts (`ui/kit/chrome/`) on the testing backend, against the web design's
//! rules: the address pill's press and its open origin. Mock time; tweens are let run out,
//! nothing is timed against the wall clock. One test (one platform per process).

use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{PointerEventButton, WindowEvent},
};

slint::slint! {
    #[edition = "1.19"]
    import { AddressPill } from "ui/kit/chrome/address-pill.slint";
    import { Favs } from "ui/kit/chrome/favs.slint";
    import { SpaceTint, SpaceSwitcher } from "ui/kit/chrome/space-switcher.slint";
    import { CommandBar } from "ui/kit/chrome/command-bar.slint";
    import { CommandBarState, CommandMode, CommandResult, TabItem } from "ui/components/model.slint";
    export { CommandBarState, CommandMode, CommandResult, TabItem }
    export component Probe inherits Window {
        width: 400px;
        height: 300px;
        in property <bool> compact;
        pill := AddressPill { x: 20px; y: 50px; width: 240px; compact: root.compact; host: "example.com"; path: "/a"; }
        out property <float> pill-press: pill.press-scale;
        in property <[TabItem]> fav-tabs: [{ id: 7, title: "a", url: "https://a.test" }, { id: 8, title: "b", url: "https://b.test" }];
        favs := Favs {
            x: 20px; y: 120px; width: 240px; compact: root.compact;
            tabs: root.fav-tabs;
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
        in property <bool> bar-on;
        in property <CommandMode> bar-mode;
        in property <[CommandResult]> bar-results;
        bar := CommandBar {
            visible: root.bar-on;
            state: root.bar-on ? CommandBarState.open : CommandBarState.closed;
            mode: root.bar-mode;
            here-host: "example.com";
            results: root.bar-results;
        }
        out property <int> bar-selected: bar.selected-row;
        out property <bool> bar-empty: bar.empty-shown;
        out property <string> bar-placeholder: bar.placeholder;
        public function bar-focus() { bar.focus-input(); }
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
    // the gap between the tiles is no tile
    at(
        p,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(78.0, 140.0),
        },
    );
    assert_eq!(p.get_fav_hover(), -1);
    at(
        p,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(100.0, 140.0),
        },
    );
    // The hovered tile goes (its tab closed) with the pointer still on it: no stale id, the
    // press would wave nothing.
    let tabs = p.get_fav_tabs();
    let first: Vec<TabItem> = tabs.iter().take(1).collect();
    p.set_fav_tabs(slint::ModelRc::from(first.as_slice()));
    assert_eq!(p.get_fav_hover(), -1);
    p.set_fav_tabs(tabs);
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

fn key(p: &Probe, k: slint::platform::Key) {
    let text: slint::SharedString = k.into();
    p.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    p.window().dispatch_event(WindowEvent::KeyReleased { text });
    mock_elapsed_time(Duration::from_millis(1));
}

/// command.tsx: the pointer selects a row, Up / Down stop at both ends, no rows show the empty
/// line, the placeholder follows the mode.
fn command_bar(p: &Probe) {
    use slint::platform::Key;
    let row = |t: &str| CommandResult {
        title: t.into(),
        ..Default::default()
    };
    p.set_bar_results(slint::ModelRc::from(
        [row("a"), row("b"), row("c")].as_slice(),
    ));
    p.set_bar_on(true);
    run_out();
    p.invoke_bar_focus();
    assert_eq!(p.get_bar_selected(), 0);
    assert!(!p.get_bar_empty());
    key(p, Key::UpArrow);
    assert_eq!(p.get_bar_selected(), 0, "clamped at the top");
    for _ in 0..4 {
        key(p, Key::DownArrow);
    }
    assert_eq!(p.get_bar_selected(), 2, "clamped at the bottom");
    // the panel: 72 from the top, padding 6, the head 52, the list's 4; rows 44 high
    let row_y = |i: f32| 72.0 + 6.0 + 52.0 + 4.0 + 22.0 + 44.0 * i;
    at(
        p,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(200.0, row_y(1.0)),
        },
    );
    assert_eq!(p.get_bar_selected(), 1, "hover selects");
    at(p, WindowEvent::PointerExited);
    p.set_bar_results(slint::ModelRc::default());
    mock_elapsed_time(Duration::from_millis(1));
    assert!(p.get_bar_empty());
    for (mode, want) in [
        (CommandMode::Here, "example.com"),
        (CommandMode::New, "Search or type an address"),
        (CommandMode::Split, "Open beside this page"),
    ] {
        p.set_bar_mode(mode);
        assert_eq!(p.get_bar_placeholder(), want);
    }
    p.set_bar_on(false);
    run_out();
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
    command_bar(&p);
}
