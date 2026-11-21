//! Links to schemes the browser cannot load (`mailto:`, `tel:`, `magnet:`, an app's own scheme):
//! whether one is opened in the app the OS has for it, asked about first, or refused.
//!
//! Chromium's behaviour (`ExternalProtocolHandler::LaunchUrl` and its dialog, BSD-3-Clause; no
//! code copied): a link opens another app only after a user gesture, the user is asked "Open
//! <app>?" unless they ticked "Always allow <site> to open links of this type", and that choice
//! is kept per site and scheme (`protocol_handler.allowed_origin_protocol_pairs`). wubble keeps
//! it in the site permission table as [`Permission::ExternalLinks`] under the origin [`key`]
//! makes, so there is one owner of per-site decisions.

use crate::site::{self, Decision, Permission, SitePermissions};

/// What happens to one external link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Hand it to the OS now.
    Open,
    /// Ask the user first.
    Ask,
    /// Drop it: no gesture, or the user said never for this site and scheme.
    Block,
}

/// The scheme of `url`, lowercase; `None` without one.
pub fn scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// The origin a decision for `scheme` links on the site `site` (an origin from
/// `site::origin_of`) is stored under.
pub fn key(scheme: &str, site: &str) -> String {
    format!("{scheme}:{site}")
}

/// The scheme and site a stored [`key`] names, for the site settings list.
pub fn parse_key(key: &str) -> Option<(&str, &str)> {
    let (scheme, site) = key.split_once(':')?;
    site.contains("://").then_some((scheme, site))
}

/// The decision key for a link to `url` from the page `page`: `None` when either has no
/// scheme or the page has an opaque origin (a `file:` page is asked each time).
pub fn key_for(url: &str, page: &str) -> Option<String> {
    Some(key(&scheme(url)?, &site::origin_of(page)?))
}

/// Schemes never handed to the OS even with a user gesture or a remembered decision.
const DENIED_SCHEMES: &[&str] = &["search-ms", "ms-msdt", "shell", "smb", "dav", "vbscript"];

/// What a link to `url` from the page `page` gets. A link the page opened without a user
/// gesture never opens another app (Chromium blocks it too); otherwise a remembered choice for
/// the site and scheme applies, and the rest is asked.
pub fn decide(url: &str, page: &str, gesture: bool, table: &SitePermissions) -> Verdict {
    let Some(scheme) = scheme(url) else {
        return Verdict::Block;
    };
    if !gesture || DENIED_SCHEMES.contains(&scheme.as_str()) {
        return Verdict::Block;
    }
    let remembered = key_for(url, page).and_then(|k| table.get(&k, Permission::ExternalLinks));
    match remembered {
        Some(Decision::Allow) => Verdict::Open,
        Some(Decision::Block) => Verdict::Block,
        None => Verdict::Ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://a.test/contact";

    #[test]
    fn schemes_are_lowercase_and_must_be_valid() {
        assert_eq!(scheme("MailTo:a@b").as_deref(), Some("mailto"));
        assert_eq!(scheme("web+app:x").as_deref(), Some("web+app"));
        assert_eq!(scheme("magnet:?xt=urn"), Some("magnet".into()));
        assert_eq!(scheme("no scheme"), None);
        assert_eq!(scheme("1abc:x"), None);
        assert_eq!(scheme("a b:x"), None);
    }

    #[test]
    fn keys_join_scheme_and_site_and_split_back() {
        assert_eq!(key("mailto", "https://a.test"), "mailto:https://a.test");
        assert_eq!(
            parse_key("mailto:https://a.test"),
            Some(("mailto", "https://a.test"))
        );
        assert_eq!(
            parse_key("https://a.test"),
            None,
            "a plain origin is not a key"
        );
        assert_eq!(
            key_for("tel:123", PAGE).as_deref(),
            Some("tel:https://a.test")
        );
        assert_eq!(key_for("tel:123", "file:///x.html"), None);
    }

    #[test]
    fn decide_table() {
        let mut t = SitePermissions::default();
        let tel = "tel:123";
        assert_eq!(decide(tel, PAGE, false, &t), Verdict::Block, "no gesture");
        assert_eq!(decide(tel, PAGE, true, &t), Verdict::Ask);
        t.set(
            "tel:https://a.test",
            Permission::ExternalLinks,
            Decision::Allow,
        );
        assert_eq!(decide(tel, PAGE, true, &t), Verdict::Open);
        assert_eq!(
            decide(tel, PAGE, false, &t),
            Verdict::Block,
            "remembered still needs a gesture"
        );
        assert_eq!(
            decide("mailto:a@b", PAGE, true, &t),
            Verdict::Ask,
            "per scheme"
        );
        assert_eq!(
            decide(tel, "https://b.test/", true, &t),
            Verdict::Ask,
            "per site"
        );
        t.set(
            "tel:https://a.test",
            Permission::ExternalLinks,
            Decision::Block,
        );
        assert_eq!(decide(tel, PAGE, true, &t), Verdict::Block);
        assert_eq!(
            decide(tel, "file:///x", true, &t),
            Verdict::Ask,
            "opaque pages ask"
        );
        assert_eq!(decide("nonsense", PAGE, true, &t), Verdict::Block);
    }

    #[test]
    fn dangerous_schemes_are_always_blocked() {
        let mut t = SitePermissions::default();
        for url in [
            "search-ms:query=x",
            "SEARCH-MS:query=x",
            "ms-msdt:/id",
            "Ms-Msdt:/id",
            "shell:desktop",
            "smb://host/share",
            "SMB://host/share",
            "dav://host/path",
            "vbscript:msgbox(1)",
            "VBScript:msgbox(1)",
        ] {
            if let Some(k) = key_for(url, PAGE) {
                t.set(&k, Permission::ExternalLinks, Decision::Allow);
            }
            assert_eq!(
                decide(url, PAGE, true, &t),
                Verdict::Block,
                "{url} must be blocked even with gesture and remembered Allow"
            );
        }
    }
}
