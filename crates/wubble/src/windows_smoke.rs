//! `--smoke windows`: two tabs on the bundled page in one window, a second window (its empty
//! Space shows the start page there, not in the first), one tab dragged into it (no ghost left behind), both
//! windows' pixels checked, the second window closed; then a link from
//! another app in a Little window, made a tab of the Space. After each step the
//! model, the engine and the windows must agree. Every wait is "until the host sees X", bounded
//! by a count of CEF work runs, never by time.

use crate::nav_smoke::MAX_WORK_RUNS;
use wubble_ui::{CommandBarState, CommandPlacement};

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
    /// Windows whose command bar is open (or still closing), the capsule of a shown start page
    /// aside ([`is_bar`]).
    pub bars: usize,
    /// The newest window shows a start page (`wubble://start`).
    pub start_in_newest: bool,
    /// The newest window's start page is up and its capsule holds the keys: the bar is open in
    /// the page (`CommandPlacement::InPage`).
    pub capsule_in_newest: bool,
    /// Windows still drawing a drag ghost.
    pub ghosts: usize,
    /// Windows whose page is still entering after a tab switch (or the hidden one leaving): its
    /// pixels are not the page's yet.
    pub entering: usize,
    /// Every model window has its size recorded.
    pub geometry: bool,
}

impl Seen {
    fn settled(&self, windows: usize, shown: usize) -> bool {
        self.model_windows == windows
            && self.open_windows == windows
            && self.views_agree
            && self.framed
            && self.shown == shown
            && self.ghosts == 0
            && self.entering == 0
            && self.bars == 0
            && self.geometry
    }
}

/// Whether a window's command bar counts as open: open or closing, unless it is the capsule of
/// the start page the window shows. That capsule is the address field and takes the keys once
/// the page's stage is built (`StartStage`'s `init`), which opens the bar in it; the host's
/// model says "start page" a few runs before that, so a new window's start page is seen first
/// without and then with it. A bar in the page still closing after the page left counts.
pub fn is_bar(state: CommandBarState, placement: CommandPlacement, start_shown: bool) -> bool {
    state != CommandBarState::Closed && !(placement == CommandPlacement::InPage && start_shown)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Waiting for both tabs' titles.
    Titles,
    /// A second window asked for, waiting for it.
    Opened,
    /// A tab dragged into it, waiting for both windows to show their page.
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
    /// The first window's hidden tab dragged onto the second window.
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
        // the start page fully up there: its capsule has the keys, no other bar opened
        Stage::Opened
            if seen.model_windows == 2
                && seen.open_windows == 2
                && seen.bars == 0
                && seen.start_in_newest
                && seen.capsule_in_newest =>
        {
            Some((Stage::Moved, Action::MoveTab))
        }
        // the moved tab shows there, in front of the start page
        Stage::Moved if seen.settled(2, 2) && !seen.start_in_newest => {
            Some((Stage::Closed, Action::CheckAndClose))
        }
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
            bars: 0,
            start_in_newest: false,
            capsule_in_newest: false,
            ghosts: 0,
            entering: 0,
            geometry: true,
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
        assert_eq!(a, None, "no start page in the new window yet");
        let (s, a) = next(
            s,
            &Seen {
                start_in_newest: true,
                ..seen(2, 2)
            },
        );
        assert_eq!(a, None, "the start page's stage is not built yet");
        let in_newest = Seen {
            start_in_newest: true,
            capsule_in_newest: true,
            ..seen(2, 2)
        };
        let (s, a) = next(
            s,
            &Seen {
                bars: 1,
                ..in_newest.clone()
            },
        );
        assert_eq!(a, None, "a command bar opened instead");
        let (s, a) = next(s, &in_newest);
        assert_eq!(a, Some(Action::MoveTab));
        let (s, a) = next(s, &in_newest);
        assert_eq!(a, None, "the moved tab is not in front yet");
        let (s, a) = next(
            s,
            &Seen {
                bars: 1,
                ..seen(2, 2)
            },
        );
        assert_eq!(a, None, "the bar still closing over the page");
        let (s, a) = next(s, &seen(2, 1));
        assert_eq!(a, None, "the moved tab is not shown yet");
        let ghost = Seen {
            ghosts: 1,
            ..seen(2, 2)
        };
        let (s, a) = next(s, &ghost);
        assert_eq!(a, None, "a ghost left in the source window");
        let entering = Seen {
            entering: 1,
            ..seen(2, 2)
        };
        let (s, a) = next(s, &entering);
        assert_eq!(a, None, "the moved tab's page still entering");
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
    fn a_start_pages_capsule_is_not_a_bar() {
        use CommandBarState::{Closed, Closing, Open};
        use CommandPlacement::{InPage, Overlay};
        // (state, placement, a start page shown, counted)
        let cases = [
            (Closed, Overlay, false, false),
            (Closed, InPage, true, false),
            // the new window's start page: its capsule took the keys
            (Open, InPage, true, false),
            (Closing, InPage, true, false),
            // the page left (a tab moved in): the bar in it is still closing
            (Closing, InPage, false, true),
            (Open, InPage, false, true),
            (Open, Overlay, true, true),
            (Closing, Overlay, false, true),
        ];
        for (state, placement, start, counted) in cases {
            assert_eq!(
                is_bar(state, placement, start),
                counted,
                "{state:?} {placement:?} start={start}"
            );
        }
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
