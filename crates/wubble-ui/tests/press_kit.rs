//! The press lane's kit controls on the testing backend: their boxes are the web design's
//! (`tools/fidelity/fixtures/controls/geometry.json`, measured from the design's own pages by
//! `node tools/fidelity/controls-capture.mjs --geometry`), the specimen lays every one of them out,
//! and they speak to assistive technology and the keyboard as the design's elements do: roles,
//! checked states and labels; Tab reaches them, Space and Enter work them; a disabled one does
//! nothing. An inline `slint!` component has no debug info, so the testing backend's element
//! queries do not reach into it: the probe reads each control's size and accessible properties
//! as Slint expressions, and the test drives it with window events. Mock time only; one test
//! (one platform per process).

use std::fs;
use std::path::Path;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

slint::slint! {
    #[edition = "1.19"]
    // the shell's fonts are imported by app.slint; the probe imports the text face as
    // crisp_text.rs does, so labels measure in the design's Geist
    import "ui/fonts/Geist-Variable.ttf";
    import { PressSpecimen } from "ui/kit/specimen/press.slint";
    import { CheckBox } from "ui/kit/controls/check-box.slint";
    import { Toggle, ToggleSize } from "ui/kit/controls/toggle.slint";
    import { Button, ButtonKind } from "ui/kit/controls/button.slint";
    import { IconButton } from "ui/kit/controls/icon-button.slint";
    import { HoldButton } from "ui/kit/controls/hold-button.slint";
    import { ConfirmButton } from "ui/kit/controls/confirm-button.slint";
    import { KbdCap } from "ui/kit/controls/kbd.slint";
    import { Glyph } from "ui/kit/controls/icon.slint";
    import { Spin } from "ui/kit/motion/tween.slint";
    import { Calm } from "ui/kit/motion/calm.slint";
    import { Controls } from "ui/components/controls/registry.slint";
    export { Calm, Controls }
    export component Probe inherits Window {
        width: 1360px;
        height: 1600px;
        in-out property <int> clicks;
        in-out property <bool> on;
        // the controls on their own, in Tab order; outside a layout an element takes its parent's
        // size, so the content-sized ones take what a layout would give them (preferred, at least
        // the minimum)
        ck := CheckBox {
            x: 0px;
            y: 0px;
            width: 256px;
            height: max(self.min-height, self.preferred-height);
            text: "Open links in a peek";
        }
        tg := Toggle {
            x: 0px;
            y: 60px;
            size: ToggleSize.sm;
            on: root.on;
            label: "Block trackers";
            toggled => {
                root.on = !root.on;
            }
        }
        lg := Toggle {
            x: 100px;
            y: 60px;
            size: ToggleSize.lg;
            label: "Block trackers";
        }
        pri := Button {
            width: max(self.min-width, self.preferred-width);
            x: 0px;
            y: 120px;
            kind: ButtonKind.pri;
            text: "Open in Space";
            clicked => {
                root.clicks += 1;
            }
        }
        off := Button {
            width: max(self.min-width, self.preferred-width);
            x: 0px;
            y: 180px;
            kind: ButtonKind.ghost;
            text: "Cancel";
            enabled: false;
            clicked => {
                root.clicks += 100;
            }
        }
        glass := Button {
            width: max(self.min-width, self.preferred-width);
            x: 200px;
            y: 180px;
            text: "Split view";
        }
        ib := IconButton {
            x: 0px;
            y: 240px;
            icon-path: Glyph.split;
            label: "Split view";
            on: true;
        }
        hold := HoldButton {
            width: max(self.min-width, self.preferred-width);
            x: 0px;
            y: 300px;
            text: "Hold to clear";
            done-text: "Cleared";
            enabled: false;
        }
        confirm := ConfirmButton {
            width: max(self.min-width, self.preferred-width);
            x: 0px;
            y: 360px;
            idle-text: "Check for updates";
            busy-text: "Checking";
            done-text: "Up to date";
            busy-ms: 1400;
        }
        cap := KbdCap {
            x: 0px;
            y: 420px;
            text: "Ctrl";
        }
        // the specimen, laid out below them (last: Tab reaches the controls above first; below:
        // the pointer reaches them)
        spec := PressSpecimen {
            y: 500px;
            width: parent.width;
            height: parent.height - self.y;
        }
        // [width, height] per control, by the geometry fixture's names
        out property <[{name: string, w: float, h: float}]> boxes: [
            { name: "button-pri", w: pri.width / 1px, h: pri.height / 1px },
            { name: "button-glass", w: glass.width / 1px, h: glass.height / 1px },
            { name: "button-ghost", w: off.width / 1px, h: off.height / 1px },
            { name: "toggle-lg", w: lg.width / 1px, h: lg.height / 1px },
            { name: "toggle-sm", w: tg.width / 1px, h: tg.height / 1px },
            { name: "icon-button", w: ib.width / 1px, h: ib.height / 1px },
            { name: "hold", w: hold.width / 1px, h: hold.height / 1px },
            { name: "confirm", w: confirm.width / 1px, h: confirm.height / 1px },
            { name: "check-row", w: ck.width / 1px, h: ck.height / 1px },
            { name: "kbd", w: cap.width / 1px, h: cap.height / 1px },
        ];
        out property <bool> roles: ck.accessible-role == AccessibleRole.checkbox
            && tg.accessible-role == AccessibleRole.switch
            && pri.accessible-role == AccessibleRole.button
            && ib.accessible-role == AccessibleRole.button
            && hold.accessible-role == AccessibleRole.button
            && confirm.accessible-role == AccessibleRole.button
            && cap.accessible-role == AccessibleRole.text;
        out property <[string]> labels: [ck.accessible-label, tg.accessible-label, pri.accessible-label, ib.accessible-label, hold.accessible-label, confirm.accessible-label, cap.accessible-label];
        out property <bool> checked: ck.accessible-checked;
        out property <bool> switch-checked: tg.accessible-checked;
        out property <bool> disabled-off: !off.accessible-enabled && !hold.accessible-enabled;
        out property <bool> live-polite: confirm.accessible-live-region == AccessibleLiveness.polite;
        out property <bool> focused-button: pri.keyboard-focused;
        // an enabled hold, for the pointer chord
        HoldButton {
            x: 0px;
            y: 1500px;
            width: 200px;
            height: 40px;
            text: "Hold";
            done-text: "Done";
        }
        // the confirm button's busy spinner
        spin := Spin {
            running: true;
            ms: 800ms;
        }
        out property <float> spin-turn: spin.turn;
        // the specimen took its size (every stage laid out)
        out property <float> specimen-height: spec.preferred-height / 1px;
    }
}

fn web_boxes() -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fidelity/fixtures/controls/geometry.json");
    serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).expect("geometry.json"))
        .expect("json")["boxes"]
        .clone()
}

fn key(p: &Probe, text: impl Into<slint::SharedString> + Clone) {
    let w = p.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: text.clone().into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    mock_elapsed_time(Duration::from_millis(16));
}

#[test]
fn kit_controls_geometry_and_keyboard() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    p.show().unwrap();
    mock_elapsed_time(Duration::from_millis(500));

    // ── boxes against the web's ──
    let web = web_boxes();
    let mut report = Vec::new();
    let mut bad = Vec::new();
    // an anonymous Slint struct comes as a tuple of its fields by name: (h, name, w)
    for (h, name, w) in p.get_boxes().iter() {
        let name = name.as_str();
        let want = &web[name];
        let (ww, wh) = (
            want[0].as_f64().expect("w") as f32,
            want[1].as_f64().expect("h") as f32,
        );
        report.push(format!("{name}: {w}x{h} (web {ww}x{wh})"));
        // widths that hold text are measured by two shapers (Chromium's, Slint's): 3 px; the
        // check row stretches across its card in both (only its height is the design's)
        let tol = match name {
            "check-row" => f32::MAX,
            "button-pri" | "button-glass" | "button-ghost" | "kbd" | "hold" | "confirm" => 3.0,
            _ => 0.01,
        };
        if (w - ww).abs() > tol || (h - wh).abs() > 0.01 {
            bad.push(format!("{name}: {w}x{h}, web {ww}x{wh}"));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(bad.is_empty(), "{bad:?}");
    assert!(p.get_specimen_height() > 0.0);

    // ── roles, states, labels ──
    assert!(p.get_roles(), "check box, switch, buttons and a text cap");
    let labels: Vec<String> = p.get_labels().iter().map(|s| s.to_string()).collect();
    assert_eq!(
        labels,
        [
            "Open links in a peek",
            "Block trackers",
            "Open in Space",
            "Split view",
            "Hold to clear",
            "Check for updates",
            "Ctrl"
        ]
    );
    assert!(!p.get_checked() && !p.get_switch_checked());
    assert!(p.get_disabled_off(), "the disabled button and hold say so");
    assert!(
        p.get_live_polite(),
        "the confirm button is aria-live=polite"
    );

    // ── the keyboard: Tab, then Space or Enter ──
    key(&p, Key::Tab);
    key(&p, " ");
    assert!(p.get_checked(), "Space ticks the focused check box");
    key(&p, Key::Tab);
    key(&p, Key::Return);
    assert!(
        p.get_on() && p.get_switch_checked(),
        "Enter flips the focused toggle"
    );
    // held keys: a `<button>`'s keyboard click comes once per press (Space on its release),
    // never on the repeats
    let w = p.window();
    w.dispatch_event(WindowEvent::KeyPressed { text: " ".into() });
    for _ in 0..3 {
        w.dispatch_event(WindowEvent::KeyPressRepeated { text: " ".into() });
    }
    assert!(p.get_on(), "Space flips on its release, not its press");
    w.dispatch_event(WindowEvent::KeyReleased { text: " ".into() });
    assert!(!p.get_on(), "held Space flips once");
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    for _ in 0..3 {
        w.dispatch_event(WindowEvent::KeyPressRepeated {
            text: Key::Return.into(),
        });
    }
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Return.into(),
    });
    assert!(p.get_on(), "held Enter flips once");
    key(&p, Key::Tab); // the lg toggle
    key(&p, Key::Tab);
    assert!(p.get_focused_button());
    key(&p, " ");
    assert_eq!(p.get_clicks(), 1, "Space clicks the focused button");
    // the disabled button is skipped by Tab; its click does nothing
    key(&p, Key::Tab);
    key(&p, Key::Return);
    assert_eq!(p.get_clicks(), 1);
    let click = |x: f32, y: f32| {
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        p.window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        p.window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        mock_elapsed_time(Duration::from_millis(16));
    };
    click(30.0, 200.0);
    assert_eq!(p.get_clicks(), 1, "a disabled button ignores a click");
    click(30.0, 140.0);
    assert_eq!(
        p.get_clicks(),
        2,
        "the enabled one takes it (the pointer reaches the probe)"
    );

    // ── a chord: only the left button's up ends a press (the web's `pointerup` comes when the
    // last button goes up) ──
    let releases = std::rc::Rc::new(std::cell::Cell::new(0));
    let r = releases.clone();
    p.global::<Controls>().on_release(move |_| {
        r.set(r.get() + 1);
        -1.0
    });
    for (x, y) in [(10.0, 1520.0), (10.0, 70.0)] {
        let position = LogicalPosition::new(x, y);
        let w = p.window();
        w.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        w.dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Right,
        });
        w.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Right,
        });
        assert_eq!(
            releases.get(),
            0,
            "the right button's up at {y} ends nothing"
        );
        w.dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        assert_eq!(
            releases.get(),
            1,
            "the left button's up at {y} ends the press"
        );
        releases.set(0);
        mock_elapsed_time(Duration::from_millis(16));
    }

    // ── the busy spinner: turns, but rests under reduced motion (the web's one 1 ms run) ──
    mock_elapsed_time(Duration::from_millis(200));
    assert!(p.get_spin_turn() > 0.0, "it turns");
    p.global::<Calm>().set_reduced(true);
    for _ in 0..5 {
        mock_elapsed_time(Duration::from_micros(8_333));
        assert_eq!(p.get_spin_turn(), 0.0, "reduced: the ring stays still");
    }
}
