//! Find in page. CEF searches with `BrowserHost::Find` and reports
//! progressively through `OnFindResult`, possibly after the search stopped;
//! results are ignored while no search runs and reported only on change.

use crate::page::PageEvent;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Find {
    #[default]
    Idle,
    Searching {
        query: String,
        match_case: bool,
        matches: u32,
        active: u32,
        done: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindInput {
    // From the host.
    /// Empty text stops the search. The same text again steps forward.
    Start {
        text: String,
        match_case: bool,
    },
    Next,
    Previous,
    Stop {
        clear_selection: bool,
    },
    // From CEF.
    Result {
        count: i32,
        active: i32,
        done: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindEffect {
    Find {
        text: String,
        forward: bool,
        match_case: bool,
        find_next: bool,
    },
    StopFinding {
        clear_selection: bool,
    },
    Emit(PageEvent),
}

pub fn transition(state: Find, input: FindInput) -> (Find, Vec<FindEffect>) {
    use {Find::*, FindInput as In};
    match (state, input) {
        (state, In::Start { text, .. }) if text.is_empty() => transition(
            state,
            In::Stop {
                clear_selection: true,
            },
        ),
        (
            Searching {
                query,
                match_case,
                matches,
                active,
                done,
            },
            In::Start {
                text,
                match_case: case,
            },
        ) if query == text && case == match_case => {
            let state = Searching {
                query,
                match_case,
                matches,
                active,
                done,
            };
            transition(state, In::Next)
        }
        (_, In::Start { text, match_case }) => (
            Searching {
                query: text.clone(),
                match_case,
                matches: 0,
                active: 0,
                done: false,
            },
            vec![FindEffect::Find {
                text,
                forward: true,
                match_case,
                find_next: false,
            }],
        ),
        (state @ Searching { .. }, step @ (In::Next | In::Previous)) => {
            let Searching {
                query, match_case, ..
            } = &state
            else {
                unreachable!("matched Searching")
            };
            let effect = FindEffect::Find {
                text: query.clone(),
                forward: step == In::Next,
                match_case: *match_case,
                find_next: true,
            };
            (state, vec![effect])
        }
        (Searching { .. }, In::Stop { clear_selection }) => {
            (Idle, vec![FindEffect::StopFinding { clear_selection }])
        }
        (
            Searching {
                query,
                match_case,
                matches,
                active,
                done,
            },
            In::Result {
                count,
                active: now,
                done: last,
            },
        ) => {
            let (count, now) = (count.max(0) as u32, now.max(0) as u32);
            let effects = if (matches, active, done) == (count, now, last) {
                vec![]
            } else {
                vec![FindEffect::Emit(PageEvent::FindResult {
                    matches: count,
                    active: now,
                    done: last,
                })]
            };
            let next = Searching {
                query,
                match_case,
                matches: count,
                active: now,
                done: last,
            };
            (next, effects)
        }
        // Idle: steps, stops and late results do nothing.
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{Find::*, FindEffect as E, FindInput as In, *};

    fn start(text: &str) -> In {
        In::Start {
            text: text.into(),
            match_case: false,
        }
    }

    fn result(count: i32, active: i32, done: bool) -> In {
        In::Result {
            count,
            active,
            done,
        }
    }

    fn find(text: &str, forward: bool, find_next: bool) -> E {
        E::Find {
            text: text.into(),
            forward,
            match_case: false,
            find_next,
        }
    }

    #[test]
    fn a_search_steps_and_reports_changes_only() {
        let (s, e) = transition(Idle, start("cat"));
        assert_eq!(e, vec![find("cat", true, false)]);
        let (s, e) = transition(s, result(3, 1, false));
        assert_eq!(
            e,
            vec![E::Emit(PageEvent::FindResult {
                matches: 3,
                active: 1,
                done: false
            })]
        );
        let (s, e) = transition(s, result(3, 1, false));
        assert!(e.is_empty(), "an unchanged result was reported");
        let (s, e) = transition(s, In::Next);
        assert_eq!(e, vec![find("cat", true, true)]);
        let (s, e) = transition(s, In::Previous);
        assert_eq!(e, vec![find("cat", false, true)]);
        let (s, e) = transition(s, start("cat"));
        assert_eq!(
            e,
            vec![find("cat", true, true)],
            "the same text again steps forward"
        );
        let (s, e) = transition(s, start("dog"));
        assert_eq!(e, vec![find("dog", true, false)], "new text starts over");
        let (s, e) = transition(
            s,
            In::Stop {
                clear_selection: false,
            },
        );
        assert_eq!(
            e,
            vec![E::StopFinding {
                clear_selection: false
            }]
        );
        assert_eq!(s, Idle);
    }

    #[test]
    fn idle_ignores_results_steps_and_stops() {
        for input in [
            result(5, 2, true),
            In::Next,
            In::Previous,
            In::Stop {
                clear_selection: true,
            },
        ] {
            assert_eq!(transition(Idle, input), (Idle, vec![]));
        }
    }

    #[test]
    fn empty_text_stops() {
        let (s, _) = transition(Idle, start("x"));
        assert_eq!(
            transition(s, start("")),
            (
                Idle,
                vec![E::StopFinding {
                    clear_selection: true
                }]
            )
        );
        assert_eq!(transition(Idle, start("")), (Idle, vec![]));
    }

    #[test]
    fn negative_counts_read_as_zero() {
        let (s, _) = transition(Idle, start("x"));
        let (_, e) = transition(s, result(-1, -1, true));
        assert_eq!(
            e,
            vec![E::Emit(PageEvent::FindResult {
                matches: 0,
                active: 0,
                done: true
            })]
        );
    }
}
