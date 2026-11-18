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
//! 4. Right-click the link: the menu lists "Open Link in New Tab". CEF then
//!    cancels it itself, once per way it does (`CANCELS`: the page blurs, the
//!    page hides as on a tab switch, a left press reaches the page); each
//!    time the shell's menu closes with no answer left for the host. The next
//!    one is dismissed through the menu's path and answered exactly once.
//! 5. Find "needle" through the find bar: 2 matches.
//! 6. Zoom in: the page reports a zoom above 100 %.
//! 7. Open DevTools through F12's path: with the endpoint on a free loopback port the panel docks
//!    beside the page (the page's view gets narrower), shows a frame (a second frame stream) and
//!    its title starts with "DevTools" (the frontend reached the page); toggle it shut and the
//!    page's width is back.
//! 8. Right-click once more and quit with the menu open: closing the browser
//!    cancels it.
//!
//! Real pointer and key use of the popup, the dialog's feel and the
//! clipboard are left to a human.

use wubble_engine::input::Point;
use wubble_engine::nav::WebEvent;
use wubble_engine::page::{JsDialogKind, PageEvent, Rect};
use wubble_gfx::FrameSize;

use crate::menu::{self, NavFlags};

/// A pane is on screen next to the page after this many logical px of its width are taken.
pub const WIDTH_SLACK: u32 = 1;

/// Runs of CEF work one step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 2_000;

/// A loopback port nothing listens on, for the DevTools endpoint (bound and released; the
/// browser binds it again a moment later).
pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

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
    /// Waiting for the menu of round `n`: an index into `CANCELS`, or
    /// `CANCELS.len()` for the one the host dismisses.
    Menu(usize),
    /// Waiting for CEF to close the menu `CANCELS[n]` cancelled.
    Cancelled(usize),
    Found,
    Zoomed,
    /// Zoomed to this percent; the next run of work records the view's width.
    Docking(u32),
    /// DevTools toggled open at this view width (logical px, rounded).
    DockOpen(u32, u32),
    /// DevTools toggled shut: waiting for the page to have its width back.
    DockClosing(u32, u32),
    /// Waiting for the last menu, left open at exit, with the zoom to pass.
    Exit(u32),
}

/// The ways CEF cancels a page's context menu itself (the callers of
/// `CefMenuManager::CancelContextMenu` in its OSR view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cancel {
    /// `SetFocus(false)`: the page loses keyboard focus.
    Blur,
    /// `WasHidden`: its tab is switched away from.
    Hide,
    /// A left press on the page beside the menu.
    Press,
}

pub const CANCELS: [Cancel; 3] = [Cancel::Blur, Cancel::Hide, Cancel::Press];

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
        panel: Panel,
    },
    Web(&'a WebEvent),
    /// A page event, and the view's physical size.
    Page(&'a PageEvent, FrameSize),
}

/// What the DevTools dock shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Closed,
    /// DevTools is in its own window (the endpoint is off or failed).
    Detached,
    Docked {
        /// The panel has shown a frame.
        framed: bool,
        /// The frontend set its title: it reached the page over the endpoint.
        titled: bool,
    },
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
    /// Make CEF cancel the open menu this way.
    CancelMenu(Cancel),
    /// Undo what `CancelMenu` did to the page (its focus, its visibility).
    Restore(Cancel),
    /// CEF closed the menu: fails unless the shell's menu closed too and a
    /// dismissal answers nothing.
    MenuGone,
    Find(&'static str),
    ZoomIn,
    /// F12's path: open DevTools, or close it.
    ToggleDevTools,
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
            (to(Menu(0)), vec![Action::RightClick(LINK_AT)])
        }
        (Menu(round), Page(PageEvent::ContextMenuRequested { menu, .. }, _)) => {
            let entries = menu::entries(menu, NavFlags::default());
            if !entries.iter().any(|e| e.label == "Open Link in New Tab") {
                let fail = Action::Fail(format!("no link entries in {entries:?}"));
                (PageSmoke::Done, vec![fail])
            } else if let Some(&cancel) = CANCELS.get(round) {
                (to(Cancelled(round)), vec![Action::CancelMenu(cancel)])
            } else {
                (to(Found), vec![Action::DismissMenu, Action::Find(NEEDLE)])
            }
        }
        (Cancelled(round), Page(PageEvent::ContextMenuDismissed, _)) => (
            to(Menu(round + 1)),
            vec![
                Action::MenuGone,
                Action::Restore(CANCELS[round]),
                Action::RightClick(LINK_AT),
            ],
        ),
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
            (to(Docking(*percent)), vec![])
        }
        (Docking(percent), Worked { view_width, .. }) => (
            to(DockOpen(percent, view_width.round() as u32)),
            vec![Action::ToggleDevTools],
        ),
        (
            DockOpen(_, _),
            Worked {
                panel: Panel::Detached,
                ..
            },
        ) => (
            PageSmoke::Done,
            vec![Action::Fail(
                "DevTools opened in its own window, not docked: is the endpoint up?".into(),
            )],
        ),
        (
            DockOpen(percent, width),
            Worked {
                panel:
                    Panel::Docked {
                        framed: true,
                        titled: true,
                    },
                view_width,
                ..
            },
        ) => {
            if view_width.round() as u32 + WIDTH_SLACK < width {
                (
                    to(DockClosing(percent, width)),
                    vec![Action::ToggleDevTools],
                )
            } else {
                (
                    PageSmoke::Done,
                    vec![Action::Fail(format!(
                        "the docked panel left the page {view_width} px of its {width}"
                    ))],
                )
            }
        }
        (
            DockClosing(percent, width),
            Worked {
                panel: Panel::Closed,
                view_width,
                ..
            },
        ) if (view_width.round() as u32).abs_diff(width) <= WIDTH_SLACK => {
            (to(Exit(percent)), vec![Action::RightClick(LINK_AT)])
        }
        (Exit(percent), Page(PageEvent::ContextMenuRequested { .. }, _)) => {
            (PageSmoke::Done, vec![Action::Pass(percent)])
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
        panel: Panel::Closed,
    };
    const DOCKED: Input = Input::Worked {
        framed: true,
        popup_framed: false,
        view_width: 400.0,
        panel: Panel::Docked {
            framed: true,
            titled: true,
        },
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
                panel: Panel::Closed,
            },
            Input::Page(&PageEvent::PopupHidden, VIEW),
            Input::Page(&alert, VIEW),
            Input::Web(&title),
            Input::Page(&menu, VIEW),
            Input::Page(&PageEvent::ContextMenuDismissed, VIEW),
            Input::Page(&menu, VIEW),
            Input::Page(&PageEvent::ContextMenuDismissed, VIEW),
            Input::Page(&menu, VIEW),
            Input::Page(&PageEvent::ContextMenuDismissed, VIEW),
            Input::Page(&menu, VIEW),
            Input::Page(&found, VIEW),
            Input::Page(&zoom, VIEW),
            // the dock: opens at the page's width, docked and framed, shut again
            WORK,
            WORK,
            DOCKED,
            WORK,
            Input::Page(&menu, VIEW),
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
                Action::CancelMenu(Cancel::Blur),
                Action::MenuGone,
                Action::Restore(Cancel::Blur),
                Action::RightClick(LINK_AT),
                Action::CancelMenu(Cancel::Hide),
                Action::MenuGone,
                Action::Restore(Cancel::Hide),
                Action::RightClick(LINK_AT),
                Action::CancelMenu(Cancel::Press),
                Action::MenuGone,
                Action::Restore(Cancel::Press),
                Action::RightClick(LINK_AT),
                Action::DismissMenu,
                Action::Find(NEEDLE),
                Action::ZoomIn,
                Action::ToggleDevTools,
                Action::ToggleDevTools,
                Action::RightClick(LINK_AT),
                Action::Pass(110),
            ]
        );
    }

    fn worked(view_width: f32, panel: Panel) -> Input<'static> {
        Input::Worked {
            framed: true,
            popup_framed: false,
            view_width,
            panel,
        }
    }

    #[test]
    fn the_dock_waits_for_the_frontend_and_for_the_pages_width() {
        let at = |step| PageSmoke::At { step, runs: 0 };
        // still on `step`, one more run of work spent
        let waiting_on = |step| PageSmoke::At { step, runs: 1 };
        let attached = Panel::Docked {
            framed: true,
            titled: true,
        };
        let open = at(Step::DockOpen(110, 800));
        // framed but not titled: still waiting
        let waiting = Panel::Docked {
            framed: true,
            titled: false,
        };
        assert_eq!(
            next(open, worked(400.0, waiting)).0,
            waiting_on(Step::DockOpen(110, 800))
        );
        // attached, but the page did not make room
        let (s, actions) = next(open, worked(800.0, attached));
        assert_eq!(s, PageSmoke::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("px of its")));
        // its own window instead of the dock
        let (s, actions) = next(open, worked(800.0, Panel::Detached));
        assert_eq!(s, PageSmoke::Done);
        assert!(matches!(actions.as_slice(), [Action::Fail(e)] if e.contains("own window")));
        // closing: the page's width must be back before the smoke goes on
        let closing = at(Step::DockClosing(110, 800));
        assert_eq!(
            next(closing, worked(600.0, Panel::Closed)).0,
            waiting_on(Step::DockClosing(110, 800))
        );
        assert_eq!(
            next(closing, worked(800.0, attached)).0,
            waiting_on(Step::DockClosing(110, 800))
        );
        let (s, actions) = next(closing, worked(800.0, Panel::Closed));
        assert_eq!(s, at(Step::Exit(110)));
        assert_eq!(actions, [Action::RightClick(LINK_AT)]);
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
