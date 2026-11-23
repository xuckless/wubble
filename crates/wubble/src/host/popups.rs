//! Windows a page opens (`window.open`, `target=_blank`): where each goes by the policy table
//! (`wubble_core::popup`), and the pop-up browsers Chromium created for them adopted as the new
//! tab's view (the opener relation holds: sign-in pop-ups post their result back and close
//! themselves). Then `--smoke popup`'s executor (`popup_smoke::next` decides).

use std::time::Duration;

use slint::winit_030::winit::keyboard::ModifiersState;
use wubble_core::model::Command;
use wubble_core::site::Permission;
use wubble_core::tab::TabId;
use wubble_core::window::WindowKind;
use wubble_engine::input::{Buttons, Modifiers, MouseButton, Point};

use wubble_engine::app::ViewId;
use wubble_engine::nav::PopupRequest;
use wubble_engine::view::Layout;

use super::{Host, new_tab};
use crate::popup_smoke::{self, Action, PopupSeen, Seen};

impl Host {
    /// A page's `window.open` or `target=_blank` that Chromium left to the host: what the
    /// policy table says (a tab, a Popup window, or held back for the address pill).
    pub(super) fn popup_requested(&mut self, tab: TabId, request: PopupRequest) {
        self.place_opened(tab, request, None);
    }

    /// Places what `tab`'s page opened: `view` is the browser Chromium created for it
    /// (`WebEvent::PopupOpened`), shown by the tab or window the table opens and closed when
    /// nothing opens; without one the new tab gets a page of its own.
    pub(super) fn place_opened(&mut self, tab: TabId, request: PopupRequest, view: Option<ViewId>) {
        use wubble_core::popup::{self, Outcome};
        // An agent session's page opens nothing off the web (wubble would load it as the
        // browser, which the page itself may not).
        if self.agent_popup_refused(tab, &request.url) {
            self.drop_popup(view);
            return;
        }
        // A `target=_blank` mailto: link opens the app and no tab, as in Chromium.
        if wubble_engine::page::is_external(&request.url) {
            self.drop_popup(view);
            self.external_link(tab, request.url, Some(request.gesture));
            return;
        }
        let site = self.site_of(tab);
        let remembered = site
            .as_deref()
            .and_then(|o| self.site_view(tab).permissions().get(o, Permission::Popups));
        let glance = self.glance_enabled;
        let url = request.url.clone();
        let command = match popup::decide(request.opened, request.gesture, remembered, glance) {
            Outcome::Tab { foreground: true } => new_tab(url, Some(tab)),
            Outcome::Tab { foreground: false } => Command::BackgroundTab { url, opener: tab },
            Outcome::PopupWindow => Command::OpenPopupWindow {
                url,
                opener: tab,
                size: request.size,
            },
            // a peek and a held request get a page of their own when they open
            Outcome::Glance if view.is_none() => match self.model.window_of(tab) {
                Some(window) => Command::OpenGlance {
                    window,
                    opener: tab,
                    url,
                },
                None => new_tab(url, Some(tab)),
            },
            Outcome::Blocked if view.is_none() => {
                let held = self.held_popups.entry(tab).or_default();
                if held.len() < super::MAX_HELD_POPUPS {
                    held.push(request);
                }
                self.after_change();
                return;
            }
            Outcome::Glance | Outcome::Blocked => {
                self.drop_popup(view);
                return;
            }
        };
        self.adopting = view.map(|v| (request.url, v));
        self.dispatch(command);
        // the command opened no tab for it
        let unused = self.adopting.take().map(|(_, v)| v);
        self.drop_popup(unused);
    }

    /// A pop-up browser nothing shows: closed (its page's `window.open` handle sees it closed).
    fn drop_popup(&mut self, view: Option<ViewId>) {
        if let Some(view) = view {
            self.engine.close_view(view);
        }
    }

    /// The pop-up browser waiting for the tab being created on `url`, taken.
    pub(super) fn adopted(&mut self, url: &str) -> Option<ViewId> {
        match self.adopting.take() {
            Some((u, view)) if u == url => Some(view),
            other => {
                self.adopting = other;
                None
            }
        }
    }

    /// The adopted pop-up's view laid out and paced as a view the tab opened would be.
    pub(super) fn adopt_view(
        &mut self,
        id: ViewId,
        layout: Layout,
        frame_rate: u32,
    ) -> Result<ViewId, wubble_engine::app::EngineError> {
        let view = self.engine.view(id).ok_or_else(|| {
            wubble_engine::app::EngineError::View("the pop-up closed before it was shown".into())
        })?;
        view.set_layout(layout);
        view.set_frame_rate(frame_rate);
        Ok(id)
    }

    /// The opener page's tab (`popup_smoke::opener_url`).
    fn popup_opener(&self) -> Option<TabId> {
        self.model
            .tabs
            .values()
            .find(|t| t.url.starts_with("http://127.0.0.1:") && t.url.ends_with("/popup"))
            .map(|t| t.id)
    }

    /// The newest Popup window and its tab.
    fn newest_popup_window(&self) -> Option<(wubble_core::window::WindowId, TabId)> {
        self.model
            .windows
            .iter()
            .filter(|w| w.kind() == WindowKind::Popup)
            .max_by_key(|w| w.id)
            .and_then(|w| Some((w.id, w.tab()?)))
    }

    /// Whether `tab`'s window has a frame of it.
    fn framed_in_its_window(&mut self, tab: TabId) -> bool {
        let framed = self
            .model
            .window_of(tab)
            .is_some_and(|id| self.enter(id) && self.w.frames.contains_key(&tab));
        self.enter_focused();
        framed
    }

    fn popup_seen(&mut self) -> Seen {
        let opener = self.popup_opener();
        let popup_tabs: Vec<TabId> = self
            .model
            .windows
            .iter()
            .filter(|w| w.kind() == WindowKind::Popup)
            .filter_map(|w| w.tab())
            .collect();
        let popup = self.newest_popup_window().map(|(_, tab)| {
            let (title, url) = self
                .model
                .tabs
                .get(&tab)
                .map(|t| (t.title.clone(), t.url.clone()))
                .unwrap_or_default();
            PopupSeen {
                title,
                url,
                framed: self.framed_in_its_window(tab),
            }
        });
        let (mut after_opener, mut misplaced) = (vec![], false);
        if let Some(opener) = opener
            && let Some((space, _)) = self.model.location(opener)
        {
            let container = self.model.tabs[&opener].container;
            let order = self.model.order(space);
            let at = order.iter().position(|&t| t == opener).unwrap_or(0);
            after_opener = order[at + 1..]
                .iter()
                .filter(|t| !popup_tabs.contains(t))
                .filter_map(|t| self.model.tabs.get(t))
                .map(|t| t.title.clone())
                .collect();
            misplaced = self.model.tabs.values().any(|t| {
                t.url.contains("/popup/")
                    && !popup_tabs.contains(&t.id)
                    && (t.container != container
                        || self.model.location(t.id).map(|(s, _)| s) != Some(space))
            });
        }
        let opener_framed = opener.is_some_and(|tab| {
            self.model.visible_tabs().contains(&tab) && self.framed_in_its_window(tab)
        });
        Seen {
            opener: opener
                .and_then(|t| self.model.tabs.get(&t))
                .map(|t| t.title.clone()),
            opener_framed,
            popup_windows: popup_tabs.len(),
            popup,
            after_opener,
            misplaced,
            held: opener.map_or(0, |t| self.held_popups.get(&t).map_or(0, Vec::len)),
        }
    }

    pub(super) fn popup_smoke_step(&mut self) {
        let Some(state) = self.checks.popup_smoke.take() else {
            return;
        };
        let seen = self.popup_seen();
        let (next, action) = popup_smoke::next(state, &seen);
        self.checks.popup_smoke = Some(next);
        if next.stage != state.stage {
            println!(
                "smoke: popup {:?} -> {:?}: {seen:?}",
                state.stage, next.stage
            );
        }
        let opener = self.popup_opener();
        // the opener's tab is shown again after a tab it opened took the screen
        if action.is_none()
            && let Some(tab) = opener
            && !self.model.visible_tabs().contains(&tab)
        {
            self.dispatch(Command::ActivateTab(tab));
        }
        let popup = self.newest_popup_window();
        match action {
            None => {}
            Some(Action::Click(at)) => {
                if let Some(view) = opener.and_then(|t| self.tab_view(t)) {
                    click(view, at);
                }
            }
            Some(Action::InOpener(script)) => {
                if let Some(view) = opener.and_then(|t| self.tab_view(t)) {
                    view.run_script(script);
                }
            }
            Some(Action::InPopup(script)) => {
                if let Some(view) = popup.and_then(|(_, t)| self.tab_view(t)) {
                    view.run_script(script);
                }
            }
            Some(Action::ClosePopupWindow) => {
                if let Some((window, _)) = popup {
                    self.dispatch(Command::CloseWindow(window));
                }
            }
            Some(Action::CloseOpener) => {
                if let Some(tab) = opener {
                    self.dispatch(Command::CloseTab(tab));
                }
            }
            Some(Action::Pass) => {
                println!(
                    "smoke: popup badge held={} (blocked without a gesture)",
                    seen.held
                );
                self.popup_passed = true;
                crate::quit();
            }
            Some(Action::Fail(reason)) => self.fail(reason),
        }
    }
}

/// A left click through CEF's input, as a user's: a user gesture for the page.
fn click(view: &mut wubble_engine::view::View, at: Point) {
    let up = Modifiers {
        keys: ModifiersState::empty(),
        buttons: Buttons::default(),
        locks: Default::default(),
    };
    let down = Modifiers {
        buttons: Buttons::default().with(MouseButton::Left, true),
        ..up
    };
    view.forget_clicks();
    view.focus(true);
    view.mouse_move(at, &up, Duration::ZERO);
    view.mouse_button(MouseButton::Left, true, at, &down, Duration::ZERO);
    view.mouse_button(MouseButton::Left, false, at, &up, Duration::ZERO);
}
