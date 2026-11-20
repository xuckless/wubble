//! The extensions' shell: the installed list in Settings, the footer button's popover with each
//! enabled extension's action, the action's popup, the install prompt and the options page. The
//! rules are `wubble_core::extensions::shell` (the prompt's and the popover's machines, the
//! popup's size, the words); this executes their effects and is the one writer of the window's
//! extension properties ([`Host::render_extensions`]).
//!
//! The list is `ExtUi::list`, the one owner while running; every change is stored through
//! `crate::extensions::change` and applies at the next start (`restart_needed` is the page's
//! note). The action popup is a windowless view of the visible tab's container (where the
//! extension's action ran), kept as one more pane under the key `POPUP_TAB`, drawn in a kit
//! popover; its content size is polled by script while it is open.

use std::path::PathBuf;
use std::time::Duration;

use slint::ComponentHandle;
use wubble_core::extensions::shell::{
    Clicked, POPUP_MAX, POPUP_TAB, Panel, PanelEffect, PanelEvent, Prompt, PromptEffect,
    PromptEvent, options_url, panel_transition, prompt_text, prompt_transition, summary,
};
use wubble_core::extensions::{Event, ExtensionId, Installed, restart_needed};
use wubble_core::model::Command;
use wubble_core::overlay::Kind;
use wubble_core::space::ContainerId;
use wubble_engine::app::ViewId;
use wubble_engine::cdp::EvalId;
use wubble_engine::ext_api::{ActionClick, ActionState};
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::FrameSize;
use wubble_ui::{AppWindow, ExtensionAction, ExtensionItem, ExtensionPopup, ExtensionPrompt};

use crate::gallery_smoke::Overlay;

use super::Host;
use crate::{physical, slint_int, with_host};

/// How often an open popup's page is asked for its size.
const SIZE_POLL: Duration = Duration::from_millis(150);

/// The popup page's size, CSS px: its body's far edges with the body's margins (Chromium lays
/// the popup out as wide as it may and shrinks it to what the page uses; a body without a width
/// of its own fills the width, so the popup is as wide as it may be).
const SIZE_SCRIPT: &str = "(() => { const b = document.body; if (!b) return [0, 0]; \
    const r = b.getBoundingClientRect(), s = getComputedStyle(b); \
    return [Math.ceil(Math.max(r.right + parseFloat(s.marginRight), b.scrollWidth)), \
    Math.ceil(Math.max(r.bottom + parseFloat(s.marginBottom), b.scrollHeight))]; })()";

/// What the extension shell holds.
pub(super) struct ExtUi {
    /// The installed list.
    pub list: Installed,
    /// The list when CEF started.
    at_start: Installed,
    /// wubble's unpacked copies of .crx extensions (`Profile::extensions_dir`).
    root: PathBuf,
    panel: Panel,
    prompt: Prompt,
    /// The open popup's view.
    popup: Option<ViewId>,
    /// The question to the popup's page the next answer is for.
    size_eval: Option<EvalId>,
    timer: slint::Timer,
    /// Bumped per prompt.
    prompt_serial: i32,
    /// `--smoke gallery`: the real list kept while a scene shows stand-in extensions, and the
    /// stand-in actions' rows (a stock CEF has no actions to ask).
    gallery: Option<(Installed, Vec<ExtensionAction>)>,
}

impl ExtUi {
    pub fn new(list: Installed) -> Self {
        Self {
            at_start: list.clone(),
            list,
            root: PathBuf::new(),
            panel: Panel::Closed,
            prompt: Prompt::Closed,
            popup: None,
            size_eval: None,
            timer: slint::Timer::default(),
            prompt_serial: 0,
            gallery: None,
        }
    }

    /// The popup's pane is on screen: its view is open and its page has a size.
    fn popup_size(&self) -> Option<(u32, u32)> {
        match self.panel {
            Panel::Popup { size, .. } if self.popup.is_some() => size,
            _ => None,
        }
    }
}

impl Host {
    /// Where unpacked copies of .crx files go (set once the profile is known).
    pub(crate) fn set_extensions_root(&mut self, root: PathBuf) {
        self.ext_ui.root = root;
    }

    // ----- Settings: the list

    /// "Install from file": a .crx file.
    pub(crate) fn extension_install_file(&mut self) {
        self.extension_pick(crate::chooser::Pick::File, "Add an extension", &["crx"]);
    }

    /// "Install from folder": an unpacked extension.
    pub(crate) fn extension_install_folder(&mut self) {
        self.extension_pick(
            crate::chooser::Pick::Folder,
            "Add an unpacked extension",
            &[],
        );
    }

    fn extension_pick(&mut self, pick: crate::chooser::Pick, title: &str, extensions: &[&str]) {
        use crate::chooser::{self, DialogSpec};
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let spec = DialogSpec {
            pick,
            title: title.into(),
            directory: None,
            file_name: None,
            filters: if extensions.is_empty() {
                vec![]
            } else {
                vec![(
                    "Extensions".into(),
                    extensions.iter().map(|e| (*e).to_owned()).collect(),
                )]
            },
        };
        let dialog = chooser::dialog(&spec, Some(window.window()));
        let task = slint::spawn_local(async move {
            if let Some(path) = chooser::pick(spec, dialog)
                .await
                .and_then(|p| p.into_iter().next())
            {
                with_host(|host| host.extension_picked(path));
            }
        });
        if let Err(e) = task {
            eprintln!("wubble: cannot show the file dialog: {e}");
        }
    }

    /// The user picked `path`: read it, and ask what it may do before anything is installed.
    fn extension_picked(&mut self, path: PathBuf) {
        match crate::extensions::read(&path, &self.ext_ui.root) {
            Ok(ext) => {
                if self.overlay_open(Kind::ExtensionSheet, None) {
                    self.prompt_event(PromptEvent::Ask(ext));
                }
            }
            Err(e) => self.extension_failed(&e.to_string()),
        }
    }

    fn extension_failed(&mut self, why: &str) {
        eprintln!("wubble: extension: {why}");
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, &format!("Could not add the extension: {why}"));
        }
    }

    /// The install prompt's Add extension (`true`) or Cancel.
    pub(crate) fn extension_prompt_answered(&mut self, accept: bool) {
        self.prompt_event(if accept {
            PromptEvent::Allow
        } else {
            PromptEvent::Cancel
        });
    }

    fn prompt_event(&mut self, event: PromptEvent) {
        if matches!(event, PromptEvent::Ask(_)) {
            self.ext_ui.prompt_serial = self.ext_ui.prompt_serial.wrapping_add(1);
        }
        let (prompt, effects) = prompt_transition(std::mem::take(&mut self.ext_ui.prompt), event);
        self.ext_ui.prompt = prompt;
        for effect in effects {
            match effect {
                PromptEffect::Install(ext) => self.extension_change(Event::Install(ext)),
            }
        }
        self.render_extensions();
    }

    fn extension_change(&mut self, event: Event) {
        let list = std::mem::take(&mut self.ext_ui.list);
        match crate::extensions::change(&self.store, list.clone(), event) {
            Ok(list) => self.ext_ui.list = list,
            Err(e) => {
                self.ext_ui.list = list;
                self.extension_failed(&e.to_string());
            }
        }
        self.render_extensions();
    }

    pub(crate) fn extension_toggled(&mut self, id: &str, on: bool) {
        if let Some(id) = ExtensionId::parse(id) {
            self.extension_change(if on {
                Event::Enable(id)
            } else {
                Event::Disable(id)
            });
        }
    }

    pub(crate) fn extension_removed(&mut self, id: &str) {
        if let Some(id) = ExtensionId::parse(id) {
            self.extension_change(Event::Remove(id));
        }
    }

    /// An extension's options page, as a normal tab (in the extension's own profile).
    pub(crate) fn extension_options(&mut self, id: &str) {
        let url = ExtensionId::parse(id)
            .and_then(|id| self.ext_ui.list.get(&id))
            .and_then(options_url);
        if let Some(url) = url {
            self.dispatch(Command::ExtensionTab {
                url,
                container: ContainerId::DEFAULT,
                space: None,
                show: true,
            });
        }
    }

    // ----- the footer button

    /// The footer's puzzle button: the popover opens when the overlays let it, or closes.
    pub fn extensions_requested(&mut self) {
        self.hover_dismiss();
        if self.ext_ui.panel == Panel::Closed && !self.overlay_open(Kind::Extensions, None) {
            return;
        }
        self.panel_event(PanelEvent::Toggle);
    }

    /// A press on an extension's row: its action runs on the visible tab.
    pub fn extension_pressed(&mut self, id: &str) {
        let Some(view) = self.shown().and_then(|t| self.views.get(&t)).copied() else {
            return;
        };
        let clicked = match self.engine.action_click(id, view) {
            ActionClick::None => Clicked::Nothing,
            ActionClick::Clicked => Clicked::Ran,
            ActionClick::Popup(url) => Clicked::Popup(url),
        };
        if let Some(id) = ExtensionId::parse(id) {
            self.panel_event(PanelEvent::Clicked(id, clicked));
        }
    }

    /// "Manage extensions": Settings, at the Extensions section's list.
    pub fn extensions_manage(&mut self) {
        self.panel_event(PanelEvent::Dismiss);
        self.dispatch(Command::OpenInternal(
            wubble_core::pages::InternalPage::Settings,
        ));
    }

    /// A press outside the popover or popup, or its Escape.
    pub fn extensions_dismissed(&mut self) {
        self.panel_event(PanelEvent::Dismiss);
    }

    pub(super) fn panel_event(&mut self, event: PanelEvent) {
        let (panel, effects) = panel_transition(std::mem::take(&mut self.ext_ui.panel), event);
        self.ext_ui.panel = panel;
        for effect in effects {
            match effect {
                PanelEffect::OpenPopup { url, .. } => {
                    if !self.open_popup(&url) {
                        self.extension_failed("its popup could not open");
                        self.ext_ui.panel = Panel::Closed;
                    }
                }
                PanelEffect::ClosePopup => self.close_popup(),
            }
        }
        self.render_extensions();
    }

    /// The action of an extension changed (badge, title, icon, popup): the popover is redrawn.
    pub(super) fn ext_action_changed(&mut self) {
        if self.ext_ui.panel == Panel::List {
            self.render_extensions();
        }
    }

    // ----- the popup's view

    /// Opens `url` in a view of the visible tab's container, as large as a popup may be until
    /// its page says what it needs; keys go to it. False when it cannot be (no window).
    fn open_popup(&mut self, url: &str) -> bool {
        let (Some(window), Some(frame_rate)) = (self.w.window.upgrade(), self.frame_rate) else {
            return false;
        };
        if self.ext_ui.popup.is_some() {
            return true;
        }
        let Some(tab) = self.shown() else {
            return false;
        };
        let container = self
            .model
            .tabs
            .get(&tab)
            .map_or(ContainerId::DEFAULT, |t| t.container);
        // the popup takes the peek's presenter, so the peek goes
        if self.model.glance_of(self.w.id).is_some() {
            self.dispatch(Command::CloseGlance(self.w.id));
        }
        let config = ViewConfig {
            url: url.to_owned(),
            layout: self.popup_layout(&window),
            frame_rate,
            begin_frames: self.pace.is_some(),
            gpu: self.w.presenters[0].context().clone(),
            container,
        };
        match self.engine.open_view(config) {
            Ok(id) => {
                self.ext_ui.popup = Some(id);
                self.key_pane = Some(POPUP_TAB);
                self.sync_focus(&window);
                self.ext_ui
                    .timer
                    .start(slint::TimerMode::Repeated, SIZE_POLL, || {
                        with_host(Host::popup_ask_size)
                    });
                true
            }
            Err(e) => {
                eprintln!("wubble: extension popup: {e}");
                false
            }
        }
    }

    fn close_popup(&mut self) {
        self.ext_ui.timer.stop();
        self.ext_ui.size_eval = None;
        if let Some(id) = self.ext_ui.popup.take() {
            self.engine.close_view(id);
        }
        if self.key_pane == Some(POPUP_TAB) {
            self.key_pane = None;
        }
        self.w.frames.remove(&POPUP_TAB);
        self.w.slots.iter_mut().for_each(|s| {
            if *s == Some(POPUP_TAB) {
                *s = None;
            }
        });
        if let Some(window) = self.w.window.upgrade() {
            self.sync_focus(&window);
        }
    }

    /// The popup's layout: its page's size once known, else the most a popup may be.
    pub(super) fn popup_layout(&self, window: &AppWindow) -> Layout {
        let (w, h) = self.ext_ui.popup_size().unwrap_or(POPUP_MAX);
        let scale = window.window().scale_factor();
        Layout {
            size: FrameSize::new(physical(w as f32, scale), physical(h as f32, scale)),
            scale,
            area: None,
        }
    }

    /// The popup's view when keys go to it: the last press was in it.
    pub(super) fn keys_to_popup(&self) -> Option<ViewId> {
        (self.key_pane == Some(POPUP_TAB))
            .then_some(self.ext_ui.popup)
            .flatten()
    }

    /// The install prompt is open (an overlay).
    pub(super) fn ext_ui_prompt_open(&self) -> bool {
        self.ext_ui.prompt != Prompt::Closed
    }

    /// The footer popover or an action's popup is open (an overlay).
    pub(super) fn ext_ui_panel_open(&self) -> bool {
        self.ext_ui.panel != Panel::Closed
    }

    /// `view` is the open popup's.
    pub(super) fn popup_view_is(&self, view: ViewId) -> bool {
        self.ext_ui.popup == Some(view)
    }

    /// The popup's pane is on screen (its frames are kept and its pointer is routed).
    pub(super) fn popup_open(&self) -> bool {
        self.ext_ui.popup.is_some()
    }

    /// The popup's own view.
    pub(super) fn popup_view(&mut self) -> Option<&mut wubble_engine::view::View> {
        let id = self.ext_ui.popup?;
        self.engine.view(id)
    }

    fn popup_ask_size(&mut self) {
        if self.ext_ui.size_eval.is_some() {
            return;
        }
        self.ext_ui.size_eval = self.popup_view().and_then(|v| v.eval(SIZE_SCRIPT));
    }

    /// The popup's page events: its answer about its size; every request it makes is refused
    /// (the engine waits on the host for them).
    pub(super) fn on_popup_page_event(&mut self, event: wubble_engine::page::PageEvent) {
        use wubble_engine::page::PageEvent;
        if let PageEvent::EvalResult { id, result } = &event {
            if self.ext_ui.size_eval == Some(*id) {
                self.ext_ui.size_eval = None;
                if let Some((w, h)) = result.as_deref().ok().and_then(parse_size) {
                    self.panel_event(PanelEvent::Sized(w, h));
                }
            }
            return;
        }
        // the same refusals as the DevTools panel's: nothing of the page's own UI applies
        if let Some(view) = self.popup_view() {
            super::devtools::refuse_requests(view, event);
        }
    }

    /// The popup's view events: a link it opens goes to a tab, `window.close()` closes it.
    pub(super) fn on_popup_web_event(&mut self, event: wubble_engine::nav::WebEvent) {
        use wubble_engine::nav::WebEvent;
        match event {
            WebEvent::PopupRequested(request) => {
                self.panel_event(PanelEvent::Dismiss);
                self.dispatch(super::new_tab(request.url, None));
            }
            WebEvent::KeyUnhandled(key) => {
                if let Some(action) = self.page_first.unhandled(key) {
                    self.perform(action);
                }
            }
            _ => {}
        }
    }

    /// The popup's page closed itself.
    pub(super) fn popup_view_gone(&mut self) {
        if self
            .ext_ui
            .popup
            .is_some_and(|id| !self.engine.has_view(id))
        {
            self.ext_ui.popup = None;
            self.panel_event(PanelEvent::PageClosed);
        }
    }

    // ----- `--smoke gallery`

    /// Puts the extension scenes' state in place, and takes the last scene's away: stand-in
    /// extensions in memory (never stored), then the popover or the prompt open through the
    /// overlays. A scene without extensions has the real list back.
    pub(super) fn gallery_extensions(&mut self, overlay: Overlay) {
        self.panel_event(PanelEvent::Dismiss);
        self.prompt_event(PromptEvent::Cancel);
        let seeded = matches!(
            overlay,
            Overlay::ExtensionsList | Overlay::ExtensionsPopover | Overlay::ExtensionPrompt
        );
        match (seeded, self.ext_ui.gallery.take()) {
            (true, None) => {
                let real = std::mem::take(&mut self.ext_ui.list);
                self.ext_ui.list = gallery_list();
                self.ext_ui.gallery = Some((real, gallery_rows()));
            }
            (true, kept) => self.ext_ui.gallery = kept,
            (false, Some((real, _))) => self.ext_ui.list = real,
            (false, None) => {}
        }
        match overlay {
            Overlay::ExtensionsPopover if self.overlay_open(Kind::Extensions, None) => {
                self.panel_event(PanelEvent::Toggle);
            }
            Overlay::ExtensionPrompt if self.overlay_open(Kind::ExtensionSheet, None) => {
                self.prompt_event(PromptEvent::Ask(gallery_asked()));
            }
            _ => self.render_extensions(),
        }
    }

    // ----- what the window shows

    /// The window's extension properties, from the list, the panel and the prompt: the one
    /// writer of them.
    pub(super) fn render_extensions(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        self.render_extensions_in(&window);
    }

    pub(super) fn render_extensions_in(&mut self, window: &AppWindow) {
        let ui = &self.ext_ui;
        let items: Vec<ExtensionItem> = ui
            .list
            .extensions()
            .iter()
            .map(|e| ExtensionItem {
                id: e.id.as_str().into(),
                name: e.name.as_str().into(),
                note: summary(e).into(),
                enabled: e.enabled,
                options: options_url(e).is_some(),
            })
            .collect();
        let restart = restart_needed(&ui.at_start, &ui.list);
        window.set_has_extensions(!items.is_empty());
        crate::bind::sync(window.get_settings_extensions(), items, |m| {
            window.set_settings_extensions(m)
        });
        window.set_settings_extensions_restart(restart);
        window.set_extensions_open(self.ext_ui.panel == Panel::List);
        if self.ext_ui.panel == Panel::List {
            let actions = self.extension_actions();
            crate::bind::sync(window.get_extension_actions(), actions, |m| {
                window.set_extension_actions(m)
            });
        }
        window.set_extension_prompt(match &self.ext_ui.prompt {
            Prompt::Asking(ext) => {
                let (title, message) = prompt_text(ext);
                ExtensionPrompt {
                    open: true,
                    title: title.into(),
                    message: message.into(),
                    serial: self.ext_ui.prompt_serial,
                }
            }
            // a sheet that closes keeps its words while it leaves
            Prompt::Closed => ExtensionPrompt {
                open: false,
                ..window.get_extension_prompt()
            },
        });
        self.render_popup(window);
    }

    /// The popover's rows: each enabled extension's action as the visible tab sees it.
    fn extension_actions(&self) -> Vec<ExtensionAction> {
        if let Some((_, rows)) = &self.ext_ui.gallery {
            return rows.clone();
        }
        let view = self.shown().and_then(|t| self.views.get(&t)).copied();
        self.ext_ui
            .list
            .extensions()
            .iter()
            .filter(|e| e.enabled)
            .map(|e| {
                let state = view.and_then(|v| self.engine.action_state(e.id.as_str(), v));
                action_row(e.id.as_str(), &e.name, e.dir(), state)
            })
            .collect()
    }

    /// The popup's card: its page's last frame at the size it asked for.
    pub(super) fn render_popup(&self, window: &AppWindow) {
        let view = match (self.ext_ui.popup_size(), self.w.frames.get(&POPUP_TAB)) {
            (Some((w, h)), frame) => ExtensionPopup {
                open: true,
                id: crate::shell::pane_id(POPUP_TAB),
                frame: frame.map(|f| f.image.clone()).unwrap_or_default(),
                clip_width: frame.map_or(0, |f| slint_int(f.clip.width)),
                clip_height: frame.map_or(0, |f| slint_int(f.clip.height)),
                width: w as f32,
                height: h as f32,
            },
            (None, _) => ExtensionPopup {
                open: false,
                ..window.get_extension_popup()
            },
        };
        let same = {
            let (a, b) = (window.get_extension_popup(), view.clone());
            let image = |p: &ExtensionPopup| p.frame.clone();
            wubble_ui::same_image(&image(&a), &image(&b))
                && (a.open, a.id, a.clip_width, a.clip_height, a.width, a.height)
                    == (b.open, b.id, b.clip_width, b.clip_height, b.width, b.height)
        };
        if !same {
            window.set_extension_popup(view);
        }
    }
}

/// The gallery's stand-in extensions, as installed.
fn gallery_list() -> Installed {
    let mut list = Installed::default();
    for (id, name, permissions) in [
        (
            "abcdefghijklmnopabcdefghijklmnop",
            "Page Notes",
            vec!["storage", "tabs", "https://*.example.com/*"],
        ),
        (
            "ponmlkjihgfedcbaponmlkjihgfedcba",
            "Tab Saver",
            vec!["storage"],
        ),
    ] {
        let ext = wubble_core::extensions::Extension {
            id: ExtensionId::parse(id).expect("a well-formed id"),
            name: name.into(),
            version: "1.4.2".into(),
            enabled: true,
            source: wubble_core::extensions::Source::Unpacked {
                dir: PathBuf::from(format!("/gallery/{name}")),
            },
            permissions: permissions.into_iter().map(str::to_owned).collect(),
            options_page: (name == "Page Notes").then(|| "options.html".to_owned()),
        };
        list = wubble_core::extensions::transition(list, Event::Install(ext)).0;
    }
    list
}

/// The popover's stand-in rows: one action with a badge and one that is off on the page.
fn gallery_rows() -> Vec<ExtensionAction> {
    let row = |id: &str, title: &str, badge: &str, usable: bool| ExtensionAction {
        id: id.into(),
        title: title.into(),
        icon: Default::default(),
        has_icon: false,
        badge: badge.into(),
        badge_bg: argb(0, BADGE_BG),
        badge_fg: argb(0, BADGE_FG),
        usable,
    };
    vec![
        row("abcdefghijklmnopabcdefghijklmnop", "Page Notes", "3", true),
        row("ponmlkjihgfedcbaponmlkjihgfedcba", "Tab Saver", "", false),
    ]
}

/// The extension the gallery's install prompt asks about.
fn gallery_asked() -> wubble_core::extensions::Extension {
    wubble_core::extensions::Extension {
        id: ExtensionId::parse("hgfedcbaponmlkjihgfedcbaponmlkji").expect("a well-formed id"),
        name: "Reading Helper".into(),
        version: "2.0.1".into(),
        enabled: true,
        source: wubble_core::extensions::Source::Unpacked {
            dir: PathBuf::from("/gallery/Reading Helper"),
        },
        permissions: [
            "storage",
            "tabs",
            "bookmarks",
            "https://*.example.com/*",
            "https://news.test/*",
        ]
        .map(str::to_owned)
        .to_vec(),
        options_page: None,
    }
}

/// `[w, h]` as the page's script answered, JSON text.
fn parse_size(text: &str) -> Option<(u32, u32)> {
    let [w, h]: [f64; 2] = serde_json::from_str(text).ok()?;
    let css = |v: f64| (v.is_finite() && v >= 0.0).then_some(v.min(f64::from(u32::MAX)) as u32);
    Some((css(w)?, css(h)?))
}

/// The default badge colours: Chromium's grey pill with white text, for an extension that set none.
const BADGE_BG: u32 = 0xFF5F_6368;
const BADGE_FG: u32 = 0xFFFF_FFFF;

fn argb(v: u32, default: u32) -> slint::Color {
    let v = if v == 0 { default } else { v };
    slint::Color::from_argb_encoded(v)
}

/// One extension's action row from the state its action has on the visible tab (`None`: no such
/// action there, so the row is dimmed and takes no press).
fn action_row(
    id: &str,
    name: &str,
    dir: &std::path::Path,
    state: Option<ActionState>,
) -> ExtensionAction {
    let usable = state.as_ref().is_some_and(|s| s.enabled);
    let state = state.unwrap_or_default();
    let icon = action_icon(&state, dir);
    ExtensionAction {
        id: id.into(),
        title: if state.title.is_empty() {
            name.into()
        } else {
            state.title.as_str().into()
        },
        has_icon: icon.is_some(),
        icon: icon.unwrap_or_default(),
        badge: state.badge.as_str().into(),
        badge_bg: argb(state.badge_bg, BADGE_BG),
        badge_fg: argb(state.badge_fg, BADGE_FG),
        usable,
    }
}

/// The action's picture: the PNG it set for the tab, else its manifest's default icon file
/// inside the extension's directory (a PNG; anything else shows the puzzle piece).
fn action_icon(state: &ActionState, dir: &std::path::Path) -> Option<slint::Image> {
    let bytes = if state.icon_png.is_empty() {
        let rel = state.icon_path.trim_start_matches('/');
        let plain = !rel.is_empty() && !rel.split('/').any(|part| part == "..");
        std::fs::read(dir.join(rel)).ok().filter(|_| plain)?
    } else {
        state.icon_png.clone()
    };
    let (w, h, rgba) = crate::shot_io::rgba(&bytes).ok()?;
    Some(crate::bind::favicon_image(w, h, &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_s_size_answer_is_two_numbers() {
        assert_eq!(parse_size("[320,480]"), Some((320, 480)));
        assert_eq!(parse_size("[320.5,10]"), Some((320, 10)));
        assert_eq!(parse_size("[-1,10]"), None);
        assert_eq!(parse_size("[1]"), None);
        assert_eq!(parse_size("null"), None);
        assert_eq!(parse_size("[1e400,2]"), None);
    }

    #[test]
    fn an_action_off_on_the_tab_is_dimmed_and_unset_colours_are_the_default() {
        let dir = std::path::Path::new("/x");
        let off = action_row("a", "Name", dir, None);
        assert!(!off.usable && !off.has_icon);
        assert_eq!(off.title, "Name");
        assert_eq!(off.badge_bg, slint::Color::from_argb_encoded(BADGE_BG));
        let on = action_row(
            "a",
            "Name",
            dir,
            Some(ActionState {
                title: "Mark this".into(),
                badge: "3".into(),
                badge_bg: 0xFF11_2233,
                enabled: true,
                ..Default::default()
            }),
        );
        assert!(on.usable);
        assert_eq!(on.title, "Mark this");
        assert_eq!(on.badge, "3");
        assert_eq!(on.badge_bg, slint::Color::from_argb_encoded(0xFF11_2233));
        assert_eq!(on.badge_fg, slint::Color::from_argb_encoded(BADGE_FG));
    }

    #[test]
    fn an_icon_path_cannot_leave_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let state = |p: &str| ActionState {
            icon_path: p.into(),
            ..Default::default()
        };
        assert!(action_icon(&state("../x.png"), dir.path()).is_none());
        assert!(action_icon(&state(""), dir.path()).is_none());
        assert!(action_icon(&state("missing.png"), dir.path()).is_none());
    }
}
