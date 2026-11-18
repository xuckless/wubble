//! The host's half of the settings page (wubble://settings): every setting the host changes is
//! one write ([`Host::write_setting`]) through the store thread into the stored rows, and one
//! live effect ([`Host::apply_setting`], from `apply::effect_of`). Start-up replays the stored
//! rows through the same effect, so a setting applies the same way read or changed.
//!
//! The stored rows here are the one owner of every setting's text while the app runs; the
//! typed copies the host keeps (engine, keymap, zoom table, ...) are what `apply_setting` last
//! made of them. The page is drawn from them each time they or the model change.

use std::collections::HashMap;

use wubble_core::action::Action;
use wubble_core::apply::{self, Applied};
use wubble_core::keys::{self, Capture, CaptureEvent, Chord, Override, Platform};
use wubble_core::model::Command;
use wubble_core::pages::{self, InternalPage};
use wubble_core::seam::SidebarMode;
use wubble_core::settings as defs;
use wubble_core::space::{Rgb, SpaceIcon, SpaceId};
use wubble_store::{KeybindingOverrides, SiteZoom, text_of};
use wubble_ui::{AboutView, AppWindow, CaptureView, SpaceSetting};

use super::{Host, now_ms};
use crate::input;

/// What the settings page holds of its own.
#[derive(Debug)]
pub(super) struct SettingsPage {
    /// Every stored setting row, `key -> text`.
    pub stored: HashMap<String, String>,
    /// The search box.
    query: String,
    /// Recording a new chord in the keybinding editor.
    capture: Capture,
}

impl SettingsPage {
    pub fn new(stored: HashMap<String, String>) -> Self {
        Self {
            stored,
            query: String::new(),
            capture: Capture::Idle,
        }
    }

    /// The stored keybinding overrides.
    fn overrides(&self) -> Vec<Override> {
        let entries = match apply::effect_of(defs::KEYBINDING_OVERRIDES.key, &self.stored) {
            Some(Applied::Keymap(entries)) => entries,
            _ => vec![],
        };
        keys::overrides_from_entries(&entries).0
    }
}

/// The About section: this build, the engine it embeds, the licence and the notices.
fn about() -> AboutView {
    let (cef, chromium) = wubble_engine::app::cef_and_chromium();
    AboutView {
        version: env!("CARGO_PKG_VERSION").into(),
        cef: cef.into(),
        chromium: chromium.into(),
        licence: "GPL-3.0-only".into(),
        notices: include_str!("../../../../THIRD_PARTY_NOTICES.md").into(),
    }
}

fn rgb(c: slint::Color) -> Rgb {
    Rgb(c.red(), c.green(), c.blue())
}

impl Host {
    /// Applies every setting as stored: the start of the run.
    pub(super) fn replay_settings(&mut self) {
        for def in defs::ALL {
            if let Some(applied) = apply::effect_of(def.key, &self.settings.stored) {
                self.apply_setting(applied);
            }
        }
    }

    /// The one place a setting takes effect while the app runs.
    fn apply_setting(&mut self, applied: Applied) {
        match applied {
            Applied::SearchEngine(engine) => self.search_engine = engine,
            Applied::LiveSuggestions(on) => self.live_suggestions = on,
            Applied::SidebarMode(mode) => {
                if let Some(window) = self.window.upgrade()
                    && SidebarMode::from(window.get_sidebar_mode()) != mode
                {
                    window.invoke_sidebar_mode_requested(mode.into());
                }
            }
            Applied::BlockerEnabled(on) => self.blocking.set_enabled(on),
            Applied::BlockerExceptions(hosts) => self.blocking.set_exceptions(&hosts),
            Applied::ExpireHistory => self.store.expire_history(now_ms()),
            Applied::ZoomDefault(percent) => {
                self.zoom_default = percent;
                self.rezoom();
            }
            Applied::SiteZoom(table) => {
                self.site_zoom = table;
                self.rezoom();
            }
            Applied::Keymap(entries) => self.keymap = input::keymap(Platform::current(), &entries),
            Applied::Sleep(policy) => {
                if policy != self.model.policy() {
                    self.dispatch(Command::SetSleepPolicy(policy));
                }
            }
            Applied::Glass(setting) => self.glass.borrow_mut().set_setting(setting),
            Applied::RestartOnly => {}
        }
    }

    /// Every live page at the zoom the settings now ask for.
    fn rezoom(&mut self) {
        let tabs: Vec<_> = self.views.keys().copied().collect();
        for tab in tabs {
            if let Some(url) = self.committed.get(&tab).cloned() {
                self.restore_zoom(tab, &url);
            }
        }
    }

    /// Writes one setting: refused unless `key` is a setting and `text` a value of it; then
    /// stored, applied and shown.
    pub(crate) fn write_setting(&mut self, key: &str, text: &str) -> bool {
        if !self.store.set_setting(key, text) {
            return false;
        }
        self.settings.stored.insert(key.to_owned(), text.to_owned());
        if let Some(applied) = apply::effect_of(key, &self.settings.stored) {
            self.apply_setting(applied);
        }
        self.render_settings();
        true
    }

    pub(crate) fn setting_reset(&mut self, key: &str) {
        if let Some(default) = apply::default_of(key) {
            self.write_setting(key, default);
        }
    }

    pub(crate) fn setting_entry_removed(&mut self, key: &str, entry: &str) {
        if let Some(text) = apply::without_entry(key, &self.settings.stored, entry) {
            self.write_setting(key, &text);
        }
    }

    pub(crate) fn setting_entry_added(&mut self, key: &str, entry: &str) {
        if let Some(text) = apply::with_entry(key, &self.settings.stored, entry) {
            self.write_setting(key, &text);
        }
    }

    pub(crate) fn settings_query(&mut self, query: &str) {
        query.clone_into(&mut self.settings.query);
        self.render_settings();
    }

    // ----- per-site data

    /// The zoom table without `host`'s level.
    pub(crate) fn site_zoom_removed(&mut self, host: &str) {
        let mut table = self.site_zoom.clone();
        if table.remove(host).is_some() {
            self.write_setting(defs::SITE_ZOOM.key, &text_of::<SiteZoom>(&table));
        }
    }

    /// Forgets the remembered decision at `index` of the Sites list.
    pub(crate) fn site_permission_removed(&mut self, index: &str) {
        let view = pages::sites_view(&self.site_zoom, &self.site_permissions);
        let Some((origin, kind, _)) = index
            .parse()
            .ok()
            .and_then(|i: usize| view.permissions.get(i))
        else {
            return;
        };
        self.site_permissions.forget(origin, *kind);
        self.store.forget_site_permission(origin, *kind);
        self.render_settings();
    }

    // ----- keybindings

    /// The stored overrides after `f`, written as the one entry the editor keeps.
    fn write_overrides(&mut self, f: impl FnOnce(&[Override]) -> Vec<Override>) {
        let next = f(&self.settings.overrides());
        let text = text_of::<KeybindingOverrides>(&keys::overrides_to_entries(&next));
        self.write_setting(defs::KEYBINDING_OVERRIDES.key, &text);
    }

    /// One step of the chord recorder; a finished recording is stored.
    pub(crate) fn capture_event(&mut self, event: CaptureEvent) {
        let (state, bind) = keys::capture(self.settings.capture, event, &self.keymap);
        self.settings.capture = state;
        match bind {
            Some((action, chord)) => self
                .write_overrides(|ov| keys::set_binding(ov, Platform::current(), action, chord).0),
            None => self.render_settings(),
        }
    }

    pub(crate) fn binding_record(&mut self, id: &str) {
        if let Some(action) = Action::from_id(id) {
            self.capture_event(CaptureEvent::Arm(action));
        }
    }

    pub(crate) fn binding_reset(&mut self, id: &str) {
        if let Some(action) = Action::from_id(id) {
            self.write_overrides(|ov| keys::reset_binding(ov, Platform::current(), action));
        }
    }

    /// While a chord is being recorded every key goes to the recorder first, before any
    /// shortcut: Escape cancels, a chord is taken. Returns whether the key was taken.
    pub(crate) fn capture_key(&mut self, chord: Option<Chord>) -> bool {
        if self.settings.capture == Capture::Idle {
            return false;
        }
        match chord {
            Some(c) if c.key == keys::Key::Escape && !c.ctrl && !c.alt && !c.cmd => {
                self.capture_event(CaptureEvent::Cancel)
            }
            Some(c) => self.capture_event(CaptureEvent::Pressed(c)),
            None => {}
        }
        true
    }

    // ----- Spaces

    pub(crate) fn space_renamed(&mut self, id: i32, name: &str) {
        let name = name.trim();
        if let (Ok(id), false) = (u32::try_from(id), name.is_empty()) {
            self.dispatch(Command::RenameSpace {
                space: SpaceId(id),
                name: name.to_owned(),
            });
        }
    }

    pub(crate) fn space_tinted(&mut self, id: i32, tint: slint::Color) {
        if let Ok(id) = u32::try_from(id) {
            self.dispatch(Command::SetSpaceTint {
                space: SpaceId(id),
                tint: rgb(tint),
            });
        }
    }

    pub(crate) fn space_icon_chosen(&mut self, id: i32, icon: &str) {
        if let (Ok(id), Some(icon)) = (u32::try_from(id), SpaceIcon::from_id(icon)) {
            self.dispatch(Command::SetSpaceIcon {
                space: SpaceId(id),
                icon,
            });
        }
    }

    // ----- the page

    /// Whether the visible tab shows the settings page.
    fn settings_shown(&self) -> bool {
        self.model
            .visible_tab()
            .and_then(|t| self.model.tabs.get(&t))
            .and_then(|t| InternalPage::from_url(&t.url))
            == Some(InternalPage::Settings)
    }

    /// Draws the page from the stored rows, the keymap, the model and the per-site data;
    /// closed (and any recording cancelled) while another tab is shown.
    pub(super) fn render_settings(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let open = self.settings_shown();
        window.set_settings_open(open);
        if !open {
            self.settings.capture = Capture::Idle;
            return;
        }
        render_page(&window, self);
    }
}

/// The page's models, each synced in place (`crate::bind::sync`), so a row keeps its
/// instance, focus and edit across a re-render.
fn render_page(window: &AppWindow, host: &Host) {
    let page = &host.settings;
    let mut stored = page.stored.clone();
    // The shell owns the sidebar's mode while the app runs.
    let mode = SidebarMode::from(window.get_sidebar_mode());
    stored.insert(defs::SIDEBAR_MODE.key.into(), mode.id().into());
    let sections = pages::filter(&pages::settings_page(&stored), &page.query);
    let rows = wubble_ui::settings::setting_items(&sections);
    crate::bind::sync(window.get_settings_rows(), rows, |m| {
        window.set_settings_rows(m)
    });

    let needle = page.query.trim().to_lowercase();
    window.set_settings_searching(!needle.is_empty());
    let overrides = page.overrides();
    let bindings: Vec<_> = keys::binding_rows(Platform::current(), &overrides)
        .into_iter()
        .filter(|r| {
            needle.is_empty()
                || r.action.label().to_lowercase().contains(&needle)
                || r.chord
                    .is_some_and(|c| c.to_string().to_lowercase().contains(&needle))
        })
        .collect();
    crate::bind::sync(
        window.get_settings_bindings(),
        wubble_ui::settings::binding_items(&bindings),
        |m| window.set_settings_bindings(m),
    );
    window.set_settings_capture(match page.capture {
        Capture::Idle => CaptureView::default(),
        Capture::Armed(action) => CaptureView {
            action: action.id().into(),
            armed: true,
            ..Default::default()
        },
        Capture::Conflict {
            action,
            chord,
            other,
        } => CaptureView {
            action: action.id().into(),
            armed: false,
            conflict: true,
            chord: chord.to_string().into(),
            other: other.label().into(),
        },
    });

    let spaces = host
        .model
        .spaces
        .iter()
        .filter_map(|s| {
            Some(SpaceSetting {
                id: i32::try_from(s.id.0).ok()?,
                name: s.name.as_str().into(),
                tint: slint::Color::from_rgb_u8(s.tint.0, s.tint.1, s.tint.2),
                icon: s.icon.id().into(),
            })
        })
        .collect();
    crate::bind::sync(window.get_settings_spaces(), spaces, |m| {
        window.set_settings_spaces(m)
    });

    let sites = pages::sites_view(&host.site_zoom, &host.site_permissions);
    let strings =
        |v: Vec<String>| -> Vec<slint::SharedString> { v.into_iter().map(Into::into).collect() };
    let zoom_lines = sites
        .zoom
        .iter()
        .map(|(h, p)| format!("{h}: {p}%"))
        .collect();
    let zoom_keys = sites.zoom.iter().map(|(h, _)| h.clone()).collect();
    let perm_lines = sites
        .permissions
        .iter()
        .map(|(o, k, d)| format!("{o}: {} ({d:?})", k.ask_text()))
        .collect();
    let perm_keys = (0..sites.permissions.len())
        .map(|i| i.to_string())
        .collect();
    crate::bind::sync(window.get_settings_site_zoom(), strings(zoom_lines), |m| {
        window.set_settings_site_zoom(m)
    });
    crate::bind::sync(
        window.get_settings_site_zoom_keys(),
        strings(zoom_keys),
        |m| window.set_settings_site_zoom_keys(m),
    );
    crate::bind::sync(
        window.get_settings_site_permissions(),
        strings(perm_lines),
        |m| window.set_settings_site_permissions(m),
    );
    crate::bind::sync(
        window.get_settings_site_permission_keys(),
        strings(perm_keys),
        |m| window.set_settings_site_permission_keys(m),
    );
    if window.get_settings_about().version.is_empty() {
        window.set_settings_about(about());
    }
}
