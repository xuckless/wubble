//! Link routing: which Space and container a link opens in, by host.
//!
//! A rule is one line of text, `pattern -> Space` or `pattern -> Space, Container` (either
//! name may be left out: `pattern -> , Container`), kept as an entry of the `routing.rules`
//! List setting. The matcher is pure; the host resolves the names to ids.
//!
//! Pattern grammar and "more specific wins" follow Chromium's content-settings patterns
//! (`ContentSettingsPattern`, BSD-3-Clause; the idea only): `example.com` matches that host,
//! `*.example.com` any host below it, not the apex itself. Ties go to the earlier rule.

use std::cmp::Reverse;

use crate::model::Model;
use crate::space::{ContainerId, SpaceId};

/// Which hosts a rule covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// This host only.
    Host(String),
    /// Any host below this one, at any depth.
    Subdomains(String),
}

impl Pattern {
    fn matches(&self, host: &str) -> bool {
        match self {
            Pattern::Host(h) => h == host,
            Pattern::Subdomains(base) => {
                // an IP literal has no subdomains: `*.0.0.1` must not cover 127.0.0.1
                host.parse::<std::net::IpAddr>().is_err()
                    && host
                        .strip_suffix(base.as_str())
                        .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1)
            }
        }
    }

    /// Higher is more specific: an exact host beats any wildcard, a wildcard with more
    /// labels beats one with fewer.
    fn specificity(&self) -> (bool, usize) {
        match self {
            Pattern::Host(_) => (true, 0),
            Pattern::Subdomains(base) => (false, base.split('.').count()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub pattern: Pattern,
    /// The Space's name, `None` when the rule only picks a container.
    pub space: Option<String>,
    pub container: Option<String>,
}

fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && !h.starts_with('.')
        && !h.contains("..")
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

impl Rule {
    /// `None` for anything that is not a rule: no `->`, a bad host, or no name at all.
    pub fn parse(text: &str) -> Option<Rule> {
        let (pattern, target) = text.split_once("->")?;
        if target.contains("->") {
            return None;
        }
        let pattern = pattern.trim().to_ascii_lowercase();
        let pattern = pattern.strip_suffix('.').unwrap_or(&pattern);
        let pattern = match pattern.strip_prefix("*.") {
            Some(base) if valid_host(base) => Pattern::Subdomains(base.to_owned()),
            Some(_) => return None,
            None if valid_host(pattern) => Pattern::Host(pattern.to_owned()),
            None => return None,
        };
        let (space, container) = match target.split_once(',') {
            Some((s, c)) => (s, c),
            None => (target, ""),
        };
        let name = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_owned())
        };
        let (space, container) = (name(space), name(container));
        if space.is_none() && container.is_none()
            || container.as_deref().is_some_and(|c| c.contains(','))
        {
            return None;
        }
        Some(Rule {
            pattern,
            space,
            container,
        })
    }
}

/// The lowercased host of a URL, or of a bare host, without port, credentials or a trailing
/// dot. `None` for URLs without a host (`about:`, `data:`, internal pages).
pub fn host_key(url_or_host: &str) -> Option<String> {
    let host = if url_or_host.contains("://") {
        ::url::Url::parse(url_or_host)
            .ok()?
            .host_str()?
            .to_ascii_lowercase()
    } else {
        url_or_host.trim().to_ascii_lowercase()
    };
    let host = host.strip_suffix('.').unwrap_or(&host);
    (!host.is_empty()).then(|| host.to_owned())
}

/// The rule that covers `url`: the most specific one, the earlier of equals. IP literals only
/// match exactly (no wildcard has an IP as its base).
pub fn route<'a>(rules: &'a [Rule], url: &str) -> Option<&'a Rule> {
    let host = host_key(url)?;
    rules
        .iter()
        .enumerate()
        .filter(|(_, r)| r.pattern.matches(&host))
        .max_by_key(|(i, r)| (r.pattern.specificity(), Reverse(*i)))
        .map(|(_, r)| r)
}

/// Where a link opens: the Space and container a rule names, `None` for what it leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Routed {
    pub space: Option<SpaceId>,
    pub container: Option<ContainerId>,
}

/// The place `url` is routed to by the stored rule `entries` (the `routing.rules` setting),
/// names resolved in `model` ignoring case. A malformed entry, or a rule naming a Space or
/// container that does not exist, is skipped, so the next most specific rule may apply.
pub fn resolve<S: AsRef<str>>(model: &Model, entries: &[S], url: &str) -> Routed {
    let rules: Vec<(Rule, Routed)> = entries
        .iter()
        .filter_map(|e| Rule::parse(e.as_ref()))
        .filter_map(|r| {
            let space = match &r.space {
                Some(n) => Some(
                    model
                        .spaces
                        .iter()
                        .find(|s| s.name.eq_ignore_ascii_case(n))?
                        .id,
                ),
                None => None,
            };
            let container = match &r.container {
                Some(n) => Some(
                    model
                        .containers
                        .iter()
                        .find(|c| c.name.eq_ignore_ascii_case(n))?
                        .id,
                ),
                None => None,
            };
            Some((r, Routed { space, container }))
        })
        .collect();
    let plain: Vec<Rule> = rules.iter().map(|(r, _)| r.clone()).collect();
    route(&plain, url)
        .and_then(|hit| rules.iter().find(|(r, _)| r == hit))
        .map_or_else(Routed::default, |(_, routed)| *routed)
}

/// Whether `url_or_host` is one of `hosts` or below one of them. The one matcher for a list of
/// sites (the never-sleep sites).
pub fn host_in<S: AsRef<str>>(hosts: &[S], url_or_host: &str) -> bool {
    let Some(host) = host_key(url_or_host) else {
        return false;
    };
    hosts.iter().any(|h| {
        host_key(h.as_ref()).is_some_and(|h| h == host || Pattern::Subdomains(h).matches(&host))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(lines: &[&str]) -> Vec<Rule> {
        lines.iter().map(|l| Rule::parse(l).expect(l)).collect()
    }

    fn space_of(rules: &[Rule], url: &str) -> Option<String> {
        route(rules, url).and_then(|r| r.space.clone())
    }

    #[test]
    fn parses_rules() {
        let r = Rule::parse("GitHub.com -> Work").unwrap();
        assert_eq!(r.pattern, Pattern::Host("github.com".into()));
        assert_eq!(r.space.as_deref(), Some("Work"));
        assert_eq!(r.container, None);

        let r = Rule::parse("*.corp.example -> My Space , Banking").unwrap();
        assert_eq!(r.pattern, Pattern::Subdomains("corp.example".into()));
        assert_eq!(r.space.as_deref(), Some("My Space"));
        assert_eq!(r.container.as_deref(), Some("Banking"));

        let r = Rule::parse("a.com -> , Banking").unwrap();
        assert_eq!((r.space, r.container.as_deref()), (None, Some("Banking")));

        for bad in [
            "",
            "a.com",
            "a.com ->",
            "a.com -> ,",
            "-> Work",
            "a.com -> b -> c",
            "a b.com -> Work",
            "*. -> Work",
            "*.*.a.com -> Work",
            "a.com/x -> Work",
            "a.com -> W, C, D",
            ".a.com -> Work",
        ] {
            assert_eq!(Rule::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn exact_beats_wildcard_and_longer_wildcard_beats_shorter() {
        let r = rules(&[
            "*.example.com -> Wild",
            "*.a.example.com -> Longer",
            "a.example.com -> Exact",
        ]);
        assert_eq!(
            space_of(&r, "https://a.example.com/").as_deref(),
            Some("Exact")
        );
        assert_eq!(
            space_of(&r, "https://b.a.example.com/").as_deref(),
            Some("Longer")
        );
        assert_eq!(
            space_of(&r, "https://c.example.com/").as_deref(),
            Some("Wild")
        );
    }

    #[test]
    fn ties_go_to_the_earlier_rule() {
        let r = rules(&[
            "a.com -> First",
            "a.com -> Second",
            "*.b.com -> X",
            "*.b.com -> Y",
        ]);
        assert_eq!(space_of(&r, "https://a.com").as_deref(), Some("First"));
        assert_eq!(space_of(&r, "https://c.b.com").as_deref(), Some("X"));
    }

    #[test]
    fn wildcard_skips_the_apex_and_lookalikes() {
        let r = rules(&["*.example.com -> W"]);
        assert_eq!(route(&r, "https://example.com/"), None);
        assert_eq!(route(&r, "https://badexample.com/"), None);
        assert!(route(&r, "https://x.y.example.com/").is_some());
    }

    #[test]
    fn host_is_normalised() {
        let r = rules(&["example.com -> W"]);
        for url in [
            "HTTPS://Example.COM/path",
            "https://example.com:8443/",
            "https://example.com./",
            "https://user:pw@example.com/",
        ] {
            assert!(route(&r, url).is_some(), "{url}");
        }
    }

    #[test]
    fn urls_without_a_host_and_ip_literals() {
        let r = rules(&["*.0.0.1 -> W", "127.0.0.1 -> L"]);
        assert_eq!(route(&r[..1], "http://127.0.0.1/"), None);
        assert_eq!(route(&r, "about:blank"), None);
        assert_eq!(route(&r, "wubble://settings"), None);
        assert_eq!(space_of(&r, "http://127.0.0.1:3000/").as_deref(), Some("L"));
        assert_eq!(route(&[], "https://a.com"), None);
    }

    #[test]
    fn host_in_covers_subdomains() {
        let sites = ["example.com".to_owned(), "Docs.Rs".to_owned()];
        assert!(host_in(&sites, "https://example.com/x"));
        assert!(host_in(&sites, "https://www.example.com/x"));
        assert!(host_in(&sites, "docs.rs"));
        assert!(!host_in(&sites, "https://notexample.com"));
        assert!(!host_in(&sites, "about:blank"));
        assert!(!host_in::<String>(&[], "https://example.com"));
    }

    #[test]
    fn resolve_names_a_space_and_a_container_and_skips_unknown_ones() {
        use crate::model::{Command, apply};
        let m = Model::new(std::time::Instant::now());
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "Work".into(),
                tint: crate::space::Rgb(1, 2, 3),
                container: ContainerId::DEFAULT,
            },
        );
        let (m, _) = apply(
            m,
            Command::NewContainer {
                name: "Banking".into(),
            },
        );
        let work = m.spaces[1].id;
        let bank = m.containers[1].id;
        let entries = [
            "*.example.com -> Nowhere",
            "a.example.com -> WORK, banking",
            "bank.test -> , Banking",
            "broken",
            "c.test -> Work, NoSuchContainer",
        ];
        let at = |url| resolve(&m, &entries, url);
        assert_eq!(
            at("https://a.example.com/"),
            Routed {
                space: Some(work),
                container: Some(bank)
            }
        );
        assert_eq!(
            at("https://bank.test/x"),
            Routed {
                space: None,
                container: Some(bank)
            }
        );
        // the wildcard names a missing Space: skipped, nothing else covers b.example.com
        assert_eq!(at("https://b.example.com/"), Routed::default());
        assert_eq!(at("https://c.test/"), Routed::default());
        assert_eq!(at("about:blank"), Routed::default());
    }
}
