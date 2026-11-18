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
    Quit,
}

impl Action {
    /// Stable kebab-case name, persisted in keybinding overrides.
    pub fn id(self) -> String {
        let s = match self {
            Action::NewTab => "new-tab",
            Action::CloseTab => "close-tab",
            Action::ReopenClosedTab => "reopen-closed-tab",
            Action::NewWindow => "new-window",
            Action::FocusAddress => "focus-address",
            Action::Reload => "reload",
            Action::HardReload => "hard-reload",
            Action::Stop => "stop",
            Action::Back => "back",
            Action::Forward => "forward",
            Action::NextTab => "next-tab",
            Action::PrevTab => "prev-tab",
            Action::SelectTab(n) => return format!("select-tab-{n}"),
            Action::SelectLastTab => "select-last-tab",
            Action::SelectSpace(n) => return format!("select-space-{n}"),
            Action::NewSpace => "new-space",
            Action::NextSpace => "next-space",
            Action::PrevSpace => "prev-space",
            Action::ToggleSidebar => "toggle-sidebar",
            Action::ToggleCompactMode => "toggle-compact-mode",
            Action::Find => "find",
            Action::FindNext => "find-next",
            Action::FindPrev => "find-prev",
            Action::ZoomIn => "zoom-in",
            Action::ZoomOut => "zoom-out",
            Action::ZoomReset => "zoom-reset",
            Action::DevTools => "dev-tools",
            Action::ViewSource => "view-source",
            Action::Fullscreen => "fullscreen",
            Action::TogglePinTab => "toggle-pin-tab",
            Action::CopyUrl => "copy-url",
            Action::Quit => "quit",
        };
        s.to_owned()
    }

    /// Inverse of [`Action::id`]; unknown names and out-of-range numbers are `None`.
    pub fn from_id(id: &str) -> Option<Action> {
        Action::all().into_iter().find(|a| a.id() == id)
    }

    /// Every action, parameterised ones expanded over their range.
    pub fn all() -> Vec<Action> {
        let mut v = vec![
            Action::NewTab,
            Action::CloseTab,
            Action::ReopenClosedTab,
            Action::NewWindow,
            Action::FocusAddress,
            Action::Reload,
            Action::HardReload,
            Action::Stop,
            Action::Back,
            Action::Forward,
            Action::NextTab,
            Action::PrevTab,
            Action::SelectLastTab,
            Action::NewSpace,
            Action::NextSpace,
            Action::PrevSpace,
            Action::ToggleSidebar,
            Action::ToggleCompactMode,
            Action::Find,
            Action::FindNext,
            Action::FindPrev,
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomReset,
            Action::DevTools,
            Action::ViewSource,
            Action::Fullscreen,
            Action::TogglePinTab,
            Action::CopyUrl,
            Action::Quit,
        ];
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
pub const PALETTE: [ActionInfo; 6] = [
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
        action: Action::ToggleCompactMode,
        title: "Toggle Compact Mode",
        keywords: "hide show",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_from_id_are_inverse() {
        let all = Action::all();
        assert_eq!(all.len(), 48);
        for a in all {
            assert_eq!(Action::from_id(&a.id()), Some(a), "{a:?}");
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<String> = Action::all().iter().map(|a| a.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 48);
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
    fn palette_is_parameterless() {
        for info in &PALETTE {
            assert!(!matches!(
                info.action,
                Action::SelectTab(_) | Action::SelectSpace(_)
            ));
        }
    }
}
