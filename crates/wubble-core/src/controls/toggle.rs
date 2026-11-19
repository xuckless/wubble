//! The toggle: the owner's design `pages/touch.tsx:139-240` (`TOGGLE_SIZE`, `TOGGLE`, the
//! `Toggle` loop and pointer handlers; @source touch-toggle, the-wallpaper-project @598f814),
//! translated onto [`crate::motion`]'s springs. Constants are the generated
//! [`crate::kit_gen::touch`] tables.
//!
//! Units are the control's own px, as in the TSX: the travel `T` is `w - 2*pad - knob` (lg 40,
//! sm 20); the drag band (26), the slop (3) and the stretch's full speed (1500 px/s, `Stretch`'s
//! default) are px whatever the size; only the wall's squash thresholds scale by `T / 40`
//! (`touch.tsx:206`). (Before this, the sm toggle ran in lg units: map.md C-097.)
//!
//! One state, one pure [`transition`]: pointer press, move and release, a value set from
//! outside, and `Tick(dt)`, the fixed-step frame. Time comes in as `dt` (s) and event times (ms);
//! nothing reads a clock.

use crate::kit_gen::touch::{TOGGLE, TOGGLE_SIZE};
use crate::kit_gen::touch_kit::SQUASH;
use crate::motion::{
    Spring, Stretch, Velocity, band_clamp, clamp, spring, squash_at, squash_of, step_k,
};

/// The design's two sizes (`TOGGLE_SIZE`): the specimen's and the settings row's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Lg,
    Sm,
}

impl Size {
    /// `(w, h, pad, knob)` px.
    pub fn geometry(self) -> (f64, f64, f64, f64) {
        match self {
            Size::Lg => (
                TOGGLE_SIZE::lg::w,
                TOGGLE_SIZE::lg::h,
                TOGGLE_SIZE::lg::pad,
                TOGGLE_SIZE::lg::knob,
            ),
            Size::Sm => (
                TOGGLE_SIZE::sm::w,
                TOGGLE_SIZE::sm::h,
                TOGGLE_SIZE::sm::pad,
                TOGGLE_SIZE::sm::knob,
            ),
        }
    }

    /// `T = w - 2*pad - knob`, px.
    pub fn travel(self) -> f64 {
        let (w, _, pad, knob) = self.geometry();
        w - 2.0 * pad - knob
    }
}

/// Where the pointer is with the toggle (the TSX's `held` and `drag` flags, as one value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Rest,
    /// Pressed, not yet moved past the slop: a release is a tap.
    Held,
    /// Pressed and dragged: the knob follows 1:1 and a release is flung.
    Drag,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Pointer down at `px` along the control, at `t_ms`.
    Press {
        px: f64,
        t_ms: f64,
    },
    Move {
        px: f64,
        t_ms: f64,
    },
    Release,
    /// The value from outside (the setting, the keyboard): ignored while held, as the TSX.
    Set(bool),
    /// One frame of `dt` seconds.
    Tick(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// A release landed on the other side: tell the owner.
    Report(bool),
}

/// The wall squash (`s.q`): 0..1 of the squash token (1 at rest), its strength, the wall's side.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Squash {
    t: f64,
    amp: f64,
    side: f64,
}

#[derive(Clone, Debug)]
pub struct State {
    travel: f64,
    on: bool,
    phase: Phase,
    x: Spring,
    st: Stretch,
    hs: Spring,
    x0: f64,
    p0: f64,
    raw: f64,
    vel: Velocity,
    q: Squash,
    /// Whether the last tick moved anything (the loop's return value).
    moving: bool,
}

impl State {
    pub fn new(on: bool, size: Size) -> Self {
        Self::with_travel(on, size.travel())
    }

    pub fn with_travel(on: bool, travel: f64) -> Self {
        let x = if on { travel } else { 0.0 };
        // the TSX loop runs once on mount, so its stretch has seen where the knob rests
        let mut st = Stretch::new();
        st.seed(x);
        Self {
            travel,
            on,
            phase: Phase::Rest,
            x: spring(x),
            st,
            hs: spring(1.0),
            x0: 0.0,
            p0: 0.0,
            raw: 0.0,
            vel: Velocity::new(),
            q: Squash {
                t: 1.0,
                amp: 1.0,
                side: 1.0,
            },
            moving: false,
        }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn travel(&self) -> f64 {
        self.travel
    }

    pub fn moving(&self) -> bool {
        self.moving
    }

    /// The TSX `paint`: the knob's two transforms and the fill.
    pub fn view(&self) -> View {
        let k = self.st.x().min(1.35) * self.hs.x;
        let (qx, qy) = squash_at(self.q.t, self.q.amp);
        let held = self.phase != Phase::Rest;
        View {
            x: self.x.x,
            scale_x: k,
            scale_y: squash_of(k),
            // held: widen away from the travel; moving: from the trailing side
            origin: if held {
                if self.on { 1.0 } else { 0.0 }
            } else if self.st.dir > 0.0 {
                0.0
            } else {
                1.0
            },
            squash_x: qx,
            squash_y: qy,
            // squash against the wall it hit
            squash_origin: if self.q.side > 0.0 { 1.0 } else { 0.0 },
            fill: clamp(self.x.x / self.travel, 0.0, 1.0).powf(0.8),
            on: self.on,
            held,
        }
    }
}

/// What the toggle draws: `translateX(x) [scale(k) about origin] [scale(q) about
/// squash_origin]`, the TSX's chain; origins as a share of the knob (0 left edge, 1 right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Knob offset, px: 0 (off) ..= T (on), banded past both while dragged.
    pub x: f64,
    /// Stretch times hold, and its area-keeping cross scale.
    pub scale_x: f64,
    pub scale_y: f64,
    pub origin: f64,
    /// The wall squash.
    pub squash_x: f64,
    pub squash_y: f64,
    pub squash_origin: f64,
    /// How far the accent has bloomed out of the knob, 0..=1.
    pub fill: f64,
    pub on: bool,
    pub held: bool,
}

/// One step of the toggle. Pure: the state in, the next state and what to tell the owner out.
pub fn transition(mut s: State, ev: Event) -> (State, Vec<Effect>) {
    let mut fx = Vec::new();
    match ev {
        Event::Press { px, t_ms } => {
            s.phase = Phase::Held;
            s.x0 = px;
            s.raw = px;
            s.p0 = s.x.x;
            s.vel.reset();
            s.vel.push(px, 0.0, t_ms);
        }
        Event::Move { px, t_ms } => {
            if s.phase != Phase::Rest {
                s.raw = px;
                s.vel.push(px, 0.0, t_ms);
                if s.phase == Phase::Held && (s.raw - s.x0).abs() > TOGGLE::slop {
                    s.phase = Phase::Drag;
                }
            }
        }
        Event::Release => {
            let landed = match s.phase {
                Phase::Rest => None,
                Phase::Held => Some(!s.on),
                Phase::Drag => Some(s.x.x + s.vel.get().0 * TOGGLE::fling > s.travel / 2.0),
            };
            if let Some(v) = landed {
                s.phase = Phase::Rest;
                if v != s.on {
                    fx.push(Effect::Report(v));
                }
                s.on = v;
            }
        }
        Event::Set(v) => {
            if s.phase == Phase::Rest {
                s.on = v;
            }
        }
        Event::Tick(dt) => s = tick(s, dt),
    }
    (s, fx)
}

/// The TSX loop body (`touch.tsx:197-216`).
fn tick(mut s: State, dt: f64) -> State {
    let t = s.travel;
    let mut moving = false;
    if s.phase == Phase::Drag {
        s.x = spring(band_clamp(s.p0 + s.raw - s.x0, 0.0, t, TOGGLE::band));
        moving = true;
    } else {
        let tgt = if s.on { t } else { 0.0 };
        moving |= step_k(&mut s.x, tgt, TOGGLE::follow, dt);
        let into = if s.on {
            s.x.x > t && s.x.v > 0.0
        } else {
            s.x.x < 0.0 && s.x.v < 0.0
        };
        if into {
            // the wall takes the energy
            let speed = s.x.v.abs();
            s.x = spring(tgt);
            if speed > TOGGLE::landing * t / 40.0 {
                s.q = Squash {
                    t: 0.0,
                    amp: clamp(speed / (TOGGLE::fullSquash * t / 40.0), 0.35, 1.0),
                    side: if s.on { 1.0 } else { -1.0 },
                };
            }
        }
    }
    if s.q.t < 1.0 {
        s.q.t += dt * 1000.0 / SQUASH::ms;
        moving = true;
    }
    let hold = if s.phase != Phase::Rest {
        TOGGLE::holdScale
    } else {
        1.0
    };
    moving |= step_k(&mut s.hs, hold, TOGGLE::hold, dt);
    // `stretchOf`: under reduced motion the stretch is 1 and does not count as moving
    moving |= s.st.update(s.x.x, dt);
    s.moving = moving;
    s
}

/// The toggle as the frame driver holds it: [`State`] behind the calls the registry makes.
#[derive(Clone, Debug)]
pub struct Toggle {
    s: State,
}

impl Toggle {
    pub fn new(on: bool, size: Size) -> Self {
        Self {
            s: State::new(on, size),
        }
    }

    fn apply(&mut self, ev: Event) -> Vec<Effect> {
        let (s, fx) = transition(self.s.clone(), ev);
        self.s = s;
        fx
    }

    /// The control was laid out with travel `px` (its size): an idle knob jumps to its side.
    pub fn resize(&mut self, travel: f64) {
        if travel > 0.0 && (travel - self.s.travel).abs() > f64::EPSILON {
            let on = self.s.on;
            let phase = self.s.phase;
            self.s.travel = travel;
            if phase == Phase::Rest {
                let x = if on { travel } else { 0.0 };
                self.s.x = spring(x);
                self.s.st.seed(x);
            }
        }
    }

    pub fn is_on(&self) -> bool {
        self.s.on
    }

    pub fn set(&mut self, on: bool) {
        self.apply(Event::Set(on));
    }

    pub fn press(&mut self, px: f64, t_ms: f64) {
        self.apply(Event::Press { px, t_ms });
    }

    pub fn drag(&mut self, px: f64, t_ms: f64) {
        self.apply(Event::Move { px, t_ms });
    }

    /// The side it lands on, if it was held.
    pub fn release(&mut self) -> Option<bool> {
        let held = self.s.phase != Phase::Rest;
        self.apply(Event::Release);
        held.then_some(self.s.on)
    }

    pub fn step(&mut self, dt: f64) -> bool {
        self.apply(Event::Tick(dt));
        self.s.moving
    }

    pub fn view(&self) -> View {
        self.s.view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    fn run(mut s: State, ev: Event) -> (State, Vec<Effect>) {
        let (n, fx) = transition(s.clone(), ev);
        s = n;
        (s, fx)
    }

    fn settle(mut s: State) -> (State, usize) {
        let mut n = 0;
        loop {
            s = run(s, Event::Tick(DT)).0;
            n += 1;
            if !s.moving() {
                return (s, n);
            }
            assert!(n < 20_000, "never settled");
        }
    }

    #[test]
    fn travel_is_the_design_geometry() {
        assert_eq!(Size::Lg.travel(), 40.0);
        assert_eq!(Size::Sm.travel(), 20.0);
    }

    #[test]
    fn a_tap_flips_reports_and_springs_to_the_far_wall() {
        for size in [Size::Lg, Size::Sm] {
            let s = State::new(false, size);
            let (s, fx) = run(s, Event::Press { px: 5.0, t_ms: 0.0 });
            assert!(fx.is_empty());
            assert_eq!(s.phase(), Phase::Held);
            let (s, fx) = run(s, Event::Release);
            assert_eq!(fx, [Effect::Report(true)]);
            let mut peak: f64 = 1.0;
            let mut s = s;
            for _ in 0..2000 {
                s = run(s, Event::Tick(DT)).0;
                peak = peak.max(s.view().squash_x);
                if !s.moving() {
                    break;
                }
            }
            assert!(peak > 1.05, "the wall squashed it: {peak}");
            let v = s.view();
            assert_eq!((v.x, v.fill, v.on), (size.travel(), 1.0, true));
            assert!((v.scale_x - 1.0).abs() < 1e-3 && (v.squash_x - 1.0).abs() < 1e-9);
            assert_eq!(v.squash_origin, 1.0, "squashed against the right wall");
        }
    }

    #[test]
    fn a_release_without_a_press_does_nothing() {
        let s = State::new(true, Size::Sm);
        let (s, fx) = run(s, Event::Release);
        assert!(fx.is_empty());
        assert!(s.on());
    }

    /// The same pointer trace in lg and sm: px thresholds (slop, band, stretch) are the same px,
    /// the wall thresholds scale with T.
    #[test]
    fn sizes_share_real_px_thresholds() {
        // 2 px: under the slop for both sizes (it was 1.5 lg units, so 3 px on sm before)
        for size in [Size::Lg, Size::Sm] {
            let s = State::new(false, size);
            let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
            let (s, _) = run(
                s,
                Event::Move {
                    px: 2.9,
                    t_ms: 50.0,
                },
            );
            assert_eq!(s.phase(), Phase::Held, "{size:?}");
            let (s, _) = run(
                s,
                Event::Move {
                    px: 3.1,
                    t_ms: 60.0,
                },
            );
            assert_eq!(s.phase(), Phase::Drag, "{size:?}");
        }
        // dragged 100 px past the far end: banded by the same 26 px band
        let over = |size: Size| {
            let t = size.travel();
            let s = State::new(false, size);
            let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
            let (s, _) = run(
                s,
                Event::Move {
                    px: t + 100.0,
                    t_ms: 1000.0,
                },
            );
            let (s, _) = run(s, Event::Tick(DT));
            s.view().x - t
        };
        let (lg, sm) = (over(Size::Lg), over(Size::Sm));
        assert!(
            (lg - sm).abs() < 1e-9 && lg > 20.0 && lg < TOGGLE::band,
            "{lg} {sm}"
        );
        // the wall squash: a tap's landing speed is proportional to T, and so are the
        // thresholds, so both sizes squash with the same strength
        let amp = |size: Size| {
            let s = State::new(false, size);
            let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
            let (mut s, _) = run(s, Event::Release);
            for _ in 0..2000 {
                s = run(s, Event::Tick(DT)).0;
                if s.q.t < 1.0 {
                    return s.q.amp;
                }
            }
            panic!("no squash");
        };
        assert!((amp(Size::Lg) - amp(Size::Sm)).abs() < 1e-9);
        // the knob's path scales with T: half the travel, half the x, frame by frame
        let trace = |size: Size| {
            let s = State::new(false, size);
            let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
            let (mut s, _) = run(s, Event::Release);
            (0..60)
                .map(|_| {
                    s = run(s.clone(), Event::Tick(DT)).0;
                    s.view().x
                })
                .collect::<Vec<_>>()
        };
        for (a, b) in trace(Size::Lg).iter().zip(trace(Size::Sm)) {
            assert!((a / 2.0 - b).abs() < 1e-9, "{a} {b}");
        }
    }

    #[test]
    fn a_fling_decides_the_side() {
        let s = State::new(false, Size::Lg);
        let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
        let (s, _) = run(
            s,
            Event::Move {
                px: 8.0,
                t_ms: 10.0,
            },
        );
        let (s, _) = run(
            s,
            Event::Move {
                px: 16.0,
                t_ms: 20.0,
            },
        );
        let (s, _) = run(s, Event::Tick(DT));
        assert!(s.view().x < 20.0);
        let (s, fx) = run(s, Event::Release);
        assert_eq!(fx, [Effect::Report(true)]);
        assert!(s.on());
        // slow and short: stays on, nothing to report
        let s = State::new(true, Size::Lg);
        let (s, _) = run(
            s,
            Event::Press {
                px: 40.0,
                t_ms: 0.0,
            },
        );
        let (s, _) = run(
            s,
            Event::Move {
                px: 36.0,
                t_ms: 100.0,
            },
        );
        let (s, _) = run(
            s,
            Event::Move {
                px: 35.0,
                t_ms: 200.0,
            },
        );
        let (s, fx) = run(s, Event::Release);
        assert!(fx.is_empty() && s.on());
    }

    #[test]
    fn a_set_from_outside_springs_there_with_the_wall_squash() {
        let s = State::new(true, Size::Sm);
        let (s, fx) = run(s, Event::Set(false));
        assert!(fx.is_empty(), "a set is not reported back");
        let mut s = s;
        let mut squashed = false;
        for _ in 0..2000 {
            s = run(s, Event::Tick(DT)).0;
            squashed |= s.q.t < 1.0;
            if !s.moving() {
                break;
            }
        }
        assert!(squashed);
        assert_eq!(s.view().squash_origin, 0.0, "against the left wall");
        assert_eq!((s.view().x, s.view().fill), (0.0, 0.0));
        // while held, a set is ignored
        let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
        let (s, _) = run(s, Event::Set(true));
        assert!(!s.on());
    }

    #[test]
    fn holding_widens_away_from_the_travel() {
        let s = State::new(false, Size::Lg);
        let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
        let (s, _) = settle(s);
        let v = s.view();
        assert!((v.scale_x - TOGGLE::holdScale).abs() < 1e-3);
        assert_eq!(
            v.origin, 0.0,
            "off: widens to the right, from the left edge"
        );
        assert!(v.held);
    }

    #[test]
    fn reduced_motion_lands_at_once() {
        crate::motion::set_reduced(true);
        let s = State::new(false, Size::Lg);
        let (s, _) = run(s, Event::Press { px: 0.0, t_ms: 0.0 });
        let (s, _) = run(s, Event::Release);
        let (s, _) = run(s, Event::Tick(DT));
        crate::motion::set_reduced(false);
        let v = s.view();
        assert_eq!(
            (v.x, v.scale_x, v.squash_x, v.squash_y),
            (40.0, 1.0, 1.0, 1.0)
        );
    }

    #[test]
    fn the_adapter_reports_the_landed_side() {
        let mut t = Toggle::new(false, Size::Sm);
        t.press(5.0, 0.0);
        assert_eq!(t.release(), Some(true));
        assert_eq!(t.release(), None, "released once");
        while t.step(DT) {}
        assert_eq!(t.view().x, 20.0);
        t.resize(40.0);
        assert_eq!(t.view().x, 40.0, "an idle knob jumps to its side");
    }
}
