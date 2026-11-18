//! What a page pane shows while its size changes (owner rule "Resize feel", TODO.md): the last
//! complete frame, drawn to fill the pane's new rectangle, stretched further on the design's
//! stretch spring while the size moves, until the frame painted at the new size arrives and the
//! residual stretch settles back to exactly 1:1.
//!
//! One enum per pane ([`PaneView`]) and one pure [`transition`]; the springs ([`PaneSprings`]) are
//! stepped by the frame driver. "Which frame size CEF is asked for" stays the engine's
//! `ViewState`; this is only "what the pane shows".
//!
//! Numbers: the design's window stretch (`WIN` in `browser/desktop.tsx` of the owner's design repo,
//! `// @source window`): up to 3.5% at 2600 px/s, on the stretch spring (k 700, c 24, `K.stretch`).
//! The design's 35% stretch is for small moving parts; a window-sized frame uses `WIN`.

use crate::motion::Stretch;

/// Up to 3.5% along the travel at speed (`WIN.stretchMax`).
pub const WIN_STRETCH_MAX: f64 = 0.035;
/// Full stretch at this speed, px/s (`WIN.stretchFull`).
pub const WIN_STRETCH_FULL: f64 = 2600.0;

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

/// What the pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneView {
    /// The frame was painted for the pane's size: 1:1, no stretch.
    Live,
    /// A frame of size `from` (the last complete one) fills a rectangle of another size.
    Stretching { from: Px },
    /// The frame at the wanted size has arrived; the residual stretch is settling.
    Settling,
}

/// A pane's view and the frame size its rectangle wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pane {
    pub view: PaneView,
    pub wanted: Px,
}

impl Pane {
    /// A pane whose first frame is still to come, wanting `wanted`.
    pub const fn new(wanted: Px) -> Self {
        Self {
            view: PaneView::Live,
            wanted,
        }
    }

    /// The size of the frame on screen.
    pub fn shown(self) -> Px {
        match self.view {
            PaneView::Stretching { from } => from,
            PaneView::Live | PaneView::Settling => self.wanted,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneEvent {
    /// The pane's rectangle now wants frames of this size.
    Layout(Px),
    /// A complete frame of this size is now on screen.
    Frame(Px),
    /// The springs came to rest.
    Rest,
}

/// The one transition. No effects: the pane's drawing follows from the state.
pub fn transition(pane: Pane, event: PaneEvent) -> Pane {
    use PaneView::*;
    let view = match event {
        PaneEvent::Layout(wanted) if wanted == pane.wanted => pane.view,
        // back to the size on screen: nothing is stale any more
        PaneEvent::Layout(wanted) if wanted == pane.shown() => Settling,
        PaneEvent::Layout(_) => Stretching { from: pane.shown() },
        PaneEvent::Frame(size) if size != pane.wanted => Stretching { from: size },
        PaneEvent::Frame(_) => match pane.view {
            Live => Live,
            Stretching { .. } | Settling => Settling,
        },
        PaneEvent::Rest => match pane.view {
            Settling => Live,
            other => other,
        },
    };
    let wanted = match event {
        PaneEvent::Layout(wanted) => wanted,
        PaneEvent::Frame(_) | PaneEvent::Rest => pane.wanted,
    };
    Pane { view, wanted }
}

/// The extra stretch per axis, on top of filling the pane: it grows with the speed the pane's
/// size changes and springs back with one or two damped wobbles.
#[derive(Clone, Debug, Default)]
pub struct PaneSprings {
    x: Stretch,
    y: Stretch,
}

impl PaneSprings {
    /// One frame of `dt` seconds with the pane at `size` (logical px). Returns true while the
    /// springs move.
    pub fn step(&mut self, size: (f64, f64), dt: f64) -> bool {
        // `Stretch` reaches `max` at `full` px/s; `WIN` reaches it at `stretchFull`.
        let full = WIN_STRETCH_FULL / WIN_STRETCH_MAX;
        let x = self.x.update_with(size.0, dt, WIN_STRETCH_MAX, full);
        let y = self.y.update_with(size.1, dt, WIN_STRETCH_MAX, full);
        x || y
    }

    /// The draw scale per axis for `view`: exactly 1 when `Live` (1:1 on whole pixels), never
    /// under 1 otherwise (under 1 would leave a gap at the pane's edge; the overshoot past the
    /// pane is clipped by it). The cross axis is not squashed: that too would leave a gap.
    pub fn scale(&self, view: PaneView) -> (f64, f64) {
        match view {
            PaneView::Live => (1.0, 1.0),
            PaneView::Stretching { .. } | PaneView::Settling => {
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

    fn run(pane: Pane, events: &[PaneEvent]) -> Pane {
        events.iter().fold(pane, |p, &e| transition(p, e))
    }

    #[test]
    fn a_new_size_stretches_the_frame_on_screen() {
        let p = transition(Pane::new(A), PaneEvent::Layout(B));
        assert_eq!(p.view, PaneView::Stretching { from: A });
        assert_eq!(p.wanted, B);
        // a second size only moves the target; the frame on screen is still A's
        let p = transition(p, PaneEvent::Layout(C));
        assert_eq!(p.view, PaneView::Stretching { from: A });
        assert_eq!(p.wanted, C);
    }

    #[test]
    fn a_stale_frame_replaces_the_one_stretched() {
        let p = run(Pane::new(A), &[PaneEvent::Layout(C), PaneEvent::Frame(B)]);
        assert_eq!(p.view, PaneView::Stretching { from: B });
        assert_eq!(p.wanted, C);
    }

    #[test]
    fn the_frame_at_the_wanted_size_settles_then_rest_is_live() {
        let p = run(Pane::new(A), &[PaneEvent::Layout(B), PaneEvent::Frame(B)]);
        assert_eq!(p.view, PaneView::Settling);
        assert_eq!(transition(p, PaneEvent::Rest).view, PaneView::Live);
        // a new size while settling stretches again, from the frame now on screen
        let p = transition(p, PaneEvent::Layout(C));
        assert_eq!(p.view, PaneView::Stretching { from: B });
    }

    #[test]
    fn back_to_the_size_on_screen_settles() {
        let p = run(Pane::new(A), &[PaneEvent::Layout(B), PaneEvent::Layout(A)]);
        assert_eq!(p.view, PaneView::Settling);
        assert_eq!(p.wanted, A);
    }

    #[test]
    fn live_ignores_its_own_size_and_rest() {
        let p = Pane::new(A);
        assert_eq!(transition(p, PaneEvent::Layout(A)), p);
        assert_eq!(transition(p, PaneEvent::Frame(A)), p);
        assert_eq!(transition(p, PaneEvent::Rest), p);
        // Rest only ends a settle: a stretched frame is still stale
        let s = transition(p, PaneEvent::Layout(B));
        assert_eq!(transition(s, PaneEvent::Rest), s);
    }

    #[test]
    fn a_first_frame_at_another_size_stretches() {
        assert_eq!(
            transition(Pane::new(B), PaneEvent::Frame(A)).view,
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
        assert_eq!(springs.scale(PaneView::Live), (1.0, 1.0));
    }
}
