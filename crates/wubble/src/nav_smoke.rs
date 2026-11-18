//! `--smoke nav`: load the bundled page, navigate to a second bundled page,
//! pass on its title. Bounded by a count of CEF work runs, not by time.

use wubble_engine::nav::WebEvent;

/// Runs of CEF work the smoke waits before it gives up.
pub const MAX_WORK_RUNS: u32 = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavSmoke {
    AwaitFirstLoad { runs: u32 },
    AwaitSecondTitle { runs: u32 },
    Done,
}

impl Default for NavSmoke {
    fn default() -> Self {
        Self::AwaitFirstLoad { runs: 0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmokeInput<'a> {
    /// CEF did a run of work.
    Worked,
    Event(&'a WebEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmokeAction {
    LoadSecondPage,
    Pass(String),
    Fail(String),
}

/// `want` is the second page's title.
pub fn next(state: NavSmoke, input: SmokeInput, want: &str) -> (NavSmoke, Option<SmokeAction>) {
    use NavSmoke::*;
    match (state, input) {
        (AwaitFirstLoad { runs } | AwaitSecondTitle { runs }, SmokeInput::Worked)
            if runs >= MAX_WORK_RUNS =>
        {
            (
                Done,
                Some(SmokeAction::Fail(format!(
                    "no title {want:?} after {MAX_WORK_RUNS} runs of CEF work"
                ))),
            )
        }
        (AwaitFirstLoad { runs }, SmokeInput::Worked) => (AwaitFirstLoad { runs: runs + 1 }, None),
        (AwaitSecondTitle { runs }, SmokeInput::Worked) => {
            (AwaitSecondTitle { runs: runs + 1 }, None)
        }
        (
            AwaitFirstLoad { runs },
            SmokeInput::Event(WebEvent::LoadingChanged { loading: false, .. }),
        ) => (AwaitSecondTitle { runs }, Some(SmokeAction::LoadSecondPage)),
        (AwaitSecondTitle { .. }, SmokeInput::Event(WebEvent::TitleChanged(title)))
            if title == want =>
        {
            (Done, Some(SmokeAction::Pass(title.clone())))
        }
        (state, _) => (state, None),
    }
}

/// The text of a page's `<title>`.
pub fn title_of(html: &str) -> Option<&str> {
    let start = html.find("<title>")? + "<title>".len();
    let len = html[start..].find("</title>")?;
    Some(html[start..start + len].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WANT: &str = "wubble nav b";

    fn run(inputs: &[SmokeInput]) -> (NavSmoke, Vec<SmokeAction>) {
        inputs
            .iter()
            .fold((NavSmoke::default(), vec![]), |(state, mut all), &input| {
                let (next, action) = next(state, input, WANT);
                all.extend(action);
                (next, all)
            })
    }

    fn title(t: &str) -> WebEvent {
        WebEvent::TitleChanged(t.into())
    }

    fn loaded() -> WebEvent {
        WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        }
    }

    #[test]
    fn happy_path() {
        let (a, done, b) = (title("wubble smoke"), loaded(), title(WANT));
        let (state, actions) = run(&[
            SmokeInput::Worked,
            SmokeInput::Event(&a),
            SmokeInput::Event(&done),
            SmokeInput::Worked,
            SmokeInput::Event(&b),
        ]);
        assert_eq!(state, NavSmoke::Done);
        assert_eq!(
            actions,
            [SmokeAction::LoadSecondPage, SmokeAction::Pass(WANT.into())]
        );
    }

    #[test]
    fn early_and_wrong_titles_are_ignored() {
        let (early, done, wrong) = (title(WANT), loaded(), title("other"));
        let (state, actions) = run(&[
            SmokeInput::Event(&early),
            SmokeInput::Event(&done),
            SmokeInput::Event(&wrong),
            SmokeInput::Event(&done),
        ]);
        assert_eq!(state, NavSmoke::AwaitSecondTitle { runs: 0 });
        assert_eq!(actions, [SmokeAction::LoadSecondPage]);
    }

    #[test]
    fn too_many_runs_fail_once() {
        let inputs = vec![SmokeInput::Worked; MAX_WORK_RUNS as usize + 3];
        let (state, actions) = run(&inputs);
        assert_eq!(state, NavSmoke::Done);
        assert!(
            matches!(actions.as_slice(), [SmokeAction::Fail(e)] if e.contains(WANT)),
            "{actions:?}"
        );
    }

    #[test]
    fn the_bundled_page_has_the_title() {
        assert_eq!(title_of(include_str!("../assets/nav.html")), Some(WANT));
        assert_eq!(title_of("<title> a </title>"), Some("a"));
        assert_eq!(title_of("<p>no title</p>"), None);
    }
}
