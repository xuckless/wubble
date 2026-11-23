//! The tab row's context menu as one table ([`ROWS`]) and the map from each row to what it does
//! ([`entries`]). Pure; the host turns [`Run`] into model commands, the mute switch and the split
//! entries it already has.
//!
//! The design has only a demo of this menu (`pages/touch.tsx` `MENU_ROWS`: Duplicate, Move to
//! Space, Copy link, Close tab, with icons, shortcuts and a danger row); the rows here are the
//! roadmap's (pin, duplicate, move to Space, split with, move to a window, mute, never sleep,
//! never sleep for the Space or site, close others, close), written new. "Close other tabs" keeps pinned tabs, as Chromium's `CloseOtherTabs` does.

use crate::busy::Reason;
use crate::model::{Command, Dest, Model, Place};
use crate::space::{FolderId, Item, SpaceKind};
use crate::tab::TabId;

/// One kind of row. Order in [`ROWS`] is the menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Reload,
    Pin,
    Duplicate,
    /// The page's address on the clipboard (the design's `MENU_ROWS` "Copy link").
    CopyLink,
    /// New folder with the tab, move it to each other folder, take it out of its folder.
    Folders,
    /// One entry per other Space.
    MoveToSpace,
    /// The split entries (the host's `menu::tab_entries` rules).
    Split,
    /// The host's move-to-window entries (a new window, each other Main window).
    Windows,
    Mute,
    NeverSleep,
    /// The host's never-sleep switches for the tab's Space and site.
    SleepScopes,
    CloseOthers,
    /// Chromium's "Close Tabs Below": the unpinned tabs under it in its Space.
    CloseBelow,
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
    row(Row::Reload, "reload", false),
    row(Row::Pin, "pin", false),
    row(Row::Duplicate, "duplicate", false),
    row(Row::CopyLink, "link", false),
    SEPARATOR,
    row(Row::Folders, "folder", false),
    row(Row::MoveToSpace, "move", false),
    row(Row::Split, "split", false),
    row(Row::Windows, "move", false),
    SEPARATOR,
    row(Row::Mute, "mute", false),
    row(Row::NeverSleep, "awake", false),
    row(Row::SleepScopes, "awake", false),
    SEPARATOR,
    row(Row::CloseOthers, "close", true),
    row(Row::CloseBelow, "close", true),
    row(Row::Close, "close", true),
];

/// What choosing an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Model(Command),
    /// Reload the tab's page (the engine owns it).
    Reload(TabId),
    /// Put this text on the clipboard.
    Copy(String),
    /// Ask for a folder's new name (the host's sheet).
    RenameFolder(FolderId),
    /// Flip the tab's mute (the engine owns it).
    ToggleMute(TabId),
    /// Stands for the host's split entries of this tab, spliced in its place.
    SplitEntries(TabId),
    /// Stands for the host's move-to-window entries of this tab.
    WindowEntries(TabId),
    /// Stands for the host's Space and site never-sleep switches of this tab.
    SleepEntries(TabId),
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
    if !model.tabs.contains_key(&tab) {
        return vec![];
    }
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
            Some(Row::Reload) => out.push(mk("Reload".into(), Run::Reload(tab))),
            Some(Row::CopyLink) => {
                let url = &model.tabs[&tab].url;
                if !url.is_empty() {
                    out.push(mk("Copy Link".into(), Run::Copy(url.clone())));
                }
            }
            Some(Row::Folders) => out.extend(
                folder_rows(model, tab)
                    .into_iter()
                    .map(|(l, c)| mk(l, Run::Model(c))),
            ),
            Some(Row::CloseBelow) => {
                let order = model.order(space);
                let below: Vec<TabId> = order
                    .iter()
                    .skip_while(|&&t| t != tab)
                    .skip(1)
                    .copied()
                    .filter(|&t| !model.is_pinned(t))
                    .collect();
                if !model.is_pinned(tab) && !below.is_empty() {
                    out.push(mk(
                        "Close Tabs Below".into(),
                        Run::Model(Command::CloseTabs(below)),
                    ));
                }
            }
            Some(Row::MoveToSpace) => {
                // a tab never crosses a private window's line: no private Space is a target, and
                // a private window's tab has none
                let private = model.is_private_space(space);
                for s in model
                    .spaces
                    .iter()
                    .filter(|s| s.id != space && s.kind != SpaceKind::Private && !private)
                {
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
            Some(Row::Windows) => out.push(mk(String::new(), Run::WindowEntries(tab))),
            Some(Row::SleepScopes) => out.push(mk(String::new(), Run::SleepEntries(tab))),
            Some(Row::Mute) => out.push(mk(
                if facts.muted {
                    "Unmute Tab"
                } else {
                    "Mute Tab"
                }
                .into(),
                Run::ToggleMute(tab),
            )),
            Some(Row::NeverSleep) => {
                let on = model.tabs[&tab].busy.contains(Reason::NeverSleep);
                out.push(mk(
                    if on { "Allow Sleep" } else { "Never Sleep" }.into(),
                    Run::Model(Command::Busy {
                        tab,
                        reason: Reason::NeverSleep,
                        on: !on,
                    }),
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
            Some(Row::Close) => {
                out.push(mk("Close Tab".into(), Run::Model(Command::CloseTab(tab))))
            }
        }
    }
    if out.last().is_some_and(|e| e.run == Run::Separator) {
        out.pop();
    }
    out
}

/// The tab's folder rows: a new folder with it (not for a pinned tab), each other folder of its
/// Space to move it into, and out of its own folder (just below it).
fn folder_rows(model: &Model, tab: TabId) -> Vec<(String, Command)> {
    let Some((space, place)) = model.location(tab) else {
        return vec![];
    };
    if matches!(place, Place::Pinned(_)) {
        return vec![];
    }
    let Some(s) = model.spaces.iter().find(|s| s.id == space) else {
        return vec![];
    };
    let own = match place {
        Place::Folder(f, _) => Some(f),
        _ => None,
    };
    let mut out = vec![(
        "New Folder with Tab".to_string(),
        Command::FolderWithTab {
            tab,
            name: "New Folder".into(),
        },
    )];
    for item in &s.items {
        if let Item::Folder(f) = item
            && Some(f.id) != own
        {
            out.push((
                format!("Move to {}", f.name),
                Command::MoveTab {
                    tab,
                    to: Dest::Folder {
                        folder: f.id,
                        index: usize::MAX,
                    },
                },
            ));
        }
    }
    if let Some(f) = own
        && let Some(at) = s
            .items
            .iter()
            .position(|i| matches!(i, Item::Folder(g) if g.id == f))
    {
        out.push((
            "Remove from Folder".into(),
            Command::MoveTab {
                tab,
                to: Dest::Normal {
                    space,
                    index: at + 1,
                },
            },
        ));
    }
    out
}

/// One kind of row of a folder header's menu. Order in [`FOLDER_ROWS`] is the menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderRow {
    Rename,
    Collapse,
    /// Its tabs stay, at its place (`Command::DeleteFolder`).
    Ungroup,
    /// Closes its tabs and the folder.
    Close,
}

/// A folder header's menu table: (row, icon, danger).
pub const FOLDER_ROWS: &[(FolderRow, &str, bool)] = &[
    (FolderRow::Rename, "folder", false),
    (FolderRow::Collapse, "chevron-down", false),
    (FolderRow::Ungroup, "stack", false),
    (FolderRow::Close, "close", true),
];

/// The entries of folder `folder`'s header menu; empty for an unknown folder.
pub fn folder_entries(model: &Model, folder: FolderId) -> Vec<Entry> {
    let Some(f) = model.spaces.iter().find_map(|s| {
        s.items.iter().find_map(|i| match i {
            Item::Folder(f) if f.id == folder => Some(f),
            _ => None,
        })
    }) else {
        return vec![];
    };
    FOLDER_ROWS
        .iter()
        .map(|&(row, icon, danger)| {
            let (label, run) = match row {
                FolderRow::Rename => ("Rename Folder", Run::RenameFolder(folder)),
                FolderRow::Collapse => (
                    if f.collapsed { "Expand" } else { "Collapse" },
                    Run::Model(Command::SetFolderCollapsed {
                        folder,
                        collapsed: !f.collapsed,
                    }),
                ),
                FolderRow::Ungroup => ("Ungroup", Run::Model(Command::DeleteFolder(folder))),
                FolderRow::Close if f.tabs.is_empty() => {
                    ("Delete Folder", Run::Model(Command::DeleteFolder(folder)))
                }
                FolderRow::Close => (
                    "Close Folder",
                    Run::Model(Command::CloseTabs(f.tabs.clone())),
                ),
            };
            Entry {
                label: label.into(),
                icon,
                danger,
                run,
            }
        })
        .collect()
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
                Run::Reload(t),
                Run::Model(Command::PinTab(t)),
                Run::Model(Command::DuplicateTab(t)),
                Run::Copy("https://0.test/".into()),
                Run::Separator,
                Run::Model(Command::FolderWithTab {
                    tab: t,
                    name: "New Folder".into()
                }),
                Run::SplitEntries(t),
                Run::WindowEntries(t),
                Run::Separator,
                Run::ToggleMute(t),
                Run::Model(Command::Busy {
                    tab: t,
                    reason: Reason::NeverSleep,
                    on: true,
                }),
                Run::SleepEntries(t),
                Run::Separator,
                Run::Model(Command::CloseOthers(t)),
                Run::Model(Command::CloseTabs(vec![TabId(2)])),
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
        let (m, _) = apply(
            m,
            Command::Busy {
                tab: t,
                reason: Reason::NeverSleep,
                on: true,
            },
        );
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
        let mv: Vec<&Entry> = e
            .iter()
            .filter(|e| e.label.starts_with("Move to"))
            .collect();
        assert_eq!(mv.len(), 1);
        assert_eq!(mv[0].label, "Move to Work");
        let Run::Model(cmd) = mv[0].run.clone() else {
            panic!()
        };
        let (m, _) = apply(m, cmd);
        assert_eq!(m.location(TabId(1)).unwrap().0, m.spaces[1].id);
    }

    #[test]
    fn close_below_and_the_folder_rows_follow_the_place() {
        let m = model(3);
        let (a, b, c) = (TabId(1), TabId(2), TabId(3));
        assert!(runs(&m, a).contains(&Run::Model(Command::CloseTabs(vec![b, c]))));
        assert!(
            !runs(&m, c)
                .iter()
                .any(|r| matches!(r, Run::Model(Command::CloseTabs(_)))),
            "nothing below the last"
        );
        let (m, _) = apply(
            m,
            Command::FolderWithTab {
                tab: b,
                name: "F".into(),
            },
        );
        let f = match &m.spaces[0].items[1] {
            Item::Folder(f) => f.id,
            Item::Tab(_) => panic!(),
        };
        let labels = |m: &Model, t| -> Vec<String> {
            entries(m, t, Facts::default())
                .into_iter()
                .map(|e| e.label)
                .collect()
        };
        assert!(labels(&m, a).contains(&"Move to F".to_string()));
        assert!(!labels(&m, a).contains(&"Remove from Folder".to_string()));
        assert!(labels(&m, b).contains(&"Remove from Folder".to_string()));
        assert!(!labels(&m, b).contains(&"Move to F".to_string()));
        // out of the folder: just below it
        let out = entries(&m, b, Facts::default())
            .into_iter()
            .find(|e| e.label == "Remove from Folder")
            .unwrap();
        let Run::Model(cmd) = out.run else { panic!() };
        let (m2, _) = apply(m.clone(), cmd);
        assert!(matches!(m2.spaces[0].items[2], Item::Tab(t) if t == b));
        // the header's menu
        let fe = folder_entries(&m, f);
        let fl: Vec<&str> = fe.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(fl, ["Rename Folder", "Collapse", "Ungroup", "Close Folder"]);
        assert_eq!(fe[3].run, Run::Model(Command::CloseTabs(vec![b])));
        assert!(folder_entries(&m, FolderId(99)).is_empty());
        // a pinned tab has no folder rows
        let (m, _) = apply(m, Command::PinTab(a));
        assert!(!labels(&m, a).iter().any(|l| l.contains("Folder")));
    }

    #[test]
    fn separators_never_lead_trail_or_double() {
        let m = model(1);
        let e = entries(&m, TabId(1), Facts::default());
        assert!(!e.is_empty());
        assert_ne!(e.first().map(|e| &e.run), Some(&Run::Separator));
        assert_ne!(e.last().map(|e| &e.run), Some(&Run::Separator));
        assert!(
            e.windows(2)
                .all(|w| !(w[0].run == Run::Separator && w[1].run == Run::Separator))
        );
    }

    #[test]
    fn unknown_tab_has_no_menu() {
        assert!(entries(&model(1), TabId(99), Facts::default()).is_empty());
    }
}
