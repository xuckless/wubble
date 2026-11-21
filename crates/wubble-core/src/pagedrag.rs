//! A page's own drag (selected text, a link, an image, an HTML5 `draggable`): where the
//! pointer is, what each target hears, and what a drop does. One kind of [`crate::drag::Drag`].
//!
//! CEF hands a windowless browser's drags to the host (`StartDragging`) and waits for the host
//! to end them (`DragSourceEndedAt`, `DragSourceSystemDragEnded`). While the button is held
//! the pointer drives CEF's drop-target calls on the page under it: the page the drag started
//! from or another pane's (`DragTargetDragEnter/Over/Leave/Drop`, the order CEF's own
//! windowless tests drive: chromiumembedded/cef `tests/ceftests/os_rendering_unittest.cc`,
//! BSD-3-Clause, no code copied). Over the shell the drop is the shell's, as Chrome's
//! `BrowserRootView` takes drops on the tab strip and the omnibox takes text:
//! - the tab list: a new tab with the link (or the text, searched) at the gap under the
//!   pointer (`NavigateToDroppedUrls`, chrome/browser/ui/views/frame/browser_root_view.cc);
//!   resting [`DWELL`] on a row's middle shows that tab, so its page can take the drop;
//! - the New tab row: a new tab at the end;
//! - the address bar: its text is the link or the text, selected, for Enter (Chrome's
//!   `OmniboxViewViews::PerformDrop` fills, it does not navigate);
//! - the Spaces switcher: resting [`DWELL`] opens the popup; a Space row opens the link there.
//!
//! Where the platform has an OS drag source (Wayland, `crate::wl` in the binary), a drag that
//! leaves the window becomes the OS's. A drag from a tab an agent session drives is confined:
//! only its own page hears it; the shell, other pages and other apps see nothing.

use std::time::{Duration, Instant};

use crate::space::SpaceId;
use crate::tab::TabId;

/// How long a drag rests on a tab row before that tab shows, and on the Spaces switcher before
/// the popup opens: Chromium's `kShowDropTargetForLinkDelay`
/// (chrome/browser/ui/views/frame/multi_contents_view_drop_target_controller.h), its one
/// hover delay for a dragged link.
pub const DWELL: Duration = Duration::from_millis(500);

/// A set of drag operations (CEF's `cef_drag_operations_mask_t`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DragOps(pub u32);

impl DragOps {
    pub const NONE: Self = Self(0);
    pub const COPY: Self = Self(1 << 0);
    pub const LINK: Self = Self(1 << 1);
    pub const MOVE: Self = Self(1 << 4);
    /// Every operation: what a drop from another app offers.
    pub const ANY: Self = Self(Self::COPY.0 | Self::LINK.0 | Self::MOVE.0);

    pub fn has(self, op: Self) -> bool {
        self.0 & op.0 != 0
    }
}

/// What the machine needs to know of a drag's data.
pub trait Payload: Clone {
    /// The link's (or the dragged picture's) URL.
    fn link(&self) -> Option<&str>;
    /// The selection's plain text.
    fn text(&self) -> Option<&str>;
}

/// What a drop on the shell opens or fills: the link, else the text.
pub fn dropped_text<P: Payload>(data: &P) -> Option<String> {
    data.link()
        .or(data.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// A shell target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    /// The tab list: a drop inserts at `index`; `row` is the tab whose row's middle the pointer
    /// is on (a rest there shows it).
    List {
        index: usize,
        row: Option<TabId>,
    },
    NewTab,
    /// The address (command) bar.
    Bar,
    /// The footer's Spaces switcher.
    SpacesTrigger,
    /// A Space row of the open Spaces popup.
    Space(SpaceId),
}

/// Where the pointer is during a drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Over {
    /// A page on screen: the source's or another pane's.
    Page(TabId),
    Shell(Shell),
    /// Anywhere else in the window: nothing takes the drop.
    Elsewhere,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dragging<P> {
    pub source: TabId,
    pub data: P,
    pub allowed: DragOps,
    pub over: Over,
    /// What the page under the pointer last said a drop would do (CEF `UpdateDragCursor`).
    pub chosen: Option<DragOps>,
    /// When the pointer came over `over`.
    pub since: Instant,
    /// The rest on `over` did what it does (a tab shown, the popup opened).
    pub dwelt: bool,
    /// This drag opened the Spaces popup.
    pub popup: bool,
    /// An agent session's tab: only its own page hears the drag.
    pub confined: bool,
    /// Released over a page: the drop waits for the page's answer to the drag there (CEF
    /// `UpdateDragCursor`), or a [`DWELL`] when it says none or nothing.
    pub dropping: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageDrag<P> {
    Dragging(Dragging<P>),
    /// The drag left the window and is the OS's: it ends when the OS says.
    Outside {
        source: TabId,
    },
}

impl<P> PageDrag<P> {
    pub fn source(&self) -> TabId {
        match self {
            PageDrag::Dragging(d) => d.source,
            PageDrag::Outside { source } => *source,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input<P> {
    /// A page started a drag; the pointer is over it.
    Started {
        source: TabId,
        data: P,
        allowed: DragOps,
        confined: bool,
        now: Instant,
    },
    /// The pointer moved, still holding the button.
    Moved { over: Over, now: Instant },
    /// A timer while the pointer rests (the dwell runs on).
    Tick { now: Instant },
    /// The button came up.
    Released(Over),
    /// `tab` named what a drop at the pointer would do.
    Cursor { tab: TabId, op: DragOps },
    /// The pointer left the window with the button held, where the platform can hand the drag
    /// to the OS.
    LeftWindow,
    /// The OS drag ended with this operation done (`NONE`: cancelled).
    OsEnded(DragOps),
    /// Escape, or the window lost focus or the pointer.
    Cancelled,
    /// `tab` closed or navigated away.
    Gone(TabId),
}

/// Engine calls and the shell's drops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect<P> {
    /// `DragTargetDragEnter` on `tab` with the source page's own data (custom types included).
    Enter {
        tab: TabId,
        allowed: DragOps,
    },
    /// `DragTargetDragOver` at the pointer.
    Over {
        tab: TabId,
        allowed: DragOps,
    },
    Leave {
        tab: TabId,
    },
    Drop {
        tab: TabId,
    },
    /// `DragSourceEndedAt` on the source with the operation done, then
    /// `DragSourceSystemDragEnded`.
    End(DragOps),
    /// A new tab with this link or text (the host searches text as the bar does), at `index` in
    /// the list (`None`: at the end), shown.
    OpenTab {
        text: String,
        index: Option<usize>,
    },
    /// The bar opens holding this text, all of it selected.
    FillBar(String),
    /// A new tab in this Space with the link or text; that Space shows.
    OpenInSpace {
        text: String,
        space: SpaceId,
    },
    /// Show this tab (the drag rested on its row).
    Activate(TabId),
    OpenSpaces,
    CloseSpaces,
    /// Show the pointer for this operation.
    Feedback(DragOps),
    /// The shell's drop indicator on this target, or none.
    Mark(Option<Shell>),
    /// Hand the drag to the OS with what it carries.
    StartOs(P, DragOps),
}

/// What a drop of `text` (a link or text) on `at` does; the switcher takes none. A drag from
/// another app dropped on the shell does the same.
pub fn shell_drop<P>(at: Shell, text: String) -> Option<Effect<P>> {
    match at {
        Shell::List { index, .. } => Some(Effect::OpenTab {
            text,
            index: Some(index),
        }),
        Shell::NewTab => Some(Effect::OpenTab { text, index: None }),
        Shell::Bar => Some(Effect::FillBar(text)),
        Shell::Space(space) => Some(Effect::OpenInSpace { text, space }),
        Shell::SpacesTrigger => None,
    }
}

/// The operation a drop does before the page has said: Chromium's preference among those the
/// page allows (copy, then move, then link).
pub fn drop_operation(allowed: DragOps) -> DragOps {
    [DragOps::COPY, DragOps::MOVE, DragOps::LINK]
        .into_iter()
        .find(|op| allowed.has(*op))
        .unwrap_or(DragOps::NONE)
}

/// What a drop on the shell does: a copy (or a link) when it carries a link or text and the
/// target takes it; the switcher only opens the popup.
fn shell_operation<P: Payload>(data: &P, allowed: DragOps, at: Shell) -> DragOps {
    if at == Shell::SpacesTrigger || dropped_text(data).is_none() {
        return DragOps::NONE;
    }
    drop_operation(DragOps(allowed.0 & (DragOps::COPY.0 | DragOps::LINK.0)))
}

/// Where a confined drag may be: its own page, else nowhere.
fn confine<P>(d: &Dragging<P>, over: Over) -> Over {
    match over {
        Over::Page(t) if !d.confined || t == d.source => over,
        _ if d.confined => Over::Elsewhere,
        _ => over,
    }
}

/// The effects of the pointer leaving `from` for `to` (`to` differs).
fn cross<P: Payload>(d: &Dragging<P>, from: Over, to: Over) -> Vec<Effect<P>> {
    let mut fx = vec![];
    if let Over::Page(tab) = from {
        fx.push(Effect::Leave { tab });
    }
    if matches!(from, Over::Shell(_)) {
        fx.push(Effect::Mark(None));
    }
    match to {
        Over::Page(tab) => fx.extend([
            Effect::Enter {
                tab,
                allowed: d.allowed,
            },
            Effect::Over {
                tab,
                allowed: d.allowed,
            },
        ]),
        Over::Shell(s) => {
            let op = shell_operation(&d.data, d.allowed, s);
            if op != DragOps::NONE {
                fx.push(Effect::Mark(Some(s)));
            }
            fx.push(Effect::Feedback(op));
        }
        Over::Elsewhere => fx.push(Effect::Feedback(DragOps::NONE)),
    }
    fx
}

/// The same target, a row's middle (`List { row }`) counted as its own target for the dwell.
fn same(a: Over, b: Over) -> bool {
    match (a, b) {
        (Over::Shell(Shell::List { row: r1, .. }), Over::Shell(Shell::List { row: r2, .. })) => {
            r1 == r2
        }
        _ => a == b,
    }
}

/// The rest on a row shows its tab; on the switcher opens the popup.
fn dwell<P>(mut d: Dragging<P>, now: Instant) -> (Dragging<P>, Vec<Effect<P>>) {
    if d.dwelt || now.saturating_duration_since(d.since) < DWELL {
        return (d, vec![]);
    }
    let fx = match d.over {
        Over::Shell(Shell::List { row: Some(tab), .. }) => vec![Effect::Activate(tab)],
        Over::Shell(Shell::SpacesTrigger) if !d.popup => {
            d.popup = true;
            vec![Effect::OpenSpaces]
        }
        _ => return (d, vec![]),
    };
    d.dwelt = true;
    (d, fx)
}

impl<P> Dragging<P> {
    /// Whether a rest would still do something: the host keeps a timer for [`Input::Tick`].
    pub fn awaits_dwell(&self) -> bool {
        self.dropping
            || !self.dwelt
                && matches!(
                    self.over,
                    Over::Shell(Shell::List { row: Some(_), .. } | Shell::SpacesTrigger)
                )
    }
}

/// The drag's start: it is over its page.
pub fn start<P: Payload>(input: Input<P>) -> Option<(PageDrag<P>, Vec<Effect<P>>)> {
    let Input::Started {
        source,
        data,
        allowed,
        confined,
        now,
    } = input
    else {
        return None;
    };
    let fx = vec![
        Effect::Enter {
            tab: source,
            allowed,
        },
        Effect::Over {
            tab: source,
            allowed,
        },
    ];
    let d = Dragging {
        source,
        data,
        allowed,
        over: Over::Page(source),
        chosen: None,
        since: now,
        dwelt: false,
        popup: false,
        confined,
        dropping: false,
    };
    Some((PageDrag::Dragging(d), fx))
}

/// The end of a drag that was in the window: the target's effects, then the source's end and
/// the popup closed if this drag opened it. `None` state: idle.
fn ended<P>(d: &Dragging<P>, mut fx: Vec<Effect<P>>, op: DragOps) -> Vec<Effect<P>> {
    fx.push(Effect::End(op));
    if d.popup {
        fx.push(Effect::CloseSpaces);
    }
    fx
}

/// The next state (`None`: idle) and its effects.
pub fn transition<P: Payload>(
    state: PageDrag<P>,
    input: Input<P>,
) -> (Option<PageDrag<P>>, Vec<Effect<P>>) {
    use Effect as E;
    let mut d = match state {
        PageDrag::Dragging(d) => d,
        PageDrag::Outside { source } => {
            return match input {
                Input::OsEnded(op) => (None, vec![E::End(op)]),
                Input::Cancelled => (None, vec![E::End(DragOps::NONE)]),
                Input::Gone(t) if t == source => (None, vec![E::End(DragOps::NONE)]),
                // the OS has the pointer
                _ => (Some(PageDrag::Outside { source }), vec![]),
            };
        }
    };
    if d.dropping {
        return dropping(d, input);
    }
    match input {
        Input::Moved { over, now } => {
            let over = confine(&d, over);
            let mut fx = if over == d.over {
                match over {
                    Over::Page(tab) => vec![E::Over {
                        tab,
                        allowed: d.allowed,
                    }],
                    _ => vec![],
                }
            } else {
                cross(&d, d.over, over)
            };
            if !same(over, d.over) {
                d.since = now;
                d.dwelt = false;
            }
            if over != d.over {
                d.chosen = None;
            }
            // a gap's mark moves with the pointer
            if let (Over::Shell(s @ Shell::List { .. }), Over::Shell(Shell::List { .. })) =
                (over, d.over)
                && over != d.over
                && shell_operation(&d.data, d.allowed, s) != DragOps::NONE
            {
                fx.retain(|e| !matches!(e, E::Mark(_)));
                fx.insert(0, E::Mark(Some(s)));
            }
            d.over = over;
            let (d, more) = dwell(d, now);
            fx.extend(more);
            (Some(PageDrag::Dragging(d)), fx)
        }
        Input::Tick { now } => {
            let (d, fx) = dwell(d, now);
            (Some(PageDrag::Dragging(d)), fx)
        }
        Input::Cursor { tab, op } if d.over == Over::Page(tab) => {
            d.chosen = Some(op);
            (Some(PageDrag::Dragging(d)), vec![E::Feedback(op)])
        }
        Input::Released(at) => {
            let at = confine(&d, at);
            let mut fx = if at == d.over {
                vec![]
            } else {
                // the release point is where the drop goes: arrive there first
                cross(&d, d.over, at)
                    .into_iter()
                    .filter(|e| !matches!(e, E::Feedback(_) | E::Mark(Some(_))))
                    .collect()
            };
            if matches!(d.over, Over::Shell(_)) && at == d.over {
                fx.push(E::Mark(None));
            }
            let (op, open) = match at {
                // the page's answer to the drag at the release point first (it may come late)
                Over::Page(tab) => {
                    fx.push(E::Over {
                        tab,
                        allowed: d.allowed,
                    });
                    d.over = at;
                    d.chosen = None;
                    d.dropping = true;
                    return (Some(PageDrag::Dragging(d)), fx);
                }
                Over::Shell(s) => {
                    let op = shell_operation(&d.data, d.allowed, s);
                    let open = dropped_text(&d.data)
                        .filter(|_| op != DragOps::NONE)
                        .and_then(|text| shell_drop(s, text));
                    (op, open)
                }
                Over::Elsewhere => (DragOps::NONE, None),
            };
            let mut fx = ended(&d, fx, op);
            fx.extend(open);
            (None, fx)
        }
        Input::LeftWindow if d.confined => {
            let over = Over::Elsewhere;
            let fx = if d.over == over {
                vec![]
            } else {
                cross(&d, d.over, over)
            };
            d.over = over;
            (Some(PageDrag::Dragging(d)), fx)
        }
        Input::LeftWindow => {
            let mut fx = match d.over {
                Over::Page(tab) => vec![E::Leave { tab }],
                Over::Shell(_) => vec![E::Mark(None)],
                Over::Elsewhere => vec![],
            };
            if d.popup {
                fx.push(E::CloseSpaces);
            }
            fx.push(E::StartOs(d.data, d.allowed));
            (Some(PageDrag::Outside { source: d.source }), fx)
        }
        Input::Cancelled => {
            let fx = match d.over {
                Over::Page(tab) => vec![E::Leave { tab }],
                Over::Shell(_) => vec![E::Mark(None)],
                Over::Elsewhere => vec![],
            };
            (None, ended(&d, fx, DragOps::NONE))
        }
        Input::Gone(tab) if tab == d.source => {
            let fx = match d.over {
                Over::Page(t) if t != tab => vec![E::Leave { tab: t }],
                Over::Shell(_) => vec![E::Mark(None)],
                _ => vec![],
            };
            (None, ended(&d, fx, DragOps::NONE))
        }
        Input::Gone(tab) if d.over == Over::Page(tab) => {
            d.over = Over::Elsewhere;
            d.chosen = None;
            (
                Some(PageDrag::Dragging(d)),
                vec![E::Leave { tab }, E::Feedback(DragOps::NONE)],
            )
        }
        // A second start is the engine's to refuse; a page's report off its page is stale; the
        // OS's end of a drag it never had is not this one's.
        _ => (Some(PageDrag::Dragging(d)), vec![]),
    }
}

/// A release over a page, waiting for its answer: an operation drops at once; none, or none
/// in time, drops once a [`DWELL`] has run (the page decides, as Blink does on any drop).
fn dropping<P>(d: Dragging<P>, input: Input<P>) -> (Option<PageDrag<P>>, Vec<Effect<P>>) {
    use Effect as E;
    let Over::Page(tab) = d.over else {
        return (None, ended(&d, vec![], DragOps::NONE));
    };
    let drop = |d: &Dragging<P>, op| ended(d, vec![E::Drop { tab }], op);
    match input {
        Input::Cursor { tab: t, op } if t == tab && op != DragOps::NONE => (None, drop(&d, op)),
        Input::Cursor { tab: t, op } if t == tab => {
            let d = Dragging {
                chosen: Some(op),
                ..d
            };
            (Some(PageDrag::Dragging(d)), vec![])
        }
        Input::Tick { .. } => {
            let op = d.chosen.unwrap_or_else(|| drop_operation(d.allowed));
            (None, drop(&d, op))
        }
        Input::Cancelled => (None, ended(&d, vec![E::Leave { tab }], DragOps::NONE)),
        Input::Gone(t) if t == tab => (None, ended(&d, vec![], DragOps::NONE)),
        Input::Gone(t) if t == d.source => (None, ended(&d, vec![E::Leave { tab }], DragOps::NONE)),
        // the button is up: the pointer is no drag's
        _ => (Some(PageDrag::Dragging(d)), vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect as E, *};

    const ALL: DragOps = DragOps::ANY;
    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    #[derive(Clone, Debug, PartialEq, Eq, Default)]
    struct Data {
        link: Option<String>,
        text: Option<String>,
    }

    impl Payload for Data {
        fn link(&self) -> Option<&str> {
            self.link.as_deref()
        }
        fn text(&self) -> Option<&str> {
            self.text.as_deref()
        }
    }

    fn link() -> Data {
        Data {
            link: Some("https://a.test/".into()),
            text: None,
        }
    }

    fn text() -> Data {
        Data {
            link: None,
            text: Some("words".into()),
        }
    }

    fn t0() -> Instant {
        static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *T0.get_or_init(Instant::now)
    }

    fn ms(n: u64) -> Instant {
        t0() + Duration::from_millis(n)
    }

    fn started_as(data: Data, confined: bool) -> PageDrag<Data> {
        start(Input::Started {
            source: A,
            data,
            allowed: ALL,
            confined,
            now: t0(),
        })
        .unwrap()
        .0
    }

    fn started(data: Data) -> PageDrag<Data> {
        started_as(data, false)
    }

    fn step(s: PageDrag<Data>, input: Input<Data>) -> (PageDrag<Data>, Vec<Effect<Data>>) {
        let (s, fx) = transition(s, input);
        (s.expect("still dragging"), fx)
    }

    fn mv(s: PageDrag<Data>, over: Over, at: u64) -> (PageDrag<Data>, Vec<Effect<Data>>) {
        step(s, Input::Moved { over, now: ms(at) })
    }

    fn list(index: usize, row: Option<TabId>) -> Over {
        Over::Shell(Shell::List { index, row })
    }

    #[test]
    fn a_drag_enters_its_page_at_once() {
        let (s, fx) = start(Input::Started {
            source: A,
            data: link(),
            allowed: ALL,
            confined: false,
            now: t0(),
        })
        .unwrap();
        assert!(matches!(&s, PageDrag::Dragging(d) if d.over == Over::Page(A)));
        assert_eq!(
            fx,
            [
                E::Enter {
                    tab: A,
                    allowed: ALL
                },
                E::Over {
                    tab: A,
                    allowed: ALL
                }
            ]
        );
        assert!(start(Input::<Data>::Cancelled).is_none());
    }

    #[test]
    fn dropping_on_the_page_drops_then_ends_with_the_operation() {
        let (s, fx) = mv(started(link()), Over::Page(A), 5);
        assert_eq!(
            fx,
            [E::Over {
                tab: A,
                allowed: ALL
            }]
        );
        let (s, fx) = step(s, Input::Released(Over::Page(A)));
        assert_eq!(
            fx,
            [E::Over {
                tab: A,
                allowed: ALL
            }]
        );
        // the pointer after the release is nothing; the page's answer drops
        let (s, fx) = mv(s, Over::Elsewhere, 6);
        assert!(fx.is_empty());
        let (s, fx) = transition(
            s,
            Input::Cursor {
                tab: A,
                op: DragOps::COPY,
            },
        );
        assert!(s.is_none());
        assert_eq!(fx, [E::Drop { tab: A }, E::End(DragOps::COPY)]);
    }

    #[test]
    fn a_slow_or_refusing_page_gets_the_drop_after_a_while() {
        let (s, _) = step(started(link()), Input::Released(Over::Page(A)));
        let PageDrag::Dragging(d) = &s else { panic!() };
        assert!(d.awaits_dwell());
        // none said (maybe an answer to an earlier move): wait on
        let (s, fx) = step(
            s,
            Input::Cursor {
                tab: A,
                op: DragOps::NONE,
            },
        );
        assert!(fx.is_empty());
        let (s, fx) = transition(s, Input::Tick { now: ms(600) });
        assert!(s.is_none());
        assert_eq!(fx, [E::Drop { tab: A }, E::End(DragOps::NONE)]);
        // no answer at all: Chromium's default
        let (s, _) = step(started(link()), Input::Released(Over::Page(A)));
        let (_, fx) = transition(s, Input::Tick { now: ms(600) });
        assert_eq!(fx, [E::Drop { tab: A }, E::End(DragOps::COPY)]);
        // Escape while waiting
        let (s, _) = step(started(link()), Input::Released(Over::Page(A)));
        let (s, fx) = transition(s, Input::Cancelled);
        assert!(s.is_none());
        assert_eq!(fx, [E::Leave { tab: A }, E::End(DragOps::NONE)]);
    }

    #[test]
    fn another_pane_hears_the_drag_and_takes_the_drop() {
        let (s, fx) = mv(started(link()), Over::Page(B), 5);
        assert_eq!(
            fx,
            [
                E::Leave { tab: A },
                E::Enter {
                    tab: B,
                    allowed: ALL
                },
                E::Over {
                    tab: B,
                    allowed: ALL
                }
            ]
        );
        // the source's report is stale there; the target's counts
        let (s, fx) = step(
            s,
            Input::Cursor {
                tab: A,
                op: DragOps::MOVE,
            },
        );
        assert!(fx.is_empty());
        let (s, fx) = step(
            s,
            Input::Cursor {
                tab: B,
                op: DragOps::MOVE,
            },
        );
        assert_eq!(fx, [E::Feedback(DragOps::MOVE)]);
        let (s, fx) = step(s, Input::Released(Over::Page(B)));
        assert_eq!(
            fx,
            [E::Over {
                tab: B,
                allowed: ALL
            }]
        );
        let (_, fx) = transition(
            s,
            Input::Cursor {
                tab: B,
                op: DragOps::MOVE,
            },
        );
        assert_eq!(fx, [E::Drop { tab: B }, E::End(DragOps::MOVE)]);
    }

    #[test]
    fn leaving_and_coming_back_enters_again() {
        let (s, fx) = mv(started(link()), Over::Elsewhere, 5);
        assert_eq!(fx, [E::Leave { tab: A }, E::Feedback(DragOps::NONE)]);
        let (s, fx) = mv(s, Over::Elsewhere, 6);
        assert!(fx.is_empty());
        let (_, fx) = mv(s, Over::Page(A), 7);
        assert_eq!(
            fx,
            [
                E::Enter {
                    tab: A,
                    allowed: ALL
                },
                E::Over {
                    tab: A,
                    allowed: ALL
                }
            ]
        );
    }

    #[test]
    fn a_link_on_the_tab_list_opens_a_tab_at_the_gap() {
        let (s, fx) = mv(started(link()), list(2, None), 5);
        assert_eq!(
            fx,
            [
                E::Leave { tab: A },
                E::Mark(Some(Shell::List {
                    index: 2,
                    row: None
                })),
                E::Feedback(DragOps::COPY)
            ]
        );
        // the mark follows the gap
        let (s, fx) = mv(s, list(3, None), 6);
        assert_eq!(
            fx,
            [
                E::Mark(Some(Shell::List {
                    index: 3,
                    row: None
                })),
                E::Feedback(DragOps::COPY)
            ]
        );
        let (s, fx) = transition(s, Input::Released(list(3, None)));
        assert!(s.is_none());
        assert_eq!(
            fx,
            [
                E::Mark(None),
                E::End(DragOps::COPY),
                E::OpenTab {
                    text: "https://a.test/".into(),
                    index: Some(3)
                }
            ]
        );
    }

    #[test]
    fn text_on_the_new_tab_row_and_the_bar() {
        let (s, _) = mv(started(text()), Over::Shell(Shell::NewTab), 5);
        let (_, fx) = transition(s, Input::Released(Over::Shell(Shell::NewTab)));
        assert_eq!(
            fx.last(),
            Some(&E::OpenTab {
                text: "words".into(),
                index: None
            })
        );
        let (s, fx) = mv(started(text()), Over::Shell(Shell::Bar), 5);
        assert!(fx.contains(&E::Mark(Some(Shell::Bar))));
        let (_, fx) = transition(s, Input::Released(Over::Shell(Shell::Bar)));
        assert_eq!(fx.last(), Some(&E::FillBar("words".into())));
    }

    #[test]
    fn released_on_a_target_not_yet_entered_arrives_there_first() {
        let (_, fx) = transition(started(link()), Input::Released(Over::Shell(Shell::Bar)));
        assert_eq!(
            fx,
            [
                E::Leave { tab: A },
                E::End(DragOps::COPY),
                E::FillBar("https://a.test/".into())
            ]
        );
        let (s, _) = mv(started(link()), Over::Elsewhere, 5);
        let (_, fx) = transition(s, Input::Released(Over::Page(B)));
        assert_eq!(
            fx,
            [
                E::Enter {
                    tab: B,
                    allowed: ALL
                },
                E::Over {
                    tab: B,
                    allowed: ALL
                },
                E::Over {
                    tab: B,
                    allowed: ALL
                }
            ]
        );
    }

    #[test]
    fn a_drag_with_nothing_to_open_is_refused_by_the_shell() {
        let empty = Data::default();
        let (s, fx) = mv(started(empty), Over::Shell(Shell::NewTab), 5);
        assert_eq!(fx, [E::Leave { tab: A }, E::Feedback(DragOps::NONE)]);
        let (_, fx) = transition(s, Input::Released(Over::Shell(Shell::NewTab)));
        assert_eq!(fx, [E::Mark(None), E::End(DragOps::NONE)]);
    }

    #[test]
    fn resting_on_a_row_shows_its_tab_once() {
        let row = list(1, Some(B));
        let (s, fx) = mv(started(link()), row, 100);
        assert!(!fx.contains(&E::Activate(B)));
        let PageDrag::Dragging(d) = &s else { panic!() };
        assert!(d.awaits_dwell());
        let (s, fx) = mv(s, row, 300);
        assert!(!fx.contains(&E::Activate(B)));
        let (s, fx) = step(s, Input::Tick { now: ms(600) });
        assert_eq!(fx, [E::Activate(B)]);
        let (s, fx) = step(s, Input::Tick { now: ms(1200) });
        assert!(fx.is_empty());
        // another row's middle starts its own rest; a gap is none
        let (s, _) = mv(s, list(2, Some(TabId(3))), 1300);
        let (s, fx) = step(s, Input::Tick { now: ms(1850) });
        assert_eq!(fx, [E::Activate(TabId(3))]);
        let (s, _) = mv(s, list(2, None), 1900);
        let (_, fx) = step(s, Input::Tick { now: ms(3000) });
        assert!(fx.is_empty());
    }

    #[test]
    fn resting_on_the_switcher_opens_the_spaces_and_a_space_takes_the_link() {
        let (s, _) = mv(started(link()), Over::Shell(Shell::SpacesTrigger), 100);
        let (s, fx) = step(s, Input::Tick { now: ms(700) });
        assert_eq!(fx, [E::OpenSpaces]);
        let space = Over::Shell(Shell::Space(SpaceId(4)));
        let (s, fx) = mv(s, space, 800);
        assert!(fx.contains(&E::Mark(Some(Shell::Space(SpaceId(4))))));
        let (_, fx) = transition(s, Input::Released(space));
        assert_eq!(
            fx,
            [
                E::Mark(None),
                E::End(DragOps::COPY),
                E::CloseSpaces,
                E::OpenInSpace {
                    text: "https://a.test/".into(),
                    space: SpaceId(4)
                }
            ]
        );
    }

    #[test]
    fn released_elsewhere_or_cancelled_ends_with_nothing_done() {
        let (_, fx) = transition(started(link()), Input::Released(Over::Elsewhere));
        assert_eq!(fx, [E::Leave { tab: A }, E::End(DragOps::NONE)]);
        let (s, fx) = transition(started(link()), Input::Cancelled);
        assert!(s.is_none());
        assert_eq!(fx, [E::Leave { tab: A }, E::End(DragOps::NONE)]);
        let (s, _) = mv(started(link()), Over::Shell(Shell::Bar), 5);
        let (_, fx) = transition(s, Input::Cancelled);
        assert_eq!(fx, [E::Mark(None), E::End(DragOps::NONE)]);
    }

    #[test]
    fn the_page_s_choice_is_the_drop_s_operation_until_the_pointer_leaves() {
        let (s, fx) = step(
            started(link()),
            Input::Cursor {
                tab: A,
                op: DragOps::MOVE,
            },
        );
        assert_eq!(fx, [E::Feedback(DragOps::MOVE)]);
        let (s, _) = mv(s, Over::Elsewhere, 5);
        let PageDrag::Dragging(d) = &s else { panic!() };
        assert_eq!(d.chosen, None, "off the page, nothing the page said holds");
    }

    #[test]
    fn leaving_the_window_hands_the_drag_to_the_os_once() {
        let (s, fx) = step(started(link()), Input::LeftWindow);
        assert_eq!(s, PageDrag::Outside { source: A });
        assert_eq!(fx, [E::Leave { tab: A }, E::StartOs(link(), ALL)]);
        let (s, fx) = step(s, Input::LeftWindow);
        assert!(fx.is_empty());
        let (s, fx) = step(s, Input::Released(Over::Page(A)));
        assert!(fx.is_empty(), "the OS has the drag");
        let (s, fx) = transition(s, Input::OsEnded(DragOps::COPY));
        assert!(s.is_none());
        assert_eq!(fx, [E::End(DragOps::COPY)]);
        let (s, fx) = transition(PageDrag::Outside { source: A }, Input::<Data>::Cancelled);
        assert!(s.is_none());
        assert_eq!(fx, [E::End(DragOps::NONE)]);
    }

    #[test]
    fn an_agent_s_drag_stays_on_its_page() {
        let s = started_as(link(), true);
        // another pane and the shell are nowhere to it
        let (s, fx) = mv(s, Over::Page(B), 5);
        assert_eq!(fx, [E::Leave { tab: A }, E::Feedback(DragOps::NONE)]);
        let (s, fx) = mv(s, Over::Shell(Shell::NewTab), 6);
        assert!(fx.is_empty());
        // no OS drag
        let (s, fx) = step(s, Input::LeftWindow);
        assert!(matches!(s, PageDrag::Dragging(_)));
        assert!(fx.is_empty());
        let (_, fx) = transition(s.clone(), Input::Released(Over::Shell(Shell::Bar)));
        assert_eq!(fx, [E::End(DragOps::NONE)]);
        let (_, fx) = transition(s, Input::Released(Over::Page(A)));
        assert!(fx.contains(&E::Over {
            tab: A,
            allowed: ALL
        }));
    }

    #[test]
    fn a_tab_gone_mid_drag() {
        // the target's: it leaves, the drag goes on
        let (s, _) = mv(started(link()), Over::Page(B), 5);
        let (s, fx) = step(s, Input::Gone(B));
        assert_eq!(fx, [E::Leave { tab: B }, E::Feedback(DragOps::NONE)]);
        let (_, fx) = transition(s.clone(), Input::Released(Over::Elsewhere));
        assert_eq!(fx, [E::End(DragOps::NONE)]);
        // the source's: the drag ends with nothing
        let (s, _) = mv(s, Over::Page(TabId(3)), 6);
        let (s, fx) = transition(s, Input::Gone(A));
        assert!(s.is_none());
        assert_eq!(fx, [E::Leave { tab: TabId(3) }, E::End(DragOps::NONE)]);
        let (s, fx) = transition(PageDrag::Outside { source: A }, Input::<Data>::Gone(A));
        assert!(s.is_none());
        assert_eq!(fx, [E::End(DragOps::NONE)]);
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
        // a move-only drag is no copy for the shell
        let (_, fx) = transition(
            start(Input::Started {
                source: A,
                data: link(),
                allowed: DragOps::MOVE,
                confined: false,
                now: t0(),
            })
            .unwrap()
            .0,
            Input::Released(Over::Shell(Shell::NewTab)),
        );
        assert_eq!(fx, [E::Leave { tab: A }, E::End(DragOps::NONE)]);
    }
}
