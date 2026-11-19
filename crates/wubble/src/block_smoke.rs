//! `--smoke block`: `block.html`, served from loopback, asks for two URLs the
//! smoke's injected list blocks, one script it replaces with uBO's `noop.js`
//! stub (`$redirect`), and one URL it allows. Once the page has loaded, the
//! blocker must have counted exactly three for the tab's view, and the
//! stub must have run (the page retitles itself from the script's `onload`;
//! `onerror`, a refused stub, fails at once). The page also holds a
//! `.cookie-notice` that only the list's generic rule `##.cookie-notice` can
//! hide (through the page's class collector and the engine's answer): the
//! page checks its computed style, retitles, then navigates itself once to a
//! second copy that must hide it again. The list is injected, so no list is
//! read from disk or fetched. Bounded by counts of CEF work runs, never by
//! time.

use wubble_engine::nav::WebEvent;

/// Runs of CEF work the smoke may take before it gives up.
pub const MAX_STEP_RUNS: u32 = 20_000;

/// The smoke's list: everything on one host, scripts on another replaced
/// by uBO's stub, and a generic rule for the cookie notice.
pub const BLOCK_LIST: &str =
    "||blocked.wubble.test^\n||served.wubble.test^$script,redirect=noop.js\n##.cookie-notice\n";

/// The requests `block.html` makes to those hosts: blocked and served
/// both count as blocked.
pub const EXPECTED: u32 = 3;

/// `block.html`'s title once the stub ran, or if it was refused.
pub const SERVED_TITLE: &str = "wubble block served";
pub const REFUSED_TITLE: &str = "wubble block refused";
/// Its titles once `.cookie-notice` is hidden (and `.kept` is not): on the page, then on the
/// page it navigated itself to; or when it never was.
pub const HIDDEN_TITLE: &str = "wubble block hidden";
pub const HIDDEN_AGAIN_TITLE: &str = "wubble block hidden again";
pub const NOT_HIDDEN_TITLE: &str = "wubble block not hidden";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Waiting for the page to finish loading.
    Load,
    /// Loaded: waiting for the count, the stub and both hidings.
    Wait,
}

/// What the smoke has seen so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The `$redirect` stub ran.
    pub served: bool,
    /// The blocker's count reached [`EXPECTED`] after the load, with the stub run.
    pub counted: bool,
    /// The notice was hidden on the page.
    pub hidden: bool,
    /// And on the page it navigated itself to.
    pub hidden_again: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSmoke {
    At { step: Step, runs: u32, seen: Seen },
    Done,
}

impl Default for BlockSmoke {
    fn default() -> Self {
        Self::At {
            step: Step::Load,
            runs: 0,
            seen: Seen::default(),
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
    let BlockSmoke::At {
        mut step,
        mut runs,
        mut seen,
    } = state
    else {
        return (state, None);
    };
    let fail = |reason: String| (BlockSmoke::Done, Some(Action::Fail(reason)));
    match input {
        Input::Web(WebEvent::TitleChanged(title)) => match title.as_str() {
            REFUSED_TITLE => {
                return fail("the $redirect stub was refused: its script failed to load".into());
            }
            NOT_HIDDEN_TITLE => {
                return fail(
                    "the generic rule did not hide .cookie-notice (or hid .kept too)".into(),
                );
            }
            SERVED_TITLE => seen.served = true,
            HIDDEN_TITLE => seen.hidden = true,
            HIDDEN_AGAIN_TITLE => seen.hidden_again = true,
            _ => {}
        },
        Input::Web(WebEvent::LoadingChanged { loading: false, .. }) if step == Step::Load => {
            step = Step::Wait;
            runs = 0;
        }
        Input::Worked { blocked } if step == Step::Wait => {
            if blocked > EXPECTED {
                return fail(format!(
                    "{blocked} requests blocked, not {EXPECTED}: the allowed one was blocked too"
                ));
            }
            seen.counted |= blocked == EXPECTED && seen.served;
            runs += 1;
            if runs > MAX_STEP_RUNS {
                return fail(format!(
                    "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work with {blocked} \
                     blocked, want {EXPECTED}; seen {seen:?}"
                ));
            }
        }
        Input::Worked { .. } => {
            runs += 1;
            if runs > MAX_STEP_RUNS {
                return fail(format!(
                    "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work; seen {seen:?}"
                ));
            }
        }
        Input::Web(_) => {}
    }
    if step == Step::Wait && seen.counted && seen.hidden && seen.hidden_again {
        return (BlockSmoke::Done, Some(Action::Pass(EXPECTED)));
    }
    (BlockSmoke::At { step, runs, seen }, None)
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

    fn title(t: &str) -> WebEvent {
        WebEvent::TitleChanged(t.into())
    }

    fn served() -> WebEvent {
        title(SERVED_TITLE)
    }

    #[test]
    fn passes_once_counted_served_and_hidden_twice() {
        let (s, h, h2) = (served(), title(HIDDEN_TITLE), title(HIDDEN_AGAIN_TITLE));
        let (st, actions) = run(&[
            worked(0),
            Input::Web(&s),
            Input::Web(&LOADED),
            worked(1),
            worked(3),
            Input::Web(&h),
            Input::Web(&h2),
        ]);
        assert_eq!(st, BlockSmoke::Done);
        assert_eq!(actions, [Action::Pass(3)]);
        // Any order after the load: the page may navigate before the count settles.
        let (st, actions) = run(&[
            Input::Web(&LOADED),
            Input::Web(&h),
            Input::Web(&h2),
            Input::Web(&s),
            worked(3),
        ]);
        assert_eq!(st, BlockSmoke::Done);
        assert_eq!(actions, [Action::Pass(3)]);
    }

    #[test]
    fn the_count_alone_does_not_pass() {
        let s = served();
        let (st, actions) = run(&[Input::Web(&LOADED), Input::Web(&s), worked(3)]);
        assert!(matches!(st, BlockSmoke::At { .. }));
        assert_eq!(actions, []);
        // Nor does one hiding without the second.
        let h = title(HIDDEN_TITLE);
        let (st, actions) = run(&[
            Input::Web(&LOADED),
            Input::Web(&s),
            worked(3),
            Input::Web(&h),
        ]);
        assert!(matches!(st, BlockSmoke::At { .. }));
        assert_eq!(actions, []);
    }

    #[test]
    fn the_count_without_the_stub_does_not_pass() {
        let mut inputs = vec![Input::Web(&LOADED)];
        inputs.extend(vec![worked(EXPECTED); MAX_STEP_RUNS as usize + 3]);
        let (s, actions) = run(&inputs);
        assert_eq!(s, BlockSmoke::Done);
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("served: false")),
            "{actions:?}"
        );
    }

    #[test]
    fn a_refused_stub_or_an_unhidden_notice_fails_at_once() {
        for (t, word) in [(REFUSED_TITLE, "refused"), (NOT_HIDDEN_TITLE, "hide")] {
            let e = title(t);
            let (s, actions) = run(&[Input::Web(&e)]);
            assert_eq!(s, BlockSmoke::Done);
            assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains(word)));
        }
    }

    #[test]
    fn counts_before_the_load_do_not_pass() {
        let s = served();
        let (s, actions) = run(&[Input::Web(&s), worked(3), worked(3)]);
        assert!(matches!(
            s,
            BlockSmoke::At {
                step: Step::Load,
                seen: Seen { counted: false, .. },
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
                matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("Wait")),
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
        for title in [HIDDEN_TITLE, HIDDEN_AGAIN_TITLE, NOT_HIDDEN_TITLE] {
            assert!(page.contains(title), "{title}");
        }
        // Hidden by the generic rule alone.
        assert!(
            BLOCK_LIST.contains("##.cookie-notice") && page.contains("class=\"cookie-notice\"")
        );
    }
}
