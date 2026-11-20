//! The thinking orb (Ultra Reactivate's `ThinkingOrb`, the owner's library in the design repo,
//! `src/ultra-reactivate/vendor/thinking-orbs`, from Libraries.dev, MIT, (c) 2026 Jakub Antalik):
//! a dotted 3D sphere whose dots move in one of nine hand-tuned ways, one per thing an agent can
//! be doing. Pure geometry: each frame is a z-sorted list of circles (and, for `connecting`,
//! line segments) with every value final, in CSS px of the tuned size; `wubble-gfx` draws it
//! (`orb.rs`) into the orb's own texture.
//!
//! - The nine modes are translated line for line from the TS (`prim`, `profiles`, `presets`,
//!   `orbits`, `lattice`, `web`, `braid`, `ribbon`, `morph`): each file says which lines, and
//!   `tests/orb_constants.rs` holds their numbers to the TS's; `tests.rs` replays frames the
//!   design's own engine computed (`tools/fidelity/sample-orbs.mjs`).
//! - The tables (states to modes, the 27 presets, the base profiles, the morph's timing, the
//!   labels) are generated: `kit_gen::orb`.
//! - [`Life`] is the orb's lifecycle on screen (one enum, one pure [`transition`]): off screen it
//!   draws nothing, still it costs no frames, and it asks for frames only while it moves.
//!
//! Time: the web runs every orb on one shared clock (`performance.now() / 1000 * speed`), so two
//! orbs of a state are in phase; [`Orb::time`] is the same function of the host's clock.

pub mod braid;
pub mod lattice;
pub mod morph;
pub mod orbits;
pub mod presets;
pub mod prim;
pub mod profiles;
pub mod ribbon;
#[cfg(test)]
mod tests;
pub mod web;

use crate::kit_gen::orb::{LABELS, STATE_TO_MODE};
use crate::kit_gen::orb_component as C;
use crate::kit_gen::orb_size;

pub use presets::{Resolved, resolve_preset};

/// A point in 2D / 3D, as the TS's `[number, number]` / `[number, number, number]`.
pub type V2 = [f64; 2];
pub type V3 = [f64; 3];
/// A painted colour: r, g, b in 0..255 (whole numbers, as the canvas's `rgba()` string), alpha
/// in 0..1.
pub type Ink = [f64; 4];

/// One dot (`Dot`): position in CSS px, depth `z`, radius, ink value (0 = darkest ink on paper,
/// mirrored on dark), alpha (`None` = 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub r: f64,
    pub white: f64,
    pub a: Option<f64>,
}

/// A stroked edge between two projected points (`Line`, the `connecting` web), `w` wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub white: f64,
    pub a: Option<f64>,
    pub w: f64,
}

/// One rendered instant (`OrbFrame`): lines are drawn first, then the dots in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrbFrame {
    pub dots: Vec<Dot>,
    pub lines: Vec<Line>,
}

/// An ink tint (`OrbTint`), 0..255 per channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbTint {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

/// A mode's draw options (`ModeOpts`): the TS object of optional numbers, keyed by the TS's
/// names. A key a mode does not read does nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeOpts(Vec<(&'static str, f64)>);

impl ModeOpts {
    pub fn of(pairs: &[(&'static str, f64)]) -> Self {
        let mut o = Self::default();
        o.merge(pairs);
        o
    }
    pub fn get(&self, key: &str) -> Option<f64> {
        self.0.iter().find(|(k, _)| *k == key).map(|&(_, v)| v)
    }
    pub fn set(&mut self, key: &'static str, value: f64) {
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }
    /// `{ ...self, ...pairs }`.
    pub fn merge(&mut self, pairs: &[(&'static str, f64)]) {
        for &(k, v) in pairs {
            self.set(k, v);
        }
    }
}

/// JS `Math.round`: halves round up (toward +∞).
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// A TS counting loop, `for (let i = a; i < n; i++)`, as numbers.
pub(crate) fn from(a: f64, n: f64) -> impl Iterator<Item = f64> {
    std::iter::successors(Some(a), |i| Some(i + 1.0)).take_while(move |&i| i < n)
}

/// `for (let i = a; i <= n; i++)`.
pub(crate) fn through(a: f64, n: f64) -> impl Iterator<Item = f64> {
    std::iter::successors(Some(a), |i| Some(i + 1.0)).take_while(move |&i| i <= n)
}

/// `i++`.
pub(crate) fn inc(i: &mut f64) {
    *i += 1.0;
}

/// A whole number used as an index.
pub(crate) fn ix(i: f64) -> usize {
    i as usize
}

/// JS truthiness of an optional number (`o.faceOn ? ...`).
pub(crate) fn truthy(v: Option<f64>) -> bool {
    v.is_some_and(nz)
}

/// JS truthiness of a number.
pub(crate) fn nz(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

/// The nine shipped states (`OrbState`), each a hand-tuned animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrbState {
    /// particles on tilted orbits
    Working,
    /// a scan meridian sweeps a dotted globe
    Searching,
    /// bands scramble in quarter turns, then click back
    Solving,
    /// a waveform rolls through latitude rings
    Listening,
    /// a constellation wires itself, packets running the edges
    Connecting,
    /// three strands plait around the sphere
    Weaving,
    /// an undulating multi-band sash
    Composing,
    /// a face-on ring slowly morphing
    Breathing,
    /// a dotted outline morphs circle, triangle, square
    Shaping,
}

impl OrbState {
    pub const ALL: [OrbState; 9] = [
        Self::Working,
        Self::Searching,
        Self::Solving,
        Self::Listening,
        Self::Connecting,
        Self::Weaving,
        Self::Composing,
        Self::Breathing,
        Self::Shaping,
    ];

    /// The TS's name for the state.
    pub fn name(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Searching => "searching",
            Self::Solving => "solving",
            Self::Listening => "listening",
            Self::Connecting => "connecting",
            Self::Weaving => "weaving",
            Self::Composing => "composing",
            Self::Breathing => "breathing",
            Self::Shaping => "shaping",
        }
    }

    /// Its mode (`STATE_TO_MODE`, generated).
    pub fn mode(self) -> ModeKey {
        let (_, mode) = STATE_TO_MODE
            .iter()
            .copied()
            .find(|&(s, _)| s == self.name())
            .expect("kit_gen::orb::STATE_TO_MODE names every state");
        ModeKey::ALL
            .into_iter()
            .find(|m| m.name() == mode)
            .expect("a known mode")
    }

    /// Its accessible label (`LABELS`, generated: "Working…").
    pub fn label(self) -> &'static str {
        LABELS
            .iter()
            .copied()
            .find(|&(s, _)| s == self.name())
            .map(|(_, l)| l)
            .expect("kit_gen::orb::LABELS names every state")
    }
}

/// The geometry builders (`ModeKey`); `ring` shares ribbon's, with `faceOn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeKey {
    Orbits,
    Globe,
    Rubik,
    Wave,
    Web,
    Braid,
    Ribbon,
    Ring,
    Morph,
}

impl ModeKey {
    pub const ALL: [ModeKey; 9] = [
        Self::Orbits,
        Self::Globe,
        Self::Rubik,
        Self::Wave,
        Self::Web,
        Self::Braid,
        Self::Ribbon,
        Self::Ring,
        Self::Morph,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Orbits => "orbits",
            Self::Globe => "globe",
            Self::Rubik => "rubik",
            Self::Wave => "wave",
            Self::Web => "web",
            Self::Braid => "braid",
            Self::Ribbon => "ribbon",
            Self::Ring => "ring",
            Self::Morph => "morph",
        }
    }

    /// One instant of this mode at `size` CSS px and time `t` (`MODE_FRAMES[mode]`).
    pub fn frame(self, size: f64, t: f64, opts: &ModeOpts) -> OrbFrame {
        match self {
            Self::Orbits => orbits::frame_orbits(size, t, opts),
            Self::Globe => lattice::frame_globe(size, t, opts),
            Self::Rubik => lattice::frame_rubik(size, t, opts),
            Self::Wave => lattice::frame_wave(size, t, opts),
            Self::Web => web::frame_web(size, t, opts),
            Self::Braid => braid::frame_braid(size, t, opts),
            Self::Ribbon | Self::Ring => ribbon::frame_ribbon(size, t, opts),
            Self::Morph => morph::frame_morph(size, t, opts),
        }
    }
}

/// The tuned sizes (`OrbSize`): 64 (chat avatar) and 20 (inline text) are hand-tuned designs,
/// 32 is interpolated between them. Other display sizes scale one of these (the kit's
/// `ThinkingOrb { tuned }`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrbSize {
    S64,
    S32,
    S20,
}

impl OrbSize {
    pub const ALL: [OrbSize; 3] = [Self::S64, Self::S32, Self::S20];

    /// CSS px (generated from types.ts).
    pub fn px(self) -> u32 {
        let px = match self {
            Self::S64 => orb_size::size_64,
            Self::S32 => orb_size::size_32,
            Self::S20 => orb_size::size_20,
        };
        px as u32
    }
}

/// The substrate the orb sits on: dark renders light ink, light renders dark ink.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OrbTheme {
    #[default]
    Dark,
    Light,
}

/// One orb's look: the component's props (`ThinkingOrbProps`) but those of the DOM.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orb {
    pub state: OrbState,
    pub size: OrbSize,
    pub theme: OrbTheme,
    /// speed multiplier on top of the preset's (`speed`, default 1)
    pub speed: f64,
    /// density multiplier (`dots`, default 1)
    pub dots: f64,
    /// radius multiplier (`dotSize`, default 1)
    pub dot_size: f64,
    pub tint: Option<OrbTint>,
}

impl Orb {
    pub fn new(state: OrbState, size: OrbSize, theme: OrbTheme) -> Self {
        Self {
            state,
            size,
            theme,
            speed: 1.0,
            dots: 1.0,
            dot_size: 1.0,
            tint: None,
        }
    }

    /// The mode, the effective speed and the options it draws with (ThinkingOrb.tsx's effect:
    /// the preset, then `dots` and `dotSize` through the presets' own scalers).
    pub fn resolved(&self) -> Resolved {
        let mut r = resolve_preset(self.state, self.size);
        if self.dots != 1.0 {
            r.opts = profiles::scale_counts(&r.opts, f64::max(C::dots_min, self.dots));
        }
        if self.dot_size != 1.0 {
            r.opts = profiles::scale_radii(&r.opts, f64::max(C::dot_size_min, self.dot_size));
        }
        r.speed *= self.speed;
        r
    }

    /// The orb's time at the shared clock's `now` seconds (`performance.now() / 1000 * speed`).
    pub fn time(&self, now: f64) -> f64 {
        now * self.resolved().speed
    }

    /// The frame at orb time `t`, in CSS px of the tuned size.
    pub fn frame(&self, t: f64) -> OrbFrame {
        let r = self.resolved();
        r.mode.frame(f64::from(self.size.px()), t, &r.opts)
    }

    /// What the painter draws for a frame, in order: every line, then every dot, each with its
    /// ink on this orb's substrate (`paintFrame`).
    pub fn marks(&self, frame: &OrbFrame) -> Marks {
        let dark = self.theme == OrbTheme::Dark;
        Marks {
            lines: frame
                .lines
                .iter()
                .map(|l| (*l, prim::line_ink(l, dark, self.tint)))
                .collect(),
            dots: frame
                .dots
                .iter()
                .map(|d| (*d, prim::dot_ink(d, dark, self.tint)))
                .collect(),
        }
    }
}

/// A frame's marks with their inks, in draw order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    pub lines: Vec<(Line, Ink)>,
    pub dots: Vec<(Dot, Ink)>,
}

/// The reduced-motion frame's time (`frame(0.6)`, generated).
pub const STILL_T: f64 = C::still_t;
/// The canvas's device-pixel cap (`Math.min(2, devicePixelRatio)`, generated).
pub const DPR_MAX: f64 = C::dpr_max;

/// An orb's life on screen. The web component draws one frame on mount, runs its loop only while
/// it is on screen, the tab is shown and it is not paused, and draws one fixed frame (t 0.6)
/// under reduced motion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Life {
    /// Not on screen: draws nothing, asks for nothing.
    Off,
    /// On screen and frozen at orb time `t` (`paused`): drawn once, then costs no frames.
    Still { t: f64 },
    /// On screen under reduced motion: the one representative frame, never animated.
    Reduced,
    /// On screen and moving: a frame each shell frame.
    Running,
}

/// What happens to an orb. Times are orb times ([`Orb::time`] of the shared clock).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LifeEvent {
    /// It came on screen (mounted and visible, the window shown).
    Shown { t: f64, paused: bool, reduced: bool },
    /// It left the screen (scrolled away, its surface closed, the window hidden).
    Hidden,
    /// `paused` set at orb time `t`: the frame on screen stays.
    Pause { t: f64 },
    /// `paused` cleared.
    Play,
    /// The reduced-motion preference changed.
    Reduce(bool),
}

/// What the host does for a transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LifeEffect {
    /// Draw the frame at orb time `t` into the orb's texture, once.
    Paint { t: f64 },
    /// Start (true) or stop asking for a frame every shell frame.
    Animate(bool),
}

/// The orb's lifecycle: one pure step. Anything not listed changes nothing.
pub fn transition(life: Life, event: LifeEvent) -> (Life, Vec<LifeEffect>) {
    use LifeEffect::*;
    match (life, event) {
        (Life::Off, LifeEvent::Shown { reduced: true, .. }) => {
            (Life::Reduced, vec![Paint { t: STILL_T }])
        }
        (
            Life::Off,
            LifeEvent::Shown {
                t, paused: true, ..
            },
        ) => (Life::Still { t }, vec![Paint { t }]),
        (Life::Off, LifeEvent::Shown { t, .. }) => {
            (Life::Running, vec![Paint { t }, Animate(true)])
        }
        (Life::Running, LifeEvent::Hidden) => (Life::Off, vec![Animate(false)]),
        (_, LifeEvent::Hidden) => (Life::Off, vec![]),
        (Life::Running, LifeEvent::Pause { t }) => (Life::Still { t }, vec![Animate(false)]),
        (Life::Still { .. }, LifeEvent::Play) => (Life::Running, vec![Animate(true)]),
        (Life::Running, LifeEvent::Reduce(true)) => {
            (Life::Reduced, vec![Animate(false), Paint { t: STILL_T }])
        }
        (Life::Still { .. }, LifeEvent::Reduce(true)) => {
            (Life::Reduced, vec![Paint { t: STILL_T }])
        }
        (Life::Reduced, LifeEvent::Reduce(false)) => (Life::Running, vec![Animate(true)]),
        (life, _) => (life, vec![]),
    }
}

impl Life {
    /// Whether the orb wants a frame every shell frame.
    pub fn animating(self) -> bool {
        self == Life::Running
    }
    /// Whether it is drawn at all.
    pub fn shown(self) -> bool {
        self != Life::Off
    }
}
