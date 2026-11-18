//! What changing a setting does while the app runs, as data: [`effect_of`] maps a key and the
//! stored rows to an [`Applied`] value, which the host executes in one place. Start-up replays
//! the stored rows through the same function, so a setting applies the same way whether it was
//! read at start or changed on the settings page.

use std::collections::HashMap;

use crate::omnibox::SearchEngine;
use crate::pagetheme::{PageTheme, ThemeTable};
use crate::reader::ReaderPrefs;
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

/// A setting's live effect, for the host to execute.
#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    SearchEngine(Option<SearchEngine>),
    LiveSuggestions(bool),
    SidebarMode(SidebarMode),
    BlockerEnabled(bool),
    BlockerExceptions(Vec<String>),
    /// Expire history by the stored retention (the store reads it).
    ExpireHistory,
    /// The zoom of sites without their own level.
    ZoomDefault(u32),
    SiteZoom(ZoomTable),
    /// The stored keybinding overrides (`keys::overrides_from_entries`).
    Keymap(Vec<String>),
    Sleep(SleepPolicy),
    Glass(GlassSetting),
    Passwords(PasswordSettings),
    /// Dark mode for pages: the default and the per-site table, applied to every live page.
    PageThemes {
        default: PageTheme,
        sites: ThemeTable,
    },
    Reader(ReaderPrefs),
    ScreenshotOutput(ShotOutput),
    /// The link hints' letters (`hints::alphabet`).
    HintsAlphabet(&'static str),
    /// The video controls and the per-site speeds.
    Video {
        controls: bool,
        rates: crate::video::RateTable,
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
    /// Read only when the app starts: the page says a restart is needed.
    RestartOnly,
}

/// `def`'s value in `stored`, or its default when absent or invalid (as the store reads it).
pub fn value_of<'a>(def: &'a SettingDef, stored: &'a HashMap<String, String>) -> &'a str {
    stored
        .get(def.key)
        .map(String::as_str)
        .filter(|v| def.kind.accepts(v))
        .unwrap_or(def.default)
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
        k if k == s::SEARCH_ENGINE.key => Applied::SearchEngine(SearchEngine::from_id(v)),
        k if k == s::LIVE_SUGGESTIONS.key => Applied::LiveSuggestions(flag(def, stored)),
        k if k == s::SIDEBAR_MODE.key => Applied::SidebarMode(SidebarMode::from_id(v)?),
        k if k == s::BLOCKER_ENABLED.key => Applied::BlockerEnabled(flag(def, stored)),
        k if k == s::BLOCKER_EXCEPTIONS.key => Applied::BlockerExceptions(list(def, stored)),
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
        k if k == s::GLASS_QUALITY.key => Applied::Glass(GlassSetting::from_id(v)?),
        k if [s::PASSWORDS_OFFER_TO_SAVE.key, s::PASSWORDS_AUTOFILL.key].contains(&k) => {
            Applied::Passwords(PasswordSettings::from_stored(stored))
        }
        k if [s::PAGE_THEME.key, s::PAGE_THEME_SITES.key].contains(&k) => Applied::PageThemes {
            default: PageTheme::from_id(value_of(&s::PAGE_THEME, stored))?,
            sites: map(&s::PAGE_THEME_SITES, stored),
        },
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
        k if [s::VIDEO_CONTROLS.key, s::VIDEO_RATES.key].contains(&k) => Applied::Video {
            controls: flag(&s::VIDEO_CONTROLS, stored),
            rates: map(&s::VIDEO_RATES, stored),
        },
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
    let mut items = list(def, stored);
    let entry = entry.trim();
    if !entry.is_empty() && !items.iter().any(|i| i == entry) {
        items.push(entry.to_owned());
    }
    serde_json::to_string(&items).ok()
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
            Some(Applied::SearchEngine(Some(SearchEngine::DuckDuckGo)))
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
            Some(Applied::SearchEngine(None))
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
    fn builtin_rows_apply_as_one_value_each() {
        let s = stored(&[
            ("pages.theme", "dark"),
            ("pages.theme_sites", r#"{"a.test":2}"#),
            ("reader.size", "24"),
            ("screenshot.output", "file"),
            ("hints.alphabet", "vimium"),
        ]);
        let themes = Applied::PageThemes {
            default: PageTheme::Dark,
            sites: ThemeTable::from([("a.test".into(), 2)]),
        };
        assert_eq!(effect_of("pages.theme", &s), Some(themes.clone()));
        assert_eq!(effect_of("pages.theme_sites", &s), Some(themes));
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
            ("sponsor.enabled", "1"),
            ("sponsor.actions", r#"{"intro":0}"#),
        ]);
        let video = Applied::Video {
            controls: true,
            rates: crate::video::RateTable::from([("a.test".into(), 150)]),
        };
        assert_eq!(effect_of("video.controls", &s), Some(video.clone()));
        assert_eq!(effect_of("video.rates", &s), Some(video));
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
}
