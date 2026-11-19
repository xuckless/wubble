//! The link status bubble, host side: the page's status text (`PageEvent::LinkStatus`, the
//! hovered link's URL) feeds one `hovercard` machine with Chromium's status timing
//! (`STATUS_BUBBLE`), keyed by the tab whose pane shows it; its state and the text are what
//! `render_panes` draws in that pane (`StatusBubble`). Moving it away from the pointer is the
//! pane's own (`SplitView`).

use wubble_core::hovercard::{self, Event, STATUS_BUBBLE};
use wubble_core::tab::TabId;

use super::Host;
use super::hover::HoverClock;

pub(super) struct Status {
    clock: HoverClock<TabId>,
    /// The last tab that had a status, and its text: kept while the bubble fades out.
    text: Option<(TabId, String)>,
}

impl Status {
    pub fn new() -> Self {
        Self {
            clock: HoverClock::new(STATUS_BUBBLE),
            text: None,
        }
    }

    /// What `tab`'s pane draws: shown, and the URL's host and rest cut to `max_chars`.
    pub fn view(&self, tab: TabId, max_chars: usize) -> (bool, String, String) {
        match &self.text {
            Some((t, url)) if *t == tab => {
                let shown = hovercard::shown(self.clock.state()) == Some(tab);
                let (host, rest) = wubble_core::url::status_parts(url, max_chars);
                (shown, host, rest)
            }
            _ => (false, String::new(), String::new()),
        }
    }

    /// The URL the bubble shows now, if it shows.
    pub fn shown(&self) -> Option<(TabId, &str)> {
        let tab = hovercard::shown(self.clock.state())?;
        self.text
            .as_ref()
            .filter(|(t, _)| *t == tab)
            .map(|(t, url)| (*t, url.as_str()))
    }
}

/// Characters of URL that fit a third of a pane `width` logical px wide (Chromium's standard
/// bubble width), at the bubble's 12 px type.
pub(super) fn status_chars(width: f64) -> usize {
    ((width / 3.0) / 6.5).max(16.0) as usize
}

impl Host {
    /// The page's status text changed: a link is hovered (`text` is its URL), or none is.
    pub(super) fn link_status(&mut self, tab: TabId, text: String) {
        if text.is_empty() {
            if self.status.clock.leave_if(tab, Self::status_tick) {
                self.status_render();
            }
            return;
        }
        self.status.text = Some((tab, text));
        let now = self.status.clock.now();
        self.status
            .clock
            .step(Event::Enter { key: tab, now }, Self::status_tick);
        self.status_render();
    }

    fn status_tick(&mut self) {
        let now = self.status.clock.now();
        self.status
            .clock
            .step(Event::Tick { now }, Self::status_tick);
        self.status_render();
    }

    fn status_render(&mut self) {
        if let Some(window) = self.w.window.upgrade() {
            self.render_panes(&window);
        }
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
