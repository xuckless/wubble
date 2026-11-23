//! The link status bubble, host side: the page's status text (`PageEvent::LinkStatus`, the
//! hovered link's URL) feeds the window's status slot of the store (`overlay::Kind::Status`,
//! Chromium's timing), keyed by the tab whose pane shows it; its state and the text are what
//! `render_panes` draws in that pane (`StatusBubble`). Moving it away from the pointer is the
//! pane's own (`SplitView`).

use wubble_core::overlay::{Cue, Key, Kind, Surface};
use wubble_core::tab::TabId;

use super::Host;

pub(super) struct Status {
    /// The last tab that had a status, and its text: kept while the bubble fades out.
    text: Option<(TabId, String)>,
}

impl Status {
    pub fn new() -> Self {
        Self { text: None }
    }
}

/// Characters of URL that fit a third of a pane `width` logical px wide (Chromium's standard
/// bubble width), at the bubble's 12 px type.
pub(super) fn status_chars(width: f64) -> usize {
    ((width / 3.0) / 6.5).max(16.0) as usize
}

impl Host {
    fn status_slot(&self) -> Surface {
        Surface {
            w: self.w.id,
            kind: Kind::Status,
        }
    }

    /// What `tab`'s pane draws: shown, and the URL's host and rest cut to `max_chars`.
    pub(super) fn status_view(&self, tab: TabId, max_chars: usize) -> (bool, String, String) {
        match &self.status.text {
            Some((t, url)) if *t == tab => {
                let shown = self.ui.shown(self.status_slot()) == Some(Key::Tab(tab));
                let (host, rest) = wubble_core::url::status_parts(url, max_chars);
                (shown, host, rest)
            }
            _ => (false, String::new(), String::new()),
        }
    }

    /// The URL the bubble shows now, if it shows.
    pub(super) fn status_shown(&self) -> Option<(TabId, &str)> {
        let Some(Key::Tab(tab)) = self.ui.shown(self.status_slot()) else {
            return None;
        };
        self.status
            .text
            .as_ref()
            .filter(|(t, _)| *t == tab)
            .map(|(t, url)| (*t, url.as_str()))
    }

    /// The page's status text changed: a link is hovered (`text` is its URL), or none is.
    pub(super) fn link_status(&mut self, tab: TabId, text: String) {
        if text.is_empty() {
            self.hover(Kind::Status, Key::Tab(tab), Cue::Leave);
            return;
        }
        let swapped = self.status.text.as_ref().is_none_or(|(_, t)| *t != text);
        self.status.text = Some((tab, text));
        self.hover(Kind::Status, Key::Tab(tab), Cue::Enter);
        // another link's words while it shows: the slot did not move, the text did
        if swapped && self.ui.shown(self.status_slot()) == Some(Key::Tab(tab)) {
            self.status_render();
        }
    }

    fn status_render(&mut self) {
        let window = self.w.handle();
        self.render_panes(&window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_third_of_the_pane_in_characters_never_below_sixteen() {
        assert_eq!(status_chars(1950.0), 100);
        assert_eq!(status_chars(100.0), 16);
    }
}
