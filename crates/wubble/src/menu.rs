//! The page's context menu: what it lists ([`entries`], one table of rows)
//! and its lifecycle ([`transition`]). Pure; the host draws [`Menu`] and
//! executes [`MenuEffect`]s.
//!
//! Which entries show for a link, an image, a selection, an editable field
//! or the bare page is Chromium's policy (`chrome/browser/renderer_context_menu/
//! render_view_context_menu.cc`: `AppendLinkItems`, `AppendImageItems`,
//! `AppendEditableItems`, `AppendCopyItem`, `AppendPageItems`; BSD-3-Clause)
//! and Firefox's (`browser/base/content/nsContextMenu.sys.mjs`:
//! `initOpenItems`, `initClipboardItems`; MPL-2.0), copied as the rows of
//! [`ROWS`], not as code. Command ids are CEF's `cef_menu_id_t`
//! (`include/internal/cef_types.h`, BSD-3-Clause): CEF runs those itself
//! when the answer names them.
//!
//! Every request is answered exactly once: choosing, dismissing and a newer
//! request each answer; CEF dropping the menu or the view going answer
//! nothing (welding already gave the default).

use wubble_engine::page::{ContextMenu, MediaKind, RequestId};

/// CEF's `cef_menu_id_t` values the menu uses.
pub mod cef_id {
    pub const BACK: i32 = 100;
    pub const FORWARD: i32 = 101;
    pub const RELOAD: i32 = 102;
    pub const UNDO: i32 = 110;
    pub const REDO: i32 = 111;
    pub const CUT: i32 = 112;
    pub const COPY: i32 = 113;
    pub const PASTE: i32 = 114;
    pub const DELETE: i32 = 116;
    pub const SELECT_ALL: i32 = 117;
    /// The first spelling suggestion; CEF numbers up to five from here.
    pub const SPELLCHECK_SUGGESTION_0: i32 = 200;
    pub const SPELLCHECK_SUGGESTION_LAST: i32 = 204;
}

/// What choosing an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A CEF menu command: the answer names it and CEF runs it.
    Engine(i32),
    /// Open the URL in a new tab next to the page's.
    OpenInNewTab(String),
    /// Put the text on the clipboard.
    CopyText(String),
    /// A divider between groups; never chosen.
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub command: Command,
    pub enabled: bool,
}

/// What the page's navigation allows (from the view's `NavState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavFlags {
    pub back: bool,
    pub forward: bool,
}

/// The context a row's predicates read.
struct Ctx<'a> {
    menu: &'a ContextMenu,
    nav: NavFlags,
}

impl Ctx<'_> {
    fn link(&self) -> bool {
        !self.menu.link_url.is_empty()
    }
    fn image(&self) -> bool {
        self.menu.media == MediaKind::Image && !self.menu.media_url.is_empty()
    }
    fn editable(&self) -> bool {
        self.menu.editable
    }
    fn selection(&self) -> bool {
        !self.menu.editable && !self.menu.selection_text.is_empty()
    }
    /// Nothing more specific was hit: the page's own entries.
    fn page(&self) -> bool {
        !self.link() && !self.image() && !self.editable() && !self.selection()
    }
    fn edit(&self, flag: u32) -> bool {
        self.menu.edit.allows(flag)
    }
}

/// One row of the table. `group` separates entries: a divider goes between
/// two shown groups.
struct Row {
    group: u8,
    label: &'static str,
    show: fn(&Ctx) -> bool,
    enabled: fn(&Ctx) -> bool,
    command: fn(&Ctx) -> Command,
}

const fn always(_: &Ctx) -> bool {
    true
}

use wubble_engine::page::EditFlags as F;

/// Every fixed entry, in menu order. Spelling suggestions go first and come
/// from the request itself.
const ROWS: &[Row] = &[
    Row {
        group: 1,
        label: "Open Link in New Tab",
        show: |c| c.link(),
        enabled: always,
        command: |c| Command::OpenInNewTab(c.menu.link_url.clone()),
    },
    Row {
        group: 1,
        label: "Copy Link Address",
        show: |c| c.link(),
        enabled: always,
        command: |c| Command::CopyText(c.menu.link_url.clone()),
    },
    Row {
        group: 2,
        label: "Open Image in New Tab",
        show: |c| c.image(),
        enabled: always,
        command: |c| Command::OpenInNewTab(c.menu.media_url.clone()),
    },
    Row {
        group: 2,
        label: "Copy Image Address",
        show: |c| c.image(),
        enabled: always,
        command: |c| Command::CopyText(c.menu.media_url.clone()),
    },
    Row {
        group: 3,
        label: "Undo",
        show: |c| c.editable(),
        enabled: |c| c.edit(F::CAN_UNDO),
        command: |_| Command::Engine(cef_id::UNDO),
    },
    Row {
        group: 3,
        label: "Redo",
        show: |c| c.editable(),
        enabled: |c| c.edit(F::CAN_REDO),
        command: |_| Command::Engine(cef_id::REDO),
    },
    Row {
        group: 4,
        label: "Cut",
        show: |c| c.editable(),
        enabled: |c| c.edit(F::CAN_CUT),
        command: |_| Command::Engine(cef_id::CUT),
    },
    Row {
        group: 4,
        label: "Copy",
        show: |c| c.editable() || c.selection(),
        enabled: |c| c.selection() || c.edit(F::CAN_COPY),
        command: |_| Command::Engine(cef_id::COPY),
    },
    Row {
        group: 4,
        label: "Paste",
        show: |c| c.editable(),
        enabled: |c| c.edit(F::CAN_PASTE),
        command: |_| Command::Engine(cef_id::PASTE),
    },
    Row {
        group: 4,
        label: "Delete",
        show: |c| c.editable(),
        enabled: |c| c.edit(F::CAN_DELETE),
        command: |_| Command::Engine(cef_id::DELETE),
    },
    Row {
        group: 4,
        label: "Select All",
        show: |c| c.editable(),
        enabled: |c| c.edit(F::CAN_SELECT_ALL),
        command: |_| Command::Engine(cef_id::SELECT_ALL),
    },
    Row {
        group: 5,
        label: "Back",
        show: |c| c.page(),
        enabled: |c| c.nav.back,
        command: |_| Command::Engine(cef_id::BACK),
    },
    Row {
        group: 5,
        label: "Forward",
        show: |c| c.page(),
        enabled: |c| c.nav.forward,
        command: |_| Command::Engine(cef_id::FORWARD),
    },
    Row {
        group: 5,
        label: "Reload",
        show: |c| c.page(),
        enabled: always,
        command: |_| Command::Engine(cef_id::RELOAD),
    },
];

/// The entries for one right-click, dividers included.
pub fn entries(menu: &ContextMenu, nav: NavFlags) -> Vec<Entry> {
    let ctx = Ctx { menu, nav };
    // Group 0: CEF's spelling suggestions for a misspelled word in a field.
    let suggestions = (menu.editable && !menu.misspelled_word.is_empty())
        .then_some(menu.suggestions.iter())
        .into_iter()
        .flatten()
        .zip(cef_id::SPELLCHECK_SUGGESTION_0..=cef_id::SPELLCHECK_SUGGESTION_LAST)
        .map(|(word, id)| {
            (
                0,
                Entry {
                    label: word.clone(),
                    command: Command::Engine(id),
                    enabled: true,
                },
            )
        });
    let fixed = ROWS.iter().filter(|r| (r.show)(&ctx)).map(|r| {
        (
            r.group,
            Entry {
                label: r.label.into(),
                command: (r.command)(&ctx),
                enabled: (r.enabled)(&ctx),
            },
        )
    });
    let mut out: Vec<Entry> = vec![];
    let mut last = None;
    for (group, entry) in suggestions.chain(fixed) {
        if last.is_some_and(|g| g != group) {
            out.push(Entry {
                label: String::new(),
                command: Command::Separator,
                enabled: false,
            });
        }
        last = Some(group);
        out.push(entry);
    }
    out
}

/// The menu of one tab.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Menu {
    #[default]
    Closed,
    Open {
        id: RequestId,
        /// The pointer, physical pixels relative to the view.
        at: (i32, i32),
        entries: Vec<Entry>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    Requested {
        id: RequestId,
        menu: Box<ContextMenu>,
        nav: NavFlags,
    },
    /// The user picked entry `i`.
    Chosen(usize),
    /// Outside click, Escape, or the tab hiding it.
    Dismiss,
    /// CEF closed the menu itself (it already answered).
    Reset,
    /// The view closed: welding answered every held request.
    ViewGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEffect {
    Answer {
        id: RequestId,
        command: Option<i32>,
    },
    /// An app command, after the dismissing answer.
    Run(Command),
}

pub fn transition(state: Menu, event: MenuEvent) -> (Menu, Vec<MenuEffect>) {
    use MenuEvent::*;
    let dismiss = |id| MenuEffect::Answer { id, command: None };
    match (state, event) {
        (state, Requested { id, menu, nav }) => {
            // CEF holds one menu at a time; a newer one replaces the old.
            let mut effects = match state {
                Menu::Open { id: old, .. } => vec![dismiss(old)],
                Menu::Closed => vec![],
            };
            let entries = entries(&menu, nav);
            if entries.is_empty() {
                effects.push(dismiss(id));
                return (Menu::Closed, effects);
            }
            let open = Menu::Open {
                id,
                at: (menu.x, menu.y),
                entries,
            };
            (open, effects)
        }
        (Menu::Open { id, at, entries }, Chosen(i)) => {
            match entries.get(i).filter(|e| e.enabled).map(|e| &e.command) {
                None | Some(Command::Separator) => (Menu::Open { id, at, entries }, vec![]),
                Some(Command::Engine(command)) => (
                    Menu::Closed,
                    vec![MenuEffect::Answer {
                        id,
                        command: Some(*command),
                    }],
                ),
                Some(command) => (
                    Menu::Closed,
                    vec![dismiss(id), MenuEffect::Run(command.clone())],
                ),
            }
        }
        (Menu::Open { id, .. }, Dismiss) => (Menu::Closed, vec![dismiss(id)]),
        (_, Reset | ViewGone) => (Menu::Closed, vec![]),
        (Menu::Closed, Chosen(_) | Dismiss) => (Menu::Closed, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_engine::page::EditFlags;

    fn labels(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .map(|e| match e.command {
                Command::Separator => "-",
                _ => e.label.as_str(),
            })
            .collect()
    }

    fn page() -> ContextMenu {
        ContextMenu {
            x: 10,
            y: 20,
            page_url: "https://a.test/".into(),
            ..Default::default()
        }
    }

    fn link() -> ContextMenu {
        ContextMenu {
            link_url: "https://a.test/next".into(),
            ..page()
        }
    }

    fn requested(id: u32, menu: ContextMenu) -> MenuEvent {
        MenuEvent::Requested {
            id: RequestId(id),
            menu: Box::new(menu),
            nav: NavFlags::default(),
        }
    }

    #[test]
    fn the_bare_page_offers_navigation() {
        let nav = NavFlags {
            back: true,
            forward: false,
        };
        let e = entries(&page(), nav);
        assert_eq!(labels(&e), ["Back", "Forward", "Reload"]);
        assert_eq!(
            e.iter().map(|e| e.enabled).collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(e[0].command, Command::Engine(cef_id::BACK));
    }

    #[test]
    fn a_link_offers_new_tab_and_copy() {
        let e = entries(&link(), NavFlags::default());
        assert_eq!(labels(&e), ["Open Link in New Tab", "Copy Link Address"]);
        assert_eq!(
            e[0].command,
            Command::OpenInNewTab("https://a.test/next".into())
        );
        assert_eq!(
            e[1].command,
            Command::CopyText("https://a.test/next".into())
        );
    }

    #[test]
    fn a_linked_image_lists_both_groups() {
        let m = ContextMenu {
            media: MediaKind::Image,
            media_url: "https://a.test/i.png".into(),
            has_image: true,
            ..link()
        };
        let e = entries(&m, NavFlags::default());
        assert_eq!(
            labels(&e),
            [
                "Open Link in New Tab",
                "Copy Link Address",
                "-",
                "Open Image in New Tab",
                "Copy Image Address"
            ]
        );
    }

    #[test]
    fn a_selection_offers_copy() {
        let m = ContextMenu {
            selection_text: "hello".into(),
            ..page()
        };
        let e = entries(&m, NavFlags::default());
        assert_eq!(labels(&e), ["Copy"]);
        assert!(e[0].enabled);
    }

    #[test]
    fn editable_entries_follow_every_flag() {
        let all = [
            ("Undo", EditFlags::CAN_UNDO),
            ("Redo", EditFlags::CAN_REDO),
            ("Cut", EditFlags::CAN_CUT),
            ("Copy", EditFlags::CAN_COPY),
            ("Paste", EditFlags::CAN_PASTE),
            ("Delete", EditFlags::CAN_DELETE),
            ("Select All", EditFlags::CAN_SELECT_ALL),
        ];
        for bits in 0..(1u32 << all.len()) {
            let m = ContextMenu {
                editable: true,
                edit: EditFlags(bits),
                ..page()
            };
            let e = entries(&m, NavFlags::default());
            assert_eq!(
                labels(&e),
                [
                    "Undo",
                    "Redo",
                    "-",
                    "Cut",
                    "Copy",
                    "Paste",
                    "Delete",
                    "Select All"
                ]
            );
            for (label, flag) in all {
                let entry = e.iter().find(|e| e.label == label).unwrap();
                assert_eq!(entry.enabled, bits & flag != 0, "{label} with {bits:#b}");
            }
        }
    }

    #[test]
    fn a_misspelled_word_lists_suggestions_first() {
        let m = ContextMenu {
            editable: true,
            misspelled_word: "teh".into(),
            suggestions: (0..7).map(|i| format!("s{i}")).collect(),
            ..page()
        };
        let e = entries(&m, NavFlags::default());
        // CEF numbers five suggestions.
        assert_eq!(labels(&e)[..6], ["s0", "s1", "s2", "s3", "s4", "-"]);
        assert_eq!(
            e[4].command,
            Command::Engine(cef_id::SPELLCHECK_SUGGESTION_LAST)
        );
        // Suggestions without a misspelled word are not shown.
        let m = ContextMenu {
            misspelled_word: String::new(),
            ..m
        };
        assert_eq!(labels(&entries(&m, NavFlags::default()))[0], "Undo");
    }

    #[test]
    fn engine_commands_answer_with_the_command() {
        let (s, fx) = transition(Menu::Closed, requested(1, page()));
        assert!(fx.is_empty());
        let (s, fx) = transition(s, MenuEvent::Chosen(2));
        assert_eq!(s, Menu::Closed);
        assert_eq!(
            fx,
            [MenuEffect::Answer {
                id: RequestId(1),
                command: Some(cef_id::RELOAD)
            }]
        );
    }

    #[test]
    fn app_commands_dismiss_then_run() {
        let (s, _) = transition(Menu::Closed, requested(1, link()));
        let (s, fx) = transition(s, MenuEvent::Chosen(0));
        assert_eq!(s, Menu::Closed);
        assert_eq!(
            fx,
            [
                MenuEffect::Answer {
                    id: RequestId(1),
                    command: None
                },
                MenuEffect::Run(Command::OpenInNewTab("https://a.test/next".into()))
            ]
        );
    }

    #[test]
    fn disabled_entries_and_separators_are_not_chosen() {
        // Back and Forward are disabled without history.
        let (s, _) = transition(Menu::Closed, requested(1, page()));
        let (s, fx) = transition(s, MenuEvent::Chosen(0));
        assert!(fx.is_empty());
        let (s, fx) = transition(s, MenuEvent::Chosen(99));
        assert!(fx.is_empty());
        assert!(matches!(s, Menu::Open { .. }));
    }

    #[test]
    fn every_request_is_answered_at_most_once() {
        use MenuEvent::*;
        let events = || {
            vec![
                requested(1, page()),
                requested(2, link()),
                Chosen(0),
                Chosen(0),
                Dismiss,
                requested(3, page()),
                Dismiss,
                Dismiss,
                requested(4, page()),
                Reset,
                Dismiss,
                requested(5, page()),
                ViewGone,
                Chosen(2),
            ]
        };
        let mut s = Menu::Closed;
        let mut answered = vec![];
        for e in events() {
            let (next, fx) = transition(s, e);
            s = next;
            for f in fx {
                if let MenuEffect::Answer { id, .. } = f {
                    assert!(!answered.contains(&id), "{id:?} answered twice");
                    answered.push(id);
                }
            }
        }
        // 4 and 5 were answered by CEF / welding, not by the menu.
        assert_eq!(answered, [RequestId(1), RequestId(2), RequestId(3)]);
    }
}
