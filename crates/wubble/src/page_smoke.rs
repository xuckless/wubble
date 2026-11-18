//! `--smoke page`: the bundled `page.html` drives the page's own UI through
//! the real host paths. Bounded by counts of CEF work runs per step, never
//! by time.
//!
//! 1. Load the page and wait for its first frame.
//! 2. Click the `<select>`: the popup widget shows and paints a frame;
//!    click the page beside it: the popup hides.
//! 3. Click the button that calls `alert()`: the dialog arrives, the sheet's
//!    state accepts it, and the page's title changes after `alert` returns.
//! 4. Right-click the link: the menu lists "Open Link in New Tab"; it is
//!    dismissed and answered exactly once.
//! 5. Find "needle" through the find bar: 2 matches.
//! 6. Zoom in: the page reports a zoom above 100 %.
//!
//! Real pointer and key use of the popup, the dialog's feel and the
//! clipboard are left to a human.

use wubble_engine::input::Point;
use wubble_engine::nav::WebEvent;
use wubble_engine::page::{JsDialogKind, PageEvent};

use crate::menu::{self, NavFlags};

/// Runs of CEF work one step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 2_000;

/// Where `page.html` puts things, view-relative logical px (it lays them
/// out absolutely).
pub const SELECT_AT: Point = Point { x: 120.0, y: 35.0 };
pub const BUTTON_AT: Point = Point { x: 80.0, y: 95.0 };
pub const LINK_AT: Point = Point { x: 50.0, y: 155.0 };
pub const BLANK_AT: Point = Point { x: 500.0, y: 400.0 };
/// The title the page sets once `alert()` has returned.
pub const ALERTED: &str = "wubble page alerted";
pub const NEEDLE: &str = "needle";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Load,
    FirstFrame,
    PopupShown,
    PopupFrame,
    PopupHidden,
    Dialog,
    Alerted,
    Menu,
    Found,
    Zoomed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSmoke {
    At { step: Step, runs: u32 },
    Done,
}

impl Default for PageSmoke {
    fn default() -> Self {
        Self::At {
            step: Step::Load,
            runs: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    /// CEF did a run of work; whether the page and the popup have a frame
    /// on screen.
    Worked {
        framed: bool,
        popup_framed: bool,
    },
    Web(&'a WebEvent),
    Page(&'a PageEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Click(Point),
    RightClick(Point),
    /// Accept the open dialog through the sheet's path.
    AcceptDialog,
    /// Dismiss the open menu through the menu's path; fails unless that
    /// answered exactly once.
    DismissMenu,
    Find(&'static str),
    ZoomIn,
    Pass(u32),
    Fail(String),
}

pub fn next(state: PageSmoke, input: Input) -> (PageSmoke, Vec<Action>) {
    use {Input::*, Step::*};
    let PageSmoke::At { step, runs } = state else {
        return (state, vec![]);
    };
    let to = |step| PageSmoke::At { step, runs: 0 };
    let stay = |runs| PageSmoke::At { step, runs };
    match (step, input) {
        (_, Worked { .. }) if runs >= MAX_STEP_RUNS => (
            PageSmoke::Done,
            vec![Action::Fail(format!(
                "stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work"
            ))],
        ),
        (Load, Web(WebEvent::LoadingChanged { loading: false, .. })) => (to(FirstFrame), vec![]),
        (FirstFrame, Worked { framed: true, .. }) => {
            (to(PopupShown), vec![Action::Click(SELECT_AT)])
        }
        (PopupShown, Page(PageEvent::PopupShown(_))) => (to(PopupFrame), vec![]),
        (
            PopupFrame,
            Worked {
                popup_framed: true, ..
            },
        ) => (to(PopupHidden), vec![Action::Click(BLANK_AT)]),
        (PopupHidden, Page(PageEvent::PopupHidden)) => (to(Dialog), vec![Action::Click(BUTTON_AT)]),
        (
            Dialog,
            Page(PageEvent::DialogRequested {
                kind: JsDialogKind::Alert,
                ..
            }),
        ) => (to(Alerted), vec![Action::AcceptDialog]),
        (Alerted, Web(WebEvent::TitleChanged(title))) if title == ALERTED => {
            (to(Menu), vec![Action::RightClick(LINK_AT)])
        }
        (Menu, Page(PageEvent::ContextMenuRequested { menu, .. })) => {
            let entries = menu::entries(menu, NavFlags::default());
            if entries.iter().any(|e| e.label == "Open Link in New Tab") {
                (to(Found), vec![Action::DismissMenu, Action::Find(NEEDLE)])
            } else {
                (
                    PageSmoke::Done,
                    vec![Action::Fail(format!("no link entries in {entries:?}"))],
                )
            }
        }
        (
            Found,
            Page(PageEvent::FindResult {
                matches,
                done: true,
                ..
            }),
        ) => match matches {
            2 => (to(Zoomed), vec![Action::ZoomIn]),
            n => (
                PageSmoke::Done,
                vec![Action::Fail(format!("found {n} of {NEEDLE:?}, not 2"))],
            ),
        },
        (Zoomed, Page(PageEvent::ZoomChanged { percent })) if *percent > 100 => {
            (PageSmoke::Done, vec![Action::Pass(*percent)])
        }
        (_, Worked { .. }) => (stay(runs + 1), vec![]),
        _ => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_engine::page::{ContextMenu, Rect, RequestId};

    fn run(inputs: &[Input]) -> (PageSmoke, Vec<Action>) {
        inputs
            .iter()
            .fold((PageSmoke::default(), vec![]), |(s, mut all), &i| {
                let (s, a) = next(s, i);
                all.extend(a);
                (s, all)
            })
    }

    const WORK: Input = Input::Worked {
        framed: true,
        popup_framed: false,
    };

    #[test]
    fn happy_path() {
        let loaded = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        let shown = PageEvent::PopupShown(Rect::default());
        let alert = PageEvent::DialogRequested {
            id: RequestId(1),
            kind: JsDialogKind::Alert,
            origin: String::new(),
            message: String::new(),
            default_text: String::new(),
        };
        let title = WebEvent::TitleChanged(ALERTED.into());
        let menu = PageEvent::ContextMenuRequested {
            id: RequestId(2),
            menu: Box::new(ContextMenu {
                link_url: "https://a.test/".into(),
                ..Default::default()
            }),
        };
        let found = PageEvent::FindResult {
            matches: 2,
            active: 1,
            done: true,
        };
        let zoom = PageEvent::ZoomChanged { percent: 110 };
        let (s, actions) = run(&[
            WORK,
            Input::Web(&loaded),
            WORK,
            Input::Page(&shown),
            WORK,
            Input::Worked {
                framed: true,
                popup_framed: true,
            },
            Input::Page(&PageEvent::PopupHidden),
            Input::Page(&alert),
            Input::Web(&title),
            Input::Page(&menu),
            Input::Page(&found),
            Input::Page(&zoom),
        ]);
        assert_eq!(s, PageSmoke::Done);
        assert_eq!(
            actions,
            [
                Action::Click(SELECT_AT),
                Action::Click(BLANK_AT),
                Action::Click(BUTTON_AT),
                Action::AcceptDialog,
                Action::RightClick(LINK_AT),
                Action::DismissMenu,
                Action::Find(NEEDLE),
                Action::ZoomIn,
                Action::Pass(110),
            ]
        );
    }

    #[test]
    fn a_stuck_step_fails_once() {
        let inputs = vec![WORK; MAX_STEP_RUNS as usize + 3];
        let (s, actions) = run(&inputs);
        assert_eq!(s, PageSmoke::Done);
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("Load")),
            "{actions:?}"
        );
    }

    #[test]
    fn the_bundled_page_has_what_the_smoke_clicks() {
        let page = include_str!("../assets/page.html");
        for needle in ["<select", "alert(", "<a ", ALERTED] {
            assert!(page.contains(needle), "{needle}");
        }
        assert_eq!(page.matches(NEEDLE).count(), 2);
    }
}
