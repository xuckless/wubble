//! The host's half of the page's own UI: page events in, through the pure
//! machines (`surface.rs`, `menu.rs`, `screen.rs`, `ime.rs`, `prompt.rs`,
//! `page_error.rs`), effects out to the engine and the shell. The visible tab's surfaces are drawn by
//! [`Host::render_surfaces`], the only writer of those Slint properties.

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::dpi::{LogicalPosition, LogicalSize};
use slint::winit_030::winit::event::Ime as WinitIme;
use std::path::PathBuf;

use wubble_core::action::Action;
use wubble_core::busy::Reason;
use wubble_core::clipboard::{self, Ask, Asker, Payload, Reader};
use wubble_core::devtools::{DockEvent, PANEL_TAB};
use wubble_core::dnd;
use wubble_core::model::Command;
use wubble_core::overlay::{self, Cue, Key, Kind, Layer, Phase};
use wubble_core::site::{Decision, Permission, SiteChange};
use wubble_core::sitesound::{self, Mute, MuteEvent, Toggle};
use wubble_core::space::Side;
use wubble_core::tab::TabId;
use wubble_core::window::{WindowId, WindowTarget};
use wubble_core::zoom;
use wubble_engine::app::ViewId;
use wubble_engine::page::{ClipboardRequest, EditCommand, PageEvent, PasteFrom, Rect, RequestId};
use wubble_ui::{
    AppWindow, DialogView, ErrorView, FindView, MenuEntry, MenuView, PermissionView, PopupView,
};

use super::layers::Content;
use super::{Host, new_tab};
use crate::chooser::{self, DialogSpec};
use crate::clipboard::{Bridge as _, Buffer};
use crate::ime::{self, ImeCall, ImeInput};
use crate::menu::{self, Command as MenuCommand, Menu, MenuEffect, MenuEvent, NavFlags};
use crate::page_error::{self, ErrorEffect, ErrorEvent, PageError};
use crate::prompt::{self, AuthEvent, PermissionEvent, PermissionPrompt, Reply, Secret};
use crate::screen::{self, Screen, ScreenEffect, ScreenEvent};
use crate::surface::{
    self, DialogContent, DialogEvent, FindBar, FindEffect, FindEvent, OpenDialog, Routed,
};

/// What the shell last drew: the tab, and its menu and prompt (a render cache, not state).
/// Slint skips setting a property to an equal value, but a menu's rows are a fresh `ModelRc`
/// (equal only by pointer), and a rewritten model restarts the rows' motion.
#[derive(Default)]
pub(super) struct Drawn {
    tab: Option<TabId>,
    menu: Option<Menu>,
    permission: Option<PermissionPrompt>,
}

/// Chromium's ERR_BLOCKED_BY_CLIENT: a request the content filter cancelled.
const BLOCKED_BY_CLIENT: i32 = -20;

impl Host {
    pub(super) fn on_page_event(&mut self, window: &AppWindow, view: ViewId, event: PageEvent) {
        // Events of a view whose tab is already gone: welding answered its
        // requests when the view closed.
        let Some(tab) = self.tab_of(view) else {
            return;
        };
        if tab == PANEL_TAB {
            return self.on_panel_page_event(event);
        }
        if tab == wubble_core::extensions::shell::POPUP_TAB {
            return self.on_popup_page_event(event);
        }
        // Pages for screen readers (`host/ax.rs`).
        if let PageEvent::AxTree(json) | PageEvent::AxLocation(json) = &event {
            let location = matches!(event, PageEvent::AxLocation(_));
            return self.ax_event(tab, json, location);
        }
        // A file chooser DevTools caught for an agent's upload (`host/agent.rs`).
        if let PageEvent::ChooserIntercepted(opened) = &event {
            return self.agent_intercepted(tab, opened);
        }
        // The built-ins' answers (`host/builtins.rs`, `host/videos.rs`).
        if matches!(
            event,
            PageEvent::CdpReply { .. } | PageEvent::EvalResult { .. }
        ) {
            self.video_smoke_page(&event);
            if self.pinch_smoke_answer(&event) {
                return;
            }
            if self.agent_answer(&event)
                || self.ax_answer(&event)
                || self.media_answer(tab, &event)
                || self.site_search_answer(tab, &event)
            {
                return;
            }
            self.builtins_answer(tab, &event);
            self.builtin_smoke_step(crate::builtin_smoke::Input::Page(&event));
            return;
        }
        if matches!(event, PageEvent::DownloadFinished { .. }) {
            self.builtin_smoke_step(crate::builtin_smoke::Input::Page(&event));
        }
        let seen = self.checks.page_smoke.is_some().then(|| event.clone());
        let visible = self.shown() == Some(tab);
        let routed = surface::route(event);
        // A page beside the focused one that asks something takes the focus, so its question
        // shows instead of blocking it unseen.
        let driven = self.agent_unseen(tab);
        if !driven
            && !visible
            && self.model.window_tabs(self.w.id).contains(&tab)
            && matches!(
                routed,
                Routed::Dialog(DialogEvent::Requested(_)) | Routed::Auth(AuthEvent::Requested(_))
            )
        {
            self.dispatch(Command::ActivateTab(tab));
        }
        // (a page's question opens its overlay, which closes the user's over its tab: `apply_dialog`)
        match routed {
            Routed::Dialog(e) => {
                self.apply_dialog(tab, e);
            }
            Routed::Menu(MenuEvent::Requested { id, menu, .. }) => {
                let nav = self
                    .tab_view(tab)
                    .map_or(NavFlags::default(), |v| NavFlags {
                        back: v.nav().page.can_go_back,
                        forward: v.nav().page.can_go_forward,
                        ..NavFlags::default()
                    });
                let nav = NavFlags {
                    zoomed: self.zoomed(tab),
                    search: self.search.default.as_ref().map(|e| e.name.clone()),
                    image: self.search.default.as_ref().and_then(|e| e.image.clone()),
                    private: self.model.side_of_tab(tab) == Side::Private,
                    ..nav
                };
                let extra = self.model.windowed(tab).map_or_else(Vec::new, |w| {
                    crate::menu::window_page_entries(&self.model, w)
                });
                // refused (a page's dialog waits): the request is answered at once
                self.apply_menu(
                    tab,
                    MenuEvent::Requested {
                        id,
                        menu,
                        nav,
                        extra,
                    },
                );
            }
            Routed::Menu(e) => {
                self.apply_menu(tab, e);
            }
            Routed::Find(e) => self.apply_find(tab, e),
            Routed::Drag { data, allowed } => self.drag_started(tab, data, allowed),
            Routed::DragCursor(op) => self.drag_cursor(tab, op),
            // the engine's rect stays as the input's origin; whether the list is up is the store's
            Routed::Popup(rect) => {
                self.surfaces.entry(tab).or_default().popup = rect;
                self.layer_is(Kind::Popup, tab, rect.is_some());
            }
            // Picture-in-picture's fullscreen is the video's, in its own window.
            Routed::Fullscreen(true) if self.pip_tab() == Some(tab) => {}
            Routed::Fullscreen(true) if visible => {
                self.apply_screen(window, ScreenEvent::PageEntered(tab));
            }
            // A hidden tab may not take the screen.
            Routed::Fullscreen(true) => {
                if let Some(v) = self.tab_view(tab) {
                    v.exit_fullscreen();
                }
            }
            Routed::Fullscreen(false) => {
                self.pip_fullscreen_left(tab);
                self.page_left(tab);
            }
            Routed::Muted(muted) => {
                let audio = self.audio.entry(tab).or_default();
                audio.muted = muted;
                // muted or unmuted from elsewhere (CEF's own sound content setting, a page
                // restored muted): the reason follows what the view says
                if muted != audio.why.muted() {
                    audio.why = if muted { Mute::ByUser } else { Mute::Unmuted };
                }
                self.after_change();
            }
            Routed::Audible(audible) => {
                self.media_audible(audible);
                self.set_busy(tab, Reason::Audible, audible);
            }
            Routed::Frozen { frozen, ok } => self.frozen_answered(tab, frozen, ok),
            Routed::TargetId(Ok(id)) => self.dock_event(DockEvent::TargetId(tab, id)),
            Routed::TargetId(Err(())) => self.dock_event(DockEvent::TargetFailed(tab)),
            Routed::Committed => {
                // the new document does not glide on with the last one's scroll
                if self.gliding_on() == Some(tab) {
                    self.fling(wubble_core::fling::FlingInput::Interrupt);
                }
                let w = self.w.id;
                self.act(overlay::Action::Navigated { w, tab });
                self.apply_permission(tab, PermissionEvent::PageChanged);
                self.busy_page_started(tab);
            }
            Routed::ImeBounds(rect) if self.key_page() == Some(tab) => {
                place_candidates(window, self.view_origin(window, tab), rect)
            }
            Routed::Dismissed(ids) => {
                self.apply_dialog(tab, DialogEvent::Dismissed(ids.clone()));
                self.apply_permission(tab, PermissionEvent::Dismissed(ids.clone()));
                self.apply_auth(tab, AuthEvent::Dismissed(ids));
            }
            Routed::Permission(e) => self.apply_permission(tab, e),
            Routed::Auth(e) => self.apply_auth(tab, e),
            Routed::Chooser { id, spec } => self.run_chooser(window, tab, id, spec),
            Routed::Cert { id, url, code } => self.certificate(tab, id, url, code),
            Routed::Zoomed => window.set_address_zoom(self.address_zoom()),
            // a failed upgrade is HTTPS-Upgrades' own: it loads http or asks
            Routed::LoadFailed { url, code } => {
                if !self.https_failed(tab, &url) {
                    let shown = self.tab_view(tab).is_some_and(|v| v.is_visible());
                    // a page the blocker stopped says so (ERR_BLOCKED_BY_CLIENT)
                    let browser = self.tab_view(tab).map(|v| v.browser_id());
                    let blocked = browser
                        .filter(|_| code == BLOCKED_BY_CLIENT)
                        .and_then(|b| self.blocking.document_block(b, &url));
                    let event = match blocked {
                        Some(dangerous) => ErrorEvent::DocumentBlocked { url, dangerous },
                        None => ErrorEvent::LoadFailed {
                            url,
                            code,
                            now_ms: wall_ms(),
                            shown,
                        },
                    };
                    self.apply_error(tab, event);
                }
            }
            Routed::Download(e) => self.download_event(window, tab, e),
            Routed::Console(line) => {
                if std::env::var_os("WUBBLE_PARITY_TRACE").is_some() {
                    eprintln!("console: {line}");
                }
                log::debug!(target: "wubble::console", "{line}")
            }
            Routed::Favicon(icon) => self.show_favicon(tab, icon),
            Routed::Frame { frame, text } => {
                self.tasks.frame_seen(tab, &frame);
                self.autofill_message(tab, frame, &text);
            }
            Routed::Probe { frame, report } => {
                self.tasks.frame_seen(tab, &frame);
                if let Some(keys) = report.strip_prefix(super::busy::PROBE_PREFIX) {
                    self.probe_report(tab, &frame, keys);
                }
            }
            Routed::Pdf { path, ok, .. } if self.print_rendered(&path, ok) => {}
            Routed::Pdf { path, text, .. } => {
                self.downloads.release(&path);
                self.toast(window, &text);
            }
            Routed::Clipboard { frame, request } => {
                self.clipboard_request(window, tab, frame, request)
            }
            // The page under the wheel is the visible one's; only that one zooms, as
            // the zoom keys do.
            Routed::WheelZoom { zoom_in } if self.model.visible_tab() == Some(tab) => {
                self.perform(if zoom_in {
                    Action::ZoomIn
                } else {
                    Action::ZoomOut
                })
            }
            Routed::Status(text) => self.link_status(tab, text),
            Routed::Tooltip(text) => self.page_tooltip(tab, text),
            Routed::External { url, gesture } => self.external_link(tab, url, gesture),
            Routed::Refused(url) => self.agent_refused(tab, &url),
            Routed::Image { url, image } => self.copy_image_done(window, &url, image),
            Routed::WheelZoom { .. } | Routed::ImeBounds(_) | Routed::Ignore => {}
        }
        self.render_surfaces(window);
        // After the event took effect, so the smokes act on the open surface.
        self.parity_menu(tab);
        if let Some(event) = seen {
            let view = self.pane_layout(window, tab).size;
            self.page_smoke_step(crate::page_smoke::Input::Page(&event, view));
        }
    }

    /// Runs one dialog event; whether an answer reached the engine.
    pub(super) fn apply_dialog(&mut self, tab: TabId, event: DialogEvent) -> Option<bool> {
        if self.agent_unseen(tab) {
            if let DialogEvent::Requested(req) = event {
                let accept = req.kind.default_accept();
                if let Some((session, _)) = self.agent_grant_of(tab) {
                    self.agent_log(
                        &session,
                        "dialog_dismissed",
                        Some(req.origin),
                        wubble_core::agent::log::Outcome::Refused,
                    );
                }
                return self
                    .tab_view(tab)
                    .map(|view| view.answer_dialog(req.id, accept, ""));
            }
            return None;
        }
        self.dialog_step(tab, event, false)
    }

    /// The tab's dialog overlay's content (its JS dialog and sign-in sheet): while it is open,
    /// or (`cancel`) while a rule closing it has it answer.
    fn dialog_content(&self, tab: TabId, cancel: bool) -> DialogContent {
        let layer = Layer::new(Kind::Dialog, Some(tab));
        match self.live(self.w.id, layer, cancel) {
            Some(Content::Dialog(c)) => c,
            _ => DialogContent::default(),
        }
    }

    /// The dialog overlay holds `next` now: opened, rewritten or closed by it. A page's question
    /// is refused by no overlay (`Row::refused_under` is empty, every window kind shows it).
    fn dialog_after(&mut self, tab: TabId, next: DialogContent) {
        let layer = Layer::new(Kind::Dialog, Some(tab));
        let refused = self.content_after(self.w.id, layer, next.live().map(Content::Dialog), false);
        debug_assert!(refused.is_none(), "a page's dialog is never refused");
    }

    /// One JS dialog event on the tab's content; whether an answer reached the engine.
    fn dialog_step(&mut self, tab: TabId, event: DialogEvent, cancel: bool) -> Option<bool> {
        let c = self.dialog_content(tab, cancel);
        let (js, answer) = surface::dialog(c.js, event);
        self.dialog_after(tab, DialogContent { js, auth: c.auth });
        let answer = answer?;
        let view = self.tab_view(tab)?;
        Some(view.answer_dialog(answer.id, answer.accept, &answer.text))
    }

    /// A rule closed the tab's dialog overlay (Escape): the sign-in sheet over it is cancelled
    /// first, else the dialog; a dialog left behind the sheet opens it again.
    pub(super) fn dialog_cancelled(&mut self, tab: TabId) {
        if self.dialog_content(tab, true).auth.is_some() {
            self.auth_step(tab, AuthEvent::Cancel, true);
        } else {
            self.dialog_step(tab, DialogEvent::Cancel, true);
        }
    }

    /// Runs one permission event: remembered decisions are stored, every
    /// answer goes to the engine.
    pub(super) fn apply_permission(&mut self, tab: TabId, event: PermissionEvent) {
        self.permission_step(tab, event, false);
    }

    /// A rule closed the tab's prompt (Escape): the shown request is denied, not remembered,
    /// and a waiting one opens it again.
    pub(super) fn permission_cancelled(&mut self, tab: TabId) {
        let deny = PermissionEvent::Answer {
            allow: false,
            remember: false,
        };
        self.permission_step(tab, deny, true);
    }

    /// One permission event on the tab's prompt (`cancel`: on the content a rule just closed).
    fn permission_step(&mut self, tab: TabId, event: PermissionEvent, cancel: bool) {
        if self.agent_drives(tab) {
            if let PermissionEvent::Requested(req) = event
                && let crate::prompt::Ask::Engine { id, .. } = req.ask
                && let Some(view) = self.tab_view(tab)
            {
                view.answer_permission(id, &[]);
            }
            return;
        }
        // a private window's tab reads its own choices over the stored ones, and remembers
        // only in memory (`site_view`, `write_site`)
        let table = self.site_view(tab).permissions().into_owned();
        let layer = Layer::new(Kind::Permission, Some(tab));
        let live = match self.live(self.w.id, layer, cancel) {
            Some(Content::Permission(p)) => Some(p),
            _ => None,
        };
        let (next, answers) = prompt::permission(live, event, &table);
        // a prompt is refused by no overlay (`Row::refused_under` is empty)
        self.content_after(self.w.id, layer, next.map(Content::Permission), false);
        for answer in answers {
            if let Some((site, kinds, decision)) = &answer.remember {
                for &kind in kinds {
                    self.write_site(tab, SiteChange::Permission(site.clone(), kind, *decision));
                }
            }
            match answer.reply {
                Reply::External { url, open } => {
                    if open {
                        self.open_in_app(&url);
                    }
                }
                Reply::Engine { id, granted } => {
                    if let Some(view) = self.tab_view(tab) {
                        view.answer_permission(id, &granted);
                    }
                }
                Reply::Download {
                    id,
                    download,
                    name,
                    allow,
                } => self.download_answered(tab, id, download, &name, allow),
            }
        }
    }

    pub(super) fn apply_auth(&mut self, tab: TabId, event: AuthEvent) {
        self.auth_step(tab, event, false);
    }

    /// One sign-in event on the tab's dialog overlay (`cancel`: on the content a rule closed).
    fn auth_step(&mut self, tab: TabId, event: AuthEvent, cancel: bool) {
        if self.agent_unseen(tab) {
            if let AuthEvent::Requested(req) = event {
                if let Some((session, _)) = self.agent_grant_of(tab) {
                    self.agent_log(
                        &session,
                        "auth_refused",
                        Some(req.host),
                        wubble_core::agent::log::Outcome::Refused,
                    );
                }
                if let Some(view) = self.tab_view(tab) {
                    view.answer_auth(req.id, None);
                }
            }
            return;
        }
        let c = self.dialog_content(tab, cancel);
        let (auth, answer) = prompt::auth(c.auth, event);
        self.dialog_after(tab, DialogContent { js: c.js, auth });
        if let Some(answer) = answer
            && let Some(view) = self.tab_view(tab)
        {
            let credentials = answer
                .credentials
                .as_ref()
                .map(|(user, password)| (user.as_str(), password.expose()));
            view.answer_auth(answer.id, credentials);
        }
    }

    /// A certificate error: proceeds at once past one the user allowed this
    /// session; otherwise the panel holds the request for the user.
    fn certificate(&mut self, tab: TabId, id: RequestId, url: String, code: i32) {
        if self.agent_unseen(tab) {
            if let Some((session, _)) = self.agent_grant_of(tab) {
                self.agent_log(
                    &session,
                    "certificate_refused",
                    Some(url),
                    wubble_core::agent::log::Outcome::Refused,
                );
            }
            if let Some(view) = self.tab_view(tab) {
                view.answer_certificate(id, false);
            }
            return;
        }
        // an upgraded navigation's certificate error is a failed upgrade: refused, and http
        // loads or the HTTPS-Only panel asks
        let browser = self.tab_view(tab).map_or(0, |v| v.browser_id());
        if self.blocking.https.owns(browser, &url) {
            if let Some(view) = self.tab_view(tab) {
                view.answer_certificate(id, false);
            }
            self.https_failed(tab, &url);
            return;
        }
        if zoom::host_of(&url).is_some_and(|h| self.cert_exceptions.allowed(&h, code)) {
            if let Some(view) = self.tab_view(tab) {
                view.answer_certificate(id, true);
            }
        } else {
            let event = ErrorEvent::CertError {
                url,
                code,
                request: id,
            };
            self.apply_error(tab, event);
        }
    }

    pub(super) fn apply_error(&mut self, tab: TabId, event: ErrorEvent) {
        let s = self.surfaces.entry(tab).or_default();
        let (next, effects) = page_error::transition(std::mem::take(&mut s.error), event);
        let open = next != PageError::None;
        s.error = next;
        // the panel is the tab's store overlay while its machine holds an error
        self.layer_is(Kind::ErrorPanel, tab, open);
        for effect in effects {
            // the view's own effects need the view; the rest are the host's
            let view = self.tab_view(tab);
            match effect {
                ErrorEffect::Allow { host, code } => self.cert_exceptions.allow(&host, code),
                ErrorEffect::ContinueHttp => self.https_continue(tab),
                ErrorEffect::Unblock { host } => self.blocking.allow_pages_for_session(&host),
                ErrorEffect::Reload { .. } => view.into_iter().for_each(|v| v.reload()),
                ErrorEffect::Back => view.into_iter().for_each(|v| v.back()),
                ErrorEffect::Blank => view.into_iter().for_each(|v| v.load("about:blank")),
                ErrorEffect::AnswerCert { request, proceed } => view.into_iter().for_each(|v| {
                    v.answer_certificate(request, proceed);
                }),
            }
        }
    }

    /// `event` for `tab`'s error panel, when it has one: the schedule's and the page's news
    /// that only a panel cares about (its tab shown or hidden, a stop, a load's end).
    pub(super) fn error_news(&mut self, tab: TabId, event: ErrorEvent) {
        let Some(before) = self
            .surfaces
            .get(&tab)
            .map(|s| s.error.clone())
            .filter(|e| *e != PageError::None)
        else {
            return;
        };
        self.apply_error(tab, event);
        // a tick that changes nothing draws nothing
        if self.surfaces.get(&tab).map(|s| &s.error) != Some(&before) {
            self.redraw_surfaces();
        }
    }

    /// `event` for every tab's error panel: the tick, the network back.
    pub(super) fn errors_news(&mut self, event: ErrorEvent) {
        let tabs: Vec<TabId> = self.surfaces.keys().copied().collect();
        for tab in tabs {
            self.error_news(tab, event.clone());
        }
    }

    /// The system came back online (`os_online`): a failed page reloads at once.
    pub fn network_online(&mut self) {
        self.errors_news(ErrorEvent::Online);
    }

    /// Shows the OS file dialog; its answer comes back on the event loop.
    fn run_chooser(&mut self, window: &AppWindow, tab: TabId, id: RequestId, spec: DialogSpec) {
        if self.agent_chooser(tab, id) {
            return;
        }
        let dialog = chooser::dialog(&spec, Some(window.window()));
        let task = slint::spawn_local(async move {
            let picked = chooser::pick(spec, dialog).await;
            crate::with_host(|host| host.chooser_answered(tab, id, picked));
        });
        if let Err(e) = task {
            eprintln!("wubble: cannot show the file dialog: {e}");
            self.chooser_answered(tab, id, None);
        }
    }

    fn chooser_answered(&mut self, tab: TabId, id: RequestId, picked: Option<Vec<PathBuf>>) {
        if let Some(view) = self.tab_view(tab) {
            view.answer_file_chooser(id, picked.as_deref());
        }
    }

    /// Runs one menu event from `tab`'s page (or the user's, on this window's menu); how many
    /// answers reached the engine.
    pub(super) fn apply_menu(&mut self, tab: TabId, event: MenuEvent) -> usize {
        self.menu_step(tab, event, false)
    }

    /// The shell's own menu over `tab`'s page (a tab row's, the pill's, a question): the one way
    /// a shell menu opens. `ask`: the app's own question, which no overlay refuses (the private
    /// downloads a close would cancel, over a page's dialog). Whether it shows.
    pub(super) fn open_shell_menu(
        &mut self,
        tab: TabId,
        at: (i32, i32),
        entries: Vec<menu::Entry>,
        ask: bool,
    ) -> bool {
        self.menu_step(tab, MenuEvent::ShellRequested { at, entries }, ask);
        let shown = self.ui.phase(self.w.id, Layer::new(Kind::Menu, None)) == Some(Phase::Open);
        if let Some(window) = self.w.window.upgrade() {
            self.render_surfaces(&window);
        }
        shown
    }

    /// A rule closed window `w`'s menu (Escape, a switch, another overlay, its window gone): a
    /// page's menu is dismissed to its page; a shell's runs nothing.
    pub(super) fn menu_cancelled(&mut self, w: WindowId) {
        if let Some(Content::Menu { tab, menu }) = self.live(w, Layer::new(Kind::Menu, None), true)
        {
            let (_, effects) = menu::transition(Some(menu), MenuEvent::Dismiss);
            self.menu_effects(tab, effects);
        }
    }

    /// One menu event on this window's menu (`ask`: an open is the app's question). A menu of
    /// another tab's page that a new one replaces is dismissed to that page; CEF closing another
    /// tab's menu is not this one's.
    fn menu_step(&mut self, tab: TabId, event: MenuEvent, ask: bool) -> usize {
        let (w, layer) = (self.w.id, Layer::new(Kind::Menu, None));
        let held = match self.live(w, layer, false) {
            Some(Content::Menu { tab, menu }) => Some((tab, menu)),
            _ => None,
        };
        let fresh = matches!(
            event,
            MenuEvent::Requested { .. } | MenuEvent::ShellRequested { .. }
        );
        let mut answered = 0;
        let (tab, live) = match held {
            Some((t, menu)) if t != tab && fresh => {
                let (_, effects) = menu::transition(Some(menu), MenuEvent::Dismiss);
                answered += self.menu_effects(t, effects);
                (tab, None)
            }
            Some((t, _)) if t != tab && event == MenuEvent::Reset => return 0,
            Some((t, menu)) => (t, Some(menu)),
            None => (tab, None),
        };
        let (next, effects) = menu::transition(live, event);
        let next = next.map(|menu| Content::Menu { tab, menu });
        let refused = self.content_after(w, layer, next, ask);
        answered += self.menu_effects(tab, effects);
        // refused (a page's dialog holds the window): the page's request is answered at once
        if let Some(Content::Menu { tab, menu }) = refused {
            let (_, effects) = menu::transition(Some(menu), MenuEvent::Dismiss);
            answered += self.menu_effects(tab, effects);
        }
        answered
    }

    /// Carries out a menu's effects for `tab`'s page; how many answers reached the engine.
    fn menu_effects(&mut self, tab: TabId, effects: Vec<MenuEffect>) -> usize {
        let mut answered = 0;
        for effect in effects {
            match effect {
                MenuEffect::Answer { id, command } => {
                    if let Some(v) = self.tab_view(tab)
                        && v.answer_context_menu(id, command)
                    {
                        answered += 1;
                    }
                }
                MenuEffect::Run(MenuCommand::OpenInNewTab(url)) => {
                    self.open_routed(url, Some(tab));
                }
                MenuEffect::Run(MenuCommand::OpenInNewWindow(url)) => {
                    let before: Vec<TabId> = self.model.tabs.keys().copied().collect();
                    self.open_routed(url, Some(tab));
                    // Only a tab the link made is torn out: a link another app takes opens none,
                    // and one routed to another Space shows that Space's tab if none was made.
                    if let Some(new) = self.model.visible_tab().filter(|t| !before.contains(t)) {
                        self.dispatch(Command::MoveTabToWindow {
                            tab: new,
                            to: WindowTarget::New,
                        });
                    }
                }
                MenuEffect::Run(MenuCommand::FrameScript { frame, script }) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.run_script_in(&frame, script);
                    }
                }
                MenuEffect::Run(MenuCommand::OpenInPrivateWindow(url)) => {
                    self.dispatch(Command::NewPrivateWindow { url: Some(url) });
                }
                MenuEffect::Run(MenuCommand::PictureInPicture { x, y }) => {
                    self.pip_event(wubble_core::pip::PipInput::RequestAt(tab, (x, y)));
                }
                MenuEffect::Run(MenuCommand::VideoFrame { x, y, output }) => {
                    self.video_frame_at(tab, (x, y), output);
                }
                MenuEffect::Run(MenuCommand::OpenInGlance(url)) => {
                    let window = self.w.id;
                    self.dispatch(Command::OpenGlance {
                        window,
                        opener: tab,
                        url,
                    });
                }
                // the field keeps its focus: the viewer types into it
                MenuEffect::Run(MenuCommand::EmojiPalette) => {
                    #[cfg(target_os = "macos")]
                    if let Some(mtm) = objc2::MainThreadMarker::new() {
                        // SAFETY: on the main thread (`mtm`), with no sender, as AppKit's
                        // Edit menu sends it
                        #[allow(unused_unsafe)]
                        unsafe {
                            objc2_app_kit::NSApplication::sharedApplication(mtm)
                                .orderFrontCharacterPalette(None);
                        }
                    }
                }
                MenuEffect::Run(MenuCommand::CopyText(text)) => self.copy(&text),
                MenuEffect::Run(MenuCommand::SaveAs(url)) => self.save_as(tab, url),
                MenuEffect::Run(MenuCommand::Search(text)) => self.search_for(tab, text),
                MenuEffect::Run(MenuCommand::Inspect { x, y }) => self.inspect_at(tab, (x, y)),
                MenuEffect::Run(MenuCommand::Media {
                    op,
                    x,
                    y,
                    frame,
                    source,
                }) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.run_script_in(&frame, &menu::media_script(op, x, y, &source));
                    }
                }
                MenuEffect::Run(MenuCommand::CopyImage(url)) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.download_image(&url);
                    }
                }
                MenuEffect::Run(MenuCommand::PastePlain(frame)) => self.paste_plain(tab, frame),
                MenuEffect::Run(MenuCommand::SplitWith(t)) => self.dispatch(Command::SplitWith(t)),
                MenuEffect::Run(MenuCommand::Unsplit(t)) => self.dispatch(Command::Unsplit(t)),
                MenuEffect::Run(MenuCommand::OpenBeside) => self.open_beside(),
                MenuEffect::Run(MenuCommand::Perform(action)) => self.perform(action),
                MenuEffect::Run(MenuCommand::Model(command)) => self.dispatch(command.0),
                MenuEffect::Run(MenuCommand::ToggleMute(t)) => self.toggle_mute(t),
                MenuEffect::Run(MenuCommand::ToggleMuteSite(t)) => self.toggle_mute_site(t),
                MenuEffect::Run(MenuCommand::FieldEdit(edit)) => self.field_edit(edit),
                MenuEffect::Run(MenuCommand::ZoomStep(action)) => {
                    self.perform(action);
                    // the popover again, on the new percent, where the chip opened it
                    if let Some((x, y)) = self.zoom_at {
                        self.zoom_popover(x, y);
                    }
                }
                MenuEffect::Run(MenuCommand::PasteAndGo(field, text)) => {
                    self.paste_and_go(field, &text);
                }
                MenuEffect::Run(MenuCommand::SetFullUrls(on)) => {
                    let key = wubble_core::settings::UI_FULL_URLS.key;
                    self.write_setting(key, if on { "1" } else { "0" });
                }
                MenuEffect::Run(MenuCommand::ReloadTab(t)) => {
                    if let Some(view) = self.tab_view(t) {
                        view.reload();
                    }
                }
                MenuEffect::Run(MenuCommand::RenameFolder(f)) => self.rename_folder_requested(f),
                MenuEffect::Run(MenuCommand::ToggleBlocker(host)) => {
                    self.write_site(tab, SiteChange::Blocker(host));
                }
                MenuEffect::Run(MenuCommand::SpaceNeverSleep(space, on)) => {
                    self.dispatch(Command::SetSpaceNeverSleep { space, on });
                }
                MenuEffect::Run(MenuCommand::ToggleNeverSleepSite(site)) => {
                    self.write_site(tab, SiteChange::NeverSleep(site));
                }
                MenuEffect::Run(MenuCommand::MoveToWindow(t, to)) => {
                    self.dispatch(Command::MoveTabToWindow { tab: t, to });
                }
                MenuEffect::Run(MenuCommand::OpenBlockedPopups) => self.open_held_popups(tab),
                MenuEffect::Run(MenuCommand::PromoteLittle(window, space)) => {
                    self.dispatch(Command::PromoteLittle { window, space });
                }
                MenuEffect::Run(MenuCommand::AllowPopups(site)) => {
                    // a private window's choice stays in memory, with its private windows
                    self.write_site(
                        tab,
                        SiteChange::Permission(site, Permission::Popups, Decision::Allow),
                    );
                    self.open_held_popups(tab);
                }
                MenuEffect::Run(MenuCommand::SaveMedia { item, choice }) => {
                    self.save_media(tab, item, choice)
                }
                MenuEffect::Run(MenuCommand::SaveSubtitle { item, index }) => {
                    self.save_subtitle(tab, item, index)
                }
                MenuEffect::Run(
                    MenuCommand::Engine(_)
                    | MenuCommand::Separator
                    | MenuCommand::Info
                    | MenuCommand::Dismiss,
                ) => {}
                MenuEffect::Run(MenuCommand::CancelDownloadsAndClose(closing)) => {
                    self.close_cancelling_downloads(closing);
                }
                // the row names the site of the menu's tab, whose page the change is for
                MenuEffect::Run(MenuCommand::SetPageTheme(_, theme)) => {
                    if let Some(page) = self.model.tabs.get(&tab).map(|t| t.url.clone()) {
                        self.set_site_theme(tab, &page, theme);
                    }
                }
                MenuEffect::Run(MenuCommand::ShotElement(at)) => self.shot_element(tab, at),
                MenuEffect::Run(MenuCommand::ToggleBoost(id)) => self.toggle_boost(id),
                MenuEffect::Run(MenuCommand::EditBoost(id)) => self.edit_boost(id),
                MenuEffect::Run(MenuCommand::ClearData(what)) => self.clear_data(tab, what),
                MenuEffect::Run(MenuCommand::CaretBrowsing { dont_ask }) => {
                    self.caret_event(wubble_core::caret::Event::Confirm { dont_ask });
                }
            }
        }
        answered
    }

    /// The menu's "Search … for": a new tab on the chosen engine's results, or
    /// with no engine chosen yet, the command bar holding the text, where one
    /// is picked.
    fn search_for(&mut self, tab: TabId, text: String) {
        let query = text.split_whitespace().collect::<Vec<_>>().join(" ");
        match self.search.default.as_ref() {
            Some(engine) => {
                let url = engine.search_url(&query);
                self.dispatch(new_tab(url, Some(tab)));
            }
            None => {
                let win = self.w.id;
                let _ = self.w.window.upgrade_in_event_loop(move |window| {
                    window.invoke_request_new_tab();
                    crate::with_host_in(win, |host| {
                        host.open_bar(&window, crate::bar::BarMode::NewTab, query)
                    });
                });
            }
        }
    }

    pub(super) fn apply_find(&mut self, tab: TabId, event: FindEvent) {
        self.find_step(tab, event, false);
    }

    /// A rule closed the tab's find bar (Escape, a navigation, another overlay): its search
    /// stops.
    pub(super) fn find_cancelled(&mut self, tab: TabId) {
        self.find_step(tab, FindEvent::Close, true);
    }

    /// One find event on the tab's bar (`cancel`: on the content a rule just closed). The last
    /// query outlives the bar (`Surfaces::find_last`); refused (a modal sheet over the page),
    /// Mod+F does nothing.
    fn find_step(&mut self, tab: TabId, event: FindEvent, cancel: bool) {
        let layer = Layer::new(Kind::Find, Some(tab));
        let live = match self.live(self.w.id, layer, cancel) {
            Some(Content::Find(f)) => Some(f),
            _ => None,
        };
        let last = self
            .surfaces
            .get(&tab)
            .map(|s| s.find_last.clone())
            .unwrap_or_default();
        let (next, effects) = surface::find(live, &last, event);
        if let Some(bar) = &next {
            self.surfaces.entry(tab).or_default().find_last = bar.query.clone();
        }
        if self
            .content_after(self.w.id, layer, next.map(Content::Find), false)
            .is_some()
        {
            return;
        }
        for effect in effects {
            if effect == FindEffect::Focus {
                self.find_focus = self.find_focus.wrapping_add(1);
                continue;
            }
            let Some(view) = self.tab_view(tab) else {
                continue;
            };
            match effect {
                FindEffect::Find(text) => view.find(&text, false),
                FindEffect::Next => view.find_next(),
                FindEffect::Prev => view.find_previous(),
                FindEffect::Stop => view.stop_find(true),
                FindEffect::Focus => {}
            }
        }
    }

    /// Runs one screen event; whether it was the screen's (Esc outside page
    /// fullscreen is the page's).
    pub(super) fn apply_screen(&mut self, window: &AppWindow, event: ScreenEvent) -> bool {
        let full = window.window().is_fullscreen();
        let state = screen::observe(self.w.screen, full);
        let (next, effects, consumed) = screen::transition(state, event);
        self.w.screen = next;
        for effect in effects {
            match effect {
                ScreenEffect::SetWindowFullscreen(on) => window.window().set_fullscreen(on),
                // `render_surfaces` draws the chrome from `self.w.screen`.
                ScreenEffect::HideChrome(_) => {}
                ScreenEffect::ExitPage(tab) => {
                    if let Some(v) = self.tab_view(tab) {
                        v.exit_fullscreen();
                    }
                }
                ScreenEffect::Hint => self.toast(window, "Press Esc to exit full screen"),
            }
        }
        self.render_surfaces(window);
        consumed
    }

    /// The tab's view closed: its surfaces close without answering (welding
    /// answered every held request), its fullscreen, sound and blocked count end.
    pub(super) fn view_gone(&mut self, tab: TabId) {
        self.drag_gone(tab);
        // an autoscroll over the page that went ends with it: it keeps no tab, and a closed tab is
        // no longer in the model, so any view going ends it (a press ends it anyway)
        let window = self.w.handle();
        self.autoscroll_input(&window, wubble_core::autoscroll::Input::Pressed);
        if self.devtools.dock.tab() == Some(tab) {
            self.dock_event(DockEvent::TabGone(tab));
        }
        self.builtins_view_gone(tab);
        // Whatever closed the view (a tab, its window, a discard): CEF answers no PDF for it (its
        // sheet's job ends before the overlays take the sheet).
        self.print_tab_closed(tab);
        // its hover slots (a page tooltip, a card, its video strip) and its overlays go, with no
        // answer (welding answered every request the view held)
        self.act(overlay::Action::ViewGone(tab));
        // a window's menu of its page closes too, unanswered
        let menu = Layer::new(Kind::Menu, None);
        let menus: Vec<WindowId> = self
            .ui
            .windows_with(Kind::Menu)
            .into_iter()
            .filter(|&(w, p)| {
                p == Phase::Open
                    && matches!(self.contents.get(w, menu), Some(Content::Menu { tab: t, .. }) if *t == tab)
            })
            .map(|(w, _)| w)
            .collect();
        for w in menus {
            self.window_closed(w, Kind::Menu);
        }
        self.ax_view_gone(tab);
        if let Some(s) = self.surfaces.get_mut(&tab) {
            s.popup = None;
            let (error, fx) =
                page_error::transition(std::mem::take(&mut s.error), ErrorEvent::ViewGone);
            debug_assert!(fx.is_empty(), "welding answered the panel's request");
            s.error = error;
        }
        self.downloads_lost(tab);
        self.autofill_gone(tab);
        self.forget_gone_blocks();
        self.audio.remove(&tab);
        self.busy_view_gone(tab);
        self.page_left(tab);
    }

    /// `tab`'s page left fullscreen (or its view closed, or it stopped being visible): the
    /// window that holds it leaves page fullscreen, whichever window the host is in; the host
    /// comes back to the one it was in.
    pub(super) fn page_left(&mut self, tab: TabId) {
        let here = self.w.id;
        let screens = std::iter::once(&self.w)
            .chain(self.parked.values())
            .map(|w| (w.id, w.screen));
        // a tab no window holds: the current window's `(_, PageLeft(_))` cell drops it, and the
        // current window is drawn again, as `Fullscreen(false)` (a PiP video's too) always did
        let owner = screen::page_owner(screens, tab).unwrap_or(here);
        if self.enter(owner)
            && let Some(window) = self.w.window.upgrade()
        {
            self.apply_screen(&window, ScreenEvent::PageLeft(tab));
        }
        self.enter(here);
    }

    /// The tab stopped being the visible one: the overlays follow their rules (menus and
    /// sheets close; dialogs, the find bar and a notice wait for the tab's return,
    /// wubble_core::overlay), its page leaves fullscreen.
    fn left_visible(&mut self, tab: TabId, now: Option<TabId>) {
        let w = self.w.id;
        self.act(overlay::Action::Switched {
            w,
            from: Some(tab),
            to: now,
        });
        if matches!(self.w.screen, Screen::Page { tab: t, .. } if t == tab) {
            if let Some(v) = self.tab_view(tab) {
                v.exit_fullscreen();
            }
            self.page_left(tab);
        }
    }

    /// Writes the visible tab's surfaces into the shell (Slint ignores
    /// values that did not change; the menu is compared here, see [`Drawn`]).
    pub(super) fn render_surfaces(&mut self, window: &AppWindow) {
        let tab = self.shown();
        if self.w.drawn.tab != tab
            && let Some(old) = self.w.drawn.tab
        {
            self.w.drawn.tab = tab;
            self.left_visible(old, tab);
        }
        let current = tab
            .and_then(|t| self.surfaces.get(&t))
            .cloned()
            .unwrap_or_default();
        // the store's overlays: each drawn open from its content while the store has it open
        let w = self.w.id;
        let shown_content =
            |kind| tab.and_then(|t| self.open_content(w, Layer::new(kind, Some(t))));
        let shown_open = |kind| {
            tab.is_some_and(|t| self.ui.phase(w, Layer::new(kind, Some(t))) == Some(Phase::Open))
        };
        let menu = match self.open_content(w, Layer::new(Kind::Menu, None)) {
            Some(Content::Menu { menu, .. }) => Some(menu.clone()),
            _ => None,
        };
        let permission = match shown_content(Kind::Permission) {
            Some(Content::Permission(p)) => Some(p.clone()),
            _ => None,
        };
        let dialog_content = match shown_content(Kind::Dialog) {
            Some(Content::Dialog(c)) => c.clone(),
            _ => DialogContent::default(),
        };
        if self.w.drawn.tab != tab || self.w.drawn.menu != menu {
            let scale = window.window().scale_factor();
            window.set_menu(menu_view(menu.as_ref(), scale, self.focus_origin(window)));
        }
        if self.w.drawn.tab != tab || self.w.drawn.permission != permission {
            window.set_permission(permission_view(permission.as_ref()));
        }
        // The sign-in sheet blocks the load; a JS dialog waits behind it. A sheet or panel that
        // closes keeps its text while it leaves; the closed dialog's serial resets its fields.
        let dialog = match (&dialog_content.auth, &dialog_content.js) {
            (Some(r), _) => auth_view(r),
            (None, js) => dialog_view(js.as_ref()),
        };
        window.set_dialog(if dialog.open {
            dialog
        } else {
            DialogView {
                open: false,
                serial: dialog.serial,
                ..window.get_dialog()
            }
        });
        let error = match shown_open(Kind::ErrorPanel) {
            true => error_view(&current.error),
            false => ErrorView::default(),
        };
        window.set_page_error(if error.open {
            error
        } else {
            ErrorView {
                open: false,
                ..window.get_page_error()
            }
        });
        // the find bar and popup are the key page's: the peek's while it has the keys; a closed
        // bar keeps the last query
        let keys = self.key_page();
        let bar_tab = keys.or(tab);
        let find = match bar_tab.and_then(|t| self.open_content(w, Layer::new(Kind::Find, Some(t))))
        {
            Some(Content::Find(f)) => Some(f),
            _ => None,
        };
        let last = bar_tab
            .and_then(|t| self.surfaces.get(&t))
            .map_or("", |s| s.find_last.as_str());
        window.set_find(find_view(find, last, self.find_focus));
        let up = keys.filter(|&t| {
            self.ui
                .phase(self.w.id, overlay::Layer::new(Kind::Popup, Some(t)))
                == Some(overlay::Phase::Open)
        });
        let popup = up.and_then(|t| self.surfaces.get(&t)).and_then(|s| s.popup);
        let origin = keys.map_or(self.focus_origin(window), |t| self.view_origin(window, t));
        let view = popup_view(window, origin, popup);
        window.set_popup(view.clone());
        match up {
            Some(tab) if view.shown => {
                let (x, y) = (f64::from(view.x), f64::from(view.y));
                let rect = wubble_core::tabdrag::Rect {
                    x,
                    y,
                    w: f64::from(view.width),
                    h: f64::from(view.height),
                };
                self.layer_placed(Kind::Popup, tab, rect);
            }
            // no popup: its ring goes with it
            _ => {
                window.set_popup_frame(slint::Image::default());
                self.w.popup_presenter.release();
            }
        }
        window.set_page_fullscreen(matches!(self.w.screen, Screen::Page { .. }));
        self.render_autofill(window, tab);
        self.w.drawn = Drawn {
            tab,
            menu,
            permission,
        };
    }

    // ----- intents from the shell (the visible tab's surfaces)

    /// The sheet's answer: the sign-in sheet's when one shows (`text` is
    /// the user name), else the JS dialog's.
    pub fn dialog_answered(&mut self, accept: bool, text: &str, secret: &str) {
        let Some(tab) = self.shown() else {
            return;
        };
        if self.dialog_content(tab, false).auth.is_some() {
            let event = if accept {
                AuthEvent::Submit {
                    user: text.into(),
                    password: Secret::from(secret),
                }
            } else {
                AuthEvent::Cancel
            };
            self.apply_auth(tab, event);
        } else {
            let event = if accept {
                DialogEvent::Accept(text.into())
            } else {
                DialogEvent::Cancel
            };
            self.apply_dialog(tab, event);
        }
        self.redraw_surfaces();
    }

    pub fn permission_answered(&mut self, allow: bool, remember: bool) {
        if let Some(tab) = self.shown() {
            self.apply_permission(tab, PermissionEvent::Answer { allow, remember });
            self.redraw_surfaces();
        }
    }

    /// The error panel's default button: Go back on a certificate panel,
    /// else Reload.
    pub fn error_primary(&mut self) {
        let Some(tab) = self.shown() else {
            return;
        };
        let event = match self.surfaces.get(&tab).map(|s| &s.error) {
            Some(
                PageError::Cert { .. } | PageError::HttpsOnly { .. } | PageError::Blocked { .. },
            ) => ErrorEvent::GoBack {
                can_go_back: self.tab_view(tab).is_some_and(|v| v.nav().page.can_go_back),
            },
            _ => ErrorEvent::Reload,
        };
        self.apply_error(tab, event);
        self.redraw_surfaces();
    }

    /// Proceed past a certificate error.
    pub fn error_secondary(&mut self) {
        if let Some(tab) = self.shown() {
            self.apply_error(tab, ErrorEvent::Proceed);
            self.redraw_surfaces();
        }
    }

    pub fn menu_chosen(&mut self, index: usize) {
        self.menu_event(MenuEvent::Chosen(index));
    }

    /// How many answers reached the engine (the page smoke checks).
    pub fn menu_dismissed(&mut self) -> usize {
        self.menu_event(MenuEvent::Dismiss)
    }

    fn menu_event(&mut self, event: MenuEvent) -> usize {
        let Some(tab) = self.shown() else {
            return 0;
        };
        let answered = self.apply_menu(tab, event);
        self.redraw_surfaces();
        answered
    }

    pub fn find_event(&mut self, event: FindEvent) {
        if let Some(tab) = self.key_page().or_else(|| self.shown()) {
            self.apply_find(tab, event);
            self.redraw_surfaces();
        }
    }

    /// The sidebar's speaker glyph, the tab menu's row and the key: the user's own mute, which
    /// a site rule leaves alone (`sitesound::transition`).
    pub fn toggle_mute(&mut self, tab: TabId) {
        self.site_sound(tab, MuteEvent::Toggled);
    }

    fn redraw_surfaces(&mut self) {
        if let Some(window) = self.w.window.upgrade() {
            self.render_surfaces(&window);
        }
    }

    // ----- zoom

    /// The user zoomed the visible `tab` (a zoom key) and its view now shows
    /// `percent` (CEF zooms at once, so the view read it back in the same
    /// call): stored for the site and shown, even when a limit left it
    /// unchanged. The view's own zoom reports (CEF re-reading the level after
    /// a load, the shell's corrections) are never stored, nor is a private
    /// window's zoom: it is kept in memory over the stored levels
    /// (`write_site`), as Chrome's off-the-record zoom map is.
    /// `tab`'s zoom while it is not the user's default (`zoom::indicator`).
    pub(super) fn zoomed(&mut self, tab: TabId) -> Option<u32> {
        let default = self.zoom_default;
        let percent = self.tab_view(tab)?.zoom_percent();
        zoom::indicator(percent, default)
    }

    /// The address pill's zoom chip: the shown tab's zoom when it is not the default, else 0.
    pub(super) fn address_zoom(&mut self) -> i32 {
        self.shown()
            .and_then(|tab| self.zoomed(tab))
            .map_or(0, |p| i32::try_from(p).unwrap_or(i32::MAX))
    }

    pub(super) fn user_zoomed(&mut self, tab: TabId, percent: u32) {
        if let Some(page) = self.model.tabs.get(&tab).map(|t| t.url.clone()) {
            self.write_site(tab, SiteChange::Zoom { page, percent });
        }
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, &format!("{percent}%"));
        }
    }

    /// A tab committed `url`: show it at the zoom stored for its site.
    pub(super) fn restore_zoom(&mut self, tab: TabId, url: &str) {
        // a private window's own level, else the stored one
        let want = self.site_view(tab).zoom(url);
        if let Some(view) = self.tab_view(tab)
            && view.zoom_percent() != want
        {
            view.set_zoom_percent(want);
        }
    }

    // ----- sound

    /// Runs one mute event for a tab with a view: its reason moves (`sitesound::transition`)
    /// and the view's mute follows when it says so.
    fn site_sound(&mut self, tab: TabId, event: MuteEvent) {
        if self.tab_view(tab).is_none() {
            return;
        }
        let why = self.audio.get(&tab).map(|a| a.why).unwrap_or_default();
        let (next, set) = sitesound::transition(why, event);
        self.audio.entry(tab).or_default().why = next;
        if let (Some(muted), Some(view)) = (set, self.tab_view(tab)) {
            view.set_muted(muted);
        }
    }

    /// A tab committed `url`: Chrome's `SoundContentSettingObserver::MuteOrUnmuteIfNecessary`.
    /// The site's sound content setting on the tab's profile is written to match the list
    /// first, so Chromium's own observer agrees with it.
    pub(super) fn restore_site_sound(&mut self, tab: TabId, url: &str) {
        let muted = sitesound::is_muted(&self.muted_sites, url);
        if let Some(host) = sitesound::web_host(url)
            && let Some(view) = self.tab_view(tab)
            && let Err(e) = view.set_sound_blocked(&sitesound::origins(&host), muted)
        {
            eprintln!("wubble: sound setting for {host}: {e}");
        }
        self.site_sound(tab, MuteEvent::Site(muted));
    }

    /// The `sound.muted_sites` setting changed (or was read at start): each container's sound
    /// content setting follows the entries that came and went, and every live page of a site
    /// whose rule changed is muted or unmuted (Chrome's observer on `OnContentSettingChanged`).
    pub(super) fn set_muted_sites(&mut self, sites: Vec<String>) {
        let (added, removed) = sitesound::diff(&self.muted_sites, &sites);
        self.muted_sites = sites;
        let tabs: Vec<TabId> = self.views.keys().copied().collect();
        // a container's profile is shared by its views: written through each, as CEF names
        // no context without one
        for &tab in &tabs {
            if let Some(view) = self.tab_view(tab) {
                for (list, blocked) in [(&added, true), (&removed, false)] {
                    for site in list.iter() {
                        if let Err(e) = view.set_sound_blocked(&sitesound::origins(site), blocked) {
                            eprintln!("wubble: sound setting for {site}: {e}");
                        }
                    }
                }
            }
        }
        for tab in tabs {
            if let Some(url) = self.committed.get(&tab).cloned() {
                self.restore_site_sound(tab, &url);
            }
        }
        self.after_change();
    }

    /// The tab menu's "Mute Site" switch: the tab's site joins the list, or the entry covering
    /// it leaves (the stored setting Settings > Privacy shows; applied through it).
    pub(super) fn toggle_mute_site(&mut self, tab: TabId) {
        let Some(url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) else {
            return;
        };
        let key = wubble_core::settings::SOUND_MUTED_SITES.key;
        match sitesound::toggle(&self.muted_sites, &url) {
            Some(Toggle::Remove(site)) => self.setting_entry_removed(key, &site),
            Some(Toggle::Add(host)) => self.setting_entry_added(key, &host),
            None => {}
        }
    }

    /// Rows of tabs with no view (asleep, not loaded yet) whose site is muted show muted: their
    /// page will be when it loads. A tab with a view shows what the view reports.
    pub(super) fn site_sound_marks(&self, view: &mut crate::shell::ShellView) {
        for row in view.pinned.iter_mut().chain(view.tabs.iter_mut()) {
            row.site_muted = row.kind == crate::shell::RowKind::Tab
                && crate::shell::tab_id(row.id).is_some_and(|t| !self.audio.contains_key(&t))
                && sitesound::is_muted(&self.muted_sites, &row.url);
        }
    }

    /// Asks every live tab's page whether it plays sound.
    pub fn probe_audible(&mut self) {
        let ids: Vec<ViewId> = self.views.values().copied().collect();
        for id in ids {
            if let Some(view) = self.engine.view(id) {
                view.probe_audible();
            }
        }
    }

    // ----- IME

    /// winit's IME events while the page has the keyboard; false when they
    /// are Slint's (a shell text field has it).
    pub(super) fn ime_event(&mut self, window: &AppWindow, event: &WinitIme) -> bool {
        if !window.get_web_focused() {
            return false;
        }
        let input = match event {
            WinitIme::Enabled => return true,
            WinitIme::Preedit(text, cursor) => ImeInput::Preedit(text.clone(), *cursor),
            WinitIme::Commit(text) => ImeInput::Commit(text.clone()),
            WinitIme::Disabled => ImeInput::Disabled,
        };
        let (next, call) = ime::transition(self.ime, input);
        self.ime = next;
        if let (Some(call), Some(view)) = (call, self.focused_view()) {
            match call {
                ImeCall::SetComposition { text, selection } => {
                    view.ime_set_composition(&text, selection)
                }
                ImeCall::Commit(text) => view.ime_commit(&text),
                ImeCall::Cancel => view.ime_cancel(),
            }
        }
        true
    }

    // ----- shell helpers

    pub(super) fn toast(&mut self, window: &AppWindow, text: &str) {
        // `--smoke media=probe` (the probe's step holds its own state while it saves)
        if std::env::var_os("WUBBLE_PROBE_SAVE").is_some() {
            println!("probe: toast {text:?}");
        }
        self.toast_replaced();
        self.show_toast(window, text, "", None);
    }

    /// A notice with a button (`action`); `sticky`: it stays until answered. What the button
    /// does is the caller's to remember (`host/videos.rs`).
    pub(super) fn toast_with(&mut self, text: &str, action: &str, sticky: bool) {
        self.toast_replaced();
        if let Some(window) = self.w.window.upgrade() {
            self.show_toast(&window, text, action, sticky.then_some(Key::None));
        }
    }

    /// The page's question over `tab` (`overlay::Kind::Notice`), held on the toast until
    /// answered; what its button does is the caller's to remember.
    pub(super) fn toast_notice(&mut self, tab: TabId, text: &str, action: &str) {
        self.toast_replaced();
        if let Some(window) = self.w.window.upgrade() {
            self.show_toast(&window, text, action, Some(Key::Notice(tab)));
        }
    }

    /// Takes whatever notice shows away.
    pub(super) fn toast_hide(&mut self) {
        self.hover(Kind::Toast, Key::None, Cue::Dismiss);
    }

    /// Shows a notice for its time (`overlay::TOAST_SHOWN_MS`; with a button the one undo
    /// window, `sponsor::TOAST_MS`, which no button outlives), restarting the time; a `held`
    /// one, for its key, until it is answered or taken away. The window's toast slot of the
    /// store times it; another toast replaces it in place.
    fn show_toast(&mut self, window: &AppWindow, text: &str, action: &str, held: Option<Key>) {
        window.set_toast_text(text.into());
        window.set_toast_action(action.into());
        window.set_toast_sticky(held.is_some());
        let cue = wubble_core::overlay::toast_ms(!action.is_empty(), held.is_some())
            .map_or(Cue::Show, Cue::ShowFor);
        self.hover(Kind::Toast, held.unwrap_or(Key::None), cue);
    }

    /// Puts `text` on the system clipboard: through the window's Wayland connection, else
    /// through Slint.
    pub(super) fn copy(&mut self, text: &str) {
        if let Some(window) = self.w.window.upgrade()
            && !self
                .clipboard
                .store(window.window(), Buffer::Clipboard, text)
        {
            window.invoke_copy_text(text.into());
        }
    }

    /// A frame's clipboard request (wubble-helper `clipboard.js`): copies go to the system
    /// clipboard with every type they have, pastes and reads come from it with the types their
    /// reader takes, or from the primary selection after a middle click. Only the frame the
    /// user is in is served (`clipboard::admit`); a refused read is answered with a refusal.
    fn clipboard_request(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        frame: String,
        request: ClipboardRequest,
    ) {
        let ask = match &request {
            ClipboardRequest::Copy(_) | ClipboardRequest::CopyImage(_) => Ask::Write,
            ClipboardRequest::Primary(_) => Ask::Primary,
            ClipboardRequest::Paste(..) | ClipboardRequest::Read(..) => Ask::Read,
        };
        let asker = Asker {
            key_page: self.key_page() == Some(tab),
            window_focused: self.input.window_focused && self.model.focused() == self.w.id,
            page_keyboard: self.input.view_focused,
            frame_focused: self
                .tab_view(tab)
                .and_then(|v| v.focused_frame())
                .is_some_and(|f| f == frame),
            reading: clipboard::in_flight(
                self.clipboard_reads.get(&tab).copied(),
                self.input.now(),
            ),
        };
        if !clipboard::admit(ask, asker) {
            if let (ClipboardRequest::Read(token, _), Some(view)) = (&request, self.tab_view(tab)) {
                view.answer_clipboard(&frame, None, token);
            }
            return;
        }
        let (buffer, reader, token) = match request {
            // Its pictures are checked off the window's thread (`page_copy`), then stored.
            ClipboardRequest::Copy(payload) => {
                let id = self.w.id;
                let spawned = std::thread::Builder::new()
                    .name("wubble-page-copy".into())
                    .spawn(move || {
                        let payload = page_copy(payload);
                        let _ = slint::invoke_from_event_loop(move || {
                            crate::with_host_in(id, |host| {
                                let Some(window) = host.w.window.upgrade() else {
                                    return;
                                };
                                let text = payload.text.clone();
                                if !host.clipboard.store_rich(window.window(), payload)
                                    && let Some(text) = text
                                {
                                    host.copy(&text);
                                }
                            })
                        });
                    });
                if let Err(e) = spawned {
                    eprintln!("wubble: a page's copy: {e}");
                }
                return;
            }
            ClipboardRequest::CopyImage(url) => {
                if let Some(view) = self.tab_view(tab) {
                    view.download_image(&url);
                }
                return;
            }
            ClipboardRequest::Primary(text) => {
                self.clipboard
                    .store(window.window(), Buffer::Primary, &text);
                return;
            }
            ClipboardRequest::Paste(from, token) => match from {
                PasteFrom::Clipboard => (Buffer::Clipboard, Reader::Paste, token),
                PasteFrom::PlainText => (Buffer::Clipboard, Reader::Text, token),
                PasteFrom::Primary => (Buffer::Primary, Reader::Text, token),
            },
            ClipboardRequest::Read(token, reader) => (Buffer::Clipboard, reader, token),
        };
        self.clipboard_fetch(window, tab, frame, (buffer, reader), token);
    }

    /// Reads what `reader` takes of `buffer` off the window's thread and answers `frame` of
    /// `tab` with it under the asking document's `token` (empty: a paste the host starts); one
    /// at a time per tab.
    fn clipboard_fetch(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        frame: String,
        (buffer, reader): (Buffer, Reader),
        token: String,
    ) {
        let (id, started) = (self.w.id, self.input.now());
        let (answer, asked) = (frame.clone(), token.clone());
        let done = Box::new(move |payload: Option<Payload>| {
            let _ = slint::invoke_from_event_loop(move || {
                // Whichever window has the host: the mark goes even when the asking window
                // closed meanwhile (one the host was too busy for expires, `in_flight`).
                crate::with_host(move |host| {
                    // this read's mark only: a later read's stays
                    if host.clipboard_reads.get(&tab) == Some(&started) {
                        host.clipboard_reads.remove(&tab);
                    }
                    if host.enter(id)
                        && let Some(view) = host.tab_view(tab)
                    {
                        view.answer_clipboard(&answer, Some(&payload.unwrap_or_default()), &asked);
                    }
                });
            });
        });
        if self
            .clipboard
            .load_then(window.window(), buffer, reader, done)
        {
            // the marks of reads given up for (their tabs closed meanwhile) go too
            self.clipboard_reads
                .retain(|_, s| clipboard::in_flight(Some(*s), started));
            self.clipboard_reads.insert(tab, started);
        } else if let Some(view) = self.tab_view(tab) {
            view.answer_clipboard(&frame, Some(&Payload::default()), &token);
        }
    }

    /// The menu's Paste as Plain Text in `frame`. Where the host carries the pages' clipboard it
    /// pastes the clipboard's text itself (`clipboard.js` cannot tell Chromium's plain paste from
    /// its paste); elsewhere Chromium's own does it.
    fn paste_plain(&mut self, tab: TabId, frame: String) {
        let window = self.w.window.upgrade();
        match window {
            Some(window) if !frame.is_empty() && self.clipboard.carries_pages(window.window()) => {
                self.clipboard_fetch(
                    &window,
                    tab,
                    frame,
                    (Buffer::Clipboard, Reader::Text),
                    String::new(),
                )
            }
            _ => {
                if let Some(view) = self.tab_view(tab) {
                    view.edit(EditCommand::PasteMatchStyle);
                }
            }
        }
    }
}

/// A page's copy as the host puts it on the clipboard: its pictures only as PNG of at most
/// `clipboard::PAGE_PICTURE_PIXELS` (by its header, before decoding), decoded and encoded
/// again, so no page hands another app a malformed or other image file through it.
fn page_copy(payload: Payload) -> Payload {
    let fits = |png: &[u8]| {
        crate::shot_io::png_size(png)
            .is_some_and(|(w, h)| u64::from(w) * u64::from(h) <= clipboard::PAGE_PICTURE_PIXELS)
    };
    let files = payload
        .files
        .into_iter()
        .filter(|f| f.mime.eq_ignore_ascii_case(dnd::PNG) && fits(&f.bytes))
        .filter_map(|f| {
            let (width, height, rgba) = crate::shot_io::rgba(&f.bytes).ok()?;
            let bytes = crate::shot_io::encode_png(width, height, &rgba).ok()?;
            Some(clipboard::File {
                mime: dnd::PNG.to_owned(),
                bytes,
                ..f
            })
        })
        .collect();
    Payload { files, ..payload }
}

/// Puts the IME candidate window next to the composition (CEF reports it
/// only while composing; physical pixels relative to the view, whose top
/// left is `origin` in the window).
fn place_candidates(window: &AppWindow, origin: (f32, f32), rect: Rect) {
    let scale = f64::from(window.window().scale_factor());
    let (x, y) = (f64::from(origin.0), f64::from(origin.1));
    let at = LogicalPosition::new(x + f64::from(rect.x) / scale, y + f64::from(rect.y) / scale);
    let size = LogicalSize::new(
        f64::from(rect.width) / scale,
        f64::from(rect.height) / scale,
    );
    window
        .window()
        .with_winit_window(|w| w.set_ime_cursor_area(at, size));
}

pub(super) fn model<T: Clone + 'static>(items: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(items)))
}

/// The menu in window coordinates: a page's is placed from its view's top left (`origin`), the
/// shell's is already in the window's.
fn menu_view(menu: Option<&Menu>, scale: f32, origin: (f32, f32)) -> MenuView {
    let (at, entries, origin) = match menu {
        None => return MenuView::default(),
        Some(Menu::Open { at, entries, .. }) => (at, entries, origin),
        Some(Menu::Shell { at, entries }) => (at, entries, (0.0, 0.0)),
    };
    MenuView {
        open: true,
        icons: entries.iter().any(|e| !e.icon.is_empty()),
        x: origin.0 + at.0 as f32 / scale,
        y: origin.1 + at.1 as f32 / scale,
        entries: model(
            entries
                .iter()
                .map(|e| MenuEntry {
                    label: e.label.as_str().into(),
                    enabled: e.enabled,
                    separator: e.command == MenuCommand::Separator,
                    icon: e.icon.into(),
                    shortcut: e.shortcut.as_str().into(),
                    danger: e.danger,
                })
                .collect(),
        ),
    }
}

fn dialog_view(dialog: Option<&OpenDialog>) -> DialogView {
    match dialog {
        None => DialogView::default(),
        Some(d) => {
            let text = surface::dialog_text(d);
            DialogView {
                open: true,
                title: text.title.into(),
                message: text.message.into(),
                accept: text.accept.into(),
                cancel: text.cancel.into(),
                prompt: text.prompt,
                secret: false,
                input: d.default_text.as_str().into(),
                // Unique per request, so a new dialog resets the sheet.
                serial: i32::try_from(d.id.0).unwrap_or(i32::MAX),
            }
        }
    }
}

fn auth_view(r: &prompt::AuthRequest) -> DialogView {
    let (title, message) = prompt::auth_text(r);
    DialogView {
        open: true,
        title: title.into(),
        message: message.into(),
        accept: "Sign in".into(),
        cancel: "Cancel".into(),
        prompt: true,
        secret: true,
        input: Default::default(),
        serial: i32::try_from(r.id.0).unwrap_or(i32::MAX),
    }
}

fn permission_view(p: Option<&PermissionPrompt>) -> PermissionView {
    match p {
        None => PermissionView::default(),
        Some(PermissionPrompt { shown: r, .. }) => {
            let t = prompt::permission_text(r);
            PermissionView {
                open: true,
                title: t.title.into(),
                lines: model(t.lines.into_iter().map(Into::into).collect()),
                remember: t.remember.into(),
                allow: t.allow.into(),
                block: t.block.into(),
                // Per request, so a new prompt resets the toggle.
                serial: r.serial(),
            }
        }
    }
}

fn error_view(e: &PageError) -> ErrorView {
    match page_error::error_text(e) {
        None => ErrorView::default(),
        Some(t) => ErrorView {
            open: true,
            title: t.title.into(),
            summary: t.summary.into(),
            detail: t.detail.into(),
            primary: t.primary.into(),
            secondary: t.secondary.into(),
            cert: t.cert,
        },
    }
}

/// The find bar: the open one's, or a closed one keeping the `last` query.
fn find_view(find: Option<&FindBar>, last: &str, focus: i32) -> FindView {
    FindView {
        open: find.is_some(),
        query: find.map_or(last, |f| f.query.as_str()).into(),
        counter: find.map(FindBar::counter).unwrap_or_default().into(),
        none: find.is_some_and(FindBar::none),
        focus_serial: focus,
    }
}

/// The popup in window coordinates (its view's top left is `view` in the
/// window), kept inside the window. Input maps back through `origin`, the
/// engine's own rect: CEF hit-tests the popup there, wherever the layer was
/// moved to fit.
fn popup_view(window: &AppWindow, view: (f32, f32), popup: Option<Rect>) -> PopupView {
    let Some(r) = popup else {
        return PopupView::default();
    };
    let scale = window.window().scale_factor();
    let size = window.window().size();
    let physical = |logical: f32| (logical * scale).round() as i32;
    let at = Rect {
        x: physical(view.0) + r.x,
        y: physical(view.1) + r.y,
        ..r
    };
    let bounds = Rect {
        x: 0,
        y: 0,
        width: size.width,
        height: size.height,
    };
    let origin = r;
    let r = surface::clamp_into(at, bounds);
    PopupView {
        shown: true,
        x: r.x as f32 / scale,
        y: r.y as f32 / scale,
        width: r.width as f32 / scale,
        height: r.height as f32 / scale,
        origin_x: origin.x as f32 / scale,
        origin_y: origin.y as f32 / scale,
    }
}

/// The wall clock in ms, as the error panel's schedule counts it.
pub(super) fn wall_ms() -> u64 {
    u64::try_from(super::now_ms()).unwrap_or(0)
}
