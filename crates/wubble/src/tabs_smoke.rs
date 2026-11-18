//! `--smoke tabs`: three tabs on bundled pages, switch between them, put the
//! hidden ones to sleep, restore one, close one, and check after each step
//! that the model and the engine agree. Every wait is "until the host sees
//! X", bounded by a count of CEF work runs, never by time.

use crate::nav_smoke::MAX_WORK_RUNS;

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The model's tab titles, in sidebar order.
    pub titles: Vec<String>,
    /// Views the engine still holds.
    pub views: usize,
    /// Tabs the model keeps alive.
    pub live_tabs: usize,
    /// Open views CEF believes visible.
    pub visible_views: usize,
    /// The model's visible tab has an open, visible view.
    pub shown_is_visible: bool,
    /// The shell shows a frame of a tab that is not the visible one.
    pub stale_frame: bool,
}

impl Seen {
    /// Exactly one view is visible, it is the model's visible tab's, and no
    /// other tab's frame is on screen.
    fn one_visible(&self) -> bool {
        self.visible_views == 1 && self.shown_is_visible && !self.stale_frame
    }

    /// The engine holds exactly the model's live tabs, `n` of them.
    fn agrees(&self, n: usize) -> bool {
        self.views == n && self.live_tabs == n && self.one_visible()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Waiting for every tab's title.
    Titles,
    /// Activated tab `i`, waiting for it to be the one visible view.
    Switched(usize),
    /// Hidden tabs discarded, waiting for their views to close.
    Discarded,
    /// A discarded tab activated, waiting for its view.
    Restored,
    /// A tab closed, waiting for its view to close.
    Closed,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabsSmoke {
    pub stage: Stage,
    runs: u32,
}

impl Default for TabsSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            runs: 0,
        }
    }
}

/// What the host does next. Tab indices are in sidebar order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the second and third bundled pages as new tabs.
    OpenPages,
    Activate(usize),
    /// A lifecycle tick far enough ahead that every hidden tab is discarded.
    DiscardHidden,
    Close(usize),
    Pass,
    Fail(String),
}

/// `want` are the three pages' titles, in tab order.
pub fn next(s: TabsSmoke, seen: &Seen, want: &[&str]) -> (TabsSmoke, Option<Action>) {
    let to = |stage| TabsSmoke { stage, runs: 0 };
    let step = match s.stage {
        Stage::Start => Some((Stage::Titles, Action::OpenPages)),
        Stage::Titles if seen.titles == want => Some((Stage::Switched(0), Action::Activate(0))),
        Stage::Switched(0) if seen.agrees(3) => Some((Stage::Switched(1), Action::Activate(1))),
        Stage::Switched(_) if seen.agrees(3) => Some((Stage::Discarded, Action::DiscardHidden)),
        Stage::Discarded if seen.agrees(1) => Some((Stage::Restored, Action::Activate(0))),
        Stage::Restored if seen.agrees(2) => Some((Stage::Closed, Action::Close(0))),
        Stage::Closed if seen.agrees(1) && seen.titles.len() == 2 => {
            Some((Stage::Done, Action::Pass))
        }
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
            TabsSmoke {
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

    const WANT: [&str; 3] = ["a", "b", "c"];

    fn seen(titles: &[&str], views: usize, live: usize) -> Seen {
        Seen {
            titles: titles.iter().map(|t| t.to_string()).collect(),
            views,
            live_tabs: live,
            visible_views: 1,
            shown_is_visible: true,
            stale_frame: false,
        }
    }

    fn run(inputs: &[Seen]) -> (TabsSmoke, Vec<Action>) {
        inputs
            .iter()
            .fold((TabsSmoke::default(), vec![]), |(s, mut all), seen| {
                let (s, action) = next(s, seen, &WANT);
                all.extend(action);
                (s, all)
            })
    }

    #[test]
    fn happy_path() {
        let three = seen(&WANT, 3, 3);
        let (s, actions) = run(&[
            seen(&["a"], 1, 1),
            seen(&["a", "b", ""], 3, 3),
            three.clone(),
            three.clone(),
            three.clone(),
            // Discarded views close a few runs later.
            seen(&WANT, 3, 1),
            seen(&WANT, 1, 1),
            seen(&WANT, 2, 2),
            seen(&["b", "c"], 2, 1),
            seen(&["b", "c"], 1, 1),
        ]);
        assert_eq!(s.stage, Stage::Done);
        assert_eq!(
            actions,
            [
                Action::OpenPages,
                Action::Activate(0),
                Action::Activate(1),
                Action::DiscardHidden,
                Action::Activate(0),
                Action::Close(0),
                Action::Pass,
            ]
        );
    }

    #[test]
    fn two_visible_views_do_not_pass_a_switch() {
        let both = Seen {
            visible_views: 2,
            ..seen(&WANT, 3, 3)
        };
        let wrong = Seen {
            shown_is_visible: false,
            ..seen(&WANT, 3, 3)
        };
        let stale = Seen {
            stale_frame: true,
            ..seen(&WANT, 3, 3)
        };
        let (s, actions) = run(&[seen(&[], 0, 0), seen(&WANT, 3, 3), both, wrong, stale]);
        assert_eq!(s.stage, Stage::Switched(0));
        assert_eq!(actions, [Action::OpenPages, Action::Activate(0)]);
    }

    #[test]
    fn a_stuck_stage_fails_once_with_its_name() {
        let mut inputs = vec![seen(&[], 0, 0)];
        inputs.extend(vec![seen(&["a"], 1, 1); MAX_WORK_RUNS as usize + 3]);
        let (s, actions) = run(&inputs);
        assert_eq!(s.stage, Stage::Done);
        assert!(
            matches!(actions.as_slice(), [Action::OpenPages, Action::Fail(e)] if e.contains("Titles")),
            "{actions:?}"
        );
    }
}
