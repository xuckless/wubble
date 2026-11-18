//! `--smoke split`: two tabs on the bundled page, split side by side; once the split rests,
//! each pane must hold its own live view painting at its pane's size and show its page in the
//! window; then the seam between them is dragged [`DRAG`] px left and let go (the shell's seam
//! callbacks), and both views must repaint at the new sizes; then the seam is pulled again, and
//! both panes must show their frame cut along the bowed string (the page edge mask) until it is
//! let go and rests straight again; then the split is undone and one view stays. Every wait is
//! "until the host sees X",
//! bounded by a count of CEF work runs, never by time.

use wubble_gfx::FrameSize;

use crate::nav_smoke::MAX_WORK_RUNS;

/// How far the smoke drags the seam left, logical px.
pub const DRAG: f32 = 80.0;
/// How far the smoke pulls the seam left again to bend the string, logical px.
pub const BEND: f32 = 150.0;
/// The left pane must narrow by more than this (physical px) for the drag to count: most of
/// [`DRAG`] at any scale of at least 1.
const NARROWED: u32 = 60;

/// One page on screen: the frame size its pane asks for, and the size of its last frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneSeen {
    pub want: FrameSize,
    pub got: Option<FrameSize>,
}

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// Tabs in the active Space.
    pub tabs: usize,
    /// The pages on screen, left to right.
    pub panes: Vec<PaneSeen>,
    /// Open views CEF believes visible.
    pub visible_views: usize,
    /// The split's strings rest and nothing they settled waits.
    pub at_rest: bool,
    /// Panes showing their frame cut along a bowed string.
    pub masked: usize,
}

impl Seen {
    /// `n` pages on screen, each one view, each painting at its pane's size.
    fn showing(&self, n: usize) -> bool {
        self.panes.len() == n
            && self.visible_views == n
            && self.at_rest
            && self.panes.iter().all(|p| p.got == Some(p.want))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// The second tab opened, waiting for it.
    Opened,
    /// Split, waiting for two live panes at their sizes.
    Split,
    /// The panes' pixels checked; next, grab the seam and drag it.
    Checked,
    /// The seam is held [`DRAG`] px left of where it rested, when the left pane was `left`
    /// physical px wide; waiting for the pane to follow before letting go.
    Held {
        left: u32,
    },
    /// Let go; waiting for both panes to rest, painted at their new sizes.
    Released {
        left: u32,
    },
    /// The resize checked; next, pull the seam again to bend it.
    Resized,
    /// The seam pulled, waiting for both panes' edges to bow with it.
    Bending,
    /// Let go, waiting for the string to rest straight and the masks to go.
    Straightening,
    /// Unsplit, waiting for one page alone.
    Unsplit,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitSmoke {
    pub stage: Stage,
    runs: u32,
}

impl Default for SplitSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            runs: 0,
        }
    }
}

/// What the host does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the second bundled page in a new tab.
    OpenSecond,
    /// Put the first tab beside the focused (second) one.
    Split,
    /// Snapshot the window and check every pane shows its page; a failure fails the smoke.
    CheckPixels,
    /// Press the seam between the panes and drag it [`DRAG`] px left (held).
    Grab,
    /// Let go of the seam.
    Release,
    /// The panes rest at their new sizes, both painting at them: (left, right) physical px.
    Resized(u32, u32),
    /// Grab the seam and pull it [`BEND`] px toward the left pane (held).
    Bend,
    /// Take the focused page out of the split.
    Unsplit,
    Pass,
    Fail(String),
}

/// The left of two panes is narrower than `left` by more than [`NARROWED`].
fn narrowed(seen: &Seen, left: u32) -> bool {
    seen.panes.len() == 2 && seen.panes[0].want.width + NARROWED < left
}

pub fn next(s: SplitSmoke, seen: &Seen) -> (SplitSmoke, Option<Action>) {
    let to = |stage| SplitSmoke { stage, runs: 0 };
    let step = match s.stage {
        Stage::Start => Some((Stage::Opened, Action::OpenSecond)),
        Stage::Opened if seen.tabs == 2 && seen.showing(1) => Some((Stage::Split, Action::Split)),
        Stage::Split if seen.showing(2) => Some((Stage::Checked, Action::CheckPixels)),
        Stage::Checked => seen
            .panes
            .first()
            .map(|p| (Stage::Held { left: p.want.width }, Action::Grab)),
        Stage::Held { left } if narrowed(seen, left) => {
            Some((Stage::Released { left }, Action::Release))
        }
        Stage::Released { left } if seen.showing(2) && narrowed(seen, left) => Some((
            Stage::Resized,
            Action::Resized(seen.panes[0].want.width, seen.panes[1].want.width),
        )),
        Stage::Resized => Some((Stage::Bending, Action::Bend)),
        Stage::Bending if seen.masked == 2 => Some((Stage::Straightening, Action::Release)),
        Stage::Straightening if seen.showing(2) && seen.masked == 0 => {
            Some((Stage::Unsplit, Action::Unsplit))
        }
        Stage::Unsplit if seen.tabs == 2 && seen.showing(1) => Some((Stage::Done, Action::Pass)),
        _ => None,
    };
    match step {
        Some((stage, action)) => (to(stage), Some(action)),
        None if s.stage == Stage::Done => (s, None),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Stage::Done),
            Some(Action::Fail(format!(
                "stuck at {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.stage
            ))),
        ),
        None => (
            SplitSmoke {
                runs: s.runs + 1,
                ..s
            },
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(w: u32, painted: bool) -> PaneSeen {
        let want = FrameSize::new(w, 700);
        PaneSeen {
            want,
            got: painted.then_some(want),
        }
    }

    fn seen(tabs: usize, panes: Vec<PaneSeen>, visible_views: usize) -> Seen {
        Seen {
            tabs,
            panes,
            visible_views,
            at_rest: true,
            masked: 0,
        }
    }

    #[test]
    fn passes_through_split_check_and_unsplit() {
        let s = SplitSmoke::default();
        let (s, a) = next(s, &Seen::default());
        assert_eq!(a, Some(Action::OpenSecond));
        // the first tab is still the only one
        let (s, a) = next(s, &seen(1, vec![pane(1000, true)], 1));
        assert_eq!(a, None);
        let (s, a) = next(s, &seen(2, vec![pane(1000, true)], 1));
        assert_eq!(a, Some(Action::Split));
        // the second pane has not painted at its size yet
        let (s, a) = next(s, &seen(2, vec![pane(496, true), pane(496, false)], 2));
        assert_eq!(a, None);
        let mut moving = seen(2, vec![pane(496, true), pane(496, true)], 2);
        moving.at_rest = false;
        let (s, a) = next(s, &moving);
        assert_eq!(a, None);
        let (s, a) = next(s, &seen(2, vec![pane(496, true), pane(496, true)], 2));
        assert_eq!(a, Some(Action::CheckPixels));
        let (s, a) = next(s, &seen(2, vec![pane(496, true), pane(496, true)], 2));
        assert_eq!(a, Some(Action::Grab));
        // held: the left pane has not followed yet, then it has
        let mut held = seen(2, vec![pane(496, true), pane(496, true)], 2);
        held.at_rest = false;
        let (s, a) = next(s, &held);
        assert_eq!(a, None);
        held.panes = vec![pane(416, false), pane(576, false)];
        let (s, a) = next(s, &held);
        assert_eq!(a, Some(Action::Release));
        // let go: not until both have painted at their new sizes
        let (s, a) = next(s, &seen(2, vec![pane(416, true), pane(576, false)], 2));
        assert_eq!(a, None);
        let (s, a) = next(s, &seen(2, vec![pane(416, true), pane(576, true)], 2));
        assert_eq!(a, Some(Action::Resized(416, 576)));
        let two = seen(2, vec![pane(416, true), pane(576, true)], 2);
        let (s, a) = next(s, &two);
        assert_eq!(a, Some(Action::Bend));
        // one edge bowed is not both
        let (s, a) = next(
            s,
            &Seen {
                masked: 1,
                ..two.clone()
            },
        );
        assert_eq!(a, None);
        let (s, a) = next(
            s,
            &Seen {
                masked: 2,
                ..two.clone()
            },
        );
        assert_eq!(a, Some(Action::Release));
        // still bowed (twanging): not yet
        let (s, a) = next(
            s,
            &Seen {
                masked: 2,
                ..two.clone()
            },
        );
        assert_eq!(a, None);
        let (s, a) = next(s, &two);
        assert_eq!(a, Some(Action::Unsplit));
        // two views still visible: not yet
        let (s, a) = next(s, &seen(2, vec![pane(1000, true)], 2));
        assert_eq!(a, None);
        let (s, a) = next(s, &seen(2, vec![pane(1000, true)], 1));
        assert_eq!(a, Some(Action::Pass));
        assert_eq!(next(s, &Seen::default()), (s, None));
    }

    #[test]
    fn a_stuck_stage_fails_once_after_the_run_budget() {
        let mut s = SplitSmoke {
            stage: Stage::Split,
            runs: 0,
        };
        let stuck = seen(2, vec![pane(496, false)], 1);
        for _ in 0..MAX_WORK_RUNS {
            let (n, a) = next(s, &stuck);
            assert_eq!(a, None);
            s = n;
        }
        let (s, a) = next(s, &stuck);
        assert!(matches!(a, Some(Action::Fail(m)) if m.contains("Split")));
        assert_eq!(next(s, &stuck).1, None);
    }
}
