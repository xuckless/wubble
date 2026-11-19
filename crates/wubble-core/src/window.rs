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

/// A window's inner size in logical pixels and, where the platform tells (not Wayland), its
/// outer position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub pos: Option<(i32, i32)>,
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
