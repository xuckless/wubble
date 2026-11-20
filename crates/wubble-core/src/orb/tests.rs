//! The port against the design's own engine: `tests/fixtures/orb_frames.json` holds frames the
//! TS computed (`tools/fidelity/sample-orbs.mjs`), every state at every tuned size at fixed times.
//! No clocks; pure numbers.

use super::*;
use serde_json::Value;

const FIXTURE: &str = include_str!("../../tests/fixtures/orb_frames.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("orb_frames.json")
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("a number: {v}"))
}

/// Equal to 1e-9, relative for big sums (the TS and Rust libms may differ in the last bit).
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

fn state(name: &str) -> OrbState {
    OrbState::ALL
        .into_iter()
        .find(|s| s.name() == name)
        .unwrap_or_else(|| panic!("state {name}"))
}

fn size(px: u64) -> OrbSize {
    OrbSize::ALL
        .into_iter()
        .find(|s| u64::from(s.px()) == px)
        .unwrap_or_else(|| panic!("size {px}"))
}

fn check_opts(what: &str, ts: &Value, ours: &ModeOpts) {
    let ts = ts.as_object().expect("opts object");
    for (k, v) in ts {
        let got = ours.get(k).unwrap_or_else(|| panic!("{what}: no {k}"));
        assert!(close(got, num(v)), "{what}: {k} {got} vs {v}");
    }
    assert_eq!(ours.0.len(), ts.len(), "{what}: keys {:?}", ours.0);
}

fn dot_row(d: &Dot) -> [Option<f64>; 6] {
    [
        Some(d.x),
        Some(d.y),
        Some(d.z),
        Some(d.r),
        Some(d.white),
        d.a,
    ]
}

fn line_row(l: &Line) -> [Option<f64>; 7] {
    [
        Some(l.x1),
        Some(l.y1),
        Some(l.x2),
        Some(l.y2),
        Some(l.white),
        l.a,
        Some(l.w),
    ]
}

fn check_row(what: &str, ts: &Value, ours: &[Option<f64>]) {
    let ts = ts.as_array().expect("row");
    assert_eq!(ts.len(), ours.len(), "{what}");
    for (i, (t, o)) in ts.iter().zip(ours).enumerate() {
        match (t.as_f64(), o) {
            (None, None) => {}
            (Some(t), Some(o)) => assert!(close(t, *o), "{what}[{i}]: {o} vs {t}"),
            _ => panic!("{what}[{i}]: {o:?} vs {t}"),
        }
    }
}

fn sum<T>(xs: &[T], f: impl Fn(&T) -> f64) -> f64 {
    xs.iter().map(f).sum()
}

#[test]
fn every_preset_resolves_to_the_designs_options_and_speed() {
    let fx = fixture();
    let frames = fx["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 27, "nine states × three sizes");
    for f in frames {
        let (s, z) = (
            state(f["state"].as_str().unwrap()),
            size(f["size"].as_u64().unwrap()),
        );
        let r = resolve_preset(s, z);
        let what = format!("{}-{}", s.name(), z.px());
        assert_eq!(r.mode.name(), f["mode"].as_str().unwrap(), "{what}");
        assert!(close(r.speed, num(&f["speed"])), "{what} speed");
        check_opts(&what, &f["opts"], &r.opts);
    }
}

#[test]
fn every_frame_is_the_designs() {
    let fx = fixture();
    for f in fx["frames"].as_array().unwrap() {
        let (s, z) = (
            state(f["state"].as_str().unwrap()),
            size(f["size"].as_u64().unwrap()),
        );
        let orb = Orb::new(s, z, OrbTheme::Dark);
        for at in f["at"].as_array().unwrap() {
            let t = num(&at["t"]);
            let what = format!("{}-{} at {t}", s.name(), z.px());
            let frame = orb.frame(t);
            assert_eq!(
                frame.dots.len() as u64,
                at["dots"].as_u64().unwrap(),
                "{what}: dots"
            );
            assert_eq!(
                frame.lines.len() as u64,
                at["lines"].as_u64().unwrap(),
                "{what}: lines"
            );
            let ds = &at["dot_sums"];
            let d = &frame.dots;
            for (k, got) in [
                ("x", sum(d, |d| d.x)),
                ("y", sum(d, |d| d.y)),
                ("z", sum(d, |d| d.z)),
                ("r", sum(d, |d| d.r)),
                ("white", sum(d, |d| d.white)),
                ("a", sum(d, |d| d.a.unwrap_or(1.0))),
            ] {
                assert!(close(got, num(&ds[k])), "{what}: Σ{k} {got} vs {}", ds[k]);
            }
            let ls = &at["line_sums"];
            let l = &frame.lines;
            for (k, got) in [
                ("x1", sum(l, |l| l.x1)),
                ("y2", sum(l, |l| l.y2)),
                ("a", sum(l, |l| l.a.unwrap_or(1.0))),
                ("w", sum(l, |l| l.w)),
            ] {
                assert!(close(got, num(&ls[k])), "{what}: Σ{k} {got} vs {}", ls[k]);
            }
            let head = at["head"].as_array().unwrap();
            for (i, row) in head.iter().enumerate() {
                check_row(&format!("{what} dot {i}"), row, &dot_row(&d[i]));
            }
            let tail = at["tail"].as_array().unwrap();
            for (i, row) in tail.iter().enumerate() {
                let k = d.len() - tail.len() + i;
                check_row(&format!("{what} dot {k}"), row, &dot_row(&d[k]));
            }
            for (i, row) in at["first_lines"].as_array().unwrap().iter().enumerate() {
                check_row(&format!("{what} line {i}"), row, &line_row(&l[i]));
            }
        }
    }
}

#[test]
fn the_density_and_dot_size_knobs_scale_as_the_component_does() {
    let fx = fixture();
    for k in fx["knobs"].as_array().unwrap() {
        let m = num(&k["k"]);
        let orb = Orb {
            dots: m,
            dot_size: m,
            ..Orb::new(OrbState::Searching, OrbSize::S64, OrbTheme::Dark)
        };
        check_opts(&format!("knob {m}"), &k["opts"], &orb.resolved().opts);
        let f = orb.frame(4.1);
        assert_eq!(f.dots.len() as u64, k["dots"].as_u64().unwrap());
        assert!(close(sum(&f.dots, |d| d.r), num(&k["dot_sums"]["r"])));
    }
}

#[test]
fn inks_ramp_as_the_painter_does() {
    let fx = fixture();
    for row in fx["inks"].as_array().unwrap() {
        let tint = row["tint"].as_object().map(|t| OrbTint {
            r: num(&t["r"]),
            g: num(&t["g"]),
            b: num(&t["b"]),
        });
        let got = prim::ink_color(num(&row["w"]), 0.7, row["dark"].as_bool().unwrap(), tint);
        let want: Vec<f64> = row["ink"].as_array().unwrap().iter().map(num).collect();
        assert_eq!(got.to_vec(), want, "{row}");
    }
}

#[test]
fn the_labels_and_modes_are_the_designs() {
    assert_eq!(OrbState::Breathing.label(), "Thinking…");
    assert_eq!(OrbState::Breathing.mode(), ModeKey::Ring);
    assert_eq!(OrbState::Shaping.mode(), ModeKey::Morph);
    assert_eq!(OrbSize::ALL.map(OrbSize::px), [64, 32, 20]);
}

#[test]
fn orbs_of_one_state_share_the_clock() {
    let a = Orb::new(OrbState::Working, OrbSize::S64, OrbTheme::Dark);
    let b = Orb { speed: 2.0, ..a };
    assert!(close(a.time(10.0), 18.85), "the preset's speed 1.885");
    assert!(close(b.time(10.0), 2.0 * a.time(10.0)));
}

#[test]
fn an_orb_off_screen_draws_nothing_and_asks_for_nothing() {
    use LifeEffect::*;
    let (life, fx) = transition(Life::Off, LifeEvent::Pause { t: 1.0 });
    assert_eq!((life, fx), (Life::Off, vec![]));
    let (life, fx) = transition(Life::Off, LifeEvent::Play);
    assert_eq!((life, fx), (Life::Off, vec![]));
    // shown: one frame at once, then frames
    let (life, fx) = transition(
        Life::Off,
        LifeEvent::Shown {
            t: 3.0,
            paused: false,
            reduced: false,
        },
    );
    assert_eq!(
        (life, fx),
        (Life::Running, vec![Paint { t: 3.0 }, Animate(true)])
    );
    assert!(life.animating());
    let (life, fx) = transition(life, LifeEvent::Hidden);
    assert_eq!((life, fx), (Life::Off, vec![Animate(false)]));
    assert!(!life.shown());
}

#[test]
fn a_paused_orb_is_drawn_once_and_costs_no_frames() {
    use LifeEffect::*;
    let shown = LifeEvent::Shown {
        t: 2.5,
        paused: true,
        reduced: false,
    };
    let (life, fx) = transition(Life::Off, shown);
    assert_eq!((life, fx), (Life::Still { t: 2.5 }, vec![Paint { t: 2.5 }]));
    assert!(!life.animating());
    // pausing a running orb keeps the frame on screen, stops the frames
    let (life, fx) = transition(Life::Running, LifeEvent::Pause { t: 7.0 });
    assert_eq!((life, fx), (Life::Still { t: 7.0 }, vec![Animate(false)]));
    let (life, fx) = transition(life, LifeEvent::Play);
    assert_eq!((life, fx), (Life::Running, vec![Animate(true)]));
    // shown again while shown changes nothing
    assert_eq!(transition(Life::Running, shown), (Life::Running, vec![]));
}

#[test]
fn reduced_motion_holds_the_representative_frame() {
    use LifeEffect::*;
    let (life, fx) = transition(
        Life::Off,
        LifeEvent::Shown {
            t: 9.0,
            paused: false,
            reduced: true,
        },
    );
    assert_eq!((life, fx), (Life::Reduced, vec![Paint { t: 0.6 }]));
    assert_eq!(transition(life, LifeEvent::Play), (Life::Reduced, vec![]));
    assert_eq!(
        transition(Life::Running, LifeEvent::Reduce(true)),
        (Life::Reduced, vec![Animate(false), Paint { t: 0.6 }])
    );
    assert_eq!(
        transition(Life::Reduced, LifeEvent::Reduce(false)),
        (Life::Running, vec![Animate(true)])
    );
}
