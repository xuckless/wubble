//! The extension tab APIs on the host's model (a CEF with wubble's patches, `Engine::ext_apis`;
//! inert otherwise): after every change the model's projection
//! (`wubble_core::extensions::project`, live tabs being those with a view) goes to the engine,
//! and what extensions ask (open, close, activate a tab) becomes the model's own commands. The
//! model decides where an opened tab goes; the action's changes are only logged until the
//! action buttons exist (TODO.md).
//!
//! The code runs on [`ExtHost`]: [`Host`] is one, and `--smoke extensions` drives the same
//! functions through a windowless stand-in (`extensions_smoke.rs`), so the smoke covers them.

use std::collections::BTreeMap;

use wubble_core::extensions::{opened_url, project};
use wubble_core::model::{Command, Model};
use wubble_core::space::{ContainerId, Side};
use wubble_core::tab::TabId;
use wubble_core::window::WindowId;
use wubble_engine::app::{Engine, ViewId};
use wubble_engine::ext_api::{ExtRequest, ExtViewTab, TabOpened};

use super::Host;

/// What the extension tab APIs need of a host.
pub(crate) trait ExtHost {
    fn model(&self) -> &Model;
    /// The tabs' views.
    fn views(&self) -> &BTreeMap<TabId, ViewId>;
    fn tab_of(&self, view: ViewId) -> Option<TabId>;
    fn engine(&mut self) -> &mut Engine;
    /// Applies `command` as the host does (`Host::dispatch`), the projection pushed after.
    fn run(&mut self, command: Command);
    /// An extension's action changed (badge, title, icon, popup): the shell's buttons redraw.
    fn action_changed(&mut self) {}
}

impl ExtHost for Host {
    fn model(&self) -> &Model {
        &self.model
    }

    fn views(&self) -> &BTreeMap<TabId, ViewId> {
        &self.views
    }

    fn tab_of(&self, view: ViewId) -> Option<TabId> {
        Host::tab_of(self, view)
    }

    fn engine(&mut self) -> &mut Engine {
        &mut self.engine
    }

    fn run(&mut self, command: Command) {
        self.dispatch(command);
    }

    fn action_changed(&mut self) {
        self.ext_action_changed();
    }
}

/// The command that opens `url` for an extension in `container`: in the Space of the Main
/// window `window` names, else in the active Space (a private window is none of an
/// extension's: its tab opens in the last normal window, `Model::normal_space`); shown when
/// `active`.
fn open_command(
    model: &Model,
    url: &str,
    container: ContainerId,
    window: Option<WindowId>,
    active: bool,
) -> Command {
    let space = window
        .and_then(|w| model.windows.iter().find(|x| x.id == w))
        .and_then(|w| w.space())
        .filter(|&s| model.side_of_space(s) != Side::Private);
    Command::ExtensionTab {
        url: opened_url(url).to_owned(),
        container,
        space,
        show: active,
    }
}

/// Sends the model's projection to the engine (which skips an unchanged one).
pub(crate) fn push(host: &mut impl ExtHost) {
    if !host.engine().ext_apis() {
        return;
    }
    let views = host.views();
    let p = project(host.model(), |tab| views.contains_key(&tab));
    let tabs = p
        .tabs
        .iter()
        .filter_map(|t| {
            Some(ExtViewTab {
                view: *views.get(&t.tab)?,
                window: t.window,
                index: t.index,
                active: t.active,
                pinned: t.pinned,
            })
        })
        .collect();
    host.engine().set_ext_projection(p.windows, tabs);
}

/// Applies what extensions asked since the last pump.
pub(crate) fn serve(host: &mut impl ExtHost) {
    for request in host.engine().take_ext_requests() {
        let command = match request {
            ExtRequest::OpenTab {
                request,
                container,
                url,
                window,
                active,
            } => {
                let known: Vec<TabId> = host.model().tabs.keys().copied().collect();
                host.run(open_command(host.model(), &url, container, window, active));
                let answer = answer(host, &known);
                host.engine().answer_open_tab(request, answer);
                continue;
            }
            // a private window's tab is none of an extension's: the request does not say which
            // extension asks, so "Allow in Private Windows" cannot let one through here (an
            // allowed extension's reach is the engine's: docs/research/extensions.md)
            ExtRequest::CloseTab(view) => host
                .tab_of(view)
                .filter(|&t| host.model().side_of_tab(t) != Side::Private)
                .map(Command::CloseTab),
            ExtRequest::ActivateTab(view) => host
                .tab_of(view)
                .filter(|&t| host.model().side_of_tab(t) != Side::Private)
                .map(Command::ActivateTab),
            ExtRequest::ActionChanged { extension } => {
                let _ = extension;
                host.action_changed();
                None
            }
        };
        if let Some(command) = command {
            host.run(command);
        }
    }
}

/// The tab an open request added (the one not in `known`): its view, or none for a tab the
/// shell draws (`wubble://`).
fn answer(host: &impl ExtHost, known: &[TabId]) -> TabOpened {
    let added = host.model().tabs.keys().find(|t| !known.contains(t));
    match added {
        Some(tab) => host
            .views()
            .get(tab)
            .map_or(TabOpened::Page, |v| TabOpened::View(*v)),
        None => TabOpened::Refused,
    }
}

impl Host {
    pub(super) fn ext_push(&mut self) {
        push(self);
    }

    pub(super) fn ext_requests(&mut self) {
        serve(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wubble_core::window::Window;

    fn open(model: &Model, url: &str, window: Option<WindowId>, active: bool) -> Command {
        open_command(model, url, ContainerId(2), window, active)
    }

    #[test]
    fn an_open_is_an_extension_tab_in_the_asking_container() {
        let m = Model::new(Instant::now());
        assert_eq!(
            open(&m, "https://example.com/", None, false),
            Command::ExtensionTab {
                url: "https://example.com/".into(),
                container: ContainerId(2),
                space: None,
                show: false,
            }
        );
    }

    #[test]
    fn an_open_in_a_main_window_goes_to_its_space_and_a_popup_window_to_the_active_one() {
        let mut m = Model::new(Instant::now());
        // a Space the model has: an unknown one is taken as private (`Model::side_of_space`)
        let s = m.active_space();
        m.windows.push(Window::main(WindowId(7), s));
        m.windows.push(Window {
            id: WindowId(8),
            shown: wubble_core::window::Shown::Popup(TabId(1)),
            geometry: None,
        });
        let space = |w| match open(&m, "https://a.test/", Some(WindowId(w)), true) {
            Command::ExtensionTab { space, .. } => space,
            c => panic!("{c:?}"),
        };
        assert_eq!(space(7), Some(s));
        assert_eq!(space(8), None);
        assert_eq!(space(99), None);
    }

    #[test]
    fn an_open_of_chromes_new_tab_page_opens_the_start_page() {
        let m = Model::new(Instant::now());
        assert!(matches!(
            open(&m, "chrome://newtab/", None, true),
            Command::ExtensionTab { url, .. } if url == "wubble://start"
        ));
    }
}
