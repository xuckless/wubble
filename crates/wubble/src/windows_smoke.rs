//! `--smoke windows`: two tabs on the bundled page in one window, a second window, one tab
//! moved into it, both windows' pixels checked, the second window closed; then a link from
//! another app in a Little window, made a tab of the Space. After each step the
//! model, the engine and the windows must agree. Every wait is "until the host sees X", bounded
//! by a count of CEF work runs, never by time.

use crate::nav_smoke::MAX_WORK_RUNS;

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// Tabs whose title has arrived.
    pub titled: usize,
    /// Windows in the model, and platform windows the host has open.
    pub model_windows: usize,
    pub open_windows: usize,
    /// Every tab a window shows has a visible view and no other view is visible: one live
    /// view per visible tab.
    pub views_agree: bool,
    /// Every tab a window shows has a frame in that window.
    pub framed: bool,
    /// Tabs on screen in any window.
    pub shown: usize,
}

impl Seen {
    fn settled(&self, windows: usize, shown: usize) -> bool {
        self.model_windows == windows
            && self.open_windows == windows
            && self.views_agree
            && self.framed
            && self.shown == shown
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Waiting for both tabs' titles.
    Titles,
    /// A second window asked for, waiting for it.
    Opened,
    /// A tab moved into it, waiting for both windows to show their page.
    Moved,
    /// The second window closed, waiting for its tab to be hidden.
    Closed,
    /// A link from another app, waiting for its Little window to show it.
    Little,
    /// The Little window's page made a tab, waiting for the window to go.
    Promoted,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowsSmoke {
    pub stage: Stage,
    runs: u32,
}

impl Default for WindowsSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A second tab on the bundled page.
    OpenPage,
    NewWindow,
    /// The first window's hidden tab into the second window.
    MoveTab,
    /// Check both windows' pixels, then close the second window.
    CheckAndClose,
    /// The bundled page as if another app opened it.
    OpenLittle,
    /// The Little window's page into the active Space.
    Promote,
    Pass,
    Fail(String),
}

pub fn next(s: WindowsSmoke, seen: &Seen) -> (WindowsSmoke, Option<Action>) {
    let to = |stage| WindowsSmoke { stage, runs: 0 };
    let step = match s.stage {
        Stage::Start => Some((Stage::Titles, Action::OpenPage)),
        Stage::Titles if seen.titled == 2 && seen.settled(1, 1) => {
            Some((Stage::Opened, Action::NewWindow))
        }
        Stage::Opened if seen.model_windows == 2 && seen.open_windows == 2 => {
            Some((Stage::Moved, Action::MoveTab))
        }
        Stage::Moved if seen.settled(2, 2) => Some((Stage::Closed, Action::CheckAndClose)),
        Stage::Closed if seen.settled(1, 1) => Some((Stage::Little, Action::OpenLittle)),
        Stage::Little if seen.settled(2, 2) => Some((Stage::Promoted, Action::Promote)),
        Stage::Promoted if seen.settled(1, 1) => Some((Stage::Done, Action::Pass)),
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
            WindowsSmoke {
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

    fn seen(windows: usize, shown: usize) -> Seen {
        Seen {
            titled: 2,
            model_windows: windows,
            open_windows: windows,
            views_agree: true,
            framed: true,
            shown,
        }
    }

    #[test]
    fn the_steps_in_order() {
        let s = WindowsSmoke::default();
        let (s, a) = next(s, &Seen::default());
        assert_eq!(a, Some(Action::OpenPage));
        let (s, a) = next(s, &seen(1, 1));
        assert_eq!(a, Some(Action::NewWindow));
        let (s, a) = next(s, &seen(2, 1));
        assert_eq!(a, Some(Action::MoveTab));
        let (s, a) = next(s, &seen(2, 1));
        assert_eq!(a, None, "the moved tab is not shown yet");
        let (s, a) = next(s, &seen(2, 2));
        assert_eq!(a, Some(Action::CheckAndClose));
        let (s, a) = next(s, &seen(1, 1));
        assert_eq!(a, Some(Action::OpenLittle));
        let (s, a) = next(s, &seen(2, 2));
        assert_eq!(a, Some(Action::Promote));
        let (s, a) = next(s, &seen(1, 1));
        assert_eq!((s.stage, a), (Stage::Done, Some(Action::Pass)));
    }

    #[test]
    fn a_window_without_its_frame_waits() {
        let s = WindowsSmoke {
            stage: Stage::Moved,
            runs: 0,
        };
        let unframed = Seen {
            framed: false,
            ..seen(2, 2)
        };
        assert_eq!(next(s, &unframed).1, None);
        let stuck = WindowsSmoke {
            stage: Stage::Moved,
            runs: MAX_WORK_RUNS,
        };
        assert!(matches!(next(stuck, &unframed).1, Some(Action::Fail(_))));
    }
}
