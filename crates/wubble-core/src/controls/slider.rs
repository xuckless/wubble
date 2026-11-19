//! The elastic slider (`ElasticSlider`, `pages/touch.tsx:27-140` @598f814, the owner's design
//! repo): held, the knob sits under the finger 1:1 (keeping the grab offset) and rubber-bands past
//! both ends; a press away from the knob sends it there on a stiff catch-up spring, then it
//! follows 1:1. Released or keyed, the follow spring. The value lives in user units
//! (`min..=max` snapped to `step`, as the TSX props); while held every new value is reported, the
//! knob itself never snaps while held, only on release.
//!
//! The grip is one enum and one pure [`transition`]; [`Slider`] adds the springs the TSX loop
//! steps (`useLoop`), with `dt` passed in.

use crate::kit_gen::touch::SLIDER;
use crate::motion::{Spring, Stretch, band_clamp, clamp, spring, squash_of, step_k};

/// The value domain: the TSX's `min`, `max` and `step` props.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cfg {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl Default for Cfg {
    /// The TSX defaults: `min = 0, max = 100, step = 1`.
    fn default() -> Self {
        Self {
            min: 0.0,
            max: 100.0,
            step: 1.0,
        }
    }
}

impl Cfg {
    /// A domain from the caller's props; an empty range or a step of 0 falls back to sane ones.
    pub fn new(min: f64, max: f64, step: f64) -> Self {
        let max = if max > min { max } else { min + 1.0 };
        let step = if step > 0.0 { step } else { 1.0 };
        Self { min, max, step }
    }

    /// `toU`: a value as 0..=100 along the travel.
    pub fn to_u(&self, v: f64) -> f64 {
        (v - self.min) / (self.max - self.min) * 100.0
    }

    /// `fromU`: a place along the travel (0..=100) as the snapped value.
    pub fn from_u(&self, u: f64) -> f64 {
        let span = self.max - self.min;
        clamp(
            self.min + (u / 100.0 * span / self.step).round() * self.step,
            self.min,
            self.max,
        )
    }

    /// `fromU(toU(v))`: `v` snapped to the step and clamped.
    pub fn snap(&self, v: f64) -> f64 {
        self.from_u(self.to_u(v))
    }
}

/// The keys the slider takes (`touch.tsx:99-101`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

impl Key {
    /// The registry's key codes (Slint passes an int): 0..=7 in declaration order.
    pub fn from_code(code: i32) -> Option<Self> {
        [
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::PageUp,
            Key::PageDown,
            Key::Home,
            Key::End,
        ]
        .get(usize::try_from(code).ok()?)
        .copied()
    }
    /// ArrowLeft -1, ArrowRight +1: the direction the segmented control and the swatches step
    /// (they take no other key).
    pub fn dir(self) -> Option<i32> {
        match self {
            Key::Left => Some(-1),
            Key::Right => Some(1),
            _ => None,
        }
    }
}

/// The value a key asks for, before the snap: arrows one step (Shift: ten), Page a tenth of the
/// range, Home and End the ends.
pub fn key_value(key: Key, shift: bool, value: f64, c: &Cfg) -> f64 {
    let st = if shift { c.step * 10.0 } else { c.step };
    let pg = (c.max - c.min) / 10.0;
    match key {
        Key::Left | Key::Down => value - st,
        Key::Right | Key::Up => value + st,
        Key::PageDown => value - pg,
        Key::PageUp => value + pg,
        Key::Home => c.min,
        Key::End => c.max,
    }
}

/// Who holds the knob.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    Free,
    /// A press away from the knob: it runs to the finger (`raw`) on the catch-up spring.
    Catching {
        raw: f64,
    },
    /// It sits under the finger, `off` px from where it was grabbed.
    Held {
        raw: f64,
        off: f64,
    },
}

impl Grip {
    pub fn held(self) -> bool {
        self != Grip::Free
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Pointer down at `px` along the track, the knob's centre at `knob_x`.
    Press {
        px: f64,
        knob_x: f64,
    },
    Drag {
        px: f64,
    },
    Release,
    /// The catch-up spring reached the finger.
    Caught,
    /// A frame while held: the knob's centre and the track's width.
    Frame {
        knob_x: f64,
        w: f64,
    },
    Key {
        key: Key,
        shift: bool,
    },
    /// A value from outside (a reset): springs there unless held, never reported back.
    Set(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// `onChange`: a new value for the setting.
    Report(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub grip: Grip,
    /// Where the follow spring takes the knob at rest, 0..=100 of the travel.
    pub target: f64,
    /// The value shown (and last reported), snapped.
    pub shown: f64,
}

impl State {
    pub fn new(value: f64, c: &Cfg) -> Self {
        let v = c.snap(value);
        Self {
            grip: Grip::Free,
            target: c.to_u(v),
            shown: v,
        }
    }
}

/// The TSX's `go(v, tell)`: snap, retarget, show, and report when told.
fn go(s: State, v: f64, tell: bool, c: &Cfg) -> (State, Vec<Effect>) {
    let v = c.snap(v);
    let next = State {
        target: c.to_u(v),
        shown: v,
        ..s
    };
    (
        next,
        if tell {
            vec![Effect::Report(v)]
        } else {
            vec![]
        },
    )
}

/// The slider's one transition. Pure: the springs are [`Slider`]'s.
pub fn transition(s: State, e: Event, c: &Cfg) -> (State, Vec<Effect>) {
    let none = |s| (s, vec![]);
    match (s.grip, e) {
        (_, Event::Press { px, knob_x }) => {
            let near = (px - knob_x).abs() <= SLIDER::grab;
            let grip = if near {
                Grip::Held {
                    raw: px,
                    off: px - knob_x,
                }
            } else {
                Grip::Catching { raw: px }
            };
            none(State { grip, ..s })
        }
        (Grip::Catching { .. }, Event::Drag { px }) => none(State {
            grip: Grip::Catching { raw: px },
            ..s
        }),
        (Grip::Held { off, .. }, Event::Drag { px }) => none(State {
            grip: Grip::Held { raw: px, off },
            ..s
        }),
        (Grip::Catching { raw }, Event::Caught) => none(State {
            grip: Grip::Held { raw, off: 0.0 },
            ..s
        }),
        (Grip::Catching { .. } | Grip::Held { .. }, Event::Frame { knob_x, w }) => {
            let v = if w > 0.0 {
                c.from_u(clamp(knob_x, 0.0, w) / w * 100.0)
            } else {
                c.min
            };
            if v == s.shown {
                none(s)
            } else {
                (State { shown: v, ..s }, vec![Effect::Report(v)])
            }
        }
        (Grip::Catching { .. } | Grip::Held { .. }, Event::Release) => none(State {
            grip: Grip::Free,
            target: c.to_u(s.shown),
            ..s
        }),
        (_, Event::Key { key, shift }) => go(s, key_value(key, shift, s.shown, c), true, c),
        (Grip::Free, Event::Set(v)) if v != s.shown => go(s, v, false, c),
        _ => none(s),
    }
}

/// What the slider draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderView {
    /// Knob centre, px from the track's left (rubber-banded past both ends while held).
    pub x: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Transform origin along the travel: 0.7 moving right, 0.3 moving left.
    pub origin: f64,
    /// The value bubble's centre, px from the track's left.
    pub bubble_x: f64,
    /// Bubble lean in degrees.
    pub lean: f64,
    /// The value shown, in the domain's units.
    pub value: f64,
    pub held: bool,
}

/// The slider with its springs: knob `x`, its stretch, the hold swell and the bubble.
#[derive(Clone, Debug)]
pub struct Slider {
    s: State,
    cfg: Cfg,
    x: Spring,
    st: Stretch,
    hs: Spring,
    bx: Spring,
    /// The track's width, px.
    w: f64,
}

impl Slider {
    pub fn new(value: f64, width: f64, cfg: Cfg) -> Self {
        let s = State::new(value, &cfg);
        let x = s.target / 100.0 * width;
        Self {
            s,
            cfg,
            x: spring(x),
            st: Stretch::new(),
            hs: spring(1.0),
            bx: spring(x),
            w: width,
        }
    }

    pub fn state(&self) -> State {
        self.s
    }

    fn apply(&mut self, e: Event) -> Vec<Effect> {
        let (s, fx) = transition(self.s, e, &self.cfg);
        self.s = s;
        fx
    }

    /// A new domain (the props changed): the value is snapped into it, the knob springs there.
    pub fn configure(&mut self, cfg: Cfg) {
        if cfg != self.cfg {
            self.cfg = cfg;
            self.s.shown = cfg.snap(self.s.shown);
            self.s.target = cfg.to_u(self.s.shown);
        }
    }

    /// The track was laid out at `width`: an idle knob jumps to its value there.
    pub fn set_width(&mut self, width: f64) {
        self.w = width;
        if !self.s.grip.held() {
            self.x = spring(self.s.target / 100.0 * width);
            self.bx = self.x;
        }
    }

    /// A value from outside (the setting, a reset): the follow spring carries the knob.
    pub fn set(&mut self, value: f64) {
        self.apply(Event::Set(value));
    }

    pub fn press(&mut self, px: f64) {
        self.apply(Event::Press {
            px,
            knob_x: self.x.x,
        });
    }

    pub fn drag(&mut self, px: f64) {
        self.apply(Event::Drag { px });
    }

    /// The value the slider lands on, if it was held.
    pub fn release(&mut self) -> Option<f64> {
        let held = self.s.grip.held();
        self.apply(Event::Release);
        held.then_some(self.s.shown)
    }

    /// A key: the value it reports.
    pub fn key(&mut self, key: Key, shift: bool) -> Option<f64> {
        self.apply(Event::Key { key, shift })
            .into_iter()
            .map(|Effect::Report(v)| v)
            .next_back()
    }

    /// One frame of the TSX loop. Returns whether anything still moves.
    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        match self.s.grip {
            Grip::Held { raw, off } => {
                self.x = spring(band_clamp(raw - off, 0.0, self.w, SLIDER::band));
                moving = true;
            }
            Grip::Catching { raw } => {
                let want = band_clamp(raw, 0.0, self.w, SLIDER::band);
                step_k(&mut self.x, want, SLIDER::catchUp, dt);
                if (self.x.x - want).abs() < 0.5 {
                    self.apply(Event::Caught);
                }
                moving = true;
            }
            Grip::Free => {
                moving |= step_k(
                    &mut self.x,
                    self.s.target / 100.0 * self.w,
                    SLIDER::follow,
                    dt,
                );
            }
        }
        if self.s.grip.held() {
            self.apply(Event::Frame {
                knob_x: self.x.x,
                w: self.w,
            });
        }
        let hold = if self.s.grip.held() {
            SLIDER::holdScale
        } else {
            1.0
        };
        moving |= step_k(&mut self.hs, hold, SLIDER::hold, dt);
        moving |= step_k(
            &mut self.bx,
            clamp(self.x.x, 0.0, self.w),
            SLIDER::bubble,
            dt,
        );
        moving |= self.st.update(self.x.x, dt);
        moving
    }

    pub fn view(&self) -> SliderView {
        let sx = self.st.x().min(1.0 + SLIDER::stretchMax);
        let k = self.hs.x;
        SliderView {
            x: self.x.x,
            scale_x: sx * k,
            scale_y: squash_of(sx) * k,
            origin: 1.0 - self.st.origin(),
            bubble_x: self.bx.x,
            lean: clamp(
                (self.bx.x - clamp(self.x.x, 0.0, self.w)) * SLIDER::lean,
                -SLIDER::leanMax,
                SLIDER::leanMax,
            ),
            value: self.s.shown,
            held: self.s.grip.held(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    fn settle(mut f: impl FnMut() -> bool) {
        let mut n = 0;
        while f() {
            n += 1;
            assert!(n < 20_000, "never settled");
        }
    }

    #[test]
    fn the_domain_snaps_to_its_step() {
        let zoom = Cfg::new(50.0, 200.0, 10.0);
        assert_eq!(zoom.snap(64.0), 60.0);
        assert_eq!(zoom.snap(66.0), 70.0);
        assert_eq!(zoom.snap(10.0), 50.0);
        assert_eq!(zoom.snap(999.0), 200.0);
        // a step that does not divide the range: the top is clamped, not overshot
        let odd = Cfg::new(0.0, 10.0, 3.0);
        assert_eq!(odd.snap(10.0), 9.0);
        assert_eq!(odd.from_u(100.0), 9.0);
        for v in [50.0, 60.0, 130.0, 200.0] {
            assert_eq!(zoom.from_u(zoom.to_u(v)), v, "round trip {v}");
        }
        // degenerate props fall back
        let d = Cfg::new(5.0, 5.0, 0.0);
        assert_eq!((d.max, d.step), (6.0, 1.0));
    }

    #[test]
    fn every_key_and_shift() {
        let c = Cfg::new(0.0, 100.0, 2.0);
        let v = 40.0;
        let cases = [
            (Key::Left, false, 38.0),
            (Key::Down, false, 38.0),
            (Key::Right, false, 42.0),
            (Key::Up, false, 42.0),
            (Key::Left, true, 20.0),
            (Key::Right, true, 60.0),
            (Key::PageDown, false, 30.0),
            (Key::PageUp, false, 50.0),
            (Key::PageUp, true, 50.0),
            (Key::Home, false, 0.0),
            (Key::End, true, 100.0),
        ];
        for (k, shift, want) in cases {
            assert_eq!(key_value(k, shift, v, &c), want, "{k:?} shift {shift}");
        }
        assert_eq!(Key::from_code(7), Some(Key::End));
        assert_eq!(Key::from_code(8), None);
        assert_eq!(Key::from_code(-1), None);
        assert_eq!(
            (Key::Left.dir(), Key::Right.dir(), Key::Home.dir()),
            (Some(-1), Some(1), None)
        );
    }

    #[test]
    fn the_grip_transitions() {
        let c = Cfg::default();
        let s = State::new(50.0, &c);
        // near the knob: held with its offset
        let (h, fx) = transition(
            s,
            Event::Press {
                px: 105.0,
                knob_x: 100.0,
            },
            &c,
        );
        assert_eq!(
            h.grip,
            Grip::Held {
                raw: 105.0,
                off: 5.0
            }
        );
        assert!(fx.is_empty());
        // far: catching up
        let (k, _) = transition(
            s,
            Event::Press {
                px: 180.0,
                knob_x: 100.0,
            },
            &c,
        );
        assert_eq!(k.grip, Grip::Catching { raw: 180.0 });
        let (k2, _) = transition(k, Event::Caught, &c);
        assert_eq!(
            k2.grip,
            Grip::Held {
                raw: 180.0,
                off: 0.0
            }
        );
        // a frame reports only a changed value
        let f = Event::Frame {
            knob_x: 100.0,
            w: 200.0,
        };
        assert!(transition(h, f, &c).1.is_empty());
        let (h2, fx) = transition(
            h,
            Event::Frame {
                knob_x: 150.0,
                w: 200.0,
            },
            &c,
        );
        assert_eq!(fx, vec![Effect::Report(75.0)]);
        // release keeps the shown value and retargets there, reporting nothing new
        let (r, fx) = transition(h2, Event::Release, &c);
        assert_eq!((r.grip, r.target, r.shown), (Grip::Free, 75.0, 75.0));
        assert!(fx.is_empty());
        // a value from outside while held is ignored; at rest it springs there, unreported
        assert_eq!(transition(h2, Event::Set(10.0), &c).0, h2);
        let (set, fx) = transition(r, Event::Set(10.0), &c);
        assert_eq!((set.shown, set.target), (10.0, 10.0));
        assert!(fx.is_empty());
        // a key reports
        let (key, fx) = transition(
            r,
            Event::Key {
                key: Key::End,
                shift: false,
            },
            &c,
        );
        assert_eq!((key.shown, fx), (100.0, vec![Effect::Report(100.0)]));
        // releasing a free slider changes nothing
        assert_eq!(transition(r, Event::Release, &c).0, r);
    }

    #[test]
    fn a_track_press_catches_up_then_follows() {
        let mut s = Slider::new(0.0, 200.0, Cfg::default());
        s.press(150.0);
        s.step(DT);
        let x = s.view().x;
        assert!(x > 0.0 && x < 150.0, "{x}");
        let mut n = 0;
        while matches!(s.state().grip, Grip::Catching { .. }) {
            s.step(DT);
            n += 1;
            assert!(n < 1000);
        }
        assert!((s.view().x - 150.0).abs() < 0.5);
        s.step(DT);
        assert_eq!(s.view().x, 150.0);
        assert_eq!(s.release(), Some(75.0));
        assert_eq!(s.release(), None, "released once");
    }

    #[test]
    fn the_band_past_both_ends_and_the_snap_on_release() {
        let c = Cfg::new(50.0, 200.0, 10.0);
        let mut s = Slider::new(100.0, 300.0, c);
        let x0 = s.view().x;
        assert!((x0 - 100.0).abs() < 1e-9);
        s.press(x0 + 4.0);
        s.drag(700.0);
        s.step(DT);
        let v = s.view();
        assert_eq!(v.x, band_clamp(700.0 - 4.0, 0.0, 300.0, SLIDER::band));
        assert_eq!(v.value, 200.0);
        s.drag(-400.0);
        s.step(DT);
        assert_eq!(
            s.view().x,
            band_clamp(-400.0 - 4.0, 0.0, 300.0, SLIDER::band)
        );
        assert_eq!(s.view().value, 50.0);
        // let go between two steps: it lands on the snapped value's place
        s.drag(64.0 + 4.0);
        s.step(DT);
        assert_eq!(s.view().value, 80.0, "64 px of 300 is 82 → 80");
        assert_eq!(s.release(), Some(80.0));
        settle(|| s.step(DT));
        assert_eq!(s.view().x, c.to_u(80.0) / 100.0 * 300.0);
        assert!((s.view().scale_y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn keys_spring_and_the_bubble_leans_behind() {
        let mut s = Slider::new(0.0, 200.0, Cfg::default());
        assert_eq!(s.key(Key::End, false), Some(100.0));
        s.step(DT);
        s.step(DT);
        let v = s.view();
        assert!(v.x > 0.0 && v.x < 200.0);
        assert!(v.bubble_x < v.x, "the bubble trails");
        assert!(v.lean < 0.0 && v.lean >= -SLIDER::leanMax);
        settle(|| s.step(DT));
        assert_eq!(s.view().x, 200.0);
    }
}
