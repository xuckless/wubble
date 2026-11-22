//! Chrome's keyboard shortcuts that need more than a model command (`perform::Plan`): Open File
//! (Ctrl+O), focus search (Ctrl+E) and caret browsing (F7). Chromium's behaviour for each
//! (`chrome/browser/ui/browser_commands.cc`, BSD-3-Clause; behaviour only).

use std::path::{Path, PathBuf};

use slint::ComponentHandle;
use wubble_core::caret::{self, Caret, Effect, Event};
use wubble_core::tab::TabId;
use wubble_engine::page::FileChooserMode;

use super::{Host, new_tab};

impl Host {
    /// Ctrl+O: the OS dialog for local files; each chosen file opens in a new tab, as Chrome's
    /// `OpenFile` (several may be chosen). Cancel opens nothing.
    pub(super) fn open_file(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let spec = crate::chooser::dialog_spec(
            FileChooserMode::OpenMultiple,
            "Open File",
            Path::new(""),
            &[],
        );
        let dialog = crate::chooser::dialog(&spec, Some(window.window()));
        let task = slint::spawn_local(async move {
            let picked = crate::chooser::pick(spec, dialog).await;
            crate::with_host(|host| host.files_chosen(picked.unwrap_or_default()));
        });
        if let Err(e) = task {
            eprintln!("wubble: cannot show the open dialog: {e}");
        }
    }

    /// The files Open File chose, each as a `file://` tab.
    fn files_chosen(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            match wubble_core::url::file_url(&path) {
                Some(url) => self.dispatch(new_tab(url, None)),
                None => eprintln!("wubble: open file: no URL for {}", path.display()),
            }
        }
    }

    /// Ctrl+E: the bar opens on the visible tab as Ctrl+L does, then empties for a search. Later,
    /// outside the host, as `Plan::FocusAddress`: opening the bar calls back into it.
    pub(super) fn focus_search(&mut self) {
        let win = self.w.id;
        let _ = self.w.window.upgrade_in_event_loop(move |window| {
            window.invoke_request_address_focus();
            crate::with_host_in(win, |host| host.bar_for_search(&window));
        });
    }

    /// F7, or a row of its question: one step of `wubble_core::caret` on the preferences of the
    /// visible tab's profile. Without them (a Chromium that registers none) nothing happens.
    pub(super) fn caret_event(&mut self, event: Event) {
        let Some(tab) = self.shown() else {
            return;
        };
        let Some((enabled, ask)) = self.tab_view(tab).and_then(|v| v.caret_browsing()) else {
            eprintln!("wubble: caret browsing: no such Chromium preference here");
            return;
        };
        match caret::transition(Caret { enabled, ask }, event) {
            Some(Effect::Set { enabled, ask }) => self.set_caret_browsing(enabled, ask),
            Some(Effect::Ask) => self.caret_menu(tab),
            None => {}
        }
    }

    /// Writes the preferences through every live view: each container is a Chromium profile of
    /// its own, and Chrome's caret browsing is all of a profile's pages at once.
    fn set_caret_browsing(&mut self, enabled: bool, ask: bool) {
        let tabs: Vec<TabId> = self.views.keys().copied().collect();
        let mut failed = None;
        for tab in tabs {
            if let Some(view) = self.tab_view(tab)
                && let Err(e) = view.set_caret_browsing(enabled, ask)
            {
                failed = Some(e);
            }
        }
        if let Some(e) = failed {
            eprintln!("wubble: caret browsing: {e}");
        }
    }

    /// The question before caret browsing turns on, as the clear-data menu is shown.
    fn caret_menu(&mut self, tab: TabId) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let size = window.window().size();
        let at = ((size.width / 2) as i32, (size.height / 5) as i32);
        self.open_shell_menu(tab, at, crate::menu::caret_entries(), false);
    }
}
