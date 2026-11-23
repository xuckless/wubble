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
use wubble_core::overlay::{Event as OverlayEvent, Kind};
use wubble_core::site::{Decision, Permission};
use wubble_core::tab::TabId;
use wubble_core::window::WindowTarget;
use wubble_core::zoom;
use wubble_engine::app::ViewId;
use wubble_engine::page::{ClipboardRequest, EditCommand, PageEvent, PasteFrom, Rect, RequestId};
use wubble_ui::{
    AppWindow, DialogView, ErrorView, FindView, MenuEntry, MenuView, PermissionView, PopupView,
};

use super::{Host, new_tab, now_ms};
use crate::chooser::{self, DialogSpec};
use crate::clipboard::{Bridge as _, Buffer};
use crate::ime::{self, ImeCall, ImeInput};
use crate::menu::{self, Command as MenuCommand, Menu, MenuEffect, MenuEvent, NavFlags};
use crate::page_error::{self, ErrorEffect, ErrorEvent, PageError};
use crate::prompt::{self, Auth, AuthEvent, PermissionEvent, PermissionPrompt, Reply, Secret};
use crate::screen::{self, Screen, ScreenEffect, ScreenEvent};
use crate::surface::{self, Dialog, DialogEvent, FindBar, FindEffect, FindEvent, Routed};

/// What the shell last drew: the tab, and its menu. Slint skips setting a
/// property to an equal value, but a menu's rows are a fresh `ModelRc`
/// (equal only by pointer), and a rewritten model restarts the rows' motion.
#[derive(Default)]
pub(super) struct Drawn {
    tab: Option<TabId>,
    menu: Menu,
    permission: PermissionPrompt,
}

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
            if self.agent_answer(&event) || self.ax_answer(&event) || self.media_answer(tab, &event)
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
        // A page's question closes the user's overlays over its tab (wubble_core::overlay).
        if !driven
            && matches!(
                routed,
                Routed::Dialog(DialogEvent::Requested(_)) | Routed::Auth(AuthEvent::Requested(_))
            )
        {
            self.overlay_open(Kind::Dialog, Some(tab));
        }
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
                    search: self.search_engine.map(|e| e.info().name),
                    image: self.search_engine.and_then(|e| e.info().image),
                    ..nav
                };
                let extra = self.model.windowed(tab).map_or_else(Vec::new, |w| {
                    crate::menu::window_page_entries(&self.model, w)
                });
                let allowed = self.overlay_open(Kind::Menu, None);
                self.apply_menu(
                    tab,
                    MenuEvent::Requested {
                        id,
                        menu,
                        nav,
                        extra,
                    },
                );
                // refused (a page's dialog waits): the request is answered at once
                if !allowed {
                    self.apply_menu(tab, MenuEvent::Dismiss);
                }
            }
            Routed::Menu(e) => {
                self.apply_menu(tab, e);
            }
            Routed::Find(e) => self.apply_find(tab, e),
            Routed::Drag { data, allowed } => self.drag_started(tab, data, allowed),
            Routed::DragCursor(op) => self.drag_cursor(tab, op),
            Routed::Popup(rect) => self.surfaces.entry(tab).or_default().popup = rect,
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
                self.apply_screen(window, ScreenEvent::PageLeft(tab));
            }
            Routed::Muted(muted) => {
                self.audio.entry(tab).or_default().muted = muted;
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
                // a new document starts its reports from nothing
                self.surfaces.entry(tab).or_default().datalist = None;
                let shown = self.shown();
                self.overlay_event(window, shown, OverlayEvent::Navigated(tab));
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
            Routed::Error(e) => self.apply_error(tab, e),
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
            // Only the main frame reports one (`datalist.js`): its box is the view's.
            Routed::Datalist { frame, report } if frame.is_main => {
                self.surfaces.entry(tab).or_default().datalist =
                    wubble_core::datalist::parse(&report).map(|offer| (frame.id, offer));
            }
            Routed::Datalist { .. } => {}
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
        let s = self.surfaces.entry(tab).or_default();
        let (next, answer) = surface::dialog(std::mem::take(&mut s.dialog), event);
        s.dialog = next;
        let answer = answer?;
        let view = self.tab_view(tab)?;
        Some(view.answer_dialog(answer.id, answer.accept, &answer.text))
    }

    /// Runs one permission event: remembered decisions are stored, every
    /// answer goes to the engine.
    pub(super) fn apply_permission(&mut self, tab: TabId, event: PermissionEvent) {
        if self.agent_drives(tab) {
            if let PermissionEvent::Requested(req) = event
                && let crate::prompt::Ask::Engine { id, .. } = req.ask
                && let Some(view) = self.tab_view(tab)
            {
                view.answer_permission(id, &[]);
            }
            return;
        }
        let s = self.surfaces.entry(tab).or_default();
        let (next, answers) = prompt::permission(
            std::mem::take(&mut s.permission),
            event,
            &self.site_permissions,
        );
        s.permission = next;
        for answer in answers {
            if let Some((site, kinds, decision)) = &answer.remember {
                for &kind in kinds {
                    self.site_permissions.set(site, kind, *decision);
                    self.store
                        .set_site_permission(site, kind, *decision, now_ms());
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
            }
        }
    }

    pub(super) fn apply_auth(&mut self, tab: TabId, event: AuthEvent) {
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
        let s = self.surfaces.entry(tab).or_default();
        let (next, answer) = prompt::auth(std::mem::take(&mut s.auth), event);
        s.auth = next;
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
        s.error = next;
        for effect in effects {
            if let ErrorEffect::Allow { host, code } = &effect {
                self.cert_exceptions.allow(host, *code);
                continue;
            }
            let Some(view) = self.tab_view(tab) else {
                continue;
            };
            match effect {
                ErrorEffect::Reload => view.reload(),
                ErrorEffect::Back => view.back(),
                ErrorEffect::Blank => view.load("about:blank"),
                ErrorEffect::AnswerCert { request, proceed } => {
                    view.answer_certificate(request, proceed);
                }
                ErrorEffect::Allow { .. } => {}
            }
        }
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

    /// Runs one menu event; how many answers reached the engine.
    pub(super) fn apply_menu(&mut self, tab: TabId, event: MenuEvent) -> usize {
        let s = self.surfaces.entry(tab).or_default();
        let (next, effects) = menu::transition(std::mem::take(&mut s.menu), event);
        s.menu = next;
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
                    let before = self.model.visible_tab();
                    self.open_routed(url, Some(tab));
                    // A link another app takes opens no tab: only a new one is torn out.
                    if let Some(new) = self.model.visible_tab().filter(|t| Some(*t) != before) {
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
                MenuEffect::Run(MenuCommand::ReloadTab(t)) => {
                    if let Some(view) = self.tab_view(t) {
                        view.reload();
                    }
                }
                MenuEffect::Run(MenuCommand::RenameFolder(f)) => self.rename_folder_requested(f),
                MenuEffect::Run(MenuCommand::ToggleBlocker(host)) => self.toggle_blocker(&host),
                MenuEffect::Run(MenuCommand::SpaceNeverSleep(space, on)) => {
                    self.dispatch(Command::SetSpaceNeverSleep { space, on });
                }
                MenuEffect::Run(MenuCommand::ToggleNeverSleepSite(site)) => {
                    self.toggle_never_sleep_site(&site);
                }
                MenuEffect::Run(MenuCommand::MoveToWindow(t, to)) => {
                    self.dispatch(Command::MoveTabToWindow { tab: t, to });
                }
                MenuEffect::Run(MenuCommand::OpenBlockedPopups) => self.open_held_popups(tab),
                MenuEffect::Run(MenuCommand::PromoteLittle(window, space)) => {
                    self.dispatch(Command::PromoteLittle { window, space });
                }
                MenuEffect::Run(MenuCommand::AllowPopups(site)) => {
                    self.site_permissions
                        .set(&site, Permission::Popups, Decision::Allow);
                    self.store.set_site_permission(
                        &site,
                        Permission::Popups,
                        Decision::Allow,
                        now_ms(),
                    );
                    self.open_held_popups(tab);
                }
                MenuEffect::Run(MenuCommand::SaveMedia { item, choice }) => {
                    self.save_media(tab, item, choice)
                }
                MenuEffect::Run(
                    MenuCommand::Engine(_) | MenuCommand::Separator | MenuCommand::Info,
                ) => {}
                MenuEffect::Run(MenuCommand::SetPageTheme(host, theme)) => {
                    self.set_site_theme(&host, theme)
                }
                MenuEffect::Run(MenuCommand::ShotElement(at)) => self.shot_element(tab, at),
                MenuEffect::Run(MenuCommand::ToggleBoost(id)) => self.toggle_boost(id),
                MenuEffect::Run(MenuCommand::EditBoost(id)) => self.edit_boost(id),
                MenuEffect::Run(MenuCommand::ClearData(what)) => self.clear_data(tab, what),
            }
        }
        answered
    }

    /// The menu's "Search … for": a new tab on the chosen engine's results, or
    /// with no engine chosen yet, the command bar holding the text, where one
    /// is picked.
    fn search_for(&mut self, tab: TabId, text: String) {
        let query = text.split_whitespace().collect::<Vec<_>>().join(" ");
        match self.search_engine {
            Some(engine) => {
                let url = wubble_core::omnibox::fill(engine.info().search, &query);
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
        let s = self.surfaces.entry(tab).or_default();
        let (next, effects) = surface::find(std::mem::take(&mut s.find), event);
        s.find = next;
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
        let state = screen::observe(self.screen, full);
        let (next, effects, consumed) = screen::transition(state, event);
        self.screen = next;
        for effect in effects {
            match effect {
                ScreenEffect::SetWindowFullscreen(on) => window.window().set_fullscreen(on),
                // `render_surfaces` draws the chrome from `self.screen`.
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
        if self.devtools.dock.tab() == Some(tab) {
            self.dock_event(DockEvent::TabGone(tab));
        }
        self.builtins_view_gone(tab);
        self.ax_view_gone(tab);
        // Whatever closed the view (a tab, its window, a discard): CEF answers no PDF for it.
        self.print_tab_closed(tab);
        if let Some(s) = self.surfaces.remove(&tab) {
            self.surfaces.insert(tab, s.view_gone());
        }
        self.downloads_lost(tab);
        self.autofill_gone(tab);
        self.forget_gone_blocks();
        self.audio.remove(&tab);
        self.busy_view_gone(tab);
        if let Some(window) = self.w.window.upgrade() {
            self.apply_screen(&window, ScreenEvent::PageLeft(tab));
        }
    }

    /// The tab stopped being the visible one: the overlays follow their rules (menus and
    /// sheets close; dialogs, the find bar and a notice wait for the tab's return,
    /// wubble_core::overlay), its page leaves fullscreen.
    fn left_visible(&mut self, window: &AppWindow, tab: TabId, now: Option<TabId>) {
        self.overlay_event(window, Some(tab), OverlayEvent::Switched(now));
        if matches!(self.screen, Screen::Page { tab: t, .. } if t == tab) {
            if let Some(v) = self.tab_view(tab) {
                v.exit_fullscreen();
            }
            self.apply_screen(window, ScreenEvent::PageLeft(tab));
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
            self.left_visible(window, old, tab);
        }
        let current = tab
            .and_then(|t| self.surfaces.get(&t))
            .cloned()
            .unwrap_or_default();
        if self.w.drawn.tab != tab || self.w.drawn.menu != current.menu {
            let scale = window.window().scale_factor();
            window.set_menu(menu_view(&current.menu, scale, self.focus_origin(window)));
        }
        if self.w.drawn.tab != tab || self.w.drawn.permission != current.permission {
            window.set_permission(permission_view(&current.permission));
        }
        // The sign-in sheet blocks the load; a JS dialog waits behind it. A sheet or panel that
        // closes keeps its text while it leaves; the closed dialog's serial resets its fields.
        let dialog = match &current.auth {
            Auth::Open(r) => auth_view(r),
            Auth::None => dialog_view(&current.dialog),
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
        let error = error_view(&current.error);
        window.set_page_error(if error.open {
            error
        } else {
            ErrorView {
                open: false,
                ..window.get_page_error()
            }
        });
        // the find bar and popup are the key page's: the peek's while it has the keys
        let keys = self.key_page();
        let find = keys
            .or(tab)
            .and_then(|t| self.surfaces.get(&t))
            .map(|s| &s.find)
            .unwrap_or(&current.find);
        window.set_find(find_view(find, self.find_focus));
        let popup = keys
            .and_then(|t| self.surfaces.get(&t))
            .and_then(|s| s.popup);
        let origin = keys.map_or(self.focus_origin(window), |t| self.view_origin(window, t));
        window.set_popup(popup_view(window, origin, popup));
        if popup.is_none() {
            window.set_popup_frame(slint::Image::default());
        }
        window.set_page_fullscreen(matches!(self.screen, Screen::Page { .. }));
        self.render_autofill(window, tab);
        self.w.drawn = Drawn {
            tab,
            menu: current.menu,
            permission: current.permission,
        };
    }

    // ----- intents from the shell (the visible tab's surfaces)

    /// The sheet's answer: the sign-in sheet's when one shows (`text` is
    /// the user name), else the JS dialog's.
    pub fn dialog_answered(&mut self, accept: bool, text: &str, secret: &str) {
        let Some(tab) = self.shown() else {
            return;
        };
        let signing_in = matches!(
            self.surfaces.get(&tab).map(|s| &s.auth),
            Some(Auth::Open(_))
        );
        if signing_in {
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
            Some(PageError::Cert { .. }) => ErrorEvent::GoBack {
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
            if event == FindEvent::Open && !self.overlay_open(Kind::Find, Some(tab)) {
                return;
            }
            self.apply_find(tab, event);
            self.redraw_surfaces();
        }
    }

    /// The sidebar's speaker glyph.
    pub fn toggle_mute(&mut self, tab: TabId) {
        if let Some(view) = self.tab_view(tab) {
            let muted = view.is_muted();
            view.set_muted(!muted);
        }
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
    /// a load, the shell's corrections) are never stored.
    pub(super) fn user_zoomed(&mut self, tab: TabId, percent: u32) {
        let url = self.model.tabs.get(&tab).map(|t| t.url.clone());
        if let Some(host) = url.as_deref().and_then(zoom::host_of) {
            let table = zoom::next_table(self.site_zoom.clone(), &host, percent, self.zoom_default);
            // Applied as the setting: the table becomes `site_zoom` there.
            self.write_setting(
                wubble_core::settings::SITE_ZOOM.key,
                &wubble_store::text_of::<wubble_store::SiteZoom>(&table),
            );
        }
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, &format!("{percent}%"));
        }
    }

    /// A tab committed `url`: show it at the zoom stored for its site.
    pub(super) fn restore_zoom(&mut self, tab: TabId, url: &str) {
        let want = zoom::wanted(&self.site_zoom, url, self.zoom_default);
        if let Some(view) = self.tab_view(tab)
            && view.zoom_percent() != want
        {
            view.set_zoom_percent(want);
        }
    }

    // ----- sound

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
        self.show_toast(window, text, "", false);
    }

    /// A notice with a button (`action`); `sticky`: it stays until answered. What the button
    /// does is the caller's to remember (`host/videos.rs`).
    pub(super) fn toast_with(&mut self, text: &str, action: &str, sticky: bool) {
        self.toast_replaced();
        if let Some(window) = self.w.window.upgrade() {
            self.show_toast(&window, text, action, sticky);
        }
    }

    /// Takes whatever notice shows away.
    pub(super) fn toast_hide(&mut self) {
        self.toast_serial = self.toast_serial.wrapping_add(1);
        if let Some(window) = self.w.window.upgrade() {
            window.set_toast_hide_serial(self.toast_serial);
        }
    }

    fn show_toast(&mut self, window: &AppWindow, text: &str, action: &str, sticky: bool) {
        self.toast_serial = self.toast_serial.wrapping_add(1);
        window.set_toast_text(text.into());
        window.set_toast_action(action.into());
        window.set_toast_sticky(sticky);
        // The one undo window: a button outlives no notice it acts on.
        window.set_toast_action_shown(wubble_core::sponsor::TOAST_MS as i64);
        window.set_toast_serial(self.toast_serial);
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
            ClipboardRequest::Paste(_) | ClipboardRequest::Read(..) => Ask::Read,
        };
        let asker = Asker {
            key_page: self.key_page() == Some(tab),
            window_focused: self.input.window_focused && self.model.focused() == self.w.id,
            frame_focused: self
                .tab_view(tab)
                .and_then(|v| v.focused_frame())
                .is_some_and(|f| f == frame),
            reading: self.clipboard_reads.contains(&tab),
        };
        if !clipboard::admit(ask, asker) {
            if let (ClipboardRequest::Read(id, _), Some(view)) = (&request, self.tab_view(tab)) {
                view.answer_clipboard(&frame, None, Some(*id));
            }
            return;
        }
        let (buffer, reader, read) = match request {
            ClipboardRequest::Copy(payload) => {
                let payload = page_copy(payload);
                let text = payload.text.clone();
                if !self.clipboard.store_rich(window.window(), payload)
                    && let Some(text) = text
                {
                    self.copy(&text);
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
            ClipboardRequest::Paste(from) => match from {
                PasteFrom::Clipboard => (Buffer::Clipboard, Reader::Paste, None),
                PasteFrom::PlainText => (Buffer::Clipboard, Reader::Text, None),
                PasteFrom::Primary => (Buffer::Primary, Reader::Text, None),
            },
            ClipboardRequest::Read(id, reader) => (Buffer::Clipboard, reader, Some(id)),
        };
        self.clipboard_fetch(window, tab, frame, (buffer, reader), read);
    }

    /// Reads what `reader` takes of `buffer` off the window's thread and answers `frame` of
    /// `tab` with it (`read`: the read's id; none: a paste); one at a time per tab.
    fn clipboard_fetch(
        &mut self,
        window: &AppWindow,
        tab: TabId,
        frame: String,
        (buffer, reader): (Buffer, Reader),
        read: Option<u32>,
    ) {
        let id = self.w.id;
        let answer = frame.clone();
        let done = Box::new(move |payload: Option<Payload>| {
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host_in(id, |host| {
                    host.clipboard_reads.remove(&tab);
                    if let Some(view) = host.tab_view(tab) {
                        view.answer_clipboard(&answer, Some(&payload.unwrap_or_default()), read);
                    }
                });
            });
        });
        if self
            .clipboard
            .load_then(window.window(), buffer, reader, done)
        {
            self.clipboard_reads.insert(tab);
        } else if let Some(view) = self.tab_view(tab) {
            view.answer_clipboard(&frame, Some(&Payload::default()), read);
        }
    }

    /// The menu's Paste as Plain Text in `frame`. Where the host carries the pages' clipboard it
    /// pastes the clipboard's text itself (`clipboard.js` cannot tell Chromium's plain paste from
    /// its paste); elsewhere Chromium's own does it.
    fn paste_plain(&mut self, tab: TabId, frame: String) {
        let window = self.w.window.upgrade();
        match window {
            Some(window) if !frame.is_empty() && self.clipboard.carries_pages(window.window()) => {
                self.clipboard_fetch(&window, tab, frame, (Buffer::Clipboard, Reader::Text), None)
            }
            _ => {
                if let Some(view) = self.tab_view(tab) {
                    view.edit(EditCommand::PasteMatchStyle);
                }
            }
        }
    }
}

/// A page's copy as the host puts it on the clipboard: its pictures only as PNG, decoded and
/// encoded again, so no page hands another app a malformed or other image file through it.
fn page_copy(payload: Payload) -> Payload {
    let files = payload
        .files
        .into_iter()
        .filter(|f| f.mime.eq_ignore_ascii_case(dnd::PNG))
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
fn menu_view(menu: &Menu, scale: f32, origin: (f32, f32)) -> MenuView {
    let (at, entries, origin) = match menu {
        Menu::Closed => return MenuView::default(),
        Menu::Open { at, entries, .. } => (at, entries, origin),
        Menu::Shell { at, entries } => (at, entries, (0.0, 0.0)),
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

fn dialog_view(dialog: &Dialog) -> DialogView {
    match dialog {
        Dialog::None => DialogView::default(),
        Dialog::Open(d) => {
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

fn permission_view(p: &PermissionPrompt) -> PermissionView {
    match p {
        PermissionPrompt::None => PermissionView::default(),
        PermissionPrompt::Open { shown: r, .. } => {
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

fn find_view(find: &FindBar, focus: i32) -> FindView {
    FindView {
        open: matches!(find, FindBar::Open { .. }),
        query: find.query().into(),
        counter: find.counter().into(),
        none: find.none(),
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
