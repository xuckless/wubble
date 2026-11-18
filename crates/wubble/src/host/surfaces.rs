//! The host's half of the page's own UI: page events in, through the pure
//! machines (`surface.rs`, `menu.rs`, `screen.rs`, `ime.rs`), effects out
//! to the engine and the shell. The visible tab's surfaces are drawn by
//! [`Host::render_surfaces`], the only writer of those Slint properties.

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::dpi::{LogicalPosition, LogicalSize};
use slint::winit_030::winit::event::Ime as WinitIme;
use wubble_core::tab::TabId;
use wubble_core::zoom;
use wubble_engine::app::ViewId;
use wubble_engine::page::{PageEvent, Rect};
use wubble_ui::{AppWindow, DialogView, FindView, MenuEntry, MenuView, PopupView};

use super::{Host, new_tab};
use crate::ime::{self, ImeCall, ImeInput};
use crate::menu::{self, Command as MenuCommand, Menu, MenuEffect, MenuEvent, NavFlags};
use crate::screen::{self, Screen, ScreenEffect, ScreenEvent};
use crate::surface::{self, Dialog, DialogEvent, FindBar, FindEffect, FindEvent, Routed};

/// What the shell last drew: the tab, and its menu. Slint skips setting a
/// property to an equal value, but a menu's rows are a fresh `ModelRc`
/// (equal only by pointer), and a rewritten model restarts the rows' motion.
#[derive(Default)]
pub(super) struct Drawn {
    tab: Option<TabId>,
    menu: Menu,
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
        match surface::route(event) {
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
            Routed::ImeBounds(rect) if visible => place_candidates(window, rect),
            Routed::Decline(request) => {
                if let Some(v) = self.tab_view(tab) {
                    v.decline(&request);
                }
            }
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
            Some(Menu::Open { .. })
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
            window.set_menu(menu_view(&current.menu, scale));
        }
        window.set_dialog(dialog_view(&current.dialog));
        window.set_find(find_view(&current.find, self.find_focus));
        window.set_popup(popup_view(window, current.popup));
        if current.popup.is_none() {
            window.set_popup_frame(slint::Image::default());
        }
        window.set_page_fullscreen(matches!(self.screen, Screen::Page { .. }));
        self.drawn = Drawn {
            tab,
            menu: current.menu,
        };
    }

    // ----- intents from the shell (the visible tab's surfaces)

    pub fn dialog_answered(&mut self, accept: bool, text: &str) {
        let Some(tab) = self.model.visible_tab() else {
            return;
        };
        let event = if accept {
            DialogEvent::Accept(text.into())
        } else {
            DialogEvent::Cancel
        };
        self.apply_dialog(tab, event);
        self.redraw_surfaces();
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
/// only while composing; physical pixels relative to the view).
fn place_candidates(window: &AppWindow, rect: Rect) {
    let scale = f64::from(window.window().scale_factor());
    let (x, y) = (
        f64::from(window.get_content_x()),
        f64::from(window.get_content_y()),
    );
    let at = LogicalPosition::new(x + f64::from(rect.x) / scale, y + f64::from(rect.y) / scale);
    let size = LogicalSize::new(
        f64::from(rect.width) / scale,
        f64::from(rect.height) / scale,
    );
    window
        .window()
        .with_winit_window(|w| w.set_ime_cursor_area(at, size));
}

fn model<T: Clone + 'static>(items: Vec<T>) -> slint::ModelRc<T> {
    slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(items)))
}

fn menu_view(menu: &Menu, scale: f32) -> MenuView {
    match menu {
        Menu::Closed => MenuView::default(),
        Menu::Open { at, entries, .. } => MenuView {
            open: true,
            x: at.0 as f32 / scale,
            y: at.1 as f32 / scale,
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
        },
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
                input: d.default_text.as_str().into(),
                // Unique per request, so a new dialog resets the sheet.
                serial: i32::try_from(d.id.0).unwrap_or(i32::MAX),
            }
        }
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

/// The popup in window coordinates, kept inside the window. Input maps
/// back through `origin`, the engine's own rect: CEF hit-tests the popup
/// there, wherever the layer was moved to fit.
fn popup_view(window: &AppWindow, popup: Option<Rect>) -> PopupView {
    let Some(r) = popup else {
        return PopupView::default();
    };
    let scale = window.window().scale_factor();
    let size = window.window().size();
    let physical = |logical: f32| (logical * scale).round() as i32;
    let at = Rect {
        x: physical(window.get_content_x()) + r.x,
        y: physical(window.get_content_y()) + r.y,
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
