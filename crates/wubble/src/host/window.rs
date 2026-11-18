//! What each window of the host owns: its Slint window, the frame rings of the panes it shows,
//! its glass, its split physics and what its shell last drew. Everything else (the engine, the
//! model, the views, the store) is the host's, shared by every window.
//!
//! The host works on one window at a time, `Host::w`; the others wait in `Host::parked`.
//! Every entry point (a window's callbacks, its rendering notifier, the pump's per-window
//! frame step) first enters its window (`Host::enter`), so each window's state has one owner
//! at any moment and the per-window code reads `self.w` as before.
//!
//! The window manager's shape (windows keyed by id, the app quits once no main window is
//! left) follows cefclient `tests/cefclient/browser/root_window_manager.cc` (BSD-3-Clause),
//! pattern only.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use slint::ComponentHandle;
use wubble_core::space::MAX_PANES;
use wubble_core::tab::TabId;
use wubble_core::window::{WindowId, WindowKind};
use wubble_gfx::FramePresenter;
use wubble_ui::AppWindow;
use wubble_ui::glass::GlassRegistry;

use super::{PaneFrame, surfaces};
use crate::backdrop::Glass;
use crate::panes::Panes;

/// One window's share of the host.
pub(crate) struct WindowHost {
    pub(super) id: WindowId,
    pub(super) kind: WindowKind,
    /// The strong handle: the window lives as long as this.
    pub(super) handle: AppWindow,
    pub(super) window: slint::Weak<AppWindow>,
    /// One ring of textures per pane on screen (each pane has its own frame size), and the tab
    /// each slot presents; a tab keeps its slot while it stays on screen.
    pub(super) presenters: [FramePresenter; MAX_PANES],
    pub(super) slots: [Option<TabId>; MAX_PANES],
    /// The popup widget's frames: a ring of their own, since a popup's size
    /// differs from the page's and changes with every popup.
    pub(super) popup_presenter: FramePresenter,
    /// The window's glass, shared with the shell's rendering notifier.
    pub(super) glass: Rc<RefCell<Glass>>,
    /// The glass the panels report (the list's one owner), for the pixel smoke.
    pub(super) panels: Rc<RefCell<GlassRegistry>>,
    /// The last frame of each tab on screen (the `panes` the shell shows).
    pub(super) frames: BTreeMap<TabId, PaneFrame>,
    /// Where the pages on screen sit: the split physics, stepped per rendered frame.
    pub(super) panes: Panes,
    /// The previous pane frame's time while the strings or a pane's stretch move.
    pub(super) panes_last: Option<Instant>,
    /// What the strings settled, waiting to be dispatched outside rendering.
    pub(super) split_commands: Vec<wubble_core::model::Command>,
    /// What of the surfaces the shell last drew.
    pub(super) drawn: surfaces::Drawn,
}

impl WindowHost {
    /// Another handle on the window.
    pub(crate) fn handle(&self) -> AppWindow {
        self.handle.clone_strong()
    }

    pub(crate) fn new(
        id: WindowId,
        kind: WindowKind,
        handle: AppWindow,
        presenters: [FramePresenter; MAX_PANES + 1],
        glass: Rc<RefCell<Glass>>,
        panels: Rc<RefCell<GlassRegistry>>,
    ) -> Self {
        let [a, b, c, popup_presenter] = presenters;
        Self {
            id,
            kind,
            window: handle.as_weak(),
            handle,
            presenters: [a, b, c],
            slots: [None; MAX_PANES],
            popup_presenter,
            glass,
            panels,
            frames: BTreeMap::new(),
            panes: Panes::default(),
            panes_last: None,
            split_commands: vec![],
            drawn: Default::default(),
        }
    }
}
