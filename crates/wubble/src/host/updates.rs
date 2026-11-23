//! The host's half of the updater (`crate::updater`): the check when wubble opens, Settings'
//! Updates group (its switch, "Check now", "Restart to update"), the ready toast, and the
//! healthy mark that lets the launch record keep this version.

use wubble_core::apply;
use wubble_core::settings as defs;
use wubble_core::update::{self, Event};
use wubble_ui::{AppWindow, UpdatesView};

use super::Host;
use crate::updater::HostEffect;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Host {
    /// The browser opened: a check when the setting is on and the last one is old enough.
    pub(crate) fn updates_opened(&mut self) {
        let stored = &self.settings.stored;
        let enabled = apply::value_of(&defs::UPDATE_CHECK_ON_OPEN, stored) == "1";
        let last = apply::value_of(&defs::UPDATE_LAST_CHECK, stored)
            .parse()
            .unwrap_or(0);
        self.update_event(Event::Opened {
            enabled,
            last,
            now: now_secs(),
        });
    }

    /// An event of the updater's (its threads post them here).
    pub(crate) fn update_event(&mut self, event: Event) {
        for effect in self.updater.event(event) {
            match effect {
                HostEffect::RecordCheck => {
                    // a Count setting holds u32 seconds: good until 2106
                    let now = u32::try_from(now_secs()).unwrap_or(u32::MAX);
                    self.write_setting(defs::UPDATE_LAST_CHECK.key, &now.to_string());
                }
                HostEffect::Notify(v) => self.toast_restart(&update::ready_text(&v)),
            }
        }
        self.render_settings();
    }

    pub(crate) fn updates_toggled(&mut self, on: bool) {
        self.write_setting(defs::UPDATE_CHECK_ON_OPEN.key, if on { "1" } else { "0" });
    }

    pub(crate) fn updates_check(&mut self) {
        self.update_event(Event::CheckNow);
    }

    /// "Restart to update": quit as usual (asking first when that cancels private downloads:
    /// `close_asks`), then start the install's program once this process is gone (it runs the
    /// new version). A question dismissed restarts nothing, then or at a later quit.
    pub(crate) fn updates_restart(&mut self) {
        if self.updater.relaunch_exe().is_some()
            && !self.close_asks(wubble_core::window::Closing::Restart)
        {
            self.restart_now();
        }
    }

    /// Quits, the install's program to start once this process is gone; nothing without an
    /// update to run.
    pub(super) fn restart_now(&mut self) {
        if let Some(exe) = self.updater.relaunch_exe() {
            crate::updater::request_relaunch(exe);
            crate::quit();
        }
    }

    pub(crate) fn updates_healthy(&mut self) {
        self.updater.healthy();
    }

    /// The Updates group's view.
    pub(super) fn render_updates(&self, window: &AppWindow) {
        let view = self.updater.view();
        let on = apply::value_of(&defs::UPDATE_CHECK_ON_OPEN, &self.settings.stored) == "1";
        window.set_settings_updates(UpdatesView {
            on,
            version: update::current_version().to_string().into(),
            status: view.status.into(),
            can_check: view.can_check,
            ready: view.ready,
            inert: view.inert,
        });
    }
}
