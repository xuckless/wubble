//! The tab row's context menu as one table ([`ROWS`]) and the map from each row to what it does
//! ([`entries`]). Pure; the host turns [`Run`] into model commands, the mute switch and the split
//! entries it already has.
//!
//! The design has only a demo of this menu (`pages/touch.tsx` `MENU_ROWS`: Duplicate, Move to
//! Space, Copy link, Close tab, with icons, shortcuts and a danger row); the rows here are the
//! roadmap's (pin, duplicate, move to Space, split with, mute, close others, never sleep, close),
//! written new. "Close other tabs" keeps pinned tabs, as Chromium's `CloseOtherTabs` does.

use crate::model::{Command, Dest, Model};
use crate::tab::TabId;

/// One kind of row. Order in [`ROWS`] is the menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Pin,
    Duplicate,
    /// One entry per other Space.
    MoveToSpace,
    /// The split entries (the host's `menu::tab_entries` rules).
    Split,
    Mute,
    NeverSleep,
    CloseOthers,
    Close,
}

/// A row of the table: its kind, icon name (`ui/components/icon.slint`), and whether it is
/// destructive (drawn in the danger colour). A `Separator` row draws a divider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub row: Option<Row>,
    pub icon: &'static str,
    pub danger: bool,
}

const fn row(row: Row, icon: &'static str, danger: bool) -> MenuRow {
    MenuRow {
        row: Some(row),
        icon,
        danger,
    }
}

const SEPARATOR: MenuRow = MenuRow {
    row: None,
    icon: "",
    danger: false,
};

pub const ROWS: &[MenuRow] = &[
    row(Row::Pin, "pin", false),
    row(Row::Duplicate, "duplicate", false),
    row(Row::MoveToSpace, "move", false),
    row(Row::Split, "split", false),
    SEPARATOR,
    row(Row::Mute, "mute", false),
    row(Row::NeverSleep, "awake", false),
    SEPARATOR,
    row(Row::CloseOthers, "close", true),
    row(Row::Close, "close", true),
];

/// What choosing an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Model(Command),
    /// Flip the tab's mute (the engine owns it).
    ToggleMute(TabId),
    /// Stands for the host's split entries of this tab, spliced in its place.
    SplitEntries(TabId),
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    pub icon: &'static str,
    pub danger: bool,
    pub run: Run,
}

/// What the menu needs that the model does not own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub muted: bool,
}

/// The entries of `tab`'s row menu. Rows that do not apply are left out; separators never
/// lead, trail or double.
pub fn entries(model: &Model, tab: TabId, facts: Facts) -> Vec<Entry> {
    let Some(t) = model.tabs.get(&tab) else {
        return vec![];
    };
    let Some((space, _)) = model.location(tab) else {
        return vec![];
    };
    let mut out: Vec<Entry> = vec![];
    for r in ROWS {
        let mk = |label: String, run: Run| Entry {
            label,
            icon: r.icon,
            danger: r.danger,
            run,
        };
        match r.row {
            None => {
                if out.last().is_some_and(|e| e.run != Run::Separator) {
                    out.push(mk(String::new(), Run::Separator));
                }
            }
            Some(Row::Pin) => out.push(if model.is_pinned(tab) {
                mk("Unpin Tab".into(), Run::Model(Command::UnpinTab(tab)))
            } else {
                mk("Pin Tab".into(), Run::Model(Command::PinTab(tab)))
            }),
            Some(Row::Duplicate) => out.push(mk(
                "Duplicate Tab".into(),
                Run::Model(Command::DuplicateTab(tab)),
            )),
            Some(Row::MoveToSpace) => {
                for s in model.spaces.iter().filter(|s| s.id != space) {
                    out.push(mk(
                        format!("Move to {}", s.name),
                        Run::Model(Command::MoveTab {
                            tab,
                            to: Dest::Normal {
                                space: s.id,
                                index: usize::MAX,
                            },
                        }),
                    ));
                }
            }
            Some(Row::Split) => out.push(mk(String::new(), Run::SplitEntries(tab))),
            Some(Row::Mute) => out.push(mk(
                if facts.muted { "Unmute Tab" } else { "Mute Tab" }.into(),
                Run::ToggleMute(tab),
            )),
            Some(Row::NeverSleep) => {
                let on = t.lifecycle.never_sleep;
                out.push(mk(
                    if on { "Allow Sleep" } else { "Never Sleep" }.into(),
                    Run::Model(Command::SetNeverSleep { tab, on: !on }),
                ));
            }
            Some(Row::CloseOthers) => {
                let others = model
                    .order(space)
                    .into_iter()
                    .any(|o| o != tab && !model.is_pinned(o));
                if others {
                    out.push(mk(
                        "Close Other Tabs".into(),
                        Run::Model(Command::CloseOthers(tab)),
                    ));
                }
            }
            Some(Row::Close) => out.push(mk(
                "Close Tab".into(),
                Run::Model(Command::CloseTab(tab)),
            )),
        }
    }
    if out.last().is_some_and(|e| e.run == Run::Separator) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::apply;
    use crate::space::{ContainerId, Rgb};
    use std::time::Instant;

    fn model(n: usize) -> Model {
        (0..n).fold(Model::new(Instant::now()), |m, i| {
            apply(
                m,
                Command::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            )
            .0
        })
    }

    fn runs(m: &Model, tab: TabId) -> Vec<Run> {
        entries(m, tab, Facts::default())
            .into_iter()
            .map(|e| e.run)
            .collect()
    }

    #[test]
    fn one_space_offers_the_base_rows_in_table_order() {
        let m = model(2);
        let t = TabId(1);
        assert_eq!(
            runs(&m, t),
            vec![
                Run::Model(Command::PinTab(t)),
                Run::Model(Command::DuplicateTab(t)),
                Run::SplitEntries(t),
                Run::Separator,
                Run::ToggleMute(t),
                Run::Model(Command::SetNeverSleep { tab: t, on: true }),
                Run::Separator,
                Run::Model(Command::CloseOthers(t)),
                Run::Model(Command::CloseTab(t)),
            ]
        );
    }

    #[test]
    fn rows_follow_state() {
        let m = model(1);
        let t = TabId(1);
        // alone: no close-others
        assert!(!runs(&m, t).contains(&Run::Model(Command::CloseOthers(t))));
        let (m, _) = apply(m, Command::PinTab(t));
        let (m, _) = apply(m, Command::SetNeverSleep { tab: t, on: true });
        let e = entries(&m, t, Facts { muted: true });
        let labels: Vec<&str> = e.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"Unpin Tab"));
        assert!(labels.contains(&"Unmute Tab"));
        assert!(labels.contains(&"Allow Sleep"));
        assert!(e.iter().any(|e| e.danger));
    }

    #[test]
    fn move_to_space_lists_the_other_spaces() {
        let m = model(1);
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "Work".into(),
                tint: Rgb(1, 2, 3),
                container: ContainerId::DEFAULT,
            },
        );
        let e = entries(&m, TabId(1), Facts::default());
        let mv: Vec<&Entry> = e.iter().filter(|e| e.label.starts_with("Move to")).collect();
        assert_eq!(mv.len(), 1);
        assert_eq!(mv[0].label, "Move to Work");
        let Run::Model(cmd) = mv[0].run.clone() else {
            panic!()
        };
        let (m, _) = apply(m, cmd);
        assert_eq!(m.location(TabId(1)).unwrap().0, m.spaces[1].id);
    }

    #[test]
    fn unknown_tab_has_no_menu() {
        assert!(entries(&model(1), TabId(99), Facts::default()).is_empty());
    }
}
