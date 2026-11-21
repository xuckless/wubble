//! Closed tabs coming back: reopen-closed-tab (Mod+Shift+T, any close on the recently-closed
//! list) and Mod+Z after a close (undo close tab), one machine for both.
//!
//! The rule is Safari's Edit > Undo Close Tab (Cmd+Z, Apple's responder-chain undo; behaviour
//! only, no code): the key is the focused text field's undo first, always; only when no field
//! has the keys and the page left the key alone does it reopen the last tab closed in this
//! session, and pressed again the one before, back through every close of the session.
//! Switching tabs or Spaces and navigating end nothing; closes from an earlier run are not
//! undoable (they stay on the recently-closed list for Mod+Shift+T). Who has the keys is the
//! router's (`input::key_route` keeps the key in a shell field); whether the page's focused
//! element is editable comes with the page's unhandled-key report (CEF
//! `CefKeyEvent::focus_on_editable_field`), so a field with nothing to undo keeps the key too.
//!
//! The recently-closed list itself is the store's (persisted, newest first); this machine
//! keeps, for the closes of this run still on it, where each tab sat, so the one restore
//! (`Host::reopen_closed`) puts it back in its Space at its place (pinned, in its folder). A
//! tab an agent session drives or an agent Space's is never on the list
//! (`Model::closes_into_recent`), so never here.

use crate::model::{Command, Dest, Model, Place};
use crate::space::SpaceId;
use crate::tab::TabId;

/// Most entries the recently-closed list keeps; older ones are dropped on a push. Chromium's
/// `TabRestoreService` keeps 25 (`kMaxEntries`).
pub const RECENTLY_CLOSED_MAX: usize = 25;

/// One close of this run, as the store recorded it (`url` and `closed_at_ms` name the entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    pub url: String,
    pub closed_at_ms: i64,
    /// Its Space and its place there, before it closed.
    pub space: SpaceId,
    pub place: Dest,
}

impl Closed {
    /// `tab` as it sits in `model` before `command` (about to run) closes it, if the close goes
    /// on the recently-closed list as far as the model knows (`Model::closes_into_recent`: not
    /// an agent Space's being deleted, not a peek's, not an empty tab).
    pub fn of(model: &Model, tab: TabId, command: &Command, closed_at_ms: i64) -> Option<Closed> {
        if !model.closes_into_recent(tab, command) {
            return None;
        }
        let url = model.tabs.get(&tab)?.url.clone();
        let (space, place) = model.location(tab)?;
        let place = match place {
            Place::Pinned(index) => Dest::Pinned { space, index },
            Place::Normal(index) => Dest::Normal { space, index },
            Place::Folder(folder, index) => Dest::Folder { folder, index },
        };
        Some(Closed {
            url,
            closed_at_ms,
            space,
            place,
        })
    }
}

/// This run's closes still on the list, oldest first, and how many list entries the host has
/// asked the store for and not yet heard back about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Undo {
    closes: Vec<Closed>,
    popping: usize,
}

impl Undo {
    /// Closes a press of Mod+Z could still reopen.
    pub fn undoable(&self) -> usize {
        self.closes.len().saturating_sub(self.popping)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A tab went on the recently-closed list.
    Closed(Closed),
    /// Mod+Shift+T or the command bar's "Reopen closed tab": the newest entry, whichever run
    /// closed it.
    Reopen,
    /// Mod+Z that no shell field took and the page left alone; `editable`: the page's focus is
    /// on an editable element (CEF's `focus_on_editable_field`).
    Key { editable: bool },
    /// The command bar's "Undo close tab": no field to hand the key to.
    Command,
    /// The store took its newest entry off the list: this one (its URL and close time), or none.
    Popped(Option<(String, i64)>),
    /// History was deleted, and with it entries of the list: what is left is not known here.
    Forget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Take the newest entry off the store's list (an [`Event::Popped`] answers).
    Pop,
    /// Open the popped entry: in `space` at `place` when this run closed it, else in the active
    /// Space, last, as any reopen.
    Restore {
        space: Option<SpaceId>,
        place: Option<Dest>,
    },
    /// The key is the page's: its focused frame's undo, as Mod+Z without a close.
    FieldUndo,
    /// Asked from the command bar with nothing to undo.
    NothingToUndo,
}

pub fn transition(mut state: Undo, event: Event) -> (Undo, Vec<Effect>) {
    let effects = match event {
        Event::Closed(closed) => {
            state.closes.push(closed);
            if state.closes.len() > RECENTLY_CLOSED_MAX {
                state.closes.remove(0);
            }
            vec![]
        }
        Event::Reopen => {
            state.popping += 1;
            vec![Effect::Pop]
        }
        Event::Key { editable: true } => vec![Effect::FieldUndo],
        Event::Key { editable: false } | Event::Command if state.undoable() > 0 => {
            state.popping += 1;
            vec![Effect::Pop]
        }
        Event::Key { editable: false } => vec![Effect::FieldUndo],
        Event::Command => vec![Effect::NothingToUndo],
        Event::Popped(entry) => {
            state.popping = state.popping.saturating_sub(1);
            match entry {
                None => vec![],
                Some((url, at)) => {
                    let this_run = state
                        .closes
                        .iter()
                        .rposition(|c| c.url == url && c.closed_at_ms == at)
                        .map(|i| state.closes.remove(i));
                    vec![Effect::Restore {
                        space: this_run.as_ref().map(|c| c.space),
                        place: this_run.map(|c| c.place),
                    }]
                }
            }
        }
        Event::Forget => {
            state.closes.clear();
            vec![]
        }
    };
    (state, effects)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(url: &str, at: i64, index: usize) -> Closed {
        let space = SpaceId(1);
        Closed {
            url: url.to_owned(),
            closed_at_ms: at,
            space,
            place: Dest::Normal { space, index },
        }
    }

    fn run(state: Undo, events: impl IntoIterator<Item = Event>) -> (Undo, Vec<Effect>) {
        events.into_iter().fold((state, vec![]), |(s, mut all), e| {
            let (s, effects) = transition(s, e);
            all.extend(effects);
            (s, all)
        })
    }

    fn popped(c: &Closed) -> Event {
        Event::Popped(Some((c.url.clone(), c.closed_at_ms)))
    }

    #[test]
    fn a_close_then_the_key_restores_it_at_its_place() {
        let a = closed("https://a.test/", 10, 2);
        let (s, effects) = run(
            Undo::default(),
            [Event::Closed(a.clone()), Event::Key { editable: false }],
        );
        assert_eq!(effects, [Effect::Pop]);
        let (s, effects) = transition(s, popped(&a));
        assert_eq!(
            effects,
            [Effect::Restore {
                space: Some(a.space),
                place: Some(a.place)
            }]
        );
        assert_eq!(s, Undo::default(), "nothing left to undo");
    }

    #[test]
    fn nothing_closed_passes_the_key_on() {
        let (s, effects) = transition(Undo::default(), Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo]);
        assert_eq!(s, Undo::default());
        let (_, effects) = transition(s, Event::Command);
        assert_eq!(effects, [Effect::NothingToUndo]);
    }

    #[test]
    fn an_editable_focus_keeps_the_key_whatever_was_closed() {
        let a = closed("https://a.test/", 10, 0);
        let (s, effects) = run(
            Undo::default(),
            [Event::Closed(a), Event::Key { editable: true }],
        );
        assert_eq!(effects, [Effect::FieldUndo]);
        assert_eq!(s.undoable(), 1, "the close stays undoable");
    }

    #[test]
    fn presses_step_back_through_several_closes_newest_first() {
        let (a, b) = (
            closed("https://a.test/", 10, 1),
            closed("https://b.test/", 20, 1),
        );
        let (s, effects) = run(
            Undo::default(),
            [
                Event::Closed(a.clone()),
                Event::Closed(b.clone()),
                Event::Key { editable: false },
                // pressed again before the store answered: the next one, not the same twice
                Event::Key { editable: false },
                Event::Key { editable: false },
            ],
        );
        assert_eq!(effects, [Effect::Pop, Effect::Pop, Effect::FieldUndo]);
        let (s, effects) = run(s, [popped(&b), popped(&a)]);
        assert_eq!(
            effects,
            [
                Effect::Restore {
                    space: Some(b.space),
                    place: Some(b.place)
                },
                Effect::Restore {
                    space: Some(a.space),
                    place: Some(a.place)
                }
            ]
        );
        assert_eq!(s.undoable(), 0);
    }

    #[test]
    fn reopen_takes_this_runs_close_and_its_place_too() {
        let a = closed("https://a.test/", 10, 3);
        let (s, effects) = run(Undo::default(), [Event::Closed(a.clone()), Event::Reopen]);
        assert_eq!(effects, [Effect::Pop]);
        assert_eq!(s.undoable(), 0, "the close is being reopened");
        let (s, effects) = transition(s, popped(&a));
        assert_eq!(
            effects,
            [Effect::Restore {
                space: Some(a.space),
                place: Some(a.place)
            }]
        );
        let (_, effects) = transition(s, Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo], "nothing left for the key");
    }

    #[test]
    fn an_earlier_runs_close_reopens_only_by_reopen_and_without_a_place() {
        // the list holds an entry from before this run: the key leaves it alone
        let (s, effects) = transition(Undo::default(), Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo]);
        let (s, effects) = run(
            s,
            [
                Event::Reopen,
                Event::Popped(Some(("https://old.test/".into(), 1))),
            ],
        );
        assert_eq!(
            effects,
            [
                Effect::Pop,
                Effect::Restore {
                    space: None,
                    place: None
                }
            ]
        );
        let (_, effects) = run(s, [Event::Reopen, Event::Popped(None)]);
        assert_eq!(effects, [Effect::Pop], "an empty list restores nothing");
    }

    #[test]
    fn deleted_history_ends_the_undo() {
        let a = closed("https://a.test/", 10, 0);
        let (s, _) = run(Undo::default(), [Event::Closed(a), Event::Forget]);
        assert_eq!(s.undoable(), 0);
        let (_, effects) = transition(s, Event::Key { editable: false });
        assert_eq!(effects, [Effect::FieldUndo]);
    }

    #[test]
    fn the_run_keeps_as_many_closes_as_the_list() {
        let events = (0..RECENTLY_CLOSED_MAX as i64 + 5)
            .map(|i| Event::Closed(closed("https://a.test/", i, 0)))
            .collect::<Vec<_>>();
        let (s, _) = run(Undo::default(), events);
        assert_eq!(s.undoable(), RECENTLY_CLOSED_MAX);
    }

    fn open(m: Model, space: Option<SpaceId>, url: &str) -> (Model, TabId) {
        let command = match space {
            Some(space) => Command::OpenInSpace {
                space,
                url: url.into(),
            },
            None => Command::NewTab {
                url: url.into(),
                container: None,
                opener: None,
            },
        };
        let (m, _) = crate::model::apply(m, command);
        let tab = *m.tabs.keys().max().unwrap();
        (m, tab)
    }

    #[test]
    fn a_tab_closes_with_its_place_and_an_agent_space_s_not_at_all() {
        let m = Model::new(std::time::Instant::now());
        let (m, _) = open(m, None, "https://a.test/");
        let (m, b) = open(m, None, "https://b.test/");
        let space = m.active_space();
        let c = Closed::of(&m, b, &Command::CloseTab(b), 5).unwrap();
        assert_eq!(
            (c.url.as_str(), c.space, c.place),
            ("https://b.test/", space, Dest::Normal { space, index: 1 })
        );
        let (m, _) = crate::model::apply(
            m,
            Command::NewSpace {
                name: "agent".into(),
                tint: crate::space::Rgb(1, 2, 3),
                container: crate::space::ContainerId::DEFAULT,
            },
        );
        let agent = m.spaces.last().unwrap().id;
        let (m, _) = crate::model::apply(m, Command::MarkAgentSpace(agent));
        let (m, t) = open(m, Some(agent), "https://agent.test/");
        // a session's teardown: never on the list, so never undoable
        assert_eq!(Closed::of(&m, t, &Command::DeleteSpace(agent), 5), None);
    }
}
