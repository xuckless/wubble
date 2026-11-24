//! The surfaces over the page (`wubble_core::overlay`): the host keeps the overlays' one store
//! (`Host::ui`), runs its pure `reduce` and carries out the effects, derives the stack with the
//! free surfaces each owner wrote, and answers what covers a page (`overlay::covers`). Every way
//! an overlay opens asks here first; Escape, a tab switch and a navigation come here before
//! anything else sees them.

use slint::ComponentHandle;
use wubble_core::devtools::PANEL_TAB;
use wubble_core::overlay::{
    self, Action, Cue, Effect, Env, Facts, Key, Kind, Layer, Phase, Scope, Stack, Surface,
};
use wubble_core::print::Sheet;
use wubble_core::space::SpaceId;
use wubble_core::tab::TabId;
use wubble_core::window::{WindowId, WindowKind};
use wubble_ui::{
    AgentWidgets, AppWindow, CommandBarState, CommandPlacement, GlassLayer, Mounted, PeekCue,
    SidebarMode,
};

use super::Host;
use super::window::WindowHost;
use crate::menu::{Menu, MenuEvent};
use crate::prompt::{Auth, AuthEvent, PermissionEvent, PermissionPrompt};
use crate::surface::{Dialog, DialogEvent, FindBar, FindEvent};

/// The overlays the store owns so far: the kit draws each from its phase, which only the host
/// writes (`settle_layers`). TRANSITIONAL: the others still belong to their machines.
const STORED: [Kind; 14] = [
    Kind::Downloads,
    Kind::Spaces,
    Kind::Popup,
    Kind::AutofillChip,
    Kind::AutofillMenu,
    Kind::AutofillPrompt,
    Kind::Reader,
    Kind::Status,
    Kind::Swipe,
    Kind::TabGhost,
    Kind::DropZone,
    Kind::PageGhost,
    Kind::DropMark,
    Kind::Autoscroll,
];

/// What the store reads of the host's other owners as an action runs (never copied into it).
struct HostEnv<'a> {
    host: &'a Host,
    window: &'a AppWindow,
    /// The tab whose menu and panels the current window shows (before a switch: the old one).
    menu_tab: Option<TabId>,
}

impl HostEnv<'_> {
    fn window(&self, w: WindowId) -> Option<&WindowHost> {
        std::iter::once(&self.host.w)
            .chain(self.host.parked.values())
            .find(|x| x.id == w)
    }
}

impl Env for HostEnv<'_> {
    fn shown(&self, w: WindowId) -> Option<TabId> {
        self.host.model.window_tab(w)
    }

    fn showing(&self, tab: TabId) -> Option<WindowId> {
        std::iter::once(&self.host.w)
            .chain(self.host.parked.values())
            .map(|x| x.id)
            .find(|&w| self.host.model.window_tab(w) == Some(tab))
    }

    fn kind(&self, w: WindowId) -> WindowKind {
        self.window(w).map_or(WindowKind::Main, |x| x.kind)
    }

    fn page_has_keys(&self, w: WindowId) -> bool {
        self.window(w).is_some_and(|x| x.handle.get_web_focused())
    }

    fn others(&self, w: WindowId) -> Vec<Layer> {
        if w != self.host.w.id {
            return vec![];
        }
        self.host.machine_layers(self.window, self.menu_tab)
    }

    fn machine(&self, kind: Kind) -> bool {
        !STORED.contains(&kind)
    }

    fn peeked(&self, w: WindowId) -> Option<TabId> {
        self.host.model.glance_of(w).map(|g| g.tab)
    }
}

/// What the kit still draws (`AppWindow::mounted`): one bit per surface that reports its exit,
/// the one table `surface_stack` and the store's exits (`settle_layers`) read (the peek's is a
/// fact of `page_covered`).
#[rustfmt::skip]
fn mounted(m: &Mounted) -> [(Kind, bool); 24] {
    [
        (Kind::Permission, m.permission), (Kind::Find, m.find), (Kind::Dialog, m.dialog),
        (Kind::BookmarkSheet, m.bookmark_sheet), (Kind::PrintSheet, m.print_sheet),
        (Kind::ExtensionSheet, m.extension_sheet), (Kind::AgentSheet, m.agent_sheet),
        (Kind::Region, m.region), (Kind::Downloads, m.downloads), (Kind::Extensions, m.extensions),
        (Kind::Spaces, m.spaces), (Kind::Menu, m.menu), (Kind::ErrorPanel, m.error),
        (Kind::Boost, m.boost), (Kind::Toast, m.toast), (Kind::Tooltip, m.tooltip),
        (Kind::Preview, m.preview), (Kind::AutofillMenu, m.autofill_menu),
        (Kind::AutofillPrompt, m.autofill_prompt), (Kind::VideoStrip, m.video_strip),
        (Kind::VideoTab, m.video_tab), (Kind::VideoSpeeds, m.video_speeds),
        (Kind::PipStrip, m.pip_strip), (Kind::Reader, m.reader),
    ]
}

impl Host {
    fn env<'a>(&'a self, window: &'a AppWindow, menu_tab: Option<TabId>) -> HostEnv<'a> {
        HostEnv {
            host: self,
            window,
            menu_tab,
        }
    }

    /// TRANSITIONAL (until each kind moves into the store): the overlays open in this window
    /// whose machine still owns their openness, each read from it (`menu_tab`: the tab whose
    /// menu and panels count).
    fn machine_layers(&self, window: &AppWindow, menu_tab: Option<TabId>) -> Vec<Layer> {
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
        if self.ext_ui_prompt_open() {
            add(Kind::ExtensionSheet, None);
        }
        if self.ext_ui_panel_open() {
            add(Kind::Extensions, None);
        }
        if menu_tab
            .and_then(|t| self.surfaces.get(&t))
            .is_some_and(|s| matches!(s.menu, Menu::Open { .. } | Menu::Shell { .. }))
        {
            add(Kind::Menu, None);
        }
        // the panels the host wrote for the visible tab
        if window.get_page_error().open {
            add(Kind::ErrorPanel, menu_tab);
        }
        if window.get_boost().open {
            add(Kind::Boost, menu_tab);
        }
        layers
    }

    /// Every surface on screen in this window now: the overlays from the store (with the
    /// machines' that still own theirs), the free surfaces from their one owner each (the view
    /// the host wrote, the driver's), and what the kit still draws as a free surface or a
    /// machine's overlay leaves (`AppWindow::mounted`). Derived each time, never kept.
    fn surface_stack(&self, window: &AppWindow) -> Stack {
        let shown = self.shown();
        let mut stack = self.ui.stack(&self.env(window, shown), self.w.id);
        let mut add = |kind, tab| stack.layers.push(Layer::new(kind, tab));
        // ----- the free surfaces, from what their owners wrote into this window
        let key = self.model.window_key_tab(self.w.id);
        let video = window.get_video();
        #[rustfmt::skip]
        let free = [
            (Kind::Toast, None, window.get_toast_shown()),
            (Kind::Tooltip, None, window.get_tip().shown),
            (Kind::Preview, None, window.get_preview().shown),
            (Kind::VideoStrip, key, video.w > 0.0 && video.open),
            (Kind::VideoTab, key, video.w > 0.0 && video.collapsed),
            (Kind::VideoSpeeds, key, video.w > 0.0 && video.open && video.speeds),
            (Kind::PipStrip, None, window.get_pip_strip()),
            (Kind::ExitPage, None, window.get_exit_t() < 1.0 && window.get_exit_page().size().width > 0),
            (Kind::Entrance, None, window.get_page_enter() < 1.0),
            (Kind::Peek, None, window.get_peeking()),
            (Kind::SeamBow, None, window.get_seam_bowed() || window.get_edge_bowed()),
            (Kind::SplitRing, None, !window.get_focus_ring().is_empty()),
            (Kind::SplitSeams, None, slint::Model::row_count(&window.get_seams()) > 0),
            (Kind::DevTools, Some(PANEL_TAB), self.panel_width() > 0.0),
            (Kind::Specimen, None, window.global::<AgentWidgets>().get_specimen()),
        ];
        for (kind, tab, on) in free {
            if on {
                add(kind, tab);
            }
        }
        // ----- what the kit still draws of the machines' overlays and the free surfaces: open,
        // or leaving (the store's own leave by their phase; the command bar's exit is the
        // driver's, seen in its state)
        let mut drawn = vec![];
        if window.get_command_bar_state() != CommandBarState::Closed
            && window.get_command_placement() == CommandPlacement::Overlay
        {
            drawn.push(Layer::new(Kind::CommandBar, None));
        }
        let at = |kind: Kind| match kind.row().scope {
            Scope::Window => None,
            Scope::Tab => shown,
            Scope::Pane => key,
        };
        drawn.extend(
            mounted(&window.get_mounted())
                .into_iter()
                .filter(|&(kind, on)| on && !STORED.contains(&kind))
                .map(|(kind, _)| Layer::new(kind, at(kind))),
        );
        stack.with_drawn(&drawn)
    }

    /// What the shell draws over the pages of this window now, for `overlay::covers` (the page
    /// layer's `Need::Covered`, `Needs::covered`) and `overlay::coverers` (its log). The peek and
    /// the picture-in-picture card are their owners' (the model's), so they come as facts.
    pub(super) fn page_covers(&self, window: &AppWindow) -> (Stack, Facts) {
        let facts = Facts {
            glass: window.global::<GlassLayer>().get_over_on(),
            glance: self.model.glance_of(self.w.id).is_some() || window.get_mounted().glance,
            glance_rect: self.glance_hit(window),
            pip_card: self.pip_card(),
        };
        (self.surface_stack(window), facts)
    }

    /// Runs `action` through the store, carries out its effects, writes the store's overlays
    /// into every window and ends the exits the kit no longer draws; returns the effects. The
    /// caller redraws the machines' surfaces.
    pub(super) fn act(&mut self, window: &AppWindow, action: Action) -> Vec<Effect> {
        let fx = self.step(window, action);
        self.settle_layers();
        fx
    }

    /// One reduce. The store is stored before any effect runs, so an owner that answers a
    /// `Cancel` by closing meets the new state (and its close is ignored).
    fn step(&mut self, window: &AppWindow, action: Action) -> Vec<Effect> {
        let menu_tab = match action {
            Action::Switched { from, .. } => from,
            _ => self.shown(),
        };
        let before = self.ui.slots();
        let ui = std::mem::take(&mut self.ui);
        let (ui, fx) = overlay::reduce(ui, &self.env(window, menu_tab), action);
        self.ui = ui;
        for &effect in &fx {
            match effect {
                Effect::Show(at, key) => self.draw_slot(at, Some(key), &before),
                Effect::Unshow(at) => self.draw_slot(at, None, &before),
                Effect::Arm(at) => self.arm_surfaces(at),
                Effect::Cancel(layer) => self.overlay_cancel(menu_tab, layer),
                Effect::Hide(Layer {
                    kind: Kind::Notice, ..
                }) => self.notice_shown(false),
                Effect::Reveal(Layer {
                    kind: Kind::Notice, ..
                }) => self.notice_shown(true),
                // the store's overlays are drawn from their phase (settle_layers), the find bar
                // and a dialog for the visible tab only (render_surfaces); the keys still follow
                // `settle-focus` as each *-open changes
                Effect::Mount(_)
                | Effect::Unmount(_)
                | Effect::Focus(..)
                | Effect::Hide(_)
                | Effect::Reveal(_) => {}
            }
        }
        fx
    }

    /// The store's one clock, in ms: every hover action and tick carries it.
    pub(super) fn surface_now(&self) -> u64 {
        self.ui_epoch.elapsed().as_millis() as u64
    }

    /// A hover or timed surface of this window: the kit's hover cue or its owner's show.
    pub(super) fn hover(&mut self, kind: Kind, key: Key, cue: Cue) {
        let at = Surface { w: self.w.id, kind };
        let now = self.surface_now();
        let window = self.w.handle();
        self.step(&window, Action::Hover { at, key, cue, now });
    }

    /// The hidden sidebar's peek: the shell's pointer cue as its slot's (chrome.tsx: the edge
    /// at once, a move in the strip after its delay, back on the panel stays, off it goes after
    /// its time).
    pub(crate) fn peek_cue(&mut self, cue: PeekCue) {
        let cue = match cue {
            PeekCue::EdgeEntered => Cue::Show,
            PeekCue::StripMoved => Cue::Enter,
            PeekCue::PanelEntered => Cue::Keep,
            PeekCue::PanelLeft => Cue::Leave,
        };
        self.hover(Kind::Peek, Key::None, cue);
    }

    /// The sidebar's mode changed: a peek never outlives the hidden mode.
    pub(crate) fn peek_mode(&mut self) {
        if self.w.handle().get_sidebar_mode() != SidebarMode::Hidden {
            self.hover(Kind::Peek, Key::None, Cue::Dismiss);
        }
    }

    /// The store's one timer fired.
    fn surface_tick(&mut self) {
        let now = self.surface_now();
        let window = self.w.handle();
        self.step(&window, Action::Tick { now });
    }

    /// `Effect::Arm`: the one timer set for `at` on the store's clock, or stopped. It runs in
    /// every mode (smokes too): the hover surfaces' times are the user's.
    fn arm_surfaces(&mut self, at: Option<u64>) {
        let Some(at) = at else {
            self.ui_timer.stop();
            return;
        };
        let wait = std::time::Duration::from_millis(at.saturating_sub(self.surface_now()));
        self.ui_timer.start(slint::TimerMode::SingleShot, wait, || {
            crate::measure::count(crate::measure::Count::SurfaceTick);
            crate::with_host(Host::surface_tick);
        });
    }

    /// A hover slot shows for `key` (or, `None`, hides): drawn into its own window, by its
    /// surface's one writer. `before`: the slots before the step (a tooltip glides from another
    /// one or a warm hide, else appears in place; the video strip that hid was this tab's).
    fn draw_slot(
        &mut self,
        at: Surface,
        key: Option<Key>,
        before: &std::collections::BTreeMap<Surface, wubble_core::hovercard::Hover<Key>>,
    ) {
        let here = self.w.id;
        if !self.enter(at.w) {
            return;
        }
        let window = self.w.handle();
        match at.kind {
            Kind::Tooltip => {
                let was = before.get(&at).copied().unwrap_or_default();
                let warm = wubble_core::hovercard::shown(was).is_some()
                    || matches!(was, wubble_core::hovercard::Hover::Cooling { .. });
                self.render_tip(key, !warm);
            }
            Kind::Preview => self.render_preview(match key {
                Some(Key::Tab(tab)) => Some(tab),
                _ => None,
            }),
            Kind::Status => self.render_panes(&window),
            Kind::Toast => window.set_toast_shown(key.is_some()),
            Kind::VideoStrip => self.video_slot(
                key,
                before
                    .get(&at)
                    .copied()
                    .and_then(wubble_core::hovercard::shown),
            ),
            Kind::PipStrip => window.set_pip_strip(key.is_some()),
            Kind::Peek => window.set_peek(key.is_some()),
            _ => {}
        }
        self.enter(here);
    }

    /// Writes the store's overlays into every window (only an open one shows; Slint ignores a
    /// value that did not change), then ends each exit the kit no longer draws. Every window:
    /// a tab's overlay acts in the window showing its tab, which may close that window's.
    fn settle_layers(&mut self) {
        let windows: Vec<(WindowId, AppWindow)> = std::iter::once(&self.w)
            .chain(self.parked.values())
            .map(|x| (x.id, x.handle.clone_strong()))
            .collect();
        for (w, window) in windows {
            let open = |kind| self.ui.phase(w, Layer::new(kind, None)) == Some(Phase::Open);
            window.set_downloads_open(open(Kind::Downloads));
            window.set_spaces_open(open(Kind::Spaces));
            // the drags', the swipe's and the autoscroll's surfaces: the kit shows each while open
            window.set_swipe_on(open(Kind::Swipe));
            window.set_drag_ghost_on(open(Kind::TabGhost));
            window.set_drop_zone_on(open(Kind::DropZone));
            window.set_page_ghost_on(open(Kind::PageGhost));
            window.set_drop_mark_on(open(Kind::DropMark));
            window.set_autoscroll_on(open(Kind::Autoscroll));
            let bits = mounted(&window.get_mounted());
            // a pane's bubble is drawn from its pane's report until the fade over (`status_drawn`);
            // the other pane layers have no exit
            let drawn = |l: Layer| match (l.kind.scope(), l.tab) {
                (Scope::Pane, Some(t)) if l.kind == Kind::Status => self.status_drawn.contains(&t),
                (Scope::Pane, _) => true,
                _ => bits.contains(&(l.kind, true)),
            };
            let gone = self.ui.gone(&self.env(&window, self.shown()), w, drawn);
            for layer in gone {
                self.step(&window, Action::Exited { w, layer });
            }
        }
    }

    /// A surface began or ended its exit (`mounted-changed`): the store's exits that ended end.
    pub(super) fn layers_mounted(&mut self) {
        self.settle_layers();
    }

    /// Asks to open `kind` (for `tab` when it is the tab's): whatever it replaces is closed;
    /// false when it may not open now (true when it is open already: its owner takes the ask).
    pub(super) fn overlay_open(&mut self, kind: Kind, tab: Option<TabId>) -> bool {
        let Some(window) = self.w.window.upgrade() else {
            return true;
        };
        let (w, layer) = (self.w.id, Layer::new(kind, tab));
        let env = self.env(&window, self.shown());
        let already =
            self.ui.phase(w, layer) == Some(Phase::Open) || env.others(w).contains(&layer);
        already
            || self
                .act(&window, Action::Open { w, layer })
                .contains(&Effect::Mount(layer))
    }

    /// The owner of a window's store overlay closed it (a pick, a dismissal, an empty list).
    pub(super) fn overlay_closed(&mut self, kind: Kind) {
        if let Some(window) = self.w.window.upgrade() {
            let (w, layer) = (self.w.id, Layer::new(kind, None));
            self.act(&window, Action::Close { w, layer });
        }
    }

    /// `kind` of `tab` is open or not, by what its owner (the engine's popup, a machine's view)
    /// says now: the store opens or closes its layer (a repeat of what it holds changes nothing).
    pub(super) fn layer_is(&mut self, kind: Kind, tab: TabId, open: bool) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let (w, layer) = (self.w.id, Layer::new(kind, Some(tab)));
        let action = if open {
            Action::Open { w, layer }
        } else {
            Action::Close { w, layer }
        };
        self.act(&window, action);
    }

    /// `kind`, a window surface its owner shows while some state holds (a drag, a swipe, an
    /// autoscroll), is open or not by what that state says now: only a change reaches the store.
    pub(super) fn surface_is(&mut self, kind: Kind, on: bool) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let (w, layer) = (self.w.id, Layer::new(kind, None));
        if (self.ui.phase(w, layer) == Some(Phase::Open)) != on {
            let action = if on {
                Action::Open { w, layer }
            } else {
                Action::Close { w, layer }
            };
            self.act(&window, action);
        }
    }

    /// The kit drew `kind` of `tab` at `rect` (window logical px); the same place again is
    /// nothing.
    pub(super) fn layer_placed(
        &mut self,
        kind: Kind,
        tab: TabId,
        rect: wubble_core::tabdrag::Rect,
    ) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let (w, layer) = (self.w.id, Layer::new(kind, Some(tab)));
        if self.ui.rect(w, layer) != Some(rect) {
            self.act(&window, Action::Placed { w, layer, rect });
        }
    }

    /// A pane's status bubble began or finished drawing: its fade over ends its exit.
    pub(crate) fn status_drawn(&mut self, pane: i32, drawn: bool) {
        let (Some(tab), Some(window)) = (crate::shell::tab_id(pane), self.w.window.upgrade())
        else {
            return;
        };
        if drawn {
            self.status_drawn.insert(tab);
        } else {
            self.status_drawn.remove(&tab);
            let (w, layer) = (self.w.id, Layer::new(Kind::Status, Some(tab)));
            self.act(&window, Action::Exited { w, layer });
        }
    }

    /// The window's visible tab's autofill part (`part` of [`autofill::PARTS`]) or reader
    /// controls were drawn at this place.
    pub(crate) fn shown_placed(&mut self, kind: Kind, x: f32, y: f32, width: f32, height: f32) {
        if let Some(tab) = self.shown() {
            let rect = wubble_core::tabdrag::Rect {
                x: f64::from(x),
                y: f64::from(y),
                w: f64::from(width),
                h: f64::from(height),
            };
            self.layer_placed(kind, tab, rect);
        }
    }

    /// `live` are the tabs drawn as panes now: a pane layer of any other tab (a bubble that was
    /// fading in a pane that went) goes, its element being gone cannot report.
    pub(super) fn panes_gone(&mut self, window: &AppWindow, live: &[TabId]) {
        let mut stale = self.ui.pane_tabs();
        stale.dedup();
        // a tab another window shows is that window's pane, not a stale one
        let me = self.w.id;
        stale.retain(|t| !live.contains(t) && self.model.window_of(*t).is_none_or(|w| w == me));
        for tab in stale {
            self.status_drawn.remove(&tab);
            let w = self.w.id;
            self.act(window, Action::PaneGone { w, tab });
        }
    }

    /// The sidebar's downloads button: the popover opens when the overlays let it, or closes.
    pub fn downloads_requested(&mut self) {
        let open = self.ui.phase(self.w.id, Layer::new(Kind::Downloads, None));
        if open == Some(Phase::Open) {
            self.overlay_closed(Kind::Downloads);
        } else if self.overlay_open(Kind::Downloads, None)
            && let Some(window) = self.w.window.upgrade()
        {
            self.render_surfaces(&window);
        }
    }

    /// The downloads popover was dismissed (a press outside).
    pub fn downloads_dismissed(&mut self) {
        self.overlay_closed(Kind::Downloads);
    }

    /// The Spaces popup was dismissed, or a pick (a Space, New Space) closed it.
    pub fn spaces_dismissed(&mut self) {
        self.overlay_closed(Kind::Spaces);
    }

    /// The footer's "^" (or the current Space's tile): the Spaces popup opens when the overlays
    /// let it (it closes the downloads popover or a menu; a page's dialog refuses it).
    pub fn spaces_menu_requested(&mut self) {
        self.hover_dismiss();
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if self.overlay_open(Kind::Spaces, None) {
            self.render_surfaces(&window);
        }
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
        let w = self.w.id;
        if self
            .ui
            .escape_target(&self.env(window, self.shown()), w)
            .is_none()
        {
            return false;
        }
        self.act(window, Action::Escape { w });
        self.render_surfaces(window);
        true
    }

    /// A rule closed `layer`: its owner answers as cancelled (`shown`: the tab whose menu it
    /// is). The store's own overlays have no one to answer.
    fn overlay_cancel(&mut self, shown: Option<TabId>, layer: Layer) {
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
            (Kind::Downloads | Kind::Spaces, _) => {}
            (Kind::Extensions, _) => self.extensions_dismissed(),
            (Kind::ExtensionSheet, _) => self.extension_prompt_answered(false),
            (Kind::Menu, _) => {
                if let Some(tab) = shown {
                    self.apply_menu(tab, MenuEvent::Dismiss);
                }
            }
            (Kind::Find | Kind::Dialog | Kind::Permission, None) => {}
            // the panels close by their machines: no rule closes them (`Row::closes`, `escape`)
            (Kind::ErrorPanel | Kind::Boost, _) => {}
            // a drag ends with nothing dropped (its own state closes its surfaces), a swipe and an
            // autoscroll end as their machines end them
            (Kind::TabGhost, _) => {
                self.cancel_tab_drag();
            }
            (Kind::PageGhost, _) => {
                self.drag_cancelled();
            }
            (Kind::Swipe, _) => {
                let window = self.w.handle();
                self.swipe_input(
                    &window,
                    wubble_core::gesture::SwipeInput::End { cancelled: true },
                );
            }
            (Kind::Autoscroll, _) => {
                let window = self.w.handle();
                self.autoscroll_input(&window, wubble_core::autoscroll::Input::Pressed);
            }
            // the free surfaces: no rule of the overlays closes one (`overlay::Group::Free`)
            (
                Kind::Popup
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
                | Kind::Glance
                | Kind::ExitPage
                | Kind::Entrance
                | Kind::Peek
                | Kind::SeamBow
                | Kind::SplitRing
                | Kind::SplitSeams
                | Kind::DevTools
                | Kind::DropZone
                | Kind::DropMark
                | Kind::Specimen,
                _,
            ) => {}
        }
    }
}
