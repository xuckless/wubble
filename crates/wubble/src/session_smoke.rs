//! `--smoke session`: one process opens a second tab on a fresh profile and
//! quits (the orderly shutdown saves the session); a second process on the
//! same profile checks that both tabs came back, in order, with only the
//! visible one live. Each half is a pure state machine over what the host
//! sees after a run of CEF work, bounded by a count of runs, never by time.
//!
//! The icons ride along: the first process files an icon for a page and visits
//! it; the second asks the store (with one live view, so no view is needed)
//! and must get that icon back. The bundled `data:` tabs have no origin to
//! file an icon under, and a tab on a real page could not be made without
//! loading it, so the icon is checked through the history row path.

use wubble_engine::favicon::FaviconImage;

use crate::cli::Phase;
use crate::nav_smoke::MAX_WORK_RUNS;
use crate::persist::Hits;

/// The page whose icon the first process files and the second must find.
pub const ICON_PAGE: &str = "https://favicon.wubble.test/";

/// The icon filed for [`ICON_PAGE`]: 2x2, one distinct byte per pixel.
pub fn smoke_icon() -> FaviconImage {
    FaviconImage {
        url: "https://favicon.wubble.test/icon.png".into(),
        width: 2,
        height: 2,
        rgba: (0..16).collect(),
    }
}

/// Whether `hits` (a history query for [`ICON_PAGE`]) carry [`smoke_icon`].
pub fn icon_restored(hits: &Hits) -> bool {
    let icon = smoke_icon();
    hits.entries.iter().any(|e| e.url == ICON_PAGE)
        && hits
            .icons
            .get(ICON_PAGE)
            .is_some_and(|got| got.icon_url == icon.url && got.rgba == icon.rgba)
}

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The window can size views (the frame rate is known), so every view
    /// the model asked for so far is open.
    pub ready: bool,
    /// The active Space's tab URLs, in sidebar order.
    pub urls: Vec<String>,
    /// Views the engine holds.
    pub views: usize,
    /// Per tab in `urls`: whether the model has it discarded (no view).
    pub discarded: Vec<bool>,
    /// The index in `urls` of the visible tab.
    pub visible: Option<usize>,
    /// Whether the store gave back [`smoke_icon`]; `None` until it answered.
    pub icon: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Phase one: the second tab is opening.
    Opening,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSmoke {
    pub phase: Phase,
    pub stage: Stage,
    runs: u32,
}

impl SessionSmoke {
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            stage: Stage::Start,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open `want[1]` as a new tab.
    OpenSecond,
    /// Quit through the orderly shutdown (which saves).
    Pass,
    Fail(String),
}

/// `want` are the two tabs' URLs, in order.
pub fn next(s: SessionSmoke, seen: &Seen, want: &[String]) -> (SessionSmoke, Option<Action>) {
    let to = |stage| SessionSmoke {
        stage,
        runs: 0,
        ..s
    };
    let step = match (s.phase, s.stage) {
        (_, Stage::Done) => return (s, None),
        (Phase::Write, Stage::Start) if seen.ready && seen.urls == want[..1] => {
            Some((Stage::Opening, Action::OpenSecond))
        }
        (Phase::Write, Stage::Start) if seen.ready => Some((
            Stage::Done,
            Action::Fail(format!(
                "a fresh profile should start with one tab on the bundled page: {seen:?}"
            )),
        )),
        (Phase::Write, Stage::Opening) if seen.urls == want && seen.views == 2 => {
            Some((Stage::Done, Action::Pass))
        }
        (Phase::Read, Stage::Start) if seen.ready && seen.icon.is_some() => {
            Some((Stage::Done, check_restored(seen, want)))
        }
        _ => None,
    };
    match step {
        Some((stage, action)) => (to(stage), Some(action)),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Stage::Done),
            Some(Action::Fail(format!(
                "stuck at {:?} {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.phase, s.stage
            ))),
        ),
        None => (
            SessionSmoke {
                runs: s.runs + 1,
                ..s
            },
            None,
        ),
    }
}

/// The restored session: both URLs in order, the second (opened last, so
/// visible) the only live tab and the only view.
fn check_restored(seen: &Seen, want: &[String]) -> Action {
    if seen.urls != want {
        return Action::Fail(format!("restored URLs {:?}, want {want:?}", seen.urls));
    }
    if seen.visible != Some(1) || seen.discarded != [true, false] {
        return Action::Fail(format!(
            "want the second tab visible and live, the first discarded: {seen:?}"
        ));
    }
    if seen.views != 1 {
        return Action::Fail(format!("want exactly one live view, have {}", seen.views));
    }
    if seen.icon != Some(true) {
        return Action::Fail("the icon filed by the first run did not come back".into());
    }
    Action::Pass
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want() -> Vec<String> {
        vec!["a".into(), "b".into()]
    }

    fn seen(ready: bool, urls: &[&str], views: usize) -> Seen {
        Seen {
            ready,
            urls: urls.iter().map(|u| u.to_string()).collect(),
            views,
            discarded: vec![false; urls.len()],
            visible: urls.len().checked_sub(1),
            icon: Some(true),
        }
    }

    fn run(phase: Phase, inputs: &[Seen]) -> (SessionSmoke, Vec<Action>) {
        inputs
            .iter()
            .fold((SessionSmoke::new(phase), vec![]), |(s, mut all), seen| {
                let (s, action) = next(s, seen, &want());
                all.extend(action);
                (s, all)
            })
    }

    #[test]
    fn phase_one_opens_the_second_tab_then_passes() {
        let (s, actions) = run(
            Phase::Write,
            &[
                seen(false, &["a"], 0),
                seen(true, &["a"], 1),
                seen(true, &["a", "b"], 1),
                seen(true, &["a", "b"], 2),
                seen(true, &["a", "b"], 2),
            ],
        );
        assert_eq!(s.stage, Stage::Done);
        assert_eq!(actions, [Action::OpenSecond, Action::Pass]);
    }

    #[test]
    fn phase_one_needs_a_fresh_profile() {
        let (_, actions) = run(Phase::Write, &[seen(true, &["a", "b"], 1)]);
        assert!(
            matches!(actions.as_slice(), [Action::Fail(_)]),
            "{actions:?}"
        );
    }

    #[test]
    fn phase_two_checks_once_ready() {
        let restored = Seen {
            discarded: vec![true, false],
            ..seen(true, &["a", "b"], 1)
        };
        let (s, actions) = run(
            Phase::Read,
            &[seen(false, &["a", "b"], 0), restored.clone()],
        );
        assert_eq!(s.stage, Stage::Done);
        assert_eq!(actions, [Action::Pass]);

        let bad = [
            Seen {
                views: 2,
                ..restored.clone()
            },
            Seen {
                discarded: vec![false, false],
                ..restored.clone()
            },
            Seen {
                urls: vec!["b".into(), "a".into()],
                ..restored.clone()
            },
            Seen {
                visible: Some(0),
                ..restored.clone()
            },
            Seen {
                icon: Some(false),
                ..restored.clone()
            },
        ];
        for b in bad {
            let (_, actions) = run(Phase::Read, std::slice::from_ref(&b));
            assert!(matches!(actions.as_slice(), [Action::Fail(_)]), "{b:?}");
        }
    }

    #[test]
    fn phase_two_waits_for_the_store_to_answer_about_the_icon() {
        let waiting = Seen {
            discarded: vec![true, false],
            icon: None,
            ..seen(true, &["a", "b"], 1)
        };
        let (s, actions) = run(Phase::Read, &[waiting]);
        assert_eq!((s.stage, actions), (Stage::Start, vec![]));
    }

    #[test]
    fn the_icon_must_be_the_one_filed() {
        use std::collections::HashMap;
        use wubble_core::omnibox::HistoryEntry;
        let icon = smoke_icon();
        let stored = |rgba: Vec<u8>| wubble_store::Favicon {
            icon_url: icon.url.clone(),
            width: 2,
            height: 2,
            rgba,
        };
        let entry = HistoryEntry {
            url: ICON_PAGE.into(),
            title: String::new(),
            visit_count: 1,
            typed_count: 1,
            last_visit_ms: 1,
        };
        let hits = |entries: Vec<HistoryEntry>, rgba: Option<Vec<u8>>| Hits {
            entries,
            icons: rgba
                .map(|r| HashMap::from([(ICON_PAGE.to_string(), stored(r))]))
                .unwrap_or_default(),
            bookmarks: vec![],
        };
        assert!(icon_restored(&hits(
            vec![entry.clone()],
            Some(icon.rgba.clone())
        )));
        assert!(!icon_restored(&hits(
            vec![entry.clone()],
            Some(vec![0; 16])
        )));
        assert!(!icon_restored(&hits(vec![entry], None)));
        assert!(!icon_restored(&hits(vec![], Some(icon.rgba))));
    }

    #[test]
    fn a_stuck_phase_fails_once() {
        let inputs = vec![seen(false, &[], 0); MAX_WORK_RUNS as usize + 3];
        let (s, actions) = run(Phase::Read, &inputs);
        assert_eq!(s.stage, Stage::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("Read")));
    }
}
