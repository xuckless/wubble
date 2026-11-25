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

use crate::action::{Action, PALETTE};
use crate::answer::{self, AnswerKind};
use crate::search::Engines;
use crate::tab::TabId;
use crate::url::percent_encode_into;

// ---------------------------------------------------------------- engines

/// Substitutes the percent-encoded `query` for `{q}` in `template`.
pub fn fill(template: &str, query: &str) -> String {
    let (head, tail) = template.split_once("{q}").unwrap_or((template, ""));
    let mut out = String::from(head);
    percent_encode_into(query, &mut out);
    out.push_str(tail);
    out
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
/// Schemes accepted when typed as `scheme://...`; `wubble` names the internal pages
/// (`pages::InternalPage`).
const HIER_SCHEMES: [&str; 5] = ["http", "https", "ftp", "file", "wubble"];
/// Reserved TLDs that only count as a host with a subdomain (Chromium). `.local` (mDNS: a Mac's
/// own `name.local`) and `.internal` name real machines with two labels, so they navigate.
const RESERVED_TLDS: [&str; 2] = ["example", "test"];

/// Looks like a file name, not a TLD, when typed after a dot.
const FILE_EXTENSIONS: [&str; 32] = [
    "txt", "md", "rs", "py", "js", "ts", "tsx", "jsx", "json", "toml", "yaml", "yml", "xml",
    "html", "htm", "css", "csv", "log", "lock", "sh", "rb", "pl", "cs", "so", "exe", "zip", "pdf",
    "png", "jpg", "jpeg", "gif", "svg",
];

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

    // Judge the typed port, not `url.port()`: the url crate drops `:80`.
    let typed_port = hostport
        .rsplit_once(':')
        .map(|(_, p)| p)
        .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));

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
        || typed_port.is_some()
        || rest.ends_with('/')
        || non_host_components(&url, typed_port.is_some()) > 1
        || matches!(host_of(&url), Some(d) if has_known_tld(d));
    if !is_url {
        return None;
    }
    // A scheme-less public name gets https (there is no fallback to http). Local and
    // non-unique hosts (IP literals, localhost, single-label and reserved names such as
    // `nas.local` or `app.test`) and any typed port but `:443` stay http, as Chromium leaves
    // them out of its typed-navigation HTTPS upgrade: those are dev servers and LAN boxes,
    // which rarely speak TLS on that port.
    let public = matches!(host_of(&url), Some(d) if is_public_name(d));
    if public && typed_port.is_none_or(|p| p == "443") {
        url.set_scheme("https").ok()?;
    }
    Some(url.into())
}

/// TLDs that never name a public site: RFC 6761/6762 reserved names, `.internal` (ICANN),
/// `.home.arpa` (RFC 8375) and the usual home-router ones. Chromium's
/// `IsHostnameNonUnique` asks the registry; this list covers what dev servers and LANs use.
const NON_UNIQUE_TLDS: [&str; 10] = [
    "localhost",
    "local",
    "test",
    "example",
    "invalid",
    "internal",
    "arpa",
    "lan",
    "home",
    "intranet",
];

/// A dotted name under a public-looking TLD (not one of [`NON_UNIQUE_TLDS`]).
fn is_public_name(domain: &str) -> bool {
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    domain
        .rsplit_once('.')
        .is_some_and(|(_, tld)| !NON_UNIQUE_TLDS.contains(&tld))
}

fn host_of(url: &Url) -> Option<&str> {
    match url.host()? {
        Host::Domain(d) => Some(d),
        _ => None,
    }
}

/// Port, path, query, fragment and userinfo each count once.
fn non_host_components(url: &Url, has_port: bool) -> usize {
    [
        has_port,
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
/// the `psl` crate if exactness ever matters. Common file extensions are
/// refused even where they are real ccTLDs (`.rs`, `.md`, `.py`), so file
/// names and identifiers search instead of navigating.
fn has_known_tld(domain: &str) -> bool {
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    let Some((_, tld)) = domain.rsplit_once('.') else {
        return false;
    };
    let labels = domain.split('.').count();
    if RESERVED_TLDS.contains(&tld) {
        return labels > 2;
    }
    if FILE_EXTENSIONS.contains(&tld) {
        return false;
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

#[derive(Debug, Default, Clone, Copy)]
pub struct Candidates<'a> {
    pub tabs: &'a [OpenTab],
    pub history: &'a [HistoryEntry],
    pub bookmarks: &'a [Bookmark],
    /// Remote suggestions, already fetched; shown after local matches.
    pub suggestions: &'a [String],
    /// No inline completion for this query: the edit deleted text ([`just_deleted`]), or the
    /// field cannot show one.
    pub prevent_inline: bool,
}

/// Declaration order is the tie-break order and mirrors Slint `CommandKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    OpenTab,
    History,
    Bookmark,
    Search,
    Action,
    /// A recently closed tab (tab search only, `tab_search`).
    ClosedTab,
    /// A calculator or unit conversion result, computed on the device (`answer`).
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    OpenTab(TabId),
    Url(String),
    Search(String),
    Action(Action),
    /// An entry of the recently-closed list (tab search only): its URL and close time name it,
    /// as they name it to the store and to `closed`.
    ClosedTab {
        url: String,
        closed_at_ms: i64,
    },
    /// Put this text on the clipboard (an answer row's result).
    Copy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniResult {
    pub kind: Kind,
    pub title: String,
    pub subtitle: String,
    pub target: Target,
}

const CLASS_UNIT: u32 = 1_000_000;
const TAB_BONUS: u32 = 500_000;
const MAX_HISTORY_BONUS: u32 = TAB_BONUS - 1;
const DAY_MS: i64 = 86_400_000;

/// Ranks `cands` against `query`. The first row is the inline completion's page when there is
/// one ([`inline_match`]: the field shows it, so Enter opens it), then the classified intent (a
/// URL intent uses `Kind::Search` with `Target::Url`; the UI has no separate "go" kind), except
/// that without a completion an action whose title the query's words name comes before a search
/// intent. A local answer (`answer::answer`) to a searched query comes first when it is a
/// calculation, second (under the intent) when it is a unit conversion. An empty query yields
/// no rows.
///
/// A query that starts with an engine's keyword and a space (`wikipedia.org rust`) searches
/// that engine, as Chrome's keyword mode: its rows are that engine's results page
/// (`Target::Url`) for the rest of the query, and the suggestions are that engine's.
pub fn rank(
    query: &str,
    engines: &Engines,
    cands: &Candidates,
    now_ms: i64,
    limit: usize,
) -> Vec<OmniResult> {
    let inline = if cands.prevent_inline {
        None
    } else {
        inline_match(query, cands.history, cands.bookmarks)
    };
    let keyword = engines.keyword_search(query);
    let query = keyword.map_or(query, |(_, rest)| rest);
    let intent = match keyword {
        Some(_) => Intent::Search(query.trim().to_owned()),
        None => classify(query),
    };
    let searched = |q: &str| match keyword {
        Some((engine, _)) => Target::Url(engine.search_url(q)),
        None => Target::Search(q.to_owned()),
    };
    let name = keyword
        .map(|(engine, _)| engine.name.as_str())
        .or(engines.default.as_ref().map(|e| e.name.as_str()))
        .unwrap_or("the Web");
    let mut out = Vec::new();
    if let Some(m) = &inline {
        out.push(row(Kind::Search, &m.url, "Open", Target::Url(m.url.clone())));
    }
    match &intent {
        Intent::Url(u) if inline.as_ref().is_some_and(|m| &m.url == u) => {}
        Intent::Url(u) => out.push(row(Kind::Search, u, "Open", Target::Url(u.clone()))),
        Intent::Search(q) if q.is_empty() => return out,
        Intent::Search(q) => out.push(row(
            Kind::Search,
            q,
            &format!("Search {name}"),
            searched(q.as_str()),
        )),
    }
    let terms = query_terms(query);

    let mut scored: Vec<(u32, OmniResult)> = Vec::new();
    // the completed page is the first row already
    let mut seen: Vec<&str> = inline.iter().map(|m| m.url.as_str()).collect();
    let sources = cands
        .tabs
        .iter()
        .map(|t| {
            (
                Kind::OpenTab,
                &t.title,
                &t.url,
                Target::OpenTab(t.id),
                TAB_BONUS,
            )
        })
        .chain(cands.history.iter().map(|h| {
            let target = Target::Url(h.url.clone());
            (
                Kind::History,
                &h.title,
                &h.url,
                target,
                history_bonus(h, now_ms),
            )
        }))
        .chain(cands.bookmarks.iter().map(|b| {
            (
                Kind::Bookmark,
                &b.title,
                &b.url,
                Target::Url(b.url.clone()),
                0,
            )
        }));
    for (kind, title, url, target, bonus) in sources {
        if kind != Kind::OpenTab && seen.contains(&url.as_str()) {
            continue;
        }
        if let Some(class) = match_class(&terms, site_text(url), title, "") {
            seen.push(url);
            scored.push((class * CLASS_UNIT + bonus, row(kind, title, url, target)));
        }
    }
    for a in &PALETTE {
        if let Some(class) = match_class(&terms, "", a.title, a.keywords) {
            scored.push((
                class * CLASS_UNIT,
                row(Kind::Action, a.title, "", Target::Action(a.action)),
            ));
        }
    }
    let mut shown: Vec<String> = vec![match &intent {
        Intent::Search(q) => q.to_lowercase(),
        Intent::Url(_) => String::new(),
    }];
    for s in cands.suggestions {
        let lower = s.to_lowercase();
        if !shown.contains(&lower) {
            shown.push(lower);
            scored.push((0, row(Kind::Search, s, "", searched(s.as_str()))));
        }
    }
    // Stable sort: ties keep input order, then kind order.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.kind.cmp(&b.1.kind)));
    out.extend(scored.into_iter().map(|(_, r)| r));
    // An action named by the query's words runs on Enter: "new boost" is the action, not a web
    // search (QA HIGH-07). A URL, or words naming no action, keep the intent row first, and so
    // do a completion (Enter opens what the field shows) and a keyword search.
    if inline.is_none()
        && keyword.is_none()
        && matches!(intent, Intent::Search(_))
        && let Some(i) = out
            .iter()
            .position(|r| r.kind == Kind::Action && names(&terms, &r.title))
    {
        let action = out.remove(i);
        out.insert(0, action);
    }
    if let Intent::Search(q) = &intent
        && keyword.is_none()
        && let Some(a) = answer::answer(q)
    {
        let at = match a.kind {
            AnswerKind::Math => 0,
            AnswerKind::Unit => 1.min(out.len()),
        };
        out.insert(at, row(Kind::Answer, &a.title, "Copy", Target::Copy(a.copy)));
    }
    out.truncate(limit);
    out
}

/// The query's words as the matcher takes them: lower case, without a scheme or a leading
/// `www.`.
fn query_terms(query: &str) -> Vec<String> {
    query
        .trim()
        .trim_start_matches('?')
        .split_whitespace()
        .map(|t| {
            let t = t.to_lowercase();
            let t = t.split_once("://").map_or(t.as_str(), |(_, r)| r);
            t.strip_prefix("www.").unwrap_or(t).to_owned()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

// ------------------------------------------------------------- tab search

/// An open tab as tab search (Mod+Shift+A) lists it. The caller leaves out what the list must
/// not show (an agent's tabs, a peek's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTab {
    pub id: TabId,
    pub title: String,
    pub url: String,
    /// Its Space's name, shown on the row and matched as the row's group (Chrome matches a
    /// tab's group title).
    pub space: String,
    /// It is in the Space the focused window shows: those rows come first.
    pub active_space: bool,
    /// When it was last on screen, unix ms; `i64::MAX` while it is on screen.
    pub last_active_ms: i64,
}

/// A recently closed tab as tab search lists it (the store's list, newest first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchClosed {
    pub title: String,
    pub url: String,
    pub closed_at_ms: i64,
}

/// What the closed rows say beside their site (Chrome's Tab Search section title).
pub const RECENTLY_CLOSED: &str = "Recently closed";

/// Tab search's rows: every open tab, the active Space's first and then the most recently on
/// screen (Chrome's Tab Search orders by last active time), each named with its Space and site;
/// then the recently closed tabs, newest first. A query keeps the rows the command bar's
/// matcher matches (every word on the site, the title or the Space's name), best class first and
/// in that order within a class, open tabs still before closed ones. Pure; `limit` caps the rows.
///
/// Chromium `chrome/browser/resources/tab_search/` (BSD-3-Clause) is the behaviour followed: one
/// list of open tabs by recency, then a "Recently closed" section; no code copied, and the
/// matching is the omnibox's own rather than Chromium's fuse.js scoring.
pub fn tab_search(
    query: &str,
    tabs: &[SearchTab],
    closed: &[SearchClosed],
    limit: usize,
) -> Vec<OmniResult> {
    let terms = query_terms(query);
    let mut open: Vec<&SearchTab> = tabs.iter().collect();
    // stable: equal keys keep the caller's (sidebar) order
    open.sort_by_key(|t| (!t.active_space, std::cmp::Reverse(t.last_active_ms)));
    let open = open.into_iter().map(|t| {
        let site = host_text(&t.url);
        let subtitle = if t.space.is_empty() {
            site.to_owned()
        } else {
            format!("{} \u{b7} {site}", t.space)
        };
        let class = match_class(&terms, site_text(&t.url), &t.title, &t.space);
        (
            class,
            row(Kind::OpenTab, &t.title, &subtitle, Target::OpenTab(t.id)),
        )
    });
    let closed = closed.iter().map(|c| {
        let site = host_text(&c.url);
        let title = if c.title.is_empty() {
            site
        } else {
            c.title.as_str()
        };
        let class = match_class(&terms, site_text(&c.url), title, "");
        let target = Target::ClosedTab {
            url: c.url.clone(),
            closed_at_ms: c.closed_at_ms,
        };
        let subtitle = format!("{RECENTLY_CLOSED} \u{b7} {site}");
        (class, row(Kind::ClosedTab, title, &subtitle, target))
    });
    let filter = |rows: Vec<(Option<u32>, OmniResult)>| -> Vec<OmniResult> {
        if terms.is_empty() {
            return rows.into_iter().map(|(_, r)| r).collect();
        }
        let mut hits: Vec<(u32, OmniResult)> = rows
            .into_iter()
            .filter_map(|(class, r)| Some((class?, r)))
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        hits.into_iter().map(|(_, r)| r).collect()
    };
    let mut out = filter(open.collect());
    out.extend(filter(closed.collect()));
    out.truncate(limit);
    out
}

/// The host of `url`, without scheme, userinfo and a leading `www.`.
fn host_text(url: &str) -> &str {
    let site = site_text(url);
    site.split(['/', '?', '#']).next().unwrap_or(site)
}

/// Every term starts a word of `title` (case-insensitive): the query names it.
fn names(terms: &[String], title: &str) -> bool {
    let title = title.to_lowercase();
    !terms.is_empty()
        && terms
            .iter()
            .all(|t| title.split_whitespace().any(|w| w.starts_with(t.as_str())))
}

fn row(kind: Kind, title: &str, subtitle: &str, target: Target) -> OmniResult {
    OmniResult {
        kind,
        title: title.to_owned(),
        subtitle: subtitle.to_owned(),
        target,
    }
}

/// URL without scheme, userinfo and a leading `www.`: `host/path?query`.
fn site_text(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let rest = match rest[..end].rfind('@') {
        Some(at) => &rest[at + 1..],
        None => rest,
    };
    rest.strip_prefix("www.").unwrap_or(rest)
}

/// Lowest per-term class (3 host prefix, 2 title-word prefix, 1 substring);
/// `None` when any term does not match or there are no terms.
fn match_class(terms: &[String], site: &str, title: &str, keywords: &str) -> Option<u32> {
    let site = site.to_lowercase();
    let host = site.split(['/', '?', '#']).next().unwrap_or("");
    let title = title.to_lowercase();
    let keywords = keywords.to_lowercase();
    terms
        .iter()
        .map(|t| {
            if t.contains('/') {
                // A term with a path matches against host + path.
                if site.starts_with(t.as_str()) {
                    Some(3)
                } else if site.contains(t.as_str()) {
                    Some(1)
                } else {
                    None
                }
            } else if host.starts_with(t.as_str()) {
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

// ------------------------------------------------------------- Enter keys

/// Where Enter in the command bar opens what it chose: Chromium's `WindowOpenDisposition` for
/// the omnibox (`OmniboxEditModel::AcceptInput`, `ui::DispositionFromEventFlags`), BSD-3-Clause;
/// behaviour only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// As the bar was opened for: this tab (Ctrl+L) or a new one (Ctrl+T).
    AsOpened,
    /// Alt+Enter (Cmd+Enter on macOS): a new tab, shown.
    NewTab,
    /// Shift+Enter: a new window.
    NewWindow,
}

/// The modifiers held when the bar's row was chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnterMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// What Enter does on `platform` with `mods` held. Ctrl alone never changes the disposition:
/// it is the `www.`/`.com` completion ([`with_desired_tld`]).
pub fn disposition(mods: EnterMods, platform: crate::keys::Platform) -> Disposition {
    let new_tab = match platform {
        crate::keys::Platform::Mac => mods.cmd || mods.alt,
        _ => mods.alt,
    };
    if mods.shift {
        Disposition::NewWindow
    } else if new_tab {
        Disposition::NewTab
    } else {
        Disposition::AsOpened
    }
}

/// Ctrl+Enter: `www.` and `.com` around a bare name (`example` opens `https://www.example.com/`),
/// Chromium's desired TLD (`AutocompleteInput` with `desired_tld` "com", `url_formatter::FixupURL`
/// adding the TLD to a host without one and `www.` before it). `None` when the text is not one
/// bare host name, which then opens as typed.
pub fn with_desired_tld(query: &str) -> Option<String> {
    let s = query.trim();
    let end = s.find(['/', '?', '#']).unwrap_or(s.len());
    let (host, rest) = s.split_at(end);
    let bare = !host.is_empty()
        && !host.contains('.')
        && host.chars().all(|c| c.is_alphanumeric() || c == '-')
        && !host.starts_with('-');
    if !bare {
        return None;
    }
    let url = Url::parse(&format!("https://www.{host}.com{rest}")).ok()?;
    url.host()?;
    Some(url.into())
}

/// Ctrl+E (Chromium's `IDC_FOCUS_SEARCH`): what the bar ranks and opens for `typed`, a search
/// whatever it looks like, as the omnibox's `?` prefix makes it ([`classify`]).
pub fn as_search(typed: &str) -> String {
    if typed.trim().is_empty() || typed.trim_start().starts_with('?') {
        typed.to_owned()
    } else {
        format!("?{typed}")
    }
}

// ----------------------------------------------------------------- inline
//
// Inline autocomplete re-expresses the rules of Chromium's HistoryURLProvider
// (`CanPromoteMatchForInlineAutocomplete`, `PromoteOrCreateShorterSuggestion`, the URL prefixes
// of `url_prefix.cc`) and the deletion test of `OmniboxViewViews::OnAfterPossibleChange`
// (components/omnibox/browser, chrome/browser/ui/views/omnibox; BSD-3-Clause; behaviour only,
// no code copied).

/// Typed visits a page with a path needs to complete inline; a bare host needs one
/// (`CanPromoteMatchForInlineAutocomplete`: "n == 1 for host-only URLs and n == 2 for others").
const INLINE_TYPED_DEEP: u32 = 2;
/// Visits that let a never-typed page complete inline: a stand-in for HistoryQuickProvider's
/// inline score bar, which only heavily visited pages reach.
const INLINE_VISITS: u32 = 10;

/// The completion the field shows after what was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineMatch {
    /// The text added after the query, shown selected.
    pub suffix: String,
    /// The page Enter opens: the history row's own URL, or its host's root.
    pub url: String,
}

/// The suffix the field completes `query` with (type `git`, get `hub.com/`), or `None`. Only
/// a history page whose URL (with or without its scheme and `www.`) starts with the query and
/// that was typed (once for a bare host, twice for a deeper page) or visited
/// [`INLINE_VISITS`] times, or a bookmark, is offered. Never for a query with whitespace in it
/// (a trailing space included), a forced search (`?`), or one the user just shortened: the
/// caller passes that as [`Candidates::prevent_inline`] to [`rank`] ([`just_deleted`]).
pub fn inline_completion(
    query: &str,
    history: &[HistoryEntry],
    bookmarks: &[Bookmark],
) -> Option<String> {
    inline_match(query, history, bookmarks).map(|m| m.suffix)
}

/// [`inline_completion`] with the page it opens. A query that ends inside the host of a deeper
/// page completes to that host's root (`git` gives `github.com/`, not `github.com/a/b`), as
/// HistoryURLProvider promotes the shorter suggestion. Among several, typed pages win, then
/// bare hosts, then more typed visits, more visits, the newest, the shortest; then input order.
pub fn inline_match(
    query: &str,
    history: &[HistoryEntry],
    bookmarks: &[Bookmark],
) -> Option<InlineMatch> {
    if query.is_empty() || query.starts_with('?') || query.contains(char::is_whitespace) {
        return None;
    }
    let pages = history
        .iter()
        .map(|h| (h.url.as_str(), h.typed_count, h.visit_count, h.last_visit_ms, false))
        .chain(
            bookmarks
                .iter()
                .map(|b| (b.url.as_str(), 0, 0, i64::MIN, true)),
        );
    let mut best: Option<((bool, bool, u32, u32, i64, std::cmp::Reverse<usize>), InlineMatch)> =
        None;
    for (url, typed, visits, last_visit, bookmarked) in pages {
        let Some((m, host_only)) = complete_inline(query, url) else {
            continue;
        };
        let qualifies = bookmarked
            || visits >= INLINE_VISITS
            || (typed >= 1 && (host_only || typed >= INLINE_TYPED_DEEP));
        if !qualifies {
            continue;
        }
        let key = (
            typed > 0 || bookmarked,
            host_only,
            typed,
            visits,
            last_visit,
            std::cmp::Reverse(m.suffix.len()),
        );
        if best.as_ref().is_none_or(|(k, _)| key > *k) {
            best = Some((key, m));
        }
    }
    best.map(|(_, m)| m)
}

/// `url` completing `query`: the match and whether it is a bare host. The query is matched,
/// ASCII case-insensitively, against the URL, the URL without its scheme, and without its
/// scheme and `www.` (Chromium's URL prefixes); http and https only.
fn complete_inline(query: &str, url: &str) -> Option<(InlineMatch, bool)> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host_end = scheme.len() + 3 + authority.len();
    let path = &url[host_end..];
    let bare_host = path.is_empty() || path == "/";
    let after_scheme = scheme.len() + 3;
    let after_www = after_scheme
        + if rest.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("www.")) {
            4
        } else {
            0
        };
    let n = query.len();
    for start in [0, after_scheme, after_www] {
        let form = &url[start..];
        if form.len() <= n || !form.is_char_boundary(n) || !form[..n].eq_ignore_ascii_case(query)
        {
            continue;
        }
        let typed_end = start + n;
        if typed_end <= host_end && !bare_host {
            // The query ends inside the host: complete to its root.
            let root = format!("{}/", &url[..host_end]);
            let suffix = root[typed_end..].to_owned();
            return Some((InlineMatch { suffix, url: root }, true));
        }
        let m = InlineMatch {
            suffix: form[n..].to_owned(),
            url: url.to_owned(),
        };
        return Some((m, bare_host));
    }
    None
}

/// Whether the edit from `before` to `after` only deleted text: `after` is `before` with one
/// run cut out, or the same text (a selected completion was dropped). Chromium's
/// `just_deleted_text`, from the texts alone: no inline completion follows such an edit.
/// Typing over a selection, or replacing the whole text, is not a deletion.
pub fn just_deleted(before: &str, after: &str) -> bool {
    if after.len() > before.len() {
        return false;
    }
    let mut head = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !after.is_char_boundary(head) {
        head -= 1;
    }
    let tail = before[head..]
        .bytes()
        .rev()
        .zip(after[head..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    head + tail >= after.len()
}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::engine_info;

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
            ("wubble://settings", url("wubble://settings")),
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
            ("foo/", url("http://foo/")),
            ("foo/bar", search("foo/bar")),
            ("foo/bar?x", url("http://foo/bar?x")),
            ("", search("")),
            ("   ", search("")),
            ("a.test", search("a.test")),
            ("www.a.test", url("http://www.a.test/")),
            ("http://", search("http://")),
            ("example.com:80", url("http://example.com/")),
            ("foo:80", url("http://foo/")),
            ("example.com:443", url("https://example.com/")),
            ("file.txt", search("file.txt")),
            ("main.rs", search("main.rs")),
            ("README.md", search("README.md")),
            ("package.json", search("package.json")),
            ("main.rs/", url("https://main.rs/")),
            // Local servers: dev servers and LAN boxes stay http (no TLS fallback exists).
            ("localhost:5173/app", url("http://localhost:5173/app")),
            ("127.0.0.1:8080", url("http://127.0.0.1:8080/")),
            ("[::1]:3000", url("http://[::1]:3000/")),
            ("app.localhost:3000", url("http://app.localhost:3000/")),
            ("app.localhost", url("http://app.localhost/")),
            ("nas.local", url("http://nas.local/")),
            ("my-mac.local:3000", url("http://my-mac.local:3000/")),
            ("api.dev.test:8443", url("http://api.dev.test:8443/")),
            ("router.lan", url("http://router.lan/")),
            ("printer.home.arpa", url("http://printer.home.arpa/")),
            ("devbox:3000", url("http://devbox:3000/")),
            ("devbox:3000/x", url("http://devbox:3000/x")),
            ("devbox", search("devbox")),
            ("intranet/", url("http://intranet/")),
            ("192.168.1.20:8000", url("http://192.168.1.20:8000/")),
            ("10.0.0.5", url("http://10.0.0.5/")),
            ("[fe80::1]:8080", url("http://[fe80::1]:8080/")),
            // A public name with a typed port: http unless it is 443.
            ("example.com:8080", url("http://example.com:8080/")),
            ("example.com:8443", url("http://example.com:8443/")),
        ];
        for (input, want) in cases {
            assert_eq!(classify(input), want, "{input:?}");
        }
    }

    #[test]
    fn fill_percent_encodes_the_query() {
        assert_eq!(
            fill(engine_info("duckduckgo").unwrap().search, "c++ & rust"),
            "https://duckduckgo.com/?q=c%2B%2B%20%26%20rust"
        );
        assert_eq!(
            fill(engine_info("google").unwrap().suggest.unwrap(), "ru"),
            "https://suggestqueries.google.com/complete/search?client=firefox&q=ru"
        );
    }

    #[test]
    fn a_keyword_searches_its_engine() {
        let engines = Engines::resolve("duckduckgo", &[], &[]);
        let r = rank(
            "wikipedia.org rust lang",
            &engines,
            &Candidates::default(),
            NOW,
            50,
        );
        assert_eq!(r[0].title, "rust lang");
        assert_eq!(r[0].subtitle, "Search Wikipedia");
        assert_eq!(
            r[0].target,
            Target::Url("https://en.wikipedia.org/wiki/Special:Search?search=rust%20lang".into())
        );
        // the keyword alone is an address; without a known keyword the default searches
        let r = rank("nosuch.example rust", &engines, &Candidates::default(), NOW, 50);
        assert_eq!(r[0].target, Target::Search("nosuch.example rust".into()));
        // suggestions in keyword mode are the keyword engine's results
        let sugg = ["rust book".to_owned()];
        let c = Candidates {
            suggestions: &sugg,
            ..Default::default()
        };
        let r = rank("bing.com rust", &engines, &c, NOW, 50);
        assert_eq!(
            r.last().unwrap().target,
            Target::Url("https://www.bing.com/search?q=rust%20book".into())
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
        rank(q, &Engines::resolve("duckduckgo", &[], &[]), c, NOW, 50)
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
                prevent_inline: true,
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
    fn url_shaped_query_finds_site_rows() {
        let tabs = [tab(1, "GH", "https://www.github.com/x")];
        let history = [hist("https://github.com/x/y", "Repo", 1, 0, 0)];
        let c = Candidates {
            tabs: &tabs,
            history: &history,
            ..Default::default()
        };
        for q in ["https://github.com", "github.com/x", "www.github.com/x"] {
            let r = run(q, &c);
            assert_eq!(r.len(), 3, "{q}");
            assert_eq!(r[1].kind, Kind::OpenTab, "{q}");
        }
        assert_eq!(run("github.com/zzz", &c).len(), 1);
        assert_eq!(run("http://", &c).len(), 1);
    }

    #[test]
    fn suggestions_dedupe_case_insensitively() {
        let sugg = ["Rust", "rust book", "rust", "Rust Book"].map(String::from);
        let c = Candidates {
            suggestions: &sugg,
            ..Default::default()
        };
        assert_eq!(titles(&run("rust", &c)), ["rust", "rust book"]);
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
        assert_eq!(r[0].target, Target::Action(Action::NewSpace));
        assert_eq!(r[1].target, Target::Search("new sp".into()));
        assert_eq!(r.len(), 2);
        // A keyword finds the action, below the search: the query does not name it.
        let r = run("refresh", &Candidates::default());
        assert_eq!(r[0].target, Target::Search("refresh".into()));
        assert_eq!(r[1].target, Target::Action(Action::Reload));
    }

    /// QA HIGH-07: the two reported queries.
    #[test]
    fn an_action_the_query_names_outranks_the_search() {
        let r = run("new boost", &Candidates::default());
        assert_eq!(r[0].target, Target::Action(Action::NewBoost));
        assert_eq!(r[1].target, Target::Search("new boost".into()));
        let r = run("New BOO", &Candidates::default());
        assert_eq!(r[0].target, Target::Action(Action::NewBoost));
        let r = run("page theme", &Candidates::default());
        assert!(
            r.iter()
                .any(|r| r.target == Target::Action(Action::CyclePageTheme)),
            "{r:?}"
        );
        // A URL, or words that name no action, keep the intent row first.
        let r = run("newboost.com", &Candidates::default());
        assert_eq!(r[0].target, Target::Url("https://newboost.com/".into()));
        let r = run("new york", &Candidates::default());
        assert_eq!(r[0].target, Target::Search("new york".into()));
        // The action leads open tabs and history too: Enter runs what the words name.
        let tabs = [OpenTab {
            id: TabId(1),
            title: "Reader mode tips".into(),
            url: "https://reader.example/".into(),
        }];
        let c = Candidates {
            tabs: &tabs,
            ..Default::default()
        };
        let r = run("reader mode", &c);
        assert_eq!(r[0].target, Target::Action(Action::ToggleReader));
    }

    #[test]
    fn math_answers_lead_and_unit_answers_follow_the_search() {
        let r = run("12*7+3", &Candidates::default());
        assert_eq!(r[0].kind, Kind::Answer);
        assert_eq!(r[0].target, Target::Copy("87".into()));
        assert_eq!(r[1].target, Target::Search("12*7+3".into()));
        let r = run("5 km in miles", &Candidates::default());
        assert_eq!(r[0].target, Target::Search("5 km in miles".into()));
        assert_eq!(r[1].kind, Kind::Answer);
        assert_eq!(r[1].target, Target::Copy("3.10686 mi".into()));
        // A bare number, a division by zero and a URL give no answer row.
        for q in ["42", "1/0", "example.com"] {
            assert!(
                run(q, &Candidates::default())
                    .iter()
                    .all(|r| r.kind != Kind::Answer),
                "{q}"
            );
        }
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
        assert_eq!(rank("same", &Engines::resolve("google", &[], &[]), &c, NOW, 2).len(), 2);
    }

    #[test]
    fn inline_completion_rules() {
        let history = [
            hist("https://github.com/rust-lang/rust", "Rust", 3, 2, 0),
            hist("https://www.example.com/", "Example", 1, 1, 0),
            hist("https://once.org/deep/page", "Once", 1, 1, 0),
            hist("https://busy.net/feed", "Busy", 12, 0, 0),
            hist("https://never.dev/", "Never", 3, 0, 0),
            hist("ftp://files.io/", "Files", 9, 9, 0),
        ];
        let bookmarks = [Bookmark {
            url: "https://marked.app/x".into(),
            title: "Marked".into(),
        }];
        let cases: [(&str, &str, Option<&str>); 16] = [
            ("query ends in the host: its root", "git", Some("hub.com/")),
            ("case-insensitive", "GIT", Some("hub.com/")),
            ("with the scheme", "https://gi", Some("thub.com/")),
            ("deeper typed twice", "github.com/rust", Some("-lang/rust")),
            ("www. skipped", "exa", Some("mple.com/")),
            ("www. typed", "www.ex", Some("ample.com/")),
            ("deep page typed once", "once.org/d", None),
            ("host of a deep page typed once", "onc", Some("e.org/")),
            ("visited 10 times", "busy.net/f", Some("eed")),
            ("visited, never typed", "nev", None),
            ("not http(s)", "fil", None),
            ("a bookmark", "marked.app/", Some("x")),
            ("trailing space", "git ", None),
            ("forced search", "?git", None),
            ("the whole url: nothing left", "https://www.example.com/", None),
            ("empty", "", None),
        ];
        for (name, query, want) in cases {
            assert_eq!(
                inline_completion(query, &history, &bookmarks).as_deref(),
                want,
                "{name}"
            );
        }
    }

    #[test]
    fn inline_completion_leads_rank_unless_prevented() {
        let history = [hist("https://github.com/", "GitHub", 5, 3, 0)];
        let c = Candidates {
            history: &history,
            ..Default::default()
        };
        let r = run("git", &c);
        assert_eq!(r[0].target, Target::Url("https://github.com/".into()));
        // the completed page is not listed twice
        assert_eq!(r.len(), 2);
        let r = run(
            "git",
            &Candidates {
                prevent_inline: true,
                ..c
            },
        );
        assert_eq!(r[0].target, Target::Search("git".into()));
    }

    #[test]
    fn just_deleted_cases() {
        let cases = [
            ("backspace at the end", "github", "githu", true),
            ("delete in the middle", "github", "gihub", true),
            ("selected completion dropped", "git", "git", true),
            ("everything deleted", "git", "", true),
            ("typed a letter", "git", "gith", false),
            ("typed over a selection", "github", "gitx", false),
            ("replaced the text", "abc", "xy", false),
        ];
        for (name, before, after, want) in cases {
            assert_eq!(just_deleted(before, after), want, "{name}");
        }
    }

    #[test]
    fn ctrl_enter_adds_www_and_com() {
        let cases = [
            ("bare name", "example", Some("https://www.example.com/")),
            ("with a path", "example/a", Some("https://www.example.com/a")),
            ("has a dot", "example.org", None),
            ("two words", "two words", None),
            ("empty", "", None),
        ];
        for (name, typed, want) in cases {
            assert_eq!(with_desired_tld(typed).as_deref(), want, "{name}");
        }
    }
}
