//! `--smoke split`: two tabs on the bundled page, split side by side; once the split rests,
//! each pane must hold its own live view painting at its pane's size and show its page in the
//! window; then the split is undone and one view stays. Every wait is "until the host sees X",
//! bounded by a count of CEF work runs, never by time.

use wubble_gfx::FrameSize;

use crate::nav_smoke::MAX_WORK_RUNS;

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
    /// The panes' pixels checked; waiting one run before unsplitting.
    Checked,
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
    /// Take the focused page out of the split.
    Unsplit,
    Pass,
    Fail(String),
}

pub fn next(s: SplitSmoke, seen: &Seen) -> (SplitSmoke, Option<Action>) {
    let to = |stage| SplitSmoke { stage, runs: 0 };
    let step = match s.stage {
        Stage::Start => Some((Stage::Opened, Action::OpenSecond)),
        Stage::Opened if seen.tabs == 2 && seen.showing(1) => Some((Stage::Split, Action::Split)),
        Stage::Split if seen.showing(2) => Some((Stage::Checked, Action::CheckPixels)),
        Stage::Checked => Some((Stage::Unsplit, Action::Unsplit)),
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
