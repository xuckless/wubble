//! The host's half of the settings page (wubble://settings): every setting the host changes is
//! one write ([`Host::write_setting`]) through the store thread into the stored rows, and one
//! live effect ([`Host::apply_setting`], from `apply::effect_of`). Start-up replays the stored
//! rows through the same effect, so a setting applies the same way read or changed.
//!
//! The stored rows here are the one owner of every setting's text while the app runs; the
//! typed copies the host keeps (engine, keymap, zoom table, ...) are what `apply_setting` last
//! made of them. The page is drawn from them each time they or the model change.

use std::collections::HashMap;

use wubble_blocker::ListRow;
use wubble_blocker::catalog::Selection;
use wubble_core::action::Action;
use wubble_core::apply::{self, Applied};
use wubble_core::keys::{self, Capture, CaptureEvent, Chord, Override, Platform};
use wubble_core::model::Command;
use wubble_core::pages::{self, InternalPage};
use wubble_core::seam::SidebarMode;
use wubble_core::settings as defs;
use wubble_core::space::{Rgb, SpaceIcon, SpaceId};
use wubble_store::{KeybindingOverrides, SiteZoom, text_of};
use wubble_ui::{AboutView, AppWindow, CaptureView, FilterListItem, SpaceSetting};

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
    /// The answer to the last draft a list editor submitted: the setting, a serial that
    /// changes with each answer, and the refusal's note (empty: taken).
    answer: (String, i32, &'static str),
}

impl SettingsPage {
    pub fn new(stored: HashMap<String, String>) -> Self {
        Self {
            stored,
            query: String::new(),
            capture: Capture::Idle,
            answer: (String::new(), 0, ""),
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
                if let Some(window) = self.w.window.upgrade()
                    && SidebarMode::from(window.get_sidebar_mode()) != mode
                {
                    window.invoke_sidebar_mode_requested(mode.into());
                }
            }
            Applied::BlockerEnabled(on) => self.blocking.set_enabled(on),
            Applied::BlockerExceptions(hosts) => self.blocking.set_exceptions(&hosts),
            Applied::BlockerLists {
                lists,
                cookie_banners,
                strip_params,
                custom,
            } => {
                self.blocking.reselect(Selection {
                    lists,
                    cookie_banners,
                    strip_params,
                    custom,
                });
            }
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
            Applied::NeverSleepSites(sites) => {
                if sites != self.model.never_sites() {
                    self.dispatch(Command::SetNeverSleepSites(sites));
                }
            }
            Applied::ArchiveAfter(after) => self.archive_after = after,
            Applied::RoutingRules(rules) => self.routing_rules = rules,
            Applied::Glass(setting) => self.w.glass.borrow_mut().set_setting(setting),
            Applied::Passwords(settings) => self.autofill.set_settings(settings),
            Applied::Media(settings) => self.apply_media_settings(settings),
            Applied::PageThemes {
                default,
                sites,
                media,
            } => self.set_page_themes(default, sites, media),
            Applied::MinFontSize(px) => self.set_min_font_size(px),
            Applied::Reader(prefs) => self.set_reader_prefs(prefs),
            Applied::ScreenshotOutput(output) => self.set_shot_output(output),
            Applied::AskWhereToSave(on) => self.downloads.ask_where = on,
            Applied::HintsAlphabet(letters) => self.set_hint_alphabet(letters),
            Applied::Video {
                controls,
                rates,
                hidden,
                collapsed,
            } => self.set_video_settings(controls, rates, hidden, collapsed),
            Applied::MediaKeys(on) => self.set_media_keys(on),
            Applied::PipOnTabSwitch(on) => self.set_pip_auto(on),
            Applied::Sponsor {
                enabled,
                asked,
                acts,
            } => self.set_sponsor(enabled, asked, acts),
            Applied::BoostsEnabled(on) => self.set_boosts_enabled(on),
            Applied::ReduceMotion(calm) => self.set_calm(calm),
            Applied::WebAccessibility(setting) => self.set_web_ax(setting),
            Applied::TouchSizing(sizing) => self.set_touch_sizing(sizing),
            Applied::Glance(on) => self.glance_enabled = on,
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

    /// A list editor's "−": the entry it names leaves its setting. The Spaces list
    /// (`pages::SPACES_LIST`) names a Space by its id: it is deleted with its tabs (the list
    /// asked twice; the last Space stays).
    pub(crate) fn setting_entry_removed(&mut self, key: &str, entry: &str) {
        if key == pages::SPACES_LIST {
            if let Ok(id) = entry.parse::<u32>() {
                self.dispatch(Command::DeleteSpace(SpaceId(id)));
            }
            return;
        }
        if let Some(text) = apply::without_entry(key, &self.settings.stored, entry) {
            self.write_setting(key, &text);
        }
    }

    /// A list editor's draft (`pages::submission`: its cells, and the entry it replaces when it
    /// edits one) goes into its setting, or is refused with a note; either way the editor gets
    /// its answer. The Spaces list's "+" adds a Space (`Model::new_space`).
    pub(crate) fn setting_entry_added(&mut self, key: &str, entry: &str) {
        if key == pages::SPACES_LIST {
            let command = self.model.new_space();
            self.dispatch(command);
            return;
        }
        let draft = pages::submission(entry);
        let text = pages::entry_text(key, &draft.cells);
        let put = apply::put_entry(key, &self.settings.stored, draft.replacing, &text);
        let serial = self.settings.answer.1.wrapping_add(1);
        self.settings.answer = (
            key.to_owned(),
            serial,
            put.as_ref().err().map_or("", |r| r.note()),
        );
        match put {
            Ok(text) => {
                self.write_setting(key, &text);
            }
            Err(_) => self.render_settings(),
        }
    }

    /// The lists' state changed on the blocker's thread (a rebuild or an update ended).
    pub(crate) fn filter_lists_changed(&mut self) {
        self.render_settings();
    }

    /// A catalog list's switch: its entry in `blocker.lists`.
    pub(crate) fn filter_list_toggled(&mut self, id: &str, on: bool) {
        let text = lists_with(&self.settings.stored, id, on);
        self.write_setting(defs::BLOCKER_LISTS.key, &text);
    }

    /// "Update now": fetches the lists, then says how it went.
    pub(crate) fn filter_lists_update(&mut self) {
        let started = self.blocking.update_now(|text| {
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host(|host| host.toast_with(&text, "", false));
            });
        });
        if started.is_some() {
            self.render_settings();
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

    /// Forgets the remembered decision the Sites list names `key` (`pages::permission_key`).
    pub(crate) fn site_permission_removed(&mut self, key: &str) {
        let Some((origin, kind)) = pages::parse_permission_key(key) else {
            return;
        };
        self.site_permissions.forget(origin, kind);
        self.store.forget_site_permission(origin, kind);
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

    /// Draws the page from the stored rows, the keymap, the model and the per-site data;
    /// closed (and any recording cancelled) while another tab is shown. The window's `page`
    /// property is set by `render_pages`, which calls this.
    pub(super) fn render_settings(&mut self) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        if self.shown_page() != Some(InternalPage::Settings) {
            self.settings.capture = Capture::Idle;
            return;
        }
        render_page(&window, self);
    }
}

/// `blocker.lists` with `id` set to `on`, as the stored text.
fn lists_with(stored: &HashMap<String, String>, id: &str, on: bool) -> String {
    let mut map: std::collections::BTreeMap<String, u32> = stored
        .get(defs::BLOCKER_LISTS.key)
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    map.insert(id.to_owned(), u32::from(on));
    serde_json::to_string(&map).expect("a map of numbers serialises")
}

/// The Filter lists section's rows: the title, and a note of licence, rule count and age.
fn filter_list_items(rows: &[ListRow], now: u64) -> Vec<FilterListItem> {
    rows.iter()
        .map(|row| {
            let mut parts: Vec<String> = Vec::new();
            if !row.licence.is_empty() {
                parts.push(row.licence.clone());
            }
            if let Some(rules) = row.rules {
                parts.push(format!("{rules} rules"));
            }
            parts.push(match (row.updated, row.bundled) {
                (Some(then), _) => format!("updated {}", pages::relative_age(now, then)),
                (None, true) => "bundled snapshot".into(),
                (None, false) => "not downloaded yet".into(),
            });
            if row.category_off {
                parts.push("its own switch above is off".into());
            }
            FilterListItem {
                id: row.id.as_str().into(),
                title: row.title.as_str().into(),
                note: parts.join(" \u{b7} ").into(),
                enabled: row.enabled,
                custom: row.custom,
                key: row.url.as_str().into(),
            }
        })
        .collect()
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
    let mut rows = wubble_ui::settings::setting_items(&sections);
    // the list editor that submitted a draft hears back
    let (answered, serial, note) = &page.answer;
    if let Some(row) = rows.iter_mut().find(|r| r.key.as_str() == answered) {
        row.answer = *serial;
        row.note = (*note).into();
    }
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
                colour: wubble_core::space::swatch_name(s.tint)
                    .unwrap_or_else(|| "Custom".into())
                    .into(),
                tabs: i32::try_from(host.model.order(s.id).len()).unwrap_or(i32::MAX),
            })
        })
        .collect();
    crate::bind::sync(window.get_settings_spaces(), spaces, |m| {
        window.set_settings_spaces(m)
    });

    let sites = pages::sites_view(&host.site_zoom, &host.site_permissions);
    let entries = wubble_ui::settings::entry_items;
    crate::bind::sync(window.get_settings_site_zoom(), entries(&sites.zoom), |m| {
        window.set_settings_site_zoom(m)
    });
    crate::bind::sync(
        window.get_settings_site_permissions(),
        entries(&sites.permissions),
        |m| window.set_settings_site_permissions(m),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    crate::bind::sync(
        window.get_settings_filter_lists(),
        filter_list_items(&host.blocking.list_rows(), now),
        |m| window.set_settings_filter_lists(m),
    );
    window.set_settings_filter_busy(host.blocking.is_busy());
    if window.get_settings_about().version.is_empty() {
        window.set_settings_about(about());
    }
}

#[cfg(test)]
mod filter_list_tests {
    use super::*;

    fn row(id: &str) -> ListRow {
        ListRow {
            id: id.into(),
            title: id.into(),
            licence: "GPL-3.0".into(),
            url: format!("https://l.test/{id}"),
            custom: false,
            enabled: true,
            category_off: false,
            bundled: true,
            rules: None,
            updated: None,
        }
    }

    #[test]
    fn rows_become_a_title_and_a_note() {
        let now = 1_000_000;
        let items = filter_list_items(
            &[
                row("a"),
                ListRow {
                    rules: Some(12),
                    updated: Some(now - 3 * 86_400),
                    ..row("b")
                },
                ListRow {
                    custom: true,
                    licence: String::new(),
                    bundled: false,
                    category_off: true,
                    ..row("c")
                },
            ],
            now,
        );
        let notes: Vec<String> = items.iter().map(|i| i.note.to_string()).collect();
        assert_eq!(
            notes,
            [
                "GPL-3.0 \u{b7} bundled snapshot",
                "GPL-3.0 \u{b7} 12 rules \u{b7} updated 3 days ago",
                "not downloaded yet \u{b7} its own switch above is off",
            ]
        );
        assert!(items[2].custom && items[2].key == "https://l.test/c");
    }

    #[test]
    fn a_toggle_writes_one_entry_into_the_stored_map() {
        let mut stored = HashMap::new();
        assert_eq!(lists_with(&stored, "easylist", false), r#"{"easylist":0}"#);
        stored.insert(
            defs::BLOCKER_LISTS.key.to_owned(),
            r#"{"easylist":0}"#.into(),
        );
        assert_eq!(
            lists_with(&stored, "urlhaus-1", true),
            r#"{"easylist":0,"urlhaus-1":1}"#
        );
    }
}
