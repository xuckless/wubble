//! The press lane's controls against the web design's, frame by frame on the fixed 1/120 s
//! clock: `tools/fidelity/fixtures/controls/<control>/*.json` are the design's own controls
//! driven in headless Chromium on the harness's clock (`node tools/fidelity/controls-capture.mjs`).
//! The CSS-driven ones (check box, button press and squash, tooltip) run here as kit components
//! on the testing backend's mock time; the physics ones (toggle, hold, confirm) are wubble-core's
//! machines stepped by the same `dt`. Each property's curve must agree with the web's to 0.90 or
//! better (1 - RMS difference / the web curve's range, as `motion_curves.rs`). No wall clock.

use std::fs;
use std::path::Path;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};
use wubble_core::controls::{confirm, hold, toggle};

slint::slint! {
    #[edition = "1.19"]
    import { CheckBox } from "ui/kit/controls/check-box.slint";
    import { Button, ButtonKind } from "ui/kit/controls/button.slint";
    import { Tooltip } from "ui/kit/controls/tooltip.slint";
    export component Probe inherits Window {
        width: 400px;
        height: 300px;
        in property <bool> tip-on: false;
        ck := CheckBox {
            x: 0px;
            y: 0px;
            width: 200px;
            text: "Open links in a peek";
        }
        btn := Button {
            x: 0px;
            y: 100px;
            kind: ButtonKind.glass;
            text: "Split view";
        }
        tip := Tooltip {
            shown: root.tip-on;
            jump: true;
            text: "Back";
            centered: true;
            anchor-x: 300px;
            anchor-y: 200px;
            anchor-top: 160px;
            bounds-width: root.width;
            bounds-height: root.height;
        }
        out property <[float]> now: [
            ck.fill, ck.tick, ck.box-scale-x, ck.box-scale-y,
            btn.transform-scale-x, btn.transform-scale-y,
            tip.shown-opacity, tip.rise-y / 1px, tip.rise-scale,
        ];
    }
}

/// The frame step: the web sampler's 1/120 s.
const DT_US: u64 = 8_333;
const DT: f64 = 1.0 / 120.0;

fn fixture(control: &str, name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fidelity/fixtures/controls")
        .join(control)
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(&path).expect("read fixture")).expect("fixture json")
}

fn curve(f: &serde_json::Value, prop: &str) -> Vec<f64> {
    f["frames"][prop]
        .as_array()
        .unwrap_or_else(|| panic!("no {prop}"))
        .iter()
        .map(|v| v.as_f64().expect("number"))
        .collect()
}

fn frames(f: &serde_json::Value) -> usize {
    f["frames"]
        .as_object()
        .and_then(|o| o.values().next())
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len)
}

/// 1 - RMS(a - b) / range(b); 1 for two equal flat curves.
fn similarity(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let rms = (a
        .iter()
        .zip(b)
        .take(n)
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        / n as f64)
        .sqrt();
    let (lo, hi) = b
        .iter()
        .take(n)
        .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    if hi - lo < 1e-9 {
        if rms < 1e-3 { 1.0 } else { 0.0 }
    } else {
        1.0 - rms / (hi - lo)
    }
}

fn check(label: &str, ours: &[f64], web: &[f64], report: &mut Vec<String>) {
    let s = similarity(ours, web);
    report.push(format!("{label}: {s:.3}"));
    assert!(s >= 0.90, "{label}: {s:.3}\nkit {ours:?}\nweb {web:?}");
}

// ── Slint side: the CSS motions ─────────────────────────────────────────────────────────────

fn now(p: &Probe) -> Vec<f64> {
    p.get_now().iter().map(f64::from).collect()
}

/// `n` samples of `Probe.now`, the first at once, then one per step.
fn sample(p: &Probe, n: usize) -> Vec<Vec<f64>> {
    slint::platform::update_timers_and_animations();
    let mut out = vec![now(p)];
    for _ in 1..n {
        mock_elapsed_time(Duration::from_micros(DT_US));
        out.push(now(p));
    }
    out
}

fn pointer(p: &Probe, down: bool, at: (f32, f32)) {
    let position = LogicalPosition::new(at.0, at.1);
    let button = PointerEventButton::Left;
    p.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}

fn compare(
    control: &str,
    name: &str,
    pairs: &[(&str, usize)],
    kit: &[Vec<f64>],
    report: &mut Vec<String>,
) {
    let f = fixture(control, name);
    for (prop, i) in pairs {
        let ours: Vec<f64> = kit.iter().map(|v| v[*i]).collect();
        check(
            &format!("{control}/{name}.{prop}"),
            &ours,
            &curve(&f, prop),
            report,
        );
    }
}

#[test]
fn css_controls_follow_the_web_curves() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    p.show().unwrap();
    // read once at rest, so every animation starts from the state (Slint animates a binding
    // only from a value it has evaluated)
    now(&p);
    mock_elapsed_time(Duration::from_secs(1));
    now(&p);
    let mut report = Vec::new();
    let ck = (40.0, 18.0);
    let click = |at| {
        pointer(&p, true, at);
        pointer(&p, false, at);
    };

    // the check box, as the capture drives the design's second row: on, off, held, let go (on)
    click(ck);
    let on = sample(&p, frames(&fixture("check-box", "on")));
    compare(
        "check-box",
        "on",
        &[("fill", 0), ("tick", 1), ("sx", 2), ("sy", 3)],
        &on,
        &mut report,
    );
    click(ck);
    let off = sample(&p, frames(&fixture("check-box", "off")));
    compare(
        "check-box",
        "off",
        &[("fill", 0), ("tick", 1)],
        &off,
        &mut report,
    );
    pointer(&p, true, ck);
    let press = sample(&p, frames(&fixture("check-box", "press")));
    compare(
        "check-box",
        "press",
        &[("px", 2), ("py", 3)],
        &press,
        &mut report,
    );
    pointer(&p, false, ck);
    let release = sample(&p, frames(&fixture("check-box", "release")));
    // the box's scale is the press times the squash (the web's `scale` and `transform`)
    let f = fixture("check-box", "release");
    let web: Vec<f64> = curve(&f, "px")
        .iter()
        .zip(curve(&f, "sx"))
        .map(|(a, b)| a * b)
        .collect();
    let ours: Vec<f64> = release.iter().map(|v| v[2]).collect();
    check("check-box/release.px*sx", &ours, &web, &mut report);

    // the button: pressed, then let go (the body's release squash times the press coming back)
    let b = (60.0, 122.0);
    pointer(&p, true, b);
    let press = sample(&p, frames(&fixture("button", "press")));
    compare("button", "press", &[("press", 4)], &press, &mut report);
    pointer(&p, false, b);
    let release = sample(&p, frames(&fixture("button", "release")));
    let f = fixture("button", "release");
    for (prop, i) in [("sx", 4), ("sy", 5)] {
        let web: Vec<f64> = curve(&f, "press")
            .iter()
            .zip(curve(&f, prop))
            .map(|(a, b)| a * b)
            .collect();
        let ours: Vec<f64> = release.iter().map(|v| v[i]).collect();
        check(
            &format!("button/release.press*{prop}"),
            &ours,
            &web,
            &mut report,
        );
    }

    // the tooltip shows, then hides
    p.set_tip_on(true);
    let show = sample(&p, frames(&fixture("tooltip", "show")));
    compare(
        "tooltip",
        "show",
        &[("opacity", 6), ("ty", 7), ("scale", 8)],
        &show,
        &mut report,
    );
    p.set_tip_on(false);
    let hide = sample(&p, frames(&fixture("tooltip", "hide")));
    compare(
        "tooltip",
        "hide",
        &[("opacity", 6), ("ty", 7), ("scale", 8)],
        &hide,
        &mut report,
    );
    eprintln!("{}", report.join("\n"));
}

// ── Rust side: the machines, stepped by dt ──────────────────────────────────────────────────

/// The toggle view's values by the fixtures' names.
fn toggle_now(s: &toggle::State) -> [(&'static str, f64); 8] {
    let v = s.view();
    [
        ("x", v.x),
        ("k", v.scale_x),
        ("ky", v.scale_y),
        ("o1", v.origin),
        ("qx", v.squash_x),
        ("qy", v.squash_y),
        ("o2", v.squash_origin),
        ("fill", v.fill),
    ]
}

fn toggle_check(name: &str, kit: &[[(&str, f64); 8]], report: &mut Vec<String>) {
    let f = fixture("toggle", name);
    for i in 0..8 {
        let prop = kit[0][i].0;
        let ours: Vec<f64> = kit.iter().map(|v| v[i].1).collect();
        check(
            &format!("toggle/{name}.{prop}"),
            &ours,
            &curve(&f, prop),
            report,
        );
    }
}

fn apply(s: toggle::State, ev: toggle::Event) -> toggle::State {
    toggle::transition(s, ev).0
}

/// `n` frames: the first as the page shows it when the events arrive (the last frame painted:
/// the TSX paints only in its loop), then the events, then one tick a frame.
fn toggle_run(
    s: &mut toggle::State,
    events: &[toggle::Event],
    n: usize,
) -> Vec<[(&'static str, f64); 8]> {
    let mut out = vec![toggle_now(s)];
    for e in events {
        *s = apply(s.clone(), *e);
    }
    for _ in 1..n {
        *s = apply(s.clone(), toggle::Event::Tick(DT));
        out.push(toggle_now(s));
    }
    out
}

const TAP: fn(f64) -> [toggle::Event; 2] = |px| {
    [
        toggle::Event::Press { px, t_ms: 0.0 },
        toggle::Event::Release,
    ]
};

#[test]
fn the_toggle_follows_the_web() {
    let mut report = Vec::new();
    let ms = |frame: usize| frame as f64 * DT * 1000.0;
    // lg, as the capture drives the specimen (it starts on): tap off, tap on
    let mut s = toggle::State::new(true, toggle::Size::Lg);
    for name in ["lg-tap-off", "lg-tap-on"] {
        let kit = toggle_run(&mut s, &TAP(46.0), frames(&fixture("toggle", name)));
        toggle_check(name, &kit, &mut report);
    }
    // a tap off and 120 frames of rest; then held 24 frames, dragged 30 px over 12, released
    toggle_run(&mut s, &TAP(46.0), 121);
    let t0 = 10_000.0;
    let press = toggle::Event::Press { px: 46.0, t_ms: t0 };
    let held = toggle_run(&mut s, &[press], frames(&fixture("toggle", "lg-hold")));
    toggle_check("lg-hold", &held, &mut report);
    let mut drag = Vec::new();
    for i in 1..=12 {
        s = apply(
            s,
            toggle::Event::Move {
                px: 46.0 + 30.0 * f64::from(i) / 12.0,
                t_ms: t0 + ms(23 + i as usize - 1),
            },
        );
        s = apply(s, toggle::Event::Tick(DT));
        drag.push(toggle_now(&s));
    }
    toggle_check("lg-drag", &drag, &mut report);
    let fling = toggle_run(
        &mut s,
        &[toggle::Event::Release],
        frames(&fixture("toggle", "lg-fling")),
    );
    toggle_check("lg-fling", &fling, &mut report);

    // sm: the settings row's, two taps from its start state
    let start = fixture("toggle", "sm-start")["on"].as_bool().expect("on");
    let mut s = toggle::State::new(start, toggle::Size::Sm);
    for name in ["sm-tap-1", "sm-tap-2"] {
        let kit = toggle_run(&mut s, &TAP(25.0), frames(&fixture("toggle", name)));
        toggle_check(name, &kit, &mut report);
    }
    eprintln!("{}", report.join("\n"));
}

#[test]
fn hold_and_confirm_follow_the_web() {
    let mut report = Vec::new();
    // one hold button through the capture's sequence: held to done, let go and reset, held
    // 60 frames, let go early
    let mut s = hold::State::new();
    let mut run = |s: &mut hold::State, first: Option<hold::Event>, name: &str| {
        if let Some(e) = first {
            *s = hold::transition(*s, e).0;
        }
        let n = frames(&fixture("hold", name));
        let mut kit = Vec::with_capacity(n);
        for i in 0..n {
            if i > 0 {
                *s = hold::transition(*s, hold::Event::Tick(DT)).0;
            }
            let v = s.view();
            kit.push([v.p, v.scale_x, v.scale_y]);
        }
        let f = fixture("hold", name);
        for (j, prop) in ["p", "k", "ky"].into_iter().enumerate() {
            let ours: Vec<f64> = kit.iter().map(|v| v[j]).collect();
            check(
                &format!("hold/{name}.{prop}"),
                &ours,
                &curve(&f, prop),
                &mut report,
            );
        }
    };
    run(&mut s, Some(hold::Event::Start), "full");
    run(&mut s, Some(hold::Event::End), "done");
    run(&mut s, Some(hold::Event::Start), "early");
    run(&mut s, Some(hold::Event::End), "drain");

    // the confirm button's states over time (ms=1400)
    let f = fixture("confirm", "run");
    let web = curve(&f, "phase");
    let mut c = confirm::State::new(1400.0);
    c = confirm::transition(c, confirm::Event::Click).0;
    let mut ours = vec![f64::from(c.phase.index())];
    for _ in 1..web.len() {
        c = confirm::transition(c, confirm::Event::Tick(DT)).0;
        ours.push(f64::from(c.phase.index()));
    }
    check("confirm/run.phase", &ours, &web, &mut report);
    eprintln!("{}", report.join("\n"));
}
