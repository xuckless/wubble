//! `--smoke settings`: open wubble://settings through its action, check the page is shown
//! (native, no view of its own) with rows from the table and scrolled by a wheel gesture that
//! starts as it opens (the pointer still, never moved over it: QA MED-06), flip one setting
//! through the toggle's
//! own callback, and read the stored value back through the store thread, queued after the
//! write. Every wait is "until the host sees X", bounded by a count of CEF work runs.

use crate::nav_smoke::MAX_WORK_RUNS;

/// The setting the smoke flips: a fresh profile has it on.
pub const KEY: &str = "blocker.enabled";

/// The wheel gesture: longer than the scroller's 160 ms budget for pushing past an end.
pub const TICKS: u32 = 12;
pub const TICK: f32 = 100.0;
pub const TICK_EVERY: std::time::Duration = std::time::Duration::from_millis(30);
/// How far the gesture must have scrolled the page: most of it.
const SCROLLED: f32 = TICK * TICKS as f32 * 0.75;

/// What the host sees.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    /// The window can open views (the frame rate is known).
    pub ready: bool,
    /// The visible tab is on wubble://settings.
    pub on_settings: bool,
    /// The shell shows the settings page.
    pub page_open: bool,
    /// Generated setting rows on the page.
    pub rows: usize,
    /// Views the engine holds.
    pub views: usize,
    /// The stored value of [`KEY`], read back after the flip.
    pub stored: Option<String>,
    /// The blocker is on.
    pub blocking: bool,
    /// The page's scroll position, logical px.
    pub scroll: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Asked for the page; waiting for it to show.
    Opening {
        views: usize,
    },
    /// Flipped the setting; waiting for the store's answer.
    Flipped,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsSmoke {
    pub stage: Stage,
    runs: u32,
}

impl Default for SettingsSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Run `Action::OpenSettings`, and start a wheel gesture down over the page's middle: one
    /// [`TICK`] px tick every [`TICK_EVERY`], [`TICKS`] of them.
    Open,
    /// Send `value` for `key` through the page's change callback, then read the store back.
    Flip {
        key: &'static str,
        value: &'static str,
    },
    Pass,
    Fail(String),
}

pub fn next(s: SettingsSmoke, seen: &Seen) -> (SettingsSmoke, Option<Action>) {
    let to = |stage| SettingsSmoke { stage, runs: 0 };
    let step = match s.stage {
        Stage::Start if seen.ready && seen.blocking => {
            Some((Stage::Opening { views: seen.views }, Action::Open))
        }
        // The page is native: opening it adds no view.
        Stage::Opening { views }
            if seen.on_settings && seen.page_open && seen.rows > 0 && seen.scroll >= SCROLLED =>
        {
            if seen.views != views {
                Some((
                    Stage::Done,
                    Action::Fail(format!("the settings tab opened a view: {seen:?}")),
                ))
            } else {
                Some((
                    Stage::Flipped,
                    Action::Flip {
                        key: KEY,
                        value: "0",
                    },
                ))
            }
        }
        Stage::Flipped => match seen.stored.as_deref() {
            Some("0") if !seen.blocking => Some((Stage::Done, Action::Pass)),
            Some(other) => Some((
                Stage::Done,
                Action::Fail(format!(
                    "stored {KEY} = {other:?}, blocking {}",
                    seen.blocking
                )),
            )),
            None => None,
        },
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
            SettingsSmoke {
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

    fn ready() -> Seen {
        Seen {
            ready: true,
            views: 1,
            blocking: true,
            ..Seen::default()
        }
    }

    #[test]
    fn opens_flips_and_reads_back() {
        let (s, a) = next(SettingsSmoke::default(), &ready());
        assert_eq!(a, Some(Action::Open));
        let (s, a) = next(s, &ready());
        assert_eq!(a, None, "waits for the page");
        let open = Seen {
            on_settings: true,
            page_open: true,
            rows: 12,
            ..ready()
        };
        let (s, a) = next(s, &open);
        assert_eq!(a, None, "waits for the gesture to scroll the page");
        let scrolled = Seen {
            scroll: SCROLLED,
            ..open.clone()
        };
        let (s, a) = next(s, &scrolled);
        assert_eq!(
            a,
            Some(Action::Flip {
                key: KEY,
                value: "0"
            })
        );
        let (s, a) = next(s, &scrolled);
        assert_eq!(a, None, "waits for the store");
        let done = Seen {
            stored: Some("0".into()),
            blocking: false,
            ..open
        };
        assert_eq!(next(s, &done).1, Some(Action::Pass));
    }

    #[test]
    fn a_view_for_the_page_or_a_wrong_value_fails() {
        let s = SettingsSmoke {
            stage: Stage::Opening { views: 1 },
            runs: 0,
        };
        let with_view = Seen {
            on_settings: true,
            page_open: true,
            rows: 3,
            views: 2,
            scroll: SCROLLED,
            ..ready()
        };
        assert!(matches!(next(s, &with_view).1, Some(Action::Fail(_))));
        let s = SettingsSmoke {
            stage: Stage::Flipped,
            runs: 0,
        };
        let kept = Seen {
            stored: Some("1".into()),
            ..ready()
        };
        assert!(matches!(next(s, &kept).1, Some(Action::Fail(_))));
    }

    #[test]
    fn gives_up_after_the_run_budget() {
        let mut s = SettingsSmoke::default();
        let mut last = None;
        for _ in 0..=MAX_WORK_RUNS {
            (s, last) = next(s, &Seen::default());
        }
        assert!(matches!(last, Some(Action::Fail(_))));
    }
}
