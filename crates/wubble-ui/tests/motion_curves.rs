//! The kit's fixed-curve motion against the web design's, frame by frame on a fixed clock:
//! `tools/fidelity/curves/*.json` are the design's CSS animations sampled in Chromium every
//! 1/120 s (`node tools/fidelity/curves.mjs`); here the kit primitives run on the testing
//! backend's mock time, stepped by the same 1/120 s, and each property's curve must agree with
//! the web's to 0.90 or better (1 - RMS difference / the web curve's range). Reduced motion: every
//! primitive lands in its end state within two frames. A flip mid-run restarts Overlap and the
//! keyframes. No wall clock; one test (one platform per
//! process).

use std::fs;
use std::path::Path;
use std::time::Duration;

use i_slint_backend_testing::{init_integration_test_with_mock_time, mock_elapsed_time};
use slint::{ComponentHandle, Model};

slint::slint! {
    #[edition = "1.19"]
    import { OverlapLayer } from "ui/kit/motion/overlap.slint";
    import { Press } from "ui/kit/motion/press.slint";
    import { Film } from "ui/kit/motion/film.slint";
    import { Wobble } from "ui/kit/motion/wobble.slint";
    import { Pulse } from "ui/kit/motion/pulse.slint";
    import { Fold } from "ui/kit/motion/fold.slint";
    import { CrossfadeText } from "ui/kit/motion/crossfade.slint";
    import { Calm } from "ui/kit/motion/calm.slint";
    export { Calm }
    export component Probe inherits Window {
        width: 200px;
        height: 200px;
        in property <bool> on: false;
        ov := OverlapLayer { shown: root.on; width: 100px; height: 40px; }
        press := Press { pressed: root.on; }
        film := Film { on: root.on; radius: 8px; width: 100px; height: 40px; }
        wob := Wobble { hit: root.on; width: 20px; height: 20px; }
        pulse := Pulse { hit: root.on; radius: 8px; width: 20px; height: 20px; }
        fold := Fold { open: root.on; content-height: 40px; width: 100px; }
        xf := CrossfadeText { text: root.on ? "on" : "off"; width: 100px; height: 20px; ink: white; }
        out property <[float]> now: [
            ov.progress, ov.offset / 1px, ov.zoom, ov.blur / 1px,
            press.scale,
            film.shown,
            wob.rotation / 1deg, wob.scale-x, wob.scale-y,
            pulse.e, fold.e, xf.e, xf.leaving,
            pulse.spread / 1px, pulse.ring.to-hsv().alpha, fold.height / 40px,
        ];
    }
}

/// The frame step: the web sampler's 1/120 s.
const DT_US: u64 = 8_333;

/// What each web curve is compared with: (curve file, its property, index into `Probe.now`).
const PAIRS: &[(&str, &[(&str, usize)])] = &[
    (
        "overlap-in",
        &[("opacity", 0), ("ty", 1), ("sx", 2), ("blur", 3)],
    ),
    ("press-in", &[("sx", 4)]),
    ("film-in", &[("opacity", 5)]),
    ("wobble", &[("rot", 6), ("sx", 7), ("sy", 8)]),
    // the Swap's entrance is `wb-in`'s (touch.css:28-29: 240 ms ease-out, +7, .98, blur 4)
    ("overlap-in", &[("opacity", 11)]),
    ("pulse", &[("spread", 13), ("alpha", 14)]),
    ("fold-in", &[("h", 15), ("opacity", 10)]),
];
const PAIRS_OUT: &[(&str, &[(&str, usize)])] = &[
    (
        "overlap-out",
        &[("opacity", 0), ("ty", 1), ("sx", 2), ("blur", 3)],
    ),
    ("press-out", &[("sx", 4)]),
    ("film-out", &[("opacity", 5)]),
    // the Swap's exit fades as `wb-out` (touch.css:30: 190 ms ease-in-out; it shifts -7, not -5)
    ("overlap-out", &[("opacity", 12)]),
    ("fold-out", &[("h", 15), ("opacity", 10)]),
];

/// The wobble's values in `Probe.now`.
const WOBBLE: std::ops::Range<usize> = 6..9;

fn web(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fidelity/curves")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(&path).expect("read curve")).expect("curve json")
}

fn now(p: &Probe) -> Vec<f32> {
    p.get_now().iter().collect()
}

/// Holds `on` until every primitive is at rest there (reading evaluates the bindings, so the
/// animations start from the state, not from the first read after the next flip).
fn settle(p: &Probe, on: bool) -> Vec<f32> {
    p.set_on(on);
    slint::platform::update_timers_and_animations();
    now(p);
    mock_elapsed_time(Duration::from_secs(2));
    now(p)
}

/// `frames` samples of `Probe.now`, the first at the flip, then one per step.
fn run(p: &Probe, on: bool, frames: usize) -> Vec<Vec<f32>> {
    p.set_on(on);
    // the event loop's turn before a frame: change handlers run (Crossfade's swap)
    slint::platform::update_timers_and_animations();
    let mut out = vec![now(p)];
    for _ in 1..frames {
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

/// Asserts that the kit's `kit` frames follow the web curve `name` in each of `props`.
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

fn frames(name: &str) -> usize {
    web(name)["frames"]["opacity"]
        .as_array()
        .expect("frames")
        .len()
}

fn compare(p: &Probe, on: bool, pairs: &[(&str, &[(&str, usize)])], report: &mut Vec<String>) {
    for (name, props) in pairs {
        // from rest in the start state
        settle(p, !on);
        let kit = run(p, on, frames(name));
        check(name, props, &kit, report);
    }
}

#[test]
fn kit_motion_follows_the_web_curves() {
    init_integration_test_with_mock_time();
    let p = Probe::new().unwrap();
    p.show().unwrap();
    let mut report = Vec::new();
    compare(&p, true, PAIRS, &mut report);
    compare(&p, false, PAIRS_OUT, &mut report);

    // A flip mid-run restarts the keyframes, as the web's two names per animation do: a re-entry
    // mid-exit comes in from clear and 7 px below again; a second wobble plays from its start.
    settle(&p, true);
    run(&p, false, 12);
    let back = run(&p, true, frames("overlap-in"));
    assert!(
        back[0][0] < 1e-3 && (back[0][1] - 7.0).abs() < 1e-3,
        "re-entry restarts: {:?}",
        &back[0][..4]
    );
    check(
        "overlap-in",
        &[("opacity", 0), ("ty", 1)],
        &back,
        &mut report,
    );
    run(&p, false, 20);
    let again = run(&p, true, frames("wobble"));
    check(
        "wobble",
        &[("rot", 6), ("sx", 7), ("sy", 8)],
        &again,
        &mut report,
    );
    eprintln!("{}", report.join("\n"));

    // Reduced motion: the design's 1 ms; every primitive is at its end within two frames.
    p.global::<Calm>().set_reduced(true);
    for on in [true, false] {
        let rest_end = settle(&p, on);
        settle(&p, !on);
        let frames = run(&p, on, 3);
        // the wobble plays per flip (no end state of its own); the rest land
        for (i, (a, b)) in frames[2].iter().zip(&rest_end).enumerate() {
            if !WOBBLE.contains(&i) {
                assert!(
                    (a - b).abs() < 1e-3,
                    "reduced, on={on}, value {i}: {a} vs {b}"
                );
            }
        }
        let wob = &frames[2][WOBBLE];
        assert!(
            wob.iter()
                .zip([0.0, 1.0, 1.0])
                .all(|(a, b)| (a - b).abs() < 1e-3),
            "reduced wobble {wob:?}"
        );
    }
}
