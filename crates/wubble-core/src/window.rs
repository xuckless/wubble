//! Browser windows as model state: what each window shows. Tabs and Spaces stay global
//! (one owner, the `Model`); a window only names the Space or the tab it shows.
//!
//! Selection stays owned by the Space (`Space::active`, `Space::split`), so two Main windows
//! show two different Spaces and a tab is on screen in at most one window by construction
//! (`Model::check` asserts it). Little and Popup windows show one tab of their own; the tab
//! stays placed in a Space (its row is in that Space's list) but is never that Space's
//! active tab or a split pane. A Pip window shows its owner tab's frames (picture-in-picture,
//! `crate::pip`): the tab stays its Space's active tab or pane, but its Main window leaves it out
//! of what it frames, so it is still on screen in exactly one window.
//!
//! The window manager's shape (windows keyed by id, quit once no main window is left) follows
//! cefclient `tests/cefclient/browser/root_window_manager.cc` (BSD-3-Clause), pattern only.
//! What a window keeps for the session (its restored bounds, show state and monitor), where a
//! new window opens and how a window off every monitor is brought back follow Chrome's
//! `chrome/browser/ui/window_sizer/window_sizer.cc` (BSD-3-Clause).

use crate::space::SpaceId;
use crate::tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    /// The full browser: sidebar, Spaces, the address pill.
    Main,
    /// A link opened from another app: one page with "Open in <Space>" and "Open as tab".
    Little,
    /// A page's `window.open` with a size: minimal chrome, the address read-only.
    Popup,
    /// Picture-in-picture: a small always-on-top window with one tab's video, no chrome.
    Pip,
}

impl WindowKind {
    /// The id the shell's `window-kind` property takes.
    pub fn index(self) -> i32 {
        match self {
            WindowKind::Main => 0,
            WindowKind::Little => 1,
            WindowKind::Popup => 2,
            WindowKind::Pip => 3,
        }
    }
}

/// What a window shows, and so what kind of window it is. One enum, so a window shows a Space
/// or one tab, never both, and its kind cannot disagree with what it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// A Main window: the Space's active tab or its shown split.
    Space(SpaceId),
    /// A Little window: this one tab.
    Little(TabId),
    /// A Popup window: this one tab.
    Popup(TabId),
    /// A picture-in-picture window: this tab's frames, the tab still placed and shown as its
    /// Space's (at most one, `Model::check`).
    Pip(TabId),
}

/// A window's show state, as Chrome keeps it (`ui::mojom::WindowShowState`: normal, maximized,
/// minimized, fullscreen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WindowState {
    #[default]
    Normal,
    Maximized,
    Fullscreen,
    Minimized,
}

/// Each show state and its stored name (`windows.state`).
pub const STATES: [(WindowState, &str); 4] = [
    (WindowState::Normal, "normal"),
    (WindowState::Maximized, "maximized"),
    (WindowState::Fullscreen, "fullscreen"),
    (WindowState::Minimized, "minimized"),
];

impl WindowState {
    /// The stored name ([`STATES`]).
    pub fn id(self) -> &'static str {
        STATES
            .iter()
            .find(|(s, _)| *s == self)
            .map_or("normal", |(_, id)| id)
    }

    pub fn from_id(id: &str) -> Option<WindowState> {
        STATES.iter().find(|(_, i)| *i == id).map(|(s, _)| *s)
    }
}

/// A window's inner size in logical pixels and, where the platform tells (not Wayland), its
/// outer position in physical pixels; its show state and the monitor it is on, by the name the
/// platform gives (winit `MonitorHandle::name`), where it gives one.
///
/// Size and position are the window's restored bounds, as Chrome saves them
/// (`BrowserWindow::GetRestoredBounds` with `GetRestoredState`): while it is maximized,
/// fullscreen or minimized they stay what they were when it was last normal, so unmaximizing a
/// restored window puts it back where it was.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub pos: Option<(i32, i32)>,
    pub state: WindowState,
    pub monitor: Option<String>,
}

/// How far a new window sits from the window it was opened from, down and right, in px
/// (Chrome's `WindowSizer::kWindowTilePixels`).
pub const CASCADE_PX: i32 = 10;

impl Geometry {
    /// What the platform reported (`now`) kept as the window's geometry, given what was kept
    /// before (`prev`). While the window is not normal the reported bounds are the screen's (or
    /// off screen: Windows moves a minimized window to -32000), not the window's own: its
    /// restored bounds stay `prev`'s; never seen normal, it keeps the reported size and no
    /// position, so it comes back where the platform puts a new window.
    pub fn observed(prev: Option<&Geometry>, now: Geometry) -> Geometry {
        match prev {
            _ if now.state == WindowState::Normal => now,
            Some(p) => Geometry {
                width: p.width,
                height: p.height,
                pos: p.pos,
                ..now
            },
            None => Geometry { pos: None, ..now },
        }
    }

    /// The geometry a window comes back with after a restart: as it was, except that the
    /// focused window never comes back minimized, so the restored session has a window to look
    /// at (Chrome's session restore shows the last active browser normally).
    pub fn restoring(&self, focused: bool) -> Geometry {
        let state = match self.state {
            WindowState::Minimized if focused => WindowState::Normal,
            s => s,
        };
        Geometry {
            state,
            ..self.clone()
        }
    }

    /// A new window opened from this one (Chrome's `WindowSizer::GetLastActiveWindowBounds`):
    /// the same size, [`CASCADE_PX`] down and right where the position is known, on the same
    /// monitor; maximized if this one is (Chrome's `GetWindowDefaultShowState`), else normal.
    pub fn cascaded(&self) -> Geometry {
        Geometry {
            width: self.width,
            height: self.height,
            pos: self.pos.map(|(x, y)| (x + CASCADE_PX, y + CASCADE_PX)),
            state: match self.state {
                WindowState::Maximized => WindowState::Maximized,
                _ => WindowState::Normal,
            },
            monitor: self.monitor.clone(),
        }
    }
}

/// The least side of a window a page opens, in CSS px (Blink's `create_window.cc` raises a
/// smaller `window.open` width or height to 100).
pub const MIN_POPUP_SIDE: u32 = 100;

/// The most a window side gets, in logical px, where no display bounds it: wgpu's default
/// `max_texture_dimension_2d` (8192, past which the surface fails to configure) at a 2x scale.
pub const MAX_SIDE: u32 = 4096;

/// `(w, h)` no larger than `display` (logical px, where it is known) nor [`MAX_SIDE`], and at
/// least 1: a stored or page-chosen size the window can have.
pub fn bounded((w, h): (u32, u32), display: Option<(u32, u32)>) -> (u32, u32) {
    let (dw, dh) = display.unwrap_or((MAX_SIDE, MAX_SIDE));
    let side = |v: u32, d: u32| v.min(d).min(MAX_SIDE).max(1);
    (side(w, dw), side(h, dh))
}

/// The window for a page `window.open` asked to be `page` big (its `width` and `height`:
/// Chromium's content size, CSS px): the page at least [`MIN_POPUP_SIDE`] each way, in the
/// shell's `Chrome.inset` on every side, [`bounded`] by `display`.
pub fn popup_window((w, h): (u32, u32), display: Option<(u32, u32)>) -> (u32, u32) {
    let frame = 2 * crate::kit_gen::tokens::chrome::inset as u32;
    let side = |v: u32| v.max(MIN_POPUP_SIDE).saturating_add(frame);
    bounded((side(w), side(h)), display)
}

/// A rectangle in physical pixels of the virtual screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    /// The area both cover (0 if they do not meet).
    fn overlap(&self, o: &Rect) -> i64 {
        let w = i64::from(self.right().min(o.right())) - i64::from(self.x.max(o.x));
        let h = i64::from(self.bottom().min(o.bottom())) - i64::from(self.y.max(o.y));
        w.max(0) * h.max(0)
    }

    /// The squared distance from this rectangle to the point (0 inside).
    fn distance2(&self, (px, py): (i32, i32)) -> i64 {
        let along = |p: i32, lo: i32, hi: i32| {
            let (p, lo, hi) = (i64::from(p), i64::from(lo), i64::from(hi));
            if p < lo {
                lo - p
            } else if p > hi {
                p - hi
            } else {
                0
            }
        };
        let dx = along(px, self.x, self.right());
        let dy = along(py, self.y, self.bottom());
        dx * dx + dy * dy
    }

    fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }

    /// Moved, and shrunk where it is larger, to fit inside `area` (Chromium `gfx::Rect::AdjustToFit`).
    fn adjust_to_fit(self, area: &Rect) -> Rect {
        // ported from Chromium ui/gfx/geometry/rect.cc `AdjustAlongAxis` (BSD-3-Clause)
        let along = |dst: i32, dst_size: i32, origin: i32, size: i32| {
            let size = size.min(dst_size);
            let origin = if origin < dst {
                dst
            } else {
                (dst + dst_size).min(origin + size) - size
            };
            (origin, size)
        };
        let (x, width) = along(area.x, area.width, self.x, self.width);
        let (y, height) = along(area.y, area.height, self.y, self.height);
        Rect {
            x,
            y,
            width,
            height,
        }
    }
}

/// A monitor as the platform lists it: its name, if it has one, and its area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub name: Option<String>,
    pub area: Rect,
}

/// The display a window with these bounds goes on (Chromium `display::FindDisplayWithBiggestIntersection`,
/// then `display::FindDisplayNearestPoint`): the one it covers most; covering none, the monitor it
/// was on (`monitor`, by name) if that is still there, else the one nearest its centre. `None`
/// only without displays.
pub fn display_for<'a>(
    bounds: &Rect,
    monitor: Option<&str>,
    displays: &'a [Display],
) -> Option<&'a Display> {
    // reversed: of equal overlaps the first listed wins, as in Chromium
    let most = displays
        .iter()
        .rev()
        .map(|d| (d.area.overlap(bounds), d))
        .filter(|(a, _)| *a > 0)
        .max_by_key(|(a, _)| *a)
        .map(|(_, d)| d);
    let named = || monitor.and_then(|m| displays.iter().find(|d| d.name.as_deref() == Some(m)));
    let nearest = || {
        displays
            .iter()
            .min_by_key(|d| d.area.distance2(bounds.center()))
    };
    most.or_else(named).or_else(nearest)
}

/// The least of a window that stays on its display, in px (Chrome's `kMinVisibleWidth` and
/// `kMinVisibleHeight`).
const MIN_VISIBLE: i32 = 30;

/// `bounds` moved so enough of the window is on `area` to grab and move it back (Chrome's
/// `WindowSizer::AdjustBoundsToBeVisibleOnDisplay`): at least 30 px each way, the title bar
/// never above the area; moved and shrunk to fit inside an area too small for that. `mac` is
/// Chrome's macOS rule, more aggressive: no taller than the area, and a window partly off it
/// flush with its left edge, or its top.
// Ported from Chromium chrome/browser/ui/window_sizer/window_sizer.cc
// `WindowSizer::AdjustBoundsToBeVisibleOnDisplay` (BSD-3-Clause). The saved work area is not
// kept (winit tells no work area), so its "fit to a changed work area" step is left out.
pub fn visible_on(bounds: Rect, area: &Rect, mac: bool) -> Rect {
    let mut b = Rect {
        width: bounds.width.max(MIN_VISIBLE),
        height: bounds.height.max(MIN_VISIBLE),
        ..bounds
    };
    b.y = b.y.max(area.y);
    if mac {
        b.height = b.height.min(area.height);
        if b.x < area.x || b.right() > area.right() {
            b.x = area.x;
        }
        if b.y < area.y || b.bottom() > area.bottom() {
            b.y = area.y;
        }
        return b;
    }
    let min_y = area.y + MIN_VISIBLE - b.height;
    let min_x = area.x + MIN_VISIBLE - b.width;
    let max_y = area.bottom() - MIN_VISIBLE;
    let max_x = area.right() - MIN_VISIBLE;
    if min_x >= max_x || min_y >= max_y {
        return b.adjust_to_fit(area);
    }
    b.y = b.y.clamp(min_y, max_y);
    b.x = b.x.clamp(min_x, max_x);
    b
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub shown: Shown,
    /// `None` until the window reports it (a new window takes the platform's default).
    pub geometry: Option<Geometry>,
}

impl Window {
    /// A Main window on `space`, with no known geometry.
    pub fn main(id: WindowId, space: SpaceId) -> Window {
        Window {
            id,
            shown: Shown::Space(space),
            geometry: None,
        }
    }

    pub fn kind(&self) -> WindowKind {
        match self.shown {
            Shown::Space(_) => WindowKind::Main,
            Shown::Little(_) => WindowKind::Little,
            Shown::Popup(_) => WindowKind::Popup,
            Shown::Pip(_) => WindowKind::Pip,
        }
    }

    /// The Space a Main window shows.
    pub fn space(&self) -> Option<SpaceId> {
        match self.shown {
            Shown::Space(s) => Some(s),
            Shown::Little(_) | Shown::Popup(_) | Shown::Pip(_) => None,
        }
    }

    /// The tab a Little or Popup window shows (its own tab, out of its Space's selection).
    pub fn tab(&self) -> Option<TabId> {
        match self.shown {
            Shown::Little(t) | Shown::Popup(t) => Some(t),
            Shown::Space(_) | Shown::Pip(_) => None,
        }
    }

    /// The tab the window belongs to: a Little or Popup window's tab, or the tab a Pip window
    /// shows. The window closes with it.
    pub fn owner(&self) -> Option<TabId> {
        match self.shown {
            Shown::Little(t) | Shown::Popup(t) | Shown::Pip(t) => Some(t),
            Shown::Space(_) => None,
        }
    }
}

/// Where `MoveTabToWindow` puts a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowTarget {
    /// A new Main window (tear-out).
    New,
    Existing(WindowId),
}

/// What the user is closing, for the question a private window's running downloads ask first
/// ([`crate::model::Model::private_tabs_ended`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    Window(WindowId),
    /// Quitting.
    App,
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn r(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    const FHD: Rect = r(0, 0, 1920, 1080);

    #[test]
    fn visible_on_follows_chromes_window_sizer() {
        // (case, bounds, area, mac, expected)
        let cases = [
            (
                "inside stays",
                r(100, 100, 800, 600),
                FHD,
                false,
                r(100, 100, 800, 600),
            ),
            (
                "off the right",
                r(3000, 100, 800, 600),
                FHD,
                false,
                r(1890, 100, 800, 600),
            ),
            (
                "off the left",
                r(-2000, 100, 800, 600),
                FHD,
                false,
                r(-770, 100, 800, 600),
            ),
            (
                "above",
                r(100, -50, 800, 600),
                FHD,
                false,
                r(100, 0, 800, 600),
            ),
            (
                "below",
                r(100, 2000, 800, 600),
                FHD,
                false,
                r(100, 1050, 800, 600),
            ),
            (
                "tiny grows",
                r(100, 100, 10, 10),
                FHD,
                false,
                r(100, 100, 30, 30),
            ),
            (
                "a second display",
                r(5000, 100, 800, 600),
                r(1920, 0, 1920, 1080),
                false,
                r(3810, 100, 800, 600),
            ),
            (
                "area too small: fit",
                r(100, 100, 10, 10),
                r(0, 0, 20, 20),
                false,
                r(0, 0, 20, 20),
            ),
            (
                "mac: inside stays",
                r(100, 100, 800, 600),
                FHD,
                true,
                r(100, 100, 800, 600),
            ),
            (
                "mac: partly right",
                r(1500, 100, 800, 600),
                FHD,
                true,
                r(0, 100, 800, 600),
            ),
            (
                "mac: partly below",
                r(100, 700, 800, 600),
                FHD,
                true,
                r(100, 0, 800, 600),
            ),
            (
                "mac: taller",
                r(100, 0, 800, 2000),
                FHD,
                true,
                r(100, 0, 800, 1080),
            ),
            (
                "mac: above",
                r(100, -50, 800, 600),
                FHD,
                true,
                r(100, 0, 800, 600),
            ),
        ];
        for (case, bounds, area, mac, expected) in cases {
            assert_eq!(visible_on(bounds, &area, mac), expected, "{case}");
        }
    }

    #[test]
    fn display_for_takes_the_most_covered_then_the_named_then_the_nearest() {
        let displays = [
            Display {
                name: Some("A".into()),
                area: FHD,
            },
            Display {
                name: Some("B".into()),
                area: r(1920, 0, 2560, 1440),
            },
        ];
        // (case, bounds, monitor, expected display)
        let cases = [
            ("inside A", r(100, 100, 800, 600), Some("B"), Some("A")),
            ("mostly on B", r(1800, 100, 800, 600), None, Some("B")),
            (
                "equal on both: the first",
                r(1820, 0, 200, 100),
                None,
                Some("A"),
            ),
            (
                "off both: its monitor",
                r(-3000, 100, 800, 600),
                Some("B"),
                Some("B"),
            ),
            (
                "off both, monitor gone: nearest",
                r(-3000, 100, 800, 600),
                Some("C"),
                Some("A"),
            ),
            (
                "off both, unnamed: nearest",
                r(10_000, 100, 800, 600),
                None,
                Some("B"),
            ),
        ];
        for (case, bounds, monitor, expected) in cases {
            let d = display_for(&bounds, monitor, &displays);
            assert_eq!(d.and_then(|d| d.name.as_deref()), expected, "{case}");
        }
        assert_eq!(display_for(&FHD, None, &[]), None, "no displays");
    }

    #[test]
    fn geometry_keeps_restored_bounds_and_comes_back_in_its_state() {
        use WindowState::*;
        let g =
            |w: u32, h: u32, pos: Option<(i32, i32)>, state: WindowState, monitor: Option<&str>| {
                Geometry {
                    width: w,
                    height: h,
                    pos,
                    state,
                    monitor: monitor.map(String::from),
                }
            };
        let normal = g(800, 600, Some((10, 20)), Normal, Some("A"));
        let screen = |state| g(1920, 1080, Some((0, 0)), state, Some("B"));
        // (case, prev, reported, kept)
        let observed = [
            ("first report", None, normal.clone(), normal.clone()),
            (
                "maximized keeps the normal bounds",
                Some(&normal),
                screen(Maximized),
                g(800, 600, Some((10, 20)), Maximized, Some("B")),
            ),
            (
                "fullscreen keeps the normal bounds",
                Some(&normal),
                screen(Fullscreen),
                g(800, 600, Some((10, 20)), Fullscreen, Some("B")),
            ),
            (
                "first seen maximized: the size, no position",
                None,
                screen(Maximized),
                g(1920, 1080, None, Maximized, Some("B")),
            ),
            (
                "first seen fullscreen: the size, no position",
                None,
                screen(Fullscreen),
                g(1920, 1080, None, Fullscreen, Some("B")),
            ),
            (
                "first seen minimized: no position",
                None,
                g(160, 28, Some((-32000, -32000)), Minimized, None),
                g(160, 28, None, Minimized, None),
            ),
            (
                "normal again takes the new bounds",
                Some(&g(800, 600, Some((10, 20)), Fullscreen, None)),
                g(900, 700, Some((5, 5)), Normal, None),
                g(900, 700, Some((5, 5)), Normal, None),
            ),
        ];
        for (case, prev, now, kept) in observed {
            assert_eq!(Geometry::observed(prev, now), kept, "{case}");
        }
        // (case, state, focused, comes back as)
        let restored = [
            (
                "the focused window is never minimized",
                Minimized,
                true,
                Normal,
            ),
            ("another stays minimized", Minimized, false, Minimized),
            ("maximized stays", Maximized, true, Maximized),
            ("fullscreen stays", Fullscreen, false, Fullscreen),
        ];
        for (case, state, focused, back) in restored {
            let was = Geometry {
                state,
                ..normal.clone()
            };
            assert_eq!(
                was.restoring(focused),
                Geometry {
                    state: back,
                    ..normal.clone()
                },
                "{case}"
            );
        }
        // (case, from, cascaded)
        let cascades = [
            (
                "down and right",
                normal.clone(),
                g(800, 600, Some((20, 30)), Normal, Some("A")),
            ),
            (
                "maximized stays, no position",
                g(800, 600, None, Maximized, None),
                g(800, 600, None, Maximized, None),
            ),
            (
                "fullscreen opens normal",
                g(800, 600, Some((0, 0)), Fullscreen, None),
                g(800, 600, Some((10, 10)), Normal, None),
            ),
        ];
        for (case, from, to) in cascades {
            assert_eq!(from.cascaded(), to, "{case}");
        }
        for (s, id) in STATES {
            assert_eq!((s.id(), WindowState::from_id(id)), (id, Some(s)));
        }
        assert_eq!(WindowState::from_id("hidden"), None);
    }

    #[test]
    fn a_popup_window_fits_its_page_and_its_display() {
        let frame = 2 * crate::kit_gen::tokens::chrome::inset as u32;
        // (case, page asked for, display, window)
        let cases = [
            (
                "the page in the inset",
                (500, 600),
                Some((1920, 1080)),
                (500 + frame, 600 + frame),
            ),
            (
                "too small: 100",
                (10, 0),
                Some((1920, 1080)),
                (100 + frame, 100 + frame),
            ),
            (
                "too big: the display",
                (100_000, 2000),
                Some((1920, 1080)),
                (1920, 1080),
            ),
            (
                "no display: the most a surface takes",
                (100_000, u32::MAX),
                None,
                (MAX_SIDE, MAX_SIDE),
            ),
        ];
        for (case, page, display, window) in cases {
            assert_eq!(popup_window(page, display), window, "{case}");
        }
        // (case, stored size, bounded)
        let stored = [
            ("kept", (800, 600), (800, 600)),
            ("a corrupt row", (u32::MAX, 0), (MAX_SIDE, 1)),
        ];
        for (case, size, kept) in stored {
            assert_eq!(bounded(size, None), kept, "{case}");
        }
    }
}
