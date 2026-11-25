//! The recently-closed list in the shell: the entries a whole Main window, folder or Space
//! makes when it closes (`wubble_core::closed::Entry`), and the list's rows on the history page
//! ("Recently closed", above the visits) and in the command bar. Each row reopens its entry
//! (`closed::Event::Take`) or deletes it (`closed::Event::Remove`); the store is the truth and
//! is listed again after every change.

use std::collections::HashMap;

use wubble_core::closed::{self, Entry, Listed};
use wubble_core::model::Command;
use wubble_store::Favicon;

use super::{Host, now_ms};
use crate::bind::favicon_image;
use crate::persist::ClosedPage;
use crate::with_host;

/// What the shell shows of the list.
#[derive(Debug, Default)]
pub(super) struct Recent {
    /// Newest first, as last listed.
    pub(super) entries: Vec<Listed>,
    /// The icons of the pages the rows show.
    pub(super) icons: HashMap<String, Favicon>,
    /// Bumped per listing; an older reply is dropped.
    generation: u64,
    /// Entries were pushed during this dispatch: list again once it is done.
    pub(super) stale: bool,
}

impl Host {
    /// The entry `command` (about to run) would put on the list as a whole: a Main window (its
    /// Space and geometry), a folder ("Close Folder") or a Space (deleted). None for any other
    /// command, and for an agent Space's or one with nothing the list may keep.
    pub(super) fn closed_group(&mut self, command: &Command) -> Option<Entry> {
        let at = now_ms();
        match command {
            Command::CloseWindow(id) => {
                let main = self
                    .model
                    .window(*id)
                    .is_some_and(|w| w.kind() == wubble_core::window::WindowKind::Main);
                if !main {
                    return None;
                }
                // the window's last size and place, not the last save's
                self.record_geometry();
                Entry::window_of(&self.model, *id, at, |t| self.keeps_closed(t))
            }
            Command::DeleteSpace(space) => {
                Entry::space_of(&self.model, *space, at, |t| self.keeps_closed(t))
            }
            Command::CloseFolder(folder) => {
                Entry::folder_of(&self.model, *folder, at, |t| self.keeps_closed(t))
            }
            _ => None,
        }
    }

    /// `command` ran: when it closed what `entry` stands for, the entry goes on the list and is
    /// this run's close for Mod+Z.
    pub(super) fn finish_closed_group(&mut self, command: &Command, entry: Entry) {
        let gone = match command {
            Command::CloseWindow(id) => self.model.window(*id).is_none(),
            Command::DeleteSpace(space) => !self.model.spaces.iter().any(|s| s.id == *space),
            Command::CloseFolder(folder) => !self.model.spaces.iter().any(|s| {
                s.items
                    .iter()
                    .any(|i| matches!(i, wubble_core::space::Item::Folder(f) if f.id == *folder))
            }),
            _ => false,
        };
        if !gone {
            return;
        }
        let undo = closed::Closed::entry(&entry);
        self.store.push_closed(entry);
        self.close_undo(closed::Event::Closed(undo));
        self.recent.stale = true;
    }

    /// Asks the store for the list again; the rows redraw when it answers.
    pub(super) fn recent_refresh(&mut self) {
        self.recent.generation += 1;
        let generation = self.recent.generation;
        self.store.recently_closed(move |page| {
            let _ = slint::invoke_from_event_loop(move || {
                with_host(|host| host.recent_arrived(generation, page));
            });
        });
    }

    fn recent_arrived(&mut self, generation: u64, page: ClosedPage) {
        if generation != self.recent.generation {
            return;
        }
        self.recent.entries = page.entries;
        self.recent.icons = page.icons;
        self.render_recent();
        if let Some(window) = self.w.window.upgrade()
            && window.get_command_bar_state() != wubble_ui::CommandBarState::Closed
        {
            self.render_bar(&window);
        }
    }

    /// The history page's "Recently closed" rows: the entries the page's search finds.
    pub(super) fn render_recent(&self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let query = self.history_search().trim().to_lowercase();
        let shown: Vec<&Listed> = self
            .recent
            .entries
            .iter()
            .filter(|l| l.entry.matches(&query))
            .collect();
        let items = wubble_ui::pages::closed_items(&shown, |url| {
            self.recent
                .icons
                .get(url)
                .map(|f| favicon_image(f.width, f.height, &f.rgba))
                .unwrap_or_default()
        });
        crate::bind::sync(window.get_closed_lines(), items, |m| {
            window.set_closed_lines(m)
        });
    }

    /// A "Recently closed" row clicked: its entry reopens and leaves the list.
    pub(crate) fn closed_reopened(&mut self, id: i32) {
        self.close_undo(closed::Event::Take(i64::from(id)));
    }

    /// A "Recently closed" row's delete (the history page's, Shift+Delete in the command bar):
    /// the entry leaves the list.
    pub(crate) fn closed_deleted(&mut self, id: i64) {
        let Some(listed) = self.recent.entries.iter().find(|l| l.id == id) else {
            return;
        };
        let event = closed::Event::Remove {
            id,
            key: listed.entry.key(),
            closed_at_ms: listed.entry.closed_at_ms(),
        };
        self.close_undo(event);
    }
}
