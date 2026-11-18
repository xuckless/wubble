//! Where a dragged tab goes: one state enum and one pure transition.
//!
//! Inside the tab list the row physics (`tabs::TabRows`, the design's reorder) keeps owning
//! the drag; this machine only decides what a drag that leaves the list means: onto another
//! Space's icon (the Space opens after a hover of [`SPACE_HOVER`], so the tab can be dropped
//! into its list; released on the icon it moves there), onto the content area's left or right
//! edge (a split), onto another wubble window, or outside every window (a new window).
//!
//! Written for wubble: the design repo has reorder and click-to-switch only (no drop onto a
//! Space, no hover dwell, no drop-to-split, no tear-out). The dwell is the Finder/Firefox
//! spring-loading idea; its length is the owner's to tune. Time is passed in (`now`), never
//! read here.
//!
//! On Wayland a client cannot learn global pointer or window positions, so the host can never
//! name another window under the pointer there: a release outside the window is always
//! `Outside` (a new window). Other platforms may report `Window`.

use std::time::{Duration, Instant};

use crate::space::SpaceId;
use crate::tab::TabId;
use crate::tabs::TABS;
use crate::window::{WindowId, WindowTarget};

/// How long a dragged tab hovers over another Space's icon before that Space opens.
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
    Space(SpaceId),
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
    pub spaces: Vec<(SpaceId, Rect)>,
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
    if inside(&r.list) {
        return DropTarget::List;
    }
    match r.content.filter(|c| c.contains(p)) {
        Some(c) if p.x < c.x + c.w * EDGE_SHARE => DropTarget::SplitEdge(Side::Left),
        Some(c) if p.x >= c.x + c.w * (1.0 - EDGE_SHARE) => DropTarget::SplitEdge(Side::Right),
        _ => DropTarget::None,
    }
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
        /// The Space the hover opened, once it has.
        opened: Option<SpaceId>,
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
    /// A frame or timer while the pointer rests (the hover delay runs on).
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
    /// The drag began: the row lifts, a ghost follows the pointer outside the list.
    Lift(TabId),
    /// The hovered Space opens (the tab is still held).
    SwitchSpace(SpaceId),
    MoveToSpace {
        tab: TabId,
        space: SpaceId,
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
    /// The ghost goes; the drag is over.
    Land,
}

impl Drag {
    /// The tab the ghost shows, while the pointer is out of the list.
    pub fn ghost(&self) -> Option<TabId> {
        match *self {
            Drag::Dragging { tab, over, .. } if over != DropTarget::List => Some(tab),
            _ => None,
        }
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
                opened: None,
            };
            (next, vec![Effect::Lift(tab)])
        }
        (
            Drag::Dragging {
                tab,
                over,
                since,
                opened,
            },
            Input::Move { hit, now, .. },
        ) => {
            let since = if hit == over { since } else { now };
            dwell(
                Drag::Dragging {
                    tab,
                    over: hit,
                    since,
                    opened,
                },
                now,
            )
        }
        (Drag::Dragging { .. }, Input::Tick { now }) => dwell(d, now),
        (Drag::Dragging { tab, opened, .. }, Input::Release { hit }) => {
            let drop = match (hit, opened) {
                // into the list of the Space the hover opened
                (DropTarget::List, Some(space)) => Some(Effect::MoveToSpace { tab, space }),
                // the row physics reorders
                (DropTarget::List, None) => return (Drag::Idle, vec![Effect::Land]),
                (DropTarget::Space(space), _) => Some(Effect::MoveToSpace { tab, space }),
                (DropTarget::SplitEdge(side), _) => Some(Effect::SplitInto { tab, side }),
                (DropTarget::Window(w), _) => Some(Effect::MoveToWindow {
                    tab,
                    to: WindowTarget::Existing(w),
                }),
                (DropTarget::Outside, _) => Some(Effect::MoveToWindow {
                    tab,
                    to: WindowTarget::New,
                }),
                (DropTarget::None, _) => None,
            };
            let fx = std::iter::once(Effect::CancelRows)
                .chain(drop)
                .chain(std::iter::once(Effect::Land))
                .collect();
            (Drag::Idle, fx)
        }
        (Drag::Dragging { .. }, Input::Cancel) => {
            (Drag::Idle, vec![Effect::CancelRows, Effect::Land])
        }
        (Drag::Pressed { .. }, Input::Release { .. } | Input::Cancel) => (Drag::Idle, vec![]),
        (d, _) => (d, vec![]),
    }
}

/// Opens the hovered Space once the pointer has rested on its icon for [`SPACE_HOVER`].
fn dwell(d: Drag, now: Instant) -> (Drag, Vec<Effect>) {
    match d {
        Drag::Dragging {
            tab,
            over: DropTarget::Space(space),
            since,
            opened,
        } if opened != Some(space) && now.saturating_duration_since(since) >= SPACE_HOVER => (
            Drag::Dragging {
                tab,
                over: DropTarget::Space(space),
                since,
                opened: Some(space),
            },
            vec![Effect::SwitchSpace(space)],
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
        assert_eq!(fx, vec![Effect::Lift(T)]);
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
        assert_eq!(release(d, DropTarget::List), vec![Effect::Land]);
    }

    #[test]
    fn hovering_a_space_opens_it_once_after_the_delay() {
        let d = dragging(DropTarget::List);
        let (d, fx) = mv(d, 60.0, DropTarget::Space(S), 100);
        assert!(fx.is_empty());
        assert_eq!(d.ghost(), Some(T));
        let early = 100 + SPACE_HOVER.as_millis() as u64 - 1;
        let (d, fx) = transition(
            d,
            Input::Tick {
                now: t0() + Duration::from_millis(early),
            },
        );
        assert!(fx.is_empty(), "not before the delay");
        let (d, fx) = mv(d, 61.0, DropTarget::Space(S), early + 1);
        assert_eq!(fx, vec![Effect::SwitchSpace(S)]);
        let (d, fx) = mv(d, 62.0, DropTarget::Space(S), early + 900);
        assert!(fx.is_empty(), "once");
        // dropped into the opened Space's list
        assert_eq!(
            release(d, DropTarget::List),
            vec![
                Effect::CancelRows,
                Effect::MoveToSpace { tab: T, space: S },
                Effect::Land
            ]
        );
    }

    #[test]
    fn leaving_the_icon_restarts_the_delay() {
        let d = dragging(DropTarget::Space(S));
        let (d, _) = mv(d, 60.0, DropTarget::None, 400);
        let (d, fx) = mv(d, 61.0, DropTarget::Space(S), 450);
        assert!(fx.is_empty());
        let (_, fx) = transition(
            d,
            Input::Tick {
                now: t0() + Duration::from_millis(900),
            },
        );
        assert!(fx.is_empty(), "450 + 500 not reached");
    }

    #[test]
    fn drops() {
        let drop = |hit| release(dragging(hit), hit);
        assert_eq!(
            drop(DropTarget::Space(S)),
            vec![
                Effect::CancelRows,
                Effect::MoveToSpace { tab: T, space: S },
                Effect::Land
            ]
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
        assert_eq!(
            drop(DropTarget::None),
            vec![Effect::CancelRows, Effect::Land]
        );
    }

    #[test]
    fn cancel_drops_nothing() {
        let (d, fx) = transition(dragging(DropTarget::Outside), Input::Cancel);
        assert_eq!(
            (d, fx),
            (Drag::Idle, vec![Effect::CancelRows, Effect::Land])
        );
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
                h: 450.0,
            }),
            spaces: vec![(
                S,
                Rect {
                    x: 20.0,
                    y: 520.0,
                    w: 30.0,
                    h: 30.0,
                },
            )],
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
        assert_eq!(hit(at(30.0, 530.0), &r), DropTarget::Space(S));
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
