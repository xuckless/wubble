//! The host's half of Boosts and of the page kit. Every view gets three document-start scripts
//! (`View::set_document_scripts`): the page kit (`assets/kit.js`, which the video controls,
//! SponsorBlock and the media keys also use) and the two scripts of the Boosts
//! (`wubble_core::boost::page_scripts`). Those carry every enabled Boost with its pattern, and
//! the kit picks the ones of the document it runs in, so the scripts are the same for every page
//! and change only with the table: a link, a redirect or a page's own navigation starts with its
//! Boosts in place. A document that started before the scripts were registered (a view's first
//! page) gets them when it commits.
//!
//! The editor panel (`boost-panel.slint`) edits one Boost (a draft) at a time; its CSS shows on
//! the page as it is typed, a script on Done (the page reloads). The stored Boosts here are the
//! one owner of the table while the app runs; the store keeps a copy.

use std::collections::BTreeMap;

use wubble_core::boost::{self, At, Boost, Pattern, Tint};
use wubble_core::tab::TabId;
use wubble_ui::{AppWindow, BoostView};

use super::Host;
use crate::menu::{Command as MenuCommand, Entry};

/// The page kit, run first in every document.
pub const KIT: &str = include_str!("../../assets/kit.js");

/// The kit's object, for scripts the host evaluates.
pub const KIT_REF: &str = "window[Symbol.for('wubble.kit')]";

#[derive(Default)]
pub(super) struct Boosts {
    /// Every stored Boost.
    all: Vec<Boost>,
    /// `settings::BOOSTS_ENABLED`.
    enabled: bool,
    /// The document-start scripts each live view was last given.
    given: BTreeMap<TabId, Vec<String>>,
    editor: Option<Editor>,
    /// Bumped when the editor's texts are replaced (`BoostView::serial`).
    serial: i32,
}

impl Boosts {
    pub fn new(all: Vec<Boost>) -> Self {
        Self {
            all,
            enabled: true,
            ..Self::default()
        }
    }
}

/// The Boost being edited, on the page of `tab`.
struct Editor {
    tab: TabId,
    draft: Boost,
    zapping: bool,
}

/// What `--smoke boost` waits for after storing a Boost.
pub(super) type Saved = Option<i64>;

impl Host {
    /// The scripts every document starts with.
    fn page_scripts(&self) -> Vec<String> {
        let b = &self.boosts;
        let [css, js] = boost::page_scripts(if b.enabled { &b.all } else { &[] });
        vec![KIT.to_owned(), css, js]
    }

    /// Gives `tab`'s view the scripts, if it has others.
    fn sync_page_scripts(&mut self, tab: TabId) {
        let scripts = self.page_scripts();
        if self.boosts.given.get(&tab) == Some(&scripts) {
            return;
        }
        if let Some(view) = self.tab_view(tab) {
            view.set_document_scripts(&scripts);
            self.boosts.given.insert(tab, scripts);
        }
    }

    /// Runs the scripts in the document `tab` shows now (each runs once per document).
    fn run_page_scripts(&mut self, tab: TabId) {
        let scripts = self.boosts.given.get(&tab).cloned().unwrap_or_default();
        if let Some(view) = self.tab_view(tab) {
            for script in &scripts {
                view.run_script(script);
            }
        }
    }

    /// A view opened: nothing was registered with it yet.
    pub(super) fn boosts_view_opened(&mut self, tab: TabId) {
        self.boosts.given.remove(&tab);
        self.sync_page_scripts(tab);
    }

    /// `tab` committed a document or began loading it: it runs its scripts unless it already did
    /// (its first page may have started before they were registered).
    pub(super) fn boosts_document(&mut self, tab: TabId) {
        self.run_page_scripts(tab);
    }

    pub(super) fn boosts_view_gone(&mut self, tab: TabId) {
        self.boosts.given.remove(&tab);
        if self.boosts.editor.as_ref().is_some_and(|e| e.tab == tab) {
            self.boosts.editor = None;
            self.redraw_boost();
        }
    }

    pub(super) fn set_boosts_enabled(&mut self, on: bool) {
        self.boosts.enabled = on;
        self.boosts_changed();
    }

    /// The table changed: every view's next document gets the new scripts.
    fn boosts_changed(&mut self) {
        let tabs: Vec<TabId> = self.views.keys().copied().collect();
        for tab in tabs {
            self.sync_page_scripts(tab);
        }
    }

    /// The page of `tab` again, so a changed Boost (a script) takes effect.
    fn reload_for_boosts(&mut self, tab: TabId) {
        if let Some(view) = self.tab_view(tab) {
            view.reload();
        }
    }

    fn url_of(&self, tab: TabId) -> Option<String> {
        self.model.tabs.get(&tab).map(|t| t.url.clone())
    }

    // ----- the address pill's menu and the action

    /// The site menu's Boost rows: each Boost of the page (on or off), and a new one.
    pub(super) fn boost_entries(&self, url: &str) -> Vec<Entry> {
        let mut out = vec![Entry {
            label: "Boosts for this site".into(),
            command: MenuCommand::Separator,
            enabled: false,
            icon: "",
            shortcut: String::new(),
            danger: false,
        }];
        let mut mine: Vec<&Boost> = self
            .boosts
            .all
            .iter()
            .filter(|b| Pattern::parse(&b.site).is_some_and(|p| p.matches(url)))
            .collect();
        mine.sort_by(|a, b| boost::by_order(a, b));
        for b in mine {
            let mark = if b.enabled { "\u{2713}" } else { "  " };
            out.push(Entry {
                label: format!("{mark} {}", b.name),
                command: MenuCommand::ToggleBoost(b.id),
                enabled: self.boosts.enabled,
                icon: "",
                shortcut: String::new(),
                danger: false,
            });
            out.push(Entry {
                label: format!("    Edit \u{201c}{}\u{201d}", b.name),
                command: MenuCommand::EditBoost(b.id),
                enabled: true,
                icon: "",
                shortcut: String::new(),
                danger: false,
            });
        }
        out.push(Entry {
            label: "New Boost for this site".into(),
            command: MenuCommand::Perform(wubble_core::action::Action::NewBoost),
            enabled: wubble_core::zoom::host_of(url).is_some(),
            icon: "",
            shortcut: String::new(),
            danger: false,
        });
        out
    }

    /// `Action::NewBoost`: the editor on a new Boost for the visible page's site.
    pub(super) fn new_boost(&mut self) {
        let Some(tab) = self.shown() else {
            return;
        };
        let host = self
            .url_of(tab)
            .and_then(|u| wubble_core::zoom::host_of(&u));
        match host {
            Some(host) => self.open_editor(tab, Boost::for_host(&host)),
            None => {
                if let Some(window) = self.w.window.upgrade() {
                    self.toast(&window, "This page has no site to boost");
                }
            }
        }
    }

    pub(super) fn edit_boost(&mut self, id: i64) {
        let found = self.boosts.all.iter().find(|b| b.id == id).cloned();
        if let (Some(tab), Some(b)) = (self.shown(), found) {
            self.open_editor(tab, b);
        }
    }

    fn open_editor(&mut self, tab: TabId, draft: Boost) {
        self.boosts.editor = Some(Editor {
            tab,
            draft,
            zapping: false,
        });
        self.boosts.serial = self.boosts.serial.wrapping_add(1);
        self.redraw_boost();
    }

    pub(super) fn toggle_boost(&mut self, id: i64) {
        let Some(mut b) = self.boosts.all.iter().find(|b| b.id == id).cloned() else {
            return;
        };
        b.enabled = !b.enabled;
        self.store_boost(b);
    }

    /// Stores `b`; once the store answered, the table and the pages follow (`boost_stored`).
    pub(super) fn store_boost(&mut self, b: Boost) {
        let tab = self.shown();
        let draft = b.clone();
        self.store.put_boost(&b, move |id| {
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host(|host| host.boost_stored(draft, id, tab));
            });
        });
    }

    fn boost_stored(&mut self, mut b: Boost, id: Saved, tab: Option<TabId>) {
        let Some(id) = id else {
            if let Some(window) = self.w.window.upgrade() {
                self.toast(&window, "The Boost could not be saved");
            }
            return;
        };
        b.id = id;
        match self.boosts.all.iter_mut().find(|x| x.id == id) {
            Some(slot) => *slot = b,
            None => self.boosts.all.push(b),
        }
        self.boosts_changed();
        if let Some(tab) = tab {
            self.reload_for_boosts(tab);
        }
        self.video_smoke_boost_saved(id);
    }

    // ----- the editor

    fn draft(&mut self) -> Option<&mut Boost> {
        self.boosts.editor.as_mut().map(|e| &mut e.draft)
    }

    pub(crate) fn boost_text(&mut self, field: &str, text: &str) {
        let Some(d) = self.draft() else {
            return;
        };
        match field {
            "name" => d.name = text.to_owned(),
            "site" => d.site = text.trim().to_owned(),
            "css" => d.css = text.to_owned(),
            "js" => d.js = text.to_owned(),
            "font" => d.font = Some(text.to_owned()).filter(|f| !f.trim().is_empty()),
            _ => return,
        }
        if field != "js" && field != "name" {
            self.show_draft_css();
        }
        self.redraw_boost();
    }

    pub(crate) fn boost_number(&mut self, field: &str, value: f32) {
        let Some(d) = self.draft() else {
            return;
        };
        match field {
            "at" => d.js_at = At::from_index(value.round() as i64),
            "strength" => {
                let strength = value.round().clamp(0.0, 100.0) as u8;
                d.tint = Some(Tint {
                    rgb: d.tint.map_or(0x7cc4ff, |t| t.rgb),
                    strength,
                });
            }
            _ => return,
        }
        self.show_draft_css();
        self.redraw_boost();
    }

    pub(crate) fn boost_tint(&mut self, colour: slint::Color) {
        let Some(d) = self.draft() else {
            return;
        };
        let rgb = u32::from_be_bytes([0, colour.red(), colour.green(), colour.blue()]);
        d.tint = match d.tint {
            // The chosen swatch again: no tint.
            Some(t) if t.rgb == rgb => None,
            t => Some(Tint {
                rgb,
                strength: t.map_or(20, |t| t.strength.max(1)),
            }),
        };
        self.show_draft_css();
        self.redraw_boost();
    }

    pub(crate) fn boost_act(&mut self, name: &str) {
        let Some(editor) = self.boosts.editor.as_mut() else {
            return;
        };
        let tab = editor.tab;
        match name {
            "enabled" => {
                editor.draft.enabled = !editor.draft.enabled;
                self.show_draft_css();
            }
            "zap" => {
                editor.zapping = !editor.zapping;
                let on = editor.zapping;
                if let Some(view) = self.tab_view(tab) {
                    view.eval(&format!("{KIT_REF}&&{KIT_REF}.zap({on})"));
                }
            }
            "done" => {
                let draft = editor.draft.clone();
                if Pattern::parse(&draft.site).is_none() {
                    self.redraw_boost();
                    return;
                }
                self.stop_zap();
                self.boosts.editor = None;
                self.store_boost(draft);
            }
            "delete" => {
                let id = editor.draft.id;
                self.stop_zap();
                self.boosts.editor = None;
                self.boosts.all.retain(|b| b.id != id);
                self.store.delete_boost(id);
                self.boosts_changed();
                self.reload_for_boosts(tab);
            }
            "close" => {
                self.stop_zap();
                self.boosts.editor = None;
                // Back to what is stored.
                self.show_css(tab, None);
            }
            _ => {}
        }
        self.redraw_boost();
    }

    fn stop_zap(&mut self) {
        let Some(editor) = self.boosts.editor.as_mut() else {
            return;
        };
        if std::mem::take(&mut editor.zapping) {
            let tab = editor.tab;
            if let Some(view) = self.tab_view(tab) {
                view.eval(&format!("{KIT_REF}&&{KIT_REF}.zap(false)"));
            }
        }
    }

    /// The kit's poll reported the picker: `selector` was zapped (hidden from now on), or the
    /// picker ended without one.
    /// The selector is the page's to answer, so only a picker the host started counts, and only
    /// a plain selector (`boost::zap_rule`) is written into the Boost.
    pub(super) fn boost_zapped(&mut self, tab: TabId, on: bool, selector: Option<&str>) {
        let Some(editor) = self
            .boosts
            .editor
            .as_mut()
            .filter(|e| e.tab == tab && e.zapping)
        else {
            return;
        };
        if on {
            return;
        }
        editor.zapping = false;
        if let Some(rule) = selector.and_then(boost::zap_rule) {
            let css = &mut editor.draft.css;
            if !css.is_empty() && !css.ends_with('\n') {
                css.push('\n');
            }
            css.push_str(&rule);
            self.boosts.serial = self.boosts.serial.wrapping_add(1);
            self.show_draft_css();
        }
        self.redraw_boost();
    }

    /// The page shows the draft's CSS as it would apply (with the other Boosts of the page).
    fn show_draft_css(&mut self) {
        if let Some(editor) = &self.boosts.editor {
            let (tab, draft) = (editor.tab, editor.draft.clone());
            self.show_css(tab, Some(draft));
        }
    }

    /// Replaces the page's Boost CSS by what the stored Boosts give, with `draft` in place of its
    /// stored self.
    fn show_css(&mut self, tab: TabId, draft: Option<Boost>) {
        let Some(url) = self.tab_view(tab).map(|v| v.nav().page.url.clone()) else {
            return;
        };
        let mut all = self.boosts.all.clone();
        if let Some(d) = draft {
            match all.iter_mut().find(|b| b.id == d.id && d.id != 0) {
                Some(slot) => *slot = d,
                None => all.push(d),
            }
        }
        let css: Vec<String> = if self.boosts.enabled {
            boost::matching(&all, &url)
                .into_iter()
                .map(boost::css_of)
                .collect()
        } else {
            vec![]
        };
        let text = serde_json::Value::String(css.join("\n")).to_string();
        if let Some(view) = self.tab_view(tab) {
            view.eval(&format!("{KIT_REF}&&{KIT_REF}.css({text})"));
        }
    }

    fn redraw_boost(&mut self) {
        if let Some(window) = self.w.window.upgrade() {
            self.render_boost(&window);
        }
    }

    pub(super) fn render_boost(&self, window: &AppWindow) {
        let view = match &self.boosts.editor {
            Some(e) => {
                let d = &e.draft;
                let tint = d.tint.unwrap_or(Tint {
                    rgb: 0x7cc4ff,
                    strength: 0,
                });
                let [_, r, g, b] = tint.rgb.to_be_bytes();
                BoostView {
                    open: self.shown() == Some(e.tab),
                    serial: self.boosts.serial,
                    is_new: d.id == 0,
                    name: d.name.as_str().into(),
                    site: d.site.as_str().into(),
                    css: d.css.as_str().into(),
                    js: d.js.as_str().into(),
                    at: d.js_at.index() as i32,
                    tint_on: d.tint.is_some_and(|t| t.strength > 0),
                    tint: slint::Color::from_rgb_u8(r, g, b),
                    strength: f32::from(tint.strength),
                    font: d.font.as_deref().unwrap_or_default().into(),
                    enabled: d.enabled,
                    zapping: e.zapping,
                    error: if Pattern::parse(&d.site).is_some() {
                        "".into()
                    } else {
                        "Not a site pattern".into()
                    },
                }
            }
            None => BoostView::default(),
        };
        window.set_boost(view);
    }

    /// `--smoke boost` stores a Boost through the same path as the editor's Done.
    pub(crate) fn boost_put_for_smoke(&mut self, b: Boost) {
        self.store_boost(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kit_has_every_entry_point_the_host_calls() {
        for needle in [
            "Symbol.for(\"wubble.kit\")",
            "k.snapshot",
            "k.jumps",
            "k.cmd",
            "k.frame",
            "k.pip",
            "k.act",
            "setActionHandler",
            "k.css",
            "k.ready",
            "k.matches",
            "k.zap",
            "DOMPath.ts",
            "adoptedStyleSheets",
        ] {
            assert!(KIT.contains(needle), "kit.js lacks {needle}");
        }
    }
}
