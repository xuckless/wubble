//! Controls kit B's motion against the web design's, frame by frame on the fixed 1/120 s clock:
//! `tools/fidelity/fixtures/controls/*.json` are the design's pages and rules sampled in Chromium
//! (by kit B's capture script, dropped with the harness it ran on: the fixtures are data). The
//! physics (slider, segmented, the swatches' ring) replays each fixture's input script through the `wubble_core::controls` machines with
//! dt = 1/120; the CSS tweens (the seam's pill, the scroll thumb, the slider's bubble and label)
//! run the kit components on the testing backend's mock time. Each property's curve must agree
//! with the web's to 0.90 or better (1 - RMS difference / the web curve's range, as
//! motion_curves.rs). No wall clock; one test (one platform per process).

use std::fs;
use std::path::Path;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, Model};
use wubble_core::controls::segmented::key_select;
use wubble_core::controls::slider::{Cfg, Key};
use wubble_core::controls::{Ring, Segmented, Slider};

slint::slint! {
    #[edition = "1.19"]
    import { Pill, PillState } from "ui/kit/controls/seam.slint";
    import { Thumb } from "ui/kit/controls/scroller.slint";
    import { ElasticSlider } from "ui/kit/controls/slider.slint";
    export { PillState }
    export component Probe inherits Window {
        width: 400px;
        height: 300px;
        in property <PillState> pill: PillState.rest;
        in property <bool> on;
        in property <bool> hover;
        p := Pill { state: root.pill; cx: 20px; cy: 40px; }
        th := Thumb { width: 40px; height: 200px; length: 40px; shown: root.on; preview-hover: root.hover; }
        sl := ElasticSlider { preview-held: root.on; }
        out property <[float]> now: [
            p.shown-opacity, p.shown-scale,
            // how far the pill is from mist (0) to sky (1), by its red channel
            (p.shown-colour.red - 163) / (124 - 163),
            th.shown-opacity, th.shown-width / 1px,
            // the bubble: opacity, scale, lift (px), blur (px)
            sl.bubble-fade, sl.bubble-scale, sl.bubble-lift / 1px, sl.bubble-blur / 1px,
            // the label: opacity, blur (px)
            sl.label-fade, sl.label-blur / 1px,
        ];
    }
}

const DT: f64 = 1.0 / 120.0;
const DT_US: u64 = 8_333;

fn web(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fidelity/fixtures/controls")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(&path).expect("read fixture")).expect("json")
}

fn curve(w: &serde_json::Value, prop: &str) -> Vec<f64> {
    w["frames"][prop]
        .as_array()
        .unwrap_or_else(|| panic!("no {prop}"))
        .iter()
        .map(|v| v.as_f64().expect("number"))
        .collect()
}

fn num(w: &serde_json::Value, k: &str) -> f64 {
    w[k].as_f64().unwrap_or_else(|| panic!("no {k}"))
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
        if rms < 1e-3 { 1.0 } else { 0.0 }
    } else {
        1.0 - rms / (hi - lo)
    }
}

fn check(name: &str, prop: &str, ours: &[f64], report: &mut Vec<String>) {
    let w = web(name);
    let theirs = curve(&w, prop);
    assert_eq!(ours.len(), theirs.len(), "{name}.{prop}: frame count");
    let s = similarity(ours, &theirs);
    report.push(format!("{name}.{prop}: {s:.3}"));
    assert!(
        s >= 0.90,
        "{name}.{prop}: {s:.3}\nkit {ours:?}\nweb {theirs:?}"
    );
}

/// The fixture's input script: (frame, event).
fn script(w: &serde_json::Value) -> Vec<(usize, serde_json::Value)> {
    w["script"]
        .as_array()
        .expect("script")
        .iter()
        .map(|s| (s["frame"].as_u64().expect("frame") as usize, s.clone()))
        .collect()
}

fn frame_count(w: &serde_json::Value) -> usize {
    w["frames"]
        .as_object()
        .and_then(|f| f.values().next())
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len)
}

fn slider_curves(name: &str, report: &mut Vec<String>) {
    let w = web(name);
    let mut s = Slider::new(num(&w, "start"), num(&w, "w"), Cfg::default());
    let script = script(&w);
    let props = ["x", "sx", "sy", "bx", "lean", "value"];
    let mut ours: Vec<Vec<f64>> = vec![vec![]; props.len()];
    for f in 0..frame_count(&w) {
        for (_, e) in script.iter().filter(|(at, _)| *at == f) {
            if let Some(px) = e["press"].as_f64() {
                s.press(px);
            }
            if let Some(px) = e["move"].as_f64() {
                s.drag(px);
            }
            // the release asks for a segment; the value comes back and the indicator follows
            if e["release"].as_bool() == Some(true)
                && let Some(i) = s.release()
            {
                s.set(i);
            }
            if e["key"].as_str() == Some("End") {
                s.key(Key::End, false);
            }
        }
        s.step(DT);
        let v = s.view();
        for (i, x) in [v.x, v.scale_x, v.scale_y, v.bubble_x, v.lean, v.value]
            .into_iter()
            .enumerate()
        {
            ours[i].push(x);
        }
    }
    for (prop, o) in props.iter().zip(&ours) {
        check(name, prop, o, report);
    }
}

fn segmented_curves(report: &mut Vec<String>) {
    let w = web("segmented");
    let mut s = Segmented::new(num(&w, "start") as usize);
    for (i, seg) in w["segs"].as_array().expect("segs").iter().enumerate() {
        s.set_segment(
            i,
            seg[0].as_f64().expect("left"),
            seg[1].as_f64().expect("width"),
        );
    }
    let script = script(&w);
    let mut ours: Vec<Vec<f64>> = vec![vec![]; 3];
    for f in 0..frame_count(&w) {
        let t = f as f64 * DT * 1000.0;
        for (_, e) in script.iter().filter(|(at, _)| *at == f) {
            if let Some(px) = e["press"].as_f64() {
                s.press(px, t);
            }
            // the release asks for a segment; the value comes back and the indicator follows
            if e["release"].as_bool() == Some(true)
                && let Some(i) = s.release()
            {
                s.set(i);
            }
        }
        s.step(DT);
        let v = s.view();
        ours[0].push(v.left);
        ours[1].push(v.width);
        ours[2].push(v.scale_y);
    }
    for (prop, o) in ["left", "width", "sy"].iter().zip(&ours) {
        check("segmented", prop, o, report);
    }
}

fn swatches_curves(report: &mut Vec<String>) {
    let w = web("swatches");
    let mut r = Ring::new(num(&w, "start") as usize);
    let n = num(&w, "n") as usize;
    let script = script(&w);
    let mut ours: Vec<Vec<f64>> = vec![vec![]; 3];
    for f in 0..frame_count(&w) {
        for (_, e) in script.iter().filter(|(at, _)| *at == f) {
            // the key asks for the neighbour; the value comes back and the ring follows it
            if e["key"].as_str() == Some("ArrowRight") {
                r.set(key_select(r.idx(), n, 1));
            }
        }
        r.step(DT);
        let v = r.view();
        ours[0].push(v.x);
        ours[1].push(v.scale_x);
        ours[2].push(v.scale_y);
    }
    for (prop, o) in ["x", "sx", "sy"].iter().zip(&ours) {
        check("swatches", prop, o, report);
    }
}

fn now(p: &Probe) -> Vec<f64> {
    p.get_now().iter().map(f64::from).collect()
}

/// Rests in `from` (reading evaluates the bindings, so the animations start from the state),
/// switches with `to`, and samples `frames` frames, the first at the switch.
fn tween(p: &Probe, from: impl Fn(&Probe), to: impl Fn(&Probe), frames: usize) -> Vec<Vec<f64>> {
    from(p);
    slint::platform::update_timers_and_animations();
    now(p);
    mock_elapsed_time(Duration::from_secs(2));
    now(p);
    to(p);
    slint::platform::update_timers_and_animations();
    let mut out = vec![now(p)];
    for _ in 1..frames {
        mock_elapsed_time(Duration::from_micros(DT_US));
        out.push(now(p));
    }
    out
}

fn css_curves(report: &mut Vec<String>) {
    let p = Probe::new().expect("probe");
    p.show().expect("show");
    let pill = |s: PillState| move |p: &Probe| p.set_pill(s);
    let on = |v: bool| move |p: &Probe| p.set_on(v);
    let hover = |v: bool| move |p: &Probe| p.set_hover(v);
    type Run<'a> = (
        &'a str,
        Box<dyn Fn(&Probe)>,
        Box<dyn Fn(&Probe)>,
        &'a [(&'a str, usize)],
    );
    let runs: Vec<Run> = vec![
        (
            "pill-hover-in",
            Box::new(pill(PillState::Rest)),
            Box::new(pill(PillState::Hover)),
            &[("opacity", 0), ("scale", 1)],
        ),
        (
            "pill-hover-out",
            Box::new(pill(PillState::Hover)),
            Box::new(pill(PillState::Rest)),
            &[("opacity", 0), ("scale", 1)],
        ),
        (
            "pill-held-in",
            Box::new(pill(PillState::Rest)),
            Box::new(pill(PillState::Held)),
            &[("opacity", 0), ("scale", 1), ("sky", 2)],
        ),
        (
            "pill-held-out",
            Box::new(pill(PillState::Held)),
            Box::new(pill(PillState::Rest)),
            &[("opacity", 0), ("scale", 1), ("sky", 2)],
        ),
        (
            "thumb-on",
            Box::new(on(false)),
            Box::new(on(true)),
            &[("opacity", 3)],
        ),
        (
            "thumb-off",
            Box::new(on(true)),
            Box::new(on(false)),
            &[("opacity", 3)],
        ),
        (
            "thumb-hover",
            Box::new(hover(false)),
            Box::new(hover(true)),
            &[("width", 4)],
        ),
        (
            "bubble-in",
            Box::new(on(false)),
            Box::new(on(true)),
            &[("opacity", 5), ("scale", 6), ("ty", 7), ("blur", 8)],
        ),
        (
            "bubble-out",
            Box::new(on(true)),
            Box::new(on(false)),
            &[("opacity", 5), ("scale", 6), ("ty", 7), ("blur", 8)],
        ),
        (
            "label-out",
            Box::new(on(false)),
            Box::new(on(true)),
            &[("opacity", 9), ("blur", 10)],
        ),
        (
            "label-in",
            Box::new(on(true)),
            Box::new(on(false)),
            &[("opacity", 9), ("blur", 10)],
        ),
    ];
    for (name, from, to, props) in runs {
        let n = frame_count(&web(name));
        let kit = tween(&p, from, to, n);
        for (prop, i) in props {
            let ours: Vec<f64> = kit.iter().map(|f| f[*i]).collect();
            check(name, prop, &ours, report);
        }
    }
}

#[test]
fn controls_follow_the_web_curves() {
    init_integration_test_with_mock_time();
    let mut report = Vec::new();
    slider_curves("slider", &mut report);
    slider_curves("slider-key", &mut report);
    segmented_curves(&mut report);
    swatches_curves(&mut report);
    css_curves(&mut report);
    eprintln!("{}", report.join("\n"));
}
