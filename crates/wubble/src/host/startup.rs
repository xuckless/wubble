//! The host's half of the startup settings (`wubble_core::startup`): the session as the
//! startup mode opens it ([`open_session`], before the host exists), the "Restore pages?" notice
//! and the command bar's "Open pages from last session" ([`Host::startup_event`]), Home
//! ([`Host::go_home`]) and "Warn before quitting" ([`Host::quit_keys`]).

use std::time::Instant;

use wubble_core::action::Action;
use wubble_core::model::{Command, Model};
use wubble_core::pages::InternalPage;
use wubble_core::quit::{self, QuitEffect};
use wubble_core::settings as defs;
use wubble_core::space::ContainerId;
use wubble_core::startup::{self, LastSession, Offer, OfferEffect, OfferEvent, StartupMode};
use wubble_core::closed::{ClosedTab, Entry};

use super::{Host, now_ms};
use crate::persist::{Loaded, StoreClient};

/// The loaded session as the stored startup mode opens it, where the model is handed to the
/// host. The loose tabs it leaves out go onto the recently-closed list (Ctrl+Shift+T brings
/// them back one by one) and into the offer (all at once). `opening`: a page from the command
/// line takes the focused window.
pub(crate) fn open_session(loaded: &mut Loaded, store: &StoreClient, opening: bool) {
    let stored = &loaded.settings.stored;
    let mode = StartupMode::from_stored(stored);
    let pages = startup::startup_pages(stored);
    let model = std::mem::replace(&mut loaded.model, Model::new(Instant::now()));
    let opened = startup::open(model, mode, &pages, opening);
    loaded.model = opened.model;
    let at = now_ms();
    for d in &opened.last.tabs {
        store.push_closed(Entry::Tab(ClosedTab {
            url: d.url.clone(),
            title: d.title.clone(),
            container: d.container,
            space: Some(d.space),
            closed_at_ms: at,
        }));
    }
    loaded.startup = Offer::at_start(opened.last, loaded.clean_exit);
}

impl Host {
    /// The windows are up: the notice asks when the last run did not quit cleanly.
    pub(crate) fn startup_opened(&mut self) {
        if self.startup.asking() {
            self.toast_restore_pages(startup::RESTORE_NOTICE.0, startup::RESTORE_NOTICE.1);
        }
    }

    /// Steps the offer and runs its effects.
    pub(crate) fn startup_event(&mut self, event: OfferEvent) {
        let (offer, effects) = startup::transition(std::mem::take(&mut self.startup), event);
        self.startup = offer;
        for effect in effects {
            match effect {
                OfferEffect::HideNotice => self.toast_hide(),
                OfferEffect::Reopen(last) => self.reopen_last_session(last),
                OfferEffect::NothingToRestore => {
                    if let Some(window) = self.w.window.upgrade() {
                        self.toast(&window, startup::NOTHING_TO_RESTORE);
                    }
                }
            }
        }
    }

    /// The left-out pages back in their Spaces and containers (the active Space and the Default
    /// container when theirs are gone), each Space's shown tab shown again where it is the
    /// focused window's; then the start pages that took their place close, if still on the
    /// start page.
    fn reopen_last_session(&mut self, last: LastSession) {
        let LastSession { tabs, start_tabs } = last;
        let mut show = None;
        for d in tabs {
            let space = Some(d.space).filter(|s| self.model.spaces.iter().any(|k| k.id == *s));
            let container = Some(d.container)
                .filter(|c| self.model.containers.iter().any(|k| k.id == *c))
                .unwrap_or(ContainerId::DEFAULT);
            let newest = self.model.tabs.keys().next_back().copied();
            self.dispatch(Command::ExtensionTab {
                url: d.url,
                container,
                space,
                show: false,
            });
            let added = self
                .model
                .tabs
                .keys()
                .next_back()
                .copied()
                .filter(|t| Some(*t) != newest);
            if d.active && space == Some(self.model.active_space()) {
                show = added.or(show);
            }
        }
        if let Some(tab) = show {
            self.dispatch(Command::ActivateTab(tab));
        }
        let start = InternalPage::Start.url();
        let stale: Vec<_> = start_tabs
            .into_iter()
            .filter(|t| self.model.tabs.get(t).is_some_and(|t| t.url == start))
            .collect();
        if !stale.is_empty() {
            self.dispatch(Command::CloseTabs(stale));
        }
    }

    /// Home: the home page in the visible tab, else in a new tab.
    pub(super) fn go_home(&mut self) {
        let url = startup::home_url(&self.settings.stored);
        match self.shown() {
            Some(tab) => self.dispatch(Command::Navigate { tab, url }),
            None => self.dispatch(Command::NewTab {
                url,
                container: None,
                opener: None,
            }),
        }
    }

    /// Quits, unless quitting would cancel private downloads: then it asks first
    /// (`close_asks`). The menu bar's and the Dock's Quit on macOS, and the quit keys once
    /// [`Self::quit_keys`] lets them.
    pub(crate) fn quit_now(&mut self) {
        if !self.close_asks(wubble_core::window::Closing::App) {
            crate::quit();
        }
    }

    /// The quit keys (or the command bar's Quit): quits, or first warns while "Warn before
    /// quitting" is on.
    pub(crate) fn quit_keys(&mut self) {
        let warn = wubble_core::apply::value_of(&defs::QUIT_WARN, &self.settings.stored) == "1";
        let (guard, effect) = quit::transition(self.quit_guard, Instant::now(), warn);
        self.quit_guard = guard;
        match effect {
            QuitEffect::Quit => self.quit_now(),
            QuitEffect::Warn => {
                let chord = self
                    .keymap
                    .shortcut_for(Action::Quit)
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "the quit keys".into());
                if let Some(window) = self.w.window.upgrade() {
                    self.toast(&window, &quit::warning(&chord));
                }
            }
            QuitEffect::Nothing => {}
        }
    }
}
