//! The surfaces over the page (`wubble_core::overlay`): the host derives the stack from each
//! surface's own owner, runs the overlays' pure transition and carries out its effects through
//! those owners, and answers what covers a page (`overlay::covers`). Every way an overlay opens
//! asks here first; Escape, a tab switch and a navigation come here before anything else sees
//! them.

use slint::ComponentHandle;
use wubble_core::devtools::PANEL_TAB;
use wubble_core::overlay::{self, Coverage, Effect, Event, Facts, Kind, Layer, Stack};
use wubble_core::print::Sheet;
use wubble_core::space::SpaceId;
use wubble_core::tab::TabId;
use wubble_ui::{
    AgentWidgets, AppWindow, CommandBarState, CommandPlacement, DropMarkKind, GlassLayer,
};

use super::Host;
use crate::menu::{Menu, MenuEvent};
use crate::prompt::{Auth, AuthEvent, PermissionEvent, PermissionPrompt};
use crate::surface::{Dialog, DialogEvent, FindBar, FindEvent};

impl Host {
    /// Every surface on screen in this window now, with `shown` the visible tab: each read from
    /// its one owner (its machine, the view the host wrote, the driver's), and what the kit
    /// still draws as it leaves (`AppWindow::mounted`). Derived each time, never kept: no
    /// surface has a second copy here.
    fn surface_stack(&self, window: &AppWindow, shown: Option<TabId>) -> Stack {
        let mut layers = vec![];
        let mut add = |kind, tab| layers.push(Layer::new(kind, tab));
        // ----- the overlays, from their machines
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
        let bar = window.get_command_bar_state();
        let overlay_bar = window.get_command_placement() == CommandPlacement::Overlay;
        if bar == CommandBarState::Open && overlay_bar {
            add(Kind::CommandBar, None);
        }
        // the popover's and the popup's one store is their Slint property (step 2 makes it the
        // host's); read, not copied
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
        // ----- the free surfaces, from what their owners wrote into this window
        let key = self.model.window_key_tab(self.w.id);
        let video = window.get_video();
        let autofill = window.get_autofill();
        #[rustfmt::skip]
        let free = [
            (Kind::ErrorPanel, shown, window.get_page_error().open),
            (Kind::Popup, None, window.get_popup().shown),
            (Kind::Toast, None, window.get_toast_shown()),
            (Kind::Tooltip, None, window.get_tip().shown),
            (Kind::Preview, None, window.get_preview().shown),
            (Kind::AutofillChip, shown, autofill.chip),
            (Kind::AutofillMenu, shown, autofill.menu_open),
            (Kind::AutofillPrompt, shown, autofill.prompt),
            (Kind::VideoStrip, key, video.w > 0.0 && video.open),
            (Kind::VideoTab, key, video.w > 0.0 && video.collapsed),
            (Kind::VideoSpeeds, key, video.w > 0.0 && video.open && video.speeds),
            (Kind::PipStrip, None, window.get_pip_strip()),
            (Kind::Reader, shown, window.get_reader_on()),
            (Kind::Boost, shown, window.get_boost().open),
            (Kind::Swipe, None, window.get_swipe_progress() != 0.0),
            (Kind::ExitPage, None, window.get_exit_t() < 1.0 && window.get_exit_page().size().width > 0),
            (Kind::Entrance, None, window.get_page_enter() < 1.0),
            (Kind::Peek, None, window.get_peeking()),
            (Kind::SeamBow, None, window.get_seam_bowed() || window.get_edge_bowed()),
            (Kind::SplitRing, None, !window.get_focus_ring().is_empty()),
            (Kind::SplitSeams, None, slint::Model::row_count(&window.get_seams()) > 0),
            (Kind::DevTools, Some(PANEL_TAB), self.panel_width() > 0.0),
            (Kind::TabGhost, None, !window.get_drag_ghost().is_empty()),
            (Kind::DropZone, None, window.get_drag_zone() > 0),
            (Kind::PageGhost, None, window.get_page_ghost().shown),
            (Kind::DropMark, None, window.get_drop_mark().kind != DropMarkKind::None),
            (Kind::Autoscroll, None, window.get_autoscroll_on()),
            (Kind::Specimen, None, window.global::<AgentWidgets>().get_specimen()),
        ];
        for (kind, tab, on) in free {
            if on {
                add(kind, tab);
            }
        }
        // the status bubble is the host's for a pane on screen here
        if let Some((tab, _)) = self.status.shown()
            && self.w.frames.contains_key(&tab)
        {
            add(Kind::Status, Some(tab));
        }
        // ----- what the kit still draws: open, or leaving (the command bar's exit is the
        // driver's, the menu's the controls registry's, both seen in what they report)
        let m = window.get_mounted();
        #[rustfmt::skip]
        let drawn = [
            (Kind::Permission, shown, m.permission),
            (Kind::Find, shown, m.find),
            (Kind::Dialog, shown, m.dialog),
            (Kind::BookmarkSheet, shown, m.bookmark_sheet),
            (Kind::PrintSheet, shown, m.print_sheet),
            (Kind::ExtensionSheet, None, m.extension_sheet),
            (Kind::AgentSheet, None, m.agent_sheet),
            (Kind::Region, shown, m.region),
            (Kind::CommandBar, None, bar != CommandBarState::Closed && overlay_bar),
            (Kind::Downloads, None, m.downloads),
            (Kind::Extensions, None, m.extensions),
            (Kind::Spaces, None, m.spaces),
            (Kind::Menu, None, m.menu),
            (Kind::ErrorPanel, shown, m.error),
            (Kind::Toast, None, m.toast),
            (Kind::Tooltip, None, m.tooltip),
            (Kind::Preview, None, m.preview),
            (Kind::AutofillMenu, shown, m.autofill_menu),
            (Kind::AutofillPrompt, shown, m.autofill_prompt),
            (Kind::VideoStrip, key, m.video_strip),
            (Kind::VideoTab, key, m.video_tab),
            (Kind::VideoSpeeds, key, m.video_speeds),
            (Kind::PipStrip, None, m.pip_strip),
            (Kind::Reader, shown, m.reader),
            (Kind::Boost, shown, m.boost),
        ];
        let drawn: Vec<Layer> = drawn
            .into_iter()
            .filter(|&(_, _, on)| on)
            .map(|(kind, tab, _)| Layer::new(kind, tab))
            .collect();
        Stack {
            shown,
            layers,
            leaving: vec![],
        }
        .with_drawn(&drawn)
    }

    /// What the shell draws over the page in `pane` of this window now: the page layer's
    /// `Need::Covered` (`Needs::covered`). The peek and the picture-in-picture card are their
    /// owners' (the model's), so they come as facts.
    #[expect(dead_code, reason = "the page layer's host wiring (the next step) reads it")]
    pub(super) fn page_covered(&self, window: &AppWindow, pane: TabId) -> Coverage {
        let facts = Facts {
            glass: window.global::<GlassLayer>().get_over_on(),
            glance: self.model.glance_of(self.w.id).is_some() || window.get_mounted().glance,
            pip_card: self.pip_card(),
        };
        overlay::covers(&self.surface_stack(window, self.shown()), &facts, pane)
    }

    /// Runs `event` with `shown` the visible tab before it; carries out the closes, hides and
    /// reveals, and returns the effects. The caller redraws.
    pub(super) fn overlay_event(
        &mut self,
        window: &AppWindow,
        shown: Option<TabId>,
        event: Event,
    ) -> Vec<Effect> {
        let (_, effects) = overlay::transition(self.surface_stack(window, shown), event);
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

    /// Escape: closes the topmost overlay that takes it from whoever has the keys (the page's
    /// Escape never answers a notice or a permission prompt); whether one closed.
    pub(super) fn overlay_escape(&mut self, window: &AppWindow) -> bool {
        let shown = self.shown();
        let event = if window.get_web_focused() {
            Event::PageEscape
        } else {
            Event::Escape
        };
        let closed = !self.overlay_event(window, shown, event).is_empty();
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
            // the free surfaces: no rule of the overlays closes one (`overlay::Group::Free`)
            (
                Kind::ErrorPanel
                | Kind::Popup
                | Kind::Toast
                | Kind::Tooltip
                | Kind::Preview
                | Kind::Status
                | Kind::AutofillChip
                | Kind::AutofillMenu
                | Kind::AutofillPrompt
                | Kind::VideoStrip
                | Kind::VideoTab
                | Kind::VideoSpeeds
                | Kind::PipStrip
                | Kind::PipCard
                | Kind::Reader
                | Kind::Boost
                | Kind::Glance
                | Kind::Swipe
                | Kind::ExitPage
                | Kind::Entrance
                | Kind::Peek
                | Kind::SeamBow
                | Kind::SplitRing
                | Kind::SplitSeams
                | Kind::DevTools
                | Kind::TabGhost
                | Kind::DropZone
                | Kind::PageGhost
                | Kind::DropMark
                | Kind::Autoscroll
                | Kind::Specimen,
                _,
            ) => {}
        }
    }
}
