//! The one table of user settings. The typed settings in `wubble-store` (keys and defaults)
//! and the settings page (`pages::settings_page`) both derive from [`ALL`]; there is no second
//! list of keys anywhere.
//!
//! Values are stored as text. [`SettingDef::default`] is that storage text (one encoding, no
//! parallel typed default), so the store decodes it with its codec and the page shows it as is.
//! Design reference, no code copied: Chromium's pref registration (path, type, default per pref).

use crate::omnibox::SearchEngine;

/// What a setting holds, and so which values are valid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingKind {
    /// Stored as `"1"` / `"0"`.
    Bool,
    /// A whole number in `min..=max`.
    Count { min: u32, max: u32 },
    /// A finite number in `min..=max`.
    Float { min: f32, max: f32 },
    /// `""` (ask on first run) or one of `omnibox::ENGINES` by id.
    SearchEngine,
    /// A JSON array of strings; entries are opaque here.
    List,
}

impl SettingKind {
    /// Whether `text` is a valid stored value of this kind.
    pub fn accepts(self, text: &str) -> bool {
        match self {
            SettingKind::Bool => matches!(text, "0" | "1"),
            SettingKind::Count { min, max } => {
                text.parse::<u32>().is_ok_and(|v| (min..=max).contains(&v))
            }
            SettingKind::Float { min, max } => text
                .parse::<f32>()
                .is_ok_and(|v| v.is_finite() && (min..=max).contains(&v)),
            SettingKind::SearchEngine => text.is_empty() || SearchEngine::from_id(text).is_some(),
            SettingKind::List => serde_json::from_str::<Vec<String>>(text).is_ok(),
        }
    }
}

/// Where the settings page shows a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Privacy,
    Appearance,
    /// Stored but not a row on the page (it has its own editor or is dragged).
    Hidden,
}

/// One setting.
#[derive(Debug, PartialEq)]
pub struct SettingDef {
    /// The row's key in the `settings` table. Never rename a shipped key.
    pub key: &'static str,
    pub kind: SettingKind,
    /// Storage text of the default.
    pub default: &'static str,
    pub label: &'static str,
    pub section: Section,
    /// A change only takes effect after a restart.
    pub restart: bool,
}

/// Empty: ask on first run.
pub const SEARCH_ENGINE: SettingDef = SettingDef {
    key: "search.engine",
    kind: SettingKind::SearchEngine,
    default: "",
    label: "Search engine",
    section: Section::General,
    restart: false,
};
/// Live engine suggestions; off until the user turns them on (it sends keystrokes out).
pub const LIVE_SUGGESTIONS: SettingDef = SettingDef {
    key: "suggest.live",
    kind: SettingKind::Bool,
    default: "0",
    label: "Search suggestions from the search engine",
    section: Section::Privacy,
    restart: false,
};
/// Logical pixels; matches `Sizes.sidebar-width` in the UI tokens.
pub const SIDEBAR_WIDTH: SettingDef = SettingDef {
    key: "ui.sidebar_width",
    kind: SettingKind::Float {
        min: 160.0,
        max: 640.0,
    },
    default: "260",
    label: "Sidebar width",
    section: Section::Hidden,
    restart: false,
};
pub const COMPACT_MODE: SettingDef = SettingDef {
    key: "ui.compact",
    kind: SettingKind::Bool,
    default: "0",
    label: "Compact mode",
    section: Section::Appearance,
    restart: false,
};
/// Opaque entries; the keybinding editor owns their format.
pub const KEYBINDING_OVERRIDES: SettingDef = SettingDef {
    key: "keys.overrides",
    kind: SettingKind::List,
    default: "[]",
    label: "Keyboard shortcuts",
    section: Section::Hidden,
    restart: false,
};
pub const BLOCKER_ENABLED: SettingDef = SettingDef {
    key: "blocker.enabled",
    kind: SettingKind::Bool,
    default: "1",
    label: "Block ads and trackers",
    section: Section::Privacy,
    restart: false,
};
/// Hosts the blocker skips.
pub const BLOCKER_EXCEPTIONS: SettingDef = SettingDef {
    key: "blocker.exceptions",
    kind: SettingKind::List,
    default: "[]",
    label: "Sites where blocking is off",
    section: Section::Privacy,
    restart: false,
};
/// Visits older than this many days are expired; 0 keeps everything.
pub const HISTORY_RETENTION_DAYS: SettingDef = SettingDef {
    key: "history.retention_days",
    kind: SettingKind::Count { min: 0, max: 3650 },
    default: "0",
    label: "Keep history for (days, 0 forever)",
    section: Section::Privacy,
    restart: false,
};

/// Every setting, in the order the page shows them within a section.
pub const ALL: &[&SettingDef] = &[
    &SEARCH_ENGINE,
    &LIVE_SUGGESTIONS,
    &SIDEBAR_WIDTH,
    &COMPACT_MODE,
    &KEYBINDING_OVERRIDES,
    &BLOCKER_ENABLED,
    &BLOCKER_EXCEPTIONS,
    &HISTORY_RETENTION_DAYS,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_labels_present() {
        for (i, d) in ALL.iter().enumerate() {
            assert!(!d.key.is_empty() && !d.label.is_empty());
            assert!(ALL[..i].iter().all(|o| o.key != d.key), "dup {}", d.key);
        }
    }

    #[test]
    fn every_default_is_valid_for_its_kind() {
        for d in ALL {
            assert!(
                d.kind.accepts(d.default),
                "{} default {:?}",
                d.key,
                d.default
            );
        }
    }

    #[test]
    fn kinds_accept_and_refuse() {
        let count = SettingKind::Count { min: 1, max: 5 };
        let float = SettingKind::Float { min: 0.0, max: 2.0 };
        let cases: &[(SettingKind, &str, bool)] = &[
            (SettingKind::Bool, "1", true),
            (SettingKind::Bool, "0", true),
            (SettingKind::Bool, "true", false),
            (SettingKind::Bool, "", false),
            (count, "1", true),
            (count, "5", true),
            (count, "0", false),
            (count, "6", false),
            (count, "-1", false),
            (count, "x", false),
            (float, "1.5", true),
            (float, "2.5", false),
            (float, "NaN", false),
            (float, "inf", false),
            (SettingKind::SearchEngine, "", true),
            (SettingKind::SearchEngine, "google", true),
            (SettingKind::SearchEngine, "duckduckgo", true),
            (SettingKind::SearchEngine, "bing", false),
            (SettingKind::List, "[]", true),
            (SettingKind::List, "[\"a\"]", true),
            (SettingKind::List, "[1]", false),
            (SettingKind::List, "{", false),
        ];
        for (kind, text, want) in cases {
            assert_eq!(kind.accepts(text), *want, "{kind:?} {text:?}");
        }
    }
}
