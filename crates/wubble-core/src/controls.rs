//! The component sheet's physics: slider and toggle, and when a control's film rim shows. Pure;
//! the frame driver steps them and Slint draws their [`views`](Slider::view). The pin, download
//! stretch, dock magnify and tooltip land with their components.
//!
//! Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
//! `src/wubble/pages/touch.tsx` (`SLIDER`, `TOGGLE`; @source touch-slider, touch-toggle). The
//! bucket has these only as TSX: the loops are ported here onto the springs of [`crate::motion`], the constants copied verbatim.
//! Check boxes are CSS transitions in the design and stay Slint tweens; the menu's lifecycle,
//! highlight and keys are [`menu`] (its curves stay Slint tweens).
//!
//! Pointer positions are px along the control's own axis; time is `dt` in seconds or `now` in ms
//! passed in, never read.

pub mod menu;

use crate::motion::{
    Spring, Stretch, Velocity, band_clamp, clamp, spring, squash_at, squash_of, step_k,
};

// @source touch-slider
#[allow(non_snake_case, non_upper_case_globals)]
pub mod SLIDER {
    pub const follow: [f64; 2] = crate::motion::K::follow;
    /// ζ≈0.8: a track press runs the knob to the finger in ~150 ms
    pub const catchUp: [f64; 2] = [900.0, 48.0];
    pub const band: f64 = 26.0;
    pub const stretchMax: f64 = 0.35;
    /// px: pressing this close to the knob grabs it where it is
    pub const grab: f64 = 22.0;
    pub const hold: [f64; 2] = [700.0, 24.0];
    pub const holdScale: f64 = 1.1;
    pub const bubble: [f64; 2] = [520.0, 24.0];
    /// ° of bubble lean per px of trail
    pub const lean: f64 = 0.8;
    pub const leanMax: f64 = 12.0;
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
    pub bubble_x: f64,
    /// Bubble lean in degrees.
    pub lean: f64,
    /// The value shown, 0..=100.
    pub value: f64,
    pub held: bool,
}

/// The elastic slider: held, the knob follows 1:1 (keeping the grab offset) and rubber-bands;
/// a press away from the knob sends it there on a catch-up spring; released, the follow spring.
#[derive(Clone, Debug)]
pub struct Slider {
    x: Spring,
    st: Stretch,
    hs: Spring,
    bx: Spring,
    /// 0..=100
    target: f64,
    shown: f64,
    held: bool,
    catching: bool,
    raw: f64,
    off: f64,
    w: f64,
}

impl Slider {
    pub fn new(value: f64, width: f64) -> Self {
        let value = clamp(value.round(), 0.0, 100.0);
        let x = value / 100.0 * width;
        Self {
            x: spring(x),
            st: Stretch::new(),
            hs: spring(1.0),
            bx: spring(x),
            target: value,
            shown: value,
            held: false,
            catching: false,
            raw: 0.0,
            off: 0.0,
            w: width,
        }
    }

    /// The track was laid out at `width`: an idle knob jumps to its value there.
    pub fn set_width(&mut self, width: f64) {
        self.w = width;
        if !self.held {
            self.x = spring(self.target / 100.0 * width);
            self.bx = self.x;
        }
    }

    /// A value from outside (the setting, the keyboard): the follow spring carries the knob.
    pub fn set(&mut self, value: f64) {
        let v = clamp(value.round(), 0.0, 100.0);
        self.target = v;
        if !self.held {
            self.shown = v;
        }
    }

    pub fn press(&mut self, px: f64) {
        let near = (px - self.x.x).abs() <= SLIDER::grab;
        self.off = if near { px - self.x.x } else { 0.0 };
        self.catching = !near;
        self.raw = px;
        self.held = true;
    }

    pub fn drag(&mut self, px: f64) {
        if self.held {
            self.raw = px;
        }
    }

    /// The value the slider lands on, if it was held.
    pub fn release(&mut self) -> Option<f64> {
        if !self.held {
            return None;
        }
        self.held = false;
        self.catching = false;
        self.target = self.shown;
        Some(self.shown)
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        if self.held {
            let want = band_clamp(self.raw - self.off, 0.0, self.w, SLIDER::band);
            if self.catching {
                step_k(&mut self.x, want, SLIDER::catchUp, dt);
                if (self.x.x - want).abs() < 0.5 {
                    self.catching = false;
                }
            } else {
                self.x = spring(want);
            }
            moving = true;
            if self.w > 0.0 {
                self.shown = (clamp(self.x.x, 0.0, self.w) / self.w * 100.0).round();
            }
        } else {
            moving |= step_k(
                &mut self.x,
                self.target / 100.0 * self.w,
                SLIDER::follow,
                dt,
            );
        }
        let hold = if self.held { SLIDER::holdScale } else { 1.0 };
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
            origin: if self.st.dir > 0.0 { 0.7 } else { 0.3 },
            bubble_x: self.bx.x,
            lean: clamp(
                (self.bx.x - clamp(self.x.x, 0.0, self.w)) * SLIDER::lean,
                -SLIDER::leanMax,
                SLIDER::leanMax,
            ),
            value: self.shown,
            held: self.held,
        }
    }
}

// @source touch-toggle
#[allow(non_snake_case, non_upper_case_globals)]
pub mod TOGGLE {
    /// px: track 92 − padding 2×5 − knob 42
    pub const travel: f64 = 40.0;
    pub const follow: [f64; 2] = crate::motion::K::follow;
    pub const band: f64 = 26.0;
    pub const hold: [f64; 2] = [700.0, 24.0];
    pub const holdScale: f64 = 1.14;
    /// px/s: hitting the wall faster than this squashes the knob
    pub const landing: f64 = 90.0;
    /// px/s for the full-strength squash
    pub const fullSquash: f64 = 650.0;
    /// s of drag velocity projected when deciding the side
    pub const fling: f64 = 0.12;
    /// px before a press becomes a drag
    pub const slop: f64 = 3.0;
}

/// What the toggle draws. Lengths are in the design's 92 px track; Slint scales them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToggleView {
    /// Knob offset along the travel, 0 (off) ..= [`TOGGLE::travel`] (on), banded past both.
    pub x: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    /// Transform origin across the knob (0 left edge, 1 right edge).
    pub origin: f64,
    /// How far the accent fill has bloomed out of the knob, 0..=1.
    pub fill: f64,
    pub on: bool,
}

#[derive(Clone, Copy, Debug)]
struct Squash {
    /// 0..1 of the squash token; 1 = at rest
    t: f64,
    amp: f64,
    side: f64,
}

/// The toggle: the knob rides the follow spring and cannot overshoot the track; the wall takes
/// its speed as a squash. Hold widens it, a drag follows 1:1, a fling decides the side.
#[derive(Clone, Debug)]
pub struct Toggle {
    on: bool,
    x: Spring,
    st: Stretch,
    hs: Spring,
    held: bool,
    drag: bool,
    x0: f64,
    p0: f64,
    raw: f64,
    vel: Velocity,
    q: Squash,
}

impl Toggle {
    pub fn new(on: bool) -> Self {
        Self {
            on,
            x: spring(if on { TOGGLE::travel } else { 0.0 }),
            st: Stretch::new(),
            hs: spring(1.0),
            held: false,
            drag: false,
            x0: 0.0,
            p0: 0.0,
            raw: 0.0,
            vel: Velocity::new(),
            q: Squash {
                t: 1.0,
                amp: 1.0,
                side: 1.0,
            },
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// A value from outside (the setting, the keyboard).
    pub fn set(&mut self, on: bool) {
        self.on = on;
    }

    /// `px` along the track, `t_ms` the event's time (for the fling's velocity).
    pub fn press(&mut self, px: f64, t_ms: f64) {
        self.held = true;
        self.drag = false;
        self.x0 = px;
        self.raw = px;
        self.p0 = self.x.x;
        self.vel.reset();
        self.vel.push(px, 0.0, t_ms);
    }

    pub fn drag(&mut self, px: f64, t_ms: f64) {
        if !self.held {
            return;
        }
        self.raw = px;
        self.vel.push(px, 0.0, t_ms);
        if !self.drag && (self.raw - self.x0).abs() > TOGGLE::slop {
            self.drag = true;
        }
    }

    /// The side it lands on, if it was held: a tap flips, a drag lands where it is flung.
    pub fn release(&mut self) -> Option<bool> {
        if !self.held {
            return None;
        }
        self.held = false;
        let on = if self.drag {
            self.drag = false;
            self.x.x + self.vel.get().0 * TOGGLE::fling > TOGGLE::travel / 2.0
        } else {
            !self.on
        };
        self.on = on;
        Some(on)
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        let t = TOGGLE::travel;
        if self.held && self.drag {
            self.x = spring(band_clamp(
                self.p0 + self.raw - self.x0,
                0.0,
                t,
                TOGGLE::band,
            ));
            moving = true;
        } else {
            let tgt = if self.on { t } else { 0.0 };
            moving |= step_k(&mut self.x, tgt, TOGGLE::follow, dt);
            let into = if self.on {
                self.x.x > t && self.x.v > 0.0
            } else {
                self.x.x < 0.0 && self.x.v < 0.0
            };
            if into {
                // the wall takes the energy
                let speed = self.x.v.abs();
                self.x = spring(tgt);
                if speed > TOGGLE::landing {
                    self.q = Squash {
                        t: 0.0,
                        amp: clamp(speed / TOGGLE::fullSquash, 0.35, 1.0),
                        side: if self.on { 1.0 } else { -1.0 },
                    };
                }
            }
        }
        if self.q.t < 1.0 {
            self.q.t += dt * 1000.0 / crate::motion::SQUASH_MS;
            moving = true;
        }
        let hold = if self.held { TOGGLE::holdScale } else { 1.0 };
        moving |= step_k(&mut self.hs, hold, TOGGLE::hold, dt);
        moving |= self.st.update(self.x.x, dt);
        moving
    }

    pub fn view(&self) -> ToggleView {
        let k = self.st.x().min(1.35) * self.hs.x;
        let (qx, qy) = squash_at(self.q.t, self.q.amp);
        let squashing = self.q.t < 1.0;
        // held: widen away from the travel; moving: from the trailing side; squash: the wall hit
        let origin = if squashing {
            if self.q.side > 0.0 { 1.0 } else { 0.0 }
        } else if self.held {
            if self.on { 1.0 } else { 0.0 }
        } else if self.st.dir > 0.0 {
            0.0
        } else {
            1.0
        };
        ToggleView {
            x: self.x.x,
            scale_x: k * qx,
            scale_y: squash_of(k) * qy,
            origin,
            fill: clamp(self.x.x / TOGGLE::travel, 0.0, 1.0).powf(0.8),
            on: self.on,
        }
    }
}

// @source touch-space
/// The segmented control's constants (`SEG` in `pages/touch-kit.tsx`), verbatim.
#[allow(non_snake_case, non_upper_case_globals)]
pub mod SEG {
    /// k300 c15: one visible overshoot
    pub const follow: [f64; 2] = crate::motion::K::follow;
    /// width settles stiffer than position so the body leads
    pub const size: [f64; 2] = crate::motion::K::stretch;
    /// rubber band past both ends while dragged
    pub const band: f64 = 26.0;
    /// px: how far a released overshoot may press past an end
    pub const wall: f64 = 6.0;
    /// +35% at 1500 px/s
    pub const stretchMax: f64 = 0.35;
    pub const press: f64 = 0.97;
    pub const pressSpring: [f64; 2] = [700.0, 30.0];
    /// s of velocity projected when choosing a segment on release
    pub const fling: f64 = 0.1;
    pub const radius: f64 = 10.0;
    /// px before a press on the indicator becomes a drag
    pub const slop: f64 = 3.0;
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

/// The segmented control with a liquid indicator (`Segmented` in `pages/touch-kit.tsx`): the
/// indicator follows on the follow spring and stretches along its travel from the trailing side;
/// grabbed, it follows 1:1 and rubber-bands; released, it picks the nearest segment along the
/// fling. A press elsewhere selects the segment under the pointer on release. The segments'
/// places come from the layout ([`Segmented::set_segments`]).
#[derive(Clone, Debug)]
pub struct Segmented {
    x: Spring,
    w: Spring,
    st: Stretch,
    pr: Spring,
    /// (left, width) of each segment, px.
    segs: Vec<(f64, f64)>,
    selected: usize,
    held: bool,
    drag: bool,
    raw: f64,
    /// Where on the indicator it was grabbed; `None` when the press missed it.
    off: Option<f64>,
    x0: f64,
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
            held: false,
            drag: false,
            raw: 0.0,
            off: None,
            x0: 0.0,
            vel: Velocity::new(),
        }
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

    /// Segment `i` was laid out at `left`, `width`: the indicator jumps to its segment there
    /// (a layout change is not a motion; the design re-measures and snaps).
    pub fn set_segment(&mut self, i: usize, left: f64, width: f64) {
        if self.segs.len() <= i {
            self.segs.resize(i + 1, (0.0, 0.0));
        }
        self.segs[i] = (left, width);
        if !self.held {
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
        self.held = true;
        self.drag = false;
        self.raw = px;
        self.x0 = px;
        self.vel.reset();
        self.vel.push(px, 0.0, t_ms);
        self.off = (px >= self.x.x && px <= self.x.x + self.w.x).then_some(px - self.x.x);
    }

    pub fn drag(&mut self, px: f64, t_ms: f64) {
        if !self.held {
            return;
        }
        self.raw = px;
        self.vel.push(px, 0.0, t_ms);
        if !self.drag && self.off.is_some() && (px - self.x0).abs() > SEG::slop {
            self.drag = true;
        }
    }

    /// The segment it lands on, if it was held: the nearest to the flung indicator after a
    /// drag, the one under the pointer after a press (none when it missed every segment).
    pub fn release(&mut self) -> Option<usize> {
        if !self.held {
            return None;
        }
        self.held = false;
        let chosen = if self.drag {
            self.drag = false;
            let centre = self.x.x + self.w.x / 2.0 + self.vel.get().0 * SEG::fling;
            self.segs
                .iter()
                .enumerate()
                .min_by(|a, b| {
                    let d = |s: &(f64, f64)| (s.0 + s.1 / 2.0 - centre).abs();
                    d(a.1).total_cmp(&d(b.1))
                })
                .map(|(i, _)| i)
        } else {
            let x = self.raw;
            self.segs.iter().position(|s| x >= s.0 && x <= s.0 + s.1)
        };
        if let Some(i) = chosen {
            self.selected = i;
        }
        chosen
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        let (tx, tw) = self.target();
        if self.held && self.drag {
            let (min, max) = self.ends();
            let off = self.off.unwrap_or(0.0);
            self.x = spring(band_clamp(self.raw - off, min, max - self.w.x, SEG::band));
            moving = true;
        } else {
            moving |= step_k(&mut self.x, tx, SEG::follow, dt);
        }
        moving |= step_k(&mut self.w, tw, SEG::size, dt);
        let press = if self.held && self.off.is_some() {
            SEG::press
        } else {
            1.0
        };
        moving |= step_k(&mut self.pr, press, SEG::pressSpring, dt);
        moving |= self.st.update(self.x.x, dt);
        moving
    }

    pub fn view(&self) -> SegmentView {
        let (min, max) = self.ends();
        let w = self.w.x;
        // the follow spring's overshoot at an end segment presses into the wall
        let x = if self.drag {
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
            held: self.held,
        }
    }
}

/// Whether a control's film rim shows: hover, press, drag or focus; never at rest (the design's
/// `wb-film`: "every touched rim").
pub fn rim_on(hover: bool, pressed: bool, dragging: bool, focused: bool) -> bool {
    hover || pressed || dragging || focused
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 120.0;

    fn settle(mut f: impl FnMut() -> bool) -> usize {
        let mut n = 0;
        while f() {
            n += 1;
            assert!(n < 20_000, "never settled");
        }
        n
    }

    #[test]
    fn slider_follows_bands_and_lands_on_release() {
        let mut s = Slider::new(50.0, 200.0);
        assert_eq!(s.view().x, 100.0);
        // grab the knob 5 px right of its centre and drag past the end
        s.press(105.0);
        s.drag(400.0);
        s.step(DT);
        let v = s.view();
        assert!(v.x > 200.0 && v.x < 200.0 + SLIDER::band, "banded: {}", v.x);
        assert_eq!(v.value, 100.0);
        assert_eq!(s.release(), Some(100.0));
        settle(|| s.step(DT));
        assert_eq!(s.view().x, 200.0);
        // the stretch rests within 1/1500 of 1 (motion::Stretch)
        assert!((s.view().scale_y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_track_press_catches_up_instead_of_jumping() {
        let mut s = Slider::new(0.0, 200.0);
        s.press(150.0);
        s.step(DT);
        let x = s.view().x;
        assert!(x > 0.0 && x < 150.0, "{x}");
        settle(|| {
            s.step(DT);
            s.catching
        });
        s.step(DT);
        assert_eq!(s.view().x, 150.0);
        assert_eq!(s.release(), Some(75.0));
    }

    #[test]
    fn toggle_lands_with_a_squash_that_scales_with_speed() {
        let mut t = Toggle::new(false);
        t.press(10.0, 0.0);
        assert_eq!(t.release(), Some(true));
        let mut peak: f64 = 1.0;
        settle(|| {
            let m = t.step(DT);
            peak = peak.max(t.view().scale_y.max(t.view().scale_x));
            m
        });
        assert!(t.q.amp >= 0.35, "the wall squashed it");
        assert!(peak > 1.05);
        let v = t.view();
        assert_eq!((v.x, v.fill), (TOGGLE::travel, 1.0));
        assert!((v.scale_x - 1.0).abs() < 1e-3 && (v.scale_y - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_fling_decides_the_side() {
        let mut t = Toggle::new(false);
        t.press(0.0, 0.0);
        // a short fast drag right: the knob is under half way, the fling carries it over
        t.drag(8.0, 10.0);
        t.drag(16.0, 20.0);
        t.step(DT);
        assert!(t.view().x < TOGGLE::travel / 2.0);
        assert_eq!(t.release(), Some(true));
        let mut t = Toggle::new(true);
        t.press(40.0, 0.0);
        t.drag(36.0, 100.0);
        t.drag(35.0, 200.0);
        assert_eq!(t.release(), Some(true), "slow and short: stays on");
    }

    fn seg3() -> Segmented {
        let mut s = Segmented::new(0);
        for (i, x) in [4.0, 64.0, 124.0].into_iter().enumerate() {
            s.set_segment(i, x, 60.0);
        }
        s
    }

    #[test]
    fn a_segment_press_selects_on_release_and_the_indicator_follows() {
        let mut s = seg3();
        assert_eq!((s.view().left, s.view().width), (4.0, 60.0));
        s.press(150.0, 0.0);
        assert_eq!(s.release(), Some(2));
        assert_eq!(s.release(), None, "released once");
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

    #[test]
    fn rim_shows_only_when_touched() {
        assert!(!rim_on(false, false, false, false));
        for i in 0..4 {
            let b = |n| i == n;
            assert!(rim_on(b(0), b(1), b(2), b(3)));
        }
    }
}
