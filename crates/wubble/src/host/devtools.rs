//! DevTools for the visible tab: the dock's lifecycle is `wubble_core::devtools`; this executes
//! its effects. The docked panel is an ordinary view (the DevTools frontend page served by the
//! remote debugging endpoint) shown as one more pane at the window's right edge, host-only (no
//! tab) under the key `PANEL_TAB`; its edge is the Source panel's string (`SourceSeam`).

use slint::ComponentHandle;
use wubble_core::devtools::{
    Dock, DockEffect, DockEvent, Mode, PANEL_TAB, Point as DevPoint, transition,
};
use wubble_core::motion::spring;
use wubble_core::space::{ContainerId, Side};
use wubble_core::split::SourceSeam;
use wubble_core::tab::TabId;
use wubble_engine::app::ViewId;
use wubble_engine::view::ViewConfig;
use wubble_ui::AppWindow;

use super::{Host, new_tab};
use crate::page_smoke::Panel;

/// The one owner of the dock's state.
pub(super) struct Docked {
    pub dock: Dock,
    pub mode: Mode,
    /// The panel's view while the dock is [`Dock::Open`].
    pub view: Option<ViewId>,
    pub seam: SourceSeam,
    /// The frontend's page title.
    pub title: String,
}

impl Docked {
    pub fn new(port: Option<u16>) -> Self {
        Self {
            dock: Dock::Closed,
            mode: Mode::of(port),
            view: None,
            seam: SourceSeam::default(),
            title: String::new(),
        }
    }

    /// The panel's width in the layout: 0 while it is closed (open from the moment it starts
    /// to open, so the view gets its pane's layout).
    pub fn width(&self) -> f64 {
        if self.seam.open {
            self.seam.w.x.max(1.0)
        } else {
            0.0
        }
    }
}

impl Host {
    /// The debugging port that refuses every agent session (`agent::refusing_port`): the one the
    /// engine of this run opened, else the stored one.
    pub(super) fn debug_port(&self) -> Option<u16> {
        use wubble_core::apply::value_of;
        use wubble_core::settings as defs;
        let opened = match self.devtools.mode {
            Mode::Docked { port } => Some(port),
            Mode::DetachedOnly => None,
        };
        let stored = defs::remote_port(
            value_of(&defs::DEVTOOLS_REMOTE_PORT, &self.settings.stored)
                .parse()
                .unwrap_or(0),
        );
        wubble_core::agent::refusing_port(opened, stored)
    }

    /// The docked panel's width in this window: it docks in the window showing the tab it
    /// inspects, 0 in the others and while it is closed.
    pub(super) fn panel_width(&self) -> f64 {
        let here = self
            .devtools
            .dock
            .tab()
            .is_some_and(|t| self.model.window_of(t) == Some(self.w.id));
        if here { self.devtools.width() } else { 0.0 }
    }

    /// The panel's view when keys go to it: the last press was in it.
    pub(super) fn keys_to_panel(&self) -> Option<ViewId> {
        (self.key_pane == Some(PANEL_TAB))
            .then_some(self.devtools.view)
            .flatten()
    }

    /// The panel's own view.
    pub(super) fn panel_view(&mut self) -> Option<&mut wubble_engine::view::View> {
        let id = self.devtools.view?;
        self.engine.view(id)
    }

    /// F12: DevTools for the visible tab.
    pub(super) fn toggle_devtools(&mut self) {
        let Some(tab) = self.shown() else {
            return;
        };
        self.reconcile_window();
        self.dock_event(DockEvent::Toggle(tab));
    }

    /// Ctrl+Shift+J: DevTools on its Console panel for the visible tab.
    pub(super) fn devtools_console(&mut self) {
        let Some(tab) = self.shown() else {
            return;
        };
        self.reconcile_window();
        self.dock_event(DockEvent::Console(tab));
    }

    /// The context menu's Inspect at `at`, physical pixels relative to the view.
    pub(super) fn inspect_at(&mut self, tab: TabId, at: (i32, i32)) {
        // the detached window is the only way to reveal a node (Decided "DevTools")
        let scale = self
            .w
            .window
            .upgrade()
            .map_or(1.0, |w| w.window().scale_factor());
        let css = |v: i32| (v as f32 / scale).round() as i32;
        self.dock_event(DockEvent::InspectAt(
            tab,
            DevPoint {
                x: css(at.0),
                y: css(at.1),
            },
        ));
    }

    /// A native window the user closed is not open any more: the next toggle opens, not closes.
    fn reconcile_window(&mut self) {
        if let Dock::Detached { tab } = self.devtools.dock
            && self.tab_view(tab).is_some_and(|v| !v.has_devtools())
        {
            self.dock_event(DockEvent::WindowClosed(tab));
        }
    }

    pub(super) fn dock_event(&mut self, event: DockEvent) {
        let dock = std::mem::take(&mut self.devtools.dock);
        let (dock, effects) = transition(dock, self.devtools.mode, event);
        self.devtools.dock = dock;
        for effect in effects {
            self.dock_effect(effect);
        }
        self.after_change();
    }

    fn dock_effect(&mut self, effect: DockEffect) {
        match effect {
            DockEffect::AskTarget(tab) => match self.tab_view(tab) {
                Some(view) => view.request_target_id(),
                None => self.dock_event(DockEvent::TargetFailed(tab)),
            },
            DockEffect::OpenFrontend { tab, url } => {
                if !self.open_panel(tab, url) {
                    self.dock_event(DockEvent::OpenFailed(tab));
                }
            }
            DockEffect::CloseFrontend => self.close_panel(),
            DockEffect::OpenDetached { tab, inspect } => {
                if let Some(view) = self.tab_view(tab) {
                    view.open_devtools_at(inspect.map(|p| (p.x, p.y)));
                }
            }
            DockEffect::CloseDetached(tab) => {
                if let Some(view) = self.tab_view(tab) {
                    view.close_devtools();
                }
            }
        }
    }

    /// Opens the panel's view, the docked front end inspecting `tab` at `url`; false when it
    /// cannot be (no window yet, an unknown tab, nothing listening on the endpoint's port, the
    /// engine refused), so the dock falls back to the window.
    fn open_panel(&mut self, tab: TabId, url: String) -> bool {
        let (Some(window), Some(frame_rate)) = (self.w.window.upgrade(), self.frame_rate) else {
            return false;
        };
        let Some(inspected) = self.model.tabs.get(&tab).map(|t| t.container) else {
            return false;
        };
        if self.devtools.view.is_some() {
            return true;
        }
        // Chromium skips the endpoint silently when its port is taken or unbindable; loopback
        // answers a connect at once, so this is no wait.
        if let Mode::Docked { port } = self.devtools.mode
            && std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).is_err()
        {
            eprintln!("wubble: DevTools endpoint is not listening on port {port}");
            return false;
        }
        // Opens at its width (the Source panel's string animates a pull, not the opening).
        let seam = &mut self.devtools.seam;
        seam.open = true;
        seam.w = spring(seam.want);
        let layout = self.pane_layout(&window, PANEL_TAB);
        let container = match self.model.side_of_container(inspected) {
            Side::Normal => ContainerId::DEFAULT,
            Side::Private => inspected,
        };
        let config = ViewConfig {
            url,
            layout,
            frame_rate,
            begin_frames: self.begin_frames(),
            gpu: self.w.presenters[0].context().clone(),
            // The front end is wubble's own page; it keeps its settings in the default store,
            // or for a private tab in the private windows' container (in memory, as Chrome's
            // DevTools of an incognito tab run off the record).
            container,
            storage: self.model.storage_of(container),
        };
        match self.engine.open_view(config) {
            Ok(id) => {
                self.devtools.view = Some(id);
                self.devtools.title.clear();
                true
            }
            Err(e) => {
                eprintln!("wubble: DevTools panel: {e}");
                self.devtools.seam.open = false;
                self.devtools.seam.w = spring(0.0);
                false
            }
        }
    }

    fn close_panel(&mut self) {
        if let Some(id) = self.devtools.view.take() {
            self.engine.close_view(id);
        }
        let seam = &mut self.devtools.seam;
        seam.open = false;
        seam.held = false;
        seam.w = spring(0.0);
        if self.key_pane == Some(PANEL_TAB) {
            self.key_pane = None;
        }
        for w in std::iter::once(&mut self.w).chain(self.parked.values_mut()) {
            w.frames.remove(&PANEL_TAB);
            w.release_slots(|t| t == PANEL_TAB);
        }
    }

    /// The panel's view closed on its own, or the seam was pulled shut.
    pub(super) fn panel_closed(&mut self) {
        self.dock_event(DockEvent::PanelClosed);
    }

    /// Events of the panel's view: its title (the smoke's proof it attached), a link it opens
    /// in a tab beside the inspected page, keys it left alone (F12 still toggles).
    pub(super) fn on_panel_event(&mut self, event: wubble_engine::nav::WebEvent) {
        use wubble_engine::nav::WebEvent;
        match event {
            WebEvent::TitleChanged(title) => self.devtools.title = title,
            WebEvent::PopupRequested(request) => {
                let opener = self.devtools.dock.tab();
                self.dispatch(new_tab(request.url, opener));
            }
            WebEvent::KeyUnhandled { key, editable } => {
                if let Some(action) = self.page_first.unhandled(key) {
                    self.perform_unhandled(action, editable);
                }
            }
            _ => {}
        }
    }

    /// The panel's page events: nothing of the page's own UI applies to it (see
    /// [`refuse_requests`]).
    pub(super) fn on_panel_page_event(&mut self, event: wubble_engine::page::PageEvent) {
        if let Some(view) = self.panel_view() {
            refuse_requests(view, event);
        }
    }

    /// The tab the dock is for went away or is not on screen: a docked panel goes with it.
    pub(super) fn dock_follows_tab(&mut self) {
        let docked = matches!(
            self.devtools.dock,
            Dock::Open { .. } | Dock::Resolving { .. }
        );
        if let (true, Some(tab)) = (docked, self.devtools.dock.tab())
            && !self.model.visible_tabs().contains(&tab)
        {
            self.dock_event(DockEvent::TabGone(tab));
        }
    }

    /// `--smoke page`: what the dock shows.
    pub(super) fn dock_seen(&self) -> Panel {
        match self.devtools.dock {
            Dock::Open { .. } => Panel::Docked {
                framed: self.w.frames.contains_key(&PANEL_TAB),
                titled: self.devtools.title.starts_with("DevTools - "),
            },
            Dock::Detached { .. } => Panel::Detached,
            Dock::Closed | Dock::Resolving { .. } => Panel::Closed,
        }
    }

    /// The keyboard's nudge of the docked panel's seam (ArrowLeft widens: `dir` 1), with its
    /// twang (`SourceSeam::nudge`). Nothing while no panel is docked.
    pub(super) fn source_nudge(&mut self, dir: f64) {
        if !self.devtools.seam.open {
            return;
        }
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let h =
            f64::from(window.window().size().height) / f64::from(window.window().scale_factor());
        self.devtools.seam.nudge(dir, h);
        window.window().request_redraw();
    }

    /// The seam's pointer gestures (window pixels): the panel's edge is the last seam.
    pub(super) fn source_grab(&mut self, window: &AppWindow, x: f32, y: f32) {
        let (wx, wy) = self.window_point(window, x, y);
        self.devtools.seam.grab(wx, wy);
    }

    pub(super) fn source_drag(&mut self, window: &AppWindow, x: f32, y: f32) {
        let (wx, wy) = self.window_point(window, x, y);
        self.devtools.seam.drag(wx, wy);
    }

    pub(super) fn source_release(&mut self) {
        if self.devtools.seam.release() {
            self.panel_closed();
        }
    }

    /// A point of the content area in the window.
    fn window_point(&self, window: &AppWindow, x: f32, y: f32) -> (f64, f64) {
        (
            f64::from(window.get_content_x() + x),
            f64::from(window.get_content_y() + y),
        )
    }
}

/// A host-only view's page events (the DevTools panel's, an extension popup's): nothing of the
/// page's own UI applies, but every request must be answered or CEF waits on it (the engine is
/// in ask-the-host mode): all are refused.
pub(super) fn refuse_requests(
    view: &mut wubble_engine::view::View,
    event: wubble_engine::page::PageEvent,
) {
    use wubble_engine::page::PageEvent;
    match event {
        PageEvent::ContextMenuRequested { id, .. } => {
            view.answer_context_menu(id, None);
        }
        PageEvent::DialogRequested { id, .. } => {
            view.answer_dialog(id, false, "");
        }
        PageEvent::PermissionRequested { id, .. } => {
            view.answer_permission(id, &[]);
        }
        PageEvent::FileChooserRequested { id, .. } => {
            view.answer_file_chooser(id, None);
        }
        PageEvent::AuthRequested { id, .. } => {
            view.answer_auth(id, None);
        }
        PageEvent::CertificateError { id, .. } => {
            view.answer_certificate(id, false);
        }
        PageEvent::DownloadRequested { id, .. } => {
            view.answer_download_path(id, None);
        }
        _ => {}
    }
}
