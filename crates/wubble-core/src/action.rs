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
    ReopenClosedTab,
    NewWindow,
    /// Close the focused window (the last Main window quits).
    CloseWindow,
    /// Move the focused page into a new window.
    MoveTabToNewWindow,
    FocusAddress,
    Reload,
    HardReload,
    Stop,
    Back,
    Forward,
    NextTab,
    PrevTab,
    SelectTab(u8),
    SelectLastTab,
    SelectSpace(u8),
    NewSpace,
    NextSpace,
    PrevSpace,
    ToggleSidebar,
    ToggleCompactMode,
    Find,
    FindNext,
    FindPrev,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    DevTools,
    ViewSource,
    Fullscreen,
    TogglePinTab,
    CopyUrl,
    /// Save the visible page as a PDF in the downloads folder.
    Print,
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
    Quit,
}

/// Name and value of every parameterless action: the one list `id`,
/// `from_id` and `all` share.
const FIXED: [(&str, Action); 54] = [
    ("new-tab", Action::NewTab),
    ("close-tab", Action::CloseTab),
    ("reopen-closed-tab", Action::ReopenClosedTab),
    ("new-window", Action::NewWindow),
    ("close-window", Action::CloseWindow),
    ("move-tab-to-new-window", Action::MoveTabToNewWindow),
    ("focus-address", Action::FocusAddress),
    ("reload", Action::Reload),
    ("hard-reload", Action::HardReload),
    ("stop", Action::Stop),
    ("back", Action::Back),
    ("forward", Action::Forward),
    ("next-tab", Action::NextTab),
    ("prev-tab", Action::PrevTab),
    ("select-last-tab", Action::SelectLastTab),
    ("new-space", Action::NewSpace),
    ("next-space", Action::NextSpace),
    ("prev-space", Action::PrevSpace),
    ("toggle-sidebar", Action::ToggleSidebar),
    ("toggle-compact-mode", Action::ToggleCompactMode),
    ("find", Action::Find),
    ("find-next", Action::FindNext),
    ("find-prev", Action::FindPrev),
    ("zoom-in", Action::ZoomIn),
    ("zoom-out", Action::ZoomOut),
    ("zoom-reset", Action::ZoomReset),
    ("dev-tools", Action::DevTools),
    ("view-source", Action::ViewSource),
    ("fullscreen", Action::Fullscreen),
    ("toggle-pin-tab", Action::TogglePinTab),
    ("copy-url", Action::CopyUrl),
    ("print", Action::Print),
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
        FIXED.iter().find(|(name, _)| *name == id).map(|&(_, a)| a)
    }

    /// What the keybinding editor calls the action: its command-bar title, else its id in
    /// words (`select-tab-3` reads "Select tab 3").
    pub fn label(self) -> String {
        if let Some(info) = PALETTE.iter().find(|i| i.action == self) {
            return info.title.to_owned();
        }
        let words = self.id().replace('-', " ");
        let mut chars = words.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
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
pub const PALETTE: [ActionInfo; 27] = [
    ActionInfo {
        action: Action::NewSpace,
        title: "New Space",
        keywords: "create workspace",
    },
    ActionInfo {
        action: Action::CloseTab,
        title: "Close Tab",
        keywords: "remove",
    },
    ActionInfo {
        action: Action::ReopenClosedTab,
        title: "Reopen Closed Tab",
        keywords: "undo restore",
    },
    ActionInfo {
        action: Action::Reload,
        title: "Reload",
        keywords: "refresh",
    },
    ActionInfo {
        action: Action::ToggleSidebar,
        title: "Toggle Sidebar",
        keywords: "hide show tabs",
    },
    ActionInfo {
        action: Action::ToggleMute,
        title: "Mute Tab",
        keywords: "sound audio unmute",
    },
    // the design's command-bar rows (browser/command.tsx)
    ActionInfo {
        action: Action::SplitNext,
        title: "Split View with the Next Tab",
        keywords: "side by side beside pane open",
    },
    ActionInfo {
        action: Action::Unsplit,
        title: "Take This Page out of the Split",
        keywords: "unsplit side by side pane",
    },
    ActionInfo {
        action: Action::ToggleCompactMode,
        title: "Toggle Compact Mode",
        keywords: "hide show",
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
        title: "Tidy Tabs by Site",
        keywords: "group folders organise sort domain",
    },
    ActionInfo {
        action: Action::UndoTidyTabs,
        title: "Undo Tidy Tabs",
        keywords: "ungroup folders restore order",
    },
    ActionInfo {
        action: Action::CloseDuplicateTabs,
        title: "Close Duplicate Tabs",
        keywords: "repeated clean duplicates",
    },
    ActionInfo {
        action: Action::OpenTasks,
        title: "Task Manager",
        keywords: "memory cpu processes sleep discard",
    },
    ActionInfo {
        action: Action::BookmarkPage,
        title: "Bookmark This Page",
        keywords: "star save favourite",
    },
    ActionInfo {
        action: Action::CyclePageTheme,
        title: "Dark Mode for This Site",
        keywords: "theme light night invert colours",
    },
    ActionInfo {
        action: Action::LinkHints,
        title: "Link Hints",
        keywords: "keyboard click follow labels vimium",
    },
    ActionInfo {
        action: Action::ToggleReader,
        title: "Reader Mode",
        keywords: "article read clean readability",
    },
    ActionInfo {
        action: Action::ScreenshotRegion,
        title: "Screenshot of a Region",
        keywords: "capture image snip select",
    },
    ActionInfo {
        action: Action::ScreenshotVisible,
        title: "Screenshot of the Visible Page",
        keywords: "capture image window",
    },
    ActionInfo {
        action: Action::ScreenshotFullPage,
        title: "Screenshot of the Full Page",
        keywords: "capture image whole long scrolling",
    },
    ActionInfo {
        action: Action::NewBoost,
        title: "New Boost for This Site",
        keywords: "css style script customise tweak userstyle userscript stylus arc",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_from_id_are_inverse() {
        let all = Action::all();
        assert_eq!(all.len(), 72);
        for a in all {
            assert_eq!(Action::from_id(&a.id()), Some(a), "{a:?}");
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<String> = Action::all().iter().map(|a| a.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 72);
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
        assert_eq!(Action::OpenSettings.label(), "Settings");
        assert_eq!(Action::SelectTab(3).label(), "Select tab 3");
        assert_eq!(Action::DevTools.label(), "Dev tools");
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
                "Dark Mode for This Site",
            ),
            ("link-hints", Action::LinkHints, "Link Hints"),
            ("toggle-reader", Action::ToggleReader, "Reader Mode"),
            (
                "screenshot",
                Action::ScreenshotRegion,
                "Screenshot of a Region",
            ),
            (
                "screenshot-visible",
                Action::ScreenshotVisible,
                "Screenshot of the Visible Page",
            ),
            (
                "screenshot-full-page",
                Action::ScreenshotFullPage,
                "Screenshot of the Full Page",
            ),
            ("new-boost", Action::NewBoost, "New Boost for This Site"),
        ] {
            assert_eq!(Action::from_id(id), Some(action));
            assert_eq!(action.id(), id);
            assert_eq!(action.label(), title);
        }
    }
}
