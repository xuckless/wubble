//! `--smoke dnd`: a page's drags inside the window, driven by real pointer input through the
//! host on `dnd.html` (served on loopback, [`home`]): a link into a text field, a selection into
//! a textarea, a picture onto a drop zone, an HTML `draggable` with its own data and drag image
//! onto another element and into the other split pane, a link onto the tab list and the New tab
//! row, text onto the address bar and into the open command bar, a link onto the bookmarks page, a rest on a tab row showing that tab, Escape cancelling; the
//! ghost is checked drawn and shot mid-drag (`--shot-dir`). The driver and executor are
//! `--smoke parity`'s ([`crate::parity::next_in`], `host/parity.rs`). Bounded by counts of CEF
//! work, never by time.

use crate::parity::{Button::Left, Expect, HostCheck as C, HostOp, Item, NONE, RESET, Spot, Step};
use Expect::{Host as H, Js as J};
use Step::*;
use wubble_engine::input::Point;

/// The page on the smoke's loopback server (`autofill_smoke::serve`).
pub fn home(port: u16) -> String {
    format!("http://127.0.0.1:{port}/dnd")
}

/// Where `dnd.html` puts things (view-relative logical px).
mod at {
    use wubble_engine::input::Point;
    const fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }
    pub const fn x(at: Point, x: f32) -> Point {
        p(x, at.y)
    }
    pub const LINK: Point = p(40.0, 33.0);
    pub const INP: Point = p(150.0, 74.0);
    /// On "alpha" of `alpha beta gamma`.
    pub const WORD: Point = p(40.0, 120.0);
    pub const TA: Point = p(150.0, 180.0);
    pub const IMG: Point = p(80.0, 275.0);
    pub const DRAG: Point = p(350.0, 40.0);
    pub const DROP: Point = p(400.0, 140.0);
    pub const ZONE: Point = p(400.0, 275.0);
    /// An empty part of the page.
    pub const EMPTY: Point = p(200.0, 450.0);
}

const fn pane(n: usize, p: Point) -> Spot {
    Spot::Pane(n, p)
}

/// A press on `from`, moves past the drag threshold, then waits for the page's drag.
macro_rules! start {
    ($from:expr) => {
        [
            Press($from),
            MoveTo(at::x($from, $from.x + 12.0)),
            MoveTo(at::x($from, $from.x + 30.0)),
            Until(H(C::Drag(true))),
        ]
    };
}

macro_rules! item {
    ($name:literal, [$($step:expr),* $(,)?], [$($expect:expr),* $(,)?]) => {
        Item {
            name: $name,
            steps: &[
                RESET[0], RESET[1], RESET[2], RESET[3], RESET[4], RESET[5], RESET[6], RESET[7],
                $($step),*
            ],
            expect: &[$($expect),*],
            gap: false,
        }
    };
}

const LINK: [Step; 4] = start!(at::LINK);
const WORD: [Step; 4] = start!(at::WORD);
const IMG: [Step; 4] = start!(at::IMG);
const DRAG: [Step; 4] = start!(at::DRAG);

pub const ITEMS: &[Item] = &[
    item!(
        "dnd.link-into-input",
        [
            // the first item can begin while the page still fades in (its entrance, app.slint
            // `page-enter`): a shot then shows the pane empty
            Until(H(C::PageEntered)),
            LINK[0],
            LINK[1],
            LINK[2],
            LINK[3],
            Until(H(C::Ghost(true))),
            MoveTo(at::EMPTY),
            Settle(6),
            Host(HostOp::Shot("dnd-ghost-link")),
            MoveTo(at::x(at::INP, 120.0)),
            MoveTo(at::INP),
            Settle(10),
            Release(at::INP),
            Settle(10)
        ],
        [
            J("$('inp').value.endsWith('/dnd/linked') || $('inp').value"),
            J("eq(seen.dragend, 'copy')"),
            H(C::Drag(false))
        ]
    ),
    item!(
        "dnd.selection-into-textarea",
        [
            Js("parity.select(0, 5)"),
            WORD[0],
            WORD[1],
            WORD[2],
            WORD[3],
            Until(H(C::Ghost(true))),
            MoveTo(at::x(at::TA, 120.0)),
            MoveTo(at::TA),
            Settle(10),
            Host(HostOp::Shot("dnd-ghost-text")),
            Release(at::TA),
            Settle(10)
        ],
        [J("eq($('ta').value, 'alpha')")]
    ),
    item!(
        "dnd.image-onto-zone",
        [
            IMG[0],
            IMG[1],
            IMG[2],
            IMG[3],
            Until(H(C::Ghost(true))),
            MoveTo(at::x(at::ZONE, 300.0)),
            MoveTo(at::ZONE),
            Settle(10),
            Host(HostOp::Shot("dnd-ghost-image")),
            Release(at::ZONE),
            Settle(10)
        ],
        [
            J("has(seen.types, ['text/uri-list', 'text/html', 'Files'])"),
            J("eq(seen.files, 1)")
        ]
    ),
    item!(
        "dnd.draggable-custom-data-and-image",
        [
            DRAG[0],
            DRAG[1],
            DRAG[2],
            DRAG[3],
            Until(H(C::Ghost(true))),
            MoveTo(at::x(at::DROP, 380.0)),
            MoveTo(at::DROP),
            Settle(10),
            Host(HostOp::Shot("dnd-ghost-draggable")),
            Release(at::DROP),
            Settle(10)
        ],
        [
            J("eq(seen.custom, 'custom')"),
            J("eq(seen.plain, 'payload')"),
            J("eq(seen.dragend, 'copy')")
        ]
    ),
    item!(
        "dnd.escape-cancels",
        [
            LINK[0],
            LINK[1],
            LINK[2],
            LINK[3],
            MoveTo(at::EMPTY),
            Settle(6),
            Keys("Escape"),
            Settle(10)
        ],
        [H(C::Drag(false)), J("eq(seen.dragend, 'none')")]
    ),
    item!(
        "dnd.link-onto-tab-list",
        [
            LINK[0],
            LINK[1],
            LINK[2],
            LINK[3],
            MoveToSpot(Spot::TabGap(1)),
            Settle(6),
            Host(HostOp::Shot("dnd-ghost-over-tab-list")),
            ReleaseAt(Spot::TabGap(0)),
            Settle(20)
        ],
        [
            H(C::Tabs(2)),
            H(C::TabUrl(0, "/dnd/linked")),
            H(C::Visible(0)),
            H(C::Drag(false))
        ]
    ),
    item!(
        "dnd.link-onto-new-tab-row",
        [
            LINK[0],
            LINK[1],
            LINK[2],
            LINK[3],
            MoveToSpot(Spot::NewTab),
            Settle(6),
            ReleaseAt(Spot::NewTab),
            Settle(20)
        ],
        [H(C::Tabs(2)), H(C::TabUrl(1, "/dnd/linked"))]
    ),
    item!(
        "dnd.text-onto-address-bar",
        [
            Js("parity.select(0, 5)"),
            WORD[0],
            WORD[1],
            WORD[2],
            WORD[3],
            MoveToSpot(Spot::Bar),
            Settle(6),
            ReleaseAt(Spot::Bar),
            Settle(10),
            Until(H(C::BarOpen(true)))
        ],
        [H(C::BarQuery("alpha")), H(C::Tabs(1))]
    ),
    item!(
        "dnd.text-onto-the-open-command-bar",
        [
            // the bar's scrim takes a press, so the drag comes from another app
            Click(Left, NONE, at::EMPTY),
            Keys("Ctrl+L"),
            Until(H(C::BarOpen(true))),
            Settle(10),
            Host(HostOp::DropTextFromApp(Spot::CommandField, "alpha")),
            Settle(10)
        ],
        [H(C::BarOpen(true)), H(C::BarQuery("alpha")), H(C::Tabs(1))]
    ),
    item!(
        "dnd.link-onto-the-bookmarks-page",
        [
            Host(HostOp::BookmarksTab),
            Settle(20),
            Until(H(C::Visible(0))),
            // showing the first tab again replays its entrance: a press before it ends lands
            // where the page is drawn then (scaled in), off the link
            Until(H(C::PageEntered)),
            LINK[0],
            LINK[1],
            LINK[2],
            LINK[3],
            // rest on its row: the page shows, then over it
            MoveToSpot(Spot::TabRow(1)),
            Settle(4),
            MoveToSpot(Spot::TabRow(1)),
            Until(H(C::Visible(1))),
            MoveToSpot(pane(0, at::x(at::DROP, 300.0))),
            MoveToSpot(pane(0, at::DROP)),
            Settle(10),
            Host(HostOp::Shot("dnd-over-bookmarks")),
            ReleaseAt(pane(0, at::DROP)),
            Settle(20)
        ],
        [H(C::Bookmarked("/dnd/linked")), H(C::Drag(false))]
    ),
    item!(
        "dnd.rest-on-a-tab-row-shows-it",
        [
            Host(HostOp::SecondTab),
            Settle(20),
            Until(H(C::Visible(0))),
            Until(H(C::PageEntered)),
            LINK[0],
            LINK[1],
            LINK[2],
            LINK[3],
            MoveToSpot(Spot::TabRow(1)),
            Settle(4),
            MoveToSpot(Spot::TabRow(1)),
            Until(H(C::Visible(1))),
            Keys("Escape"),
            Settle(10)
        ],
        [H(C::Visible(1)), H(C::Drag(false))]
    ),
    // A person who shows a tab and at once presses a link on it presses where the link is drawn
    // while the page slides in: the press reaches the link (Slint maps it through the entrance)
    // and the drag starts. The second tab shown and loaded first; then each try shows the other
    // tab, its entrance anew.
    item!(
        "dnd.press-during-the-entrance-starts-a-drag",
        [
            Host(HostOp::SecondTab),
            Host(HostOp::NextTab),
            Until(H(C::Visible(1))),
            Until(H(C::TabTitle(1, "wubble dnd"))),
            Until(H(C::PageEntered)),
            Watch(HostOp::NextTab, H(C::PageEntering)),
            PressAt(Spot::Drawn(at::LINK)),
            MoveToSpot(Spot::Drawn(at::x(at::LINK, at::LINK.x + 12.0))),
            MoveToSpot(Spot::Drawn(at::x(at::LINK, at::LINK.x + 30.0))),
            Until(H(C::Drag(true))),
            Keys("Escape"),
            Settle(10)
        ],
        [H(C::Drag(false)), J("eq(seen.dragend, 'none')")]
    ),
    item!(
        "dnd.draggable-into-the-other-pane",
        [
            Host(HostOp::SplitHome),
            Settle(30),
            Until(H(C::PageEntered)),
            Settle(60),
            DRAG[0],
            DRAG[1],
            DRAG[2],
            DRAG[3],
            MoveToSpot(pane(1, at::x(at::DROP, 200.0))),
            MoveToSpot(pane(1, at::DROP)),
            Settle(10),
            Host(HostOp::Shot("dnd-ghost-split")),
            ReleaseAt(pane(1, at::DROP)),
            Settle(20),
            Until(H(C::TabTitle(1, "drop:custom")))
        ],
        [H(C::Drag(false)), J("eq(seen.dragend, 'copy')")]
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = ITEMS.iter().map(|i| i.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ITEMS.len());
    }
}
