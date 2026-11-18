//! `--smoke block`: the bundled `block.html` asks for two URLs the smoke's
//! injected list blocks, one script it replaces with uBO's `noop.js` stub
//! (`$redirect`), and one URL it allows. Once the page has loaded, the
//! blocker must have counted exactly three for the tab's view, and the
//! stub must have run (the page retitles itself from the script's `onload`;
//! `onerror`, a refused stub, fails at once). The list is injected, so no
//! list is read from disk or fetched. Bounded by counts of CEF work runs,
//! never by time.

use wubble_engine::nav::WebEvent;

/// Runs of CEF work the count may take to arrive before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 2_000;

/// The smoke's list: everything on one host, and scripts on another
/// replaced by uBO's stub.
pub const BLOCK_LIST: &str =
    "||blocked.wubble.test^\n||served.wubble.test^$script,redirect=noop.js\n";

/// The requests `block.html` makes to those hosts: blocked and served
/// both count as blocked.
pub const EXPECTED: u32 = 3;

/// `block.html`'s title once the stub ran, or if it was refused.
pub const SERVED_TITLE: &str = "wubble block served";
pub const REFUSED_TITLE: &str = "wubble block refused";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Waiting for the page to finish loading.
    Load,
    /// Loaded: waiting for the blocker's count to settle at [`EXPECTED`].
    Count,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSmoke {
    /// `served`: the page reported that the `$redirect` stub ran.
    At {
        step: Step,
        runs: u32,
        served: bool,
    },
    Done,
}

impl Default for BlockSmoke {
    fn default() -> Self {
        Self::At {
            step: Step::Load,
            runs: 0,
            served: false,
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
    let BlockSmoke::At { step, runs, served } = state else {
        return (state, None);
    };
    let fail = |reason: String| (BlockSmoke::Done, Some(Action::Fail(reason)));
    match (step, input) {
        (_, Input::Web(WebEvent::TitleChanged(title))) if title == REFUSED_TITLE => {
            fail("the $redirect stub was refused: its script failed to load".into())
        }
        (_, Input::Web(WebEvent::TitleChanged(title))) if title == SERVED_TITLE => (
            BlockSmoke::At {
                step,
                runs,
                served: true,
            },
            None,
        ),
        (Step::Load, Input::Web(WebEvent::LoadingChanged { loading: false, .. })) => (
            BlockSmoke::At {
                step: Step::Count,
                runs: 0,
                served,
            },
            None,
        ),
        (Step::Count, Input::Worked { blocked }) if blocked == EXPECTED && served => {
            (BlockSmoke::Done, Some(Action::Pass(blocked)))
        }
        (Step::Count, Input::Worked { blocked }) if blocked > EXPECTED => fail(format!(
            "{blocked} requests blocked, not {EXPECTED}: the allowed one was blocked too"
        )),
        (_, Input::Worked { blocked }) if runs >= MAX_STEP_RUNS => fail(format!(
            "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work with {blocked} blocked, \
             want {EXPECTED}; stub ran: {served}"
        )),
        (_, Input::Worked { .. }) => (
            BlockSmoke::At {
                step,
                runs: runs + 1,
                served,
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

    fn served() -> WebEvent {
        WebEvent::TitleChanged(SERVED_TITLE.into())
    }

    #[test]
    fn passes_at_three_after_the_load_and_the_stub() {
        let s = served();
        let (st, actions) = run(&[
            worked(0),
            Input::Web(&s),
            Input::Web(&LOADED),
            worked(1),
            worked(3),
        ]);
        assert_eq!(st, BlockSmoke::Done);
        assert_eq!(actions, [Action::Pass(3)]);
        // The stub may report after the load.
        let (st, actions) = run(&[Input::Web(&LOADED), worked(3), Input::Web(&s), worked(3)]);
        assert_eq!(st, BlockSmoke::Done);
        assert_eq!(actions, [Action::Pass(3)]);
    }

    #[test]
    fn the_count_without_the_stub_does_not_pass() {
        let mut inputs = vec![Input::Web(&LOADED)];
        inputs.extend(vec![worked(EXPECTED); MAX_STEP_RUNS as usize + 3]);
        let (s, actions) = run(&inputs);
        assert_eq!(s, BlockSmoke::Done);
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("stub ran: false")),
            "{actions:?}"
        );
    }

    #[test]
    fn a_refused_stub_fails_at_once() {
        let refused = WebEvent::TitleChanged(REFUSED_TITLE.into());
        let (s, actions) = run(&[Input::Web(&refused)]);
        assert_eq!(s, BlockSmoke::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("refused")));
    }

    #[test]
    fn counts_before_the_load_do_not_pass() {
        let s = served();
        let (s, actions) = run(&[Input::Web(&s), worked(3), worked(3)]);
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
    fn four_fails_at_once() {
        let (s, actions) = run(&[Input::Web(&LOADED), worked(4)]);
        assert_eq!(s, BlockSmoke::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains('4')));
    }

    #[test]
    fn too_few_fail_once_the_wait_is_spent() {
        for blocked in [0, 2] {
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
        let blocked = page.matches("https://blocked.wubble.test/").count();
        let served = page.matches("https://served.wubble.test/").count();
        assert_eq!((blocked, served), (2, 1));
        assert_eq!((blocked + served) as u32, EXPECTED);
        assert!(page.contains(SERVED_TITLE) && page.contains(REFUSED_TITLE));
        assert!(page.contains("data:image/gif"));
    }
}
