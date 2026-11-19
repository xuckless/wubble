//! `--smoke ax`: pages for screen readers through the host's real path. The smoke turns
//! `a11y.web` on (the settings' write), then waits until the window's accessible page items
//! (`ax-items`, `host/ax.rs`) show the bundled page's headings, link, field, check box and
//! button; it runs the button's default action as a screen reader would (`Host::ax_action`):
//! the page retitles itself from its click handler; then sets the text field's value: the page
//! retitles itself from its `input` handler, and the field's item shows the value once the
//! page's next tree change came; last it runs the default action of a button below the fold
//! (the host scrolls it into view first): the page retitles itself. Bounded by counts of CEF
//! work runs, never by time. Run by hand (it opens a window); what a screen reader makes of it
//! is checked by hand with Orca or an AT-SPI dump.

use wubble_ui::AxKind;

/// Runs of CEF work one step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 4_000;

pub const PAGE: &str = r##"<!doctype html><html lang="en"><head><title>ax</title></head>
<body style="font:16px sans-serif">
<h1>Welcome</h1>
<p>Read the <a href="#docs">docs</a> first.</p>
<h2>Sign in</h2>
<label for="email">Email</label>
<input id="email" value="a@b" oninput="document.title='ax value '+this.value">
<label><input type="checkbox" checked> Remember me</label>
<button onclick="document.title='ax clicked'">Send</button>
<div style="height:3000px"></div>
<button onclick="document.title='ax far'">Far</button>
</body></html>"##;

/// The items the page must show: kind (`AxItemView::kind`) and label.
pub const EXPECTED: &[(AxKind, &str)] = &[
    (AxKind::Heading, "Welcome"),
    (AxKind::Link, "docs"),
    (AxKind::Heading, "Sign in"),
    (AxKind::TextField, "Email"),
    (AxKind::CheckBox, "Remember me"),
    (AxKind::Button, "Send"),
    (AxKind::Button, "Far"),
];

/// What the smoke types into the field.
pub const TYPED: &str = "typed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Turning the setting on (at the first run of work).
    Start,
    /// Waiting for the page's items.
    Items,
    /// The button's default action ran: waiting for the page's title.
    Clicked,
    /// The field's value was set: waiting for the page's title.
    Typed,
    /// Waiting for the field's item to show the value.
    Value,
    /// The default action of the button below the fold ran (the page scrolls to it first):
    /// waiting for the page's title.
    Far,
    /// Passed: counting the items.
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxSmoke {
    step: Step,
    runs: u32,
}

impl Default for AxSmoke {
    fn default() -> Self {
        Self {
            step: Step::Start,
            runs: 0,
        }
    }
}

/// One accessible item as the window shows it: kind, label, value.
pub type Item = (AxKind, String, String);

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    /// CEF did a run of work; the window's items now.
    Worked(&'a [Item]),
    Title(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `a11y.web` on.
    TurnOn,
    /// A screen reader's request of item `index`: "click" or "set-value" with the text.
    Act {
        index: usize,
        name: &'static str,
        text: &'static str,
    },
    Pass(usize),
    Fail(String),
}

fn find(items: &[Item], kind: AxKind, label: &str) -> Option<usize> {
    items.iter().position(|(k, l, _)| *k == kind && l == label)
}

pub fn next(mut s: AxSmoke, input: Input) -> (AxSmoke, Vec<Action>) {
    let to = |step: Step, actions: Vec<Action>| (AxSmoke { step, runs: 0 }, actions);
    if let Input::Worked(_) = input {
        s.runs += 1;
        if s.runs > MAX_STEP_RUNS {
            let step = s.step;
            return (
                s,
                vec![Action::Fail(format!(
                    "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work"
                ))],
            );
        }
    }
    match (s.step, input) {
        (Step::Start, Input::Worked(_)) => to(Step::Items, vec![Action::TurnOn]),
        (Step::Items, Input::Worked(items))
            if EXPECTED.iter().all(|(k, l)| find(items, *k, l).is_some()) =>
        {
            let index = find(items, AxKind::Button, "Send").unwrap_or_default();
            to(
                Step::Clicked,
                vec![Action::Act {
                    index,
                    name: "click",
                    text: "",
                }],
            )
        }
        (Step::Clicked, Input::Title("ax clicked")) => (
            AxSmoke {
                step: Step::Typed,
                runs: 0,
            },
            vec![],
        ),
        (Step::Typed, Input::Worked(items)) if s.runs == 1 => {
            let index = find(items, AxKind::TextField, "Email").unwrap_or_default();
            (
                s,
                vec![Action::Act {
                    index,
                    name: "set-value",
                    text: TYPED,
                }],
            )
        }
        (Step::Typed, Input::Title(t)) if t == format!("ax value {TYPED}") => {
            to(Step::Value, vec![])
        }
        (Step::Value, Input::Worked(items))
            if items
                .iter()
                .any(|(k, l, v)| *k == AxKind::TextField && l == "Email" && v == TYPED) =>
        {
            let index = find(items, AxKind::Button, "Far").unwrap_or_default();
            to(
                Step::Far,
                vec![Action::Act {
                    index,
                    name: "click",
                    text: "",
                }],
            )
        }
        (Step::Far, Input::Title("ax far")) => to(Step::Done, vec![]),
        (Step::Done, Input::Worked(items)) => (s, vec![Action::Pass(items.len())]),
        _ => (s, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(value: &str) -> Vec<Item> {
        EXPECTED
            .iter()
            .map(|(k, l)| {
                let v = if *k == AxKind::TextField { value } else { "" };
                (*k, (*l).to_owned(), v.to_owned())
            })
            .collect()
    }

    #[test]
    fn the_smoke_walks_its_steps() {
        let (s, a) = next(AxSmoke::default(), Input::Worked(&[]));
        assert_eq!(a, vec![Action::TurnOn]);
        let (s, a) = next(s, Input::Worked(&items("a@b")[..3]));
        assert!(a.is_empty(), "not all items yet");
        let (s, a) = next(s, Input::Worked(&items("a@b")));
        assert_eq!(
            a,
            vec![Action::Act {
                index: 5,
                name: "click",
                text: ""
            }]
        );
        let (s, _) = next(s, Input::Title("ax clicked"));
        let (s, a) = next(s, Input::Worked(&items("a@b")));
        assert!(matches!(
            &a[..],
            [Action::Act {
                index: 3,
                name: "set-value",
                ..
            }]
        ));
        let (s, a) = next(s, Input::Worked(&items("a@b")));
        assert!(a.is_empty(), "asked once");
        let (s, _) = next(s, Input::Title("ax value typed"));
        let (s, a) = next(s, Input::Worked(&items("a@b")));
        assert!(a.is_empty(), "the tree has not caught up");
        let (s, a) = next(s, Input::Worked(&items(TYPED)));
        assert_eq!(
            a,
            vec![Action::Act {
                index: 6,
                name: "click",
                text: ""
            }],
            "the button below the fold"
        );
        let (s, a) = next(s, Input::Worked(&items(TYPED)));
        assert!(a.is_empty(), "waiting for its click");
        let (s, _) = next(s, Input::Title("ax far"));
        let (_, a) = next(s, Input::Worked(&items(TYPED)));
        assert_eq!(a, vec![Action::Pass(7)]);
    }

    #[test]
    fn a_stuck_step_fails() {
        let mut s = AxSmoke {
            step: Step::Items,
            runs: 0,
        };
        for _ in 0..MAX_STEP_RUNS {
            s = next(s, Input::Worked(&[])).0;
        }
        assert!(matches!(
            &next(s, Input::Worked(&[])).1[..],
            [Action::Fail(_)]
        ));
    }
}
