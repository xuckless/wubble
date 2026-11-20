//! Controls kit C (kit `controls/menu.slint`, `select.slint`, `search-capsule.slint`,
//! `sheet.slint`) against the web design, frame by frame on the fixed clock:
//! `tools/fidelity/fixtures/controls/menu/*.json` are the design's CSS transitions sampled in
//! Chromium every 1/120 s (`node tools/fidelity/controls-curves.mjs`); here the kit runs on the
//! testing backend's mock time, stepped by the same 1/120 s, and each property must follow the
//! web's to 0.90 or better (1 - RMS difference / the web curve's range, as motion_curves.rs).
//! The menu's phase and highlight come from `Controls.views`, which this test writes by hand
//! (the machine itself is `wubble_core::controls::menu`, unit-tested there; the shell's wired
//! menu is driven in `src/lib.rs` and `a11y.rs`). Reduced motion lands within two frames. The
//! specimen builds. No wall clock; one test (one platform per process).

use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

slint::slint! {
    #[edition = "1.19"]
    import { Menu, MenuRow, MenuHighlight, MenuTrigger } from "ui/kit/controls/menu.slint";
    import { SearchCapsule } from "ui/kit/controls/search-capsule.slint";
    import { Controls, ControlView } from "ui/components/controls/registry.slint";
    import { Calm } from "ui/kit/motion/calm.slint";
    import { MenuSpecimen } from "ui/kit/specimen/menu.slint";
    export { Calm, Controls, ControlView }
    export component Probe inherits Window {
        width: 600px;
        height: 400px;
        in property <bool> on: false;
        in property <bool> hl-on: false;
        in property <bool> jump: true;
        in property <length> top: 0px;
        menu := Menu {
            width: 600px;
            height: 400px;
            open: root.on;
            entries: [{ label: "A", enabled: true }, { label: "B", enabled: true }];
        }
        r0 := MenuRow { shown: root.on; index: 0; width: 200px; label: "a"; }
        r3 := MenuRow { shown: root.on; index: 3; width: 200px; label: "d"; }
        // a row with an icon, one without in a menu that has icons, one in a menu without
        ri := MenuRow { y: 100px; width: 200px; label: "i"; icon: "M4 4h16"; }
        rk := MenuRow { y: 140px; width: 200px; label: "k"; icon-column: true; }
        rn := MenuRow { y: 180px; width: 200px; label: "n"; }
        out property <[length]> label-xs: [ri.label-x, rk.label-x, rn.label-x];
        hl := MenuHighlight { on: root.hl-on; jump: root.jump; top: root.top; panel-width: 236px; }
        sc := SearchCapsule { width: 300px; text: root.on ? "q" : ""; }
        trig := MenuTrigger { open: root.on; text: "x"; }
        out property <[float]> now: [
            menu.body-e, menu.body-zoom, menu.body-blur / 1px,
            r0.e, r0.offset / 1px, r3.e, r3.offset / 1px,
            hl.y / 1px, hl.opacity,
            sc.clear-opacity, sc.clear-scale, sc.clear-blur / 1px,
            cos(trig.chevron-turn * 180deg),
        ];
        out property <bool> menu-mounted: menu.mounted;
    }
    export component Sheet1 inherits Window {
        width: 1400px;
        height: 900px;
        spec := MenuSpecimen { }
        out property <length> specimen-height: spec.preferred-height;
    }
}

/// The frame step: the web sampler's 1/120 s.
const DT_US: u64 = 8_333;

/// (fixture, its property, index into `Probe.now`) for a flip to on, then to off.
const IN: &[(&str, &[(&str, usize)])] = &[
    ("menu-in", &[("opacity", 0), ("sx", 1), ("blur", 2)]),
    ("mrow-in-0", &[("opacity", 3), ("ty", 4)]),
    ("mrow-in-3", &[("opacity", 5), ("ty", 6)]),
    ("clear-in", &[("opacity", 9), ("sx", 10), ("blur", 11)]),
    ("chev", &[("cos", 12)]),
];
const OUT: &[(&str, &[(&str, usize)])] = &[
    ("menu-out", &[("opacity", 0), ("sx", 1), ("blur", 2)]),
    ("mrow-out", &[("opacity", 3), ("ty", 4)]),
    ("clear-out", &[("opacity", 9), ("sx", 10), ("blur", 11)]),
];

fn web(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fidelity/fixtures/controls/menu")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(&path).expect("read fixture")).expect("fixture json")
}

fn frames(name: &str) -> usize {
    web(name)["frames"]["opacity"]
        .as_array()
        .expect("frames")
        .len()
}

fn now(p: &Probe) -> Vec<f32> {
    p.get_now().iter().collect()
}

/// The menu's view as the registry would write it: open (phase 1) or closed.
fn set_on(p: &Probe, views: &VecModel<ControlView>, on: bool) {
    let mut v = views.row_data(0).expect("slot 0");
    v.phase = i32::from(on);
    views.set_row_data(0, v);
    p.set_on(on);
    // the event loop's turn before a frame: change handlers run
    slint::platform::update_timers_and_animations();
}

fn settle(p: &Probe, views: &VecModel<ControlView>, on: bool) -> Vec<f32> {
    set_on(p, views, on);
    now(p);
    mock_elapsed_time(Duration::from_secs(2));
    now(p)
}

/// `n` samples of `Probe.now`, the first at the change `f` makes, then one per step.
fn run(p: &Probe, n: usize, f: impl FnOnce()) -> Vec<Vec<f32>> {
    f();
    slint::platform::update_timers_and_animations();
    let mut out = vec![now(p)];
    for _ in 1..n {
        mock_elapsed_time(Duration::from_micros(DT_US));
        out.push(now(p));
    }
    out
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
        .fold((f64::MAX, f64::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    if hi - lo < 1e-9 {
        if rms < 1e-6 { 1.0 } else { 0.0 }
    } else {
        1.0 - rms / (hi - lo)
    }
}

fn check(name: &str, props: &[(&str, usize)], kit: &[Vec<f32>], report: &mut Vec<String>) {
    let w = web(name);
    for (prop, i) in props {
        let web: Vec<f64> = w["frames"][prop]
            .as_array()
            .expect("prop")
            .iter()
            .map(|v| v.as_f64().expect("number"))
            .collect();
        let ours: Vec<f64> = kit.iter().map(|f| f64::from(f[*i])).collect();
        let s = similarity(&ours, &web);
        report.push(format!("{name}.{prop} (kit value {i}): {s:.3}"));
        assert!(
            s >= 0.90,
            "{name}.{prop}: {s:.3}\nkit {ours:?}\nweb {web:?}"
        );
    }
}

#[test]
fn kit_menu_follows_the_web_curves() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    // the registry, by hand: one slot whose view the test writes
    let views = Rc::new(VecModel::from(vec![ControlView {
        scale_x: 1.0,
        scale_y: 1.0,
        origin: 0.5,
        hl: -1,
        jump: true,
        focus: -1,
        done: -1,
        ..ControlView::default()
    }]));
    let g = p.global::<Controls>();
    g.set_views(ModelRc::from(views.clone()));
    g.on_acquire(|_, _| 0);
    p.show().unwrap();
    let mut report = Vec::new();

    // A menu with icons keeps the icon's column on a row without one (16 px and the 10 gap),
    // as the web's rows all carry one; a menu without icons starts its labels at the padding.
    let xs: Vec<f32> = p.get_label_xs().iter().collect();
    assert_eq!(xs[0], xs[1], "the icon-less row lines up with the icon row");
    assert_eq!(xs[0] - xs[2], 16.0 + 10.0);

    for (pairs, on) in [(IN, true), (OUT, false)] {
        for (name, props) in pairs {
            settle(&p, &views, !on);
            let kit = run(&p, frames(name), || set_on(&p, &views, on));
            check(name, props, &kit, &mut report);
        }
    }
    assert!(!p.get_menu_mounted(), "closed: the body is not drawn");

    // The one highlight: glides from row 0 to row 2; arrives in place (jump) and only fades in;
    // fades out.
    p.set_hl_on(true);
    p.set_jump(false);
    p.set_top(0.0);
    mock_elapsed_time(Duration::from_secs(1));
    now(&p);
    let glide = run(&p, frames("hl-glide"), || p.set_top(72.0));
    check("hl-glide", &[("ty", 7)], &glide, &mut report);
    p.set_hl_on(false);
    p.set_top(0.0);
    mock_elapsed_time(Duration::from_secs(1));
    now(&p);
    let jump = run(&p, frames("hl-jump"), || {
        p.set_jump(true);
        p.set_hl_on(true);
        p.set_top(72.0);
    });
    assert!(
        (jump[0][7] - 72.0).abs() < 1e-3,
        "a jump lands in place: {}",
        jump[0][7]
    );
    check("hl-jump", &[("opacity", 8)], &jump, &mut report);
    mock_elapsed_time(Duration::from_secs(1));
    now(&p);
    let fade = run(&p, frames("hl-fade"), || p.set_hl_on(false));
    check("hl-fade", &[("opacity", 8)], &fade, &mut report);
    eprintln!("{}", report.join("\n"));

    // Reduced motion: the design's 1 ms; the body, the leaving rows, the highlight, the clear
    // button and the chevron land within two frames (rows keep their entrance delays, as CSS).
    p.global::<Calm>().set_reduced(true);
    for on in [true, false] {
        let end = settle(&p, &views, on);
        settle(&p, &views, !on);
        let f = run(&p, 3, || set_on(&p, &views, on));
        let check_at: &[usize] = if on {
            &[0, 1, 2, 9, 10, 11, 12]
        } else {
            &[0, 1, 2, 3, 4, 5, 6, 9, 10, 11, 12]
        };
        for &i in check_at {
            assert!(
                (f[2][i] - end[i]).abs() < 1e-3,
                "reduced, on={on}, value {i}: {} vs {}",
                f[2][i],
                end[i]
            );
        }
    }
    p.global::<Calm>().set_reduced(false);

    // The specimen builds and lays out (its cells carry element ids, `MenuSpecimen::<cell>`, for
    // the gallery; the testing backend reads ids and roles only where the compiler kept debug
    // info, the app's build.rs, not this slint! macro, so they are not looked up here).
    let sheet = Sheet1::new().unwrap();
    sheet.global::<Controls>().on_acquire(|_, _| 0);
    sheet.show().unwrap();
    mock_elapsed_time(Duration::from_secs(1));
    assert!(sheet.get_specimen_height() > 0.0, "the specimen lays out");
}
