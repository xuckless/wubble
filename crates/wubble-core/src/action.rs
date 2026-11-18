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
    /// Mute or unmute the visible tab (also the sidebar's speaker glyph).
    ToggleMute,
    Quit,
}

/// Name and value of every parameterless action: the one list `id`,
/// `from_id` and `all` share.
const FIXED: [(&str, Action); 31] = [
    ("new-tab", Action::NewTab),
    ("close-tab", Action::CloseTab),
    ("reopen-closed-tab", Action::ReopenClosedTab),
    ("new-window", Action::NewWindow),
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
    ("toggle-mute", Action::ToggleMute),
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
pub const PALETTE: [ActionInfo; 7] = [
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
        assert_eq!(all.len(), 49);
        for a in all {
            assert_eq!(Action::from_id(&a.id()), Some(a), "{a:?}");
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<String> = Action::all().iter().map(|a| a.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 49);
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
