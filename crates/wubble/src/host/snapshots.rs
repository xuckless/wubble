//! Snapshots of hidden tabs: a small GPU copy of a tab's last frame, taken as it is hidden
//! (`Life::CaptureSnapshot`), shown in its pane while it comes back (a discarded tab restoring,
//! a background tab repainting) and dropped on its first real frame, on `Life::ReleaseSnapshot`
//! and under critical memory pressure. `wubble_core::snapshot::Book` keeps them to a byte budget.
//! No CPU readback: the copy is one render pass in wubble-gfx (`FramePresenter::snapshot`).

use std::collections::BTreeSet;

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
    /// The hidden tab's last frame, from its pane's presenter, into the book. Nothing when its
    /// pane shows no frame of it (never painted, or still showing its snapshot).
    pub(super) fn capture_snapshot(&mut self, tab: TabId) {
        if !self.frames.contains_key(&tab) || self.snapshots.standing_in(tab) {
            return;
        }
        let Some(slot) = self.slots.iter().position(|s| *s == Some(tab)) else {
            return;
        };
        match self.presenters[slot].snapshot(WIDTH) {
            Ok(Some((image, bytes))) => {
                self.snapshots.book.insert(tab, image, bytes);
            }
            Ok(None) => {}
            Err(e) => eprintln!("wubble: snapshot of tab {}: {e}", tab.0),
        }
    }

    /// Each tab on screen without a frame shows its snapshot, if it has one. Run after the
    /// frames of tabs no longer on screen are dropped.
    pub(super) fn stand_in_snapshots(&mut self, window: &AppWindow) {
        let frames = &self.frames;
        self.snapshots
            .standing_in
            .retain(|t| frames.contains_key(t));
        for tab in self.framed_tabs() {
            if self.frames.contains_key(&tab) {
                continue;
            }
            let Some(image) = self.snapshots.book.get(tab).cloned() else {
                continue;
            };
            let size = image.size();
            let wanted = self.pane_layout(window, tab).frame_size();
            self.frames.insert(
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
    }
}
