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
//!
//! A tab row's menu ([`tab_entries`]) runs through the same machine as a
//! menu the shell opened itself ([`Menu::Shell`]): nothing to answer.

use wubble_core::action::Action;
use wubble_core::model::Model;
use wubble_core::space::MAX_PANES;
use wubble_core::tab::TabId;
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
    pub const PASTE_MATCH_STYLE: i32 = 115;
    pub const DELETE: i32 = 116;
    pub const SELECT_ALL: i32 = 117;
    /// The first spelling suggestion; CEF numbers up to five from here.
    pub const SPELLCHECK_SUGGESTION_0: i32 = 200;
    pub const SPELLCHECK_SUGGESTION_LAST: i32 = 204;
}

/// What a media entry does to the element under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaOp {
    /// Play if paused, else pause.
    PlayPause,
    /// Toggle muted.
    Mute,
    /// Toggle looping.
    Loop,
    /// Toggle the element's own controls.
    Controls,
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
    /// Download the URL with the page's cookies, asking where to save it.
    SaveAs(String),
    /// Search the default engine for the text.
    Search(String),
    /// Open DevTools on the element at this point (physical px, view-relative).
    Inspect { x: i32, y: i32 },
    /// Act on the media element at this point (physical px, view-relative).
    Media { op: MediaOp, x: i32, y: i32 },
    /// Put the tab beside the focused page (split view).
    SplitWith(TabId),
    /// Take the tab out of its split.
    Unsplit(TabId),
    /// Open the command bar to choose a page beside the focused one.
    OpenBeside,
    /// Run an action, as from a key.
    Perform(Action),
    /// A divider between groups; never chosen.
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub command: Command,
    pub enabled: bool,
}

/// What the menu needs besides the click: the page's navigation (from the
/// view's `NavState`) and the search engine's name, if one is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavFlags {
    pub back: bool,
    pub forward: bool,
    pub search: Option<&'static str>,
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
    /// A video or audio element.
    fn media(&self) -> bool {
        matches!(self.menu.media, MediaKind::Video | MediaKind::Audio)
    }
    fn media_url(&self) -> bool {
        self.media() && !self.menu.media_url.is_empty()
    }
    fn media_has(&self, flag: u32) -> bool {
        self.menu.media_flags.has(flag)
    }
    fn video(&self) -> bool {
        self.menu.media == MediaKind::Video
    }
    fn editable(&self) -> bool {
        self.menu.editable
    }
    fn selection(&self) -> bool {
        !self.menu.editable && !self.menu.selection_text.is_empty()
    }
    /// Nothing more specific was hit: the page's own entries.
    fn page(&self) -> bool {
        !self.link() && !self.image() && !self.media() && !self.editable() && !self.selection()
    }
    fn edit(&self, flag: u32) -> bool {
        self.menu.edit.allows(flag)
    }
    fn at(&self) -> (i32, i32) {
        (self.menu.x, self.menu.y)
    }
    fn media_op(&self, op: MediaOp) -> Command {
        let (x, y) = self.at();
        Command::Media { op, x, y }
    }
    /// "Video" or "Audio", for the media entries' labels.
    fn kind(&self) -> &'static str {
        if self.video() { "Video" } else { "Audio" }
    }
}

/// Chromium's `kMaxSelectionTextLength` for the search entry's label is 50
/// characters; longer selections end in an ellipsis.
const SEARCH_LABEL_CHARS: usize = 50;

/// `Search <engine> for “<selection>”`, the selection shortened and on one line.
pub fn search_label(engine: Option<&str>, text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = flat.chars().take(SEARCH_LABEL_CHARS).collect();
    let more = if flat.chars().count() > SEARCH_LABEL_CHARS {
        "\u{2026}"
    } else {
        ""
    };
    format!(
        "Search {} for \u{201c}{short}{more}\u{201d}",
        engine.unwrap_or("the Web")
    )
}

/// The script that does `op` to the video or audio element at `x, y`
/// (physical px, view-relative). CEF has no media-player API, so this is
/// Chromium's `MediaPlayerAction` done from the page's side; physical px are
/// CSS px times `devicePixelRatio` (the device scale times the zoom).
pub fn media_script(op: MediaOp, x: i32, y: i32) -> String {
    let act = match op {
        MediaOp::PlayPause => "m.paused ? m.play() : m.pause()",
        MediaOp::Mute => "m.muted = !m.muted",
        MediaOp::Loop => "m.loop = !m.loop",
        MediaOp::Controls => "m.controls = !m.controls",
    };
    format!(
        "(() => {{ const e = document.elementFromPoint({x} / devicePixelRatio, {y} / devicePixelRatio); \
         const m = e && e.closest('video, audio'); if (m) {{ {act}; }} }})()"
    )
}

/// One row of the table. `group` separates entries: a divider goes between
/// two shown groups.
struct Row {
    group: u8,
    label: fn(&Ctx) -> String,
    show: fn(&Ctx) -> bool,
    enabled: fn(&Ctx) -> bool,
    command: fn(&Ctx) -> Command,
}

const fn always(_: &Ctx) -> bool {
    true
}

use wubble_engine::page::EditFlags as F;
use wubble_engine::page::MediaFlags as M;

/// A row with a fixed label.
macro_rules! row {
    ($group:literal, $label:literal, $show:expr, $enabled:expr, $command:expr) => {
        Row {
            group: $group,
            label: |_| $label.into(),
            show: $show,
            enabled: $enabled,
            command: $command,
        }
    };
}

/// Every fixed entry, in menu order (Chromium's `AppendLinkItems`,
/// `AppendImageItems`, `AppendMediaItems`, `AppendEditableItems`,
/// `AppendCopyItem`, `AppendSearchProvider`, `AppendPageItems`,
/// `AppendDeveloperItems`). Spelling suggestions go first and come from the
/// request itself. Left out, with the reason in TODO.md: "Open Link in New
/// Window" (one window), "Copy Image" (the clipboard carries text only),
/// "Picture in Picture" (CEF's windowless views have no PiP window).
const ROWS: &[Row] = &[
    row!(1, "Open Link in New Tab", |c| c.link(), always, |c| {
        Command::OpenInNewTab(c.menu.link_url.clone())
    }),
    row!(1, "Save Link As\u{2026}", |c| c.link(), always, |c| {
        Command::SaveAs(c.menu.link_url.clone())
    }),
    row!(1, "Copy Link Address", |c| c.link(), always, |c| {
        Command::CopyText(c.menu.link_url.clone())
    }),
    row!(2, "Open Image in New Tab", |c| c.image(), always, |c| {
        Command::OpenInNewTab(c.menu.media_url.clone())
    }),
    row!(2, "Save Image As\u{2026}", |c| c.image(), always, |c| {
        Command::SaveAs(c.menu.media_url.clone())
    }),
    row!(2, "Copy Image Address", |c| c.image(), always, |c| {
        Command::CopyText(c.menu.media_url.clone())
    }),
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::PAUSED) {
                "Play"
            } else {
                "Pause"
            }
            .into()
        },
        show: |c| c.media(),
        enabled: always,
        command: |c| c.media_op(MediaOp::PlayPause),
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::MUTED) {
                "Unmute"
            } else {
                "Mute"
            }
            .into()
        },
        show: |c| c.media() && c.media_has(M::HAS_AUDIO),
        enabled: always,
        command: |c| c.media_op(MediaOp::Mute),
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::LOOP) {
                "Stop Looping"
            } else {
                "Loop"
            }
            .into()
        },
        show: |c| c.media(),
        enabled: always,
        command: |c| c.media_op(MediaOp::Loop),
    },
    Row {
        group: 3,
        label: |c| {
            if c.media_has(M::CONTROLS) {
                "Hide Controls"
            } else {
                "Show Controls"
            }
            .into()
        },
        show: |c| c.media(),
        enabled: |c| c.media_has(M::CAN_TOGGLE_CONTROLS),
        command: |c| c.media_op(MediaOp::Controls),
    },
    Row {
        group: 4,
        label: |c| format!("Open {} in New Tab", c.kind()),
        show: |c| c.media_url(),
        enabled: always,
        command: |c| Command::OpenInNewTab(c.menu.media_url.clone()),
    },
    Row {
        group: 4,
        label: |c| format!("Save {} As\u{2026}", c.kind()),
        show: |c| c.media_url(),
        enabled: |c| c.media_has(M::CAN_SAVE),
        command: |c| Command::SaveAs(c.menu.media_url.clone()),
    },
    Row {
        group: 4,
        label: |c| format!("Copy {} Address", c.kind()),
        show: |c| c.media_url(),
        enabled: always,
        command: |c| Command::CopyText(c.menu.media_url.clone()),
    },
    row!(5, "Undo", |c| c.editable(), |c| c.edit(F::CAN_UNDO), |_| {
        Command::Engine(cef_id::UNDO)
    }),
    row!(5, "Redo", |c| c.editable(), |c| c.edit(F::CAN_REDO), |_| {
        Command::Engine(cef_id::REDO)
    }),
    row!(6, "Cut", |c| c.editable(), |c| c.edit(F::CAN_CUT), |_| {
        Command::Engine(cef_id::CUT)
    }),
    row!(
        6,
        "Copy",
        |c| c.editable() || c.selection(),
        |c| c.selection() || c.edit(F::CAN_COPY),
        |_| { Command::Engine(cef_id::COPY) }
    ),
    row!(
        6,
        "Paste",
        |c| c.editable(),
        |c| c.edit(F::CAN_PASTE),
        |_| { Command::Engine(cef_id::PASTE) }
    ),
    row!(
        6,
        "Paste as Plain Text",
        |c| c.editable(),
        |c| c.edit(F::CAN_PASTE),
        |_| { Command::Engine(cef_id::PASTE_MATCH_STYLE) }
    ),
    row!(
        6,
        "Delete",
        |c| c.editable(),
        |c| c.edit(F::CAN_DELETE),
        |_| { Command::Engine(cef_id::DELETE) }
    ),
    row!(
        6,
        "Select All",
        |c| c.editable(),
        |c| c.edit(F::CAN_SELECT_ALL),
        |_| { Command::Engine(cef_id::SELECT_ALL) }
    ),
    Row {
        group: 7,
        label: |c| search_label(c.nav.search, &c.menu.selection_text),
        show: |c| !c.menu.selection_text.trim().is_empty(),
        enabled: always,
        command: |c| Command::Search(c.menu.selection_text.clone()),
    },
    row!(7, "Print\u{2026}", |c| c.selection(), always, |_| {
        Command::Perform(Action::Print)
    }),
    row!(
        8,
        "Back",
        |c| c.page(),
        |c| c.nav.back,
        |_| Command::Engine(cef_id::BACK)
    ),
    row!(8, "Forward", |c| c.page(), |c| c.nav.forward, |_| {
        Command::Engine(cef_id::FORWARD)
    }),
    row!(8, "Reload", |c| c.page(), always, |_| Command::Engine(
        cef_id::RELOAD
    )),
    row!(9, "Save Page As\u{2026}", |c| c.page(), always, |c| {
        Command::SaveAs(c.menu.page_url.clone())
    }),
    row!(9, "Print\u{2026}", |c| c.page(), always, |_| {
        Command::Perform(Action::Print)
    }),
    row!(10, "View Page Source", |c| c.page(), always, |_| {
        Command::Perform(Action::ViewSource)
    }),
    row!(10, "Inspect", always, always, |c| {
        let (x, y) = c.at();
        Command::Inspect { x, y }
    }),
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
                label: (r.label)(&ctx),
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

/// The split entries of `tab`'s row (the design has no tab menu; these are its command-bar
/// rows, `browser/command.tsx`, for the row's tab): the focused page splits with the next tab or
/// a page chosen beside it; another tab of the Space goes beside the focused page; a pane leaves
/// its split.
pub fn tab_entries(model: &Model, tab: TabId) -> Vec<Entry> {
    let entry = |label: &str, command| Entry {
        label: label.into(),
        command,
        enabled: true,
    };
    if model.split_of(tab).is_some() {
        return vec![entry("Take Out of Split View", Command::Unsplit(tab))];
    }
    let room = model.visible_tabs().len() < MAX_PANES;
    let in_space = model.order(model.active_space).contains(&tab);
    if !room || !in_space {
        return vec![];
    }
    if model.visible_tab() == Some(tab) {
        let mut out = vec![];
        if model.split_candidate().is_some() {
            out.push(entry(
                "Split View with the Next Tab",
                Command::Perform(Action::SplitNext),
            ));
        }
        out.push(entry("Open Beside\u{2026}", Command::OpenBeside));
        out
    } else {
        vec![entry("Open in Split View", Command::SplitWith(tab))]
    }
}

/// The menu of one tab, or one the shell opened.
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
    /// The shell's own menu (a tab row's): no request to answer.
    Shell {
        /// The pointer, physical pixels in the window.
        at: (i32, i32),
        entries: Vec<Entry>,
    },
}

impl Menu {
    /// Whether the page keeps keyboard focus while this menu holds the shell's: a page's own
    /// menu is part of it, as Chromium's native menu, which never blurs the page. Blurring it
    /// would make CEF cancel the request the menu is about to answer (`SetFocus(false)` calls
    /// `CancelContextMenu`), closing the menu the moment it takes focus.
    pub fn keeps_page_focus(&self) -> bool {
        matches!(self, Menu::Open { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    Requested {
        id: RequestId,
        menu: Box<ContextMenu>,
        nav: NavFlags,
    },
    /// The shell opens its own menu.
    ShellRequested { at: (i32, i32), entries: Vec<Entry> },
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
                Menu::Closed | Menu::Shell { .. } => vec![],
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
        (state, ShellRequested { at, entries }) => {
            let effects = match state {
                Menu::Open { id, .. } => vec![dismiss(id)],
                Menu::Closed | Menu::Shell { .. } => vec![],
            };
            if entries.is_empty() {
                return (Menu::Closed, effects);
            }
            (Menu::Shell { at, entries }, effects)
        }
        (Menu::Shell { at, entries }, Chosen(i)) => {
            match entries.get(i).filter(|e| e.enabled).map(|e| &e.command) {
                None | Some(Command::Separator) => (Menu::Shell { at, entries }, vec![]),
                Some(command) => (Menu::Closed, vec![MenuEffect::Run(command.clone())]),
            }
        }
        (_, Reset | ViewGone) => (Menu::Closed, vec![]),
        (Menu::Closed | Menu::Shell { .. }, Dismiss) | (Menu::Closed, Chosen(_)) => {
            (Menu::Closed, vec![])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_engine::page::{EditFlags, MediaFlags};

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
            search: None,
        };
        let e = entries(&page(), nav);
        assert_eq!(
            labels(&e),
            [
                "Back",
                "Forward",
                "Reload",
                "-",
                "Save Page As\u{2026}",
                "Print\u{2026}",
                "-",
                "View Page Source",
                "Inspect"
            ]
        );
        assert_eq!(
            e.iter().take(3).map(|e| e.enabled).collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(e[4].command, Command::SaveAs("https://a.test/".into()));
        assert_eq!(e[7].command, Command::Perform(Action::ViewSource));
        assert_eq!(e[0].command, Command::Engine(cef_id::BACK));
    }

    #[test]
    fn a_link_offers_new_tab_and_copy() {
        let e = entries(&link(), NavFlags::default());
        assert_eq!(
            labels(&e),
            [
                "Open Link in New Tab",
                "Save Link As\u{2026}",
                "Copy Link Address",
                "-",
                "Inspect"
            ]
        );
        assert_eq!(
            e[0].command,
            Command::OpenInNewTab("https://a.test/next".into())
        );
        assert_eq!(e[1].command, Command::SaveAs("https://a.test/next".into()));
        assert_eq!(
            e[2].command,
            Command::CopyText("https://a.test/next".into())
        );
        assert_eq!(e[4].command, Command::Inspect { x: 10, y: 20 });
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
                "Save Link As\u{2026}",
                "Copy Link Address",
                "-",
                "Open Image in New Tab",
                "Save Image As\u{2026}",
                "Copy Image Address",
                "-",
                "Inspect"
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
        assert_eq!(
            labels(&e),
            [
                "Copy",
                "-",
                "Search the Web for \u{201c}hello\u{201d}",
                "Print\u{2026}",
                "-",
                "Inspect"
            ]
        );
        assert!(e[0].enabled);
        assert_eq!(e[2].command, Command::Search("hello".into()));
        let named = NavFlags {
            search: Some("DuckDuckGo"),
            ..NavFlags::default()
        };
        assert_eq!(
            entries(&m, named)[2].label,
            "Search DuckDuckGo for \u{201c}hello\u{201d}"
        );
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
                    "Paste as Plain Text",
                    "Delete",
                    "Select All",
                    "-",
                    "Inspect"
                ]
            );
            let plain = e.iter().find(|e| e.label == "Paste as Plain Text").unwrap();
            assert_eq!(plain.enabled, bits & EditFlags::CAN_PASTE != 0);
            assert_eq!(plain.command, Command::Engine(cef_id::PASTE_MATCH_STYLE));
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
    fn the_search_label_is_one_short_line() {
        assert_eq!(
            search_label(None, "  two\n words "),
            "Search the Web for \u{201c}two words\u{201d}"
        );
        let long = "x".repeat(60);
        let label = search_label(Some("Google"), &long);
        assert!(label.ends_with("\u{2026}\u{201d}"), "{label}");
        assert_eq!(
            label.chars().filter(|&c| c == 'x').count(),
            SEARCH_LABEL_CHARS
        );
    }

    #[test]
    fn media_entries_follow_the_element_state() {
        let video = |flags| ContextMenu {
            media: MediaKind::Video,
            media_url: "https://a.test/v.webm".into(),
            media_flags: MediaFlags(flags),
            x: 7,
            y: 9,
            ..page()
        };
        let e = entries(
            &video(MediaFlags::PAUSED | MediaFlags::CAN_TOGGLE_CONTROLS),
            NavFlags::default(),
        );
        assert_eq!(
            labels(&e),
            [
                "Play",
                "Loop",
                "Show Controls",
                "-",
                "Open Video in New Tab",
                "Save Video As\u{2026}",
                "Copy Video Address",
                "-",
                "Inspect"
            ]
        );
        assert_eq!(
            e[0].command,
            Command::Media {
                op: MediaOp::PlayPause,
                x: 7,
                y: 9
            }
        );
        assert!(e[2].enabled);
        // Saving needs CAN_SAVE.
        assert!(!e[5].enabled);
        let all =
            MediaFlags::HAS_AUDIO | MediaFlags::MUTED | MediaFlags::LOOP | MediaFlags::CONTROLS;
        let e = entries(&video(all), NavFlags::default());
        assert_eq!(
            labels(&e)[..4],
            ["Pause", "Unmute", "Stop Looping", "Hide Controls"]
        );
        assert!(!e[3].enabled);
    }

    #[test]
    fn media_scripts_act_on_the_element_under_the_point() {
        let s = media_script(MediaOp::Loop, 30, 40);
        assert!(
            s.contains("elementFromPoint(30 / devicePixelRatio, 40 / devicePixelRatio)"),
            "{s}"
        );
        assert!(s.contains("m.loop = !m.loop"), "{s}");
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
    fn only_the_page_s_own_menu_keeps_its_focus() {
        let open = transition(Menu::Closed, requested(1, link())).0;
        assert!(open.keeps_page_focus());
        assert!(!Menu::Closed.keeps_page_focus());
        let shell = Menu::Shell {
            at: (0, 0),
            entries: vec![],
        };
        assert!(
            !shell.keeps_page_focus(),
            "the shell's menu is not the page's"
        );
        // CEF closed it (window blurred, tab hidden): the page is free to blur.
        assert!(!transition(open, MenuEvent::Reset).0.keeps_page_focus());
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

    fn tabs(n: usize) -> Model {
        use wubble_core::model::{Command as Cmd, apply};
        (0..n).fold(Model::new(std::time::Instant::now()), |m, i| {
            apply(
                m,
                Cmd::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            )
            .0
        })
    }

    #[test]
    fn a_tab_row_offers_the_split_entries_that_apply() {
        use wubble_core::model::{Command as Cmd, apply};
        let commands = |m: &Model, t: u64| -> Vec<Command> {
            tab_entries(m, TabId(t))
                .into_iter()
                .map(|e| e.command)
                .collect()
        };
        let m = tabs(1);
        assert_eq!(commands(&m, 1), [Command::OpenBeside]);
        let m = tabs(3);
        assert_eq!(
            commands(&m, 3),
            [Command::Perform(Action::SplitNext), Command::OpenBeside]
        );
        assert_eq!(commands(&m, 1), [Command::SplitWith(TabId(1))]);
        let (m, _) = apply(m, Cmd::SplitWith(TabId(1)));
        assert_eq!(commands(&m, 1), [Command::Unsplit(TabId(1))]);
        assert_eq!(commands(&m, 3), [Command::Unsplit(TabId(3))]);
        let (m, _) = apply(m, Cmd::SplitWith(TabId(2)));
        assert_eq!(commands(&m, 2), [Command::Unsplit(TabId(2))]);
        // a full split takes no more
        let (m, _) = apply(m, Cmd::Unsplit(TabId(2)));
        let (m, _) = apply(m, Cmd::SplitWith(TabId(2)));
        assert!(commands(&m, 99).is_empty());
    }

    #[test]
    fn the_shell_s_menu_runs_its_entry_and_answers_nothing() {
        let entries = tab_entries(&tabs(3), TabId(1));
        let (open, fx) = transition(
            Menu::Closed,
            MenuEvent::ShellRequested {
                at: (5, 6),
                entries: entries.clone(),
            },
        );
        assert!(fx.is_empty());
        assert_eq!(
            open,
            Menu::Shell {
                at: (5, 6),
                entries
            }
        );
        let (closed, fx) = transition(open.clone(), MenuEvent::Chosen(0));
        assert_eq!(closed, Menu::Closed);
        assert_eq!(fx, [MenuEffect::Run(Command::SplitWith(TabId(1)))]);
        assert_eq!(transition(open.clone(), MenuEvent::Chosen(9)).0, open);
        assert_eq!(transition(open, MenuEvent::Dismiss), (Menu::Closed, vec![]));
        // nothing to show: no menu
        let (state, fx) = transition(
            Menu::Closed,
            MenuEvent::ShellRequested {
                at: (0, 0),
                entries: vec![],
            },
        );
        assert_eq!((state, fx), (Menu::Closed, vec![]));
        // the shell's menu replaces a page's, which is answered
        let (page, _) = transition(Menu::Closed, requested(1, page()));
        let (_, fx) = transition(
            page,
            MenuEvent::ShellRequested {
                at: (0, 0),
                entries: tab_entries(&tabs(1), TabId(1)),
            },
        );
        assert_eq!(
            fx,
            [MenuEffect::Answer {
                id: RequestId(1),
                command: None
            }]
        );
    }
}
