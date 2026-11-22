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

impl WindowState {
    /// The stored name (`windows.state`).
    pub fn id(self) -> &'static str {
        match self {
            WindowState::Normal => "normal",
            WindowState::Maximized => "maximized",
            WindowState::Fullscreen => "fullscreen",
            WindowState::Minimized => "minimized",
        }
    }

    pub fn from_id(id: &str) -> Option<WindowState> {
        [
            WindowState::Normal,
            WindowState::Maximized,
            WindowState::Fullscreen,
            WindowState::Minimized,
        ]
        .into_iter()
        .find(|s| s.id() == id)
    }

    /// The state a window comes back in after a restart: as it was, except that the focused
    /// window never comes back minimized, so the restored session has a window to look at
    /// (Chrome's session restore shows the last active browser normally).
    pub fn restored(self, focused: bool) -> WindowState {
        match self {
            WindowState::Minimized if focused => WindowState::Normal,
            s => s,
        }
    }

    /// Whether the size and position the platform reports in this state are not the
    /// window's own (they are the screen's, or none).
    fn hides_bounds(self) -> bool {
        self != WindowState::Normal
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
    /// before (`prev`): the restored bounds stay `prev`'s while the window is not normal.
    pub fn observed(prev: Option<&Geometry>, now: Geometry) -> Geometry {
        match prev {
            Some(p) if now.state.hides_bounds() => Geometry {
                width: p.width,
                height: p.height,
                pos: p.pos,
                ..now
            },
            // never seen normal: a minimized window's reported position is off screen
            // (Windows moves it to -32000), not one to come back at
            None if now.state == WindowState::Minimized => Geometry { pos: None, ..now },
            _ => now,
        }
    }

    /// The geometry a window comes back with after a restart (`WindowState::restored`).
    pub fn restoring(&self, focused: bool) -> Geometry {
        Geometry {
            state: self.state.restored(focused),
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
    let most = displays
        .iter()
        .map(|d| (d.area.overlap(bounds), d))
        .filter(|(a, _)| *a > 0)
        .max_by_key(|(a, _)| *a)
        .map(|(_, d)| d);
    let named = || {
        monitor.and_then(|m| displays.iter().find(|d| d.name.as_deref() == Some(m)))
    };
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
/// never above the area. `fit` (macOS, where Chrome is aggressive) moves and shrinks the
/// window to fit wholly inside the area.
// Ported from Chromium chrome/browser/ui/window_sizer/window_sizer.cc
// `WindowSizer::AdjustBoundsToBeVisibleOnDisplay` (BSD-3-Clause). The saved work area is not
// kept (winit tells no work area), so its "resize to the new work area" step is left out.
pub fn visible_on(bounds: Rect, area: &Rect, fit: bool) -> Rect {
    let mut b = Rect {
        width: bounds.width.max(MIN_VISIBLE),
        height: bounds.height.max(MIN_VISIBLE),
        ..bounds
    };
    if b.y < area.y {
        b.y = area.y;
    }
    if fit {
        b.height = b.height.min(area.height);
        return b.adjust_to_fit(area);
    }
    let min_y = area.y + MIN_VISIBLE - b.height;
    let min_x = area.x + MIN_VISIBLE - b.width;
    let max_y = area.bottom() - MIN_VISIBLE;
    let max_x = area.right() - MIN_VISIBLE;
    // `max` then `min`, not `clamp`: a window taller than its area has min > max
    b.y = b.y.max(min_y).min(max_y);
    b.x = b.x.max(min_x).min(max_x);
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
