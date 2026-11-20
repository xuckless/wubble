//! Where a dragged tab goes: one state enum and one pure transition.
//!
//! Inside the tab list the row physics (`tabs::TabRows`, the design's reorder) keeps owning
//! the drag; this machine only decides what a drag that leaves the list means: onto the
//! footer's Spaces switcher (its "^" or the current Space's tile: after a hover of
//! [`SPACE_HOVER`] the Spaces popup opens, and a release on one of its Space rows moves the tab
//! there, on its New Space row into a new Space; the popup closes when the drag ends or has
//! been off it for [`SPACE_HOVER`]), onto the content area's left or right edge (a split), onto
//! another wubble window, or outside every window (a new window).
//!
//! Written for wubble: the design repo has reorder and click-to-switch only (no drop onto a
//! Space, no hover dwell, no drop-to-split, no tear-out). The dwell is the Finder/Firefox
//! spring-loading idea; its length is the owner's to tune (owner, 2026-11-20: "when drag on top
//! of the ^ the popup opens and place the tab into the desired space"). Time is passed in
//! (`now`), never read here.
//!
//! On Wayland a client cannot learn global pointer or window positions, so the host can never
//! name another window under the pointer there: a release outside the window is always
//! `Outside` (a new window). Other platforms may report `Window`.

use std::time::{Duration, Instant};

use crate::space::SpaceId;
use crate::tab::TabId;
use crate::tabs::TABS;
use crate::window::{WindowId, WindowTarget};

/// How long a dragged tab hovers over the Spaces switcher before the popup opens, and how long
/// it may be off the open popup before the popup closes.
pub const SPACE_HOVER: Duration = Duration::from_millis(500);
/// The share of the content area's width, at each side, that drops into a split.
pub const EDGE_SHARE: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.w && p.y >= self.y && p.y < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// What is under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    /// Nothing that takes a tab (the toolbar, the content area's middle).
    None,
    /// The tab list: the row physics reorders.
    List,
    /// The footer's switcher: the "^" or the current Space's tile.
    SpacesTrigger,
    /// The open Spaces popup, on none of its targets (a rule, the current Space's row).
    SpacesMenu,
    /// A row of the open Spaces popup.
    Space(SpaceId),
    /// The popup's New Space row.
    NewSpace,
    SplitEdge(Side),
    /// Another wubble window (never on Wayland).
    Window(WindowId),
    /// Beyond this window and every other wubble window.
    Outside,
}

/// The window's drop targets, in its logical coordinates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rects {
    /// The window itself: (0, 0, width, height).
    pub window: Option<Rect>,
    pub list: Option<Rect>,
    /// The footer's switcher.
    pub trigger: Option<Rect>,
    /// The open Spaces popup's panel, its Space rows (but the current Space's) and its New
    /// Space row.
    pub menu: Option<Rect>,
    pub spaces: Vec<(SpaceId, Rect)>,
    pub new_space: Option<Rect>,
    pub content: Option<Rect>,
    /// Other wubble windows, where the platform tells (empty on Wayland).
    pub windows: Vec<(WindowId, Rect)>,
}

/// The drop target at `p`.
pub fn hit(p: Point, r: &Rects) -> DropTarget {
    let inside = |rect: &Option<Rect>| rect.is_some_and(|r| r.contains(p));
    if !inside(&r.window) {
        return r
            .windows
            .iter()
            .find(|(_, rect)| rect.contains(p))
            .map_or(DropTarget::Outside, |(w, _)| DropTarget::Window(*w));
    }
    if let Some((s, _)) = r.spaces.iter().find(|(_, rect)| rect.contains(p)) {
        return DropTarget::Space(*s);
    }
    if inside(&r.new_space) {
        return DropTarget::NewSpace;
    }
    if inside(&r.menu) {
        return DropTarget::SpacesMenu;
    }
    if inside(&r.trigger) {
        return DropTarget::SpacesTrigger;
    }
    if inside(&r.list) {
        return DropTarget::List;
    }
    match r.content.filter(|c| c.contains(p)) {
        Some(c) if p.x < c.x + c.w * EDGE_SHARE => DropTarget::SplitEdge(Side::Left),
        Some(c) if p.x >= c.x + c.w * (1.0 - EDGE_SHARE) => DropTarget::SplitEdge(Side::Right),
        _ => DropTarget::None,
    }
}

impl DropTarget {
    /// The switcher or the open popup: the pointer keeps the popup open.
    fn keeps_popup(self) -> bool {
        matches!(
            self,
            DropTarget::SpacesTrigger
                | DropTarget::SpacesMenu
                | DropTarget::Space(_)
                | DropTarget::NewSpace
        )
    }
}

/// The Spaces popup, as this drag opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popup {
    Closed,
    Open,
    /// Open, the pointer off it since then.
    Left(Instant),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drag {
    Idle,
    /// Pressed on a row, not yet past the drag threshold: a release is a click.
    Pressed {
        tab: TabId,
        at: Point,
    },
    Dragging {
        tab: TabId,
        over: DropTarget,
        /// When the pointer came over `over`.
        since: Instant,
        popup: Popup,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    Press {
        tab: TabId,
        at: Point,
    },
    Move {
        at: Point,
        hit: DropTarget,
        now: Instant,
    },
    /// A frame or timer while the pointer rests (the hover delays run on).
    Tick {
        now: Instant,
    },
    Release {
        hit: DropTarget,
    },
    /// Esc, or the platform took the pointer away.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The Spaces popup opens (the tab is still held), through the overlays.
    OpenSpaces,
    CloseSpaces,
    MoveToSpace {
        tab: TabId,
        space: SpaceId,
    },
    /// A new Space (`Model::new_space`), then the tab moves there.
    MoveToNewSpace {
        tab: TabId,
    },
    SplitInto {
        tab: TabId,
        side: Side,
    },
    MoveToWindow {
        tab: TabId,
        to: WindowTarget,
    },
    /// The drag ended away from the list: the row physics must not also act on the release.
    CancelRows,
}

impl Drag {
    /// The tab the ghost shows, while the pointer is out of the list.
    pub fn ghost(&self) -> Option<TabId> {
        match *self {
            Drag::Dragging { tab, over, .. } if over != DropTarget::List => Some(tab),
            _ => None,
        }
    }

    /// Whether a hover delay runs (the pointer on the switcher with the popup closed, or off
    /// the open popup): a later [`Input::Tick`] may open or close it, so the host keeps a timer
    /// running.
    pub fn awaits_dwell(&self) -> bool {
        matches!(
            *self,
            Drag::Dragging {
                over: DropTarget::SpacesTrigger,
                popup: Popup::Closed,
                ..
            } | Drag::Dragging {
                popup: Popup::Left(_),
                ..
            }
        )
    }

    /// The popup this drag opened is up.
    pub fn popup_open(&self) -> bool {
        matches!(
            *self,
            Drag::Dragging {
                popup: Popup::Open | Popup::Left(_),
                ..
            }
        )
    }

    /// The target the pointer is over, while dragging.
    pub fn over(&self) -> Option<DropTarget> {
        match *self {
            Drag::Dragging { over, .. } => Some(over),
            _ => None,
        }
    }
}

/// The one transition.
pub fn transition(d: Drag, input: Input) -> (Drag, Vec<Effect>) {
    match (d, input) {
        (_, Input::Press { tab, at }) => (Drag::Pressed { tab, at }, vec![]),
        (Drag::Pressed { tab, at }, Input::Move { at: p, hit, now }) => {
            if (p.x - at.x).hypot(p.y - at.y) <= TABS::dragStart {
                return (d, vec![]);
            }
            let next = Drag::Dragging {
                tab,
                over: hit,
                since: now,
                popup: Popup::Closed,
            };
            dwell(next, now)
        }
        (
            Drag::Dragging {
                tab,
                over,
                since,
                popup,
            },
            Input::Move { hit, now, .. },
        ) => {
            let since = if hit == over { since } else { now };
            let popup = match popup {
                Popup::Closed => Popup::Closed,
                _ if hit.keeps_popup() => Popup::Open,
                Popup::Open => Popup::Left(now),
                left => left,
            };
            dwell(
                Drag::Dragging {
                    tab,
                    over: hit,
                    since,
                    popup,
                },
                now,
            )
        }
        (Drag::Dragging { .. }, Input::Tick { now }) => dwell(d, now),
        (Drag::Dragging { tab, .. }, Input::Release { hit }) => {
            let close = d.popup_open().then_some(Effect::CloseSpaces);
            let drop = match hit {
                // the row physics reorders
                DropTarget::List => return (Drag::Idle, close.into_iter().collect()),
                DropTarget::Space(space) => Some(Effect::MoveToSpace { tab, space }),
                DropTarget::NewSpace => Some(Effect::MoveToNewSpace { tab }),
                DropTarget::SplitEdge(side) => Some(Effect::SplitInto { tab, side }),
                DropTarget::Window(w) => Some(Effect::MoveToWindow {
                    tab,
                    to: WindowTarget::Existing(w),
                }),
                DropTarget::Outside => Some(Effect::MoveToWindow {
                    tab,
                    to: WindowTarget::New,
                }),
                DropTarget::None | DropTarget::SpacesTrigger | DropTarget::SpacesMenu => None,
            };
            (
                Drag::Idle,
                std::iter::once(Effect::CancelRows)
                    .chain(drop)
                    .chain(close)
                    .collect(),
            )
        }
        (Drag::Dragging { .. }, Input::Cancel) => (
            Drag::Idle,
            std::iter::once(Effect::CancelRows)
                .chain(d.popup_open().then_some(Effect::CloseSpaces))
                .collect(),
        ),
        (Drag::Pressed { .. }, Input::Release { .. } | Input::Cancel) => (Drag::Idle, vec![]),
        (d, _) => (d, vec![]),
    }
}

/// Opens the Spaces popup once the pointer has rested on the switcher for [`SPACE_HOVER`];
/// closes it once the pointer has been off it as long.
fn dwell(d: Drag, now: Instant) -> (Drag, Vec<Effect>) {
    let waited = |t: Instant| now.saturating_duration_since(t) >= SPACE_HOVER;
    match d {
        Drag::Dragging {
            tab,
            over: DropTarget::SpacesTrigger,
            since,
            popup: Popup::Closed,
        } if waited(since) => (
            Drag::Dragging {
                tab,
                over: DropTarget::SpacesTrigger,
                since,
                popup: Popup::Open,
            },
            vec![Effect::OpenSpaces],
        ),
        Drag::Dragging {
            tab,
            over,
            since,
            popup: Popup::Left(left),
        } if waited(left) => (
            Drag::Dragging {
                tab,
                over,
                since,
                popup: Popup::Closed,
            },
            vec![Effect::CloseSpaces],
        ),
        d => (d, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: TabId = TabId(7);
    const S: SpaceId = SpaceId(2);

    fn t0() -> Instant {
        static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *T0.get_or_init(Instant::now)
    }

    fn at(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn mv(d: Drag, x: f64, hit: DropTarget, ms: u64) -> (Drag, Vec<Effect>) {
        transition(
            d,
            Input::Move {
                at: at(x, 10.0),
                hit,
                now: t0() + Duration::from_millis(ms),
            },
        )
    }

    /// Pressed at (0, 10) and dragged past the threshold over `hit`.
    fn dragging(hit: DropTarget) -> Drag {
        let (d, _) = transition(
            Drag::Idle,
            Input::Press {
                tab: T,
                at: at(0.0, 10.0),
            },
        );
        let (d, fx) = mv(d, 50.0, hit, 0);
        assert!(fx.is_empty());
        assert!(matches!(d, Drag::Dragging { .. }));
        d
    }

    fn release(d: Drag, hit: DropTarget) -> Vec<Effect> {
        let (d, fx) = transition(d, Input::Release { hit });
        assert_eq!(d, Drag::Idle);
        fx
    }

    #[test]
    fn under_the_threshold_it_is_a_click() {
        let (d, _) = transition(
            Drag::Idle,
            Input::Press {
                tab: T,
                at: at(0.0, 10.0),
            },
        );
        let (d, fx) = mv(d, TABS::dragStart - 1.0, DropTarget::Outside, 0);
        assert!(fx.is_empty() && matches!(d, Drag::Pressed { .. }));
        assert!(
            release(d, DropTarget::Outside).is_empty(),
            "the rows pick it"
        );
    }

    #[test]
    fn in_the_list_the_rows_reorder() {
        let d = dragging(DropTarget::List);
        assert_eq!(d.ghost(), None, "the row itself moves");
        assert_eq!(release(d, DropTarget::List), vec![]);
    }

    fn tick(d: Drag, ms: u64) -> (Drag, Vec<Effect>) {
        transition(
            d,
            Input::Tick {
                now: t0() + Duration::from_millis(ms),
            },
        )
    }

    const HOVER: u64 = SPACE_HOVER.as_millis() as u64;

    #[test]
    fn hovering_the_switcher_opens_the_popup_once_after_the_delay() {
        let d = dragging(DropTarget::List);
        let (d, fx) = mv(d, 60.0, DropTarget::SpacesTrigger, 100);
        assert!(fx.is_empty());
        assert_eq!(d.ghost(), Some(T));
        assert!(d.awaits_dwell());
        let (d, fx) = tick(d, 100 + HOVER - 1);
        assert!(fx.is_empty(), "not before the delay");
        let (d, fx) = mv(d, 61.0, DropTarget::SpacesTrigger, 100 + HOVER);
        assert_eq!(fx, vec![Effect::OpenSpaces]);
        assert!(d.popup_open() && !d.awaits_dwell());
        let (d, fx) = mv(d, 62.0, DropTarget::SpacesTrigger, 100 + 3 * HOVER);
        assert!(fx.is_empty(), "once");
        // onto a row and released there: the tab moves to its Space, the popup closes
        let (d, fx) = mv(d, 63.0, DropTarget::Space(S), 100 + 3 * HOVER);
        assert!(fx.is_empty());
        assert_eq!(
            release(d, DropTarget::Space(S)),
            vec![
                Effect::CancelRows,
                Effect::MoveToSpace { tab: T, space: S },
                Effect::CloseSpaces
            ]
        );
    }

    /// Opened, then the pointer goes to the popup's New Space row, or its current Space's row.
    fn opened() -> Drag {
        let d = dragging(DropTarget::SpacesTrigger);
        let (d, fx) = tick(d, HOVER);
        assert_eq!(fx, vec![Effect::OpenSpaces]);
        d
    }

    #[test]
    fn the_new_space_row_takes_the_tab_and_the_current_row_cancels() {
        let (d, _) = mv(opened(), 70.0, DropTarget::NewSpace, HOVER + 10);
        assert_eq!(
            release(d, DropTarget::NewSpace),
            vec![
                Effect::CancelRows,
                Effect::MoveToNewSpace { tab: T },
                Effect::CloseSpaces
            ]
        );
        let (d, _) = mv(opened(), 70.0, DropTarget::SpacesMenu, HOVER + 10);
        assert_eq!(
            release(d, DropTarget::SpacesMenu),
            vec![Effect::CancelRows, Effect::CloseSpaces]
        );
        // released on the switcher before the popup opened: nothing
        assert_eq!(
            release(
                dragging(DropTarget::SpacesTrigger),
                DropTarget::SpacesTrigger
            ),
            vec![Effect::CancelRows]
        );
    }

    #[test]
    fn off_the_popup_for_the_delay_closes_it_and_coming_back_keeps_it() {
        let (d, fx) = mv(opened(), 300.0, DropTarget::None, 1000);
        assert!(fx.is_empty());
        assert!(d.awaits_dwell(), "the leave delay runs");
        // back before the delay: it stays, and the delay starts again on the next leave
        let (d, _) = mv(d, 60.0, DropTarget::SpacesMenu, 1000 + HOVER - 1);
        assert!(!d.awaits_dwell());
        let (d, _) = mv(d, 300.0, DropTarget::List, 2000);
        let (d, fx) = tick(d, 2000 + HOVER - 1);
        assert!(fx.is_empty());
        let (d, fx) = tick(d, 2000 + HOVER);
        assert_eq!(fx, vec![Effect::CloseSpaces]);
        assert!(!d.popup_open() && !d.awaits_dwell());
        // closed: the list takes the drop as before
        assert_eq!(release(d, DropTarget::List), vec![]);
        // the drag cancelled while it is open closes it
        assert_eq!(
            transition(opened(), Input::Cancel).1,
            vec![Effect::CancelRows, Effect::CloseSpaces]
        );
        // released in the list while open: the rows reorder, the popup closes
        assert_eq!(
            release(opened(), DropTarget::List),
            vec![Effect::CloseSpaces]
        );
    }

    #[test]
    fn leaving_the_switcher_restarts_the_delay() {
        let d = dragging(DropTarget::SpacesTrigger);
        let (d, _) = mv(d, 60.0, DropTarget::None, 400);
        let (d, fx) = mv(d, 61.0, DropTarget::SpacesTrigger, 450);
        assert!(fx.is_empty());
        let (_, fx) = tick(d, 450 + HOVER - 1);
        assert!(fx.is_empty(), "450 + the delay not reached");
    }

    #[test]
    fn drops() {
        let drop = |hit| release(dragging(hit), hit);
        assert_eq!(
            drop(DropTarget::Space(S)),
            vec![Effect::CancelRows, Effect::MoveToSpace { tab: T, space: S }]
        );
        assert_eq!(
            drop(DropTarget::SplitEdge(Side::Right))[1],
            Effect::SplitInto {
                tab: T,
                side: Side::Right
            }
        );
        assert_eq!(
            drop(DropTarget::Outside)[1],
            Effect::MoveToWindow {
                tab: T,
                to: WindowTarget::New
            }
        );
        assert_eq!(
            drop(DropTarget::Window(WindowId(3)))[1],
            Effect::MoveToWindow {
                tab: T,
                to: WindowTarget::Existing(WindowId(3))
            }
        );
        assert_eq!(drop(DropTarget::None), vec![Effect::CancelRows]);
    }

    #[test]
    fn cancel_drops_nothing() {
        let (d, fx) = transition(dragging(DropTarget::Outside), Input::Cancel);
        assert_eq!((d, fx), (Drag::Idle, vec![Effect::CancelRows]));
        let (d, _) = transition(
            Drag::Idle,
            Input::Press {
                tab: T,
                at: at(0.0, 0.0),
            },
        );
        assert_eq!(transition(d, Input::Cancel), (Drag::Idle, vec![]));
        assert_eq!(transition(Drag::Idle, Input::Cancel), (Drag::Idle, vec![]));
    }

    #[test]
    fn hit_rects() {
        let r = Rects {
            window: Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 1000.0,
                h: 600.0,
            }),
            list: Some(Rect {
                x: 0.0,
                y: 50.0,
                w: 200.0,
                h: 560.0,
            }),
            trigger: Some(Rect {
                x: 10.0,
                y: 560.0,
                w: 74.0,
                h: 36.0,
            }),
            // the popup over the list, above the switcher: a Space row, the New Space row
            menu: Some(Rect {
                x: 10.0,
                y: 400.0,
                w: 236.0,
                h: 150.0,
            }),
            spaces: vec![(
                S,
                Rect {
                    x: 16.0,
                    y: 406.0,
                    w: 224.0,
                    h: 36.0,
                },
            )],
            new_space: Some(Rect {
                x: 16.0,
                y: 508.0,
                w: 224.0,
                h: 36.0,
            }),
            content: Some(Rect {
                x: 200.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            }),
            windows: vec![(
                WindowId(1),
                Rect {
                    x: 1100.0,
                    y: 0.0,
                    w: 500.0,
                    h: 500.0,
                },
            )],
        };
        assert_eq!(hit(at(30.0, 420.0), &r), DropTarget::Space(S));
        assert_eq!(hit(at(30.0, 520.0), &r), DropTarget::NewSpace);
        assert_eq!(hit(at(30.0, 470.0), &r), DropTarget::SpacesMenu);
        assert_eq!(hit(at(30.0, 570.0), &r), DropTarget::SpacesTrigger);
        assert_eq!(hit(at(30.0, 100.0), &r), DropTarget::List);
        assert_eq!(hit(at(250.0, 100.0), &r), DropTarget::SplitEdge(Side::Left));
        assert_eq!(
            hit(at(950.0, 100.0), &r),
            DropTarget::SplitEdge(Side::Right)
        );
        assert_eq!(hit(at(600.0, 100.0), &r), DropTarget::None);
        assert_eq!(hit(at(1200.0, 100.0), &r), DropTarget::Window(WindowId(1)));
        assert_eq!(hit(at(-5.0, 100.0), &r), DropTarget::Outside);
        assert_eq!(hit(at(30.0, 100.0), &Rects::default()), DropTarget::Outside);
    }
}
