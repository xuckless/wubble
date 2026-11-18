//! `--smoke block`: the bundled `block.html` asks for two URLs the smoke's
//! injected list blocks (and one it allows); once the page has loaded, the
//! blocker must have counted exactly two for the tab's view. The list is
//! injected, so no list is read from disk or fetched. Bounded by counts of CEF
//! work runs, never by time.

use wubble_engine::nav::WebEvent;

/// Runs of CEF work the count may take to arrive before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 2_000;

/// The one rule of the smoke's list: everything on this host.
pub const BLOCK_LIST: &str = "||blocked.wubble.test^\n";

/// The requests `block.html` makes to that host.
pub const EXPECTED: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Waiting for the page to finish loading.
    Load,
    /// Loaded: waiting for the blocker's count to settle at [`EXPECTED`].
    Count,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSmoke {
    At { step: Step, runs: u32 },
    Done,
}

impl Default for BlockSmoke {
    fn default() -> Self {
        Self::At {
            step: Step::Load,
            runs: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    /// CEF did a run of work; the blocker's count for the tab's view now.
    Worked {
        blocked: u32,
    },
    Web(&'a WebEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Passes with this count.
    Pass(u32),
    Fail(String),
}

pub fn next(state: BlockSmoke, input: Input) -> (BlockSmoke, Option<Action>) {
    let BlockSmoke::At { step, runs } = state else {
        return (state, None);
    };
    let fail = |reason: String| (BlockSmoke::Done, Some(Action::Fail(reason)));
    match (step, input) {
        (Step::Load, Input::Web(WebEvent::LoadingChanged { loading: false, .. })) => (
            BlockSmoke::At {
                step: Step::Count,
                runs: 0,
            },
            None,
        ),
        (Step::Count, Input::Worked { blocked }) if blocked == EXPECTED => {
            (BlockSmoke::Done, Some(Action::Pass(blocked)))
        }
        (Step::Count, Input::Worked { blocked }) if blocked > EXPECTED => fail(format!(
            "{blocked} requests blocked, not {EXPECTED}: the allowed one was blocked too"
        )),
        (_, Input::Worked { blocked }) if runs >= MAX_STEP_RUNS => fail(format!(
            "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work with {blocked} blocked, \
             want {EXPECTED}"
        )),
        (_, Input::Worked { .. }) => (
            BlockSmoke::At {
                step,
                runs: runs + 1,
            },
            None,
        ),
        _ => (state, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOADED: WebEvent = WebEvent::LoadingChanged {
        loading: false,
        can_go_back: false,
        can_go_forward: false,
    };

    fn run(inputs: &[Input]) -> (BlockSmoke, Vec<Action>) {
        inputs
            .iter()
            .fold((BlockSmoke::default(), vec![]), |(s, mut all), &i| {
                let (s, a) = next(s, i);
                all.extend(a);
                (s, all)
            })
    }

    fn worked(blocked: u32) -> Input<'static> {
        Input::Worked { blocked }
    }

    #[test]
    fn passes_at_two_after_the_load() {
        let (s, actions) = run(&[worked(0), Input::Web(&LOADED), worked(1), worked(2)]);
        assert_eq!(s, BlockSmoke::Done);
        assert_eq!(actions, [Action::Pass(2)]);
    }

    #[test]
    fn counts_before_the_load_do_not_pass() {
        let (s, actions) = run(&[worked(2), worked(2)]);
        assert!(matches!(
            s,
            BlockSmoke::At {
                step: Step::Load,
                ..
            }
        ));
        assert_eq!(actions, []);
    }

    #[test]
    fn three_fails_at_once() {
        let (s, actions) = run(&[Input::Web(&LOADED), worked(3)]);
        assert_eq!(s, BlockSmoke::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains('3')));
    }

    #[test]
    fn too_few_fail_once_the_wait_is_spent() {
        for blocked in [0, 1] {
            let mut inputs = vec![Input::Web(&LOADED)];
            inputs.extend(vec![worked(blocked); MAX_STEP_RUNS as usize + 3]);
            let (s, actions) = run(&inputs);
            assert_eq!(s, BlockSmoke::Done);
            assert!(
                matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("Count")),
                "{blocked}: {actions:?}"
            );
        }
    }

    #[test]
    fn a_page_that_never_loads_fails_once() {
        let (s, actions) = run(&vec![worked(0); MAX_STEP_RUNS as usize + 3]);
        assert_eq!(s, BlockSmoke::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("Load")));
    }

    #[test]
    fn the_bundled_page_requests_what_the_list_blocks() {
        let page = include_str!("../assets/block.html");
        assert_eq!(
            page.matches("https://blocked.wubble.test/").count(),
            EXPECTED as usize
        );
        assert!(page.contains("data:image/gif"));
    }
}
