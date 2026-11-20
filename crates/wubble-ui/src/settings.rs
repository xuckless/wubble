//! The settings page's models from the core view-models (`wubble_core::pages`, `keys`): one
//! generic mapping per row kind, so a new setting needs no code here either.

use slint::{ModelRc, SharedString, VecModel};
use wubble_core::keys::BindingRow;
use wubble_core::omnibox::ENGINES;
use wubble_core::pages::{Control, Entry, PageLink, SectionView, SettingRow};
use wubble_core::settings::{Section, SettingKind};

use crate::{BindingItem, EntryItem, MenuEntry, PageLinkItem, SettingControl, SettingItem};

fn strings<I: IntoIterator<Item = S>, S: Into<SharedString>>(items: I) -> ModelRc<SharedString> {
    ModelRc::from(std::rc::Rc::new(VecModel::from(
        items
            .into_iter()
            .map(Into::into)
            .collect::<Vec<SharedString>>(),
    )))
}

pub fn section_title(section: Section) -> &'static str {
    match section {
        Section::General => "General",
        Section::Privacy => "Privacy",
        Section::Appearance => "Appearance",
        Section::Performance => "Performance",
        Section::Hidden => "",
    }
}

/// What a segment shows for a stored word: an engine's name, or the word capitalised.
fn word_label(kind: SettingKind, word: &str) -> String {
    if kind == SettingKind::SearchEngine
        && let Some(e) = ENGINES.iter().find(|e| e.id == word)
    {
        return e.name.to_owned();
    }
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A list's entries for a list editor (kit/controls/list-editor.slint): a row of cells each.
pub fn entry_items(entries: &[Entry]) -> Vec<EntryItem> {
    entries
        .iter()
        .map(|e| EntryItem {
            label: e.label.as_str().into(),
            key: e.key.as_str().into(),
            cells: strings(e.cells.iter().map(String::as_str)),
            ..Default::default()
        })
        .collect()
}

/// One row's item: the fields its control reads, the rest at their defaults.
pub fn setting_item(row: &SettingRow) -> SettingItem {
    let mut item = SettingItem {
        key: row.key.into(),
        label: row.label.into(),
        value: row.value.as_str().into(),
        changed: row.changed,
        restart: row.restart,
        selected: -1,
        entries: ModelRc::from(std::rc::Rc::new(VecModel::from(entry_items(&row.entries)))),
        columns: strings(row.columns.iter().copied()),
        ..Default::default()
    };
    match row.control {
        Control::Toggle => {
            item.control = SettingControl::Toggle;
            item.on = row.value == "1";
        }
        Control::Segmented(words) => {
            item.control = SettingControl::Segmented;
            let labels: Vec<String> = words.iter().map(|w| word_label(row.kind, w)).collect();
            item.selected = words
                .iter()
                .position(|w| *w == row.value)
                .map_or(-1, |i| i as i32);
            // The engine is picked from a Select, as the design's. None chosen yet (the stored
            // default, "ask on first run") keeps `selected` at -1: the Select shows the first
            // engine's label, and picking any engine, the first too, stores it.
            if row.kind == SettingKind::SearchEngine {
                item.options = ModelRc::from(std::rc::Rc::new(VecModel::from(
                    labels
                        .iter()
                        .map(|l| MenuEntry {
                            label: l.as_str().into(),
                            enabled: true,
                            ..Default::default()
                        })
                        .collect::<Vec<_>>(),
                )));
            }
            item.labels = strings(labels);
            item.words = strings(words.iter().copied());
        }
        Control::Stepper { min, max } => {
            item.control = SettingControl::Stepper;
            item.number = row.value.parse().unwrap_or(min as f32);
            (item.min, item.max) = (min as f32, max as f32);
        }
        Control::Slider { min, max } => {
            item.control = SettingControl::Slider;
            item.number = row.value.parse().unwrap_or(min);
            (item.min, item.max) = (min, max);
        }
        Control::ListEditor => item.control = SettingControl::List,
        Control::MapEditor => item.control = SettingControl::Map,
        Control::TextField => item.control = SettingControl::Text,
    }
    item
}

/// The page's flat row list: each section's rows in order, the first naming the section; every
/// row carries its section's place on the page (the nav's row).
pub fn setting_items(sections: &[SectionView]) -> Vec<SettingItem> {
    sections
        .iter()
        .enumerate()
        .flat_map(|(index, s)| {
            let n = s.rows.len();
            s.rows.iter().enumerate().map(move |(i, row)| SettingItem {
                section: if i == 0 { section_title(s.section) } else { "" }.into(),
                section_index: index as i32,
                first: i == 0,
                last: i + 1 == n,
                ..setting_item(row)
            })
        })
        .collect()
}

pub fn binding_items(rows: &[BindingRow]) -> Vec<BindingItem> {
    rows.iter()
        .map(|r| BindingItem {
            action: r.action.id().into(),
            label: r.action.label().into(),
            chord: r.chord.map(|c| c.to_string()).unwrap_or_default().into(),
            keys: strings(
                r.chord
                    .map(|c| c.to_string().split('+').map(str::to_owned).collect())
                    .unwrap_or_else(Vec::new),
            ),
            changed: r.overridden,
        })
        .collect()
}

/// The built-in pages Settings links to (`wubble_core::pages::PAGE_LINKS`, or a search's
/// matches) as the page's nav rows and redirect rows.
pub fn page_link_items<'a>(links: impl IntoIterator<Item = &'a PageLink>) -> Vec<PageLinkItem> {
    links
        .into_iter()
        .map(|l| PageLinkItem {
            title: l.title().into(),
            url: l.url().into(),
            note: l.note.into(),
            action: l.action.id().into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;
    use std::collections::HashMap;

    #[test]
    fn every_row_of_the_table_maps_to_its_control() {
        let page = wubble_core::pages::settings_page(&HashMap::new());
        let items = setting_items(&page);
        let rows: Vec<&SettingRow> = page.iter().flat_map(|s| &s.rows).collect();
        assert_eq!(items.len(), rows.len());
        for (item, row) in items.iter().zip(&rows) {
            assert_eq!(item.key.as_str(), row.key);
            let want = match row.control {
                Control::Toggle => SettingControl::Toggle,
                Control::Segmented(_) => SettingControl::Segmented,
                Control::Stepper { .. } => SettingControl::Stepper,
                Control::Slider { .. } => SettingControl::Slider,
                Control::ListEditor => SettingControl::List,
                Control::MapEditor => SettingControl::Map,
                Control::TextField => SettingControl::Text,
            };
            assert_eq!(item.control, want, "{}", item.key);
        }
        let headers: Vec<&str> = items
            .iter()
            .filter(|i| i.first)
            .map(|i| i.section.as_str())
            .collect();
        assert_eq!(headers, ["General", "Privacy", "Appearance", "Performance"]);
        assert!(items.last().unwrap().last);
        let engine = setting_item(
            page.iter()
                .flat_map(|s| &s.rows)
                .find(|r| r.key == "search.engine")
                .unwrap(),
        );
        assert_eq!(
            engine.selected, -1,
            "no engine chosen yet: none is selected, so picking the first stores it"
        );
        let labels: Vec<String> = engine.labels.iter().map(|s| s.to_string()).collect();
        assert_eq!(labels, ["Google", "DuckDuckGo"]);
        let options: Vec<String> = engine.options.iter().map(|o| o.label.to_string()).collect();
        assert_eq!(options, labels, "the engine's Select rows");
        let archive = items
            .iter()
            .find(|i| i.control == SettingControl::Segmented && i.key != "search.engine")
            .expect("a segmented row");
        assert_eq!(
            archive.options.row_count(),
            0,
            "segmented rows have no Select"
        );
    }

    #[test]
    fn a_stored_engine_is_selected() {
        let stored = HashMap::from([("search.engine".to_string(), "duckduckgo".to_string())]);
        let page = wubble_core::pages::settings_page(&stored);
        let engine = page
            .iter()
            .flat_map(|s| &s.rows)
            .find(|r| r.key == "search.engine")
            .unwrap();
        assert_eq!(setting_item(engine).selected, 1);
    }

    #[test]
    fn a_chord_is_one_key_cap_per_key() {
        let rows = wubble_core::keys::binding_rows(wubble_core::keys::Platform::Linux, &[]);
        let items = binding_items(&rows);
        let with_chord = items.iter().find(|i| i.chord.contains('+')).unwrap();
        let keys: Vec<String> = with_chord.keys.iter().map(|k| k.to_string()).collect();
        assert_eq!(keys.join("+"), with_chord.chord.as_str());
        assert!(keys.len() >= 2);
    }
}
