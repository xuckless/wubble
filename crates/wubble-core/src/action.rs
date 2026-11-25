//! Everything the user can trigger, defined once.
//!
//! Keybindings (`keys`) and the command bar (`omnibox`) both speak this enum.
//! Executing an action belongs to the shell wiring, not here.

/// One user-triggerable action. `SelectTab` carries 1..=8 (9 is
/// `SelectLastTab`), `SelectSpace` carries 1..=10 (10 is the `0` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    CloseTab,
    DuplicateTab,
    ReopenClosedTab,
    /// Mod+Z: the focused field's undo, else the last tab closed this run back
    /// (`closed`, Safari's Undo Close Tab).
    UndoCloseTab,
    NewWindow,
    /// Mod+Shift+N: a private window (Chrome's incognito window, `IDC_NEW_INCOGNITO_WINDOW`).
    NewPrivateWindow,
    /// Close the focused window (the last Main window quits).
    CloseWindow,
    /// Minimize the focused window (Cmd+M on macOS, Chromium `IDC_MINIMIZE_WINDOW`).
    MinimizeWindow,
    /// Focus the next or previous of wubble's windows (Cmd+` and Cmd+Shift+` on macOS).
    NextWindow,
    PrevWindow,
    /// Move the focused page into a new window.
    MoveTabToNewWindow,
    /// Mod+Shift+A: the command bar on every open and recently closed tab (Chrome's Tab Search).
    SearchTabs,
    FocusAddress,
    /// Ctrl+E: the command bar on the visible tab, every query a search (Chromium
    /// `IDC_FOCUS_SEARCH`, the omnibox's `?` prefix).
    FocusSearch,
    /// F6: keyboard focus to the next region of the window (`region`).
    NextRegion,
    /// Shift+F6: keyboard focus to the region before.
    PrevRegion,
    Reload,
    HardReload,
    /// DevTools' "Empty cache and hard reload": the cache cleared, then a hard reload.
    EmptyCacheHardReload,
    Stop,
    Back,
    Forward,
    NextTab,
    PrevTab,
    /// Move the visible tab one place up or down, inside its pinned row, its folder or the
    /// unpinned list (Chromium `IDC_MOVE_TAB_PREVIOUS` / `IDC_MOVE_TAB_NEXT`).
    MoveTabUp,
    MoveTabDown,
    SelectTab(u8),
    SelectLastTab,
    SelectSpace(u8),
    NewSpace,
    NextSpace,
    PrevSpace,
    ToggleSidebar,
    Find,
    FindNext,
    FindPrev,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    DevTools,
    /// DevTools on its Console panel (Chromium `IDC_DEV_TOOLS_CONSOLE`).
    DevToolsConsole,
    ViewSource,
    Fullscreen,
    TogglePinTab,
    CopyUrl,
    /// Load the clipboard's text in the visible tab, as typed in the address (Chromium
    /// `IDC_PASTE_AND_GO`, the omnibox menu's "Paste and go"); Chrome binds no key to it.
    PasteAndGo,
    /// Save the visible page as a PDF in the downloads folder.
    Print,
    /// Save Page As: download the visible page, asking where (Chromium `IDC_SAVE_PAGE`).
    SavePage,
    /// Open File: local files chosen in the OS dialog, each in a new tab (`IDC_OPEN_FILE`).
    OpenFile,
    /// F7: caret browsing on or off (Chromium `IDC_CARET_BROWSING_TOGGLE`).
    ToggleCaretBrowsing,
    /// Mute or unmute the visible tab (also the sidebar's speaker glyph).
    ToggleMute,
    /// Put the next tab down beside the focused page, or open a page beside it.
    SplitNext,
    /// Take the focused page out of its split.
    Unsplit,
    /// Show the settings page (`pages::InternalPage::Settings`).
    OpenSettings,
    /// Show the history page (`pages::InternalPage::History`).
    OpenHistory,
    /// Show the bookmarks page.
    OpenBookmarks,
    /// Show the downloads page.
    OpenDownloads,
    /// Show the passwords page.
    OpenPasswords,
    /// Show the archive page (tabs not visited for a while).
    OpenArchive,
    /// Show the Task manager (`pages::InternalPage::Tasks`).
    OpenTasks,
    /// Show the agent activity log (`pages::InternalPage::AgentLog`).
    OpenAgentLog,
    /// End every agent session (`agent::Event::Stop`): the kill switch, which works while a
    /// session runs headless.
    StopAgents,
    /// Bookmark the visible page: opens the name and folder sheet.
    BookmarkPage,
    /// Group the Space's tabs into folders by site.
    TidyTabs,
    /// Put the tabs back as they were before the last tidy.
    UndoTidyTabs,
    /// Close the Space's tabs that repeat another tab's address.
    CloseDuplicateTabs,
    /// The visible site's page theme: as the site shows it, dark, light (`pagetheme`).
    CyclePageTheme,
    /// Labels on the page's links; typing one clicks it (`hints`).
    LinkHints,
    /// Show the page's article alone, or leave it (`reader`).
    ToggleReader,
    /// Drag a region of the page to capture (`shot`).
    ScreenshotRegion,
    ScreenshotVisible,
    ScreenshotFullPage,
    /// The Boost editor on a new Boost for the visible site (`boost`).
    NewBoost,
    /// Edit commands on the focused page (its frame's undo, cut, copy ...): Chromium's Edit menu.
    /// On macOS Blink leaves their chords to the browser, so these run them there.
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    /// Paste as plain text (Chromium `IDC_PASTE_AND_MATCH_STYLE`, CEF `PasteAndMatchStyle`).
    PasteAndMatchStyle,
    SelectAll,
    /// Widen or narrow the docked DevTools panel a step (its seam's keyboard nudge).
    DockWider,
    DockNarrower,
    /// Choose whose cookies and cache to delete (this container's, every
    /// container's) or which site to forget; the menu is the confirmation.
    ClearBrowsingData,
    Quit,
}

/// Name and value of every parameterless action: the one list `id`,
/// `from_id` and `all` share.
const FIXED: [(&str, Action); 83] = [
    ("new-tab", Action::NewTab),
    ("close-tab", Action::CloseTab),
    ("duplicate-tab", Action::DuplicateTab),
    ("reopen-closed-tab", Action::ReopenClosedTab),
    ("undo-close-tab", Action::UndoCloseTab),
    ("new-window", Action::NewWindow),
    ("new-private-window", Action::NewPrivateWindow),
    ("close-window", Action::CloseWindow),
    ("minimize-window", Action::MinimizeWindow),
    ("next-window", Action::NextWindow),
    ("prev-window", Action::PrevWindow),
    ("move-tab-to-new-window", Action::MoveTabToNewWindow),
    ("search-tabs", Action::SearchTabs),
    ("focus-address", Action::FocusAddress),
    ("focus-search", Action::FocusSearch),
    ("next-region", Action::NextRegion),
    ("prev-region", Action::PrevRegion),
    ("reload", Action::Reload),
    ("hard-reload", Action::HardReload),
    ("empty-cache-hard-reload", Action::EmptyCacheHardReload),
    ("stop", Action::Stop),
    ("back", Action::Back),
    ("forward", Action::Forward),
    ("next-tab", Action::NextTab),
    ("prev-tab", Action::PrevTab),
    ("move-tab-up", Action::MoveTabUp),
    ("move-tab-down", Action::MoveTabDown),
    ("select-last-tab", Action::SelectLastTab),
    ("new-space", Action::NewSpace),
    ("next-space", Action::NextSpace),
    ("prev-space", Action::PrevSpace),
    ("toggle-sidebar", Action::ToggleSidebar),
    ("find", Action::Find),
    ("find-next", Action::FindNext),
    ("find-prev", Action::FindPrev),
    ("zoom-in", Action::ZoomIn),
    ("zoom-out", Action::ZoomOut),
    ("zoom-reset", Action::ZoomReset),
    ("dev-tools", Action::DevTools),
    ("dev-tools-console", Action::DevToolsConsole),
    ("view-source", Action::ViewSource),
    ("fullscreen", Action::Fullscreen),
    ("toggle-pin-tab", Action::TogglePinTab),
    ("copy-url", Action::CopyUrl),
    ("paste-and-go", Action::PasteAndGo),
    ("print", Action::Print),
    ("save-page", Action::SavePage),
    ("open-file", Action::OpenFile),
    ("toggle-caret-browsing", Action::ToggleCaretBrowsing),
    ("toggle-mute", Action::ToggleMute),
    ("split-next", Action::SplitNext),
    ("unsplit", Action::Unsplit),
    ("open-settings", Action::OpenSettings),
    ("open-history", Action::OpenHistory),
    ("open-bookmarks", Action::OpenBookmarks),
    ("open-downloads", Action::OpenDownloads),
    ("open-passwords", Action::OpenPasswords),
    ("open-archive", Action::OpenArchive),
    ("open-tasks", Action::OpenTasks),
    ("open-agent-log", Action::OpenAgentLog),
    ("stop-agents", Action::StopAgents),
    ("bookmark-page", Action::BookmarkPage),
    ("tidy-tabs", Action::TidyTabs),
    ("undo-tidy-tabs", Action::UndoTidyTabs),
    ("close-duplicate-tabs", Action::CloseDuplicateTabs),
    ("cycle-page-theme", Action::CyclePageTheme),
    ("link-hints", Action::LinkHints),
    ("toggle-reader", Action::ToggleReader),
    ("screenshot", Action::ScreenshotRegion),
    ("screenshot-visible", Action::ScreenshotVisible),
    ("screenshot-full-page", Action::ScreenshotFullPage),
    ("new-boost", Action::NewBoost),
    ("undo", Action::Undo),
    ("redo", Action::Redo),
    ("cut", Action::Cut),
    ("copy", Action::Copy),
    ("paste", Action::Paste),
    ("paste-and-match-style", Action::PasteAndMatchStyle),
    ("select-all", Action::SelectAll),
    ("dock-wider", Action::DockWider),
    ("dock-narrower", Action::DockNarrower),
    ("clear-browsing-data", Action::ClearBrowsingData),
    ("quit", Action::Quit),
];

impl Action {
    /// Stable kebab-case name, persisted in keybinding overrides.
    pub fn id(self) -> String {
        match self {
            Action::SelectTab(n) => format!("select-tab-{n}"),
            Action::SelectSpace(n) => format!("select-space-{n}"),
            _ => FIXED
                .iter()
                .find(|(_, a)| *a == self)
                .map(|(name, _)| (*name).to_owned())
                .expect("parameterless action missing from FIXED"),
        }
    }

    /// Inverse of [`Action::id`]; unknown names and out-of-range numbers are `None`.
    pub fn from_id(id: &str) -> Option<Action> {
        let numbered = |prefix: &str, max: u8| {
            let n: u8 = id.strip_prefix(prefix)?.parse().ok()?;
            (1..=max).contains(&n).then_some(n)
        };
        if let Some(n) = numbered("select-tab-", 8) {
            return Some(Action::SelectTab(n));
        }
        if let Some(n) = numbered("select-space-", 10) {
            return Some(Action::SelectSpace(n));
        }
        // Saved user overrides written before compact mode merged into ToggleSidebar.
        if id == "toggle-compact-mode" {
            return Some(Action::ToggleSidebar);
        }
        FIXED.iter().find(|(name, _)| *name == id).map(|&(_, a)| a)
    }

    /// What the keybinding editor calls the action: its command-bar title, else its id in
    /// sentence case, as the design writes its labels (`select-tab-3` reads "Select tab 3",
    /// `copy-url` "Copy URL").
    pub fn label(self) -> String {
        if let Some(info) = PALETTE.iter().find(|i| i.action == self) {
            return info.title.to_owned();
        }
        let words: Vec<String> = self
            .id()
            .split('-')
            .enumerate()
            .map(|(i, word)| match word {
                // an initialism keeps its capitals
                "url" => "URL".to_owned(),
                _ if i == 0 => {
                    let mut chars = word.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
                _ => word.to_owned(),
            })
            .collect();
        words.join(" ")
    }

    /// Whether the action is an edit command (undo, cut, copy ...): a shell text field
    /// may have the focus without the shell knowing, so these never run from shell focus.
    /// Undo close tab is one: its key is a field's undo first.
    pub fn is_edit(self) -> bool {
        matches!(
            self,
            Action::Undo
                | Action::UndoCloseTab
                | Action::Redo
                | Action::Cut
                | Action::Copy
                | Action::Paste
                | Action::PasteAndMatchStyle
                | Action::SelectAll
        )
    }

    /// Every action, parameterised ones expanded over their range.
    pub fn all() -> Vec<Action> {
        let mut v: Vec<Action> = FIXED.iter().map(|&(_, a)| a).collect();
        v.extend((1..=8).map(Action::SelectTab));
        v.extend((1..=10).map(Action::SelectSpace));
        v
    }
}

/// A command-bar entry: the action plus the text it is matched on.
pub struct ActionInfo {
    pub action: Action,
    pub title: &'static str,
    pub keywords: &'static str,
}

/// Actions the command bar lists (parameterless only). The shortcut column
/// comes from `Keymap::shortcut_for`, so no chord text is stored here.
pub const PALETTE: [ActionInfo; 45] = [
    ActionInfo {
        action: Action::NewSpace,
        title: "New Space",
        keywords: "create workspace",
    },
    ActionInfo {
        action: Action::CloseTab,
        title: "Close tab",
        keywords: "remove",
    },
    ActionInfo {
        action: Action::NewPrivateWindow,
        title: "New private window",
        keywords: "incognito private browsing window",
    },
    ActionInfo {
        action: Action::DuplicateTab,
        title: "Duplicate tab",
        keywords: "clone copy tab",
    },
    ActionInfo {
        action: Action::ReopenClosedTab,
        title: "Reopen closed tab",
        keywords: "undo restore",
    },
    ActionInfo {
        action: Action::SearchTabs,
        title: "Search tabs",
        keywords: "find switch open closed recent tabs list",
    },
    ActionInfo {
        action: Action::UndoCloseTab,
        title: "Undo close tab",
        keywords: "reopen restore closed tab",
    },
    ActionInfo {
        action: Action::Reload,
        title: "Reload",
        keywords: "refresh",
    },
    ActionInfo {
        action: Action::ToggleSidebar,
        title: "Toggle sidebar",
        keywords: "hide show tabs compact",
    },
    ActionInfo {
        action: Action::ToggleMute,
        title: "Mute tab",
        keywords: "sound audio unmute",
    },
    // the design's command-bar rows (browser/command.tsx)
    ActionInfo {
        action: Action::SplitNext,
        title: "Split view with the next tab",
        keywords: "side by side beside pane open",
    },
    ActionInfo {
        action: Action::Unsplit,
        title: "Take this page out of the split",
        keywords: "unsplit side by side pane",
    },
    ActionInfo {
        action: Action::OpenSettings,
        title: "Settings",
        keywords: "preferences options customise keyboard shortcuts",
    },
    ActionInfo {
        action: Action::OpenHistory,
        title: "History",
        keywords: "visited pages browsing",
    },
    ActionInfo {
        action: Action::OpenBookmarks,
        title: "Bookmarks",
        keywords: "favourites saved",
    },
    ActionInfo {
        action: Action::OpenDownloads,
        title: "Downloads",
        keywords: "files saved",
    },
    ActionInfo {
        action: Action::OpenPasswords,
        title: "Passwords",
        keywords: "logins credentials keys",
    },
    ActionInfo {
        action: Action::OpenArchive,
        title: "Archive",
        keywords: "old tabs stale restore",
    },
    ActionInfo {
        action: Action::TidyTabs,
        title: "Tidy tabs by site",
        keywords: "group folders organise sort domain",
    },
    ActionInfo {
        action: Action::UndoTidyTabs,
        title: "Undo tidy tabs",
        keywords: "ungroup folders restore order",
    },
    ActionInfo {
        action: Action::CloseDuplicateTabs,
        title: "Close duplicate tabs",
        keywords: "repeated clean duplicates",
    },
    ActionInfo {
        action: Action::OpenTasks,
        title: "Task manager",
        keywords: "memory cpu processes sleep discard",
    },
    ActionInfo {
        action: Action::OpenAgentLog,
        title: "Agent activity",
        keywords: "mcp sessions calls log agents",
    },
    ActionInfo {
        action: Action::StopAgents,
        title: "Stop all agents",
        keywords: "mcp sessions kill end agents headless",
    },
    ActionInfo {
        action: Action::BookmarkPage,
        title: "Bookmark this page",
        keywords: "star save favourite",
    },
    ActionInfo {
        action: Action::CyclePageTheme,
        title: "Dark mode for this site",
        keywords: "page theme light night invert colours colors appearance",
    },
    ActionInfo {
        action: Action::LinkHints,
        title: "Link hints",
        keywords: "keyboard click follow labels vimium",
    },
    ActionInfo {
        action: Action::ToggleReader,
        title: "Reader mode",
        keywords: "article read clean readability",
    },
    ActionInfo {
        action: Action::ScreenshotRegion,
        title: "Screenshot of a region",
        keywords: "capture image snip select",
    },
    ActionInfo {
        action: Action::ScreenshotVisible,
        title: "Screenshot of the visible page",
        keywords: "capture image window",
    },
    ActionInfo {
        action: Action::ScreenshotFullPage,
        title: "Screenshot of the full page",
        keywords: "capture image whole long scrolling",
    },
    ActionInfo {
        action: Action::NewBoost,
        title: "New Boost for this site",
        keywords: "css style script customise tweak userstyle userscript stylus arc",
    },
    ActionInfo {
        action: Action::ClearBrowsingData,
        title: "Clear browsing data",
        keywords: "cookies cache site storage logins forget privacy delete container history browsing",
    },
    // Chromium's menu items for the chords `keys::DEFAULTS` took from its accelerator table
    ActionInfo {
        action: Action::SavePage,
        title: "Save page as",
        keywords: "download html file keep offline",
    },
    ActionInfo {
        action: Action::OpenFile,
        title: "Open file",
        keywords: "local disk html pdf image browse",
    },
    ActionInfo {
        action: Action::MoveTabUp,
        title: "Move tab up",
        keywords: "reorder order earlier previous",
    },
    ActionInfo {
        action: Action::MoveTabDown,
        title: "Move tab down",
        keywords: "reorder order later next",
    },
    ActionInfo {
        action: Action::DevToolsConsole,
        title: "Console in developer tools",
        keywords: "devtools developer tools log errors inspect",
    },
    ActionInfo {
        action: Action::ToggleCaretBrowsing,
        title: "Caret browsing",
        keywords: "cursor keyboard arrow keys select text accessibility a11y",
    },
    ActionInfo {
        action: Action::NextWindow,
        title: "Next window",
        keywords: "switch cycle windows focus",
    },
    ActionInfo {
        action: Action::PrevWindow,
        title: "Previous window",
        keywords: "switch cycle windows focus back",
    },
    ActionInfo {
        action: Action::PasteAndMatchStyle,
        title: "Paste as plain text",
        keywords: "unformatted match style clipboard",
    },
    ActionInfo {
        action: Action::MinimizeWindow,
        title: "Minimize window",
        keywords: "hide dock taskbar",
    },
    ActionInfo {
        action: Action::PasteAndGo,
        title: "Paste and go",
        keywords: "clipboard open address search",
    },
    ActionInfo {
        action: Action::EmptyCacheHardReload,
        title: "Empty cache and hard reload",
        keywords: "refresh clear cache devtools",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_from_id_are_inverse() {
        let all = Action::all();
        assert_eq!(all.len(), 101);
        for a in all {
            assert_eq!(Action::from_id(&a.id()), Some(a), "{a:?}");
        }
        assert_eq!(
            Action::from_id("toggle-compact-mode"),
            Some(Action::ToggleSidebar)
        );
    }

    #[test]
    fn edit_actions() {
        let edits: Vec<_> = Action::all().into_iter().filter(|a| a.is_edit()).collect();
        assert_eq!(
            edits,
            [
                Action::UndoCloseTab,
                Action::Undo,
                Action::Redo,
                Action::Cut,
                Action::Copy,
                Action::Paste,
                Action::PasteAndMatchStyle,
                Action::SelectAll
            ]
        );
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<String> = Action::all().iter().map(|a| a.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 101);
    }

    #[test]
    fn out_of_range_and_unknown_ids_are_rejected() {
        for id in [
            "select-tab-0",
            "select-tab-9",
            "select-space-0",
            "select-space-11",
            "nope",
            "",
        ] {
            assert_eq!(Action::from_id(id), None, "{id}");
        }
        assert_eq!(
            Action::from_id("select-space-10"),
            Some(Action::SelectSpace(10))
        );
    }

    #[test]
    fn open_settings_and_labels() {
        assert_eq!(Action::from_id("open-settings"), Some(Action::OpenSettings));
        assert_eq!(Action::from_id("duplicate-tab"), Some(Action::DuplicateTab));
        assert_eq!(Action::OpenSettings.label(), "Settings");
        assert_eq!(Action::DuplicateTab.label(), "Duplicate tab");
        assert_eq!(Action::NewTab.label(), "New tab");
        assert_eq!(Action::CloseTab.label(), "Close tab");
        assert_eq!(Action::CloseWindow.label(), "Close window");
        assert_eq!(Action::CopyUrl.label(), "Copy URL");
        assert_eq!(Action::SelectTab(3).label(), "Select tab 3");
        assert_eq!(Action::DevTools.label(), "Dev tools");
        assert_eq!(Action::MoveTabToNewWindow.label(), "Move tab to new window");
        // the command bar's titles are sentence case too (a Space is a name)
        for info in &PALETTE {
            let rest: String = info.title.chars().skip(1).collect();
            let named = ["Space", "Boost"];
            assert!(
                rest.split(' ')
                    .all(|w| named.contains(&w) || w.chars().all(|c| !c.is_uppercase())),
                "{}",
                info.title
            );
        }
        let clear = PALETTE
            .iter()
            .find(|i| i.action == Action::ClearBrowsingData)
            .expect("ClearBrowsingData in PALETTE");
        assert!(clear.keywords.contains("history"));
        assert!(clear.keywords.contains("browsing"));
    }

    #[test]
    fn palette_is_parameterless() {
        for info in &PALETTE {
            assert!(!matches!(
                info.action,
                Action::SelectTab(_) | Action::SelectSpace(_)
            ));
        }
    }

    #[test]
    fn builtin_actions_have_their_ids_and_titles() {
        for (id, action, title) in [
            (
                "cycle-page-theme",
                Action::CyclePageTheme,
                "Dark mode for this site",
            ),
            ("link-hints", Action::LinkHints, "Link hints"),
            ("toggle-reader", Action::ToggleReader, "Reader mode"),
            (
                "screenshot",
                Action::ScreenshotRegion,
                "Screenshot of a region",
            ),
            (
                "screenshot-visible",
                Action::ScreenshotVisible,
                "Screenshot of the visible page",
            ),
            (
                "screenshot-full-page",
                Action::ScreenshotFullPage,
                "Screenshot of the full page",
            ),
            ("new-boost", Action::NewBoost, "New Boost for this site"),
            (
                "clear-browsing-data",
                Action::ClearBrowsingData,
                "Clear browsing data",
            ),
        ] {
            assert_eq!(Action::from_id(id), Some(action));
            assert_eq!(action.id(), id);
            assert_eq!(action.label(), title);
        }
    }
}
