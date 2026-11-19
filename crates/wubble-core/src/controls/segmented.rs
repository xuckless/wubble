//! The segmented control with a liquid indicator (`Segmented`, `pages/touch-kit.tsx:185-305`
//! @598f814, the owner's design repo): the indicator follows on the follow spring and stretches
//! along its travel from the trailing side; grabbed, it follows 1:1 and rubber-bands; released,
//! it picks the nearest segment along the fling. A press elsewhere selects the segment under the
//! pointer on release. ArrowLeft/Right choose the neighbour. The segments' places come from the
//! layout ([`Segmented::set_segment`]).
//!
//! The grip is one enum and one pure [`transition`]; [`Segmented`] adds the springs.

use crate::kit_gen::controls_b::SEG_SLOP;
use crate::kit_gen::touch_kit::SEG;
use crate::motion::{Spring, Stretch, Velocity, band_clamp, spring, squash_of, step_k};

/// What the pointer holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    Idle,
    /// Down, not yet moved past the slop: `off` is where on the indicator it was grabbed, `None`
    /// when the press missed it (then it only selects on release).
    Pressed {
        off: Option<f64>,
    },
    /// The indicator follows the pointer, `off` px from its left.
    Dragging {
        off: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Down at `px`; the indicator spans `left..left + width`.
    Press {
        px: f64,
        left: f64,
        width: f64,
    },
    Move {
        px: f64,
    },
    Release,
}

/// How a release chooses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// After a drag: the segment nearest the flung indicator.
    Fling,
    /// After a press: the segment under `x`.
    Pick { x: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    pub grip: Grip,
    /// Where the press began.
    pub x0: f64,
    /// The pointer now.
    pub raw: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            grip: Grip::Idle,
            x0: 0.0,
            raw: 0.0,
        }
    }
}

/// The pointer's one transition. Pure.
pub fn transition(s: State, e: Event) -> (State, Option<Effect>) {
    match (s.grip, e) {
        (_, Event::Press { px, left, width }) => {
            let off = (px >= left && px <= left + width).then_some(px - left);
            (
                State {
                    grip: Grip::Pressed { off },
                    x0: px,
                    raw: px,
                },
                None,
            )
        }
        (Grip::Pressed { off: Some(off) }, Event::Move { px }) if (px - s.x0).abs() > SEG_SLOP => (
            State {
                grip: Grip::Dragging { off },
                raw: px,
                ..s
            },
            None,
        ),
        (Grip::Pressed { .. } | Grip::Dragging { .. }, Event::Move { px }) => {
            (State { raw: px, ..s }, None)
        }
        (Grip::Dragging { .. }, Event::Release) => (
            State {
                grip: Grip::Idle,
                ..s
            },
            Some(Effect::Fling),
        ),
        (Grip::Pressed { .. }, Event::Release) => (
            State {
                grip: Grip::Idle,
                ..s
            },
            Some(Effect::Pick { x: s.raw }),
        ),
        (Grip::Idle, _) => (s, None),
    }
}

/// ArrowLeft (`dir` < 0) or ArrowRight (> 0) from `selected` among `n`: the neighbour, clamped
/// at both ends (`touch-kit.tsx:297-304`; the settings swatches do the same).
pub fn key_select(selected: usize, n: usize, dir: i32) -> usize {
    if n == 0 {
        return 0;
    }
    let i = selected as i64 + i64::from(dir.signum());
    i.clamp(0, n as i64 - 1) as usize
}

/// What the segmented control draws: the indicator's rectangle (already scaled about its
/// trailing side, so Slint needs no transform) and how much of its height it keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentView {
    /// Left and width of the indicator, px from the control's left.
    pub left: f64,
    pub width: f64,
    /// Along-travel scale (with the press), for the corner radius.
    pub scale_x: f64,
    /// Cross-axis scale: the indicator is `scale_y` of the segment's height, centred.
    pub scale_y: f64,
    /// The segment chosen.
    pub selected: usize,
    pub held: bool,
}

#[derive(Clone, Debug)]
pub struct Segmented {
    x: Spring,
    w: Spring,
    st: Stretch,
    pr: Spring,
    /// (left, width) of each segment, px.
    segs: Vec<(f64, f64)>,
    selected: usize,
    s: State,
    vel: Velocity,
}

impl Segmented {
    pub fn new(selected: usize) -> Self {
        Self {
            x: spring(0.0),
            w: spring(0.0),
            st: Stretch::new(),
            pr: spring(1.0),
            segs: vec![],
            selected,
            s: State::default(),
            vel: Velocity::new(),
        }
    }

    pub fn state(&self) -> State {
        self.s
    }

    fn target(&self) -> (f64, f64) {
        self.segs
            .get(self.selected)
            .or(self.segs.last())
            .copied()
            .unwrap_or((0.0, 0.0))
    }

    fn ends(&self) -> (f64, f64) {
        let min = self.segs.first().map_or(0.0, |s| s.0);
        let max = self.segs.last().map_or(0.0, |s| s.0 + s.1);
        (min, max)
    }

    fn held(&self) -> bool {
        self.s.grip != Grip::Idle
    }

    /// Segment `i` was laid out at `left`, `width`: the indicator jumps to its segment there
    /// (a layout change is not a motion; the design re-measures and snaps).
    pub fn set_segment(&mut self, i: usize, left: f64, width: f64) {
        if self.segs.len() <= i {
            self.segs.resize(i + 1, (0.0, 0.0));
        }
        self.segs[i] = (left, width);
        if !self.held() {
            let (tx, tw) = self.target();
            self.x = spring(tx);
            self.w = spring(tw);
            self.st.forget();
        }
    }

    /// A choice from outside (the setting, the keyboard): the follow spring carries it there.
    pub fn set(&mut self, selected: usize) {
        self.selected = selected;
    }

    /// `px` from the control's left, `t_ms` the event's time.
    pub fn press(&mut self, px: f64, t_ms: f64) {
        self.s = transition(
            self.s,
            Event::Press {
                px,
                left: self.x.x,
                width: self.w.x,
            },
        )
        .0;
        self.vel.reset();
        self.vel.push(px, 0.0, t_ms);
    }

    pub fn drag(&mut self, px: f64, t_ms: f64) {
        if !self.held() {
            return;
        }
        self.s = transition(self.s, Event::Move { px }).0;
        self.vel.push(px, 0.0, t_ms);
    }

    /// The segment it lands on, if it was held: the nearest to the flung indicator after a
    /// drag, the one under the pointer after a press (none when it missed every segment). Only
    /// a candidate: the control is controlled (the TSX's `onChange`), so the indicator goes
    /// there when the caller's value comes back through [`Segmented::set`].
    pub fn release(&mut self) -> Option<usize> {
        let (s, fx) = transition(self.s, Event::Release);
        self.s = s;
        match fx? {
            Effect::Fling => {
                let centre = self.x.x + self.w.x / 2.0 + self.vel.get().0 * SEG::fling;
                self.segs
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        let d = |s: &(f64, f64)| (s.0 + s.1 / 2.0 - centre).abs();
                        d(a.1).total_cmp(&d(b.1))
                    })
                    .map(|(i, _)| i)
            }
            Effect::Pick { x } => self.segs.iter().position(|s| x >= s.0 && x <= s.0 + s.1),
        }
    }

    /// ArrowLeft/Right: the neighbour to ask for (a candidate, as [`Segmented::release`]).
    pub fn key(&self, dir: i32) -> usize {
        key_select(self.selected, self.segs.len(), dir)
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        let (tx, tw) = self.target();
        if let Grip::Dragging { off } = self.s.grip {
            let (min, max) = self.ends();
            self.x = spring(band_clamp(self.s.raw - off, min, max - self.w.x, SEG::band));
            moving = true;
        } else {
            moving |= step_k(&mut self.x, tx, SEG::follow, dt);
        }
        moving |= step_k(&mut self.w, tw, SEG::size, dt);
        let grabbed = matches!(
            self.s.grip,
            Grip::Pressed { off: Some(_) } | Grip::Dragging { .. }
        );
        let press = if grabbed { SEG::press } else { 1.0 };
        moving |= step_k(&mut self.pr, press, SEG::pressSpring, dt);
        moving |= self.st.update(self.x.x, dt);
        moving
    }

    pub fn view(&self) -> SegmentView {
        let (min, max) = self.ends();
        let w = self.w.x;
        let dragging = matches!(self.s.grip, Grip::Dragging { .. });
        // the follow spring's overshoot at an end segment presses into the wall
        let x = if dragging {
            self.x.x
        } else {
            band_clamp(self.x.x, min, max - w, SEG::wall)
        };
        let fwd = self.st.dir > 0.0;
        let o = if fwd { 0.0 } else { w };
        // the leading edge may reach, but never out of the track
        let room = if w > 0.0 {
            if fwd {
                (max - x) / w
            } else {
                (x + w - min) / w
            }
        } else {
            1.0
        };
        let sx = self.st.x().min(1.0 + SEG::stretchMax).min(room.max(1.0));
        let sy = squash_of(sx) * self.pr.x;
        let ax = sx * self.pr.x;
        let l = x + o - o * ax;
        let r = x + o + (w - o) * ax;
        SegmentView {
            left: l,
            width: r - l,
            scale_x: ax,
            scale_y: sy,
            selected: self.selected,
            held: self.held(),
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

    fn seg3() -> Segmented {
        let mut s = Segmented::new(0);
        for (i, x) in [4.0, 64.0, 124.0].into_iter().enumerate() {
            s.set_segment(i, x, 60.0);
        }
        s
    }

    #[test]
    fn the_grip_transitions() {
        let s = State::default();
        let on = Event::Press {
            px: 10.0,
            left: 4.0,
            width: 60.0,
        };
        let (p, fx) = transition(s, on);
        assert_eq!(p.grip, Grip::Pressed { off: Some(6.0) });
        assert_eq!(fx, None);
        // within the slop it stays a press
        let (p2, _) = transition(
            p,
            Event::Move {
                px: 10.0 + SEG_SLOP,
            },
        );
        assert_eq!(p2.grip, Grip::Pressed { off: Some(6.0) });
        let (d, _) = transition(p2, Event::Move { px: 20.0 });
        assert_eq!((d.grip, d.raw), (Grip::Dragging { off: 6.0 }, 20.0));
        assert_eq!(transition(d, Event::Release).1, Some(Effect::Fling));
        // a press off the indicator never drags; it picks where it is let go
        let (m, _) = transition(
            s,
            Event::Press {
                px: 100.0,
                left: 4.0,
                width: 60.0,
            },
        );
        let (m, _) = transition(m, Event::Move { px: 150.0 });
        assert_eq!(m.grip, Grip::Pressed { off: None });
        let (m, fx) = transition(m, Event::Release);
        assert_eq!((m.grip, fx), (Grip::Idle, Some(Effect::Pick { x: 150.0 })));
        // idle ignores moves and releases
        assert_eq!(transition(s, Event::Release), (s, None));
        assert_eq!(transition(s, Event::Move { px: 3.0 }), (s, None));
    }

    #[test]
    fn keys_choose_the_neighbour_and_clamp() {
        assert_eq!(key_select(0, 3, -1), 0);
        assert_eq!(key_select(0, 3, 1), 1);
        assert_eq!(key_select(2, 3, 1), 2);
        assert_eq!(key_select(2, 3, -1), 1);
        assert_eq!(key_select(0, 0, 1), 0);
        let mut s = seg3();
        assert_eq!(s.key(1), 1);
        assert_eq!(s.view().selected, 0, "a key only asks");
        s.set(1);
        s.step(DT);
        let v = s.view();
        assert!(
            v.left > 4.0 && v.left < 64.0,
            "springs, not jumps: {}",
            v.left
        );
        settle(|| s.step(DT));
        assert!((s.view().left - 64.0).abs() < 0.1, "{}", s.view().left);
    }

    #[test]
    fn a_segment_press_selects_on_release_and_the_indicator_follows() {
        let mut s = seg3();
        assert_eq!((s.view().left, s.view().width), (4.0, 60.0));
        s.press(150.0, 0.0);
        assert_eq!(s.release(), Some(2));
        assert_eq!(s.release(), None, "released once");
        // controlled: the indicator stays until the value comes back
        settle(|| s.step(DT));
        assert_eq!((s.view().left, s.view().selected), (4.0, 0));
        s.set(2);
        s.step(DT);
        assert!(s.view().left > 4.0 && s.view().left < 124.0);
        settle(|| s.step(DT));
        let v = s.view();
        assert_eq!((v.left, v.width, v.selected), (124.0, 60.0, 2));
        assert!((v.scale_y - 1.0).abs() < 1e-3);
        // a press outside every segment chooses nothing
        s.press(1.0, 0.0);
        assert_eq!(s.release(), None);
    }

    #[test]
    fn a_dragged_indicator_bands_and_lands_on_the_flung_segment() {
        let mut s = seg3();
        s.press(30.0, 0.0);
        s.drag(500.0, 10.0);
        s.step(DT);
        let v = s.view();
        assert!(
            v.left > 124.0 && v.left < 124.0 + SEG::band + 1.0,
            "banded past the end: {}",
            v.left
        );
        let mut s = seg3();
        // a slow drag 20 px right: the indicator is still nearest its own segment
        s.press(30.0, 0.0);
        s.drag(40.0, 1000.0);
        s.drag(50.0, 2000.0);
        s.step(DT);
        assert_eq!(s.release(), Some(0));
        // the same distance, flung at 1000 px/s: 100 px further on, past segment 1
        let mut s = seg3();
        s.press(30.0, 0.0);
        s.drag(40.0, 10.0);
        s.drag(50.0, 20.0);
        s.step(DT);
        assert_eq!(s.release(), Some(2));
    }

    #[test]
    fn the_wall_holds_a_released_overshoot() {
        let mut s = seg3();
        s.set(2);
        let mut most: f64 = 0.0;
        settle(|| {
            let m = s.step(DT);
            let v = s.view();
            most = most.max(v.left + v.width);
            m
        });
        assert!(most <= 184.0 + SEG::wall, "{most}");
    }

    #[test]
    fn reduced_motion_snaps_the_indicator() {
        let mut s = seg3();
        crate::motion::set_reduced(true);
        s.set(1);
        assert!(!s.step(DT));
        crate::motion::set_reduced(false);
        let v = s.view();
        assert_eq!((v.left, v.width, v.scale_y), (64.0, 60.0, 1.0));
    }
}
