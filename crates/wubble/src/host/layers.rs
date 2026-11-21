//! The overlays' order and scope (`wubble_core::overlay`): the host reads the stack from each
//! overlay's own machine, runs the pure transition, and carries out its effects through those
//! machines. Every way an overlay opens asks here first; Escape, a tab switch and a navigation
//! come here before anything else sees them.

use wubble_core::overlay::{self, Effect, Event, Kind, Layer, Stack};
use wubble_core::print::Sheet;
use wubble_core::space::SpaceId;
use wubble_core::tab::TabId;
use wubble_ui::{AppWindow, CommandBarState, CommandPlacement};

use super::Host;
use crate::menu::{Menu, MenuEvent};
use crate::prompt::{Auth, AuthEvent, PermissionEvent, PermissionPrompt};
use crate::surface::{Dialog, DialogEvent, FindBar, FindEvent};

impl Host {
    /// What is open now, with `shown` the visible tab.
    fn overlay_stack(&self, window: &AppWindow, shown: Option<TabId>) -> Stack {
        let mut layers = vec![];
        let mut add = |kind, tab| layers.push(Layer::new(kind, tab));
        if let Some(tab) = self.notice_tab() {
            add(Kind::Notice, Some(tab));
        }
        for (&tab, s) in &self.surfaces {
            if matches!(s.permission, PermissionPrompt::Open { .. }) {
                add(Kind::Permission, Some(tab));
            }
            if matches!(s.find, FindBar::Open { .. }) {
                add(Kind::Find, Some(tab));
            }
            if matches!(s.dialog, Dialog::Open(_)) || matches!(s.auth, Auth::Open(_)) {
                add(Kind::Dialog, Some(tab));
            }
        }
        if let Some(tab) = self.bookmark_sheet_tab() {
            add(Kind::BookmarkSheet, Some(tab));
        }
        if let Sheet::Open { tab, .. } | Sheet::Working { tab, .. } = &self.print {
            add(Kind::PrintSheet, Some(*tab));
        }
        if self.agent_sheet_open() {
            add(Kind::AgentSheet, None);
        }
        if let Some(tab) = self.region_tab() {
            add(Kind::Region, Some(tab));
        }
        if let Some(tab) = self.hints_tab() {
            add(Kind::Hints, Some(tab));
        }
        // the start page's capsule is the page's own field, not an overlay
        if window.get_command_bar_state() == CommandBarState::Open
            && window.get_command_placement() == CommandPlacement::Overlay
        {
            add(Kind::CommandBar, None);
        }
        if window.get_downloads_open() {
            add(Kind::Downloads, None);
        }
        if self.ext_ui_prompt_open() {
            add(Kind::ExtensionSheet, None);
        }
        if self.ext_ui_panel_open() {
            add(Kind::Extensions, None);
        }
        if window.get_spaces_open() {
            add(Kind::Spaces, None);
        }
        if shown
            .and_then(|t| self.surfaces.get(&t))
            .is_some_and(|s| matches!(s.menu, Menu::Open { .. } | Menu::Shell { .. }))
        {
            add(Kind::Menu, None);
        }
        Stack { shown, layers }
    }

    /// Runs `event` with `shown` the visible tab before it; carries out the closes, hides and
    /// reveals, and returns the effects. The caller redraws.
    pub(super) fn overlay_event(
        &mut self,
        window: &AppWindow,
        shown: Option<TabId>,
        event: Event,
    ) -> Vec<Effect> {
        let (_, effects) = overlay::transition(self.overlay_stack(window, shown), event);
        for effect in &effects {
            match *effect {
                Effect::Close(layer) => self.overlay_close(window, shown, layer),
                Effect::Hide(Layer {
                    kind: Kind::Notice, ..
                }) => self.notice_shown(false),
                Effect::Reveal(Layer {
                    kind: Kind::Notice, ..
                }) => self.notice_shown(true),
                // the find bar and a dialog are drawn for the visible tab only (render_surfaces)
                Effect::Hide(_) | Effect::Reveal(_) | Effect::Open(_) => {}
            }
        }
        effects
    }

    /// Asks to open `kind` (for `tab` when it is the tab's): whatever it replaces is closed;
    /// false when it may not open now.
    pub(super) fn overlay_open(&mut self, kind: Kind, tab: Option<TabId>) -> bool {
        let Some(window) = self.w.window.upgrade() else {
            return true;
        };
        let layer = Layer::new(kind, tab);
        let shown = self.shown();
        self.overlay_event(&window, shown, Event::Open(layer))
            .contains(&Effect::Open(layer))
    }

    /// The footer's "^" (or the current Space's tile): the Spaces popup opens when the overlays
    /// let it (it closes the downloads popover or a menu; a page's dialog refuses it).
    pub fn spaces_menu_requested(&mut self) {
        self.hover_dismiss();
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if window.get_spaces_open() || !self.overlay_open(Kind::Spaces, None) {
            return;
        }
        window.set_spaces_open(true);
        self.render_surfaces(&window);
    }

    /// The dustbin on an agent Space's row: that Space is cleared (`agent_space_cleared`); the
    /// popup stays open over the rows left, and a window that showed the Space shows another.
    pub fn space_cleared(&mut self, space: SpaceId) {
        self.agent_space_cleared(space);
        if let Some(window) = self.w.window.upgrade() {
            self.render_surfaces(&window);
        }
    }

    /// This window's Spaces popup (wubble_core::spacemenu), as drawn and as dropped on.
    pub(super) fn spaces_popup(&self) -> wubble_core::spacemenu::Popup {
        wubble_core::spacemenu::popup(
            &self.model.spaces,
            crate::shell::window_space(&self.model, self.w.id),
        )
    }

    /// Escape: closes the topmost overlay; whether there was one to close.
    pub(super) fn overlay_escape(&mut self, window: &AppWindow) -> bool {
        let shown = self.shown();
        let closed = !self.overlay_event(window, shown, Event::Escape).is_empty();
        if closed {
            self.render_surfaces(window);
        }
        closed
    }

    fn overlay_close(&mut self, window: &AppWindow, shown: Option<TabId>, layer: Layer) {
        match (layer.kind, layer.tab) {
            (Kind::Notice, _) => self.notice_close(),
            (Kind::Permission, Some(tab)) => self.apply_permission(
                tab,
                PermissionEvent::Answer {
                    allow: false,
                    remember: false,
                },
            ),
            (Kind::Find, Some(tab)) => self.apply_find(tab, FindEvent::Close),
            (Kind::Dialog, Some(tab)) => {
                if self
                    .surfaces
                    .get(&tab)
                    .is_some_and(|s| matches!(s.auth, Auth::Open(_)))
                {
                    self.apply_auth(tab, AuthEvent::Cancel);
                } else {
                    self.apply_dialog(tab, DialogEvent::Cancel);
                }
            }
            (Kind::BookmarkSheet, _) => self.bookmark_sheet_answered(false, false, "", -1),
            (Kind::PrintSheet, _) => self.print_event(wubble_core::print::Event::Cancel),
            (Kind::AgentSheet, _) => self.agent_sheet_answered(false, false),
            (Kind::Region, _) => self.shot_cancelled(),
            (Kind::Hints, _) => self.hints_cancel(),
            // later, outside the host: closing the bar calls back into it
            (Kind::CommandBar, _) => {
                let _ = self.w.window.upgrade_in_event_loop(|window| {
                    window.invoke_command_dismissed();
                    window.invoke_dismiss_command_bar();
                });
            }
            (Kind::Downloads, _) => window.set_downloads_open(false),
            (Kind::Spaces, _) => window.set_spaces_open(false),
            (Kind::Extensions, _) => self.extensions_dismissed(),
            (Kind::ExtensionSheet, _) => self.extension_prompt_answered(false),
            (Kind::Menu, _) => {
                if let Some(tab) = shown {
                    self.apply_menu(tab, MenuEvent::Dismiss);
                }
            }
            (Kind::Find | Kind::Dialog | Kind::Permission, None) => {}
        }
    }
}
