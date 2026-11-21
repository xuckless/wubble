//! `--smoke settings`: open wubble://settings through its action, check the page is shown
//! (native, no view of its own) with rows from the table and scrolled by a wheel gesture that
//! starts as it opens (the pointer still, never moved over it: QA MED-06), flip one setting
//! through the toggle's
//! own callback, and read the stored value back through the store thread, queued after the
//! write. Then the search engine's dropdown is driven as a user does ([`SCRIPT`]): by pointer
//! (press the trigger, press a row, press outside), by key (Escape, Enter on the trigger, Down,
//! Enter), each step checked against what the host sees (the list open, the value the trigger
//! shows, the stored and applied engine), and the pick read back through the store thread.
//! Every wait is "until the host sees X", bounded by a count of CEF work runs.

use crate::nav_smoke::MAX_WORK_RUNS;

/// The setting the smoke flips: a fresh profile has it on.
pub const KEY: &str = "blocker.enabled";

/// The dropdown the script drives: its row's label and key.
pub const SELECT: &str = "Search engine";
pub const SELECT_KEY: &str = "search.engine";

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
    /// The page shows the dropdown (it is on screen).
    pub select_found: bool,
    /// The dropdown's list is open (the trigger says expanded).
    pub select_open: bool,
    /// The label the dropdown's trigger shows.
    pub select_shown: String,
    /// The stored search engine (`""` until one is picked).
    pub engine_stored: String,
    /// The engine the address bar searches with now (`""` until one is picked).
    pub engine_applied: String,
    /// Script actions the host has run, counted after the event loop turn that follows each.
    pub acted: u32,
}

/// Something a user does to the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Do {
    /// A left press and release on the dropdown's trigger.
    PressTrigger,
    /// ... on the row of the open list with this label.
    PressRow(&'static str),
    /// ... on the page where the list is not (the page's title).
    PressOutside,
    /// The trigger's own action (what Enter on it, or a screen reader, does).
    ActivateTrigger,
    /// A key press and release to the window.
    Press(Keys),
    /// The wheel up, far: back to the page's top.
    ScrollTop,
    /// After the animations have run, the window as a PNG named so in `WUBBLE_SETTINGS_SHOTS`
    /// (nothing without it).
    Shoot(&'static str),
}

/// What the page must show after a [`Do`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// The dropdown is on screen.
    Found,
    Open(bool),
    /// The trigger's label.
    Shows(&'static str),
    /// The stored and the applied engine are this id.
    Engine(&'static str),
}

/// The keys the script presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    Escape,
    Down,
    Enter,
}

/// One step of the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Do(Do),
    Expect(Expect),
}

use Do::*;
use Expect::*;
use Step::{Do as Act, Expect as Then};

/// The dropdown, as a user meets it. A fresh profile has no engine picked.
pub const SCRIPT: &[Step] = &[
    // the gesture that opened the page left it scrolled down
    Act(ScrollTop),
    Then(Found),
    Then(Open(false)),
    Then(Engine("")),
    // the pointer: open, pick
    Act(PressTrigger),
    Then(Open(true)),
    Act(Shoot("select-open")),
    Act(PressRow("DuckDuckGo")),
    Then(Engine("duckduckgo")),
    Then(Open(false)),
    Then(Shows("DuckDuckGo")),
    // Escape closes it and picks nothing
    Act(PressTrigger),
    Then(Open(true)),
    Act(Press(Keys::Escape)),
    Then(Open(false)),
    Then(Engine("duckduckgo")),
    // a press outside closes it and picks nothing
    Act(PressTrigger),
    Then(Open(true)),
    Act(PressOutside),
    Then(Open(false)),
    Then(Engine("duckduckgo")),
    // the trigger's action opens it with the focus on the list: Down moves off the current
    // row (DuckDuckGo, the last) and wraps to the first, Enter picks it
    Act(ActivateTrigger),
    Then(Open(true)),
    Act(Press(Keys::Down)),
    Act(Press(Keys::Enter)),
    Then(Engine("google")),
    Then(Open(false)),
    Then(Shows("Google")),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Asked for the page; waiting for it to show.
    Opening {
        views: usize,
    },
    /// Flipped the setting; waiting for the store's answer.
    Flipped,
    /// At step `at` of [`SCRIPT`]; `acting`: the action was sent when the host had run this many.
    Script {
        at: usize,
        acting: Option<u32>,
    },
    /// The script's last pick, asked of the store.
    ReadBack,
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
    /// One step of the script.
    Drive(Do),
    /// Read `key` back through the store thread.
    Read {
        key: &'static str,
    },
    Pass,
    Fail(String),
}

/// The engine the script ends on, as the store must hold it.
pub const FINAL_ENGINE: &str = "google";

pub fn next(s: SettingsSmoke, seen: &Seen) -> (SettingsSmoke, Option<Action>) {
    let to = |stage| SettingsSmoke { stage, runs: 0 };
    let fail = |why: String| Some((Stage::Done, Some(Action::Fail(why))));
    let step = match s.stage {
        Stage::Start if seen.ready && seen.blocking => {
            Some((Stage::Opening { views: seen.views }, Some(Action::Open)))
        }
        // The page is native: opening it adds no view.
        Stage::Opening { views }
            if seen.on_settings && seen.page_open && seen.rows > 0 && seen.scroll >= SCROLLED =>
        {
            if seen.views != views {
                fail(format!("the settings tab opened a view: {seen:?}"))
            } else {
                Some((
                    Stage::Flipped,
                    Some(Action::Flip {
                        key: KEY,
                        value: "0",
                    }),
                ))
            }
        }
        Stage::Flipped => match seen.stored.as_deref() {
            Some("0") if !seen.blocking => Some((
                Stage::Script {
                    at: 0,
                    acting: None,
                },
                None,
            )),
            Some(other) => fail(format!(
                "stored {KEY} = {other:?}, blocking {}",
                seen.blocking
            )),
            None => None,
        },
        Stage::Script { at, acting } => match (SCRIPT.get(at), acting) {
            (None, _) => Some((Stage::ReadBack, Some(Action::Read { key: SELECT_KEY }))),
            (Some(Step::Do(act)), None) => Some((
                Stage::Script {
                    at,
                    acting: Some(seen.acted),
                },
                Some(Action::Drive(*act)),
            )),
            (Some(Step::Do(_)), Some(before)) if seen.acted > before => Some((
                Stage::Script {
                    at: at + 1,
                    acting: None,
                },
                None,
            )),
            (Some(Step::Expect(want)), _) if met(*want, seen) => Some((
                Stage::Script {
                    at: at + 1,
                    acting: None,
                },
                None,
            )),
            _ => None,
        },
        Stage::ReadBack => match seen.stored.as_deref() {
            Some(FINAL_ENGINE) => Some((Stage::Done, Some(Action::Pass))),
            Some(other) => fail(format!(
                "stored {SELECT_KEY} = {other:?} after the script, wanted {FINAL_ENGINE:?}"
            )),
            None => None,
        },
        _ => None,
    };
    match step {
        Some((stage, action)) => (to(stage), action),
        None if s.stage == Stage::Done => (s, None),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Stage::Done),
            Some(Action::Fail(format!(
                "stuck at {:?}{} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.stage,
                match s.stage {
                    Stage::Script { at, .. } => format!(" ({:?})", SCRIPT.get(at)),
                    _ => String::new(),
                }
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

/// Whether the page shows what `want` says.
fn met(want: Expect, seen: &Seen) -> bool {
    match want {
        Found => seen.select_found,
        Open(open) => seen.select_found && seen.select_open == open,
        Shows(label) => seen.select_shown == label,
        Engine(id) => seen.engine_stored == id && seen.engine_applied == id,
    }
}

/// The script's hands and eyes on the real window: the dropdown found by what a screen reader
/// sees of it, pointer events through the window's own event path, as a mouse would send them.
pub mod drive {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};
    use slint::LogicalPosition;
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    use wubble_ui::AppWindow;

    use super::{Do, Keys, SELECT};

    fn trigger(window: &AppWindow) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(window, SELECT)
            .find(|e| e.accessible_role() == Some(AccessibleRole::Combobox))
    }

    fn centre(e: &ElementHandle) -> LogicalPosition {
        let (p, s) = (e.absolute_position(), e.size());
        LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
    }

    /// Whether the dropdown's list is open and the label its trigger shows; `None` until the
    /// page has the dropdown.
    pub fn look(window: &AppWindow) -> Option<(bool, String)> {
        let t = trigger(window)?;
        Some((
            t.accessible_expanded() == Some(true),
            t.accessible_value()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        ))
    }

    fn click(window: &AppWindow, at: LogicalPosition) {
        let w = slint::ComponentHandle::window(window);
        let button = PointerEventButton::Left;
        w.dispatch_event(WindowEvent::PointerMoved { position: at });
        w.dispatch_event(WindowEvent::PointerPressed {
            position: at,
            button,
        });
        w.dispatch_event(WindowEvent::PointerReleased {
            position: at,
            button,
        });
    }

    /// Does one step; the reason when the page has nothing to act on.
    pub fn run(window: &AppWindow, act: Do, done: impl FnOnce() + 'static) -> Result<(), String> {
        let w = slint::ComponentHandle::window(window);
        let missing = || format!("no {SELECT} dropdown on the page");
        match act {
            Do::PressTrigger => click(window, centre(&trigger(window).ok_or_else(missing)?)),
            Do::ActivateTrigger => trigger(window)
                .ok_or_else(missing)?
                .invoke_accessible_default_action(),
            Do::PressRow(label) => {
                let row = ElementHandle::find_by_accessible_label(window, label)
                    .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
                    .ok_or_else(|| format!("no row {label:?} in an open list"))?;
                click(window, centre(&row));
            }
            Do::PressOutside => {
                // left of the trigger, in its own row: the open list's scrim lies over the page
                let at = centre(&trigger(window).ok_or_else(missing)?);
                click(window, LogicalPosition::new(at.x - 300.0, at.y));
            }
            Do::ScrollTop => {
                let size = w.size().to_logical(w.scale_factor());
                w.dispatch_event(WindowEvent::PointerScrolled {
                    position: LogicalPosition::new(size.width * 0.6, size.height / 2.0),
                    delta_x: 0.0,
                    delta_y: 5000.0,
                });
            }
            Do::Shoot(name) => {
                // later: the list's grow and its rows' stagger take a quarter of a second
                let weak = slint::ComponentHandle::as_weak(window);
                slint::Timer::single_shot(std::time::Duration::from_millis(900), move || {
                    if let (Some(window), Ok(dir)) =
                        (weak.upgrade(), std::env::var("WUBBLE_SETTINGS_SHOTS"))
                    {
                        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
                        if let Err(e) = crate::shot::take_to(&window, &path) {
                            eprintln!("smoke: shot {name} failed: {e}");
                        }
                    }
                    done();
                });
                return Ok(());
            }
            Do::Press(keys) => {
                let text: slint::SharedString = match keys {
                    Keys::Escape => Key::Escape,
                    Keys::Down => Key::DownArrow,
                    Keys::Enter => Key::Return,
                }
                .into();
                w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                w.dispatch_event(WindowEvent::KeyReleased { text });
            }
        }
        done();
        Ok(())
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
        let (s, a) = next(s, &done);
        assert_eq!(a, None);
        assert_eq!(
            s.stage,
            Stage::Script {
                at: 0,
                acting: None
            },
            "the script follows the flip"
        );
    }

    /// A page that does what a user would see: the script's every step against it, the way
    /// the host runs them.
    #[derive(Default)]
    struct Page {
        seen: Seen,
        row: usize,
    }

    impl Page {
        fn press(&mut self, act: Do) {
            const NAMES: [(&str, &str); 2] = [("google", "Google"), ("duckduckgo", "DuckDuckGo")];
            let s = &mut self.seen;
            let pick = |id: &str, s: &mut Seen| {
                s.engine_stored = id.into();
                s.engine_applied = id.into();
                s.select_shown = NAMES.iter().find(|n| n.0 == id).unwrap().1.into();
                s.select_open = false;
            };
            match act {
                Do::PressTrigger => s.select_open = true,
                Do::ActivateTrigger => {
                    s.select_open = true;
                    self.row = NAMES
                        .iter()
                        .position(|n| n.0 == s.engine_stored)
                        .unwrap_or(0);
                }
                Do::PressRow(label) => {
                    let id = NAMES.iter().find(|n| n.1 == label).unwrap().0;
                    pick(id, s);
                }
                Do::ScrollTop => s.select_found = true,
                Do::Shoot(_) => {}
                Do::PressOutside | Do::Press(Keys::Escape) => s.select_open = false,
                Do::Press(Keys::Down) => self.row = (self.row + 1) % NAMES.len(),
                Do::Press(Keys::Enter) => pick(NAMES[self.row].0, s),
            }
            s.acted += 1;
        }
    }

    fn run_script(mut page: Page) -> (SettingsSmoke, Option<Action>) {
        let mut s = SettingsSmoke {
            stage: Stage::Script {
                at: 0,
                acting: None,
            },
            runs: 0,
        };
        loop {
            let (next_state, action) = next(s, &page.seen);
            s = next_state;
            match action {
                Some(Action::Drive(act)) => page.press(act),
                Some(Action::Read { .. }) => {
                    page.seen.stored = Some(page.seen.engine_stored.clone())
                }
                Some(other) => return (s, Some(other)),
                None => {}
            }
        }
    }

    #[test]
    fn the_script_walks_a_page_that_behaves() {
        let (_, last) = run_script(Page::default());
        assert_eq!(last, Some(Action::Pass));
    }

    #[test]
    fn the_script_ends_on_the_engine_it_expects() {
        let last_engine = SCRIPT.iter().rev().find_map(|step| match step {
            Step::Expect(Expect::Engine(id)) => Some(*id),
            _ => None,
        });
        assert_eq!(last_engine, Some(FINAL_ENGINE));
    }

    #[test]
    fn the_script_fails_on_a_list_that_stays_open() {
        // a page whose list ignores Escape: the step waits, then the budget fails it
        let mut page = Page::default();
        let mut s = SettingsSmoke {
            stage: Stage::Script {
                at: 0,
                acting: None,
            },
            runs: 0,
        };
        loop {
            let (next_state, action) = next(s, &page.seen);
            s = next_state;
            match action {
                Some(Action::Drive(Do::Press(Keys::Escape))) => page.seen.acted += 1,
                Some(Action::Drive(act)) => page.press(act),
                Some(Action::Fail(why)) => {
                    assert!(why.contains("Expect(Open(false))"), "{why}");
                    break;
                }
                Some(Action::Pass) => panic!("passed a list that ignores Escape"),
                _ => {}
            }
        }
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
