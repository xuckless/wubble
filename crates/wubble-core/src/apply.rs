//! What changing a setting does while the app runs, as data: [`effect_of`] maps a key and the
//! stored rows to an [`Applied`] value, which the host executes in one place. Start-up replays
//! the stored rows through the same function, so a setting applies the same way whether it was
//! read at start or changed on the settings page.

use std::collections::HashMap;

use crate::archive::ArchiveAfter;
use crate::pagetheme::{Contrast, MediaPrefs, Motion, PageTheme, ThemeTable};
use crate::reader::ReaderPrefs;
use crate::routing::Rule;
use crate::search::{EngineEntry, Engines};
use crate::seam::SidebarMode;
use crate::settings::{self, SettingDef};
use crate::shot::ShotOutput;
use crate::sleep::SleepPolicy;
use crate::zoom::ZoomTable;

/// Which glass levels the scene may use (`settings::GLASS_QUALITY`). The renderer maps it onto
/// its quality levels; `Off` is its flat glass, live. The Slint-only fallback is a different
/// renderer, chosen before the window exists: `settings::GLASS_SCENE` off (restart) or
/// `WUBBLE_GLASS=slint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlassSetting {
    /// The governor picks, from the best level down, by the frame budget.
    Auto,
    /// Always the best level.
    Full,
    /// The governor picks among the reduced levels only.
    Reduced,
    /// Flat glass: no refraction, a frozen backdrop.
    Off,
}

/// The stored words of [`GlassSetting`], in its order: the row's choices.
pub const GLASS_WORDS: [&str; 4] = ["auto", "full", "reduced", "off"];

impl GlassSetting {
    pub const ALL: [GlassSetting; 4] = [Self::Auto, Self::Full, Self::Reduced, Self::Off];

    /// The stored word, one of the row's choices.
    pub fn id(self) -> &'static str {
        GLASS_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == id)
    }
}

/// `passwords.*` (`settings::PASSWORDS_OFFER_TO_SAVE`, `settings::PASSWORDS_AUTOFILL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordSettings {
    pub offer_to_save: bool,
    pub autofill: bool,
}

impl PasswordSettings {
    /// The two rows' values in `stored` (defaults when absent).
    pub fn from_stored(stored: &HashMap<String, String>) -> Self {
        Self {
            offer_to_save: flag(&settings::PASSWORDS_OFFER_TO_SAVE, stored),
            autofill: flag(&settings::PASSWORDS_AUTOFILL, stored),
        }
    }
}

impl Default for PasswordSettings {
    fn default() -> Self {
        Self::from_stored(&HashMap::new())
    }
}

/// `media.*`: the media downloader's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSettings {
    pub enabled: bool,
    /// Ask YouTube's player API (a request of wubble's own).
    pub youtube: bool,
    pub quality: crate::media::select::Quality,
    /// Empty: the downloads folder.
    pub save_dir: String,
    pub ask_each_time: bool,
    pub concurrency: usize,
    pub min_bytes: u64,
}

impl MediaSettings {
    pub fn from_stored(stored: &HashMap<String, String>) -> Self {
        use settings as s;
        Self {
            enabled: flag(&s::MEDIA_ENABLED, stored),
            youtube: flag(&s::MEDIA_YOUTUBE, stored),
            quality: crate::media::select::Quality::from_word(value_of(&s::MEDIA_QUALITY, stored)),
            save_dir: value_of(&s::MEDIA_SAVE_DIR, stored).trim().to_owned(),
            ask_each_time: flag(&s::MEDIA_ASK_EACH_TIME, stored),
            concurrency: count(&s::MEDIA_CONCURRENCY, stored) as usize,
            min_bytes: u64::from(count(&s::MEDIA_MIN_BYTES, stored)),
        }
    }
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self::from_stored(&HashMap::new())
    }
}

/// A setting's live effect, for the host to execute.
#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    /// The engines and the default (`search.engine`, `search.custom`, `search.discovered`).
    SearchEngines(Engines),
    LiveSuggestions(bool),
    /// Site searches are found on pages (`search.discover_sites`).
    DiscoverSites(bool),
    SidebarMode(SidebarMode),
    BlockerEnabled(bool),
    BlockerExceptions(Vec<String>),
    /// Which filter lists are on (`blocker.lists`, `.cookie_banners`, `.strip_params`,
    /// `.custom_lists`): the blocker is rebuilt in the background.
    BlockerLists {
        lists: std::collections::BTreeMap<String, u32>,
        cookie_banners: bool,
        strip_params: bool,
        custom: Vec<String>,
    },
    /// Expire history by the stored retention (the store reads it).
    ExpireHistory,
    /// The zoom of sites without their own level.
    ZoomDefault(u32),
    SiteZoom(ZoomTable),
    /// The stored keybinding overrides (`keys::overrides_from_entries`).
    Keymap(Vec<String>),
    Sleep(SleepPolicy),
    NeverSleepSites(Vec<String>),
    /// The sites whose pages are muted (`sitesound`).
    MutedSites(Vec<String>),
    ArchiveAfter(ArchiveAfter),
    /// The stored link-routing rules, as typed (`routing::Rule::parse`).
    RoutingRules(Vec<String>),
    Glass(GlassSetting),
    Passwords(PasswordSettings),
    Media(MediaSettings),
    /// Dark mode for pages: the default and the per-site table, and the accessibility media
    /// features, applied to every live page.
    PageThemes {
        default: PageTheme,
        sites: ThemeTable,
        media: MediaPrefs,
    },
    /// Chromium's minimum font size in CSS pixels (0: none), for the whole profile.
    MinFontSize(u32),
    Reader(ReaderPrefs),
    ScreenshotOutput(ShotOutput),
    /// The link hints' letters (`hints::alphabet`).
    HintsAlphabet(&'static str),
    /// The video controls and the per-site speeds.
    Video {
        controls: bool,
        rates: crate::video::RateTable,
        /// Hosts where the controls stay hidden.
        hidden: Vec<String>,
        /// The strip starts folded into its tab.
        collapsed: bool,
    },
    MediaKeys(bool),
    PipOnTabSwitch(bool),
    /// SponsorBlock: on, asked about, and what each category does.
    Sponsor {
        enabled: bool,
        asked: bool,
        acts: crate::sponsor::ActTable,
    },
    BoostsEnabled(bool),
    /// Every download asks where to save it (`settings::DOWNLOADS_ASK_WHERE`).
    AskWhereToSave(bool),
    /// Calmer motion: the setting (the host combines it with the OS preference).
    ReduceMotion(crate::motion::Calm),
    /// Pages for screen readers: the setting (the host combines it with whether a screen reader
    /// runs, [`web_accessibility`]).
    WebAccessibility(crate::motion::Calm),
    /// Touch-sized rows: auto, always or never.
    TouchSizing(crate::touch::Sizing),
    /// Shift+click opens a peek.
    Glance(bool),
    /// The address pill shows whole URLs.
    FullUrls(bool),
    /// Read only when the app starts: the page says a restart is needed.
    RestartOnly,
    /// Whether wubble hosts its MCP server (`agent::hosting`), switched at once.
    McpHosting(bool),
}

/// `def`'s value in `stored`, or its default when absent or invalid (as the store reads it).
pub fn value_of<'a>(def: &'a SettingDef, stored: &'a HashMap<String, String>) -> &'a str {
    stored
        .get(def.key)
        .map(String::as_str)
        .filter(|v| def.kind.accepts(v))
        .unwrap_or(def.default)
}

/// Whether pages' accessibility trees are built and shown to assistive technology: the
/// `a11y.web` setting's on or off, or on auto whether a screen reader runs. The same three-way
/// choice as "Calmer motion" (`motion::Calm`: on, off, or follow the OS).
pub fn web_accessibility(setting: crate::motion::Calm, screen_reader: bool) -> bool {
    setting.reduced(screen_reader)
}

fn flag(def: &SettingDef, stored: &HashMap<String, String>) -> bool {
    value_of(def, stored) == "1"
}

fn count(def: &SettingDef, stored: &HashMap<String, String>) -> u32 {
    let parsed = value_of(def, stored).parse();
    parsed.unwrap_or_else(|_| def.default.parse().expect("a count default is a number"))
}

pub(crate) fn list(def: &SettingDef, stored: &HashMap<String, String>) -> Vec<String> {
    serde_json::from_str(value_of(def, stored)).unwrap_or_default()
}

pub(crate) fn map(def: &SettingDef, stored: &HashMap<String, String>) -> ZoomTable {
    serde_json::from_str(value_of(def, stored)).unwrap_or_default()
}

/// What changing `key` does now, given every stored row (`key -> text`). `None` for a key that
/// is no setting, or one with nothing to apply (the sidebar width: the seam owns it while the
/// app runs).
pub fn effect_of(key: &str, stored: &HashMap<String, String>) -> Option<Applied> {
    use settings as s;
    let def = settings::find(key)?;
    if def.restart {
        return Some(Applied::RestartOnly);
    }
    let v = value_of(def, stored);
    Some(match key {
        k if [
            s::SEARCH_ENGINE.key,
            s::SEARCH_CUSTOM.key,
            s::SEARCH_DISCOVERED.key,
        ]
        .contains(&k) =>
        {
            Applied::SearchEngines(Engines::from_stored(stored))
        }
        k if k == s::LIVE_SUGGESTIONS.key => Applied::LiveSuggestions(flag(def, stored)),
        k if k == s::SEARCH_DISCOVER_SITES.key => Applied::DiscoverSites(flag(def, stored)),
        k if k == s::SIDEBAR_MODE.key => Applied::SidebarMode(SidebarMode::from_id(v)?),
        k if k == s::BLOCKER_ENABLED.key => Applied::BlockerEnabled(flag(def, stored)),
        k if k == s::BLOCKER_EXCEPTIONS.key => Applied::BlockerExceptions(list(def, stored)),
        k if [
            s::BLOCKER_LISTS.key,
            s::BLOCKER_COOKIE_BANNERS.key,
            s::BLOCKER_STRIP_PARAMS.key,
            s::BLOCKER_CUSTOM_LISTS.key,
        ]
        .contains(&k) =>
        {
            Applied::BlockerLists {
                lists: serde_json::from_str(value_of(&s::BLOCKER_LISTS, stored))
                    .unwrap_or_default(),
                cookie_banners: flag(&s::BLOCKER_COOKIE_BANNERS, stored),
                strip_params: flag(&s::BLOCKER_STRIP_PARAMS, stored),
                custom: list(&s::BLOCKER_CUSTOM_LISTS, stored),
            }
        }
        k if k == s::HISTORY_RETENTION_DAYS.key => Applied::ExpireHistory,
        k if k == s::ZOOM_DEFAULT.key => Applied::ZoomDefault(count(def, stored)),
        k if k == s::SITE_ZOOM.key => Applied::SiteZoom(map(def, stored)),
        k if k == s::KEYBINDING_OVERRIDES.key => Applied::Keymap(list(def, stored)),
        k if [
            s::SLEEP_FREEZE_AFTER.key,
            s::SLEEP_DISCARD_AFTER.key,
            s::SLEEP_KEEP_PINNED.key,
        ]
        .contains(&k) =>
        {
            Applied::Sleep(SleepPolicy::from_settings(
                count(&s::SLEEP_FREEZE_AFTER, stored),
                count(&s::SLEEP_DISCARD_AFTER, stored),
                flag(&s::SLEEP_KEEP_PINNED, stored),
            ))
        }
        k if k == s::SLEEP_NEVER_SITES.key => Applied::NeverSleepSites(list(def, stored)),
        k if k == s::SOUND_MUTED_SITES.key => Applied::MutedSites(list(def, stored)),
        k if k == s::ARCHIVE_AFTER.key => Applied::ArchiveAfter(ArchiveAfter::from_id(v)?),
        k if k == s::ROUTING_RULES.key => Applied::RoutingRules(list(def, stored)),
        k if k == s::GLASS_QUALITY.key => Applied::Glass(GlassSetting::from_id(v)?),
        k if [s::PASSWORDS_OFFER_TO_SAVE.key, s::PASSWORDS_AUTOFILL.key].contains(&k) => {
            Applied::Passwords(PasswordSettings::from_stored(stored))
        }
        k if [
            s::PAGE_THEME.key,
            s::PAGE_THEME_SITES.key,
            s::PAGES_MOTION.key,
            s::PAGES_CONTRAST.key,
        ]
        .contains(&k) =>
        {
            Applied::PageThemes {
                default: PageTheme::from_id(value_of(&s::PAGE_THEME, stored))?,
                sites: map(&s::PAGE_THEME_SITES, stored),
                media: MediaPrefs {
                    motion: Motion::from_id(value_of(&s::PAGES_MOTION, stored))?,
                    contrast: Contrast::from_id(value_of(&s::PAGES_CONTRAST, stored))?,
                },
            }
        }
        k if k == s::PAGES_MIN_FONT_SIZE.key => Applied::MinFontSize(count(def, stored)),
        k if [
            s::READER_DETECT.key,
            s::READER_SIZE.key,
            s::READER_WIDTH.key,
            s::READER_FONT.key,
            s::READER_THEME.key,
        ]
        .contains(&k) =>
        {
            Applied::Reader(ReaderPrefs::from_stored(stored))
        }
        k if k == s::SCREENSHOT_OUTPUT.key => Applied::ScreenshotOutput(ShotOutput::from_id(v)?),
        k if k == s::HINTS_ALPHABET.key => Applied::HintsAlphabet(crate::hints::alphabet(v)),
        k if [
            s::VIDEO_CONTROLS.key,
            s::VIDEO_RATES.key,
            s::VIDEO_HIDDEN_SITES.key,
            s::VIDEO_COLLAPSED.key,
        ]
        .contains(&k) =>
        {
            Applied::Video {
                controls: flag(&s::VIDEO_CONTROLS, stored),
                rates: map(&s::VIDEO_RATES, stored),
                hidden: list(&s::VIDEO_HIDDEN_SITES, stored),
                collapsed: flag(&s::VIDEO_COLLAPSED, stored),
            }
        }
        k if k == s::MEDIA_KEYS.key => Applied::MediaKeys(flag(def, stored)),
        k if k == s::PIP_ON_TAB_SWITCH.key => Applied::PipOnTabSwitch(flag(def, stored)),
        k if [
            s::SPONSOR_ENABLED.key,
            s::SPONSOR_ASKED.key,
            s::SPONSOR_ACTIONS.key,
        ]
        .contains(&k) =>
        {
            Applied::Sponsor {
                enabled: flag(&s::SPONSOR_ENABLED, stored),
                asked: flag(&s::SPONSOR_ASKED, stored),
                acts: map(&s::SPONSOR_ACTIONS, stored),
            }
        }
        k if k == s::BOOSTS_ENABLED.key => Applied::BoostsEnabled(flag(def, stored)),
        k if k == s::DOWNLOADS_ASK_WHERE.key => Applied::AskWhereToSave(flag(def, stored)),
        k if k == s::REDUCE_MOTION.key => Applied::ReduceMotion(crate::motion::Calm::parse(v)),
        k if k == s::A11Y_WEB.key => Applied::WebAccessibility(crate::motion::Calm::parse(v)),
        k if k == s::TOUCH_SIZING.key => Applied::TouchSizing(crate::touch::Sizing::parse(v)),
        k if k == s::GLANCE_ENABLED.key => Applied::Glance(flag(def, stored)),
        k if k == s::UI_FULL_URLS.key => Applied::FullUrls(flag(def, stored)),
        k if k == s::AGENT_MCP_ENABLED.key => Applied::McpHosting(flag(def, stored)),
        k if k.starts_with("media.") => Applied::Media(MediaSettings::from_stored(stored)),
        _ => return None,
    })
}

/// Whether `text` may be stored under `key`: a known setting and a valid value of its kind.
pub fn accepts(key: &str, text: &str) -> bool {
    settings::find(key).is_some_and(|d| d.kind.accepts(text))
}

/// The value a row's reset writes.
pub fn default_of(key: &str) -> Option<&'static str> {
    settings::find(key).map(|d| d.default)
}

/// The stored text of a list or map setting after the editor removed `entry` (a list item, or
/// a map key). `None` when `key` is no list or map setting.
pub fn without_entry(key: &str, stored: &HashMap<String, String>, entry: &str) -> Option<String> {
    let def = settings::find(key)?;
    match def.kind {
        settings::SettingKind::List => {
            let mut items = list(def, stored);
            items.retain(|i| i != entry);
            serde_json::to_string(&items).ok()
        }
        settings::SettingKind::Map => {
            let mut table = map(def, stored);
            table.remove(entry);
            serde_json::to_string(&table).ok()
        }
        _ => None,
    }
}

/// The stored text of a list setting after the editor added `entry` (trimmed; a blank or a
/// duplicate leaves it as it is). `None` when `key` is no list setting.
pub fn with_entry(key: &str, stored: &HashMap<String, String>, entry: &str) -> Option<String> {
    let def = settings::find(key).filter(|d| d.kind == settings::SettingKind::List)?;
    put_entry(key, stored, None, entry)
        .ok()
        .or_else(|| serde_json::to_string(&list(def, stored)).ok())
}

/// Why the list editor's draft was turned down: the note its row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotAList,
    Blank,
    /// `routing.rules`: not `site -> Space, Container`.
    NotARule,
    /// A site list: no host in it (`routing::site_host`).
    NotASite,
    /// `blocker.custom_lists`: not an `https://` address (the only kind fetched).
    NotAnAddress,
    Duplicate,
    /// `search.custom`: the engine does not check out (`search::check`).
    Engine(crate::search::Problem),
}

impl Refusal {
    pub fn note(self) -> &'static str {
        match self {
            Refusal::NotAList => "This list cannot be changed here",
            Refusal::Blank => "Type something to add",
            Refusal::NotARule => "Needs a site and a Space, a container or both",
            Refusal::NotASite => "Needs a site, such as example.com",
            Refusal::NotAnAddress => "Needs an address that starts with https://",
            Refusal::Duplicate => "Already in the list",
            Refusal::Engine(problem) => problem.note(),
        }
    }
}

/// The stored text of a list setting with `entry` (trimmed) added at the end, or put in the
/// place of `replacing` (an edit in place; added at the end when that one is gone).
pub fn put_entry(
    key: &str,
    stored: &HashMap<String, String>,
    replacing: Option<&str>,
    entry: &str,
) -> Result<String, Refusal> {
    let def = settings::find(key)
        .filter(|d| d.kind == settings::SettingKind::List)
        .ok_or(Refusal::NotAList)?;
    let mut items = list(def, stored);
    let entry = entry.trim();
    if entry.is_empty() {
        return Err(Refusal::Blank);
    }
    // a malformed rule is refused rather than stored and silently ignored
    if key == settings::ROUTING_RULES.key && Rule::parse(entry).is_none() {
        return Err(Refusal::NotARule);
    }
    // a filter list that is not an HTTPS address would sit in the list and never be fetched
    if key == settings::BLOCKER_CUSTOM_LISTS.key
        && !url::Url::parse(entry).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
    {
        return Err(Refusal::NotAnAddress);
    }
    // an engine is checked as a whole: its keyword against the built-ins' and the other
    // engines of the list (not the one it replaces)
    if [settings::SEARCH_CUSTOM.key, settings::SEARCH_DISCOVERED.key].contains(&key) {
        let engine = EngineEntry::parse(entry).ok_or(Refusal::Engine(crate::search::Problem::Incomplete))?;
        let others: Vec<String> = items
            .iter()
            .filter(|i| Some(i.as_str()) != replacing)
            .cloned()
            .collect();
        let engines = if key == settings::SEARCH_CUSTOM.key {
            Engines::resolve("", &others, &[])
        } else {
            Engines::resolve("", &list(&settings::SEARCH_CUSTOM, stored), &others)
        };
        crate::search::check(&engine, |k| engines.knows_keyword(k)).map_err(Refusal::Engine)?;
        let text = engine.text();
        match replacing.and_then(|r| items.iter().position(|i| i == r)) {
            Some(at) => items[at] = text,
            None => items.push(text),
        }
        return Ok(serde_json::to_string(&items).expect("a list of strings serialises"));
    }
    // a site is kept as its host, the way the matchers read a URL's: a scheme, a port or a path
    // typed with it would never match anything
    let site = settings::SITE_LISTS.contains(&key);
    let entry = if site {
        crate::routing::site_host(entry).ok_or(Refusal::NotASite)?
    } else {
        entry.to_owned()
    };
    let same = |item: &str| match key {
        _ if site => crate::routing::site_host(item).is_some_and(|h| h == entry),
        k if k == settings::ROUTING_RULES.key => crate::routing::same_rule(item, &entry),
        _ => item == entry,
    };
    if items
        .iter()
        .any(|i| same(i) && Some(i.as_str()) != replacing)
    {
        return Err(Refusal::Duplicate);
    }
    match replacing.and_then(|r| items.iter().position(|i| i == r)) {
        Some(at) => items[at] = entry,
        None => items.push(entry),
    }
    Ok(serde_json::to_string(&items).expect("a list of strings serialises"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{ALL, Section};
    use std::time::Duration;

    fn stored(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn pages_reach_screen_readers_on_auto_only_while_one_runs() {
        use crate::motion::Calm;
        for (setting, reader, on) in [
            (Calm::Auto, false, false),
            (Calm::Auto, true, true),
            (Calm::On, false, true),
            (Calm::On, true, true),
            (Calm::Off, false, false),
            (Calm::Off, true, false),
        ] {
            assert_eq!(
                web_accessibility(setting, reader),
                on,
                "{setting:?} {reader}"
            );
        }
    }

    /// A profile seeded before start (the Mac hand checks) turns picture-in-picture on tab switch
    /// on only with the row `pip.on_tab_switch` = `1`; start-up replays it as stored.
    #[test]
    fn picture_in_picture_on_tab_switch_reads_its_row() {
        for (name, rows, on) in [
            ("seeded on", &[("pip.on_tab_switch", "1")][..], true),
            ("seeded off", &[("pip.on_tab_switch", "0")], false),
            ("no row: the default", &[], false),
            (
                "not a Bool's text: the default",
                &[("pip.on_tab_switch", "true")],
                false,
            ),
            ("another key", &[("pip.auto", "1")], false),
        ] {
            assert_eq!(
                effect_of("pip.on_tab_switch", &stored(rows)),
                Some(Applied::PipOnTabSwitch(on)),
                "{name}"
            );
        }
    }

    #[test]
    fn media_settings_read_their_rows() {
        let m = MediaSettings::default();
        assert!(m.enabled && !m.youtube && m.ask_each_time);
        assert_eq!(
            (m.quality, m.concurrency, m.min_bytes),
            (crate::media::select::Quality::UpTo(720), 4, 65_536)
        );
        let changed = stored(&[
            ("media.youtube", "1"),
            ("media.quality", "best"),
            ("media.save_dir", " /v "),
        ]);
        let Some(Applied::Media(m)) = effect_of("media.save_dir", &changed) else {
            panic!()
        };
        assert!(m.youtube);
        assert_eq!(
            (m.quality, m.save_dir.as_str()),
            (crate::media::select::Quality::Best, "/v")
        );
    }

    #[test]
    fn every_live_row_applies_and_restart_rows_wait() {
        let empty = HashMap::new();
        for d in ALL {
            let effect = effect_of(d.key, &empty);
            if d.restart {
                assert_eq!(effect, Some(Applied::RestartOnly), "{}", d.key);
            } else if d.section != Section::Hidden {
                assert!(
                    effect.is_some() && effect != Some(Applied::RestartOnly),
                    "{} has no live effect",
                    d.key
                );
            }
        }
        assert_eq!(effect_of("no.such", &empty), None);
        // The seam owns the width while the app runs.
        assert_eq!(effect_of(settings::SIDEBAR_WIDTH.key, &empty), None);
    }

    #[test]
    fn the_list_settings_apply_live_as_one_selection() {
        let s = stored(&[
            ("blocker.lists", r#"{"easylist":0}"#),
            ("blocker.cookie_banners", "0"),
            ("blocker.custom_lists", r#"["https://l.test/a.txt"]"#),
        ]);
        let want = Some(Applied::BlockerLists {
            lists: [("easylist".to_owned(), 0)].into(),
            cookie_banners: false,
            strip_params: true,
            custom: vec!["https://l.test/a.txt".into()],
        });
        for key in [
            "blocker.lists",
            "blocker.cookie_banners",
            "blocker.strip_params",
            "blocker.custom_lists",
        ] {
            assert_eq!(effect_of(key, &s), want, "{key}");
        }
    }

    #[test]
    fn values_are_decoded_and_bad_text_reads_as_the_default() {
        let s = stored(&[
            ("search.engine", "duckduckgo"),
            ("blocker.enabled", "0"),
            ("zoom.default", "nope"),
            ("ui.sidebar_mode", "rail"),
            ("keys.overrides", r#"["[]"]"#),
        ]);
        assert_eq!(
            effect_of("search.engine", &s),
            Some(Applied::SearchEngines(Engines::resolve("duckduckgo", &[], &[])))
        );
        assert_eq!(
            effect_of("blocker.enabled", &s),
            Some(Applied::BlockerEnabled(false))
        );
        assert_eq!(
            effect_of("zoom.default", &s),
            Some(Applied::ZoomDefault(100))
        );
        assert_eq!(
            effect_of("ui.sidebar_mode", &s),
            Some(Applied::SidebarMode(SidebarMode::Rail))
        );
        assert_eq!(
            effect_of("keys.overrides", &s),
            Some(Applied::Keymap(vec!["[]".into()]))
        );
        assert_eq!(
            effect_of("search.engine", &HashMap::new()),
            Some(Applied::SearchEngines(Engines::resolve("", &[], &[])))
        );
    }

    #[test]
    fn password_rows_make_one_value_on_by_default() {
        assert_eq!(
            PasswordSettings::default(),
            PasswordSettings {
                offer_to_save: true,
                autofill: true,
            }
        );
        let s = stored(&[("passwords.autofill", "0")]);
        assert_eq!(
            effect_of("passwords.offer_to_save", &s),
            Some(Applied::Passwords(PasswordSettings {
                offer_to_save: true,
                autofill: false,
            }))
        );
    }

    #[test]
    fn sleep_rows_make_one_policy_with_discard_after_freeze() {
        let s = stored(&[
            ("sleep.freeze_after_s", "600"),
            ("sleep.discard_after_s", "120"),
            ("sleep.keep_pinned", "0"),
        ]);
        let Some(Applied::Sleep(p)) = effect_of("sleep.keep_pinned", &s) else {
            panic!("no sleep policy")
        };
        assert_eq!(p.freeze_after, Duration::from_secs(600));
        assert_eq!(p.discard_after, Duration::from_secs(600));
        assert!(!p.keep_pinned_alive);
        assert_eq!(
            effect_of("sleep.freeze_after_s", &HashMap::new()),
            Some(Applied::Sleep(SleepPolicy::default()))
        );
    }

    #[test]
    fn every_glass_word_applies() {
        for g in GlassSetting::ALL {
            assert_eq!(
                effect_of("glass.quality", &stored(&[("glass.quality", g.id())])),
                Some(Applied::Glass(g))
            );
        }
    }

    #[test]
    fn list_and_map_entries_are_added_and_removed() {
        let s = stored(&[
            ("blocker.exceptions", r#"["a.test"]"#),
            ("zoom.sites", r#"{"a.test":125,"b.test":90}"#),
        ]);
        let added = with_entry("blocker.exceptions", &s, " b.test ").unwrap();
        assert_eq!(added, r#"["a.test","b.test"]"#);
        assert_eq!(
            with_entry("blocker.exceptions", &s, "a.test").unwrap(),
            r#"["a.test"]"#
        );
        assert_eq!(
            without_entry("blocker.exceptions", &s, "a.test").unwrap(),
            "[]"
        );
        assert_eq!(
            without_entry("zoom.sites", &s, "a.test").unwrap(),
            r#"{"b.test":90}"#
        );
        assert_eq!(with_entry("zoom.sites", &s, "x"), None);
        assert_eq!(without_entry("blocker.enabled", &s, "x"), None);
    }

    #[test]
    fn the_new_rows_apply_and_a_malformed_rule_is_refused() {
        let empty = HashMap::new();
        assert_eq!(
            effect_of("archive.after", &empty),
            Some(Applied::ArchiveAfter(ArchiveAfter::Hours24))
        );
        assert_eq!(
            effect_of("archive.after", &stored(&[("archive.after", "off")])),
            Some(Applied::ArchiveAfter(ArchiveAfter::Off))
        );
        assert_eq!(
            effect_of(
                "sleep.never_sites",
                &stored(&[("sleep.never_sites", r#"["a.test"]"#)])
            ),
            Some(Applied::NeverSleepSites(vec!["a.test".into()]))
        );
        assert_eq!(
            effect_of(
                "sound.muted_sites",
                &stored(&[("sound.muted_sites", r#"["a.test"]"#)])
            ),
            Some(Applied::MutedSites(vec!["a.test".into()]))
        );
        assert_eq!(
            effect_of("routing.rules", &empty),
            Some(Applied::RoutingRules(vec![]))
        );
        let ok = with_entry("routing.rules", &empty, "a.test -> Work").unwrap();
        assert_eq!(ok, r#"["a.test -> Work"]"#);
        assert_eq!(
            with_entry("routing.rules", &empty, "a.test"),
            Some("[]".into())
        );
        assert_eq!(
            with_entry("routing.rules", &empty, "a.test -> "),
            Some("[]".into())
        );
    }

    #[test]
    fn an_edit_takes_the_place_of_its_row_and_a_refusal_says_why() {
        let s = stored(&[("blocker.exceptions", r#"["a.test","b.test"]"#)]);
        let key = "blocker.exceptions";
        assert_eq!(
            put_entry(key, &s, Some("a.test"), " c.test ").unwrap(),
            r#"["c.test","b.test"]"#
        );
        assert_eq!(
            put_entry(key, &s, Some("a.test"), "a.test").unwrap(),
            r#"["a.test","b.test"]"#,
            "an edit that changes nothing is no duplicate"
        );
        assert_eq!(
            put_entry(key, &s, Some("gone.test"), "c.test").unwrap(),
            r#"["a.test","b.test","c.test"]"#
        );
        assert_eq!(
            put_entry(key, &s, Some("a.test"), "b.test"),
            Err(Refusal::Duplicate)
        );
        assert_eq!(put_entry(key, &s, None, "  "), Err(Refusal::Blank));
        assert_eq!(
            put_entry("routing.rules", &s, None, "a.test"),
            Err(Refusal::NotARule)
        );
        assert_eq!(
            put_entry("zoom.sites", &s, None, "a.test"),
            Err(Refusal::NotAList)
        );
        assert!(!Refusal::NotARule.note().is_empty());
    }

    #[test]
    fn a_filter_list_must_be_an_https_address() {
        let key = "blocker.custom_lists";
        let s = stored(&[(key, r#"["https://l.test/a.txt"]"#)]);
        assert_eq!(
            put_entry(key, &s, None, " https://m.test/b.txt ").unwrap(),
            r#"["https://l.test/a.txt","https://m.test/b.txt"]"#
        );
        for bad in [
            "http://m.test/b.txt",
            "m.test/b.txt",
            "file:///x.txt",
            "https://",
            "ftp://a",
        ] {
            assert_eq!(
                put_entry(key, &s, None, bad),
                Err(Refusal::NotAnAddress),
                "{bad}"
            );
        }
        assert_eq!(
            put_entry(key, &s, None, "https://l.test/a.txt"),
            Err(Refusal::Duplicate)
        );
        assert!(!Refusal::NotAnAddress.note().is_empty());
    }

    #[test]
    fn a_site_is_kept_as_its_host_and_a_rule_typed_twice_is_refused() {
        let key = "blocker.exceptions";
        let s = stored(&[(key, r#"["a.test"]"#)]);
        for (typed, kept) in [
            ("https://B.test:8443/x?y", "b.test"),
            ("  *.B.TEST. ", "b.test"),
            ("bücher.de", "xn--bcher-kva.de"),
            ("127.0.0.1:3000", "127.0.0.1"),
        ] {
            assert_eq!(
                put_entry(key, &s, None, typed).unwrap(),
                format!(r#"["a.test","{kept}"]"#),
                "{typed}"
            );
        }
        // the same site, however it is typed, is one entry; a name with a space is none
        assert_eq!(put_entry(key, &s, None, "A.test."), Err(Refusal::Duplicate));
        assert_eq!(
            put_entry(key, &s, None, "http://a.test:99/"),
            Err(Refusal::Duplicate)
        );
        assert_eq!(put_entry(key, &s, None, "a b"), Err(Refusal::NotASite));
        assert_eq!(
            put_entry(key, &s, None, "me@a.test"),
            Err(Refusal::NotASite)
        );
        assert!(!Refusal::NotASite.note().is_empty());
        // an entry stored by an older version, with its port, still counts as the site
        let old = stored(&[(key, r#"["Old.test:8080"]"#)]);
        assert_eq!(
            put_entry(key, &old, None, "old.test"),
            Err(Refusal::Duplicate)
        );
        // rules
        let rules = stored(&[("routing.rules", r#"["a.test -> Work, Bank"]"#)]);
        assert_eq!(
            put_entry("routing.rules", &rules, None, "A.TEST. -> work,bank"),
            Err(Refusal::Duplicate)
        );
        assert!(put_entry("routing.rules", &rules, None, "a.test -> Home").is_ok());
        assert!(put_entry("routing.rules", &rules, None, "bücher.de -> Home").is_ok());
    }

    #[test]
    fn builtin_rows_apply_as_one_value_each() {
        let s = stored(&[
            ("pages.theme", "dark"),
            ("pages.theme_sites", r#"{"a.test":2}"#),
            ("pages.reduced_motion", "reduce"),
            ("pages.min_font_size", "12"),
            ("reader.size", "24"),
            ("screenshot.output", "file"),
            ("hints.alphabet", "vimium"),
        ]);
        let themes = Applied::PageThemes {
            default: PageTheme::Dark,
            sites: ThemeTable::from([("a.test".into(), 2)]),
            media: MediaPrefs {
                motion: Motion::Reduce,
                contrast: Contrast::Auto,
            },
        };
        assert_eq!(effect_of("pages.theme", &s), Some(themes.clone()));
        assert_eq!(effect_of("pages.reduced_motion", &s), Some(themes.clone()));
        assert_eq!(effect_of("pages.contrast", &s), Some(themes.clone()));
        assert_eq!(effect_of("pages.theme_sites", &s), Some(themes));
        assert_eq!(
            effect_of("pages.min_font_size", &s),
            Some(Applied::MinFontSize(12))
        );
        let Some(Applied::Reader(p)) = effect_of("reader.theme", &s) else {
            panic!("no reader prefs")
        };
        assert_eq!(p.size, 24);
        assert_eq!(
            effect_of("screenshot.output", &s),
            Some(Applied::ScreenshotOutput(ShotOutput::File))
        );
        assert_eq!(
            effect_of("hints.alphabet", &s),
            Some(Applied::HintsAlphabet("sadfjklewcmpgh"))
        );
    }

    #[test]
    fn media_rows_apply_as_one_value_each() {
        let s = stored(&[
            ("video.rates", r#"{"a.test":150}"#),
            ("video.hidden_sites", r#"["b.test"]"#),
            ("video.collapsed", "1"),
            ("sponsor.enabled", "1"),
            ("sponsor.actions", r#"{"intro":0}"#),
        ]);
        let video = Applied::Video {
            controls: true,
            rates: crate::video::RateTable::from([("a.test".into(), 150)]),
            hidden: vec!["b.test".into()],
            collapsed: true,
        };
        assert_eq!(effect_of("video.controls", &s), Some(video.clone()));
        assert_eq!(effect_of("video.rates", &s), Some(video.clone()));
        assert_eq!(effect_of("video.hidden_sites", &s), Some(video.clone()));
        assert_eq!(effect_of("video.collapsed", &s), Some(video));
        let sponsor = Applied::Sponsor {
            enabled: true,
            asked: false,
            acts: crate::sponsor::ActTable::from([("intro".into(), 0)]),
        };
        assert_eq!(effect_of("sponsor.asked", &s), Some(sponsor));
        assert_eq!(
            effect_of("pip.on_tab_switch", &s),
            Some(Applied::PipOnTabSwitch(false))
        );
        assert_eq!(effect_of("media.keys", &s), Some(Applied::MediaKeys(true)));
        assert_eq!(
            effect_of("ui.full_urls", &s),
            Some(Applied::FullUrls(false))
        );
        assert_eq!(
            effect_of("ui.full_urls", &stored(&[("ui.full_urls", "1")])),
            Some(Applied::FullUrls(true))
        );
        assert_eq!(
            effect_of("a11y.web", &s),
            Some(Applied::WebAccessibility(crate::motion::Calm::Auto))
        );
        assert_eq!(
            effect_of("boosts.enabled", &s),
            Some(Applied::BoostsEnabled(true))
        );
    }

    #[test]
    fn accepts_and_defaults() {
        assert!(accepts("blocker.enabled", "0"));
        assert!(!accepts("blocker.enabled", "yes"));
        assert!(!accepts("no.such", "1"));
        assert_eq!(default_of("zoom.default"), Some("100"));
    }

    #[test]
    fn an_engine_entry_is_checked_whole() {
        let key = "search.custom";
        let s = stored(&[(key, r#"["Docs | d | https://d.test/?q=%s"]"#)]);
        let refused = |entry: &str| put_entry(key, &s, None, entry).unwrap_err();
        assert_eq!(
            put_entry(key, &s, None, "Shop | s | https://s.test/?q={q}").unwrap(),
            r#"["Docs | d | https://d.test/?q=%s","Shop | s | https://s.test/?q={q}"]"#
        );
        assert_eq!(
            refused("Other | D | https://o.test/?q=%s"),
            Refusal::Engine(crate::search::Problem::KeywordTaken)
        );
        assert_eq!(
            refused("Mine | bing.com | https://o.test/?q=%s"),
            Refusal::Engine(crate::search::Problem::KeywordTaken)
        );
        assert_eq!(
            refused("Mine | m | https://o.test/"),
            Refusal::Engine(crate::search::Problem::NoPlaceholder)
        );
        assert_eq!(
            refused("no fields"),
            Refusal::Engine(crate::search::Problem::Incomplete)
        );
        // an edit keeps its own keyword
        assert_eq!(
            put_entry(
                key,
                &s,
                Some("Docs | d | https://d.test/?q=%s"),
                "Docs | d | https://docs.test/?q=%s"
            )
            .unwrap(),
            r#"["Docs | d | https://docs.test/?q=%s"]"#
        );
        let engines = effect_of(key, &s);
        assert!(
            matches!(&engines, Some(Applied::SearchEngines(e)) if e.by_id("custom:d").is_some()),
            "{engines:?}"
        );
        assert_eq!(
            effect_of("search.discover_sites", &HashMap::new()),
            Some(Applied::DiscoverSites(false))
        );
    }
}
