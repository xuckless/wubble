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
pub struct SiteOverrides(pub BTreeSet<String>);

impl SiteOverrides {
    /// Shields for a page. An unparsable URL or one without a host gets `On`.
    pub fn shields_for(&self, page_url: &str) -> Shields {
        let Some(host) = Url::parse(page_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
        else {
            return Shields::On;
        };
        let mut rest = host.as_str();
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
        SiteOverrides(["example.com".to_owned()].into())
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
    fn bad_url_is_on() {
        assert_eq!(overrides().shields_for("not a url"), Shields::On);
        assert_eq!(overrides().shields_for("about:blank"), Shields::On);
    }
}
