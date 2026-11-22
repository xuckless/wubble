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

/// A host as a rule names it: ASCII as typed, an internationalised name as the punycode a URL's
/// host has (`bücher.de` is `xn--bcher-kva.de`), so the rule can match it.
fn ascii_host(h: &str) -> Option<String> {
    if h.is_ascii() {
        Some(h.to_owned())
    } else {
        site_host(h)
    }
}

impl Rule {
    /// `None` for anything that is not a rule: no `->`, a bad host, or no name at all.
    pub fn parse(text: &str) -> Option<Rule> {
        let (pattern, target) = text.split_once("->")?;
        if target.contains("->") {
            return None;
        }
        let pattern = pattern.trim().to_lowercase();
        let pattern = pattern.strip_suffix('.').unwrap_or(&pattern);
        let host = |h: &str| ascii_host(h).filter(|h| valid_host(h));
        let pattern = match pattern.strip_prefix("*.") {
            Some(base) => Pattern::Subdomains(host(base)?),
            None => Pattern::Host(host(pattern)?),
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

/// The host a typed site stands for, as the site lists keep it: lowercase, no scheme, port, path
/// or credentials, no leading `*.` (an entry covers its subdomains already) and no trailing dot;
/// an internationalised name as its punycode. `None` for what is no site: empty, or with a space.
pub fn site_host(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix("*.").unwrap_or(text);
    if text.is_empty() || text.contains(|c: char| c.is_whitespace() || c == '@') {
        return None;
    }
    let host = if text.contains("://") {
        host_key(text)
    } else {
        host_key(&format!("http://{text}"))
    };
    host.filter(|h| !h.starts_with('.') && !h.contains(".."))
}

/// Whether two rule texts are one rule: the same sites and the same names (case aside, as
/// [`resolve`] reads names).
pub fn same_rule(a: &str, b: &str) -> bool {
    match (Rule::parse(a), Rule::parse(b)) {
        (Some(a), Some(b)) => {
            let lower = |n: Option<String>| n.map(|n| n.to_lowercase());
            a.pattern == b.pattern
                && lower(a.space) == lower(b.space)
                && lower(a.container) == lower(b.container)
        }
        _ => a == b,
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
    // an agent Space and its container are the session's: a link of the user's never lands there,
    // where the agent could read it (a hand-off makes them the owner's, but still never by rule).
    // A private window's Space and its in-memory container are no rule's either.
    let agent_containers: Vec<ContainerId> = model
        .spaces
        .iter()
        .filter(|s| s.agent || s.private)
        .map(|s| s.container)
        .collect();
    let rules: Vec<(Rule, Routed)> = entries
        .iter()
        .filter_map(|e| Rule::parse(e.as_ref()))
        .filter_map(|r| {
            let space = match &r.space {
                Some(n) => Some(
                    model
                        .spaces
                        .iter()
                        .find(|s| !s.agent && !s.private && s.name.eq_ignore_ascii_case(n))?
                        .id,
                ),
                None => None,
            };
            let container = match &r.container {
                Some(n) => Some(
                    model
                        .containers
                        .iter()
                        .find(|c| {
                            !agent_containers.contains(&c.id) && c.name.eq_ignore_ascii_case(n)
                        })?
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
    fn an_international_name_is_a_rule_on_its_punycode() {
        let r = Rule::parse("Bücher.de -> Work").unwrap();
        assert_eq!(r.pattern, Pattern::Host("xn--bcher-kva.de".into()));
        let w = Rule::parse("*.bücher.de -> Work").unwrap();
        assert_eq!(w.pattern, Pattern::Subdomains("xn--bcher-kva.de".into()));
        // the URL's host is punycode: both rules reach it
        assert!(route(&[r], "https://bücher.de/").is_some());
        assert!(route(&[w], "https://shop.xn--bcher-kva.de/").is_some());
    }

    #[test]
    fn site_host_is_the_one_entry_a_site_list_keeps() {
        for (typed, host) in [
            ("example.com", "example.com"),
            ("  Example.COM. ", "example.com"),
            ("https://www.Example.com:8443/a/b?c#d", "www.example.com"),
            ("example.com:8080", "example.com"),
            ("*.example.com", "example.com"),
            ("127.0.0.1:3000", "127.0.0.1"),
            ("[::1]:80", "[::1]"),
            ("bücher.de", "xn--bcher-kva.de"),
            ("localhost", "localhost"),
        ] {
            assert_eq!(site_host(typed).as_deref(), Some(host), "{typed}");
        }
        for bad in [
            "",
            "  ",
            "a b.com",
            "me@example.com",
            "http://",
            "a..b",
            "\u{0}",
        ] {
            assert_eq!(site_host(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_rule_typed_twice_is_one_rule() {
        assert!(same_rule("A.com -> Work, Bank", "a.com->work ,bank"));
        assert!(same_rule("a.com. -> Work", "a.com -> Work"));
        assert!(!same_rule("a.com -> Work", "a.com -> Home"));
        assert!(!same_rule("a.com -> Work", "*.a.com -> Work"));
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

    /// A rule must not carry the user's link into a session's Space or container.
    #[test]
    fn a_rule_never_names_an_agent_space_or_its_container() {
        use crate::model::{Command, apply};
        let m = Model::new(std::time::Instant::now());
        let (m, _) = apply(
            m,
            Command::NewContainer {
                name: "Agent 1".into(),
            },
        );
        let agent_container = m.containers[1].id;
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "Agent: draft".into(),
                tint: crate::space::Rgb(1, 2, 3),
                container: agent_container,
            },
        );
        let agent_space = m.spaces[1].id;
        let (m, _) = apply(m, Command::MarkAgentSpace(agent_space));
        let entries = [
            "a.test -> Agent: draft",
            "b.test -> , Agent 1",
            "c.test -> Space 1",
        ];
        let at = |url| resolve(&m, &entries, url);
        assert_eq!(at("https://a.test/"), Routed::default());
        assert_eq!(at("https://b.test/"), Routed::default());
        assert_eq!(at("https://c.test/").space, Some(m.spaces[0].id));
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
