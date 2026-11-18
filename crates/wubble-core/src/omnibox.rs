//! Command-bar brain: classify what was typed and rank the results.
//!
//! Pure and deterministic: no clocks (`now_ms` is a parameter), no I/O. The
//! store lane supplies history entries, the wiring converts [`OmniResult`] to
//! the Slint `CommandResult` (kinds map 1:1 onto `CommandKind`).
//!
//! `classify` re-expresses the rule order of Chromium's
//! `AutocompleteInput::Parse` (components/omnibox/browser/autocomplete_input.cc,
//! BSD-3-Clause; no code copied). Canonicalisation is delegated to the `url`
//! crate, which plays the role of Chromium's url_fixer.

use ::url::{Host, Url};

use crate::tab::TabId;
use crate::url::percent_encode_into;

// ---------------------------------------------------------------- engines

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEngine {
    Google,
    DuckDuckGo,
}

/// One search engine as data. Templates contain exactly one `{q}`.
#[derive(Debug, PartialEq, Eq)]
pub struct EngineInfo {
    pub engine: SearchEngine,
    pub name: &'static str,
    pub search: &'static str,
    /// OpenSearch suggestions JSON: `["query", ["a", "b", ...]]`.
    pub suggest: &'static str,
}

pub const ENGINES: [EngineInfo; 2] = [
    EngineInfo {
        engine: SearchEngine::Google,
        name: "Google",
        search: "https://www.google.com/search?q={q}",
        suggest: "https://suggestqueries.google.com/complete/search?client=firefox&q={q}",
    },
    EngineInfo {
        engine: SearchEngine::DuckDuckGo,
        name: "DuckDuckGo",
        search: "https://duckduckgo.com/?q={q}",
        suggest: "https://duckduckgo.com/ac/?type=list&q={q}",
    },
];

impl SearchEngine {
    pub fn info(self) -> &'static EngineInfo {
        ENGINES
            .iter()
            .find(|e| e.engine == self)
            .expect("every engine has a table row")
    }
}

/// Substitutes the percent-encoded `query` for `{q}` in `template`.
pub fn fill(template: &str, query: &str) -> String {
    let (head, tail) = template.split_once("{q}").unwrap_or((template, ""));
    let mut out = String::from(head);
    percent_encode_into(query, &mut out);
    out.push_str(tail);
    out
}

pub fn search_url(engine: SearchEngine, query: &str) -> String {
    fill(engine.info().search, query)
}

pub fn suggest_url(engine: SearchEngine, query: &str) -> String {
    fill(engine.info().suggest, query)
}

// --------------------------------------------------------------- classify

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Normalised URL to navigate to.
    Url(String),
    /// Trimmed query to hand to the search engine.
    Search(String),
}

/// Schemes that are URLs without `//` (Chromium: about, data, view-source).
const OPAQUE_SCHEMES: [&str; 3] = ["about", "data", "view-source"];
/// Schemes accepted when typed as `scheme://...`.
const HIER_SCHEMES: [&str; 4] = ["http", "https", "ftp", "file"];
/// Reserved TLDs that only count as a host with a subdomain (Chromium).
const RESERVED_TLDS: [&str; 4] = ["example", "test", "local", "internal"];

pub fn classify(input: &str) -> Intent {
    let s = input.trim();
    if let Some(q) = s.strip_prefix('?') {
        return Intent::Search(q.trim().to_owned());
    }
    match parse_url_intent(s) {
        Some(url) => Intent::Url(url),
        None => Intent::Search(s.to_owned()),
    }
}

fn parse_url_intent(s: &str) -> Option<String> {
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    // Explicit scheme.
    if let Some((scheme, rest)) = s.split_once(':') {
        let scheme = scheme.to_ascii_lowercase();
        let hier = rest.starts_with("//") && HIER_SCHEMES.contains(&scheme.as_str());
        if hier || OPAQUE_SCHEMES.contains(&scheme.as_str()) {
            let url = Url::parse(s).ok()?;
            if hier && url.scheme() != "file" && url.host().is_none() {
                return None;
            }
            return Some(url.into());
        }
        // Anything else (`localhost:3000`, `foo:bar`, `javascript:x`) is
        // re-parsed as a host below, as Chromium re-parses with `http://`.
    }
    let mut url = Url::parse(&format!("http://{s}")).ok()?;
    let host = url.host()?;

    let auth_end = s.find(['/', '?', '#']).unwrap_or(s.len());
    let (authority, rest) = s.split_at(auth_end);
    let hostport = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let typed_host = if hostport.starts_with('[') {
        hostport
    } else {
        hostport.rsplit_once(':').map_or(hostport, |(h, _)| h)
    };

    let local = match host {
        Host::Ipv6(_) => true,
        Host::Ipv4(ip) => {
            // `13.5` parses as 13.0.0.5; only four typed components count.
            let octets = ip.octets();
            if typed_host.split('.').count() != 4 || (octets[0] == 0 && octets != [0; 4]) {
                return None;
            }
            true
        }
        Host::Domain(d) => d == "localhost" || d.ends_with(".localhost"),
    };

    // Email-like `user@host`: a username without a password is not a URL.
    if !url.username().is_empty() && url.password().is_none() {
        return None;
    }
    let is_url = local
        || url.port().is_some()
        || rest.ends_with('/')
        || non_host_components(&url) > 1
        || matches!(host_of(&url), Some(d) if has_known_tld(d));
    if !is_url {
        return None;
    }
    // Scheme-less hosts get https; IP and localhost stay http.
    if !local {
        url.set_scheme("https").ok()?;
    }
    Some(url.into())
}

fn host_of(url: &Url) -> Option<&str> {
    match url.host()? {
        Host::Domain(d) => Some(d),
        _ => None,
    }
}

/// Port, path, query, fragment and userinfo each count once.
fn non_host_components(url: &Url) -> usize {
    [
        url.port().is_some(),
        url.path().len() > 1,
        url.query().is_some(),
        url.fragment().is_some(),
        !url.username().is_empty() || url.password().is_some(),
    ]
    .into_iter()
    .filter(|&b| b)
    .count()
}

/// Heuristic stand-in for a public-suffix lookup; the one place to swap for
/// the `psl` crate if exactness ever matters.
fn has_known_tld(domain: &str) -> bool {
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    let Some((_, tld)) = domain.rsplit_once('.') else {
        return false;
    };
    let labels = domain.split('.').count();
    if RESERVED_TLDS.contains(&tld) {
        return labels > 2;
    }
    tld.starts_with("xn--") || (tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()))
}

// ------------------------------------------------------------------- rank

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenTab {
    pub id: TabId,
    pub title: String,
    pub url: String,
}

/// Same fields as the store's history hit; core does not depend on the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visit_count: u32,
    pub typed_count: u32,
    pub last_visit_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    NewSpace,
    CloseTab,
    ReopenClosedTab,
    Reload,
    ToggleSidebar,
    ToggleCompactMode,
}

pub struct Action {
    pub id: ActionId,
    pub title: &'static str,
    pub keywords: &'static str,
}

pub const ACTIONS: [Action; 6] = [
    Action {
        id: ActionId::NewSpace,
        title: "New Space",
        keywords: "create workspace",
    },
    Action {
        id: ActionId::CloseTab,
        title: "Close Tab",
        keywords: "remove",
    },
    Action {
        id: ActionId::ReopenClosedTab,
        title: "Reopen Closed Tab",
        keywords: "undo restore",
    },
    Action {
        id: ActionId::Reload,
        title: "Reload",
        keywords: "refresh",
    },
    Action {
        id: ActionId::ToggleSidebar,
        title: "Toggle Sidebar",
        keywords: "hide show tabs",
    },
    Action {
        id: ActionId::ToggleCompactMode,
        title: "Toggle Compact Mode",
        keywords: "hide show",
    },
];

#[derive(Debug, Default, Clone, Copy)]
pub struct Candidates<'a> {
    pub tabs: &'a [OpenTab],
    pub history: &'a [HistoryEntry],
    pub bookmarks: &'a [Bookmark],
    /// Remote suggestions, already fetched; shown after local matches.
    pub suggestions: &'a [String],
}

/// Declaration order is the tie-break order and mirrors Slint `CommandKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    OpenTab,
    History,
    Bookmark,
    Search,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    OpenTab(TabId),
    Url(String),
    Search(String),
    Action(ActionId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniResult {
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    pub target: Target,
    pub score: u32,
}

const CLASS_UNIT: u32 = 1_000_000;
const TAB_BONUS: u32 = 500_000;
const MAX_HISTORY_BONUS: u32 = TAB_BONUS - 1;
const DAY_MS: i64 = 86_400_000;

/// Ranks `cands` against `query`. The first row is always the classified
/// intent (a URL intent uses `Kind::Search` with `Target::Url`; the UI has no
/// separate "go" kind). An empty query yields no rows.
pub fn rank(
    query: &str,
    engine: SearchEngine,
    cands: &Candidates,
    now_ms: i64,
    limit: usize,
) -> Vec<OmniResult> {
    let intent = classify(query);
    let mut out = Vec::new();
    match &intent {
        Intent::Url(u) => out.push(row(
            Kind::Search,
            u,
            "Open",
            Target::Url(u.clone()),
            u32::MAX,
        )),
        Intent::Search(q) if q.is_empty() => return out,
        Intent::Search(q) => out.push(row(
            Kind::Search,
            q,
            &format!("Search {}", engine.info().name),
            Target::Search(q.clone()),
            u32::MAX,
        )),
    }
    let terms: Vec<String> = query
        .trim()
        .trim_start_matches('?')
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();

    let mut scored: Vec<OmniResult> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for t in cands.tabs {
        if let Some(class) = match_class(&terms, host_text(&t.url), &t.title, "") {
            seen.push(&t.url);
            let score = class * CLASS_UNIT + TAB_BONUS;
            scored.push(row(
                Kind::OpenTab,
                &t.title,
                &t.url,
                Target::OpenTab(t.id),
                score,
            ));
        }
    }
    for h in cands.history {
        if seen.contains(&h.url.as_str()) {
            continue;
        }
        if let Some(class) = match_class(&terms, host_text(&h.url), &h.title, "") {
            seen.push(&h.url);
            let score = class * CLASS_UNIT + history_bonus(h, now_ms);
            scored.push(row(
                Kind::History,
                &h.title,
                &h.url,
                Target::Url(h.url.clone()),
                score,
            ));
        }
    }
    for b in cands.bookmarks {
        if seen.contains(&b.url.as_str()) {
            continue;
        }
        if let Some(class) = match_class(&terms, host_text(&b.url), &b.title, "") {
            seen.push(&b.url);
            let score = class * CLASS_UNIT;
            scored.push(row(
                Kind::Bookmark,
                &b.title,
                &b.url,
                Target::Url(b.url.clone()),
                score,
            ));
        }
    }
    for a in &ACTIONS {
        if let Some(class) = match_class(&terms, "", a.title, a.keywords) {
            scored.push(row(
                Kind::Action,
                a.title,
                "",
                Target::Action(a.id),
                class * CLASS_UNIT,
            ));
        }
    }
    let own_query = match &intent {
        Intent::Search(q) => q.as_str(),
        Intent::Url(_) => "",
    };
    for s in cands.suggestions {
        if s != own_query {
            scored.push(row(Kind::Search, s, "", Target::Search(s.clone()), 0));
        }
    }
    // Stable sort: ties keep input order, then kind order.
    scored.sort_by(|a, b| b.score.cmp(&a.score).then(a.kind.cmp(&b.kind)));
    out.extend(scored);
    out.truncate(limit);
    out
}

fn row(kind: Kind, title: &str, subtitle: &str, target: Target, score: u32) -> OmniResult {
    OmniResult {
        kind,
        title: title.to_owned(),
        subtitle: subtitle.to_owned(),
        target,
        score,
    }
}

/// Host without scheme, userinfo and a leading `www.`, lowercased by the caller.
fn host_text(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
    let rest = rest.rsplit_once('@').map_or(rest, |(_, h)| h);
    rest.strip_prefix("www.").unwrap_or(rest)
}

/// Lowest per-term class (3 host prefix, 2 title-word prefix, 1 substring);
/// `None` when any term does not match or there are no terms.
fn match_class(terms: &[String], host: &str, title: &str, keywords: &str) -> Option<u32> {
    let host = host.to_lowercase();
    let title = title.to_lowercase();
    let keywords = keywords.to_lowercase();
    terms
        .iter()
        .map(|t| {
            if host.starts_with(t.as_str()) {
                Some(3)
            } else if title
                .split_whitespace()
                .chain(keywords.split_whitespace())
                .any(|w| w.starts_with(t.as_str()))
            {
                Some(2)
            } else if host.contains(t.as_str()) || title.contains(t.as_str()) {
                Some(1)
            } else {
                None
            }
        })
        .min()
        .flatten()
}

/// Typed visits count double; age buckets are Firefox Places' 100/70/50/30/10.
fn history_bonus(h: &HistoryEntry, now_ms: i64) -> u32 {
    let days = (now_ms - h.last_visit_ms).max(0) / DAY_MS;
    let weight = match days {
        0..=3 => 100,
        4..=13 => 70,
        14..=30 => 50,
        31..=89 => 30,
        _ => 10,
    };
    let points = u64::from(h.typed_count) * 2 + u64::from(h.visit_count);
    (points * weight).min(u64::from(MAX_HISTORY_BONUS)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Intent {
        Intent::Url(s.into())
    }
    fn search(s: &str) -> Intent {
        Intent::Search(s.into())
    }

    #[test]
    fn classify_table() {
        let cases = [
            ("example.com", url("https://example.com/")),
            ("  example.com  ", url("https://example.com/")),
            ("http://example.com", url("http://example.com/")),
            ("https://a.b/c?d#e", url("https://a.b/c?d#e")),
            ("foo bar", search("foo bar")),
            ("foo.bar", url("https://foo.bar/")),
            ("foo", search("foo")),
            ("localhost", url("http://localhost/")),
            ("localhost:3000", url("http://localhost:3000/")),
            ("localhost/", url("http://localhost/")),
            ("192.168.0.1", url("http://192.168.0.1/")),
            ("192.168.0.1:8080", url("http://192.168.0.1:8080/")),
            ("13.5", search("13.5")),
            ("2.5", search("2.5")),
            ("0.5.5.5", search("0.5.5.5")),
            ("what is 2.5", search("what is 2.5")),
            ("[::1]", url("http://[::1]/")),
            ("user@host", search("user@host")),
            ("user@host.com", search("user@host.com")),
            ("user:pw@host.com", url("https://user:pw@host.com/")),
            ("c++", search("c++")),
            ("about:blank", url("about:blank")),
            ("file:///tmp/x", url("file:///tmp/x")),
            ("javascript:alert(1)", search("javascript:alert(1)")),
            ("foo:bar", search("foo:bar")),
            ("?rust", search("rust")),
            ("? a.com", search("a.com")),
            ("b\u{fc}cher.de", url("https://xn--bcher-kva.de/")),
            ("example.com.", url("https://example.com./")),
            ("foo.", search("foo.")),
            ("example.com/a b", search("example.com/a b")),
            ("foo/", url("https://foo/")),
            ("foo/bar", search("foo/bar")),
            ("foo/bar?x", url("https://foo/bar?x")),
            ("", search("")),
            ("   ", search("")),
            ("a.test", search("a.test")),
            ("www.a.test", url("https://www.a.test/")),
            ("http://", search("http://")),
        ];
        for (input, want) in cases {
            assert_eq!(classify(input), want, "{input:?}");
        }
    }

    #[test]
    fn engines_table() {
        for e in &ENGINES {
            assert_eq!(e.search.matches("{q}").count(), 1, "{}", e.name);
            assert_eq!(e.suggest.matches("{q}").count(), 1, "{}", e.name);
            assert_eq!(e.engine.info(), e);
        }
        assert_eq!(
            search_url(SearchEngine::DuckDuckGo, "c++ & rust"),
            "https://duckduckgo.com/?q=c%2B%2B%20%26%20rust"
        );
        assert_eq!(
            suggest_url(SearchEngine::Google, "ru"),
            "https://suggestqueries.google.com/complete/search?client=firefox&q=ru"
        );
        assert_eq!(
            suggest_url(SearchEngine::DuckDuckGo, "ru"),
            "https://duckduckgo.com/ac/?type=list&q=ru"
        );
    }

    const NOW: i64 = 1_000 * DAY_MS;

    fn tab(id: u64, title: &str, url: &str) -> OpenTab {
        OpenTab {
            id: TabId(id),
            title: title.into(),
            url: url.into(),
        }
    }
    fn hist(url: &str, title: &str, visits: u32, typed: u32, days_ago: i64) -> HistoryEntry {
        HistoryEntry {
            url: url.into(),
            title: title.into(),
            visit_count: visits,
            typed_count: typed,
            last_visit_ms: NOW - days_ago * DAY_MS,
        }
    }
    fn run(q: &str, c: &Candidates) -> Vec<OmniResult> {
        rank(q, SearchEngine::DuckDuckGo, c, NOW, 50)
    }
    fn titles(r: &[OmniResult]) -> Vec<&str> {
        r.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn match_classes_order() {
        let history = [
            hist("https://x.org/a", "about rustc", 1, 0, 0),
            hist("https://x.org/b", "trust issues", 1, 0, 0),
            hist("https://rust-lang.org/", "Home", 1, 0, 0),
        ];
        let r = run(
            "rust",
            &Candidates {
                history: &history,
                ..Default::default()
            },
        );
        assert_eq!(titles(&r), ["rust", "Home", "about rustc", "trust issues"]);
    }

    #[test]
    fn tab_beats_equal_class_history_and_dedupes() {
        let tabs = [tab(1, "Rust", "https://rust-lang.org/")];
        let history = [
            hist("https://rust-lang.org/", "Rust", 99, 9, 0),
            hist("https://rustup.rs/", "rustup", 99, 9, 0),
        ];
        let r = run(
            "rust",
            &Candidates {
                tabs: &tabs,
                history: &history,
                ..Default::default()
            },
        );
        assert_eq!(r[1].target, Target::OpenTab(TabId(1)));
        assert_eq!(r[1].kind, Kind::OpenTab);
        assert_eq!(r[2].target, Target::Url("https://rustup.rs/".into()));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn frequency_and_recency_win_within_a_class() {
        let history = [
            hist("https://a.com/old", "a old", 50, 0, 200),
            hist("https://a.com/few", "a few", 1, 0, 0),
            hist("https://a.com/many", "a many", 10, 0, 0),
            hist("https://a.com/typed", "a typed", 4, 5, 0),
        ];
        let r = run(
            "a.com",
            &Candidates {
                history: &history,
                ..Default::default()
            },
        );
        assert_eq!(
            titles(&r)[1..],
            ["a typed", "a many", "a old", "a few"].map(String::from)[..]
        );
    }

    #[test]
    fn terms_all_match_and_non_matches_drop() {
        let history = [
            hist("https://github.com/x", "Slint repo", 1, 0, 0),
            hist("https://github.com/y", "Other", 1, 0, 0),
        ];
        let c = Candidates {
            history: &history,
            ..Default::default()
        };
        assert_eq!(
            titles(&run("github slint", &c)),
            ["github slint", "Slint repo"]
        );
        assert_eq!(titles(&run("zzz", &c)), ["zzz"]);
    }

    #[test]
    fn intent_row_first_and_mapped() {
        let r = run("example.com", &Candidates::default());
        assert_eq!(r[0].kind, Kind::Search);
        assert_eq!(r[0].target, Target::Url("https://example.com/".into()));
        let r = run("hello world", &Candidates::default());
        assert_eq!(r[0].target, Target::Search("hello world".into()));
        assert!(run("  ", &Candidates::default()).is_empty());
    }

    #[test]
    fn actions_match_by_word_prefix() {
        let r = run("new sp", &Candidates::default());
        assert_eq!(r[1].target, Target::Action(ActionId::NewSpace));
        assert_eq!(r.len(), 2);
        let r = run("refresh", &Candidates::default());
        assert_eq!(r[1].target, Target::Action(ActionId::Reload));
    }

    #[test]
    fn suggestions_come_last_and_skip_own_query() {
        let history = [hist("https://rust-lang.org/", "Rust", 1, 0, 0)];
        let sugg = ["rust".to_owned(), "rust book".to_owned()];
        let c = Candidates {
            history: &history,
            suggestions: &sugg,
            ..Default::default()
        };
        assert_eq!(titles(&run("rust", &c)), ["rust", "Rust", "rust book"]);
    }

    #[test]
    fn deterministic_stable_and_limited() {
        let history = [
            hist("https://a.com/1", "same", 1, 0, 0),
            hist("https://a.com/2", "same", 1, 0, 0),
            hist("https://a.com/3", "same", 1, 0, 0),
        ];
        let c = Candidates {
            history: &history,
            ..Default::default()
        };
        let first = run("same", &c);
        assert_eq!(first, run("same", &c));
        let urls: Vec<_> = first[1..].iter().map(|r| r.subtitle.as_str()).collect();
        assert_eq!(
            urls,
            ["https://a.com/1", "https://a.com/2", "https://a.com/3"]
        );
        assert_eq!(rank("same", SearchEngine::Google, &c, NOW, 2).len(), 2);
    }
}
