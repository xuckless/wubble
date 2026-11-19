//! The chrome's pure rules (the design's `browser/chrome.tsx` and `command.tsx`): what the
//! address pill shows. Pure, no timers: the host runs the effects.

use crate::url;

/// What the address pill shows for `url`: the host in ink and the rest in haze (chrome.tsx
/// `addressOf`). wubble's own pages read `wubble` + `://settings`; no page (a new tab, the start
/// page) reads `wubble` + `://start`, as the design's; a `data:` page (bundled pages) is named
/// by its scheme alone, never its raw body.
pub fn address_parts(url: &str) -> (String, String) {
    let u = url.trim();
    if u.is_empty() || u.eq_ignore_ascii_case("about:blank") {
        return ("wubble".into(), "://start".into());
    }
    if u.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("data:")) {
        return ("data".into(), String::new());
    }
    let (host, path) = url::address_parts(u);
    (host.to_owned(), path.to_owned())
}

/// The pill's lock: a site over https (the design shows the lock for every site, the bubble for
/// its own pages); everything else gets the bubble.
pub fn is_secure(url: &str) -> bool {
    url.trim()
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(url: &str) -> (String, String) {
        address_parts(url)
    }

    #[test]
    fn a_site_shows_its_host_and_the_rest() {
        assert_eq!(
            parts("https://example.com/a/b?q=1"),
            ("example.com".into(), "/a/b?q=1".into())
        );
        assert_eq!(
            parts("http://localhost:8080/"),
            ("localhost:8080".into(), "/".into())
        );
    }

    #[test]
    fn own_pages_read_wubble_and_their_name() {
        assert_eq!(
            parts("wubble://settings"),
            ("wubble".into(), "://settings".into())
        );
        assert_eq!(
            parts("wubble://history"),
            ("wubble".into(), "://history".into())
        );
    }

    #[test]
    fn no_page_is_the_start_page() {
        for url in ["", "  ", "about:blank", "ABOUT:BLANK"] {
            assert_eq!(parts(url), ("wubble".into(), "://start".into()), "{url:?}");
        }
    }

    #[test]
    fn a_data_page_never_shows_its_body() {
        assert_eq!(
            parts("data:text/html;charset=utf-8,%3Ch1%3Ehi"),
            ("data".into(), String::new())
        );
        assert_eq!(parts("DATA:,x"), ("data".into(), String::new()));
    }

    #[test]
    fn only_https_sites_get_the_lock() {
        assert!(is_secure("https://example.com/"));
        assert!(is_secure("HTTPS://example.com"));
        for url in [
            "http://example.com",
            "wubble://settings",
            "",
            "data:,x",
            "file:///x",
        ] {
            assert!(!is_secure(url), "{url}");
        }
    }
}
