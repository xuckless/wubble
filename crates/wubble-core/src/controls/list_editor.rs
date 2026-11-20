//! The list editor's interaction (`kit/controls/list-editor.slint`, wubble-only: the design has
//! no list editor; it follows macOS's add / remove lists): which row is selected, and whether a
//! draft row (a new one at the end, or a row edited in place) is open, waiting for its owner's
//! answer, or confirming a removal. One enum, one pure [`transition`]; the widget runs the same
//! table in Slint (it cannot call Rust: no global of its own reaches the host), and
//! `wubble-ui/tests/list_editor.rs` drives the widget through these tests' scripts to keep the
//! two the same.
//!
//! The owner answers a submitted draft: accepted (the list now holds it; the draft closes on
//! that row) or refused (the draft stays open, its note in coral). Rows are indices into the
//! list as the owner shows it now; `len` is its length.

/// What the editor shows besides its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Rows only.
    Browse,
    /// The draft row is open: a new row at the end (`row: None`) or row `i` edited in place.
    /// `refused`: the owner turned the last submission down (its note shows).
    Draft { row: Option<usize>, refused: bool },
    /// The draft went to the owner; its answer closes it or opens it again.
    Sent { row: Option<usize> },
    /// The owner was asked to add a row of its own making (`Adds::Owner`); the list's next
    /// row is edited in place when it arrives.
    Asked,
    /// "−" was pressed once on the selected row; a second press removes it.
    Confirm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListEditor {
    pub mode: Mode,
    pub selected: Option<usize>,
    /// The list's length the editor last saw.
    pub len: usize,
}

/// What "+" does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adds {
    /// Nothing ("+" is disabled).
    None,
    /// Opens a draft row at the end.
    Draft,
    /// Asks the owner to add a row (a Space with its default name); that row is then edited.
    Owner,
}

/// What the owner lets the editor do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub adds: Adds,
    pub removes: bool,
    /// Enter and a double click edit a row in place.
    pub edits: bool,
    /// "−" needs a second press.
    pub confirm_remove: bool,
    /// Rows that always stay: "−" is disabled at this length or below (the last Space).
    pub keep: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The list's length changed.
    Rows(usize),
    /// A click on row `i`.
    Pick(usize),
    Up,
    Down,
    /// "+".
    Add,
    /// A double click on row `i`, or Enter (`None`: the selected row).
    Edit(Option<usize>),
    /// "−", Delete or Backspace.
    Remove,
    /// Enter in the draft.
    Commit,
    /// Typing in the draft (its refusal note goes).
    Typed,
    /// Escape.
    Cancel,
    /// The owner's answer to the draft it was sent; `len` is the list's length with it.
    Answer {
        accepted: bool,
        len: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Remove row `i`.
    Remove(usize),
    /// Send the draft: a new row (`None`) or a replacement for row `i`.
    Submit(Option<usize>),
    /// Ask the owner to add a row of its own.
    AskAdd,
}

impl ListEditor {
    pub fn new(len: usize) -> Self {
        Self {
            mode: Mode::Browse,
            selected: None,
            len,
        }
    }

    /// Whether "−" is enabled.
    pub fn can_remove(&self, caps: Caps) -> bool {
        caps.removes
            && self.len > caps.keep
            && self.selected.is_some()
            && matches!(self.mode, Mode::Browse | Mode::Confirm)
    }

    /// Whether "+" is enabled.
    pub fn can_add(&self, caps: Caps) -> bool {
        caps.adds != Adds::None && matches!(self.mode, Mode::Browse | Mode::Confirm)
    }
}

fn browse(selected: Option<usize>, len: usize) -> ListEditor {
    ListEditor {
        mode: Mode::Browse,
        selected,
        len,
    }
}

/// One step: the editor after `event`, and what it asks of its owner.
pub fn transition(s: ListEditor, caps: Caps, event: Event) -> (ListEditor, Option<Effect>) {
    let idle = matches!(s.mode, Mode::Browse | Mode::Confirm);
    let edit = |i: usize| ListEditor {
        mode: Mode::Draft {
            row: Some(i),
            refused: false,
        },
        selected: Some(i),
        len: s.len,
    };
    match event {
        Event::Rows(len) => {
            let selected = s.selected.filter(|&i| i < len).or(match s.selected {
                Some(_) => len.checked_sub(1),
                None => None,
            });
            let next = match s.mode {
                Mode::Asked if len > s.len => {
                    let row = len - 1;
                    return (
                        ListEditor {
                            mode: if caps.edits {
                                Mode::Draft {
                                    row: Some(row),
                                    refused: false,
                                }
                            } else {
                                Mode::Browse
                            },
                            selected: Some(row),
                            len,
                        },
                        None,
                    );
                }
                Mode::Draft { row: Some(i), .. } | Mode::Sent { row: Some(i) } if i >= len => {
                    Mode::Browse
                }
                Mode::Confirm if selected != s.selected || len != s.len => Mode::Browse,
                m => m,
            };
            (
                ListEditor {
                    mode: next,
                    selected,
                    len,
                },
                None,
            )
        }
        Event::Pick(i) if i < s.len && !matches!(s.mode, Mode::Sent { .. }) => {
            (browse(Some(i), s.len), None)
        }
        Event::Up | Event::Down if idle && s.len > 0 => {
            let last = s.len - 1;
            let down = event == Event::Down;
            let selected = match s.selected {
                None if down => 0,
                None => last,
                Some(i) if down => (i + 1).min(last),
                Some(i) => i.saturating_sub(1),
            };
            (browse(Some(selected), s.len), None)
        }
        Event::Add if idle => match caps.adds {
            Adds::None => (s, None),
            Adds::Draft => (
                ListEditor {
                    mode: Mode::Draft {
                        row: None,
                        refused: false,
                    },
                    ..s
                },
                None,
            ),
            Adds::Owner => (
                ListEditor {
                    mode: Mode::Asked,
                    ..s
                },
                Some(Effect::AskAdd),
            ),
        },
        Event::Edit(row) if idle && caps.edits => match row.or(s.selected) {
            Some(i) if i < s.len => (edit(i), None),
            _ => (s, None),
        },
        Event::Remove if s.can_remove(caps) => {
            let i = s.selected.expect("can_remove has a selection");
            if caps.confirm_remove && s.mode == Mode::Browse {
                (
                    ListEditor {
                        mode: Mode::Confirm,
                        ..s
                    },
                    None,
                )
            } else {
                (browse(Some(i), s.len), Some(Effect::Remove(i)))
            }
        }
        Event::Commit => match s.mode {
            Mode::Draft { row, .. } => (
                ListEditor {
                    mode: Mode::Sent { row },
                    ..s
                },
                Some(Effect::Submit(row)),
            ),
            _ => (s, None),
        },
        Event::Typed => match s.mode {
            Mode::Draft { row, refused: true } => (
                ListEditor {
                    mode: Mode::Draft {
                        row,
                        refused: false,
                    },
                    ..s
                },
                None,
            ),
            _ => (s, None),
        },
        Event::Cancel => match s.mode {
            Mode::Draft { .. } | Mode::Confirm | Mode::Asked => (browse(s.selected, s.len), None),
            _ => (s, None),
        },
        Event::Answer { accepted, len } => match s.mode {
            Mode::Sent { row } if accepted => {
                let selected = row.or(len.checked_sub(1)).filter(|&i| i < len);
                (browse(selected, len), None)
            }
            Mode::Sent { row } => (
                ListEditor {
                    mode: Mode::Draft { row, refused: true },
                    len,
                    ..s
                },
                None,
            ),
            _ => (s, None),
        },
        _ => (s, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: Caps = Caps {
        adds: Adds::Draft,
        removes: true,
        edits: true,
        confirm_remove: false,
        keep: 0,
    };
    const SPACES: Caps = Caps {
        adds: Adds::Owner,
        removes: true,
        edits: true,
        confirm_remove: true,
        keep: 1,
    };
    const READ_ONLY: Caps = Caps {
        adds: Adds::None,
        removes: true,
        edits: false,
        confirm_remove: false,
        keep: 0,
    };

    fn run(caps: Caps, len: usize, events: &[Event]) -> (ListEditor, Vec<Effect>) {
        let mut s = ListEditor::new(len);
        let mut out = vec![];
        for &e in events {
            let (next, fx) = transition(s, caps, e);
            s = next;
            out.extend(fx);
        }
        (s, out)
    }

    #[test]
    fn keys_move_the_selection_without_wrapping() {
        let (s, _) = run(LIST, 3, &[Event::Down]);
        assert_eq!(
            s.selected,
            Some(0),
            "Down with nothing selected picks the first"
        );
        let (s, _) = run(LIST, 3, &[Event::Up]);
        assert_eq!(
            s.selected,
            Some(2),
            "Up with nothing selected picks the last"
        );
        let (s, _) = run(
            LIST,
            3,
            &[Event::Down, Event::Down, Event::Down, Event::Down],
        );
        assert_eq!(s.selected, Some(2));
        let (s, _) = run(LIST, 3, &[Event::Pick(1), Event::Up, Event::Up]);
        assert_eq!(s.selected, Some(0));
        let (s, _) = run(LIST, 0, &[Event::Down]);
        assert_eq!(s.selected, None, "an empty list selects nothing");
    }

    #[test]
    fn remove_needs_a_selection_and_the_list_keeps_its_place() {
        let (s, fx) = run(LIST, 3, &[Event::Remove]);
        assert!(fx.is_empty(), "nothing selected: − is disabled");
        assert!(!s.can_remove(LIST));
        let (s, fx) = run(LIST, 3, &[Event::Pick(2), Event::Remove, Event::Rows(2)]);
        assert_eq!(fx, [Effect::Remove(2)]);
        assert_eq!(
            s.selected,
            Some(1),
            "the last row gone: the new last is selected"
        );
        let (s, fx) = run(LIST, 3, &[Event::Pick(0), Event::Remove, Event::Rows(2)]);
        assert_eq!(fx, [Effect::Remove(0)]);
        assert_eq!(
            s.selected,
            Some(0),
            "the next row moves up under the selection"
        );
        let (s, _) = run(LIST, 1, &[Event::Pick(0), Event::Remove, Event::Rows(0)]);
        assert_eq!(s.selected, None);
    }

    #[test]
    fn plus_opens_a_draft_enter_sends_it_and_the_answer_closes_it() {
        let (s, fx) = run(LIST, 2, &[Event::Add]);
        assert_eq!(
            s.mode,
            Mode::Draft {
                row: None,
                refused: false
            }
        );
        assert!(fx.is_empty());
        assert!(!s.can_add(LIST), "+ waits while a draft is open");
        let (s, fx) = run(LIST, 2, &[Event::Add, Event::Commit]);
        assert_eq!(s.mode, Mode::Sent { row: None });
        assert_eq!(fx, [Effect::Submit(None)]);
        let (s, _) = run(
            LIST,
            2,
            &[
                Event::Add,
                Event::Commit,
                Event::Answer {
                    accepted: true,
                    len: 3,
                },
            ],
        );
        assert_eq!(s, browse(Some(2), 3), "the added row is selected");
    }

    #[test]
    fn a_refused_draft_stays_open_with_its_note_until_typed_in() {
        let refused = [
            Event::Add,
            Event::Commit,
            Event::Answer {
                accepted: false,
                len: 2,
            },
        ];
        let (s, _) = run(LIST, 2, &refused);
        assert_eq!(
            s.mode,
            Mode::Draft {
                row: None,
                refused: true
            }
        );
        let (s, _) = run(LIST, 2, &[&refused[..], &[Event::Typed]].concat());
        assert_eq!(
            s.mode,
            Mode::Draft {
                row: None,
                refused: false
            }
        );
        let (s, _) = run(LIST, 2, &[&refused[..], &[Event::Cancel]].concat());
        assert_eq!(s.mode, Mode::Browse, "Escape drops the draft");
    }

    #[test]
    fn enter_and_a_double_click_edit_in_place() {
        let (s, _) = run(LIST, 3, &[Event::Pick(1), Event::Edit(None)]);
        assert_eq!(
            s.mode,
            Mode::Draft {
                row: Some(1),
                refused: false
            }
        );
        let (s, fx) = run(LIST, 3, &[Event::Edit(Some(2)), Event::Commit]);
        assert_eq!(fx, [Effect::Submit(Some(2))]);
        let (s, _) = transition(
            s,
            LIST,
            Event::Answer {
                accepted: true,
                len: 3,
            },
        );
        assert_eq!(s, browse(Some(2), 3), "the edited row stays selected");
        let (s, _) = run(READ_ONLY, 3, &[Event::Pick(1), Event::Edit(None)]);
        assert_eq!(
            s.mode,
            Mode::Browse,
            "a list that does not edit ignores Enter"
        );
        let (s, _) = run(READ_ONLY, 3, &[Event::Add]);
        assert_eq!(s.mode, Mode::Browse, "+ is disabled");
    }

    #[test]
    fn a_click_elsewhere_drops_the_draft_but_not_one_sent() {
        let (s, _) = run(LIST, 3, &[Event::Add, Event::Pick(0)]);
        assert_eq!(s, browse(Some(0), 3));
        let (s, _) = run(LIST, 3, &[Event::Add, Event::Commit, Event::Pick(0)]);
        assert_eq!(s.mode, Mode::Sent { row: None });
    }

    #[test]
    fn spaces_are_added_by_the_owner_then_named_in_place() {
        let (s, fx) = run(SPACES, 2, &[Event::Add]);
        assert_eq!((s.mode, fx), (Mode::Asked, vec![Effect::AskAdd]));
        let (s, _) = run(SPACES, 2, &[Event::Add, Event::Rows(3)]);
        assert_eq!(
            s,
            ListEditor {
                mode: Mode::Draft {
                    row: Some(2),
                    refused: false
                },
                selected: Some(2),
                len: 3
            },
            "the new Space is selected and its name edited"
        );
    }

    #[test]
    fn removing_a_space_asks_twice_and_never_the_last() {
        let (s, fx) = run(SPACES, 2, &[Event::Pick(1), Event::Remove]);
        assert_eq!((s.mode, fx), (Mode::Confirm, vec![]));
        let (s, fx) = run(SPACES, 2, &[Event::Pick(1), Event::Remove, Event::Remove]);
        assert_eq!((s.mode, fx), (Mode::Browse, vec![Effect::Remove(1)]));
        let (s, fx) = run(SPACES, 2, &[Event::Pick(1), Event::Remove, Event::Cancel]);
        assert_eq!((s.mode, fx), (Mode::Browse, vec![]));
        let (s, _) = run(SPACES, 2, &[Event::Pick(1), Event::Remove, Event::Pick(0)]);
        assert_eq!(
            s.mode,
            Mode::Browse,
            "picking another row drops the question"
        );
        let (s, fx) = run(SPACES, 1, &[Event::Pick(0), Event::Remove]);
        assert!(!s.can_remove(SPACES), "the last Space stays");
        assert!(fx.is_empty());
    }

    #[test]
    fn a_shorter_list_ends_a_draft_on_a_row_that_went() {
        let (s, _) = run(LIST, 3, &[Event::Edit(Some(2)), Event::Rows(2)]);
        assert_eq!(s, browse(Some(1), 2));
    }
}
