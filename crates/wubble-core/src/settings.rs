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
    /// One of these words.
    Choice(&'static [&'static str]),
    /// A JSON object of string to whole number.
    Map,
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
            SettingKind::Choice(words) => words.contains(&text),
            SettingKind::Map => {
                serde_json::from_str::<std::collections::BTreeMap<String, u32>>(text).is_ok()
            }
        }
    }
}

/// Where the settings page shows a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    General,
    Privacy,
    Appearance,
    /// Tab sleep and glass quality.
    Performance,
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
    label: "Search suggestions from the search engine (sends what you type, never addresses)",
    section: Section::Privacy,
    restart: false,
};
/// The extended sidebar's width, logical px (the seam's `saved`); the default is the design's
/// `seam::SIDEBAR_DEFAULT`.
pub const SIDEBAR_WIDTH: SettingDef = SettingDef {
    key: "ui.sidebar_width",
    kind: SettingKind::Float {
        min: 160.0,
        max: 640.0,
    },
    default: "252",
    label: "Sidebar width",
    section: Section::Hidden,
    restart: false,
};
/// Extended, rail or hidden (`seam::SidebarMode`; migration 4 carries the old boolean
/// `ui.compact` over).
pub const SIDEBAR_MODE: SettingDef = SettingDef {
    key: "ui.sidebar_mode",
    kind: SettingKind::Choice(&["extended", "rail", "hidden"]),
    default: "extended",
    label: "Sidebar",
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
/// Download EasyList, EasyPrivacy and uBlock's lists on their schedule. Off: nothing is fetched
/// and lists come only from `<profile>/blocklists/*.txt`. Read when the blocker starts.
pub const BLOCKER_LISTS_AUTO_UPDATE: SettingDef = SettingDef {
    key: "blocker.lists_auto_update",
    kind: SettingKind::Bool,
    default: "0",
    label: "Download blocklist updates",
    section: Section::Privacy,
    restart: true,
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
/// Per-site zoom (`zoom::ZoomTable`, host to percent): hosts at 100 % are absent.
pub const SITE_ZOOM: SettingDef = SettingDef {
    key: "zoom.sites",
    kind: SettingKind::Map,
    default: "{}",
    label: "Site zoom levels",
    section: Section::Hidden,
    restart: false,
};

/// The zoom a site without its own level shows at, percent.
pub const ZOOM_DEFAULT: SettingDef = SettingDef {
    key: "zoom.default",
    kind: SettingKind::Count { min: 25, max: 500 },
    default: "100",
    label: "Default page zoom (%)",
    section: Section::General,
    restart: false,
};
/// A hidden tab is frozen after this long (`sleep::SleepPolicy::freeze_after`).
pub const SLEEP_FREEZE_AFTER: SettingDef = SettingDef {
    key: "sleep.freeze_after_s",
    kind: SettingKind::Count {
        min: 10,
        max: 86_400,
    },
    default: "60",
    label: "Freeze background tabs after (seconds)",
    section: Section::Performance,
    restart: false,
};
/// A hidden tab is discarded after this long (`sleep::SleepPolicy::discard_after`).
pub const SLEEP_DISCARD_AFTER: SettingDef = SettingDef {
    key: "sleep.discard_after_s",
    kind: SettingKind::Count {
        min: 60,
        max: 604_800,
    },
    default: "900",
    label: "Unload background tabs after (seconds)",
    section: Section::Performance,
    restart: false,
};
/// Pinned tabs are frozen but never unloaded on a timer.
pub const SLEEP_KEEP_PINNED: SettingDef = SettingDef {
    key: "sleep.keep_pinned",
    kind: SettingKind::Bool,
    default: "1",
    label: "Keep pinned tabs loaded",
    section: Section::Performance,
    restart: false,
};
/// Sites whose tabs are never put to sleep (a host covers its subdomains).
pub const SLEEP_NEVER_SITES: SettingDef = SettingDef {
    key: "sleep.never_sites",
    kind: SettingKind::List,
    default: "[]",
    label: "Never put these sites to sleep",
    section: Section::Performance,
    restart: false,
};
/// How long a tab may go unvisited before it moves to the Archive (`archive::ArchiveAfter`).
pub const ARCHIVE_AFTER: SettingDef = SettingDef {
    key: "archive.after",
    kind: SettingKind::Choice(&crate::archive::ARCHIVE_WORDS),
    default: "24h",
    label: "Archive tabs not visited for",
    section: Section::General,
    restart: false,
};
/// Where links open: one rule per entry, `host -> Space` or `host -> Space, Container`
/// (`routing::Rule`). Applies to links from other apps and "Open Link in New Tab"; typing in
/// the command bar, pop-ups and background tabs are not routed.
pub const ROUTING_RULES: SettingDef = SettingDef {
    key: "routing.rules",
    kind: SettingKind::List,
    default: "[]",
    label: "Open sites in a Space or container (site -> Space, Container)",
    section: Section::General,
    restart: false,
};
/// Which glass levels the scene's governor may use (`apply::GlassSetting`).
pub const GLASS_QUALITY: SettingDef = SettingDef {
    key: "glass.quality",
    kind: SettingKind::Choice(&crate::apply::GLASS_WORDS),
    default: "auto",
    label: "Glass quality",
    section: Section::Performance,
    restart: false,
};
/// The wgpu glass scene; off, the panels draw their own Slint glass on the plain ground. The
/// renderer is chosen before the window exists, so it changes at the next start.
pub const GLASS_SCENE: SettingDef = SettingDef {
    key: "glass.scene",
    kind: SettingKind::Bool,
    default: "1",
    label: "GPU glass scene (off: plain panels)",
    section: Section::Performance,
    restart: true,
};
/// Chromium's `OverlayScrollbar` feature, a CEF start switch.
pub const OVERLAY_SCROLLBARS: SettingDef = SettingDef {
    key: "scroll.overlay_scrollbars",
    kind: SettingKind::Bool,
    default: "1",
    label: "Thin scrollbars over the page",
    section: Section::General,
    restart: true,
};
/// Chromium's `MiddleClickAutoscroll` feature, a CEF start switch.
pub const MIDDLE_CLICK_AUTOSCROLL: SettingDef = SettingDef {
    key: "scroll.middle_click_autoscroll",
    kind: SettingKind::Bool,
    default: "1",
    label: "Scroll by middle-click and drag",
    section: Section::General,
    restart: true,
};
/// Offer to save a password after a sign-in (the save prompt).
pub const PASSWORDS_OFFER_TO_SAVE: SettingDef = SettingDef {
    key: "passwords.offer_to_save",
    kind: SettingKind::Bool,
    default: "1",
    label: "Offer to save passwords",
    section: Section::Privacy,
    restart: false,
};
/// The key chip on sign-in forms: saved logins and generated passwords.
pub const PASSWORDS_AUTOFILL: SettingDef = SettingDef {
    key: "passwords.autofill",
    kind: SettingKind::Bool,
    default: "1",
    label: "Offer saved passwords on sign-in forms",
    section: Section::Privacy,
    restart: false,
};

/// Chromium's back/forward cache, which keeps whole left pages alive in the renderer:
/// Chromium's default, a small cache (`BackForwardCache:cache_size/2`) or off
/// (`disable-back-forward-cache`). A CEF start switch (`wubble_engine::app::PageFeatures`).
pub const MEMORY_BACK_FORWARD_CACHE: SettingDef = SettingDef {
    key: "memory.back_forward_cache",
    kind: SettingKind::Choice(&BackForwardCache::WORDS),
    default: "default",
    label: "Back/forward cache",
    section: Section::Performance,
    restart: true,
};
/// `max-decoded-image-size-mb`: caps one decoded image, not the image cache's total (Chromium
/// has no switch for that). 0 keeps Chromium's default.
pub const MEMORY_MAX_DECODED_IMAGE_MB: SettingDef = SettingDef {
    key: "memory.max_decoded_image_mb",
    kind: SettingKind::Count { min: 0, max: 4096 },
    default: "0",
    label: "Largest decoded image (MB, 0 default)",
    section: Section::Performance,
    restart: true,
};
/// Site isolation (one renderer per site). Off (`disable-site-isolation-trials`) saves
/// processes and weakens the defence against Spectre-style attacks.
pub const PROCESS_SITE_ISOLATION: SettingDef = SettingDef {
    key: "process.site_isolation",
    kind: SettingKind::Bool,
    default: "1",
    label: "Site isolation (off saves memory, weakens security)",
    section: Section::Performance,
    restart: true,
};
/// `renderer-process-limit`: past it Chromium reuses renderers. 0 lets Chromium decide.
pub const PROCESS_RENDERER_LIMIT: SettingDef = SettingDef {
    key: "process.renderer_limit",
    kind: SettingKind::Count { min: 0, max: 100 },
    default: "0",
    label: "Renderer process limit (0 automatic)",
    section: Section::Performance,
    restart: true,
};
/// Chromium keeps a spare renderer started for the next navigation
/// (`SpareRendererForSitePerProcess`); off saves one process.
pub const PROCESS_SPARE_RENDERER: SettingDef = SettingDef {
    key: "process.spare_renderer",
    kind: SettingKind::Bool,
    default: "1",
    label: "Keep a spare renderer ready",
    section: Section::Performance,
    restart: true,
};

/// Chromium's remote debugging endpoint (`remote-debugging-port`), on 127.0.0.1 only. Off by
/// default: with a port set, any program on this computer can read and drive every open tab.
/// Docked DevTools is the frontend page served by that endpoint, so it needs the port; without
/// one, DevTools opens in its own window. Ports 1 to 1023 are privileged and count as off.
pub const DEVTOOLS_REMOTE_PORT: SettingDef = SettingDef {
    key: "devtools.remote_port",
    kind: SettingKind::Count { min: 0, max: 65535 },
    default: "0",
    label: "Debugging port, 1024 to 65535 (0 off). Warning: any program on this computer can read and control every open tab. Docks DevTools beside the page",
    section: Section::Privacy,
    restart: true,
};

/// The port `DEVTOOLS_REMOTE_PORT`'s value asks for: `None` for 0 (off) and for the privileged
/// ports 1 to 1023, and for anything past 65535.
pub fn remote_port(raw: u32) -> Option<u16> {
    u16::try_from(raw).ok().filter(|p| *p >= 1024)
}

/// What `MEMORY_BACK_FORWARD_CACHE` asks of Chromium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackForwardCache {
    /// Chromium's own size.
    Default,
    /// At most two left pages per tab.
    Small,
    Off,
}

impl BackForwardCache {
    /// The stored words, in the variants' order.
    pub const WORDS: [&'static str; 3] = ["default", "small", "off"];
    const ALL: [BackForwardCache; 3] = [Self::Default, Self::Small, Self::Off];

    pub fn word(self) -> &'static str {
        Self::WORDS[self as usize]
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.word() == word)
    }
}

/// Every setting, in the order the page shows them within a section.
pub const ALL: &[&SettingDef] = &[
    &SEARCH_ENGINE,
    &LIVE_SUGGESTIONS,
    &SIDEBAR_WIDTH,
    &SIDEBAR_MODE,
    &KEYBINDING_OVERRIDES,
    &BLOCKER_ENABLED,
    &BLOCKER_EXCEPTIONS,
    &BLOCKER_LISTS_AUTO_UPDATE,
    &HISTORY_RETENTION_DAYS,
    &SITE_ZOOM,
    &ZOOM_DEFAULT,
    &OVERLAY_SCROLLBARS,
    &MIDDLE_CLICK_AUTOSCROLL,
    &SLEEP_FREEZE_AFTER,
    &SLEEP_DISCARD_AFTER,
    &SLEEP_KEEP_PINNED,
    &SLEEP_NEVER_SITES,
    &ARCHIVE_AFTER,
    &ROUTING_RULES,
    &GLASS_QUALITY,
    &GLASS_SCENE,
    &MEMORY_BACK_FORWARD_CACHE,
    &MEMORY_MAX_DECODED_IMAGE_MB,
    &PROCESS_SITE_ISOLATION,
    &PROCESS_RENDERER_LIMIT,
    &PROCESS_SPARE_RENDERER,
    &DEVTOOLS_REMOTE_PORT,
    &PASSWORDS_OFFER_TO_SAVE,
    &PASSWORDS_AUTOFILL,
];

/// The definition of `key`, if it is a setting.
pub fn find(key: &str) -> Option<&'static SettingDef> {
    ALL.iter().copied().find(|d| d.key == key)
}

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
    fn the_debugging_port_is_off_by_default_and_ignores_privileged_ports() {
        assert_eq!(remote_port(0), None);
        assert_eq!(remote_port(1), None);
        assert_eq!(remote_port(1023), None);
        assert_eq!(remote_port(1024), Some(1024));
        assert_eq!(remote_port(65535), Some(65535));
        assert_eq!(remote_port(65536), None);
        let d = &DEVTOOLS_REMOTE_PORT;
        assert!(d.restart && d.default == "0");
        assert_eq!(d.kind, SettingKind::Count { min: 0, max: 65535 });
        assert_eq!(remote_port(d.default.parse().unwrap()), None);
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
    fn new_rows_have_their_ranges_and_sections() {
        assert_eq!(ZOOM_DEFAULT.kind, SettingKind::Count { min: 25, max: 500 });
        assert!(!ZOOM_DEFAULT.kind.accepts("20") && ZOOM_DEFAULT.kind.accepts("500"));
        // The sleep defaults are `SleepPolicy::default()`'s.
        let policy = crate::sleep::SleepPolicy::default();
        assert_eq!(
            SLEEP_FREEZE_AFTER.default.parse::<u64>().unwrap(),
            policy.freeze_after.as_secs()
        );
        assert_eq!(
            SLEEP_DISCARD_AFTER.default.parse::<u64>().unwrap(),
            policy.discard_after.as_secs()
        );
        assert_eq!(SLEEP_KEEP_PINNED.default == "1", policy.keep_pinned_alive);
        for d in [&SLEEP_FREEZE_AFTER, &SLEEP_DISCARD_AFTER, &GLASS_QUALITY] {
            assert_eq!(d.section, Section::Performance, "{}", d.key);
        }
        const { assert!(OVERLAY_SCROLLBARS.restart && MIDDLE_CLICK_AUTOSCROLL.restart) };
        // The memory and process rows are start switches whose defaults leave Chromium as is.
        for d in [
            &MEMORY_BACK_FORWARD_CACHE,
            &MEMORY_MAX_DECODED_IMAGE_MB,
            &PROCESS_SITE_ISOLATION,
            &PROCESS_RENDERER_LIMIT,
            &PROCESS_SPARE_RENDERER,
        ] {
            assert!(d.restart && d.section == Section::Performance, "{}", d.key);
        }
        assert_eq!(find("glass.quality"), Some(&GLASS_QUALITY));
        for word in BackForwardCache::WORDS {
            assert_eq!(
                BackForwardCache::from_word(word).map(BackForwardCache::word),
                Some(word)
            );
        }
        assert_eq!(BackForwardCache::from_word("huge"), None);
        assert_eq!(find("nope"), None);
    }

    #[test]
    fn sidebar_width_default_is_the_design_default() {
        assert_eq!(
            SIDEBAR_WIDTH.default.parse::<f64>().unwrap(),
            crate::seam::SIDEBAR_DEFAULT
        );
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
            (SettingKind::Choice(&["a", "b"]), "b", true),
            (SettingKind::Choice(&["a", "b"]), "c", false),
            (SettingKind::Map, "{}", true),
            (SettingKind::Map, "{\"a.test\": 125}", true),
            (SettingKind::Map, "{\"a.test\": -5}", false),
            (SettingKind::Map, "[]", false),
        ];
        for (kind, text, want) in cases {
            assert_eq!(kind.accepts(text), *want, "{kind:?} {text:?}");
        }
    }
}
