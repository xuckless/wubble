//! The settings page's models from the core view-models (`wubble_core::pages`, `keys`): one
//! generic mapping per row kind, so a new setting needs no code here either.

use slint::{ModelRc, SharedString, VecModel};
use wubble_core::keys::BindingRow;
use wubble_core::omnibox::ENGINES;
use wubble_core::pages::{Control, Entry, SectionView, SettingRow};
use wubble_core::settings::{Section, SettingKind};

use crate::{BindingItem, EntryItem, SettingControl, SettingItem};

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

/// The ± step of a stepper over `min..=max`: about a hundredth of the range, a power of ten.
pub fn stepper_step(min: u32, max: u32) -> u32 {
    let digits = (max - min).max(1).ilog10();
    10u32.pow(digits.saturating_sub(1))
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

/// A list's entries for an `EntryList`.
pub fn entry_items(entries: &[Entry]) -> Vec<EntryItem> {
    entries
        .iter()
        .map(|e| EntryItem {
            label: e.label.as_str().into(),
            key: e.key.as_str().into(),
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
        ..Default::default()
    };
    match row.control {
        Control::Toggle => {
            item.control = SettingControl::Toggle;
            item.on = row.value == "1";
        }
        Control::Segmented(words) => {
            item.control = SettingControl::Segmented;
            item.selected = words
                .iter()
                .position(|w| *w == row.value)
                .map_or(-1, |i| i as i32);
            item.labels = strings(words.iter().map(|w| word_label(row.kind, w)));
            item.words = strings(words.iter().copied());
        }
        Control::Stepper { min, max } => {
            item.control = SettingControl::Stepper;
            item.number = row.value.parse().unwrap_or(min as f32);
            (item.min, item.max) = (min as f32, max as f32);
            item.step = stepper_step(min, max) as f32;
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
            changed: r.overridden,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;
    use std::collections::HashMap;

    #[test]
    fn steps_are_about_a_hundredth_of_the_range() {
        assert_eq!(stepper_step(25, 500), 10);
        assert_eq!(stepper_step(10, 86_400), 1000);
        assert_eq!(stepper_step(0, 9), 1);
        assert_eq!(stepper_step(5, 5), 1);
    }

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
        assert_eq!(engine.selected, -1, "no engine chosen yet");
        let labels: Vec<String> = engine.labels.iter().map(|s| s.to_string()).collect();
        assert_eq!(labels, ["Google", "DuckDuckGo"]);
    }
}
