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
use wubble_engine::view::Desktop;
use wubble_gfx::FramePresenter;
use wubble_ui::AppWindow;
use wubble_ui::glass::GlassRegistry;

use super::{Pace, PaneFrame, surfaces};
use crate::backdrop::Glass;
use crate::panes::Panes;

/// The presenter slots: the pages', then the DevTools panel's, then the peek's.
pub(super) const SLOTS: usize = MAX_PANES + 2;
pub(super) const PANEL_SLOT: usize = MAX_PANES;
pub(super) const PEEK_SLOT: usize = MAX_PANES + 1;

/// The slot that presents `tab`: `own` for a view with a slot of its own (the panel, the
/// peek), else the page slot it holds, else a page slot no framed tab holds.
pub(super) fn pick_slot(
    slots: &mut [Option<TabId>; SLOTS],
    tab: TabId,
    own: Option<usize>,
    framed: &[TabId],
) -> usize {
    let pages = &slots[..MAX_PANES];
    let i = own
        .or_else(|| pages.iter().position(|s| *s == Some(tab)))
        .or_else(|| {
            pages
                .iter()
                .position(|s| s.is_none_or(|t| !framed.contains(&t)))
        })
        .unwrap_or(MAX_PANES - 1);
    slots[i] = Some(tab);
    i
}

/// Frees the slots of tabs no longer framed, so the page slots held count the pages on screen.
pub(super) fn free_slots(slots: &mut [Option<TabId>; SLOTS], framed: &[TabId]) {
    for s in slots.iter_mut() {
        if s.is_some_and(|t| !framed.contains(&t)) {
            *s = None;
        }
    }
}

/// How many pages the slots present (not the panel, not the peek).
pub(super) fn page_panes(slots: &[Option<TabId>; SLOTS]) -> usize {
    slots[..MAX_PANES].iter().filter(|s| s.is_some()).count()
}

/// One window's share of the host.
pub(crate) struct WindowHost {
    /// The platform's extras (`crate::wl`) know this window: told with its first event.
    pub(super) aux_added: bool,
    /// The pages hidden from CEF while this window is (occluded, minimised: `Host::occlusion`),
    /// shown again when it is.
    pub(super) occluded: Vec<TabId>,
    /// The window is hidden (`Host::occlusion`): its pages are not asked about their video.
    pub(super) hidden: bool,
    pub(super) id: WindowId,
    pub(super) kind: WindowKind,
    /// The strong handle: the window lives as long as this.
    pub(super) handle: AppWindow,
    pub(super) window: slint::Weak<AppWindow>,
    /// One ring of textures per pane on screen (each pane has its own frame size), and the tab
    /// each slot presents; a tab keeps its slot while it stays on screen.
    /// After the pages' come the DevTools panel's ([`PANEL_SLOT`]) and the peek's
    /// ([`PEEK_SLOT`]).
    pub(super) presenters: [FramePresenter; SLOTS],
    pub(super) slots: [Option<TabId>; SLOTS],
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
    /// The window's monitor and place on the desktop, read by `Host::follow_desktop`: told to
    /// every page as its screen.
    pub(super) desktop: Option<Desktop>,
    /// The previous pane frame's time while the strings or a pane's stretch move.
    pub(super) panes_last: Option<Instant>,
    /// What the strings settled, waiting to be dispatched outside rendering.
    pub(super) split_commands: Vec<wubble_core::model::Command>,
    /// What of the surfaces the shell last drew.
    pub(super) drawn: surfaces::Drawn,
    /// The begin frames of the pages this window shows (`Pacing::Shell`), one per render.
    pub(super) pace: Pace,
    /// The page's pointer shape on the window.
    pub(super) cursor: super::cursor::PageCursor,
    /// The touchscreen's fingers down on this window's pages (`host/touch.rs`).
    pub(super) fingers: wubble_core::touch::Fingers,
    /// When this window's toast goes (`Host::show_toast`): its one clock.
    pub(super) toast: slint::Timer,
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
        presenters: [FramePresenter; SLOTS + 1],
        glass: Rc<RefCell<Glass>>,
        panels: Rc<RefCell<GlassRegistry>>,
    ) -> Self {
        let [a, b, c, popup_presenter, panel_presenter, peek_presenter] = presenters;
        Self {
            aux_added: false,
            occluded: Vec::new(),
            hidden: false,
            id,
            kind,
            window: handle.as_weak(),
            handle,
            presenters: [a, b, c, panel_presenter, peek_presenter],
            slots: [None; SLOTS],
            popup_presenter,
            glass,
            panels,
            frames: BTreeMap::new(),
            panes: Panes::default(),
            desktop: None,
            panes_last: None,
            split_commands: vec![],
            drawn: Default::default(),
            pace: Pace::default(),
            cursor: Default::default(),
            fingers: Default::default(),
            toast: slint::Timer::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: u64) -> TabId {
        TabId(n)
    }

    #[test]
    fn a_peek_over_a_full_split_has_its_own_slot() {
        let mut slots = [None; SLOTS];
        let framed = [t(1), t(2), t(3), t(9)];
        let panes: Vec<usize> = (1..=3)
            .map(|n| pick_slot(&mut slots, t(n), None, &framed))
            .collect();
        let peek = pick_slot(&mut slots, t(9), Some(PEEK_SLOT), &framed);
        assert_eq!(peek, PEEK_SLOT);
        assert!(!panes.contains(&peek));
        // the third pane keeps its slot after the peek presented
        assert_eq!(pick_slot(&mut slots, t(3), None, &framed), panes[2]);
    }

    #[test]
    fn a_single_page_counts_one_pane_after_a_split_or_a_peek() {
        let mut slots = [None; SLOTS];
        let framed = [t(1), t(2), t(9)];
        pick_slot(&mut slots, t(1), None, &framed);
        pick_slot(&mut slots, t(2), None, &framed);
        pick_slot(&mut slots, t(9), Some(PEEK_SLOT), &framed);
        assert_eq!(page_panes(&slots), 2);
        // unsplit, peek closed
        free_slots(&mut slots, &[t(1)]);
        assert_eq!(page_panes(&slots), 1);
        assert_eq!(slots[PEEK_SLOT], None);
    }
}
