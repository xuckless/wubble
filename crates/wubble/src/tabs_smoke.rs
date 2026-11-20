//! `--smoke tabs`: three tabs on bundled pages, switch between them, put the
//! hidden ones to sleep, restore one (showing its snapshot until it paints),
//! close one, then freeze a hidden page and check its timers stop while a
//! hidden control page's keep running, and run again once thawed. Last, two fresh tabs of one
//! solid colour each: once the switch to the second settles, its card shows none of the first's
//! pixels (QA HIGH-04). Then a pinned page opens a tab by a `target=_blank` link click (it goes
//! last, under another tab); closing it shows the page again, not the tab above it (qa4: a tab
//! closes back to its opener). Again by a middle click (a background tab), then the tab is selected
//! in the sidebar and closed: the page shows again. After each step the model and the engine must agree. Every wait is
//! "until the host sees X", bounded by a count of CEF work runs, never by time.

use std::collections::{BTreeMap, BTreeSet};

use wubble_core::tab::TabId;

use crate::nav_smoke::MAX_WORK_RUNS;

/// The timers page's titles start with this, then count `setInterval` ticks.
pub const TIMER_TITLE: &str = "timer ";

/// The solid pages' colours, far from any of the shell's: the first page, then the fresh tab
/// switched to from it.
pub const SOLID: [[u8; 3]; 2] = [[0xff, 0x00, 0xff], [0x00, 0xff, 0xff]];

/// Solid page `i`: its colour edge to edge, titled `solid i`.
pub fn solid_page(i: usize) -> String {
    let [r, g, b] = SOLID[i];
    format!(
        "<!doctype html><meta charset=utf-8><title>{}</title>\
         <body style=\"margin:0;background:#{r:02x}{g:02x}{b:02x}\">",
        solid_title(i)
    )
}

fn solid_title(i: usize) -> String {
    format!("solid {i}")
}

/// The opener page's title.
pub const LINK_TITLE: &str = "opener";

/// The opener page: one `target=_blank` link over the whole page, so a click anywhere on it
/// opens a new tab (on `about:blank`: a page may not open a `data:` URL).
pub fn link_page() -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>{LINK_TITLE}</title>\
         <body style=\"margin:0\"><a href=\"about:blank#opened\" target=\"_blank\" \
         style=\"position:fixed;inset:0;display:block;background:#335\">open</a>"
    )
}

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
    /// The visible tab's pane shows a frame it painted (not its snapshot).
    pub painted: bool,
    /// The visible tab's pane showed its snapshot until its first frame replaced it.
    pub restored_from_snapshot: bool,
    /// Each tab's last freeze answer (frozen, ok), in sidebar order.
    pub frozen: Vec<Option<(bool, bool)>>,
    /// The visible tab's title.
    pub shown_title: String,
    /// No tab switch is fading: the entrance and the hidden page's exit are over.
    pub settled: bool,
    /// Where the visible tab is in `titles`.
    pub shown_index: Option<usize>,
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

    fn title(&self, i: usize) -> &str {
        self.titles.get(i).map_or("", String::as_str)
    }

    /// Tab `i` shows the timers page and has ticked at least once.
    fn ticking(&self, i: usize) -> bool {
        self.title(i)
            .strip_prefix(TIMER_TITLE)
            .is_some_and(|n| n != "0")
    }

    fn frozen(&self, i: usize) -> Option<(bool, bool)> {
        self.frozen.get(i).copied().flatten()
    }
}

/// The frozen tab and the control tab of the freeze check, in sidebar order.
const FROZEN: usize = 2;
const CONTROL: usize = 3;
/// Title changes of the control tab that must pass while the frozen one shows none.
const CONTROL_TICKS: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Waiting for every tab's title.
    Titles,
    /// Activated tab `i`, waiting for it to be the one visible view, painted.
    Switched(usize),
    /// Hidden tabs discarded, waiting for their views to close.
    Discarded,
    /// A discarded tab activated, waiting for its view and for its snapshot to give way to its
    /// first frame.
    Restored,
    /// A tab closed, waiting for its view to close.
    Closed,
    /// Two hidden timers pages opened, waiting for both to tick.
    Timers,
    /// Asked to freeze the first, waiting for Chromium's answer.
    Freezing,
    /// Chromium answered, but posted the freeze to the renderer only as it did: a tick already
    /// under way may still land. Waiting for the control tab to change once (`control` was its
    /// title at the answer) before the frozen tab's title counts.
    Settling {
        control: String,
    },
    /// The first is frozen at title `frozen`; the control has changed `ticks` times, last to
    /// `control`.
    Frozen {
        frozen: String,
        control: String,
        ticks: u32,
    },
    /// Asked to thaw the first, frozen at title `frozen`, waiting for it to tick again.
    Thawing {
        frozen: String,
    },
    /// Opened solid page `i` as a new tab, waiting for it to show, painted, the switch over.
    Solid(usize),
    /// The bleed check ran: next the opener page.
    Opener,
    /// Opened the opener page, pinned, waiting for it to show, painted.
    LinkShown,
    /// Clicked its link with `tabs` tabs open, waiting for the opened tab to show.
    Linked {
        tabs: usize,
    },
    /// Closed the opened tab, waiting for the opener to show again; then the background round
    /// (`background` false) or the pass.
    Reverted {
        tabs: usize,
        background: bool,
    },
    /// Middle-clicked the link with `tabs` tabs open, waiting for the background tab.
    Background {
        tabs: usize,
    },
    /// Selected the background tab, waiting for it to show.
    Selected {
        tabs: usize,
    },
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabsSmoke {
    pub stage: Stage,
    runs: u32,
    /// What the host told the smoke as it happened; the host keeps none of it itself.
    pub heard: Heard,
}

/// Host events the smoke needs and the host has no other use for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Heard {
    /// Each tab's last freeze answer (frozen, ok).
    pub frozen: BTreeMap<TabId, (bool, bool)>,
    /// Tabs whose snapshot stand-in a real frame replaced.
    pub replaced: BTreeSet<TabId>,
}

impl Default for TabsSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            runs: 0,
            heard: Heard::default(),
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
    /// Open the timers page twice, then show the first tab again: both stay hidden.
    OpenTimers,
    /// Freeze (true) or thaw tab `i`'s page.
    Freeze(usize, bool),
    /// Open solid page `i` ([`solid_page`]) as a new tab.
    OpenSolid(usize),
    /// Snapshot the window: the card shows solid page 1 and no pixel of solid page 0; fail on
    /// it.
    CheckNoBleed,
    /// Open [`link_page`] as a new tab and pin it.
    OpenLinkPage,
    /// Click the visible page's middle (middle button: `true`).
    ClickLink(bool),
    /// Close the visible tab.
    CloseShown,
    Pass,
    Fail(String),
}

/// `want` are the three pages' titles, in tab order.
pub fn next(s: TabsSmoke, seen: &Seen, want: &[&str]) -> (TabsSmoke, Option<Action>) {
    use Stage::*;
    let fail = |why: String| Some((Done, Some(Action::Fail(why))));
    let step: Option<(Stage, Option<Action>)> = match &s.stage {
        Start => Some((Titles, Some(Action::OpenPages))),
        Titles if seen.titles == want => Some((Switched(0), Some(Action::Activate(0)))),
        Switched(0) if seen.agrees(3) && seen.painted => {
            Some((Switched(1), Some(Action::Activate(1))))
        }
        Switched(_) if seen.agrees(3) && seen.painted => {
            Some((Discarded, Some(Action::DiscardHidden)))
        }
        Discarded if seen.agrees(1) => Some((Restored, Some(Action::Activate(0)))),
        Restored if seen.agrees(2) && seen.restored_from_snapshot => {
            Some((Closed, Some(Action::Close(0))))
        }
        Closed if seen.agrees(1) && seen.titles.len() == 2 => {
            Some((Timers, Some(Action::OpenTimers)))
        }
        Timers if seen.one_visible() && seen.ticking(FROZEN) && seen.ticking(CONTROL) => {
            Some((Freezing, Some(Action::Freeze(FROZEN, true))))
        }
        Freezing => match seen.frozen(FROZEN) {
            Some((true, true)) => Some((
                Settling {
                    control: seen.title(CONTROL).to_owned(),
                },
                None,
            )),
            Some((true, false)) => fail("Page.setWebLifecycleState frozen was refused".into()),
            _ => None,
        },
        Settling { control } if seen.title(CONTROL) != control => Some((
            Frozen {
                frozen: seen.title(FROZEN).to_owned(),
                control: seen.title(CONTROL).to_owned(),
                ticks: 0,
            },
            None,
        )),
        Frozen { frozen, .. } if seen.title(FROZEN) != frozen => fail(format!(
            "the frozen tab ran its timers: {frozen:?} became {:?}",
            seen.title(FROZEN)
        )),
        Frozen {
            frozen,
            control,
            ticks,
        } if seen.title(CONTROL) != control => {
            let ticks = ticks + 1;
            if ticks >= CONTROL_TICKS {
                Some((
                    Thawing {
                        frozen: frozen.clone(),
                    },
                    Some(Action::Freeze(FROZEN, false)),
                ))
            } else {
                Some((
                    Frozen {
                        frozen: frozen.clone(),
                        control: seen.title(CONTROL).to_owned(),
                        ticks,
                    },
                    None,
                ))
            }
        }
        Thawing { frozen } => match seen.frozen(FROZEN) {
            Some((false, false)) => fail("Page.setWebLifecycleState active was refused".into()),
            Some((false, true)) if seen.title(FROZEN) != frozen => {
                Some((Solid(0), Some(Action::OpenSolid(0))))
            }
            _ => None,
        },
        Solid(i)
            if seen.one_visible()
                && seen.painted
                && seen.settled
                && seen.shown_title == solid_title(*i) =>
        {
            if *i == 0 {
                Some((Solid(1), Some(Action::OpenSolid(1))))
            } else {
                Some((Opener, Some(Action::CheckNoBleed)))
            }
        }
        Opener => Some((LinkShown, Some(Action::OpenLinkPage))),
        LinkShown if seen.one_visible() && seen.painted && seen.shown_title == LINK_TITLE => {
            Some((
                Linked {
                    tabs: seen.titles.len(),
                },
                Some(Action::ClickLink(false)),
            ))
        }
        Linked { tabs }
            if seen.titles.len() == tabs + 1
                && seen.one_visible()
                && seen.shown_title != LINK_TITLE =>
        {
            let above = seen
                .shown_index
                .and_then(|i| i.checked_sub(1))
                .map(|i| seen.title(i));
            match above {
                Some(t) if t != LINK_TITLE => Some((
                    Reverted {
                        tabs: *tabs,
                        background: false,
                    },
                    Some(Action::CloseShown),
                )),
                _ => fail(format!(
                    "the opened tab is not under another tab, the close proves nothing: {seen:?}"
                )),
            }
        }
        Reverted { tabs, background } if seen.titles.len() == *tabs && seen.one_visible() => {
            if seen.shown_title != LINK_TITLE {
                fail(format!(
                    "closing the tab a link opened showed {:?}, not its opener",
                    seen.shown_title
                ))
            } else if *background {
                Some((Done, Some(Action::Pass)))
            } else {
                Some((Background { tabs: *tabs }, Some(Action::ClickLink(true))))
            }
        }
        // the opener stays shown; the new tab is last (the opener is pinned)
        Background { tabs } if seen.titles.len() == tabs + 1 => {
            if seen.shown_title == LINK_TITLE {
                Some((Selected { tabs: *tabs }, Some(Action::Activate(*tabs))))
            } else {
                fail(format!("a middle click showed {:?}", seen.shown_title))
            }
        }
        Selected { tabs } if seen.shown_index == Some(*tabs) && seen.one_visible() => Some((
            Reverted {
                tabs: *tabs,
                background: true,
            },
            Some(Action::CloseShown),
        )),
        _ => None,
    };
    match step {
        Some((stage, action)) => (
            TabsSmoke {
                stage,
                runs: 0,
                ..s
            },
            action,
        ),
        None if s.stage == Done => (s, None),
        None if s.runs >= MAX_WORK_RUNS => {
            let why = format!(
                "stuck at {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.stage
            );
            let s = TabsSmoke {
                stage: Done,
                runs: 0,
                ..s
            };
            (s, Some(Action::Fail(why)))
        }
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
            painted: true,
            restored_from_snapshot: false,
            frozen: vec![],
            shown_title: String::new(),
            settled: true,
            shown_index: None,
        }
    }

    fn solid(i: usize, painted: bool, settled: bool) -> Seen {
        Seen {
            shown_title: solid_title(i),
            painted,
            settled,
            ..seen(&WANT, 3, 3)
        }
    }

    fn run_from(s: TabsSmoke, inputs: &[Seen]) -> (TabsSmoke, Vec<Action>) {
        inputs.iter().fold((s, vec![]), |(s, mut all), seen| {
            let (s, action) = next(s, seen, &WANT);
            all.extend(action);
            (s, all)
        })
    }

    fn run(inputs: &[Seen]) -> (TabsSmoke, Vec<Action>) {
        run_from(TabsSmoke::default(), inputs)
    }

    fn timers(a: &str, b: &str, frozen: Option<(bool, bool)>) -> Seen {
        let titles = ["b", "c", a, b];
        Seen {
            frozen: vec![None, None, frozen, None],
            ..seen(&titles, 3, 3)
        }
    }

    fn at(stage: Stage) -> TabsSmoke {
        TabsSmoke {
            stage,
            ..TabsSmoke::default()
        }
    }

    #[test]
    fn happy_path() {
        let three = seen(&WANT, 3, 3);
        let restored = Seen {
            restored_from_snapshot: true,
            ..seen(&WANT, 2, 2)
        };
        let (s, actions) = run(&[
            seen(&["a"], 1, 1),
            seen(&["a", "b", ""], 3, 3),
            three.clone(),
            three.clone(),
            three.clone(),
            // Discarded views close a few runs later.
            seen(&WANT, 3, 1),
            seen(&WANT, 1, 1),
            // The view is back, its snapshot not yet replaced.
            seen(&WANT, 2, 2),
            restored,
            seen(&["b", "c"], 2, 1),
            seen(&["b", "c"], 1, 1),
            timers("timer 0", "timer 0", None),
            timers("timer 1", "timer 2", None),
            timers("timer 2", "timer 3", Some((true, true))),
            // A tick under way as the freeze was answered lands: not a failure.
            timers("timer 3", "timer 3", Some((true, true))),
            timers("timer 3", "timer 4", Some((true, true))),
            timers("timer 3", "timer 4", Some((true, true))),
            timers("timer 3", "timer 5", Some((true, true))),
            timers("timer 3", "timer 6", Some((true, true))),
            timers("timer 3", "timer 7", Some((true, true))),
            timers("timer 3", "timer 8", Some((false, true))),
            timers("timer 4", "timer 9", Some((false, true))),
            solid(0, true, true),
            solid(1, false, false),
            solid(1, true, true),
            solid(1, true, true),
            link(&["opener", "a", "b", "c"], 0),
            link(&["opener", "a", "b", "c", ""], 4),
            link(&["opener", "a", "b", "c"], 0),
            link(&["opener", "a", "b", "c", ""], 0),
            link(&["opener", "a", "b", "c", ""], 4),
            link(&["opener", "a", "b", "c"], 0),
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
                Action::OpenTimers,
                Action::Freeze(FROZEN, true),
                Action::Freeze(FROZEN, false),
                Action::OpenSolid(0),
                Action::OpenSolid(1),
                Action::CheckNoBleed,
                Action::OpenLinkPage,
                Action::ClickLink(false),
                Action::CloseShown,
                Action::ClickLink(true),
                Action::Activate(4),
                Action::CloseShown,
                Action::Pass,
            ]
        );
    }

    /// The opener page's stage: `titles` in order, the visible one at `shown`.
    fn link(titles: &[&str], shown: usize) -> Seen {
        Seen {
            shown_title: titles[shown].to_string(),
            shown_index: Some(shown),
            ..seen(titles, titles.len(), titles.len())
        }
    }

    #[test]
    fn closing_the_opened_tab_must_show_its_opener() {
        let opened = link(&["opener", "a", "b", "c", ""], 4);
        let (s, actions) = run_from(
            at(Stage::Linked { tabs: 4 }),
            &[opened, link(&["opener", "a", "b", "c"], 3)],
        );
        assert_eq!(s.stage, Stage::Done);
        assert!(
            matches!(actions.as_slice(), [Action::CloseShown, Action::Fail(e)] if e.contains("not its opener")),
            "{actions:?}"
        );
        // a tab opened right under its opener proves nothing
        let (_, actions) = run_from(
            at(Stage::Linked { tabs: 2 }),
            &[link(&["a", "opener", ""], 2)],
        );
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("proves nothing")),
            "{actions:?}"
        );
    }

    #[test]
    fn a_frozen_tab_that_ticks_fails() {
        let (s, actions) = run_from(
            at(Stage::Freezing),
            &[
                timers("timer 2", "timer 3", Some((true, true))),
                // The baseline is taken once the control ticks after the answer.
                timers("timer 3", "timer 4", Some((true, true))),
                timers("timer 4", "timer 4", Some((true, true))),
            ],
        );
        assert_eq!(s.stage, Stage::Done);
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("ran its timers")),
            "{actions:?}"
        );
    }

    #[test]
    fn a_tick_landing_right_after_the_freeze_answer_does_not_fail() {
        let (s, actions) = run_from(
            at(Stage::Freezing),
            &[
                timers("timer 2", "timer 3", Some((true, true))),
                timers("timer 3", "timer 3", Some((true, true))),
            ],
        );
        assert_eq!(
            (s.stage, actions),
            (
                Stage::Settling {
                    control: "timer 3".into()
                },
                vec![]
            )
        );
    }

    #[test]
    fn a_refused_freeze_fails_with_the_reason() {
        let (_, actions) = run_from(
            at(Stage::Freezing),
            &[timers("timer 2", "timer 3", Some((true, false)))],
        );
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("refused")),
            "{actions:?}"
        );
    }

    #[test]
    fn the_bleed_check_waits_for_the_fresh_tab_to_settle() {
        let (s, actions) = run_from(
            at(Stage::Solid(1)),
            &[
                // still the first page, then the fresh one fading in, then painted mid-fade
                solid(0, true, true),
                solid(1, false, false),
                solid(1, true, false),
                solid(1, true, true),
            ],
        );
        assert_eq!(
            (s.stage, actions),
            (Stage::Opener, vec![Action::CheckNoBleed])
        );
    }

    #[test]
    fn a_restore_waits_for_the_snapshot_to_give_way() {
        let (s, actions) = run_from(at(Stage::Restored), &[seen(&WANT, 2, 2), seen(&WANT, 2, 2)]);
        assert_eq!((s.stage, actions), (Stage::Restored, vec![]));
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
        let unpainted = Seen {
            painted: false,
            ..seen(&WANT, 3, 3)
        };
        let (s, actions) = run(&[
            seen(&[], 0, 0),
            seen(&WANT, 3, 3),
            both,
            wrong,
            stale,
            unpainted,
        ]);
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
