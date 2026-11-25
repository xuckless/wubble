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
    /// Any one line of text (a folder path; empty for the default).
    Text,
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
            SettingKind::Text => !text.contains(['\n', '\r']),
        }
    }
}

/// The list settings whose entries are sites (a host covers its subdomains): the list editor
/// keeps each as `routing::site_host` makes it.
pub const SITE_LISTS: [&str; 4] = [
    "blocker.exceptions",
    "sleep.never_sites",
    "sound.muted_sites",
    "video.hidden_sites",
];

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

impl Section {
    /// The section's heading on the page (none for a hidden setting).
    pub fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Privacy => "Privacy",
            Section::Appearance => "Appearance",
            Section::Performance => "Performance",
            Section::Hidden => "",
        }
    }
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
/// Calmer motion (`motion::reduced`): on, off, or follow the OS preference.
pub const REDUCE_MOTION: SettingDef = SettingDef {
    key: "ui.reduce_motion",
    kind: SettingKind::Choice(&["auto", "on", "off"]),
    default: "auto",
    label: "Calmer motion",
    section: Section::Appearance,
    restart: false,
};
/// Touch sizing (`touch::Sizing`): 44 px rows under a finger, or always, or never.
pub const TOUCH_SIZING: SettingDef = SettingDef {
    key: "ui.touch_sizing",
    kind: SettingKind::Choice(&["auto", "touch", "mouse"]),
    default: "auto",
    label: "Touch-sized rows",
    section: Section::Appearance,
    restart: false,
};
/// The address pill shows the whole URL, scheme and all (Chrome's "Always show full URLs",
/// `omnibox.prevent_url_elisions`); off, it shows the host and the rest (`chrome::address_parts`).
pub const UI_FULL_URLS: SettingDef = SettingDef {
    key: "ui.full_urls",
    kind: SettingKind::Bool,
    default: "0",
    label: "Always show full URLs",
    section: Section::Appearance,
    restart: false,
};
/// Shift+click on a link opens it in a peek over the page (`model::Glance`), not a new tab.
pub const GLANCE_ENABLED: SettingDef = SettingDef {
    key: "glance.enabled",
    kind: SettingKind::Bool,
    default: "1",
    label: "Open links in a peek (Shift+click)",
    section: Section::General,
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
/// Keep the filter lists up to date: download uBlock Origin's default set (and the custom lists)
/// on their schedule. Off: nothing is fetched; the lists bundled with wubble (and any already
/// downloaded into `<profile>/blocklists`) are used. Read when the blocker starts.
pub const BLOCKER_LISTS_AUTO_UPDATE: SettingDef = SettingDef {
    key: "blocker.lists_auto_update",
    kind: SettingKind::Bool,
    default: "0",
    label: "Keep filter lists up to date",
    section: Section::Privacy,
    restart: true,
};
/// The cookie-notice lists (EasyList and uBlock's): banners are hidden, never clicked. Their
/// generic class and id rules apply through the page's collector (`cosmetic.js`). Live: the
/// blocker is rebuilt in the background.
pub const BLOCKER_COOKIE_BANNERS: SettingDef = SettingDef {
    key: "blocker.cookie_banners",
    kind: SettingKind::Bool,
    default: "1",
    label: "Hide cookie banners",
    section: Section::Privacy,
    restart: false,
};
/// The URL tracking list: `$removeparam` rules strip tracking parameters from requests.
pub const BLOCKER_STRIP_PARAMS: SettingDef = SettingDef {
    key: "blocker.strip_params",
    kind: SettingKind::Bool,
    default: "1",
    label: "Remove tracking parameters from links",
    section: Section::Privacy,
    restart: false,
};
/// Filter lists of the user's own, by URL; only HTTPS URLs are fetched, and only while lists
/// are kept up to date or when the user presses "Update now".
pub const BLOCKER_CUSTOM_LISTS: SettingDef = SettingDef {
    key: "blocker.custom_lists",
    kind: SettingKind::List,
    default: "[]",
    label: "Your own filter lists (HTTPS URLs)",
    section: Section::Hidden,
    restart: false,
};
/// Per-list switches by catalog id (`wubble_blocker::catalog`): 0 off, 1 on; a list without an
/// entry keeps its default.
pub const BLOCKER_LISTS: SettingDef = SettingDef {
    key: "blocker.lists",
    kind: SettingKind::Map,
    default: "{}",
    label: "Filter lists switched from their default",
    section: Section::Hidden,
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
/// Sites whose pages play no sound, in every tab and container (a host covers its subdomains):
/// Chrome's "Mute site", its sound content setting (`sitesound`).
pub const SOUND_MUTED_SITES: SettingDef = SettingDef {
    key: "sound.muted_sites",
    kind: SettingKind::List,
    default: "[]",
    label: "Muted sites",
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
/// Pages on their own OS layer, in the platform's mode (`page_layer::PLATFORM_MODES`: macOS
/// `window`, nothing elsewhere); off, every page goes through the shell. Read at start;
/// `--page-layer` overrides it.
pub const PAGE_LAYER: SettingDef = SettingDef {
    key: "page.layer",
    kind: SettingKind::Bool,
    default: "1",
    label: "Show pages on their own layer (macOS)",
    section: Section::Performance,
    restart: true,
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
/// The media downloader: a download glyph on the address pill when a page plays a video or
/// audio stream. Detection reads response headers; a detected HLS or DASH manifest (and one
/// variant playlist of an HLS master) is read again to list its qualities, through the page's
/// HTTP cache and session (a cache miss is a request; see TODO's Decided "Media detection").
pub const MEDIA_ENABLED: SettingDef = SettingDef {
    key: "media.enabled",
    kind: SettingKind::Bool,
    default: "1",
    label: "Offer to save videos and audio",
    section: Section::General,
    restart: false,
};
/// Ask YouTube's player API for a video's formats (a request wubble makes itself, so off by
/// default; `media::youtube`).
pub const MEDIA_YOUTUBE: SettingDef = SettingDef {
    key: "media.youtube",
    kind: SettingKind::Bool,
    default: "0",
    label: "Look up YouTube video formats (asks YouTube)",
    section: Section::General,
    restart: false,
};
/// The quality a download takes when the menu is not asked (`media::select::Quality`).
pub const MEDIA_QUALITY: SettingDef = SettingDef {
    key: "media.quality",
    kind: SettingKind::Choice(&crate::media::select::QUALITY_WORDS),
    default: "720p",
    label: "Video quality",
    section: Section::General,
    restart: false,
};
/// Where saved media goes; empty: the downloads folder.
pub const MEDIA_SAVE_DIR: SettingDef = SettingDef {
    key: "media.save_dir",
    kind: SettingKind::Text,
    default: "",
    label: "Save videos to (empty: the downloads folder)",
    section: Section::General,
    restart: false,
};
/// Every download asks where to save it (the Save ... As rows always ask).
pub const DOWNLOADS_ASK_WHERE: SettingDef = SettingDef {
    key: "downloads.ask_where",
    kind: SettingKind::Bool,
    default: "0",
    label: "Always ask where to save",
    section: Section::General,
    restart: false,
};
/// The glyph opens the menu of qualities; off, it saves the default quality at once.
pub const MEDIA_ASK_EACH_TIME: SettingDef = SettingDef {
    key: "media.ask_each_time",
    kind: SettingKind::Bool,
    default: "1",
    label: "Ask which quality each time",
    section: Section::General,
    restart: false,
};
/// Segments one media download fetches at once.
pub const MEDIA_CONCURRENCY: SettingDef = SettingDef {
    key: "media.concurrency",
    kind: SettingKind::Count { min: 1, max: 8 },
    default: "4",
    label: "Parallel fetches per video download",
    section: Section::Hidden,
    restart: false,
};
/// Files known to be smaller than this many bytes are not offered (sounds, pings).
pub const MEDIA_MIN_BYTES: SettingDef = SettingDef {
    key: "media.min_bytes",
    kind: SettingKind::Count {
        min: 0,
        max: 100_000_000,
    },
    default: "65536",
    label: "Smallest file offered (bytes)",
    section: Section::Hidden,
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
/// Hardware video decode on Linux with an NVIDIA GPU and the system package
/// `libva-nvidia-driver` (`VaapiOnNvidiaGPUs`; no effect elsewhere or without the package).
/// Off by default: the driver starts CUDA threads before Chromium starts the GPU process's
/// seccomp sandbox, which then does not start at all (`chrome://gpu` "Sandboxed: false"). Software
/// decode of 4K60 VP9 costs 130 to 290 % of a core, hardware 25 to 55 % (TODO.md "Decided", video
/// decode on Linux).
pub const VIDEO_HARDWARE_DECODE: SettingDef = SettingDef {
    key: "video.hardware_decode",
    kind: SettingKind::Bool,
    default: "0",
    label: "Hardware video decode on Linux NVIDIA with libva-nvidia-driver (runs the GPU process without its sandbox)",
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

/// The MCP server for agent sessions (`agent::hosting`). Off by default: with it off nothing
/// listens. Switched live from its own Settings group (Agents (MCP)), not a table row. A session
/// is also refused while `devtools.remote_port` is set.
pub const AGENT_MCP_ENABLED: SettingDef = SettingDef {
    key: "agent.mcp_enabled",
    kind: SettingKind::Bool,
    default: "0",
    label: "Host wubble's MCP server for agents on this computer: each session needs your approval",
    section: Section::Hidden,
    restart: false,
};

/// The agent apps the owner took wubble out of (`agent::harness` ids): turning hosting on
/// leaves them alone.
pub const AGENT_MCP_OPTED_OUT: SettingDef = SettingDef {
    key: "agent.mcp_opted_out",
    kind: SettingKind::List,
    default: "[]",
    label: "Agent apps wubble is not added to",
    section: Section::Hidden,
    restart: false,
};

/// The entries wubble owns: a JSON object of `agent::harness` id to the exact entry it wrote
/// (`agent::harness::plan`): turning hosting off removes an entry only while it still is this.
pub const AGENT_MCP_ADDED: SettingDef = SettingDef {
    key: "agent.mcp_added",
    kind: SettingKind::Text,
    default: "{}",
    label: "Agent apps wubble added itself to",
    section: Section::Hidden,
    restart: false,
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
/// Dark mode for pages (`pagetheme::PageTheme`): sites without their own choice.
pub const PAGE_THEME: SettingDef = SettingDef {
    key: "pages.theme",
    kind: SettingKind::Choice(&crate::pagetheme::THEME_WORDS),
    default: "auto",
    label: "Dark mode for pages",
    section: Section::Appearance,
    restart: false,
};
/// Per-site dark mode (`pagetheme::ThemeTable`, host to 1 dark / 2 light): sites at the default
/// are absent. Set from the address pill's menu.
pub const PAGE_THEME_SITES: SettingDef = SettingDef {
    key: "pages.theme_sites",
    kind: SettingKind::Map,
    default: "{}",
    label: "Site page themes",
    section: Section::Hidden,
    restart: false,
};
/// `prefers-reduced-motion` for every page (`pagetheme::Motion`); `auto` follows the OS.
pub const PAGES_MOTION: SettingDef = SettingDef {
    key: "pages.reduced_motion",
    kind: SettingKind::Choice(&crate::pagetheme::MOTION_WORDS),
    default: "auto",
    label: "Reduce motion on pages",
    section: Section::Appearance,
    restart: false,
};
/// More contrast or forced colours for every page (`pagetheme::Contrast`); `auto` follows the OS.
pub const PAGES_CONTRAST: SettingDef = SettingDef {
    key: "pages.contrast",
    kind: SettingKind::Choice(&crate::pagetheme::CONTRAST_WORDS),
    default: "auto",
    label: "Contrast on pages",
    section: Section::Appearance,
    restart: false,
};
/// Chromium's minimum font size, in CSS pixels (its `webkit.webprefs.minimum_font_size`
/// preference, 0 to 24 in Chrome's settings); 0 is none.
pub const PAGES_MIN_FONT_SIZE: SettingDef = SettingDef {
    key: "pages.min_font_size",
    kind: SettingKind::Count { min: 0, max: 24 },
    default: "0",
    label: "Minimum font size",
    section: Section::Appearance,
    restart: false,
};
/// Probe pages for an article: the address pill offers reader mode.
pub const READER_DETECT: SettingDef = SettingDef {
    key: "reader.detect",
    kind: SettingKind::Bool,
    default: "1",
    label: "Offer reader mode on articles",
    section: Section::Appearance,
    restart: false,
};
/// The reader's body text, CSS px (`reader::ReaderPrefs`).
pub const READER_SIZE: SettingDef = SettingDef {
    key: "reader.size",
    kind: SettingKind::Count { min: 14, max: 28 },
    default: "18",
    label: "Reader text size",
    section: Section::Appearance,
    restart: false,
};
pub const READER_WIDTH: SettingDef = SettingDef {
    key: "reader.width",
    kind: SettingKind::Choice(&crate::reader::WIDTH_WORDS),
    default: "medium",
    label: "Reader column",
    section: Section::Appearance,
    restart: false,
};
pub const READER_FONT: SettingDef = SettingDef {
    key: "reader.font",
    kind: SettingKind::Choice(&crate::reader::FONT_WORDS),
    default: "serif",
    label: "Reader font",
    section: Section::Appearance,
    restart: false,
};
pub const READER_THEME: SettingDef = SettingDef {
    key: "reader.theme",
    kind: SettingKind::Choice(&crate::reader::THEME_WORDS),
    default: "dark",
    label: "Reader theme",
    section: Section::Appearance,
    restart: false,
};
/// Where screenshots go (`shot::ShotOutput`).
pub const SCREENSHOT_OUTPUT: SettingDef = SettingDef {
    key: "screenshot.output",
    kind: SettingKind::Choice(&crate::shot::OUTPUT_WORDS),
    default: "both",
    label: "Screenshots go to (Downloads, clipboard)",
    section: Section::General,
    restart: false,
};
/// The letters link hints are made of (`hints::ALPHABETS`).
pub const HINTS_ALPHABET: SettingDef = SettingDef {
    key: "hints.alphabet",
    kind: SettingKind::Choice(&crate::hints::ALPHABET_WORDS),
    default: "home-row",
    label: "Link hint letters",
    section: Section::General,
    restart: false,
};

/// Video controls over every page's main video (`video`).
pub const VIDEO_CONTROLS: SettingDef = SettingDef {
    key: "video.controls",
    kind: SettingKind::Bool,
    default: "1",
    label: "Video controls on pages (speed, loop, frame)",
    section: Section::General,
    restart: false,
};
/// Per-site video speed (`video::RateTable`, host to rate in hundredths): sites at normal speed
/// are absent. Set from the video controls.
pub const VIDEO_RATES: SettingDef = SettingDef {
    key: "video.rates",
    kind: SettingKind::Map,
    default: "{}",
    label: "Site video speeds",
    section: Section::Hidden,
    restart: false,
};
/// Sites whose pages never show the video controls (the strip's "Hide on this site"); removing
/// the row shows them again.
pub const VIDEO_HIDDEN_SITES: SettingDef = SettingDef {
    key: "video.hidden_sites",
    kind: SettingKind::List,
    default: "[]",
    label: "Sites where video controls are hidden",
    section: Section::General,
    restart: false,
};
/// The strip is tucked into its tab at the page's edge (remembered from the strip's collapse
/// button).
pub const VIDEO_COLLAPSED: SettingDef = SettingDef {
    key: "video.collapsed",
    kind: SettingKind::Bool,
    default: "0",
    label: "Video controls start folded into their tab",
    section: Section::Hidden,
    restart: false,
};
/// Media keys and the system's media controls (MPRIS on Linux) for the playing tab.
pub const MEDIA_KEYS: SettingDef = SettingDef {
    key: "media.keys",
    kind: SettingKind::Bool,
    default: "1",
    label: "Media keys control the playing tab",
    section: Section::General,
    restart: false,
};
/// Leaving a tab whose video plays aloud pops it into picture-in-picture (`pip`, a Pip window);
/// coming back to the tab closes it.
pub const PIP_ON_TAB_SWITCH: SettingDef = SettingDef {
    key: "pip.on_tab_switch",
    kind: SettingKind::Bool,
    default: "0",
    label: "Picture-in-picture when leaving a tab with a playing video",
    section: Section::General,
    restart: false,
};
/// Web pages in the screen reader's tree (`apply::web_accessibility`): auto while a screen
/// reader runs, or always, or never. CEF builds every page's accessibility tree while it is on,
/// which costs memory and CPU, so auto is off without one.
pub const A11Y_WEB: SettingDef = SettingDef {
    key: "a11y.web",
    kind: SettingKind::Choice(&["auto", "on", "off"]),
    default: "auto",
    label: "Pages for screen readers (auto: while one runs)",
    section: Section::General,
    restart: false,
};
/// SponsorBlock lookups (`sponsor`): a request wubble itself makes, so off until the user
/// turns it on (the first YouTube video asks once). The label carries the data's attribution.
pub const SPONSOR_ENABLED: SettingDef = SettingDef {
    key: "sponsor.enabled",
    kind: SettingKind::Bool,
    default: "0",
    label: "Skip sponsor segments on YouTube (uses SponsorBlock data from https://sponsor.ajay.app/)",
    section: Section::Privacy,
    restart: false,
};
/// The one-time "Skip sponsor segments?" question was answered.
pub const SPONSOR_ASKED: SettingDef = SettingDef {
    key: "sponsor.asked",
    kind: SettingKind::Bool,
    default: "0",
    label: "Asked about sponsor segments",
    section: Section::Hidden,
    restart: false,
};
/// Per-category choices (`sponsor::ActTable`, category to skip 0 / show 1 / off 2); absent
/// categories use their default.
pub const SPONSOR_ACTIONS: SettingDef = SettingDef {
    key: "sponsor.actions",
    kind: SettingKind::Map,
    default: "{}",
    label: "Sponsor segment categories",
    section: Section::Hidden,
    restart: false,
};
/// Boosts (`boost`): the kill switch for every site's own CSS and scripts.
pub const BOOSTS_ENABLED: SettingDef = SettingDef {
    key: "boosts.enabled",
    kind: SettingKind::Bool,
    default: "1",
    label: "Boosts: your own CSS and scripts for sites",
    section: Section::Appearance,
    restart: false,
};

/// The updater's check when wubble opens (`update`): on by default, the one request wubble makes
/// on its own unless asked (docs/decisions.md "Auto-update"). Its row is in the page's Updates
/// group, beside the status, not in a table section. Read when wubble opens.
pub const UPDATE_CHECK_ON_OPEN: SettingDef = SettingDef {
    key: "update.check_on_open",
    kind: SettingKind::Bool,
    default: "1",
    label: "Check for updates when wubble opens",
    section: Section::Hidden,
    restart: false,
};
/// When the updater last asked for the manifest, Unix seconds (0: never; `update::Policy`).
pub const UPDATE_LAST_CHECK: SettingDef = SettingDef {
    key: "update.last_check",
    kind: SettingKind::Count {
        min: 0,
        max: u32::MAX,
    },
    default: "0",
    label: "Last update check",
    section: Section::Hidden,
    restart: false,
};

/// The installed extensions (`extensions::Installed`, a JSON array): its own page edits it, and
/// it is read as CEF starts (`--load-extension`), so a change applies at the next start.
pub const EXTENSIONS_INSTALLED: SettingDef = SettingDef {
    key: "extensions.installed",
    kind: SettingKind::Text,
    default: "[]",
    label: "Installed extensions",
    section: Section::Hidden,
    restart: true,
};

/// Every setting, in the order the page shows them within a section.
pub const ALL: &[&SettingDef] = &[
    &SEARCH_ENGINE,
    &LIVE_SUGGESTIONS,
    &SIDEBAR_WIDTH,
    &SIDEBAR_MODE,
    &REDUCE_MOTION,
    &TOUCH_SIZING,
    &GLANCE_ENABLED,
    &UI_FULL_URLS,
    &KEYBINDING_OVERRIDES,
    &BLOCKER_ENABLED,
    &BLOCKER_EXCEPTIONS,
    &BLOCKER_LISTS_AUTO_UPDATE,
    &BLOCKER_COOKIE_BANNERS,
    &BLOCKER_STRIP_PARAMS,
    &BLOCKER_CUSTOM_LISTS,
    &BLOCKER_LISTS,
    &HISTORY_RETENTION_DAYS,
    &SOUND_MUTED_SITES,
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
    &PAGE_LAYER,
    &MEMORY_BACK_FORWARD_CACHE,
    &MEMORY_MAX_DECODED_IMAGE_MB,
    &PROCESS_SITE_ISOLATION,
    &PROCESS_RENDERER_LIMIT,
    &PROCESS_SPARE_RENDERER,
    &VIDEO_HARDWARE_DECODE,
    &DEVTOOLS_REMOTE_PORT,
    &AGENT_MCP_ENABLED,
    &AGENT_MCP_OPTED_OUT,
    &AGENT_MCP_ADDED,
    &PASSWORDS_OFFER_TO_SAVE,
    &PASSWORDS_AUTOFILL,
    &MEDIA_ENABLED,
    &MEDIA_YOUTUBE,
    &MEDIA_QUALITY,
    &MEDIA_SAVE_DIR,
    &MEDIA_ASK_EACH_TIME,
    &MEDIA_CONCURRENCY,
    &MEDIA_MIN_BYTES,
    &DOWNLOADS_ASK_WHERE,
    &PAGE_THEME,
    &PAGE_THEME_SITES,
    &PAGES_MOTION,
    &PAGES_CONTRAST,
    &PAGES_MIN_FONT_SIZE,
    &READER_DETECT,
    &READER_SIZE,
    &READER_WIDTH,
    &READER_FONT,
    &READER_THEME,
    &SCREENSHOT_OUTPUT,
    &HINTS_ALPHABET,
    &VIDEO_CONTROLS,
    &VIDEO_RATES,
    &VIDEO_HIDDEN_SITES,
    &VIDEO_COLLAPSED,
    &MEDIA_KEYS,
    &PIP_ON_TAB_SWITCH,
    &A11Y_WEB,
    &SPONSOR_ENABLED,
    &SPONSOR_ASKED,
    &SPONSOR_ACTIONS,
    &BOOSTS_ENABLED,
    &UPDATE_CHECK_ON_OPEN,
    &UPDATE_LAST_CHECK,
    &EXTENSIONS_INSTALLED,
];

/// The definition of `key`, if it is a setting.
pub fn find(key: &str) -> Option<&'static SettingDef> {
    ALL.iter().copied().find(|d| d.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_site_lists_are_list_settings() {
        for key in SITE_LISTS {
            assert_eq!(find(key).map(|d| d.kind), Some(SettingKind::List), "{key}");
        }
        assert!(SITE_LISTS.contains(&BLOCKER_EXCEPTIONS.key));
    }

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
            &VIDEO_HARDWARE_DECODE,
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
    fn media_rows_default_off_where_they_reach_out() {
        // The YouTube lookup is a request wubble makes itself: off by default.
        assert_eq!(MEDIA_YOUTUBE.default, "0");
        assert_eq!(MEDIA_ENABLED.default, "1");
        assert!(MEDIA_QUALITY.kind.accepts("1080p") && !MEDIA_QUALITY.kind.accepts("4k"));
        assert_eq!(
            find("media.save_dir").map(|d| d.kind),
            Some(SettingKind::Text)
        );
        assert!(MEDIA_CONCURRENCY.kind.accepts("8") && !MEDIA_CONCURRENCY.kind.accepts("0"));
    }

    #[test]
    fn media_rows_start_as_decided() {
        // A request wubble makes itself starts off (TODO.md "Decided": nothing phones home).
        assert_eq!(SPONSOR_ENABLED.default, "0");
        assert!(
            SPONSOR_ENABLED
                .label
                .contains(crate::sponsor::DEFAULT_ENDPOINT)
        );
        assert_eq!(PIP_ON_TAB_SWITCH.default, "0");
        assert_eq!(VIDEO_CONTROLS.default, "1");
        assert_eq!(BOOSTS_ENABLED.default, "1");
        for d in [
            &VIDEO_RATES,
            &VIDEO_COLLAPSED,
            &SPONSOR_ASKED,
            &SPONSOR_ACTIONS,
            &BLOCKER_CUSTOM_LISTS,
            &BLOCKER_LISTS,
        ] {
            assert_eq!(d.section, Section::Hidden, "{}", d.key);
        }
        // A row of its own now that the Pip window exists.
        assert_eq!(PIP_ON_TAB_SWITCH.section, Section::General);
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
            (SettingKind::Text, "", true),
            (SettingKind::Text, "/home/x/Videos", true),
            (SettingKind::Text, "a\nb", false),
        ];
        for (kind, text, want) in cases {
            assert_eq!(kind.accepts(text), *want, "{kind:?} {text:?}");
        }
    }
}
