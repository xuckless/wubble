//! The filter lists wubble knows, as one table of data.
//!
//! The default set is uBlock Origin's (gorhill/uBlock `assets/assets.json`,
//! GPL-3.0: the nine lists without `off: true`), plus three optional
//! categories uBO also offers: URL tracking parameters and two cookie-notice
//! lists. Lists whose licence allows it ship as a snapshot inside the binary
//! (`data/lists`, refreshed by hand with the `snapshot` example), so blocking
//! works on a first run with nothing fetched; the others load only once
//! downloaded (`blocker.lists_auto_update`, or the settings page's explicit
//! update).

use std::collections::BTreeMap;

/// How a list's lines are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Adblock Plus / uBO filter syntax.
    Standard,
    /// A hosts file (`0.0.0.0 host`).
    Hosts,
}

/// What a list is for; decides which setting switches it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Ads,
    Privacy,
    Malware,
    /// Fixes for breakage the other lists cause.
    Unbreak,
    /// Cookie notices (`blocker.cookie_banners`).
    Cookies,
    /// URL tracking parameters (`blocker.strip_params`).
    Tracking,
}

/// One filter list.
#[derive(Debug, PartialEq)]
pub struct ListDef {
    /// Stable id: the setting's map key and the downloaded file's stem. uBO's
    /// asset id where uBO has the list.
    pub id: &'static str,
    pub title: &'static str,
    /// Where an update is fetched from (HTTPS).
    pub url: &'static str,
    /// Update period, from the list's own `Expires` header.
    pub interval_secs: u64,
    pub licence: &'static str,
    pub format: Format,
    pub category: Category,
    /// On unless the user turns it off.
    pub default_on: bool,
    /// The snapshot shipped in the binary, if its licence allows that.
    pub bundled: Option<&'static str>,
    /// uBO's own lists: may use `trusted-*` scriptlets (`PermissionMask` bit 0).
    pub trusted: bool,
    /// Keep only cosmetic hiding rules (no network rules, no scriptlets).
    pub cosmetic_only: bool,
    /// Lines containing any of these are dropped before parsing.
    pub drop_lines: &'static [&'static str],
}

const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;

const UBO: &str = "GPL-3.0 (uBlockOrigin/uAssets)";
const EASYLIST: &str = "GPL-3.0-or-later OR CC-BY-SA-3.0 (easylist.to)";

/// Cookie lists hide banners only: never a scriptlet that clicks consent or
/// writes a consent cookie, never a network rule.
const COOKIE_DROPS: &[&str] = &["+js("];

/// A uBO list: trusted, standard format, bundled.
const fn ubo(
    id: &'static str,
    title: &'static str,
    url: &'static str,
    interval_secs: u64,
    category: Category,
    bundled: &'static str,
) -> ListDef {
    ListDef {
        id,
        title,
        url,
        interval_secs,
        licence: UBO,
        format: Format::Standard,
        category,
        default_on: true,
        bundled: Some(bundled),
        trusted: true,
        cosmetic_only: false,
        drop_lines: &[],
    }
}

/// Every known list. The uBO CDN copies are used because they have uBO's
/// `!#include` directives already expanded.
pub const LISTS: &[ListDef] = &[
    ubo(
        "ublock-filters",
        "uBlock filters – Ads",
        "https://ublockorigin.github.io/uAssetsCDN/filters/filters.min.txt",
        5 * DAY,
        Category::Ads,
        include_str!("../data/lists/ublock-filters.txt"),
    ),
    ubo(
        "ublock-badware",
        "uBlock filters – Badware risks",
        "https://ublockorigin.github.io/uAssetsCDN/filters/badware.min.txt",
        5 * DAY,
        Category::Malware,
        include_str!("../data/lists/ublock-badware.txt"),
    ),
    ubo(
        "ublock-privacy",
        "uBlock filters – Privacy",
        "https://ublockorigin.github.io/uAssetsCDN/filters/privacy.min.txt",
        7 * DAY,
        Category::Privacy,
        include_str!("../data/lists/ublock-privacy.txt"),
    ),
    ubo(
        "ublock-unbreak",
        "uBlock filters – Unbreak",
        "https://ublockorigin.github.io/uAssetsCDN/filters/unbreak.min.txt",
        5 * DAY,
        Category::Unbreak,
        include_str!("../data/lists/ublock-unbreak.txt"),
    ),
    ubo(
        "ublock-quick-fixes",
        "uBlock filters – Quick fixes",
        "https://ublockorigin.github.io/uAssetsCDN/filters/quick-fixes.min.txt",
        8 * HOUR,
        Category::Unbreak,
        include_str!("../data/lists/ublock-quick-fixes.txt"),
    ),
    ListDef {
        id: "easylist",
        title: "EasyList",
        url: "https://easylist.to/easylist/easylist.txt",
        interval_secs: 6 * DAY,
        licence: EASYLIST,
        format: Format::Standard,
        category: Category::Ads,
        default_on: true,
        bundled: Some(include_str!("../data/lists/easylist.txt")),
        trusted: false,
        cosmetic_only: false,
        drop_lines: &[],
    },
    ListDef {
        id: "easyprivacy",
        title: "EasyPrivacy",
        url: "https://easylist.to/easylist/easyprivacy.txt",
        interval_secs: 6 * DAY,
        licence: EASYLIST,
        format: Format::Standard,
        category: Category::Privacy,
        default_on: true,
        bundled: Some(include_str!("../data/lists/easyprivacy.txt")),
        trusted: false,
        cosmetic_only: false,
        drop_lines: &[],
    },
    // Not bundled: 12 MB, and the repository's licence for the list itself
    // was not established. Loads once downloaded.
    ListDef {
        id: "urlhaus-1",
        title: "Online Malicious URL Blocklist",
        url: "https://malware-filter.gitlab.io/urlhaus-filter/urlhaus-filter-vivaldi-online.txt",
        interval_secs: 12 * HOUR,
        licence: "see malware-filter/urlhaus-filter (not bundled)",
        format: Format::Standard,
        category: Category::Malware,
        default_on: true,
        bundled: None,
        trusted: false,
        cosmetic_only: false,
        drop_lines: &[],
    },
    // Not bundled: no redistribution licence found. Loads once downloaded.
    ListDef {
        id: "plowe-0",
        title: "Peter Lowe’s Ad and tracking server list",
        url: "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=hosts&showintro=1&mimetype=plaintext",
        interval_secs: 13 * DAY,
        licence: "see pgl.yoyo.org (not bundled)",
        format: Format::Hosts,
        category: Category::Ads,
        default_on: true,
        bundled: None,
        trusted: false,
        cosmetic_only: false,
        drop_lines: &[],
    },
    ubo(
        "privacy-removeparam",
        "AdGuard/uBO – URL Tracking Protection",
        "https://ublockorigin.github.io/uAssetsCDN/filters/privacy-removeparam.txt",
        7 * DAY,
        Category::Tracking,
        include_str!("../data/lists/privacy-removeparam.txt"),
    ),
    ListDef {
        id: "easylist-cookies",
        title: "EasyList – Cookie Notices",
        url: "https://ublockorigin.github.io/uAssetsCDN/thirdparties/easylist-cookies.txt",
        interval_secs: 7 * DAY,
        licence: EASYLIST,
        format: Format::Standard,
        category: Category::Cookies,
        default_on: true,
        bundled: Some(include_str!("../data/lists/easylist-cookies.txt")),
        trusted: false,
        cosmetic_only: true,
        drop_lines: COOKIE_DROPS,
    },
    ListDef {
        cosmetic_only: true,
        drop_lines: COOKIE_DROPS,
        ..ubo(
            "ublock-cookies",
            "uBlock filters – Cookie Notices",
            "https://ublockorigin.github.io/uAssetsCDN/filters/annoyances-cookies.txt",
            7 * DAY,
            Category::Cookies,
            include_str!("../data/lists/ublock-cookies.txt"),
        )
    },
];

/// The list with this id.
pub fn find(id: &str) -> Option<&'static ListDef> {
    LISTS.iter().find(|l| l.id == id)
}

/// Which lists are on: the settings that pick them, as data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Per-list switches (`blocker.lists`: id to 0/1); absent ids keep their default.
    pub lists: BTreeMap<String, u32>,
    /// `blocker.cookie_banners`: the [`Category::Cookies`] lists.
    pub cookie_banners: bool,
    /// `blocker.strip_params`: the [`Category::Tracking`] lists.
    pub strip_params: bool,
    /// `blocker.custom_lists`: URLs of the user's own lists (HTTPS).
    pub custom: Vec<String>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            lists: BTreeMap::new(),
            cookie_banners: true,
            strip_params: true,
            custom: Vec::new(),
        }
    }
}

impl Selection {
    /// Whether `list` is on: its category's switch, then its own.
    pub fn includes(&self, list: &ListDef) -> bool {
        let category = match list.category {
            Category::Cookies => self.cookie_banners,
            Category::Tracking => self.strip_params,
            _ => true,
        };
        category
            && self
                .lists
                .get(list.id)
                .map_or(list.default_on, |&on| on != 0)
    }

    /// The lists that are on, in table order.
    pub fn enabled(&self) -> impl Iterator<Item = &'static ListDef> + '_ {
        LISTS.iter().filter(|l| self.includes(l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Blocker;

    #[test]
    fn ids_are_unique_and_urls_https() {
        let mut ids: Vec<_> = LISTS.iter().map(|l| l.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), LISTS.len());
        for l in LISTS {
            assert!(!l.id.is_empty() && !l.title.is_empty(), "{l:?}");
            assert!(l.url.starts_with("https://"), "{}", l.id);
            assert!(l.interval_secs >= HOUR, "{}", l.id);
        }
    }

    #[test]
    fn the_defaults_are_ubo_s_nine_plus_the_optional_categories() {
        let on: Vec<_> = Selection::default().enabled().map(|l| l.id).collect();
        assert_eq!(
            on,
            [
                "ublock-filters",
                "ublock-badware",
                "ublock-privacy",
                "ublock-unbreak",
                "ublock-quick-fixes",
                "easylist",
                "easyprivacy",
                "urlhaus-1",
                "plowe-0",
                "privacy-removeparam",
                "easylist-cookies",
                "ublock-cookies",
            ]
        );
    }

    #[test]
    fn category_switches_and_per_list_overrides() {
        let off = Selection {
            cookie_banners: false,
            strip_params: false,
            lists: [("easylist".to_owned(), 0), ("nope".to_owned(), 1)].into(),
            custom: Vec::new(),
        };
        let on: Vec<_> = off.enabled().map(|l| l.category).collect();
        assert!(!on.contains(&Category::Cookies) && !on.contains(&Category::Tracking));
        assert!(!off.includes(find("easylist").unwrap()));
        assert!(off.includes(find("easyprivacy").unwrap()));
        // A category switch wins over a list's own switch.
        let forced = Selection {
            cookie_banners: false,
            lists: [("ublock-cookies".to_owned(), 1)].into(),
            ..Selection::default()
        };
        assert!(!forced.includes(find("ublock-cookies").unwrap()));
    }

    #[test]
    fn only_ubo_lists_are_trusted_and_cookie_lists_hide_only() {
        for l in LISTS {
            assert_eq!(l.trusted, l.licence == UBO, "{}", l.id);
            assert_eq!(l.cosmetic_only, l.category == Category::Cookies, "{}", l.id);
        }
    }

    #[test]
    fn bundled_lists_are_non_empty_and_parse() {
        for l in LISTS {
            let Some(text) = l.bundled else { continue };
            assert!(text.len() > 10_000, "{}", l.id);
            assert!(text.starts_with('!') || text.starts_with('['), "{}", l.id);
        }
        // Parsing every list is the slow part; the whole bundled set compiles
        // and blocks a well-known ad host.
        let b = Blocker::from_sources(&crate::lists::bundled_sources(&Selection::default()));
        let req = crate::NetRequest {
            url: "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            source_url: "https://news.example/",
            resource_type: crate::ResourceType::Script,
        };
        assert!(matches!(
            b.decide(req, crate::Shields::On),
            crate::Decision::Block | crate::Decision::Redirect { .. }
        ));
    }
}
