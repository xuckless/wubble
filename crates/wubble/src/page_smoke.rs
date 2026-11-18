//! `--smoke page`: the bundled `page.html` drives the page's own UI through
//! the real host paths. Bounded by counts of CEF work runs per step, never
//! by time.
//!
//! 1. Load the page and wait for its first frame.
//! 2. Click the `<select>`, which sits at the view's right edge hanging past it: the popup
//!    widget shows, extends past the view (the host reports the window as the
//!    screen, so Blink may place it outside the page) and paints a frame;
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
use wubble_engine::page::{JsDialogKind, PageEvent, Rect};
use wubble_gfx::FrameSize;

use crate::menu::{self, NavFlags};

/// Runs of CEF work one step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 2_000;

/// Where `page.html` puts things, view-relative logical px (it lays them
/// out absolutely).
pub const SELECT_Y: f32 = 35.0;
/// `page.html` puts the select's left part (where the click lands) this far
/// left of the view's right edge.
pub const SELECT_FROM_RIGHT: f32 = 30.0;
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
        /// The view's width, logical px.
        view_width: f32,
    },
    Web(&'a WebEvent),
    /// A page event, and the view's physical size.
    Page(&'a PageEvent, FrameSize),
}

/// Whether a popup (physical px, view-relative) starts left of or above the
/// view. Blink can only put it there when told the window is the screen: with
/// the page as the screen, a list wider than the select is right-aligned to it
/// and clamped into the page's own rectangle or left to overflow its right.
pub fn starts_outside_view(rect: Rect) -> bool {
    rect.x < 0 || rect.y < 0
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
        (
            FirstFrame,
            Worked {
                framed: true,
                view_width,
                ..
            },
        ) => (
            to(PopupShown),
            vec![Action::Click(Point {
                x: view_width - SELECT_FROM_RIGHT,
                y: SELECT_Y,
            })],
        ),
        (PopupShown, Page(PageEvent::PopupShown(rect), view)) => {
            if starts_outside_view(*rect) {
                (to(PopupFrame), vec![])
            } else {
                (
                    PageSmoke::Done,
                    vec![Action::Fail(format!(
                        "the popup {rect:?} does not start outside the {}x{} view",
                        view.width, view.height
                    ))],
                )
            }
        }
        (
            PopupFrame,
            Worked {
                popup_framed: true, ..
            },
        ) => (to(PopupHidden), vec![Action::Click(BLANK_AT)]),
        (PopupHidden, Page(PageEvent::PopupHidden, _)) => {
            (to(Dialog), vec![Action::Click(BUTTON_AT)])
        }
        (
            Dialog,
            Page(
                PageEvent::DialogRequested {
                    kind: JsDialogKind::Alert,
                    ..
                },
                _,
            ),
        ) => (to(Alerted), vec![Action::AcceptDialog]),
        (Alerted, Web(WebEvent::TitleChanged(title))) if title == ALERTED => {
            (to(Menu), vec![Action::RightClick(LINK_AT)])
        }
        (Menu, Page(PageEvent::ContextMenuRequested { menu, .. }, _)) => {
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
            Page(
                PageEvent::FindResult {
                    matches,
                    done: true,
                    ..
                },
                _,
            ),
        ) => match matches {
            2 => (to(Zoomed), vec![Action::ZoomIn]),
            n => (
                PageSmoke::Done,
                vec![Action::Fail(format!("found {n} of {NEEDLE:?}, not 2"))],
            ),
        },
        (Zoomed, Page(PageEvent::ZoomChanged { percent }, _)) if *percent > 100 => {
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

    const VIEW: FrameSize = FrameSize::new(800, 600);
    /// A popup right-aligned to a select at the right edge, wider than the view.
    const OUTSIDE: Rect = Rect {
        x: -108,
        y: 50,
        width: 1112,
        height: 92,
    };
    const WORK: Input = Input::Worked {
        framed: true,
        popup_framed: false,
        view_width: 800.0,
    };

    #[test]
    fn happy_path() {
        let loaded = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        let shown = PageEvent::PopupShown(OUTSIDE);
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
            Input::Page(&shown, VIEW),
            WORK,
            Input::Worked {
                framed: true,
                popup_framed: true,
                view_width: 800.0,
            },
            Input::Page(&PageEvent::PopupHidden, VIEW),
            Input::Page(&alert, VIEW),
            Input::Web(&title),
            Input::Page(&menu, VIEW),
            Input::Page(&found, VIEW),
            Input::Page(&zoom, VIEW),
        ]);
        assert_eq!(s, PageSmoke::Done);
        assert_eq!(
            actions,
            [
                Action::Click(Point {
                    x: 800.0 - SELECT_FROM_RIGHT,
                    y: SELECT_Y
                }),
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
    fn a_clamped_popup_fails() {
        // Clamped into the page: inside, or hanging off its right edge only.
        let inside = Rect { x: 0, ..OUTSIDE };
        let overflowing = Rect { x: 944, ..OUTSIDE };
        assert!(starts_outside_view(OUTSIDE));
        assert!(!starts_outside_view(inside));
        assert!(!starts_outside_view(overflowing));
        assert!(starts_outside_view(Rect { y: -10, ..inside }));
        let state = PageSmoke::At {
            step: Step::PopupShown,
            runs: 0,
        };
        let shown = PageEvent::PopupShown(overflowing);
        let (s, actions) = next(state, Input::Page(&shown, VIEW));
        assert_eq!(s, PageSmoke::Done);
        assert!(
            matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("outside")),
            "{actions:?}"
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
