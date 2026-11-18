//! The component sheet's physics: slider, toggle, download stretch, pin bend, dock magnify and the
//! dock's one tooltip. Pure; the frame driver steps them and Slint draws their [`views`](Slider::view).
//!
//! Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
//! `src/wubble/pages/touch.tsx` (`SLIDER`, `TOGGLE`, `PIN`, `DL`, `DOCK`; @source touch-slider,
//! touch-toggle, touch-pin, touch-download, touch-dock). The bucket has these only as TSX: the
//! loops are ported here onto the springs of [`crate::motion`], the constants copied verbatim.
//! Check boxes and menus are CSS transitions in the design and stay Slint tweens.
//!
//! Pointer positions are px along the control's own axis; time is `dt` in seconds or `now` in ms
//! passed in, never read.

use crate::motion::{
    Spring, Stretch, Velocity, band, band_clamp, clamp, spring, squash_at, squash_of, step_k,
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
            moving |= step_k(&mut self.x, self.target / 100.0 * self.w, SLIDER::follow, dt);
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

// @source touch-download
#[allow(non_snake_case, non_upper_case_globals)]
pub mod DL {
    /// +12% width while held
    pub const held: f64 = 0.12;
    /// ζ≈0.76: eases into the held stretch without bouncing
    pub const hold: [f64; 2] = [700.0, 40.0];
    /// px: rubber band for pulling past the edges
    pub const pull: f64 = 26.0;
    /// ζ≈0.35: back to rest with one visible overshoot
    pub const release: [f64; 2] = [520.0, 16.0];
}

/// The stretch button (the design's download button): pressed, it stretches outward from its
/// centre; released, it rubber-bands back with one overshoot.
#[derive(Clone, Debug)]
pub struct StretchButton {
    s: Spring,
    held: bool,
    /// Pointer and centre along the width, and the width, px.
    px: f64,
    cx: f64,
    w: f64,
}

impl Default for StretchButton {
    fn default() -> Self {
        Self::new()
    }
}

impl StretchButton {
    pub fn new() -> Self {
        Self {
            s: spring(1.0),
            held: false,
            px: 0.0,
            cx: 0.0,
            w: 0.0,
        }
    }

    pub fn press(&mut self, px: f64, width: f64) {
        self.w = width;
        self.cx = width / 2.0;
        self.px = px;
        self.held = true;
    }

    pub fn drag(&mut self, px: f64) {
        if self.held {
            self.px = px;
        }
    }

    pub fn release(&mut self) {
        self.held = false;
    }

    /// A key press: a kick instead of a hold.
    pub fn kick(&mut self) {
        self.s.v += 2.4;
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut target = 1.0;
        if self.held && self.w > 0.0 {
            let out = ((self.px - self.cx).abs() - self.w / 2.0).max(0.0);
            target = 1.0 + DL::held + 2.0 * band(out, DL::pull) / self.w;
        }
        let kc = if self.held { DL::hold } else { DL::release };
        step_k(&mut self.s, target, kc, dt) || self.held
    }

    /// `(scale_x, scale_y)`: the cross axis keeps the volume.
    pub fn view(&self) -> (f64, f64) {
        let k = self.s.x.max(0.5);
        (k, squash_of(k))
    }
}

// @source touch-pin
#[allow(non_snake_case, non_upper_case_globals)]
pub mod PIN {
    pub const rest: [f64; 10] = [6.0, 3.0, 18.0, 3.0, 18.0, 21.0, 12.0, 17.0, 6.0, 21.0];
    /// b = +1
    pub const hit: [f64; 10] = [4.4, 4.6, 19.6, 4.6, 20.0, 19.6, 12.0, 13.2, 4.0, 19.6];
    /// b = −1
    pub const rebound: [f64; 10] = [6.9, 1.8, 17.1, 1.8, 17.1, 22.2, 12.0, 18.8, 6.9, 22.2];
    /// k520 c9 (ζ≈0.2): two visible twangs
    pub const bend: [f64; 2] = crate::motion::K::seamBend;
    pub const press: f64 = 0.35;
    pub const pressSpring: [f64; 2] = [700.0, 30.0];
    /// release velocity into the hit, units/s
    pub const strike: f64 = 24.0;
    /// ° rotateX per unit of bend (Slint has no 3D: unused there)
    pub const tilt: f64 = 14.0;
    /// button scale per unit of bend: 0.35 → 0.97, the press token
    pub const scale: f64 = 0.086;
    pub const fill: [f64; 2] = crate::motion::K::follow;
}

/// The pin: a five-point ribbon bent by one under-damped spring; release strikes it.
#[derive(Clone, Debug)]
pub struct PinBend {
    b: Spring,
    f: Spring,
    held: bool,
    on: bool,
}

/// What the pin draws.
#[derive(Clone, Debug, PartialEq)]
pub struct PinView {
    /// The ribbon's outline on a 24 px grid, as an SVG path (`M x y L … Z`).
    pub path: String,
    /// How far the fill has poured in from the top, 0..=1.
    pub fill: f64,
    pub scale: f64,
    pub on: bool,
}

impl PinBend {
    pub fn new(on: bool) -> Self {
        Self {
            b: spring(0.0),
            f: spring(if on { 1.0 } else { 0.0 }),
            held: false,
            on,
        }
    }

    pub fn set(&mut self, on: bool) {
        self.on = on;
    }

    pub fn press(&mut self) {
        self.held = true;
    }

    /// `commit`: the pointer came up on the pin (it toggles and twangs); else it just lets go.
    pub fn release(&mut self, commit: bool) -> Option<bool> {
        if !self.held {
            return None;
        }
        self.held = false;
        commit.then(|| self.toggle())
    }

    /// Flips and strikes the ribbon (a click, a key). Returns the new state.
    pub fn toggle(&mut self) -> bool {
        self.on = !self.on;
        self.b.v += PIN::strike;
        self.on
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let (target, kc) = if self.held {
            (PIN::press, PIN::pressSpring)
        } else {
            (0.0, PIN::bend)
        };
        let m = step_k(&mut self.b, target, kc, dt);
        step_k(&mut self.f, if self.on { 1.0 } else { 0.0 }, PIN::fill, dt) || m
    }

    pub fn bend(&self) -> f64 {
        clamp(self.b.x, -1.4, 1.4)
    }

    pub fn view(&self) -> PinView {
        let b = self.bend();
        let src = if b >= 0.0 { PIN::hit } else { PIN::rebound };
        let u = b.abs();
        let p: Vec<f64> = PIN::rest
            .iter()
            .zip(src)
            .map(|(r, s)| r + (s - r) * u)
            .collect();
        PinView {
            path: format!(
                "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} Z",
                p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7], p[8], p[9]
            ),
            fill: clamp(self.f.x, 0.0, 1.0),
            scale: 1.0 - b * PIN::scale,
            on: self.on,
        }
    }
}

// @source touch-dock
#[allow(non_snake_case, non_upper_case_globals)]
pub mod DOCK {
    /// the icon under the pointer grows to 1.12
    pub const swell: f64 = 0.12;
    /// px: gaussian falloff; a neighbour one slot (46 px) away reaches 1.04
    pub const reach: f64 = 44.0;
    pub const spring: [f64; 2] = crate::motion::K::follow;
    /// tooltip glide + resize, ease in-out
    pub const glideMs: f64 = 260.0;
    /// ms of hover before the first tooltip
    pub const intent: f64 = 280.0;
    /// ms after leaving during which the next tooltip shows at once
    pub const warm: f64 = 520.0;
}

/// The scale an icon centred at `centre` aims for with the pointer at `px` (none: at rest).
pub fn dock_target(px: Option<f64>, centre: f64) -> f64 {
    px.map_or(1.0, |p| {
        1.0 + DOCK::swell * (-((p - centre) / DOCK::reach).powi(2)).exp()
    })
}

/// The toolbar dock's magnification: each icon's scale on its own follow spring.
#[derive(Clone, Debug, Default)]
pub struct Dock {
    scales: Vec<Spring>,
    centres: Vec<f64>,
    px: Option<f64>,
}

impl Dock {
    /// Icon centres, px along the dock.
    pub fn set_centres(&mut self, centres: &[f64]) {
        self.centres = centres.to_vec();
        self.scales.resize(centres.len(), spring(1.0));
    }

    /// The pointer along the dock, or `None` when it left.
    pub fn point(&mut self, px: Option<f64>) {
        self.px = px;
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let px = self.px;
        let mut moving = false;
        for (s, &c) in self.scales.iter_mut().zip(&self.centres) {
            moving |= step_k(s, dock_target(px, c), DOCK::spring, dt);
        }
        moving
    }

    pub fn scales(&self) -> Vec<f64> {
        self.scales.iter().map(|s| s.x).collect()
    }
}

/// The dock's one tooltip: the first waits for hover intent and appears in place; after that
/// it glides between icons; leaving keeps it warm for a moment. Times are ms, passed in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tooltip {
    Hidden {
        /// The next hover before this shows at once.
        warm_until: f64,
    },
    /// Hovering `item` since `since`, not shown yet.
    Intent { item: usize, since: f64 },
    /// Shown over `item`; `glide`: it moved from another icon (animate), else it appeared.
    Shown { item: usize, glide: bool },
}

impl Default for Tooltip {
    fn default() -> Self {
        Tooltip::Hidden {
            warm_until: f64::NEG_INFINITY,
        }
    }
}

impl Tooltip {
    /// The pointer entered `item` at `now` (`focus`: keyboard focus, shows at once).
    pub fn hover(self, item: usize, now: f64, focus: bool) -> Tooltip {
        match self {
            Tooltip::Shown { .. } => Tooltip::Shown { item, glide: true },
            Tooltip::Hidden { warm_until } if focus || now < warm_until => Tooltip::Shown {
                item,
                glide: false,
            },
            Tooltip::Intent { .. } if focus => Tooltip::Shown { item, glide: false },
            _ => Tooltip::Intent { item, since: now },
        }
    }

    /// The pointer left the dock at `now`.
    pub fn leave(self, now: f64) -> Tooltip {
        match self {
            Tooltip::Shown { .. } => Tooltip::Hidden {
                warm_until: now + DOCK::warm,
            },
            Tooltip::Hidden { .. } => self,
            Tooltip::Intent { .. } => Tooltip::default(),
        }
    }

    /// Time passed: intent long enough becomes shown.
    pub fn tick(self, now: f64) -> Tooltip {
        match self {
            Tooltip::Intent { item, since } if now - since >= DOCK::intent => Tooltip::Shown {
                item,
                glide: false,
            },
            other => other,
        }
    }

    /// When [`Tooltip::tick`] should next be called, if anything is pending.
    pub fn due(self) -> Option<f64> {
        match self {
            Tooltip::Intent { since, .. } => Some(since + DOCK::intent),
            _ => None,
        }
    }

    pub fn shown(self) -> Option<usize> {
        match self {
            Tooltip::Shown { item, .. } => Some(item),
            _ => None,
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

    #[test]
    fn download_stretch_peaks_held_and_overshoots_once() {
        let mut b = StretchButton::new();
        b.press(50.0, 100.0);
        for _ in 0..240 {
            b.step(DT);
        }
        assert!((b.view().0 - 1.12).abs() < 1e-3, "{}", b.view().0);
        b.release();
        let mut below = false;
        let mut crossings = 0;
        settle(|| {
            let m = b.step(DT);
            let now_below = b.view().0 < 1.0;
            if now_below != below {
                crossings += 1;
                below = now_below;
            }
            m
        });
        assert!(crossings >= 1, "narrows past rest");
        assert_eq!(b.view(), (1.0, 1.0));
    }

    #[test]
    fn download_pulled_past_an_edge_stretches_further_but_bands() {
        let mut b = StretchButton::new();
        b.press(50.0, 100.0);
        b.drag(1000.0);
        for _ in 0..240 {
            b.step(DT);
        }
        let k = b.view().0;
        assert!(k > 1.12 && k < 1.12 + 2.0 * 26.0 / 100.0, "{k}");
    }

    #[test]
    fn pin_twangs_and_returns_to_rest() {
        let mut p = PinBend::new(false);
        p.press();
        for _ in 0..120 {
            p.step(DT);
        }
        assert!((p.bend() - PIN::press).abs() < 0.01);
        assert_eq!(p.release(true), Some(true));
        let mut neg = false;
        settle(|| {
            let m = p.step(DT);
            neg |= p.bend() < 0.0;
            m
        });
        assert!(neg, "rebounds past rest");
        assert_eq!(p.bend(), 0.0);
        let v = p.view();
        assert_eq!(v.fill, 1.0);
        assert_eq!(
            v.path,
            "M 6.00 3.00 L 18.00 3.00 L 18.00 21.00 L 12.00 17.00 L 6.00 21.00 Z"
        );
        let mut p = PinBend::new(false);
        p.press();
        assert_eq!(p.release(false), None, "dragged off: no toggle");
    }

    #[test]
    fn dock_swells_under_the_pointer_and_its_neighbour_a_little() {
        assert!((dock_target(Some(0.0), 0.0) - 1.12).abs() < 1e-12);
        let n = dock_target(Some(0.0), 46.0);
        assert!((n - 1.04).abs() < 0.005, "{n}");
        assert_eq!(dock_target(None, 0.0), 1.0);
        let mut d = Dock::default();
        d.set_centres(&[22.0, 68.0]);
        d.point(Some(22.0));
        settle(|| d.step(DT));
        let s = d.scales();
        assert!((s[0] - 1.12).abs() < 0.03 && s[1] < s[0]);
        d.point(None);
        settle(|| d.step(DT));
        assert_eq!(d.scales(), vec![1.0, 1.0]);
    }

    #[test]
    fn tooltip_waits_for_intent_then_glides_and_stays_warm() {
        let t = Tooltip::default().hover(0, 1000.0, false);
        assert_eq!(t.shown(), None);
        assert_eq!(t.due(), Some(1280.0));
        assert_eq!(t.tick(1279.0).shown(), None);
        let t = t.tick(1280.0);
        assert_eq!(t, Tooltip::Shown { item: 0, glide: false });
        let t = t.hover(1, 1300.0, false);
        assert_eq!(t, Tooltip::Shown { item: 1, glide: true });
        let t = t.leave(1400.0);
        assert_eq!(t.shown(), None);
        // back within the warm window: at once, in place
        assert_eq!(
            t.hover(2, 1900.0, false),
            Tooltip::Shown { item: 2, glide: false }
        );
        // after it: intent again
        assert_eq!(t.hover(2, 1920.0, false).shown(), None);
        // focus never waits
        assert_eq!(Tooltip::default().hover(3, 0.0, true).shown(), Some(3));
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
