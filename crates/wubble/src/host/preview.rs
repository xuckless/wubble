//! The tab hover preview, host side: the pointer's enter and leave on tab rows feed the
//! window's card slot of the store (`overlay::Kind::Preview`, Firefox's timing); its effects
//! show or hide the card (`PreviewCard`), which this is the one writer of. Each tab's snapshot,
//! taken when the tab is hidden (`host/snapshots.rs`), stands for it on the card.

use wubble_core::overlay::{Cue, Key, Kind};
use wubble_core::tab::TabId;

use super::Host;

/// What the preview holds of its own.
pub(super) struct Preview {
    /// The row the pointer is on and its top, window px.
    over: Option<(TabId, f32)>,
}

impl Preview {
    pub fn new() -> Self {
        Self { over: None }
    }

    /// Drops what belongs to tabs that are gone.
    pub fn forget_except(&mut self, alive: impl Fn(&TabId) -> bool) {
        if self.over.is_some_and(|(t, _)| !alive(&t)) {
            self.over = None;
        }
    }
}

impl Host {
    /// A tab row's pointer enter or leave; `y` is the row's top in window px.
    pub(crate) fn tab_hovered(&mut self, id: i32, on: bool, y: f32) {
        let Some(tab) = crate::shell::tab_id(id) else {
            return;
        };
        if on {
            self.preview.over = Some((tab, y));
            self.hover(Kind::Preview, Key::Tab(tab), Cue::Enter);
        } else if self.preview.over.is_some_and(|(t, _)| t == tab) {
            // (a leave of a row already replaced by the next one is stale)
            self.preview.over = None;
            self.hover(Kind::Preview, Key::Tab(tab), Cue::Leave);
        }
    }

    /// A press, drag, menu, scroll or tab switch: no card.
    pub(super) fn hover_dismiss(&mut self) {
        self.hover(Kind::Preview, Key::None, Cue::Dismiss);
    }

    /// Draws the card for `tab`, or fades it out (it keeps its words while it does).
    pub(super) fn render_preview(&mut self, tab: Option<TabId>) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let mut view = window.get_preview();
        match tab.and_then(|t| Some((t, self.model.tabs.get(&t)?))) {
            Some((id, t)) => {
                let (title, address) = crate::shell::preview_words(t);
                view.title = title.into();
                view.address = address.into();
                // the page on screen has no snapshot: the card says only what it is
                let on_screen = self.model.visible_tabs().contains(&id);
                view.thumb = self
                    .snapshot_of(id)
                    .filter(|_| !on_screen)
                    .cloned()
                    .unwrap_or_default();
                view.y = self
                    .preview
                    .over
                    .filter(|(o, _)| *o == id)
                    .map_or(view.y, |(_, y)| y);
                view.shown = true;
            }
            None => view.shown = false,
        }
        window.set_preview(view);
    }
}
