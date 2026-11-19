//! Dark mode for pages: Chromium's own forced dark (Blink's auto dark mode)
//! and the `prefers-color-scheme` the page sees, per site, switched over the
//! DevTools protocol per tab so it applies live. The same media call carries the
//! accessibility preferences pages see ([`MediaPrefs`]: `prefers-reduced-motion`,
//! `prefers-contrast`, `forced-colors`), since `Emulation.setEmulatedMedia` replaces the whole
//! feature list each time.
//!
//! The per-site table follows `zoom.rs`: host to a theme, hosts at the default absent
//! (`settings::PAGE_THEME_SITES`), the default in `settings::PAGE_THEME`. [`calls`] is the one
//! place that decides what each theme asks of Chromium; [`sync`] is the one explicit state: what
//! the view was last told.
//!
//! Called, not copied: `Emulation.setAutoDarkModeOverride` flips Blink's `ForceDarkModeEnabled`
//! setting (Chromium `third_party/blink/renderer/core/inspector/dev_tools_emulator.cc`,
//! BSD-3-Clause); `Emulation.setEmulatedMedia` sets the media feature. Both outlive navigations
//! while the DevTools session stays attached, which CEF keeps for the browser's life.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::zoom::host_of;

/// What a page is shown as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageTheme {
    /// The page as it is: no override.
    Auto,
    /// Chromium's forced dark, and `prefers-color-scheme: dark`.
    Dark,
    /// Never forced dark, and `prefers-color-scheme: light`.
    Light,
}

/// The stored words, in [`PageTheme::ALL`]'s order: the setting's choices.
pub const THEME_WORDS: [&str; 3] = ["auto", "dark", "light"];

impl PageTheme {
    pub const ALL: [PageTheme; 3] = [Self::Auto, Self::Dark, Self::Light];

    pub fn id(self) -> &'static str {
        THEME_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// The table's number for the theme (0 is never stored: it is the default's absence).
    fn code(self) -> u32 {
        self as u32
    }

    fn from_code(code: u32) -> Option<Self> {
        Self::ALL.get(usize::try_from(code).ok()?).copied()
    }

    /// The next theme in the cycle Auto, Dark, Light (`Action::CyclePageTheme`).
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    /// What the menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "As the site shows it",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

/// `prefers-reduced-motion` for pages. `Auto` leaves it to Chromium, which reads the OS
/// (GTK's `gtk-enable-animations` on Linux, the macOS and Windows accessibility settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Motion {
    #[default]
    Auto,
    Reduce,
    NoPreference,
}

/// [`Motion`]'s stored words, in the setting's order (`allow` is `no-preference`).
const MOTIONS: [(&str, Motion); 3] = [
    ("auto", Motion::Auto),
    ("reduce", Motion::Reduce),
    ("allow", Motion::NoPreference),
];
/// The setting's choices.
pub const MOTION_WORDS: [&str; 3] = words(&MOTIONS);

impl Motion {
    pub fn id(self) -> &'static str {
        word_of(&MOTIONS, self)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        of_word(&MOTIONS, id)
    }
}

/// Contrast for pages. `More` is `prefers-contrast: more`; `Forced` is `forced-colors: active`
/// (Blink's forced colours, as DevTools' rendering emulation turns them on) with
/// `prefers-contrast: more`. `Auto` leaves both to Chromium and the OS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Contrast {
    #[default]
    Auto,
    More,
    Forced,
}

/// [`Contrast`]'s stored words, in the setting's order.
const CONTRASTS: [(&str, Contrast); 3] = [
    ("auto", Contrast::Auto),
    ("more", Contrast::More),
    ("forced", Contrast::Forced),
];
/// The setting's choices.
pub const CONTRAST_WORDS: [&str; 3] = words(&CONTRASTS);

impl Contrast {
    pub fn id(self) -> &'static str {
        word_of(&CONTRASTS, self)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        of_word(&CONTRASTS, id)
    }
}

/// The words of a (word, value) table, in its order.
const fn words<T: Copy, const N: usize>(table: &[(&'static str, T); N]) -> [&'static str; N] {
    let mut out = [""; N];
    let mut i = 0;
    while i < N {
        out[i] = table[i].0;
        i += 1;
    }
    out
}

fn word_of<T: Copy + PartialEq>(table: &[(&'static str, T)], value: T) -> &'static str {
    table
        .iter()
        .find(|(_, v)| *v == value)
        .map(|(w, _)| *w)
        .expect("every value has a word")
}

fn of_word<T: Copy>(table: &[(&'static str, T)], word: &str) -> Option<T> {
    table.iter().find(|(w, _)| *w == word).map(|(_, v)| *v)
}

/// The accessibility media features every page is shown with (settings, not per site).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MediaPrefs {
    pub motion: Motion,
    pub contrast: Contrast,
}

impl MediaPrefs {
    /// The emulated media features these ask for, in the protocol's `{name, value}` form.
    fn features(self) -> Vec<Value> {
        let feature = |name: &str, value: &str| json!({ "name": name, "value": value });
        let mut v = vec![];
        match self.motion {
            Motion::Auto => {}
            Motion::Reduce => v.push(feature("prefers-reduced-motion", "reduce")),
            Motion::NoPreference => v.push(feature("prefers-reduced-motion", "no-preference")),
        }
        match self.contrast {
            Contrast::Auto => {}
            Contrast::More => v.push(feature("prefers-contrast", "more")),
            Contrast::Forced => {
                v.push(feature("prefers-contrast", "more"));
                v.push(feature("forced-colors", "active"));
            }
        }
        v
    }
}

/// What a view was last told: its theme and the media preferences.
pub type Shown = (PageTheme, MediaPrefs);

/// Host to theme code (1 dark, 2 light). Hosts at the default are absent.
pub type ThemeTable = BTreeMap<String, u32>;

/// The table after the user chose `theme` for `host`: the default removes the row.
pub fn next_table(
    mut table: ThemeTable,
    host: &str,
    theme: PageTheme,
    default: PageTheme,
) -> ThemeTable {
    if theme == default {
        table.remove(host);
    } else {
        table.insert(host.to_string(), theme.code());
    }
    table
}

/// The theme a page at `page` should show: its site's, else `default`. Pages without a host
/// (`data:`, `file:`) follow the default.
pub fn wanted(table: &ThemeTable, page: &str, default: PageTheme) -> PageTheme {
    host_of(page)
        .and_then(|host| table.get(&host).copied())
        .and_then(PageTheme::from_code)
        .unwrap_or(default)
}

/// The DevTools calls that show a page as `theme` with `media`, in order.
pub fn calls((theme, media): Shown) -> Vec<(&'static str, Value)> {
    let scheme = |value: &str| json!({ "name": "prefers-color-scheme", "value": value });
    let (mut features, dark) = match theme {
        PageTheme::Auto => (vec![], json!({})),
        PageTheme::Dark => (vec![scheme("dark")], json!({ "enabled": true })),
        PageTheme::Light => (vec![scheme("light")], json!({ "enabled": false })),
    };
    features.extend(media.features());
    vec![
        (
            "Emulation.setEmulatedMedia",
            json!({ "features": features }),
        ),
        ("Emulation.setAutoDarkModeOverride", dark),
    ]
}

/// Whether the view must be told: `Some(wanted)` unless it was last told exactly that.
/// `applied` is `None` for a view that was told nothing yet (a new view, a renderer that went
/// away), so the first page always gets its calls.
pub fn sync(applied: Option<Shown>, wanted: Shown) -> Option<Shown> {
    (applied != Some(wanted)).then_some(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_round_trip_and_cycle() {
        for t in PageTheme::ALL {
            assert_eq!(PageTheme::from_id(t.id()), Some(t));
            assert_eq!(PageTheme::from_code(t.code()), Some(t));
        }
        assert_eq!(PageTheme::from_id("sepia"), None);
        assert_eq!(PageTheme::from_code(3), None);
        assert_eq!(PageTheme::Auto.next(), PageTheme::Dark);
        assert_eq!(PageTheme::Dark.next(), PageTheme::Light);
        assert_eq!(PageTheme::Light.next(), PageTheme::Auto);
    }

    #[test]
    fn media_words_round_trip() {
        for (word, m) in MOTIONS {
            assert_eq!((m.id(), Motion::from_id(word)), (word, Some(m)));
        }
        for (word, c) in CONTRASTS {
            assert_eq!((c.id(), Contrast::from_id(word)), (word, Some(c)));
        }
        assert_eq!(Motion::NoPreference.id(), "allow");
        assert_eq!(Contrast::from_id("high"), None);
    }

    #[test]
    fn the_table_holds_only_what_differs_from_the_default() {
        let d = PageTheme::Auto;
        let t = next_table(ThemeTable::new(), "a.test", PageTheme::Dark, d);
        let t = next_table(t, "b.test", PageTheme::Light, d);
        assert_eq!(
            t,
            ThemeTable::from([("a.test".into(), 1), ("b.test".into(), 2)])
        );
        let t = next_table(t, "a.test", PageTheme::Auto, d);
        assert_eq!(t, ThemeTable::from([("b.test".into(), 2)]));
        // With Dark the default, Dark removes the row and Auto is a choice of its own.
        let t = next_table(t, "b.test", PageTheme::Dark, PageTheme::Dark);
        assert!(t.is_empty());
        let t = next_table(t, "c.test", PageTheme::Auto, PageTheme::Dark);
        assert_eq!(t.get("c.test"), Some(&0));
    }

    #[test]
    fn wanted_reads_the_site_else_the_default() {
        let t = next_table(
            ThemeTable::new(),
            "a.test",
            PageTheme::Dark,
            PageTheme::Auto,
        );
        assert_eq!(
            wanted(&t, "https://A.test/x", PageTheme::Auto),
            PageTheme::Dark
        );
        assert_eq!(
            wanted(&t, "https://b.test/", PageTheme::Auto),
            PageTheme::Auto
        );
        assert_eq!(
            wanted(&t, "https://b.test/", PageTheme::Light),
            PageTheme::Light
        );
        assert_eq!(
            wanted(&t, "data:text/html,x", PageTheme::Dark),
            PageTheme::Dark
        );
        // A bad stored code reads as the default.
        let bad = ThemeTable::from([("a.test".into(), 9)]);
        assert_eq!(
            wanted(&bad, "https://a.test/", PageTheme::Light),
            PageTheme::Light
        );
    }

    #[test]
    fn each_theme_makes_its_two_calls() {
        let none = MediaPrefs::default();
        let methods = |t| {
            calls((t, none))
                .into_iter()
                .map(|(m, _)| m)
                .collect::<Vec<_>>()
        };
        for t in PageTheme::ALL {
            assert_eq!(
                methods(t),
                [
                    "Emulation.setEmulatedMedia",
                    "Emulation.setAutoDarkModeOverride"
                ]
            );
        }
        let params = |t| {
            calls((t, none))
                .into_iter()
                .map(|(_, p)| p)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            params(PageTheme::Dark),
            [
                json!({"features":[{"name":"prefers-color-scheme","value":"dark"}]}),
                json!({"enabled": true})
            ]
        );
        assert_eq!(
            params(PageTheme::Light),
            [
                json!({"features":[{"name":"prefers-color-scheme","value":"light"}]}),
                json!({"enabled": false})
            ]
        );
        // Auto resets both: no features, and no `enabled` (the override's reset).
        assert_eq!(params(PageTheme::Auto), [json!({"features":[]}), json!({})]);
    }

    #[test]
    fn sync_sends_once_and_again_after_a_reset() {
        let m = MediaPrefs::default();
        let (auto, dark) = ((PageTheme::Auto, m), (PageTheme::Dark, m));
        assert_eq!(sync(None, auto), Some(auto));
        assert_eq!(sync(Some(auto), auto), None);
        assert_eq!(sync(Some(auto), dark), Some(dark));
        assert_eq!(sync(Some(dark), dark), None);
        assert_eq!(sync(None, dark), Some(dark));
        // a changed preference is news too
        let reduced = (
            PageTheme::Auto,
            MediaPrefs {
                motion: Motion::Reduce,
                ..m
            },
        );
        assert_eq!(sync(Some(auto), reduced), Some(reduced));
    }

    #[test]
    fn media_prefs_join_the_theme_in_one_feature_list() {
        assert_eq!(Motion::from_id("reduce"), Some(Motion::Reduce));
        assert_eq!(Contrast::from_id("forced"), Some(Contrast::Forced));
        assert_eq!(Contrast::from_id("high"), None);
        let media = |motion, contrast| MediaPrefs { motion, contrast };
        let first = |shown| calls(shown).remove(0).1;
        assert_eq!(
            first((PageTheme::Dark, media(Motion::Reduce, Contrast::More))),
            json!({"features":[
                {"name":"prefers-color-scheme","value":"dark"},
                {"name":"prefers-reduced-motion","value":"reduce"},
                {"name":"prefers-contrast","value":"more"}
            ]})
        );
        assert_eq!(
            first((
                PageTheme::Auto,
                media(Motion::NoPreference, Contrast::Forced)
            )),
            json!({"features":[
                {"name":"prefers-reduced-motion","value":"no-preference"},
                {"name":"prefers-contrast","value":"more"},
                {"name":"forced-colors","value":"active"}
            ]})
        );
        // all automatic: the reset (no features), whatever was emulated before
        assert_eq!(
            first((PageTheme::Auto, MediaPrefs::default())),
            json!({"features":[]})
        );
    }
}
