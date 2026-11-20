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
    import { SpaceTint, SpaceCurrent } from "ui/kit/chrome/space-switcher.slint";
    import { CommandBar } from "ui/kit/chrome/command-bar.slint";
    import { TabRow } from "ui/kit/chrome/tab-row.slint";
    import { SpacesButton } from "ui/kit/chrome/spaces-button.slint";
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
        // the current Space's tile: a click asks for the popup
        in-out property <int> current-clicks;
        current := SpaceCurrent {
            x: 20px; y: 200px; known: true;
            space: { id: 2, name: "b", icon: "work" };
            clicked => { root.current-clicks += 1; }
        }
        out property <length> current-w: current.width;
        out property <length> current-h: current.height;
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
        // a tab on the rail: its close badge on the icon's top-left corner
        in-out property <int> rail-closes;
        in-out property <int> rail-presses;
        rail := TabRow {
            x: 320px; y: 20px; width: 48px; compact: true;
            tab: { id: 9, title: "r", url: "https://r.test" };
            close => { root.rail-closes += 1; }
            press(y) => { root.rail-presses += 1; }
        }
        out property <bool> rail-close-shown: rail.rail-close-shown;
        out property <length> rail-h: rail.height;
        // an agent's tab on the rail: the agent badge on the icon's bottom-right corner
        in-out property <int> marked-stops;
        in-out property <int> marked-presses;
        marked := TabRow {
            x: 250px; y: 20px; width: 48px; compact: true;
            tab: { id: 10, title: "m", url: "https://m.test", agent: true };
            agent-stop => { root.marked-stops += 1; }
            press(y) => { root.marked-presses += 1; }
        }
        // the switcher's "^"
        in-out property <int> mores;
        in property <bool> more-open;
        more := SpacesButton { x: 320px; y: 100px; open: root.more-open; clicked => { root.mores += 1; } }
        out property <bool> more-hovered: more.hovered;
        out property <length> more-w: more.width;
        out property <float> more-turn: more.chevron-turn;
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
/// inherit }` outranks `.wb-space`'s haze); the current Space's tile is one 36 px `.wb-space`
/// whose click asks for the Spaces popup.
fn spaces(p: &Probe) {
    use slint::Model;
    let tints: Vec<slint::Color> = p.get_space_tints().iter().collect();
    let (haze, ink) = (
        slint::Color::from_rgb_u8(0x7E, 0x84, 0x96),
        slint::Color::from_rgb_u8(0xF2, 0xF4, 0xFA),
    );
    assert_eq!(tints, [ink, ink, ink]);
    assert_ne!(tints[0], haze, "the web renders no haze glyph");
    assert_eq!((p.get_current_w(), p.get_current_h()), (36.0, 36.0));
    let position = LogicalPosition::new(38.0, 218.0);
    let button = PointerEventButton::Left;
    at(p, WindowEvent::PointerPressed { position, button });
    at(p, WindowEvent::PointerReleased { position, button });
    assert_eq!(p.get_current_clicks(), 1);
    at(p, WindowEvent::PointerExited);
    run_out();
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

/// The rail's close badge (derived: the close light on the icon's top-left corner): it shows
/// while the row is hovered, a click on it closes the tab, a click on the rest of the icon picks
/// it.
fn rail_close(p: &Probe) {
    let click = |x: f32, y: f32| {
        let position = LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        at(p, WindowEvent::PointerMoved { position });
        at(p, WindowEvent::PointerPressed { position, button });
        at(p, WindowEvent::PointerReleased { position, button });
    };
    let h = p.get_rail_h();
    // the 18 px icon box, centred in the 48 wide row
    let (icon_x, icon_y) = (320.0 + (48.0 - 18.0) / 2.0, 20.0 + (h - 18.0) / 2.0);
    run_out();
    assert!(!p.get_rail_close_shown(), "hidden at rest");
    at(
        p,
        WindowEvent::PointerMoved {
            position: LogicalPosition::new(icon_x + 9.0, icon_y + 9.0),
        },
    );
    run_out();
    assert!(p.get_rail_close_shown(), "the hovered row shows it");
    // the badge, on the icon's top-left corner
    click(icon_x, icon_y);
    run_out();
    assert_eq!((p.get_rail_closes(), p.get_rail_presses()), (1, 0));
    assert!(p.get_rail_close_shown(), "still shown under the pointer");
    // the rest of the icon picks the tab
    click(icon_x + 12.0, icon_y + 12.0);
    assert_eq!((p.get_rail_closes(), p.get_rail_presses()), (1, 1));
    at(p, WindowEvent::PointerExited);
    run_out();
    assert!(!p.get_rail_close_shown());
}

/// The rail's agent badge (derived: the close badge's circle on the icon's bottom-right corner):
/// drawn at rest, a press on it stops the agents, the rest of the icon still picks the tab.
fn rail_agent_badge(p: &Probe) {
    let click = |x: f32, y: f32| {
        let position = LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        at(p, WindowEvent::PointerMoved { position });
        at(p, WindowEvent::PointerPressed { position, button });
        at(p, WindowEvent::PointerReleased { position, button });
    };
    let row_h = p.get_rail_h();
    let (icon_x, icon_y) = (250.0 + (48.0 - 18.0) / 2.0, 20.0 + (row_h - 18.0) / 2.0);
    run_out();
    // the pointer is out: the badge is there with no hover on the row
    click(icon_x + 18.0, icon_y + 18.0);
    run_out();
    assert_eq!(
        (p.get_marked_stops(), p.get_marked_presses()),
        (1, 0),
        "the bottom-right badge stops the agents"
    );
    click(icon_x + 9.0, icon_y + 9.0);
    assert_eq!(
        (p.get_marked_stops(), p.get_marked_presses()),
        (1, 1),
        "the icon's middle picks the tab"
    );
    at(p, WindowEvent::PointerExited);
    run_out();
}

/// The switcher's "^": a `.wb-space` tile (36 square) that reports a click (the popup) and its
/// hover; its chevron points up at rest and turns over (`.tp-chev.up`) while the popup is open.
fn spaces_button(p: &Probe) {
    assert_eq!(p.get_more_w(), 36.0);
    assert_eq!(p.get_more_turn(), 0.0);
    let position = LogicalPosition::new(338.0, 118.0);
    let button = PointerEventButton::Left;
    at(p, WindowEvent::PointerMoved { position });
    assert!(p.get_more_hovered());
    at(p, WindowEvent::PointerPressed { position, button });
    at(p, WindowEvent::PointerReleased { position, button });
    assert_eq!(p.get_mores(), 1);
    p.set_more_open(true);
    run_out();
    assert_eq!(p.get_more_turn(), 1.0);
    p.set_more_open(false);
    at(p, WindowEvent::PointerExited);
    run_out();
    assert_eq!(p.get_more_turn(), 0.0);
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
    rail_close(&p);
    rail_agent_badge(&p);
    spaces_button(&p);
}
