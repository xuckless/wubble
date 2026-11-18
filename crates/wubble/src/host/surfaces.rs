//! The host's half of the page's own UI: page events in, through the pure
//! machines (`surface.rs`, `menu.rs`, `screen.rs`, `ime.rs`, `prompt.rs`,
//! `page_error.rs`), effects out to the engine and the shell. The visible tab's surfaces are drawn by
//! [`Host::render_surfaces`], the only writer of those Slint properties.

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::dpi::{LogicalPosition, LogicalSize};
use slint::winit_030::winit::event::Ime as WinitIme;
use std::path::PathBuf;

use wubble_core::model::Command;
use wubble_core::tab::TabId;
use wubble_core::zoom;
use wubble_engine::app::ViewId;
use wubble_engine::page::{PageEvent, Rect, RequestId};
use wubble_ui::{
    AppWindow, DialogView, ErrorView, FindView, MenuEntry, MenuView, PermissionView, PopupView,
};

use super::{Host, new_tab, now_ms};
use crate::chooser::{self, DialogSpec};
use crate::ime::{self, ImeCall, ImeInput};
use crate::menu::{self, Command as MenuCommand, Menu, MenuEffect, MenuEvent, NavFlags};
use crate::page_error::{self, ErrorEffect, ErrorEvent, PageError};
use crate::prompt::{self, Auth, AuthEvent, PermissionEvent, PermissionPrompt, Secret};
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
        let seen = self.checks.page_smoke.is_some().then(|| event.clone());
        let visible = self.model.visible_tab() == Some(tab);
        let routed = surface::route(event);
        // A page beside the focused one that asks something takes the focus, so its question
        // shows instead of blocking it unseen.
        if !visible
            && self.model.visible_tabs().contains(&tab)
            && matches!(
                routed,
                Routed::Dialog(DialogEvent::Requested(_)) | Routed::Auth(AuthEvent::Requested(_))
            )
        {
            self.dispatch(Command::ActivateTab(tab));
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
                    });
                self.apply_menu(tab, MenuEvent::Requested { id, menu, nav });
            }
            Routed::Menu(e) => {
                self.apply_menu(tab, e);
            }
            Routed::Find(e) => self.apply_find(tab, e),
            Routed::Popup(rect) => self.surfaces.entry(tab).or_default().popup = rect,
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
                self.apply_screen(window, ScreenEvent::PageLeft(tab));
            }
            Routed::Muted(muted) => {
                self.audio.entry(tab).or_default().muted = muted;
                self.after_change();
            }
            Routed::Audible(audible) => {
                self.audio.entry(tab).or_default().audible = audible;
                self.after_change();
            }
            Routed::ImeBounds(rect) if visible => {
                place_candidates(window, self.focus_origin(window), rect)
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
            Routed::Console(line) => log::debug!(target: "wubble::console", "{line}"),
            Routed::ImeBounds(_) | Routed::Ignore => {}
        }
        self.render_surfaces(window);
        // After the event took effect, so the smoke acts on the open surface.
        if let Some(event) = seen {
            self.page_smoke_step(crate::page_smoke::Input::Page(&event));
        }
    }

    /// Runs one dialog event; whether an answer reached the engine.
    pub(super) fn apply_dialog(&mut self, tab: TabId, event: DialogEvent) -> Option<bool> {
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
            if let Some(view) = self.tab_view(tab) {
                view.answer_permission(answer.id, &answer.granted);
            }
        }
    }

    pub(super) fn apply_auth(&mut self, tab: TabId, event: AuthEvent) {
        let s = self.surfaces.entry(tab).or_default();
        let (next, answer) = prompt::auth(std::mem::take(&mut s.auth), event);
        s.auth = next;
        if let Some(answer) = answer
            && let Some(view) = self.tab_view(tab)
        {
            let credentials = answer
                .credentials
                .as_ref()
                .map(|(user, Secret(password))| (user.as_str(), password.as_str()));
            view.answer_auth(answer.id, credentials);
        }
    }

    /// A certificate error: proceeds at once past one the user allowed this
    /// session; otherwise the panel holds the request for the user.
    fn certificate(&mut self, tab: TabId, id: RequestId, url: String, code: i32) {
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
                    self.dispatch(new_tab(url, Some(tab)));
                }
                MenuEffect::Run(MenuCommand::CopyText(text)) => self.copy(&text),
                MenuEffect::Run(MenuCommand::SplitWith(t)) => self.dispatch(Command::SplitWith(t)),
                MenuEffect::Run(MenuCommand::Unsplit(t)) => self.dispatch(Command::Unsplit(t)),
                MenuEffect::Run(MenuCommand::OpenBeside) => self.open_beside(),
                MenuEffect::Run(MenuCommand::Perform(action)) => self.perform(action),
                MenuEffect::Run(MenuCommand::Engine(_) | MenuCommand::Separator) => {}
            }
        }
        answered
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
    /// answered every held request), its fullscreen and sound end.
    pub(super) fn view_gone(&mut self, tab: TabId) {
        if let Some(s) = self.surfaces.remove(&tab) {
            self.surfaces.insert(tab, s.view_gone());
        }
        self.downloads_lost(tab);
        self.audio.remove(&tab);
        if let Some(window) = self.window.upgrade() {
            self.apply_screen(&window, ScreenEvent::PageLeft(tab));
        }
    }

    /// The tab stopped being the visible one: its menu is dismissed (menus
    /// are transient), its page leaves fullscreen. Dialogs and the find bar
    /// wait for the tab's return.
    fn left_visible(&mut self, window: &AppWindow, tab: TabId) {
        if matches!(
            self.surfaces.get(&tab).map(|s| &s.menu),
            Some(Menu::Open { .. } | Menu::Shell { .. })
        ) {
            self.apply_menu(tab, MenuEvent::Dismiss);
        }
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
        let tab = self.model.visible_tab();
        if self.drawn.tab != tab
            && let Some(old) = self.drawn.tab
        {
            self.drawn.tab = tab;
            self.left_visible(window, old);
        }
        let current = tab
            .and_then(|t| self.surfaces.get(&t))
            .cloned()
            .unwrap_or_default();
        if self.drawn.tab != tab || self.drawn.menu != current.menu {
            let scale = window.window().scale_factor();
            window.set_menu(menu_view(&current.menu, scale, self.focus_origin(window)));
        }
        if self.drawn.tab != tab || self.drawn.permission != current.permission {
            window.set_permission(permission_view(&current.permission));
        }
        // The sign-in sheet blocks the load; a JS dialog waits behind it.
        window.set_dialog(match &current.auth {
            Auth::Open(r) => auth_view(r),
            Auth::None => dialog_view(&current.dialog),
        });
        window.set_page_error(error_view(&current.error));
        window.set_find(find_view(&current.find, self.find_focus));
        window.set_popup(popup_view(window, self.focus_origin(window), current.popup));
        if current.popup.is_none() {
            window.set_popup_frame(slint::Image::default());
        }
        window.set_page_fullscreen(matches!(self.screen, Screen::Page { .. }));
        self.drawn = Drawn {
            tab,
            menu: current.menu,
            permission: current.permission,
        };
    }

    // ----- intents from the shell (the visible tab's surfaces)

    /// The sheet's answer: the sign-in sheet's when one shows (`text` is
    /// the user name), else the JS dialog's.
    pub fn dialog_answered(&mut self, accept: bool, text: &str, secret: &str) {
        let Some(tab) = self.model.visible_tab() else {
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
                    password: Secret(secret.into()),
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
        if let Some(tab) = self.model.visible_tab() {
            self.apply_permission(tab, PermissionEvent::Answer { allow, remember });
            self.redraw_surfaces();
        }
    }

    /// The error panel's default button: Go back on a certificate panel,
    /// else Reload.
    pub fn error_primary(&mut self) {
        let Some(tab) = self.model.visible_tab() else {
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
        if let Some(tab) = self.model.visible_tab() {
            self.apply_error(tab, ErrorEvent::Proceed);
            self.redraw_surfaces();
        }
    }

    pub fn menu_chosen(&mut self, index: usize) {
        self.menu_event(MenuEvent::Chosen(index));
    }

    /// Whether exactly one answer reached the engine (the page smoke checks).
    pub fn menu_dismissed(&mut self) -> bool {
        self.menu_event(MenuEvent::Dismiss) == 1
    }

    fn menu_event(&mut self, event: MenuEvent) -> usize {
        let Some(tab) = self.model.visible_tab() else {
            return 0;
        };
        let answered = self.apply_menu(tab, event);
        self.redraw_surfaces();
        answered
    }

    pub fn find_event(&mut self, event: FindEvent) {
        if let Some(tab) = self.model.visible_tab() {
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
        if let Some(window) = self.window.upgrade() {
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
            let table = std::mem::take(&mut self.site_zoom);
            self.site_zoom = zoom::next_table(table, &host, percent);
            self.store.set_site_zoom(self.site_zoom.clone());
        }
        if let Some(window) = self.window.upgrade() {
            self.toast(&window, &format!("{percent}%"));
        }
    }

    /// A tab committed `url`: show it at the zoom stored for its site.
    pub(super) fn restore_zoom(&mut self, tab: TabId, url: &str) {
        let want = zoom::wanted(&self.site_zoom, url);
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
        if let (Some(call), Some(view)) = (call, self.view()) {
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
        self.toast_serial = self.toast_serial.wrapping_add(1);
        window.set_toast_text(text.into());
        window.set_toast_serial(self.toast_serial);
    }

    pub(super) fn copy(&self, text: &str) {
        if let Some(window) = self.window.upgrade() {
            window.invoke_copy_text(text.into());
        }
    }
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
        x: origin.0 + at.0 as f32 / scale,
        y: origin.1 + at.1 as f32 / scale,
        entries: model(
            entries
                .iter()
                .map(|e| MenuEntry {
                    label: e.label.as_str().into(),
                    enabled: e.enabled,
                    separator: e.command == MenuCommand::Separator,
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
            let (title, lines) = prompt::permission_text(r);
            PermissionView {
                open: true,
                title: title.into(),
                lines: model(lines.into_iter().map(Into::into).collect()),
                // Unique per request, so a new prompt resets the toggle.
                serial: i32::try_from(r.id.0).unwrap_or(i32::MAX),
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
