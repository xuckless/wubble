use std::collections::BTreeSet;

use url::Url;

/// Whether blocking applies on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shields {
    On,
    Off,
}

/// Hosts where blocking is turned off. Pure data owned by the caller; a host entry
/// also covers its subdomains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SiteOverrides(BTreeSet<String>);

impl SiteOverrides {
    /// Add a host (case and one trailing dot ignored). Returns false if it was present
    /// or empty.
    pub fn insert(&mut self, host: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        let host = host.strip_suffix('.').unwrap_or(&host);
        !host.is_empty() && self.0.insert(host.to_owned())
    }

    /// Shields for a page. An unparsable URL or one without a host gets `On`.
    pub fn shields_for(&self, page_url: &str) -> Shields {
        let Some(host) = Url::parse(page_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
        else {
            return Shields::On;
        };
        let mut rest = host.strip_suffix('.').unwrap_or(&host);
        loop {
            if self.0.contains(rest) {
                return Shields::Off;
            }
            match rest.split_once('.') {
                Some((_, tail)) => rest = tail,
                None => return Shields::On,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides() -> SiteOverrides {
        let mut o = SiteOverrides::default();
        o.insert("example.com");
        o
    }

    #[test]
    fn host_and_subdomains_match() {
        let o = overrides();
        assert_eq!(o.shields_for("https://example.com/a"), Shields::Off);
        assert_eq!(o.shields_for("https://sub.example.com/"), Shields::Off);
        assert_eq!(o.shields_for("https://a.b.example.com/"), Shields::Off);
    }

    #[test]
    fn lookalike_hosts_do_not_match() {
        let o = overrides();
        assert_eq!(o.shields_for("https://notexample.com/"), Shields::On);
        assert_eq!(o.shields_for("https://example.com.evil.org/"), Shields::On);
    }

    #[test]
    fn trailing_dot_host_matches() {
        assert_eq!(
            overrides().shields_for("https://example.com./"),
            Shields::Off
        );
        assert_eq!(
            overrides().shields_for("https://a.example.com./"),
            Shields::Off
        );
    }

    #[test]
    fn insert_normalises_entries() {
        let mut o = SiteOverrides::default();
        assert!(o.insert(" Example.COM. "));
        assert!(!o.insert("example.com"));
        assert!(!o.insert(" "));
        assert_eq!(o.shields_for("https://example.com/"), Shields::Off);
    }

    #[test]
    fn bad_url_is_on() {
        assert_eq!(overrides().shields_for("not a url"), Shields::On);
        assert_eq!(overrides().shields_for("about:blank"), Shields::On);
    }
}
