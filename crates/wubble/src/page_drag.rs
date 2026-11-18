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
//! BSD-3-Clause, no code copied). Dragging out to other apps needs an OS drag
//! source (TODO.md).

use wubble_core::tab::TabId;
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
    },
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
    /// Escape, or the window lost the pointer.
    Cancelled,
}

/// Engine calls on the source page's view, and the host's own drop target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DragEffect {
    /// `DragTargetDragEnter` with what is dragged.
    Enter(DragData, DragOps),
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
}

/// The operation a drop does: Chromium's preference among those the page
/// allows (copy, then move, then link). CEF's `UpdateDragCursor` would name
/// the target's choice; welding does not report it yet.
pub fn drop_operation(allowed: DragOps) -> DragOps {
    [DragOps::COPY, DragOps::MOVE, DragOps::LINK]
        .into_iter()
        .find(|op| allowed.0 & op.0 != 0)
        .unwrap_or(DragOps::NONE)
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
                data: data.clone(),
                allowed,
                over: Over::Page,
            },
            vec![E::Enter(data, allowed), E::Over(allowed)],
        ),
        (
            PageDrag::Dragging {
                source,
                data,
                allowed,
                over,
            },
            DragInput::Moved(to),
        ) => {
            let effects = match (over, to) {
                (Over::Page, Over::Page) => vec![E::Over(allowed)],
                (Over::Page, _) => vec![E::Leave],
                (_, Over::Page) => vec![E::Enter(data.clone(), allowed), E::Over(allowed)],
                _ => vec![],
            };
            let next = PageDrag::Dragging {
                source,
                data,
                allowed,
                over: to,
            };
            (next, effects)
        }
        (
            PageDrag::Dragging {
                data,
                allowed,
                over,
                ..
            },
            DragInput::Released(at),
        ) => {
            let mut effects = match (over, at) {
                (Over::Page, Over::Page) => vec![],
                (Over::Page, _) => vec![E::Leave],
                (_, Over::Page) => vec![E::Enter(data.clone(), allowed)],
                _ => vec![],
            };
            match at {
                Over::Page => {
                    effects.extend([E::Over(allowed), E::Drop, E::End(drop_operation(allowed))])
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
        (PageDrag::Dragging { over, .. }, DragInput::Cancelled) => {
            let mut effects = if over == Over::Page {
                vec![E::Leave]
            } else {
                vec![]
            };
            effects.push(E::End(DragOps::NONE));
            (PageDrag::Idle, effects)
        }
        // A second start while dragging is CEF's to refuse (the engine's
        // `DragSource` gate); pointer input while idle is not a drag.
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
        assert_eq!(fx, [E::Enter(link(), ALL), E::Over(ALL)]);
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
        assert_eq!(fx, [E::Leave]);
        let (s, fx) = transition(s, DragInput::Moved(Over::Sidebar));
        assert!(fx.is_empty());
        let (_, fx) = transition(s, DragInput::Moved(Over::Page));
        assert_eq!(fx, [E::Enter(link(), ALL), E::Over(ALL)]);
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
}
