//! The tab hover preview, host side: the pointer's enter and leave on tab rows feed the
//! `hovercard` machine; its effects show or hide the card (`PreviewCard`), which this is the one
//! writer of. A timer is armed only while the machine waits for a deadline. Each tab's last frame,
//! taken when the tab is hidden, stands for it on the card.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use wubble_core::hovercard::{self, Effect, Event, Hover};
use wubble_core::tab::TabId;

use super::Host;
use crate::with_host;

/// Widest thumbnail kept, physical px: the card shows it at 220 logical px.
const THUMB_WIDTH: u32 = 320;
/// Thumbnails kept at once (about 250 KB each); the oldest capture goes first.
const THUMBS_MAX: usize = 48;

/// Values by tab, at most `cap`, the oldest insertion evicted first.
#[derive(Debug)]
pub(super) struct Capped<T> {
    map: BTreeMap<TabId, T>,
    order: VecDeque<TabId>,
    cap: usize,
}

impl<T> Capped<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            map: BTreeMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    /// Stores `value` for `tab` as the newest.
    pub fn insert(&mut self, tab: TabId, value: T) {
        self.order.retain(|t| *t != tab);
        self.order.push_back(tab);
        self.map.insert(tab, value);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    pub fn get(&self, tab: TabId) -> Option<&T> {
        self.map.get(&tab)
    }

    /// Keeps only the tabs `keep` accepts.
    pub fn retain(&mut self, keep: impl Fn(&TabId) -> bool) {
        self.map.retain(|t, _| keep(t));
        self.order.retain(|t| keep(t));
    }
}

/// What the preview holds of its own.
pub(super) struct Preview {
    state: Hover,
    timer: slint::Timer,
    /// The monotonic zero of the machine's milliseconds.
    epoch: Instant,
    /// The row the pointer is on and its top, window px.
    over: Option<(TabId, f32)>,
    thumbs: Capped<slint::Image>,
}

impl Preview {
    pub fn new() -> Self {
        Self {
            state: Hover::Idle,
            timer: slint::Timer::default(),
            epoch: Instant::now(),
            over: None,
            thumbs: Capped::new(THUMBS_MAX),
        }
    }

    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// Drops what belongs to tabs that are gone.
    pub fn forget_except(&mut self, alive: impl Fn(&TabId) -> bool) {
        self.thumbs.retain(&alive);
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
        let now = self.preview.now();
        if on {
            self.preview.over = Some((tab, y));
            self.hover_event(Event::Enter { tab, now });
        } else if self.preview.over.is_some_and(|(t, _)| t == tab) {
            // (a leave of a row already replaced by the next one is stale)
            self.preview.over = None;
            self.hover_event(Event::Leave { now });
        }
    }

    /// A press, drag, menu, scroll or tab switch: no card.
    pub(super) fn hover_dismiss(&mut self) {
        if self.preview.state != Hover::Idle {
            self.hover_event(Event::Dismiss);
        }
    }

    fn hover_tick(&mut self) {
        let now = self.preview.now();
        self.hover_event(Event::Tick { now });
    }

    fn hover_event(&mut self, event: Event) {
        let (state, effect) = hovercard::transition(self.preview.state, event);
        self.preview.state = state;
        match hovercard::deadline(state) {
            Some(at) => {
                let wait = at.saturating_sub(self.preview.now());
                self.preview.timer.start(
                    slint::TimerMode::SingleShot,
                    Duration::from_millis(wait),
                    || with_host(|host| host.hover_tick()),
                );
            }
            None => self.preview.timer.stop(),
        }
        match effect {
            Some(Effect::Show(tab)) => self.render_preview(Some(tab)),
            Some(Effect::Hide) => self.render_preview(None),
            None => {}
        }
    }

    /// Draws the card for `tab`, or fades it out (it keeps its words while it does).
    fn render_preview(&mut self, tab: Option<TabId>) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let mut view = window.get_preview();
        match tab.and_then(|t| Some((t, self.model.tabs.get(&t)?))) {
            Some((id, t)) => {
                let (host, path) = wubble_core::url::address_parts(&t.url);
                view.title = if t.title.is_empty() { &t.url } else { &t.title }
                    .as_str()
                    .into();
                view.address = format!("{host}{path}").into();
                // the page on screen has no snapshot: the card says only what it is
                let on_screen = self.model.visible_tabs().contains(&id);
                view.thumb = self
                    .preview
                    .thumbs
                    .get(id)
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

    /// The tab was just hidden: keep a small copy of its last frame for its card.
    pub(super) fn capture_thumbnail(&mut self, tab: TabId) {
        let slot = |w: &super::WindowHost| w.slots.iter().position(|s| *s == Some(tab));
        let image = match slot(&self.w) {
            Some(i) => self.w.presenters[i].thumbnail(THUMB_WIDTH),
            None => match self.parked.values().find_map(|w| Some((w, slot(w)?))) {
                Some((w, i)) => w.presenters[i].thumbnail(THUMB_WIDTH),
                None => return,
            },
        };
        match image {
            Ok(Some(image)) => self.preview.thumbs.insert(tab, image),
            Ok(None) => {}
            Err(e) => eprintln!("wubble: no thumbnail for tab {}: {e}", tab.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_capture_goes_first_and_a_recapture_is_the_newest() {
        let mut c = Capped::new(2);
        c.insert(TabId(1), "a");
        c.insert(TabId(2), "b");
        c.insert(TabId(1), "a2");
        c.insert(TabId(3), "c");
        assert_eq!(c.get(TabId(2)), None);
        assert_eq!(
            (c.get(TabId(1)), c.get(TabId(3))),
            (Some(&"a2"), Some(&"c"))
        );
        c.retain(|t| *t != TabId(1));
        assert_eq!(c.get(TabId(1)), None);
        c.insert(TabId(4), "d");
        c.insert(TabId(5), "e");
        assert_eq!(c.get(TabId(3)), None, "cap holds after a retain");
    }
}
