// Source: the owner's design repo (xuckless/the-wallpaper-project @ 6c840da),
// `src/wubble/slint/seam.rs`, copied verbatim except the marked `wubble change`s.

//! The rubber-string seam between the sidebar and the web view, ported 1:1 from
//! `browser/seam.ts`. Feed it the pointer, step it, read `w` (sidebar width), `bend` (how far the
//! string bows at the grab point), `gy` (the grab height) and `contents` (the sidebar contents'
//! width, which settles a beat later).
//!
//! Three sidebar modes share one width spring: extended (200–340, resizable), rail (56), hidden (0).
//! Dragging the seam past the minimum rubber-bands, and pulling further snaps to the rail (and
//! back), so the modes are reached by the same gesture as resizing.
//!
//! In Slint: `w` → the sidebar width, `contents` → the sidebar content width, and `clip()`'s
//! `seam` string → a `Path { commands: ... }` stroke for the bowed string. Slint can't clip an
//! element by an arbitrary path, so the web view keeps its rounded-rect clip; `path` is still
//! returned for renderers that can (a wgpu mask, or a Path filled with the page snapshot).

use crate::bow::{Curve, EdgeTable, cubics, edge_table};
use crate::motion::{Spring, band, band_clamp, clamp, spring, step};

// @source slint-seam
// wubble change: public, so the driver and the token test read the same numbers.
pub const INSET: f64 = 8.0; // chrome.inset
pub const SIDEBAR_DEFAULT: f64 = 252.0; // chrome.sidebarDefault

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Extended,
    Rail,
    Hidden,
}

// wubble change: the sidebar toggle, as `cycleSidebar` in browser/chrome.tsx: hidden opens the
// extended sidebar, anything else (the rail too) hides it. The rail is reached by the seam.
impl SidebarMode {
    /// The stored word (`settings::SIDEBAR_MODE`'s choices).
    pub fn id(self) -> &'static str {
        match self {
            SidebarMode::Extended => "extended",
            SidebarMode::Rail => "rail",
            SidebarMode::Hidden => "hidden",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            SidebarMode::Extended,
            SidebarMode::Rail,
            SidebarMode::Hidden,
        ]
        .into_iter()
        .find(|m| m.id() == id)
    }

    pub fn toggled(self) -> Self {
        if self == SidebarMode::Hidden {
            SidebarMode::Extended
        } else {
            SidebarMode::Hidden
        }
    }
}

/// `SEAM` in seam.ts (generated): min/max/rail (chrome tokens), bandK (width rubber band past
/// min/max), bendK (the string's bow saturates there), toRail/toHidden (release snap points),
/// fromRail (a pulled rail snaps back to extended past it), and the springs.
pub use crate::kit_gen::seam::SEAM;

/// What an arrow key on the sidebar's grab does (`gKey`, browser/chrome.tsx:240-248).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyAction {
    /// Change the mode (the store's `sidebar`): the mode's own spring and twang follow.
    Mode(SidebarMode),
    /// Resize the extended sidebar to `saved` and kick the string's bend by `kick` px/s.
    Resize { saved: f64, kick: f64 },
    /// No grab to key (the hidden sidebar has none).
    Stay,
}

/// The arrow key `dir` (+1 Right, -1 Left) on the grab of a sidebar in `mode` at `saved` wide:
/// a rail widens to extended; an extended sidebar at its minimum narrows to the rail; otherwise
/// the width steps by `SEAM_KEY_STEP` within `SEAM::min..=max` and the string twangs. Pure.
pub fn key(mode: SidebarMode, saved: f64, dir: f64) -> KeyAction {
    use crate::kit_gen::controls_b::{SEAM_KEY_KICK, SEAM_KEY_STEP};
    let dir = dir.signum();
    match mode {
        SidebarMode::Hidden => KeyAction::Stay,
        SidebarMode::Rail if dir > 0.0 => KeyAction::Mode(SidebarMode::Extended),
        SidebarMode::Extended if dir < 0.0 && saved <= SEAM::min => {
            KeyAction::Mode(SidebarMode::Rail)
        }
        _ => KeyAction::Resize {
            saved: clamp(saved + dir * SEAM_KEY_STEP, SEAM::min, SEAM::max),
            kick: dir * SEAM_KEY_KICK,
        },
    }
}

#[derive(Clone, Debug)]
pub struct Seam {
    pub w: Spring,
    pub bend: Spring,
    pub contents: Spring,
    pub gy: f64,
    pub held: bool,
    pub px: f64,
    pub py: f64,
    pub saved: f64,
    pub mode: SidebarMode,
    held_mode: SidebarMode,
}

impl Default for Seam {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam {
    pub fn new() -> Self {
        Seam {
            w: spring(SIDEBAR_DEFAULT),
            bend: spring(0.0),
            contents: spring(SIDEBAR_DEFAULT),
            gy: 0.0,
            held: false,
            px: 0.0,
            py: 0.0,
            saved: SIDEBAR_DEFAULT,
            mode: SidebarMode::Extended,
            held_mode: SidebarMode::Extended,
        }
    }

    /// wubble change: the seam as the last session left it, at rest. `saved`
    /// is the extended width (clamped to `SEAM::min..=SEAM::max`; not finite
    /// gives the default), and every spring starts at its target for `mode`,
    /// so the window opens without a twang.
    pub fn restored(saved: f64, mode: SidebarMode) -> Self {
        let saved = if saved.is_finite() {
            clamp(saved, SEAM::min, SEAM::max)
        } else {
            SIDEBAR_DEFAULT
        };
        let mut seam = Seam {
            saved,
            mode,
            held_mode: mode,
            ..Seam::new()
        };
        let w = seam.target_for(mode);
        seam.w = spring(w);
        seam.contents = spring(w);
        seam
    }

    pub fn target_for(&self, mode: SidebarMode) -> f64 {
        match mode {
            SidebarMode::Extended => self.saved,
            SidebarMode::Rail => SEAM::rail,
            SidebarMode::Hidden => 0.0,
        }
    }

    /// `kick` defaults to true in the TS: the string twangs in the direction of travel.
    pub fn set_mode(&mut self, mode: SidebarMode, kick: bool) {
        if mode == self.mode {
            return;
        }
        if kick {
            self.bend.v += (if self.target_for(mode) > self.w.x {
                1.0
            } else {
                -1.0
            }) * 260.0;
        }
        self.mode = mode;
    }

    /// An arrow key on the grab (`dir` +1 Right, -1 Left; `h` the window's inner height): a
    /// resize is applied here (width, twang, the pill to mid height); a mode change is returned
    /// for the caller's store, as the web's `gKey` writes `d.sidebar`.
    pub fn key(&mut self, dir: f64, h: f64) -> KeyAction {
        let action = key(self.mode, self.saved, dir);
        if let KeyAction::Resize { saved, kick } = action {
            self.saved = saved;
            self.bend.v += kick;
            self.gy = h / 2.0;
        }
        action
    }

    /// `px, py`: pointer in window px. `h`: window inner height.
    pub fn grab(&mut self, px: f64, py: f64, h: f64) {
        self.held = true;
        self.px = px;
        self.py = py;
        self.gy = clamp(py, 18.0, h - 18.0);
        self.held_mode = self.mode;
    }

    pub fn drag(&mut self, px: f64, py: f64) {
        self.px = px;
        self.py = py;
    }

    // @source slint-seam-release
    /// Returns the mode the release settles into.
    pub fn release(&mut self) -> SidebarMode {
        self.held = false;
        let raw = self.px - INSET;
        if raw < SEAM::toHidden && self.held_mode != SidebarMode::Extended {
            self.mode = SidebarMode::Hidden;
        } else if raw
            < (if self.held_mode == SidebarMode::Rail {
                SEAM::fromRail
            } else {
                SEAM::toRail
            })
        {
            // from the rail, the snap point moves out to fromRail, matching what step() shows while held
            self.mode = SidebarMode::Rail;
        } else {
            self.mode = SidebarMode::Extended;
            self.saved = clamp(self.w.x, SEAM::min, SEAM::max);
        }
        self.mode
    }

    // @source slint-seam-step
    /// `h`: window inner height. Returns true while anything moves.
    pub fn step(&mut self, dt: f64, h: f64) -> bool {
        let mut moving = false;
        if self.held {
            let raw = self.px - INSET;
            // while held: past the minimum it resists, and beyond the snap point it's already the rail
            let snap = if self.held_mode == SidebarMode::Rail {
                SEAM::fromRail
            } else {
                SEAM::toRail
            };
            let want = if raw < snap {
                band_clamp(raw, SEAM::rail, SEAM::rail, SEAM::bandK)
            } else {
                band_clamp(raw, SEAM::min, SEAM::max, SEAM::bandK)
            };
            step(&mut self.w, want, SEAM::held[0], SEAM::held[1], dt);
            // the string bows toward the pointer at the grab height
            self.bend.x = band(self.px - (INSET + self.w.x + 4.0), SEAM::bendK);
            self.bend.v = 0.0;
            self.gy += (clamp(self.py, 18.0, h - 18.0) - self.gy) * 1f64.min(dt * 30.0);
            moving = true;
        } else {
            let target = self.target_for(self.mode);
            moving = step(&mut self.w, target, SEAM::release[0], SEAM::release[1], dt) || moving;
            moving = step(
                &mut self.bend,
                0.0,
                SEAM::bendSpring[0],
                SEAM::bendSpring[1],
                dt,
            ) || moving;
        }
        // follow-through: contents settle a beat after the seam
        moving = step(
            &mut self.contents,
            self.w.x,
            SEAM::contents[0],
            SEAM::contents[1],
            dt,
        ) || moving;
        self.contents.x = clamp(self.contents.x, self.w.x - 40.0, self.w.x + 8.0);
        moving
    }

    /// wubble change: how far the string is displaced at each height of the window (`h` high,
    /// window coordinates; the web view's frame is inset by [`INSET`], corner radius `r`), the
    /// same curve `clip` draws. The sidebar's sections, its glass edge and the page's edge
    /// follow it. Empty while the string is straight.
    pub fn edge(&self, h: f64, r: f64) -> EdgeTable {
        let (top, bot) = (INSET + r, h - INSET - r);
        edge_table(Curve::Seam, self.bend.x, self.gy, top, bot, h)
    }

    // @source slint-seam-clip
    /// Clip path for the web view (in its own coordinates, `lw`×`lh`, corner radius `r`): the left
    /// edge is the bowed string, two cubic arcs meeting at the grab point. Returns (path, seam), SVG
    /// path strings formatted exactly like the TS (`toFixed(2)`), usable as Slint `Path.commands`.
    pub fn clip(&self, a: f64, lw: f64, lh: f64, r: f64, gy_local: f64) -> (String, String) {
        let f = to_fixed2;
        let g = clamp(gy_local, r + 4.0, lh - r - 4.0);
        let (top, bot) = (r, lh - r);
        // wubble change: the control points come from `bow::Curve` (one owner with the edge
        // table); the string is the same, bottom to top.
        let mut points = Curve::Seam.points(a, self.bend.x, top, g, bot);
        points.reverse();
        let strings = cubics(&points);
        let (jw, jh, jr) = (js_num(lw - r), js_num(lh - r), js_num(r));
        let (lw_, lh_) = (js_num(lw), js_num(lh));
        let path = format!(
            "M{} 0 L{jw} 0 Q{lw_} 0 {lw_} {jr} L{lw_} {jh} Q{lw_} {lh_} {jw} {lh_} L{} {lh_} Q{} {lh_} {} {} {strings} Q{} 0 {} 0 Z",
            f(a + r),
            f(a + r),
            f(a),
            f(a),
            f(bot),
            f(a),
            f(a + r)
        );
        let seam = format!("M{} {} {strings}", f(a), f(bot));
        (path, seam)
    }
}

/// `Number.prototype.toFixed(2)`: exact-decimal rounding, ties away from zero. Rust's `{:.2}` is
/// also exact-decimal but breaks exact ties (e.g. 0.125) to even, so those are handled here.
pub fn to_fixed2(x: f64) -> String {
    let t = x * 1000.0;
    // an exact tie needs x to be a dyadic rational whose third decimal is 5
    if (x * 8.0).fract() == 0.0 && t.fract() == 0.0 && (t as i64).abs() % 10 == 5 {
        let r = (x.abs() * 100.0 + 0.5).floor() / 100.0;
        return format!("{:.2}", if x < 0.0 { -r } else { r });
    }
    if x == 0.0 {
        return "0.00".into(); // also −0: (-0).toFixed(2) === "0.00"
    }
    format!("{:.2}", x)
}

/// A JS number interpolated into a template string (integers print without ".0").
fn js_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{}", x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keys_resize_and_change_modes() {
        use crate::kit_gen::controls_b::{SEAM_KEY_KICK, SEAM_KEY_STEP};
        assert_eq!(
            key(SidebarMode::Rail, 260.0, 1.0),
            KeyAction::Mode(SidebarMode::Extended)
        );
        assert_eq!(
            key(SidebarMode::Extended, SEAM::min, -1.0),
            KeyAction::Mode(SidebarMode::Rail)
        );
        assert_eq!(key(SidebarMode::Hidden, 260.0, 1.0), KeyAction::Stay);
        assert_eq!(
            key(SidebarMode::Extended, 260.0, -1.0),
            KeyAction::Resize {
                saved: 260.0 - SEAM_KEY_STEP,
                kick: -SEAM_KEY_KICK
            }
        );
        // clamped at both ends; a rail keyed left stays a rail and only twangs
        assert_eq!(
            key(SidebarMode::Extended, SEAM::max - 1.0, 1.0),
            KeyAction::Resize {
                saved: SEAM::max,
                kick: SEAM_KEY_KICK
            }
        );
        assert_eq!(
            key(SidebarMode::Extended, SEAM::min + 1.0, -1.0),
            KeyAction::Resize {
                saved: SEAM::min,
                kick: -SEAM_KEY_KICK
            }
        );
        let mut s = Seam::new();
        let a = s.key(1.0, 800.0);
        assert_eq!(s.saved, SIDEBAR_DEFAULT + SEAM_KEY_STEP);
        assert_eq!((s.bend.v, s.gy), (SEAM_KEY_KICK, 400.0));
        assert!(matches!(a, KeyAction::Resize { .. }));
        assert!(s.step(1.0 / 120.0, 800.0), "the twang moves");
    }

    /// wubble: the edge table is the string `clip` draws, in window coordinates: displaced by
    /// the bend at the grab height and not at all at the ends.
    #[test]
    fn the_edge_is_the_drawn_string() {
        let mut s = Seam::new();
        s.gy = 300.0;
        s.bend = spring(40.0);
        let (h, r) = (800.0, 14.0);
        let e = s.edge(h, r);
        assert!((e.at(300.0) - 40.0).abs() < 1e-3, "{}", e.at(300.0));
        assert_eq!(e.at(INSET), 0.0);
        assert_eq!(e.at(h - INSET), 0.0);
        // the string's joint, "… {b} {g} C…", sits at a + bend, g in the seam's frame
        let (_, line) = s.clip(100.0, 900.0, h - 2.0 * INSET, r, s.gy - INSET);
        assert!(line.contains("140.00 292.00 C"), "{line}");
        s.bend = spring(0.0);
        assert!(s.edge(h, r).offset.is_empty());
    }

    #[test]
    fn to_fixed_matches_js() {
        assert_eq!(to_fixed2(0.125), "0.13");
        assert_eq!(to_fixed2(-0.125), "-0.13");
        assert_eq!(to_fixed2(1.005), "1.00"); // 1.005 is really 1.00499999…
        assert_eq!(to_fixed2(2.0), "2.00");
        assert_eq!(to_fixed2(-0.001), "-0.00"); // JS: (-0.001).toFixed(2) === "-0.00"
    }

    #[test]
    fn pull_to_rail_and_back() {
        let mut s = Seam::new();
        s.grab(INSET + 252.0, 300.0, 800.0);
        s.drag(INSET + 60.0, 300.0);
        for _ in 0..30 {
            s.step(1.0 / 60.0, 800.0);
        }
        assert_eq!(s.release(), SidebarMode::Rail);
        while s.step(1.0 / 60.0, 800.0) {}
        assert_eq!(s.w.x, SEAM::rail);
        s.grab(INSET + 56.0, 300.0, 800.0);
        s.drag(INSET + 10.0, 300.0);
        s.step(1.0 / 60.0, 800.0);
        assert_eq!(s.release(), SidebarMode::Hidden);
    }

    #[test]
    fn restored_rests_at_its_mode_with_the_saved_width_clamped() {
        for (saved, mode, w) in [
            (300.0, SidebarMode::Extended, 300.0),
            (900.0, SidebarMode::Extended, SEAM::max),
            (10.0, SidebarMode::Extended, SEAM::min),
            (f64::NAN, SidebarMode::Extended, SIDEBAR_DEFAULT),
            (300.0, SidebarMode::Rail, SEAM::rail),
            (300.0, SidebarMode::Hidden, 0.0),
        ] {
            let mut s = Seam::restored(saved, mode);
            assert_eq!(s.mode, mode);
            assert_eq!(s.w.x, w, "{saved} {mode:?}");
            assert_eq!(s.w.x, s.target_for(mode));
            assert!(!s.step(1.0 / 60.0, 800.0), "{saved} {mode:?} is at rest");
            assert_eq!(s.contents.x, w);
        }
        assert_eq!(Seam::restored(300.0, SidebarMode::Hidden).saved, 300.0);
    }

    #[test]
    fn toggled_cycles_as_the_design_does() {
        assert_eq!(SidebarMode::Hidden.toggled(), SidebarMode::Extended);
        assert_eq!(SidebarMode::Extended.toggled(), SidebarMode::Hidden);
        assert_eq!(SidebarMode::Rail.toggled(), SidebarMode::Hidden);
    }

    /// Release at `raw` px (pointer x minus the inset) after a grab in `held`.
    fn released(held: SidebarMode, raw: f64) -> Seam {
        let mut s = Seam::new();
        s.mode = held;
        s.w.x = s.target_for(held);
        s.grab(INSET + s.w.x, 300.0, 800.0);
        s.drag(INSET + raw, 300.0);
        s.step(1.0 / 60.0, 800.0);
        s.release();
        s
    }

    #[test]
    fn release_rules() {
        use SidebarMode::*;
        let table = [
            // never hidden straight from extended: the rail first
            (Extended, 21.0, Rail),
            (Rail, 21.0, Hidden),
            (Hidden, 21.0, Hidden),
            (Extended, 127.0, Rail),
            (Extended, 129.0, Extended),
            // from the rail the snap point moves out to fromRail
            (Rail, 149.0, Rail),
            (Rail, 151.0, Extended),
        ];
        for (held, raw, want) in table {
            assert_eq!(released(held, raw).mode, want, "{held:?} released at {raw}");
        }
        // released extended, the width is saved inside min..max
        let s = released(Extended, 129.0);
        assert!(s.saved >= SEAM::min && s.saved <= SEAM::max, "{}", s.saved);
        let mut s = Seam::new();
        s.w.x = 1000.0;
        s.grab(INSET + 1000.0, 300.0, 800.0);
        s.release();
        assert_eq!(s.saved, SEAM::max);
        let mut s = Seam::new();
        s.w.x = 150.0;
        s.grab(INSET + 150.0, 300.0, 800.0);
        s.drag(INSET + 160.0, 300.0);
        s.release();
        assert_eq!(s.saved, SEAM::min);
    }

    /// One JSON number list, as the TS fixture writes it (comma-joined).
    fn numbers(v: &serde_json::Value) -> Vec<f64> {
        v.as_str()
            .unwrap()
            .split(',')
            .map(|x| x.parse().unwrap())
            .collect()
    }

    /// parity2's seam scenario against the TS run stored in the fixture (generated by running
    /// browser/seam.ts and core/physics.ts under bun). Exact: the maths is + - * / and sqrt.
    #[test]
    fn parity_with_ts() {
        let line = include_str!("../tests/fixtures/ts_parity.jsonl")
            .lines()
            .next()
            .unwrap();
        let ts: serde_json::Value = serde_json::from_str(line).unwrap();
        let mut s = Seam::new();
        let (h, dt) = (800.0, 1.0 / 60.0);
        let mut trace = vec![];
        s.grab(260.0, 300.0, h);
        for i in 1..=30 {
            let f = f64::from(i);
            s.drag(260.0 - f * 6.0, 300.0 + f * 3.0);
            s.step(dt, h);
            trace.extend([s.w.x, s.bend.x, s.gy]);
        }
        assert_eq!(s.release(), SidebarMode::Rail);
        assert_eq!(ts["m1"], "rail");
        for _ in 0..40 {
            s.step(dt, h);
        }
        s.grab(64.0, 200.0, h);
        s.drag(12.0, 210.0);
        for _ in 0..10 {
            s.step(dt, h);
        }
        assert_eq!(s.release(), SidebarMode::Hidden);
        assert_eq!(ts["m2"], "hidden");
        for _ in 0..20 {
            s.step(dt, h);
        }
        s.set_mode(SidebarMode::Extended, true);
        for _ in 0..25 {
            s.step(dt, h);
            trace.extend([s.w.x, s.bend.x, s.contents.x]);
        }
        let want = numbers(&ts["trace"]);
        assert_eq!(want.len(), (30 + 25) * 3);
        assert_eq!(trace, want);
        // wubble change: `ride` is gone (the sections follow `bow::EdgeTable`).
        let (path, seam) = s.clip(3.5, 900.0, 700.0, 20.0, 333.0);
        assert_eq!(path, ts["path"].as_str().unwrap());
        assert_eq!(seam, ts["seam"].as_str().unwrap());
    }
}
