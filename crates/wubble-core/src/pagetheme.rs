//! Dark mode for pages: Chromium's own forced dark (Blink's auto dark mode)
//! and the `prefers-color-scheme` the page sees, per site, switched over the
//! DevTools protocol per tab so it applies live.
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

/// The DevTools calls that show a page as `theme`, in order.
pub fn calls(theme: PageTheme) -> Vec<(&'static str, Value)> {
    let scheme =
        |value: &str| json!({ "features": [{ "name": "prefers-color-scheme", "value": value }] });
    let (media, dark) = match theme {
        PageTheme::Auto => (json!({ "features": [] }), json!({})),
        PageTheme::Dark => (scheme("dark"), json!({ "enabled": true })),
        PageTheme::Light => (scheme("light"), json!({ "enabled": false })),
    };
    vec![
        ("Emulation.setEmulatedMedia", media),
        ("Emulation.setAutoDarkModeOverride", dark),
    ]
}

/// Whether the view must be told: `Some(wanted)` unless it was last told exactly that.
/// `applied` is `None` for a view that was told nothing yet (a new view, a renderer that went
/// away), so the first page always gets its calls.
pub fn sync(applied: Option<PageTheme>, wanted: PageTheme) -> Option<PageTheme> {
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
        let methods = |t| calls(t).into_iter().map(|(m, _)| m).collect::<Vec<_>>();
        for t in PageTheme::ALL {
            assert_eq!(
                methods(t),
                [
                    "Emulation.setEmulatedMedia",
                    "Emulation.setAutoDarkModeOverride"
                ]
            );
        }
        let params = |t| calls(t).into_iter().map(|(_, p)| p).collect::<Vec<_>>();
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
        assert_eq!(sync(None, PageTheme::Auto), Some(PageTheme::Auto));
        assert_eq!(sync(Some(PageTheme::Auto), PageTheme::Auto), None);
        assert_eq!(
            sync(Some(PageTheme::Auto), PageTheme::Dark),
            Some(PageTheme::Dark)
        );
        assert_eq!(sync(Some(PageTheme::Dark), PageTheme::Dark), None);
        assert_eq!(sync(None, PageTheme::Dark), Some(PageTheme::Dark));
    }
}
