//! What a page pane shows while its size changes (owner rule "Resize feel", TODO.md): the last
//! complete frame, drawn to fill the pane's new rectangle, stretched further on the design's
//! stretch spring while the size moves, until the frame painted at the new size arrives: then the
//! stretch overshoots once and settles back to exactly 1:1, and the stale frame fades out over it.
//!
//! One enum per pane ([`PaneView`]) and one pure [`transition`]; the springs ([`PaneSprings`]) are
//! stepped by the frame driver. "Which frame size CEF is asked for" is the engine's `ViewState`,
//! the one owner of the wanted size: [`transition`] takes it as an input and keeps no copy.
//!
//! Numbers: the design's window stretch (`WIN` in `browser/desktop.tsx` of the owner's design repo,
//! `// @source window`): up to 3.5% at 2600 px/s, on the stretch spring (k 700, c 24, `K.stretch`).
//! The design's 35% stretch is for small moving parts; a window-sized frame uses `WIN`. The
//! cross-fade is the design's content swap (`Overlap` in the README's motion tokens: out 190 ms).

use crate::motion::Stretch;

/// Up to 3.5% along the travel at speed (`WIN.stretchMax`).
pub const WIN_STRETCH_MAX: f64 = 0.035;
/// Full stretch at this speed, px/s (`WIN.stretchFull`).
pub const WIN_STRETCH_FULL: f64 = 2600.0;
/// The stale frame fades out over the new one in this long, seconds (`Overlap` out, 190 ms).
pub const FADE: f64 = 0.19;
/// The velocity (scale per second) the frame at the new size arrives with after a large size
/// change: on the stretch spring it peaks near [`WIN_STRETCH_MAX`] (one damped wobble).
pub const ARRIVAL_KICK: f64 = 3.4;
/// A size change of this fraction of the new size or more gets the whole kick.
pub const ARRIVAL_FULL: f64 = 0.4;

/// A frame size in physical pixels (core has no gfx type).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Px {
    pub w: u32,
    pub h: u32,
}

impl Px {
    pub const fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }
}

/// What the pane shows. The shown frame's size is carried by the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneView {
    /// The frame was painted for the pane's size: 1:1, no stretch.
    Live { size: Px },
    /// A frame of size `from` (the last complete one) fills a rectangle wanting another size.
    Stretching { from: Px },
    /// The frame at the wanted size is on screen; the residual stretch settles, and while
    /// `fading` the stale frame it replaced fades out over it.
    Settling { size: Px, fading: bool },
}

impl PaneView {
    /// The size of the frame on screen.
    pub fn shown(self) -> Px {
        match self {
            PaneView::Live { size } | PaneView::Settling { size, .. } => size,
            PaneView::Stretching { from } => from,
        }
    }

    pub fn is_live(self) -> bool {
        matches!(self, PaneView::Live { .. })
    }

    /// The stale frame still fades out over the new one.
    pub fn is_fading(self) -> bool {
        matches!(self, PaneView::Settling { fading: true, .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneEvent {
    /// The pane's rectangle was laid out (the wanted size may have changed).
    Layout,
    /// A complete frame of this size is now on screen.
    Frame(Px),
    /// The stale frame's fade finished.
    Faded,
    /// The springs came to rest.
    Rest,
}

/// What the host does after a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneEffect {
    /// The frame at the wanted size replaced a stale one of size `from`: kick the springs
    /// ([`PaneSprings::arrive`]) and fade the stale frame out over it.
    Arrived { from: Px },
}

/// The one transition. `wanted` is the frame size the engine asked CEF for (its `ViewState`).
pub fn transition(view: PaneView, wanted: Px, event: PaneEvent) -> (PaneView, Option<PaneEffect>) {
    use PaneView::*;
    match event {
        // back to the size on screen: nothing is stale any more
        PaneEvent::Layout if wanted == view.shown() => match view {
            Stretching { from } => (
                Settling {
                    size: from,
                    fading: false,
                },
                None,
            ),
            other => (other, None),
        },
        PaneEvent::Layout => (Stretching { from: view.shown() }, None),
        PaneEvent::Frame(size) if size != wanted => (Stretching { from: size }, None),
        PaneEvent::Frame(size) => match view {
            Live { .. } => (Live { size }, None),
            Stretching { from } => (
                Settling { size, fading: true },
                Some(PaneEffect::Arrived { from }),
            ),
            // a second frame during the fade ends it: the stale one is no longer kept
            Settling { .. } => (
                Settling {
                    size,
                    fading: false,
                },
                None,
            ),
        },
        PaneEvent::Faded => match view {
            Settling { size, .. } => (
                Settling {
                    size,
                    fading: false,
                },
                None,
            ),
            other => (other, None),
        },
        PaneEvent::Rest => match view {
            Settling {
                size,
                fading: false,
            } => (Live { size }, None),
            other => (other, None),
        },
    }
}

/// The extra stretch per axis, on top of filling the pane: it grows with the speed the pane's
/// size changes and springs back with one or two damped wobbles; and the stale frame's fade.
#[derive(Clone, Debug)]
pub struct PaneSprings {
    x: Stretch,
    y: Stretch,
    /// 0 → 1: how far the stale frame has faded out.
    fade: f64,
}

impl Default for PaneSprings {
    fn default() -> Self {
        Self {
            x: Stretch::default(),
            y: Stretch::default(),
            fade: 1.0,
        }
    }
}

impl PaneSprings {
    /// One frame of `dt` seconds with the pane at `size` (logical px). Returns true while the
    /// stretch or the fade moves. A frame with no time passed (the first after idle) only notes the size: a
    /// size jump in no time is not a speed.
    pub fn step(&mut self, size: (f64, f64), dt: f64) -> bool {
        if dt <= 0.0 {
            self.x.seed(size.0);
            self.y.seed(size.1);
            return self.x.x() != 1.0
                || self.y.x() != 1.0
                || self.x.s.v != 0.0
                || self.y.s.v != 0.0
                || self.fade < 1.0;
        }
        self.fade = (self.fade + dt / FADE).min(1.0);
        // `Stretch` reaches `max` at `full` px/s; `WIN` reaches it at `stretchFull`.
        let full = WIN_STRETCH_FULL / WIN_STRETCH_MAX;
        let x = self.x.update_with(size.0, dt, WIN_STRETCH_MAX, full);
        let y = self.y.update_with(size.1, dt, WIN_STRETCH_MAX, full);
        x || y || self.fade < 1.0
    }

    /// The pane was not stepped for a while (off screen): its next size is measured from
    /// scratch, so a resize it missed is not a speed.
    pub fn forget(&mut self) {
        self.x.forget();
        self.y.forget();
    }

    /// The frame at the new size `to` replaced one of `from`: a kick on each axis that changed,
    /// in proportion to the change (whole at [`ARRIVAL_FULL`]), and the fade starts.
    pub fn arrive(&mut self, from: Px, to: Px) {
        let kick = |a: u32, b: u32| {
            let rel = f64::from(a.abs_diff(b)) / f64::from(b.max(1));
            ARRIVAL_KICK * (rel / ARRIVAL_FULL).min(1.0)
        };
        self.x.s.v += kick(from.w, to.w);
        self.y.s.v += kick(from.h, to.h);
        self.fade = 0.0;
    }

    /// How far the stale frame has faded out, 0 → 1.
    pub fn fade(&self) -> f64 {
        self.fade
    }

    /// The draw scale per axis for `view`: exactly 1 when `Live` (1:1 on whole pixels), never
    /// under 1 otherwise (under 1 would leave a gap at the pane's edge; the overshoot past the
    /// pane is clipped by it). The cross axis is not squashed: that too would leave a gap.
    pub fn scale(&self, view: PaneView) -> (f64, f64) {
        match view {
            PaneView::Live { .. } => (1.0, 1.0),
            PaneView::Stretching { .. } | PaneView::Settling { .. } => {
                (self.x.x().max(1.0), self.y.x().max(1.0))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{K, step};

    const A: Px = Px::new(1000, 600);
    const B: Px = Px::new(1040, 600);
    const C: Px = Px::new(1080, 600);
    const DT: f64 = 1.0 / 60.0;

    fn view(v: PaneView, wanted: Px, e: PaneEvent) -> PaneView {
        transition(v, wanted, e).0
    }

    #[test]
    fn a_new_size_stretches_the_frame_on_screen() {
        let live = PaneView::Live { size: A };
        let s = view(live, B, PaneEvent::Layout);
        assert_eq!(s, PaneView::Stretching { from: A });
        // a second size only moves the target; the frame on screen is still A's
        assert_eq!(view(s, C, PaneEvent::Layout), s);
    }

    #[test]
    fn a_stale_frame_replaces_the_one_stretched() {
        let s = PaneView::Stretching { from: A };
        assert_eq!(
            view(s, C, PaneEvent::Frame(B)),
            PaneView::Stretching { from: B }
        );
    }

    #[test]
    fn the_frame_at_the_wanted_size_arrives_fades_settles_then_rest_is_live() {
        let s = PaneView::Stretching { from: A };
        let (v, effect) = transition(s, B, PaneEvent::Frame(B));
        assert_eq!(
            v,
            PaneView::Settling {
                size: B,
                fading: true
            }
        );
        assert_eq!(effect, Some(PaneEffect::Arrived { from: A }));
        // still fading: rest waits for the fade
        assert_eq!(view(v, B, PaneEvent::Rest), v);
        let done = view(v, B, PaneEvent::Faded);
        assert_eq!(
            done,
            PaneView::Settling {
                size: B,
                fading: false
            }
        );
        assert_eq!(view(done, B, PaneEvent::Rest), PaneView::Live { size: B });
        // a new size while settling stretches again, from the frame now on screen
        assert_eq!(
            view(v, C, PaneEvent::Layout),
            PaneView::Stretching { from: B }
        );
    }

    #[test]
    fn a_second_arrival_during_the_fade_ends_it() {
        let v = PaneView::Settling {
            size: B,
            fading: true,
        };
        let (v, effect) = transition(v, B, PaneEvent::Frame(B));
        assert_eq!(
            v,
            PaneView::Settling {
                size: B,
                fading: false
            }
        );
        assert_eq!(effect, None);
    }

    #[test]
    fn back_to_the_size_on_screen_settles_without_a_fade() {
        let s = PaneView::Stretching { from: A };
        let (v, effect) = transition(s, A, PaneEvent::Layout);
        assert_eq!(
            v,
            PaneView::Settling {
                size: A,
                fading: false
            }
        );
        assert_eq!(effect, None);
    }

    #[test]
    fn live_ignores_its_own_size_fade_and_rest() {
        let live = PaneView::Live { size: A };
        for e in [
            PaneEvent::Layout,
            PaneEvent::Frame(A),
            PaneEvent::Faded,
            PaneEvent::Rest,
        ] {
            assert_eq!(transition(live, A, e), (live, None), "{e:?}");
        }
        // Rest only ends a settle: a stretched frame is still stale
        let s = PaneView::Stretching { from: A };
        assert_eq!(view(s, B, PaneEvent::Rest), s);
    }

    #[test]
    fn a_first_frame_at_another_size_stretches() {
        assert_eq!(
            view(PaneView::Live { size: A }, B, PaneEvent::Frame(A)),
            PaneView::Stretching { from: A }
        );
    }

    /// The same numbers as a hand-run of the design's window stretch: velocity smoothed 0.6/0.4,
    /// target `1 + min(0.035, v / 2600 * 0.035)`, stretch spring k 700 c 24.
    #[test]
    fn springs_match_the_design_window_stretch() {
        let mut springs = PaneSprings::default();
        let (mut s, mut vel, mut last) = (crate::motion::spring(1.0), 0.0f64, f64::NAN);
        let mut w = 800.0;
        for i in 0..60 {
            w += if i < 20 { 20.0 } else { 0.0 };
            springs.step((w, 600.0), DT);
            let inst = if last.is_nan() { 0.0 } else { (w - last) / DT };
            last = w;
            vel = vel * 0.6 + inst * 0.4;
            let target = 1.0 + WIN_STRETCH_MAX.min(vel.abs() / WIN_STRETCH_FULL * WIN_STRETCH_MAX);
            step(&mut s, target, K::stretch[0], K::stretch[1], DT);
            let view = PaneView::Stretching { from: A };
            assert!(
                (springs.scale(view).0 - s.x.max(1.0)).abs() < 1e-12,
                "frame {i}"
            );
            assert_eq!(springs.scale(view).1, 1.0);
        }
    }

    #[test]
    fn it_overshoots_then_rests_at_exactly_one_never_under() {
        let mut springs = PaneSprings::default();
        let view = PaneView::Stretching { from: A };
        let mut w = 800.0;
        let mut peak: f64 = 1.0;
        for _ in 0..20 {
            w += 40.0; // 2400 px/s
            springs.step((w, 600.0), DT);
            peak = peak.max(springs.scale(view).0);
        }
        assert!(peak > 1.02 && peak <= 1.0 + WIN_STRETCH_MAX * 1.5, "{peak}");
        let mut n = 0;
        while springs.step((w, 600.0), DT) {
            assert!(springs.scale(view).0 >= 1.0);
            n += 1;
            assert!(n < 10_000);
        }
        assert!((springs.scale(view).0 - 1.0).abs() < 1e-3);
        assert_eq!(springs.scale(PaneView::Live { size: A }), (1.0, 1.0));
    }

    /// Review finding: the first frame after idle steps with dt 0; a size jump then is no speed.
    #[test]
    fn a_size_jump_in_no_time_does_not_stretch() {
        let mut springs = PaneSprings::default();
        let view = PaneView::Stretching { from: A };
        springs.step((1000.0, 600.0), 0.0);
        springs.step((1010.0, 600.0), 0.0);
        assert_eq!(springs.scale(view), (1.0, 1.0));
        // the next real frame measures from 1010, not from nothing
        springs.step((1010.0, 600.0), DT);
        assert_eq!(springs.scale(view), (1.0, 1.0));
    }

    /// Review finding: a pane off screen while the window resized comes back without a twang.
    #[test]
    fn a_pane_back_on_screen_measures_from_scratch() {
        let mut springs = PaneSprings::default();
        let view = PaneView::Stretching { from: A };
        springs.step((1000.0, 600.0), DT);
        springs.forget();
        springs.step((1300.0, 600.0), DT);
        assert_eq!(springs.scale(view), (1.0, 1.0));
        // and a real move still stretches
        springs.step((1340.0, 600.0), DT);
        assert!(springs.scale(view).0 > 1.0);
    }

    /// Review finding: the frame at the new size arrives with an overshoot that settles to
    /// exactly 1, within a fixed number of fixed steps; the fade runs to 1 alongside.
    #[test]
    fn arrival_overshoots_once_and_settles_to_exactly_one() {
        let mut springs = PaneSprings::default();
        let settling = PaneView::Settling {
            size: Px::new(1400, 600),
            fading: true,
        };
        let w = 1400.0;
        springs.step((w, 600.0), DT);
        springs.arrive(Px::new(1000, 600), Px::new(1400, 600));
        assert_eq!(springs.fade(), 0.0);
        // a frame with no time passed (the first after idle) keeps the fade going
        assert!(springs.step((w, 600.0), 0.0));
        let mut peak: f64 = 1.0;
        let mut steps = 0;
        while springs.step((w, 600.0), DT) {
            peak = peak.max(springs.scale(settling).0);
            assert_eq!(springs.scale(settling).1, 1.0, "height did not change");
            steps += 1;
            assert!(steps < 120, "settles within 120 steps");
        }
        assert!(peak > 1.02 && peak <= 1.0 + WIN_STRETCH_MAX * 1.1, "{peak}");
        assert_eq!(springs.scale(settling), (1.0, 1.0));
        assert_eq!(springs.fade(), 1.0);
        // a small change kicks in proportion
        let mut small = PaneSprings::default();
        small.step((1000.0, 600.0), DT);
        small.arrive(Px::new(990, 600), Px::new(1000, 600));
        let mut peak_small: f64 = 1.0;
        while small.step((1000.0, 600.0), DT) {
            peak_small = peak_small.max(small.scale(settling).0);
        }
        assert!(peak_small < 1.002, "{peak_small}");
    }
}
