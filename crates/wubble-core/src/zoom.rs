//! Per-site zoom: one table from host to zoom percent, the one owner of a
//! site's zoom. CEF keeps a per-host level in memory too; that is only a
//! cache the shell overwrites from this table when a tab commits a URL.
//!
//! Design reference, no code copied: Chromium's per-host zoom map
//! (`content/browser/host_zoom_map_impl.cc`: keyed by host, a default zoom
//! removes the entry), BSD-3-Clause.

use std::collections::BTreeMap;

use crate::url;

/// Host to zoom percent. Hosts at the default are absent.
pub type ZoomTable = BTreeMap<String, u32>;

/// The default zoom before the user sets another (`settings::ZOOM_DEFAULT`).
pub const DEFAULT_PERCENT: u32 = 100;

/// The key a URL's zoom is stored under: its host, lowercased, without a
/// port's absence mattering (`localhost:8080` is its own site). `None` for
/// URLs without a host (`about:`, `data:`, `file:`), which always show at
/// the default.
pub fn host_of(page: &str) -> Option<String> {
    if !page.contains("://") {
        return None;
    }
    let (host, _) = url::address_parts(page);
    // Credentials are not part of the site.
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The table after the user zoomed `host` to `percent`: the default zoom
/// (`default`) removes the row, so the table only holds what differs.
pub fn next_table(mut table: ZoomTable, host: &str, percent: u32, default: u32) -> ZoomTable {
    if percent == default {
        table.remove(host);
    } else {
        table.insert(host.to_string(), percent);
    }
    table
}

/// The zoom a page at `page` should show: its site's, else `default`.
pub fn wanted(table: &ZoomTable, page: &str, default: u32) -> u32 {
    host_of(page)
        .and_then(|host| table.get(&host).copied())
        .unwrap_or(default)
}

/// What the address pill's zoom chip shows for a page at `percent`: the percent while it is
/// not the user's default (Chrome's omnibox zoom icon), else no chip.
pub fn indicator(percent: u32, default: u32) -> Option<u32> {
    (percent != default).then_some(percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_shows_a_zoom_other_than_the_default() {
        for (percent, default, want) in [
            (100, 100, None),
            (125, 100, Some(125)),
            (90, 100, Some(90)),
            (100, 125, Some(100)),
            (125, 125, None),
            (25, 100, Some(25)),
            (500, 100, Some(500)),
        ] {
            assert_eq!(indicator(percent, default), want, "{percent} {default}");
        }
    }

    #[test]
    fn hosts_are_the_key() {
        let cases = [
            ("https://Example.com/a?b", Some("example.com")),
            ("http://localhost:8080/", Some("localhost:8080")),
            ("https://user:pw@site.test/x", Some("site.test")),
            ("https://日本.jp/", Some("日本.jp")),
            ("about:blank", None),
            ("data:text/html,x", None),
            ("file:///tmp/a.html", None),
            ("", None),
        ];
        for (page, want) in cases {
            assert_eq!(host_of(page).as_deref(), want, "{page}");
        }
    }

    #[test]
    fn the_default_removes_the_row() {
        let d = DEFAULT_PERCENT;
        let t = next_table(ZoomTable::new(), "a.test", 125, d);
        assert_eq!(t.get("a.test"), Some(&125));
        let t = next_table(t, "b.test", 90, d);
        let t = next_table(t, "a.test", DEFAULT_PERCENT, d);
        assert_eq!(t, ZoomTable::from([("b.test".to_string(), 90)]));
        // Another default: 100 is a level of its own, the new default removes the row.
        let t = next_table(t, "b.test", 100, 125);
        assert_eq!(t.get("b.test"), Some(&100));
        assert!(next_table(t, "b.test", 125, 125).is_empty());
    }

    /// Reset zoom shows the user's default, not CEF's 100 %: the site's row goes and the
    /// page shows at the default again.
    #[test]
    fn a_reset_to_another_default_forgets_the_site() {
        let d = 125;
        let t = next_table(ZoomTable::new(), "a.test", 150, d);
        let t = next_table(t, "a.test", d, d);
        assert!(t.is_empty());
        assert_eq!(wanted(&t, "https://a.test/", d), d);
    }

    #[test]
    fn wanted_reads_the_table_or_the_default() {
        let d = DEFAULT_PERCENT;
        let t = next_table(ZoomTable::new(), "a.test", 150, d);
        assert_eq!(wanted(&t, "https://a.test/page", d), 150);
        assert_eq!(wanted(&t, "https://A.test/", d), 150);
        assert_eq!(wanted(&t, "https://b.test/", d), DEFAULT_PERCENT);
        assert_eq!(wanted(&t, "about:blank", d), DEFAULT_PERCENT);
        assert_eq!(wanted(&t, "https://b.test/", 80), 80);
    }
}
