//! The extension tab APIs on the host's model (a CEF with wubble's patches, `Engine::ext_apis`;
//! inert otherwise): after every change the model's projection
//! (`wubble_core::extensions::project`, live tabs being those with a view) goes to the engine,
//! and what extensions ask (open, close, activate a tab) becomes the model's own commands. The
//! model decides where an opened tab goes; the action's changes are only logged until the
//! action buttons exist (TODO.md).

use wubble_core::extensions::project;
use wubble_core::model::{Command, Model};
use wubble_core::space::ContainerId;
use wubble_core::tab::TabId;
use wubble_engine::ext_api::{ExtRequest, ExtViewTab};

use super::Host;

/// An extension's request in the model's terms.
#[derive(Clone, Debug, PartialEq)]
enum Ask {
    Open {
        url: String,
        container: Option<ContainerId>,
        active: bool,
    },
    Close(TabId),
    Activate(TabId),
}

/// The command that does `ask`. An inactive tab opens in the background after the shown
/// Space's active tab (a new tab when the Space has none); an active one is a new tab.
fn command(model: &Model, ask: Ask) -> Command {
    match ask {
        Ask::Open {
            url,
            container,
            active,
        } => {
            let space = model.active_space();
            let opener = model
                .spaces
                .iter()
                .find(|s| s.id == space)
                .and_then(|s| s.active);
            match opener {
                Some(opener) if !active => Command::BackgroundTab { url, opener },
                _ => Command::NewTab {
                    url,
                    container,
                    opener: None,
                },
            }
        }
        Ask::Close(tab) => Command::CloseTab(tab),
        Ask::Activate(tab) => Command::ActivateTab(tab),
    }
}

impl Host {
    /// Sends the model's projection to the engine (which skips an unchanged one).
    pub(super) fn ext_push(&mut self) {
        if !self.engine.ext_apis() {
            return;
        }
        let views = &self.views;
        let p = project(&self.model, |tab| views.contains_key(&tab));
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
        self.engine.set_ext_projection(p.windows, tabs);
    }

    /// Applies what extensions asked since the last pump.
    pub(super) fn ext_requests(&mut self) {
        for request in self.engine.take_ext_requests() {
            let ask = match request {
                ExtRequest::OpenTab {
                    request,
                    container,
                    url,
                    active,
                    ..
                } => {
                    let known: Vec<TabId> = self.model.tabs.keys().copied().collect();
                    let open = Ask::Open {
                        url,
                        container,
                        active,
                    };
                    self.dispatch(command(&self.model, open));
                    // The tab the command added, by its view.
                    let view = self
                        .model
                        .tabs
                        .keys()
                        .find(|t| !known.contains(t))
                        .and_then(|t| self.views.get(t).copied());
                    self.engine.answer_open_tab(request, view);
                    continue;
                }
                ExtRequest::CloseTab(view) => self.tab_of(view).map(Ask::Close),
                ExtRequest::ActivateTab(view) => self.tab_of(view).map(Ask::Activate),
                ExtRequest::ActionChanged { extension } => {
                    eprintln!("wubble: extension {extension}: its action changed");
                    None
                }
            };
            if let Some(ask) = ask {
                self.dispatch(command(&self.model, ask));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wubble_core::space::Item;

    fn with_active(active: Option<u64>) -> Model {
        let mut m = Model::new(Instant::now());
        m.spaces[0].items = vec![Item::Tab(TabId(1))];
        m.spaces[0].active = active.map(TabId);
        m
    }

    fn open(active: bool) -> Ask {
        Ask::Open {
            url: "https://example.com/".into(),
            container: Some(ContainerId(2)),
            active,
        }
    }

    #[test]
    fn an_active_open_is_a_new_tab_in_its_container() {
        assert_eq!(
            command(&with_active(Some(1)), open(true)),
            Command::NewTab {
                url: "https://example.com/".into(),
                container: Some(ContainerId(2)),
                opener: None,
            }
        );
    }

    #[test]
    fn an_inactive_open_goes_in_the_background_after_the_shown_spaces_active_tab() {
        assert_eq!(
            command(&with_active(Some(1)), open(false)),
            Command::BackgroundTab {
                url: "https://example.com/".into(),
                opener: TabId(1),
            }
        );
    }

    #[test]
    fn an_inactive_open_without_an_active_tab_is_a_new_tab() {
        assert!(matches!(
            command(&with_active(None), open(false)),
            Command::NewTab { .. }
        ));
    }

    #[test]
    fn close_and_activate_are_the_models_own_commands() {
        let m = with_active(Some(1));
        assert_eq!(
            command(&m, Ask::Close(TabId(1))),
            Command::CloseTab(TabId(1))
        );
        assert_eq!(
            command(&m, Ask::Activate(TabId(1))),
            Command::ActivateTab(TabId(1))
        );
    }
}
