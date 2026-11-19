//! A page's own drag (selected text, a link, an image, an HTML5 `draggable`),
//! run inside the window.
//!
//! CEF hands a windowless browser's drags to the host (`StartDragging`) and
//! waits for the host to end them. Slint has no OS drag source, so the drag
//! stays inside the window: while the button is held, the pointer drives
//! CEF's drop-target calls on the page it started from instead of mouse
//! moves, and the release drops it there, or on the sidebar (a link opens as
//! a tab), or nowhere. The order of calls is the one CEF's own windowless
//! tests drive (chromiumembedded/cef `tests/ceftests/os_rendering_unittest.cc`,
//! `StartDragging`: `DragTargetDragEnter`, `DragTargetDragOver`,
//! `DragTargetDrop`, `DragSourceEndedAt`, `DragSourceSystemDragEnded`;
//! BSD-3-Clause, no code copied). Where the platform has an OS drag source
//! (Wayland, `crate::wl`), a drag that leaves the window becomes the OS's.

use wubble_core::tab::TabId;
use wubble_engine::nav::Cursor;
use wubble_engine::page::{DragData, DragOps};

/// Where the pointer is during a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Over {
    /// The page the drag started from.
    Page,
    /// The sidebar: a link dropped there opens as a tab.
    Sidebar,
    /// Anywhere else in the window: nothing takes the drop.
    Elsewhere,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PageDrag {
    #[default]
    Idle,
    Dragging {
        source: TabId,
        data: DragData,
        allowed: DragOps,
        over: Over,
        /// What the page last said a drop would do (CEF `UpdateDragCursor`);
        /// `None` until it says.
        chosen: Option<DragOps>,
    },
    /// The drag left the window and is the OS's (a Wayland drag source):
    /// it ends when the OS says.
    Outside { source: TabId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragInput {
    /// The page started a drag; the pointer is over it.
    Started {
        source: TabId,
        data: DragData,
        allowed: DragOps,
    },
    /// The pointer moved, still holding the button.
    Moved(Over),
    /// The button came up.
    Released(Over),
    /// The page named what a drop at the pointer would do.
    Cursor(DragOps),
    /// The pointer left the window with the button held, where the platform
    /// can hand the drag to the OS.
    LeftWindow,
    /// The OS drag ended with this operation done (`NONE`: cancelled).
    OsEnded(DragOps),
    /// Escape, or the window lost the pointer.
    Cancelled,
}

/// Engine calls on the source page's view, and the host's own drop target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragEffect {
    /// `DragTargetDragEnter` with the page's own drag data (custom types
    /// included; `DragPhase::EnterSource`).
    Enter(DragOps),
    /// `DragTargetDragOver` at the pointer.
    Over(DragOps),
    /// `DragTargetDragLeave`.
    Leave,
    /// `DragTargetDrop` at the pointer.
    Drop,
    /// `DragSourceEndedAt` with the operation done, then
    /// `DragSourceSystemDragEnded`.
    End(DragOps),
    /// Open the URL in a new tab after the source's.
    OpenTab(String),
    /// Show the pointer for this operation ([`cursor`]).
    Feedback(DragOps),
    /// Hand the drag to the OS with what it carries.
    StartOs(DragData, DragOps),
}

/// The operation a drop does before the page has said: Chromium's
/// preference among those the page allows (copy, then move, then link).
pub fn drop_operation(allowed: DragOps) -> DragOps {
    [DragOps::COPY, DragOps::MOVE, DragOps::LINK]
        .into_iter()
        .find(|op| allowed.0 & op.0 != 0)
        .unwrap_or(DragOps::NONE)
}

/// The pointer during a drag that would do `op`, as Chromium's drag
/// feedback: copy and link have their own, a move the plain arrow.
pub fn cursor(op: DragOps) -> Cursor {
    if op.0 & DragOps::COPY.0 != 0 {
        Cursor::Copy
    } else if op.0 & DragOps::MOVE.0 != 0 {
        Cursor::Default
    } else if op.0 & DragOps::LINK.0 != 0 {
        Cursor::Alias
    } else {
        Cursor::NotAllowed
    }
}

pub fn transition(state: PageDrag, input: DragInput) -> (PageDrag, Vec<DragEffect>) {
    use DragEffect as E;
    match (state, input) {
        (
            PageDrag::Idle,
            DragInput::Started {
                source,
                data,
                allowed,
            },
        ) => (
            PageDrag::Dragging {
                source,
                data,
                allowed,
                over: Over::Page,
                chosen: None,
            },
            vec![E::Enter(allowed), E::Over(allowed)],
        ),
        (
            PageDrag::Dragging {
                source,
                data,
                allowed,
                over,
                chosen,
            },
            DragInput::Moved(to),
        ) => {
            let effects = match (over, to) {
                (Over::Page, Over::Page) => vec![E::Over(allowed)],
                (Over::Page, _) => vec![E::Leave, E::Feedback(DragOps::NONE)],
                (_, Over::Page) => vec![E::Enter(allowed), E::Over(allowed)],
                _ => vec![],
            };
            // Off the page, nothing the page said holds.
            let chosen = if to == Over::Page { chosen } else { None };
            let next = PageDrag::Dragging {
                source,
                data,
                allowed,
                over: to,
                chosen,
            };
            (next, effects)
        }
        (
            PageDrag::Dragging {
                source,
                data,
                allowed,
                over: Over::Page,
                ..
            },
            DragInput::Cursor(op),
        ) => {
            let next = PageDrag::Dragging {
                source,
                data,
                allowed,
                over: Over::Page,
                chosen: Some(op),
            };
            (next, vec![E::Feedback(op)])
        }
        (
            PageDrag::Dragging {
                data,
                allowed,
                over,
                chosen,
                ..
            },
            DragInput::Released(at),
        ) => {
            let mut effects = match (over, at) {
                (Over::Page, Over::Page) => vec![],
                (Over::Page, _) => vec![E::Leave],
                (_, Over::Page) => vec![E::Enter(allowed)],
                _ => vec![],
            };
            match at {
                Over::Page => {
                    let op = chosen.unwrap_or_else(|| drop_operation(allowed));
                    effects.extend([E::Over(allowed), E::Drop, E::End(op)])
                }
                Over::Sidebar => match data.link_url {
                    Some(url) if !url.is_empty() => {
                        let op = drop_operation(DragOps(
                            allowed.0 & (DragOps::COPY.0 | DragOps::LINK.0),
                        ));
                        effects.extend([E::End(op), E::OpenTab(url)]);
                    }
                    _ => effects.push(E::End(DragOps::NONE)),
                },
                Over::Elsewhere => effects.push(E::End(DragOps::NONE)),
            }
            (PageDrag::Idle, effects)
        }
        (
            PageDrag::Dragging {
                source,
                data,
                allowed,
                over,
                ..
            },
            DragInput::LeftWindow,
        ) => {
            let mut effects = if over == Over::Page {
                vec![E::Leave]
            } else {
                vec![]
            };
            effects.push(E::StartOs(data, allowed));
            (PageDrag::Outside { source }, effects)
        }
        (PageDrag::Outside { .. }, DragInput::OsEnded(op)) => (PageDrag::Idle, vec![E::End(op)]),
        (PageDrag::Dragging { over, .. }, DragInput::Cancelled) => {
            let mut effects = if over == Over::Page {
                vec![E::Leave]
            } else {
                vec![]
            };
            effects.push(E::End(DragOps::NONE));
            (PageDrag::Idle, effects)
        }
        (PageDrag::Outside { .. }, DragInput::Cancelled) => {
            (PageDrag::Idle, vec![E::End(DragOps::NONE)])
        }
        // A second start while dragging is CEF's to refuse (the engine's
        // `DragSource` gate); pointer input while idle or outside is not
        // this drag's; a report off the page is stale.
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{DragEffect as E, *};

    const ALL: DragOps = DragOps(DragOps::COPY.0 | DragOps::LINK.0 | DragOps::MOVE.0);

    fn link() -> DragData {
        DragData {
            link_url: Some("https://a.test/".into()),
            ..DragData::default()
        }
    }

    fn started(data: DragData) -> PageDrag {
        transition(
            PageDrag::Idle,
            DragInput::Started {
                source: TabId(1),
                data,
                allowed: ALL,
            },
        )
        .0
    }

    #[test]
    fn a_drag_enters_its_page_at_once() {
        let (s, fx) = transition(
            PageDrag::Idle,
            DragInput::Started {
                source: TabId(1),
                data: link(),
                allowed: ALL,
            },
        );
        assert!(matches!(
            s,
            PageDrag::Dragging {
                over: Over::Page,
                ..
            }
        ));
        assert_eq!(fx, [E::Enter(ALL), E::Over(ALL)]);
    }

    #[test]
    fn dropping_on_the_page_drops_then_ends_with_the_operation() {
        let (s, fx) = transition(started(link()), DragInput::Moved(Over::Page));
        assert_eq!(fx, [E::Over(ALL)]);
        let (s, fx) = transition(s, DragInput::Released(Over::Page));
        assert_eq!(s, PageDrag::Idle);
        assert_eq!(fx, [E::Over(ALL), E::Drop, E::End(DragOps::COPY)]);
    }

    #[test]
    fn leaving_and_coming_back_enters_again() {
        let (s, fx) = transition(started(link()), DragInput::Moved(Over::Elsewhere));
        assert_eq!(fx, [E::Leave, E::Feedback(DragOps::NONE)]);
        let (s, fx) = transition(s, DragInput::Moved(Over::Sidebar));
        assert!(fx.is_empty());
        let (_, fx) = transition(s, DragInput::Moved(Over::Page));
        assert_eq!(fx, [E::Enter(ALL), E::Over(ALL)]);
    }

    #[test]
    fn a_link_dropped_on_the_sidebar_opens_a_tab() {
        let (s, _) = transition(started(link()), DragInput::Moved(Over::Sidebar));
        let (s, fx) = transition(s, DragInput::Released(Over::Sidebar));
        assert_eq!(s, PageDrag::Idle);
        assert_eq!(
            fx,
            [E::End(DragOps::COPY), E::OpenTab("https://a.test/".into())]
        );
        // Text is not a tab.
        let text = DragData {
            text: Some("words".into()),
            ..DragData::default()
        };
        let (_, fx) = transition(started(text), DragInput::Released(Over::Sidebar));
        assert_eq!(fx, [E::Leave, E::End(DragOps::NONE)]);
    }

    #[test]
    fn released_elsewhere_or_cancelled_ends_with_nothing_done() {
        let (_, fx) = transition(started(link()), DragInput::Released(Over::Elsewhere));
        assert_eq!(fx, [E::Leave, E::End(DragOps::NONE)]);
        let (s, fx) = transition(started(link()), DragInput::Cancelled);
        assert_eq!(s, PageDrag::Idle);
        assert_eq!(fx, [E::Leave, E::End(DragOps::NONE)]);
    }

    #[test]
    fn pointer_input_while_idle_is_not_a_drag() {
        for input in [
            DragInput::Moved(Over::Page),
            DragInput::Released(Over::Page),
            DragInput::Cancelled,
        ] {
            assert_eq!(transition(PageDrag::Idle, input), (PageDrag::Idle, vec![]));
        }
    }

    #[test]
    fn the_drop_prefers_copy_then_move_then_link() {
        assert_eq!(drop_operation(ALL), DragOps::COPY);
        assert_eq!(
            drop_operation(DragOps(DragOps::MOVE.0 | DragOps::LINK.0)),
            DragOps::MOVE
        );
        assert_eq!(drop_operation(DragOps::LINK), DragOps::LINK);
        assert_eq!(drop_operation(DragOps::NONE), DragOps::NONE);
    }

    #[test]
    fn the_page_s_choice_is_the_drop_s_operation() {
        let (s, fx) = transition(started(link()), DragInput::Cursor(DragOps::MOVE));
        assert_eq!(fx, [E::Feedback(DragOps::MOVE)]);
        let (_, fx) = transition(s.clone(), DragInput::Released(Over::Page));
        assert_eq!(fx, [E::Over(ALL), E::Drop, E::End(DragOps::MOVE)]);
        // Leaving the page forgets it; a stale report off the page is ignored.
        let (s, _) = transition(s, DragInput::Moved(Over::Elsewhere));
        let (s, fx) = transition(s, DragInput::Cursor(DragOps::MOVE));
        assert!(fx.is_empty());
        let (s, _) = transition(s, DragInput::Moved(Over::Page));
        let (_, fx) = transition(s, DragInput::Released(Over::Page));
        assert_eq!(fx.last(), Some(&E::End(DragOps::COPY)));
    }

    #[test]
    fn leaving_the_window_hands_the_drag_to_the_os_once() {
        let (s, fx) = transition(started(link()), DragInput::LeftWindow);
        assert_eq!(s, PageDrag::Outside { source: TabId(1) });
        assert_eq!(fx, [E::Leave, E::StartOs(link(), ALL)]);
        let (s, fx) = transition(s, DragInput::LeftWindow);
        assert!(fx.is_empty());
        let (s, fx) = transition(s, DragInput::Released(Over::Page));
        assert!(fx.is_empty(), "the OS has the drag");
        let (s, fx) = transition(s, DragInput::OsEnded(DragOps::COPY));
        assert_eq!(s, PageDrag::Idle);
        assert_eq!(fx, [E::End(DragOps::COPY)]);
        let (_, fx) = transition(PageDrag::Idle, DragInput::OsEnded(DragOps::COPY));
        assert!(fx.is_empty());
    }

    #[test]
    fn cancelling_an_os_drag_ends_it() {
        let (s, _) = transition(started(link()), DragInput::Moved(Over::Elsewhere));
        let (s, fx) = transition(s, DragInput::LeftWindow);
        assert_eq!(fx, [E::StartOs(link(), ALL)]);
        let (s, fx) = transition(s, DragInput::Cancelled);
        assert_eq!(s, PageDrag::Idle);
        assert_eq!(fx, [E::End(DragOps::NONE)]);
    }

    #[test]
    fn the_pointer_shows_the_operation() {
        assert_eq!(cursor(DragOps::COPY), Cursor::Copy);
        assert_eq!(cursor(DragOps::MOVE), Cursor::Default);
        assert_eq!(cursor(DragOps::LINK), Cursor::Alias);
        assert_eq!(cursor(DragOps::NONE), Cursor::NotAllowed);
    }
}
