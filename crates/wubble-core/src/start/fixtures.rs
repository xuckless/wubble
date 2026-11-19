//! The port against the web: trajectories sampled from the design's own `SoftBody` and `Minis`
//! (and its `living.ts` loop, transcribed) by `tools/fidelity/sample-start.mjs` on the fixed
//! 1/120 s clock, with the web's `Math.random()` draws replayed in order. Each curve must agree
//! (`1 - rms / range` >= 0.90, the kit's curve bar) and, since the arithmetic is the same IEEE
//! operations, stay within 1e-6 of the web at every step (only libm's last bits can differ).
//! Counts, steps and random seeds compare exactly. No clocks anywhere.

use super::living::{Effect, Event, Living, Options, transition};
use super::minis::{Circle, Minis, Throw};
use super::rand::{Random, Replay};
use super::softbody::{PopState, SoftBody};
use crate::kit_gen::softbody as KB;
use serde_json::Value;
use std::collections::BTreeMap;

const AGREE: f64 = 0.90;
const MAX_ERR: f64 = 1e-6;

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).expect("fixture json")
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .expect("array")
        .iter()
        .map(|x| x.as_f64().expect("number"))
        .collect()
}

fn draws(fx: &Value) -> Replay {
    Replay::new(floats(&fx["draws"]))
}

fn dt(fx: &Value) -> f64 {
    fx["dt"].as_f64().unwrap()
}

/// `1 - rms(web - rust) / range(web)`, clamped to 0..1.
fn agreement(web: &[f64], rust: &[f64]) -> f64 {
    let n = web.len() as f64;
    let rms = (web
        .iter()
        .zip(rust)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        / n)
        .sqrt();
    let (lo, hi) = web
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    (1.0 - rms / (hi - lo).max(1e-9)).clamp(0.0, 1.0)
}

fn check_curve(scenario: &str, name: &str, web: &[f64], rust: &[f64]) {
    assert_eq!(web.len(), rust.len(), "{scenario} {name}: length");
    let a = agreement(web, rust);
    assert!(a >= AGREE, "{scenario} {name}: agreement {a:.4}");
    for (i, (w, r)) in web.iter().zip(rust).enumerate() {
        assert!(
            (w - r).abs() < MAX_ERR,
            "{scenario} {name}: step {i}: web {w}, rust {r}"
        );
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < MAX_ERR
}

// ── the soft body alone ──

fn body_scenario(name: &str, text: &str) {
    let fx = fixture(text);
    let dt = dt(&fx);
    let mut rnd = draws(&fx);
    let mut b = SoftBody::new(None, None, rnd.random() * KB::seed);
    let home = floats(&fx["home"]);
    b.home = (home[0], home[1]);
    let events = fx["events"].as_array().unwrap();
    let steps = fx["steps"].as_u64().unwrap() as usize;
    let mut got: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for i in 0..steps {
        for e in events
            .iter()
            .filter(|e| e["step"].as_u64() == Some(i as u64))
        {
            let (x, y, t) = (
                e["x"].as_f64().unwrap_or(0.0),
                e["y"].as_f64().unwrap_or(0.0),
                e["t_ms"].as_f64().unwrap(),
            );
            match e["kind"].as_str().unwrap() {
                "down" => b.down(x, y, t),
                "move" => b.r#move(x, y, t),
                "up" => {
                    b.up();
                }
                "leave" => b.leave(),
                "poke" => b.poke(x, y, 1.0),
                k => panic!("unknown event {k}"),
            }
        }
        b.step(dt);
        let s = &b.state;
        let w = s.w.unwrap();
        let grab = match b.grab {
            super::grab::Grab::Free => 0.0,
            super::grab::Grab::Held(_) => 1.0,
            super::grab::Grab::Dragging(_) => 2.0,
        };
        for (k, v) in [
            ("ring0", b.ring[0] as f64),
            ("ring6", b.ring[6] as f64),
            ("ring12", b.ring[12] as f64),
            ("ring18", b.ring[18] as f64),
            ("cx", s.x),
            ("cy", s.y),
            ("yaw", b.yaw.x),
            ("pitch", b.pitch.x),
            ("swirl", s.swirl),
            ("winv0", w.inv[0]),
            ("winv1", w.inv[1]),
            ("winv2", w.inv[2]),
            ("winv3", w.inv[3]),
            ("woff0", w.off[0]),
            ("woff1", w.off[1]),
            ("grab", grab),
        ] {
            got.entry(k).or_default().push(v);
        }
    }
    for (k, rust) in &got {
        let web = floats(&fx["curves"][k]);
        if *k == "grab" {
            assert_eq!(&web, rust, "grab states");
        } else {
            check_curve(name, k, &web, rust);
        }
    }
    assert_eq!(rnd.taken(), fx["draws"].as_array().unwrap().len());
}

#[test]
fn soft_body_at_rest_breathes_like_the_web() {
    body_scenario("rest", include_str!("../../tests/fixtures/start_rest.json"));
}

#[test]
fn soft_body_leans_toward_the_pointer_like_the_web() {
    body_scenario("lean", include_str!("../../tests/fixtures/start_lean.json"));
}

#[test]
fn soft_body_poked_in_the_middle_squashes_the_w_like_the_web() {
    body_scenario(
        "poke_mid",
        include_str!("../../tests/fixtures/start_poke_mid.json"),
    );
}

#[test]
fn soft_body_poked_on_the_rim_dents_like_the_web() {
    body_scenario(
        "poke_rim",
        include_str!("../../tests/fixtures/start_poke_rim.json"),
    );
}

#[test]
fn soft_body_dragged_and_flung_like_the_web() {
    body_scenario(
        "drag_fling",
        include_str!("../../tests/fixtures/start_drag_fling.json"),
    );
}

// ── pops ──

fn check_pops(what: &str, web: &[Value], rust: &[(usize, PopState)]) {
    assert_eq!(web.len(), rust.len(), "{what}: pop count");
    for (w, (step, p)) in web.iter().zip(rust) {
        assert_eq!(
            w["step"].as_u64().unwrap() as usize,
            *step,
            "{what}: pop step"
        );
        assert_eq!(w["seed"].as_f64().unwrap(), p.seed, "{what}: pop seed");
        assert_eq!(w["life"].as_f64().unwrap(), p.life, "{what}: pop life");
        for (k, v) in [("x", p.x), ("y", p.y), ("r", p.r), ("angle", p.angle)] {
            assert!(
                close(w[k].as_f64().unwrap(), v),
                "{what}: pop {k}: {w} vs {p:?}"
            );
        }
    }
}

// ── the minis alone ──

#[test]
fn minis_spawn_rise_avoid_and_pop_like_the_web() {
    let fx = fixture(include_str!("../../tests/fixtures/start_minis.json"));
    let dt = dt(&fx);
    let mut rnd = draws(&fx);
    let mut m = Minis::new();
    let av = &fx["avoid"];
    let avoid = [Circle {
        x: av["x"].as_f64().unwrap(),
        y: av["y"].as_f64().unwrap(),
        r: av["r"].as_f64().unwrap(),
    }];
    let snaps = fx["snaps"].as_array().unwrap();
    let mut web: BTreeMap<(u64, usize), Vec<f64>> = BTreeMap::new();
    let mut rust: BTreeMap<(u64, usize), Vec<f64>> = BTreeMap::new();
    let mut pops = Vec::new();
    for (i, snap) in snaps.iter().enumerate() {
        match i {
            0 => {
                m.spawn(400.0, 420.0, 3, Throw::Burst, 1.0, &mut rnd);
                m.spawn(300.0, 700.0, 1, Throw::Gentle, 1.0, &mut rnd);
            }
            30 => m.spawn(420.0, 330.0, 5, Throw::Burst, 0.8, &mut rnd),
            60 => {
                let e = &fx["events"][2];
                assert_eq!(e["kind"], "hit");
                let (x, y) = (e["x"].as_f64().unwrap(), e["y"].as_f64().unwrap());
                assert!(m.hit(x, y, &mut rnd));
            }
            _ => {}
        }
        m.step(dt, i as f64 * dt, &avoid, &mut rnd);
        pops.extend(m.pops.drain(..).map(|p| (i, p)));
        let snap = snap.as_array().unwrap();
        assert_eq!(snap.len(), m.list.len(), "step {i}: mini count");
        for (w, r) in snap.iter().zip(&m.list) {
            let id = w[0].as_u64().unwrap();
            let r = [r.x, r.y, r.b.r, r.b.swirl];
            for k in 0..4 {
                web.entry((id, k))
                    .or_default()
                    .push(w[k + 1].as_f64().unwrap());
                rust.entry((id, k)).or_default().push(r[k]);
            }
        }
    }
    let names = ["x", "y", "b.r", "b.swirl"];
    for (key, w) in &web {
        check_curve(&format!("mini {}", key.0), names[key.1], w, &rust[key]);
    }
    check_pops("minis", fx["pops"].as_array().unwrap(), &pops);
    assert_eq!(rnd.taken(), fx["draws"].as_array().unwrap().len());
}

// ── the living loop ──

#[test]
fn the_living_loop_runs_like_the_web() {
    let fx = fixture(include_str!("../../tests/fixtures/start_living.json"));
    let dt = dt(&fx);
    let size = floats(&fx["size"]);
    let (mut l, _) = transition(
        Living::new(Options::start_page(), draws(&fx)),
        Event::Resize {
            w: size[0],
            h: size[1],
        },
    );
    let events = fx["events"].as_array().unwrap();
    let c = &fx["curves"];
    let steps = fx["steps"].as_u64().unwrap() as usize;
    let (mut cx, mut cy, mut time) = (vec![], vec![], vec![]);
    let mut ripples = vec![];
    for i in 0..steps {
        let mut fx_all = vec![];
        for e in events
            .iter()
            .filter(|e| e["step"].as_u64() == Some(i as u64))
        {
            let (x, y, t_ms) = (
                e["x"].as_f64().unwrap_or(0.0),
                e["y"].as_f64().unwrap_or(0.0),
                e["t_ms"].as_f64().unwrap(),
            );
            let ev = match e["kind"].as_str().unwrap() {
                "poke" => Event::Poke { rim: None },
                "down" => Event::Down { x, y, t_ms },
                "move" => Event::Move { x, y, t_ms },
                "up" => Event::Up,
                k => panic!("unknown event {k}"),
            };
            let (next, out) = transition(l, ev);
            l = next;
            fx_all.extend(out);
        }
        let (next, out) = transition(l, Event::Tick { dt });
        l = next;
        fx_all.extend(out);
        for e in fx_all {
            if let Effect::Ripple {
                x,
                y,
                reach,
                strength,
            } = e
            {
                ripples.push((i, x, y, reach, strength));
            }
        }
        cx.push(l.body.state.x);
        cy.push(l.body.state.y);
        time.push(l.time);
        assert_eq!(
            l.minis.list.len() as u64,
            c["minis"][i].as_u64().unwrap(),
            "step {i}: mini count"
        );
        assert_eq!(
            l.body.state.waves.len() as u64,
            c["waves"][i].as_u64().unwrap(),
            "step {i}: waves"
        );
        let web_pops = c["pops"][i].as_array().unwrap();
        assert_eq!(web_pops.len(), l.pops.len(), "step {i}: pops drawn");
        for (w, p) in web_pops.iter().zip(&l.pops) {
            let w = floats(w);
            assert_eq!(w[5], p.seed, "step {i}: pop seed");
            for (a, b) in w[..5].iter().zip([p.x, p.y, p.r, p.age, p.angle]) {
                assert!(close(*a, b), "step {i}: pop {w:?} vs {p:?}");
            }
        }
    }
    check_curve("living", "cx", &floats(&c["cx"]), &cx);
    check_curve("living", "cy", &floats(&c["cy"]), &cy);
    check_curve("living", "time", &floats(&c["time"]), &time);
    let web_ripples = fx["effects"].as_array().unwrap();
    assert_eq!(web_ripples.len(), ripples.len(), "ripples");
    for (w, (i, x, y, reach, strength)) in web_ripples.iter().zip(&ripples) {
        assert_eq!(w["step"].as_u64().unwrap() as usize, *i);
        for (k, v) in [("x", x), ("y", y), ("reach", reach), ("strength", strength)] {
            assert!(close(w[k].as_f64().unwrap(), *v), "ripple {k}: {w}");
        }
    }
}
