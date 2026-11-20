//! Snapshots of hidden tabs: a small GPU copy of a tab's last frame, taken as it is hidden
//! (`Life::CaptureSnapshot`), shown in its pane while it comes back (a discarded tab restoring,
//! a background tab repainting) and dropped on its first real frame, on `Life::ReleaseSnapshot`
//! and under critical memory pressure. `wubble_core::snapshot::Book` keeps them to a byte budget.
//! No CPU readback: the copy is one render pass in wubble-gfx (`FramePresenter::snapshot`).

use std::collections::BTreeSet;

use slint::ComponentHandle;

use wubble_core::snapshot::Book;
use wubble_core::stretch::{PaneSprings, PaneView};
use wubble_core::tab::TabId;
use wubble_gfx::FrameSize;
use wubble_ui::AppWindow;

use super::panes::px;
use super::{Host, PaneFrame};

/// A snapshot's width, physical pixels (about 0.4 MB at 16:10).
const WIDTH: u32 = 384;
/// The bytes all snapshots may take: about 90 tabs' worth.
const BUDGET: u64 = 32 << 20;

pub(super) struct Snapshots {
    book: Book<slint::Image>,
    /// Tabs whose pane shows their snapshot until their first frame.
    standing_in: BTreeSet<TabId>,
}

impl Default for Snapshots {
    fn default() -> Self {
        Self {
            book: Book::new(BUDGET),
            standing_in: BTreeSet::new(),
        }
    }
}

impl Snapshots {
    pub(super) fn release(&mut self, tab: TabId) {
        self.book.release(tab);
    }

    pub(super) fn release_except(&mut self, keep: &[TabId]) {
        self.book.release_except(keep);
    }

    /// Whether `tab`'s pane shows its snapshot, not a frame.
    pub(super) fn standing_in(&self, tab: TabId) -> bool {
        self.standing_in.contains(&tab)
    }
}

impl Host {
    /// The hidden tab's last frame, from its pane's presenter in whichever window showed it,
    /// into the book. Nothing when its pane shows no frame of it (never painted, or still
    /// showing its snapshot). The tab hover preview shows it too.
    pub(super) fn capture_snapshot(&mut self, tab: TabId) {
        if self.snapshots.standing_in(tab) {
            return;
        }
        let slot = |w: &super::WindowHost| {
            w.frames
                .contains_key(&tab)
                .then(|| w.slots.iter().position(|s| *s == Some(tab)))
                .flatten()
        };
        let shot = match slot(&self.w) {
            Some(i) => self.w.presenters[i].snapshot(WIDTH),
            None => match self.parked.values().find_map(|w| Some((w, slot(w)?))) {
                Some((w, i)) => w.presenters[i].snapshot(WIDTH),
                None => return,
            },
        };
        match shot {
            Ok(Some((image, bytes))) => {
                self.snapshots.book.insert(tab, image, bytes);
            }
            Ok(None) => {}
            Err(e) => eprintln!("wubble: snapshot of tab {}: {e}", tab.0),
        }
        self.capture_exit_page(tab);
    }

    /// The hidden tab's last frame at full size, for its exit under the incoming page (the
    /// driver plays it and drops it, `exit-page`). Only when it was this window's one pane and
    /// motion is not reduced; a split's panes keep their places.
    fn capture_exit_page(&mut self, tab: TabId) {
        let panes = super::window::page_panes(&self.w.slots);
        if wubble_core::motion::reduced() || panes != 1 || !self.w.frames.contains_key(&tab) {
            return;
        }
        let Some(i) = self.w.slots.iter().position(|s| *s == Some(tab)) else {
            return;
        };
        let presenter = &self.w.presenters[i];
        match presenter.snapshot(u32::MAX) {
            Ok(Some((image, _))) => {
                if let Some(window) = self.w.window.upgrade() {
                    window.set_exit_page(image);
                    // the driver plays it, or drops it at once when no switch hid the tab
                    window.window().request_redraw();
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("wubble: exit page of tab {}: {e}", tab.0),
        }
    }

    /// `tab`'s snapshot, if it has one (the hover preview's thumbnail).
    pub(super) fn snapshot_of(&self, tab: TabId) -> Option<&slint::Image> {
        self.snapshots.book.get(tab)
    }

    /// Each tab on screen without a frame shows its snapshot, if it has one. Run after the
    /// frames of tabs no longer on screen are dropped.
    pub(super) fn stand_in_snapshots(&mut self, window: &AppWindow) {
        let frames = &self.w.frames;
        self.snapshots
            .standing_in
            .retain(|t| frames.contains_key(t));
        for tab in self.framed_tabs() {
            if self.w.frames.contains_key(&tab) {
                continue;
            }
            let Some(image) = self.snapshots.book.get(tab).cloned() else {
                continue;
            };
            let size = image.size();
            let wanted = self.pane_layout(window, tab).frame_size();
            self.w.frames.insert(
                tab,
                PaneFrame {
                    image,
                    clip: FrameSize::new(size.width, size.height),
                    view: PaneView::Live { size: px(wanted) },
                    springs: PaneSprings::default(),
                    prev: None,
                    masked: None,
                },
            );
            self.snapshots.standing_in.insert(tab);
        }
    }

    /// `tab` presented a real frame: its snapshot is not needed any more.
    pub(super) fn frame_arrived(&mut self, tab: TabId) {
        if self.snapshots.standing_in.remove(&tab)
            && let Some(smoke) = self.checks.tabs_smoke.as_mut()
        {
            smoke.heard.replaced.insert(tab);
        }
        self.snapshots.book.release(tab);
        if self.checks.mcp_smoke.is_some() {
            self.mcp_smoke_frame(tab);
        }
    }
}
