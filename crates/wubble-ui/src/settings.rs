//! The settings page's models from the core view-models (`wubble_core::pages`, `keys`): one
//! generic mapping per row kind, so a new setting needs no code here either.

use slint::{Model, ModelRc, SharedString, VecModel};
use wubble_core::engines::ENGINES;
use wubble_core::keys::BindingRow;
use wubble_core::pages::{Control, Entry, PageLink, SectionView, SettingRow};
use wubble_core::secure_dns::{self, SecureDns};
use wubble_core::settings::{self, Section, SettingKind};

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
    section.title()
}

/// What a segment shows for a stored word: an engine's name, a Select row's label
/// (`settings::SELECT_ROWS`), or the word capitalised.
fn word_label(key: &str, kind: SettingKind, word: &str) -> String {
    if kind == SettingKind::SearchEngine
        && let Some(e) = ENGINES.iter().find(|e| e.id == word)
    {
        return e.name.to_owned();
    }
    if let Some((_, label)) =
        settings::select_labels(key).and_then(|labels| labels.iter().find(|(w, _)| *w == word))
    {
        return (*label).to_owned();
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
        fixed: row.entries_fixed,
        kept: row.remove_only,
        ..Default::default()
    };
    match row.control {
        Control::Toggle => {
            item.control = SettingControl::Toggle;
            item.on = row.value == "1";
        }
        Control::Segmented(words) => {
            item.control = SettingControl::Segmented;
            // The engine is picked from a Select, as the design's, among the row's options (the
            // built-ins and the user's own engines). None chosen yet (the stored default, "ask
            // on first search") keeps `selected` at -1: the Select reads "Ask on first search",
            // and picking any engine, the first too, stores it.
            let (words, labels): (Vec<String>, Vec<String>) =
                if row.kind == SettingKind::SearchEngine {
                    row.options.iter().cloned().unzip()
                } else {
                    (
                        words.iter().map(|w| (*w).to_owned()).collect(),
                        words
                            .iter()
                            .map(|w| word_label(row.key, row.kind, w))
                            .collect(),
                    )
                };
            // a custom secure DNS template is chosen under its menu word, `custom:`
            let chosen = match SecureDns::parse(&row.value) {
                Ok(dns) if row.kind == SettingKind::SecureDns => dns.word(),
                _ => row.value.as_str(),
            };
            item.selected = words
                .iter()
                .position(|w| *w == chosen)
                .map_or(-1, |i| i as i32);
            // custom secure DNS: the template's own field below the Select
            if row.kind == SettingKind::SecureDns && chosen == secure_dns::CUSTOM {
                item.field_prefix = secure_dns::CUSTOM.into();
                item.field_text = row.value[secure_dns::CUSTOM.len()..].into();
            }
            // A Select row of the table (the startup mode, secure DNS) has a menu of its labels.
            if row.kind == SettingKind::SearchEngine || settings::select_labels(row.key).is_some() {
                if row.kind == SettingKind::SearchEngine {
                    item.placeholder = "Ask on first search".into();
                }
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
            item.words = strings(words);
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
        Control::Folder => item.control = SettingControl::Folder,
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

/// Whether two `ModelRc`s have the same length and element-wise equal contents (`ModelRc::eq`
/// uses `Rc::ptr_eq`, so freshly built models always compare unequal).
fn same_model<T: Clone + 'static>(
    a: &ModelRc<T>,
    b: &ModelRc<T>,
    eq: impl Fn(&T, &T) -> bool,
) -> bool {
    a.row_count() == b.row_count() && a.iter().zip(b.iter()).all(|(x, y)| eq(&x, &y))
}

/// Content equality for [`EntryItem`], comparing `cells` by value rather than `Rc::ptr_eq`.
pub fn same_entry_item(a: &EntryItem, b: &EntryItem) -> bool {
    let (mut a, mut b) = (a.clone(), b.clone());
    let (a_cells, b_cells) = (std::mem::take(&mut a.cells), std::mem::take(&mut b.cells));
    same_model(&a_cells, &b_cells, PartialEq::eq) && a == b
}

/// Content equality for [`SettingItem`], comparing all inner `ModelRc` fields by value so
/// `render_settings` does not tear down and rebuild unchanged rows.
pub fn same_setting_item(a: &SettingItem, b: &SettingItem) -> bool {
    let (mut a, mut b) = (a.clone(), b.clone());
    let (a_words, b_words) = (std::mem::take(&mut a.words), std::mem::take(&mut b.words));
    let (a_labels, b_labels) = (std::mem::take(&mut a.labels), std::mem::take(&mut b.labels));
    let (a_entries, b_entries) = (
        std::mem::take(&mut a.entries),
        std::mem::take(&mut b.entries),
    );
    let (a_columns, b_columns) = (
        std::mem::take(&mut a.columns),
        std::mem::take(&mut b.columns),
    );
    let (a_options, b_options) = (
        std::mem::take(&mut a.options),
        std::mem::take(&mut b.options),
    );
    a == b
        && same_model(&a_words, &b_words, PartialEq::eq)
        && same_model(&a_labels, &b_labels, PartialEq::eq)
        && same_model(&a_entries, &b_entries, same_entry_item)
        && same_model(&a_columns, &b_columns, PartialEq::eq)
        && same_model(&a_options, &b_options, PartialEq::eq)
}

/// Content equality for [`BindingItem`], comparing `keys` by value rather than `Rc::ptr_eq`.
pub fn same_binding_item(a: &BindingItem, b: &BindingItem) -> bool {
    let (mut a, mut b) = (a.clone(), b.clone());
    let (a_keys, b_keys) = (std::mem::take(&mut a.keys), std::mem::take(&mut b.keys));
    same_model(&a_keys, &b_keys, PartialEq::eq) && a == b
}

#[cfg(test)]
mod tests {
    use super::*;
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
                Control::Folder => SettingControl::Folder,
            };
            assert_eq!(item.control, want, "{}", item.key);
        }
        let headers: Vec<&str> = items
            .iter()
            .filter(|i| i.first)
            .map(|i| i.section.as_str())
            .collect();
        assert_eq!(
            headers,
            [
                "General",
                "Privacy",
                "Downloads",
                "Appearance",
                "Performance"
            ]
        );
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
        // no region stored: the everywhere engines (Chromium's default list adds none)
        let names: Vec<&str> = ENGINES
            .iter()
            .filter(|e| e.reach == wubble_core::engines::Reach::Everywhere)
            .map(|e| e.name)
            .collect();
        assert_eq!(labels, names);
        assert!(engine.fixed, "the built-ins are listed under the Select");
        assert_eq!(engine.entries.row_count(), ENGINES.len());
        let options: Vec<String> = engine.options.iter().map(|o| o.label.to_string()).collect();
        assert_eq!(options, labels, "the engine's Select rows");
        assert_eq!(engine.placeholder, "Ask on first search");
        let archive = items
            .iter()
            .find(|i| {
                i.control == SettingControl::Segmented
                    && i.key != "search.engine"
                    && settings::select_labels(&i.key).is_none()
            })
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

    /// A choice made after the page was drawn reaches the shown row through the in-place sync
    /// the host uses (`same_setting_item`): the engine's Select goes from "Ask on first search"
    /// (none chosen) to the engine, and back.
    #[test]
    fn a_changed_engine_reaches_the_synced_row() {
        let engine_row = |stored: &HashMap<String, String>| {
            let items = setting_items(&wubble_core::pages::settings_page(stored));
            let i = items.iter().position(|r| r.key == "search.engine").unwrap();
            (items, i)
        };
        let (items, i) = engine_row(&HashMap::new());
        assert_eq!(items[i].selected, -1);
        assert_eq!(items[i].placeholder, "Ask on first search");
        let model = VecModel::from(items);
        let chosen = HashMap::from([("search.engine".to_owned(), "duckduckgo".to_owned())]);
        let (next, _) = engine_row(&chosen);
        assert!(crate::sync_by(&model, next, same_setting_item));
        let row = model.row_data(i).unwrap();
        assert_eq!(row.value, "duckduckgo");
        assert_eq!(
            row.options.row_data(row.selected as usize).unwrap().label,
            "DuckDuckGo"
        );
        // the same view again changes nothing
        let (again, _) = engine_row(&chosen);
        assert!(!crate::sync_by(&model, again, same_setting_item));
        let (none, _) = engine_row(&HashMap::new());
        assert!(crate::sync_by(&model, none, same_setting_item));
        assert_eq!(model.row_data(i).unwrap().selected, -1);
    }

    #[test]
    fn same_item_compares_inner_models_by_content() {
        let page = wubble_core::pages::settings_page(&HashMap::new());
        let first = setting_items(&page);
        let second = setting_items(&page);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert!(same_setting_item(a, b), "unchanged row {}", a.key);
        }
        let mut modified = second[0].clone();
        modified.changed = !modified.changed;
        assert!(!same_setting_item(&first[0], &modified));

        let rows = wubble_core::keys::binding_rows(wubble_core::keys::Platform::Linux, &[]);
        let b1 = binding_items(&rows);
        let b2 = binding_items(&rows);
        for (a, b) in b1.iter().zip(&b2) {
            assert!(same_binding_item(a, b), "unchanged binding {}", a.action);
        }
        let mut bm = b2[0].clone();
        bm.keys = strings(["Alt", "X"]);
        assert!(!same_binding_item(&b1[0], &bm));

        let e1 = entry_items(&[Entry {
            label: "a".into(),
            key: "a".into(),
            cells: vec!["x".into(), "y".into()],
        }]);
        let e2 = entry_items(&[Entry {
            label: "a".into(),
            key: "a".into(),
            cells: vec!["x".into(), "y".into()],
        }]);
        let e3 = entry_items(&[Entry {
            label: "a".into(),
            key: "a".into(),
            cells: vec!["x".into(), "z".into()],
        }]);
        assert!(same_entry_item(&e1[0], &e2[0]));
        assert!(!same_entry_item(&e1[0], &e3[0]));
    }
}
