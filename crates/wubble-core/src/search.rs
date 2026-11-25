//! Search engines as data: the built-ins (`engines::ENGINES`), the user's own
//! (`settings::SEARCH_CUSTOM`) and the site searches found on pages
//! (`settings::SEARCH_DISCOVERED`, inactive), resolved from the stored rows into [`Engines`];
//! keyword search (`bing.com cats` searches Bing); and the OpenSearch discovery machine
//! ([`Discovery`], [`transition`]), whose fetch is an effect the host runs.
//!
//! Behaviour after Chromium (`components/search_engines`, BSD-3-Clause; no code copied):
//! `TemplateURL` (a name, a keyword and an address with one search-terms placeholder, `%s` in
//! the settings editor), `TemplateURL::GenerateKeyword` (the page's host without `www.`),
//! `SearchEngineTabHelper::PageHasOpenSearchDescriptionDocument` and `TemplateURLFetcher`
//! (http(s) only, one download per document, nothing for a keyword already known, a found
//! engine is never made the default), and `TemplateURLParser` (`ShortName`, the `text/html` GET
//! `Url` and the `application/x-suggestions+json` one; `{searchTerms}`, the encodings and the
//! optional `{name?}` parameters).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::engines::{self, ENGINES, EngineInfo, Reach};
use crate::omnibox::fill;
use crate::settings;
use crate::tab::TabId;

/// A user's engine is stored as the default by this and its keyword (lower-cased).
pub const CUSTOM_PREFIX: &str = "custom:";
/// Between an entry's fields: `name | keyword | address[ | suggestions]`.
pub const FIELD_SEP: &str = " | ";
/// Where the query goes in an address the user types: Chrome's `%s`, or wubble's `{q}`.
pub const PLACEHOLDERS: [&str; 2] = ["%s", "{q}"];
/// The list editor's columns for an engine list (and the built-ins under the Select).
pub const COLUMNS: [&str; 3] = ["Name", "Keyword", "Address"];

// ---------------------------------------------------------------- engines

/// One engine, resolved: what searching with it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineRef {
    /// What `settings::SEARCH_ENGINE` stores for it: a built-in's id, or
    /// [`CUSTOM_PREFIX`] and the keyword.
    pub id: String,
    pub name: String,
    pub keyword: String,
    /// The results page, with exactly one `{q}`.
    pub search: String,
    /// The OpenSearch suggestions endpoint, with one `{q}`; `None`: the engine has none, and
    /// live suggestions stay off for it.
    pub suggest: Option<String>,
    /// Search by image URL, with one `{q}` for the image's address (the page menu's "Search
    /// <engine> for Image"); `None`: the engine has none.
    pub image: Option<String>,
}

impl EngineRef {
    fn of_builtin(e: &EngineInfo) -> Self {
        EngineRef {
            id: e.id.to_owned(),
            name: e.name.to_owned(),
            keyword: e.keyword.to_owned(),
            search: e.search.to_owned(),
            suggest: e.suggest.map(str::to_owned),
            image: e.image.map(str::to_owned),
        }
    }

    /// The built-in engine stored as `id`.
    pub fn builtin(id: &str) -> Option<Self> {
        engines::engine_info(id).map(Self::of_builtin)
    }

    /// The results page for `query`.
    pub fn search_url(&self, query: &str) -> String {
        fill(&self.search, query)
    }

    /// The suggestions request for `query`, if the engine has an endpoint.
    pub fn suggest_url(&self, query: &str) -> Option<String> {
        self.suggest.as_deref().map(|t| fill(t, query))
    }

    pub fn is_builtin(&self) -> bool {
        !self.id.starts_with(CUSTOM_PREFIX)
    }
}

/// One engine as the user wrote it (an entry of `search.custom`) or as a site described it (an
/// entry of `search.discovered`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineEntry {
    pub name: String,
    pub keyword: String,
    /// With one `%s` or `{q}`.
    pub url: String,
    pub suggest: Option<String>,
}

impl EngineEntry {
    /// Reads a stored entry, `name | keyword | address[ | suggestions]`. A name may hold the
    /// separator; a keyword and an address cannot (they have no spaces), so the fields are read
    /// from the right.
    pub fn parse(text: &str) -> Option<Self> {
        let parts: Vec<&str> = text.split(FIELD_SEP).map(str::trim).collect();
        let address = |s: &str| s.contains("://");
        let n = parts.len();
        if n < 3 {
            return None;
        }
        let (fields, suggest) = if n >= 4 && address(parts[n - 1]) && address(parts[n - 2]) {
            (&parts[..n - 1], Some(parts[n - 1].to_owned()))
        } else {
            (&parts[..], None)
        };
        let m = fields.len();
        Some(EngineEntry {
            name: fields[..m - 2].join(FIELD_SEP),
            keyword: fields[m - 2].to_owned(),
            url: fields[m - 1].to_owned(),
            suggest,
        })
    }

    /// The list editor's three cells (name, keyword, address), trimmed.
    pub fn from_cells(cells: &[&str]) -> Self {
        let cell = |i: usize| cells.get(i).map_or("", |c| c.trim()).to_owned();
        EngineEntry {
            name: cell(0),
            keyword: cell(1),
            url: cell(2),
            suggest: None,
        }
    }

    /// The stored text ([`Self::parse`] reads it back).
    pub fn text(&self) -> String {
        let mut text = [self.name.as_str(), &self.keyword, &self.url].join(FIELD_SEP);
        if let Some(s) = &self.suggest {
            text.push_str(FIELD_SEP);
            text.push_str(s);
        }
        text
    }

    /// What the list editor shows: name, keyword, address.
    pub fn cells(&self) -> Vec<String> {
        vec![self.name.clone(), self.keyword.clone(), self.url.clone()]
    }

    /// What `settings::SEARCH_ENGINE` stores when this one is the default.
    pub fn id(&self) -> String {
        format!("{CUSTOM_PREFIX}{}", self.keyword.to_lowercase())
    }

    fn to_ref(&self) -> EngineRef {
        EngineRef {
            id: self.id(),
            name: self.name.clone(),
            keyword: self.keyword.clone(),
            search: template(&self.url),
            suggest: self
                .suggest
                .as_deref()
                .filter(|s| check_template(s).is_ok())
                .map(template),
            image: None,
        }
    }
}

/// An address as typed (`%s` or `{q}`) as a template with `{q}`.
fn template(url: &str) -> String {
    url.replace(PLACEHOLDERS[0], "{q}")
}

/// Why an engine is turned down: the note its list editor shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Not three fields.
    Incomplete,
    NoName,
    /// Empty, or more than one word.
    BadKeyword,
    NoPlaceholder,
    TwoPlaceholders,
    /// Not an http or https address.
    NotWeb,
    /// Another engine (a built-in, or one of the user's) has the keyword.
    KeywordTaken,
}

impl Problem {
    pub fn note(self) -> &'static str {
        match self {
            Problem::Incomplete => "Needs a name, a keyword and an address",
            Problem::NoName => "Needs a name",
            Problem::BadKeyword => "Needs a keyword of one word",
            Problem::NoPlaceholder => "Needs %s in the address where the search goes",
            Problem::TwoPlaceholders => "Needs %s only once in the address",
            Problem::NotWeb => "Needs an address that starts with http:// or https://",
            Problem::KeywordTaken => "Another search engine has this keyword",
        }
    }
}

/// A keyword is one word (Chrome's are any text without spaces); never the field separator.
pub fn valid_keyword(keyword: &str) -> bool {
    !keyword.is_empty() && !keyword.contains(char::is_whitespace) && !keyword.contains('|')
}

/// An address checks out: exactly one placeholder (`%s` or `{q}`), no spaces, and with a
/// query in it an http(s) URL with a host.
pub fn check_template(url: &str) -> Result<(), Problem> {
    match PLACEHOLDERS
        .iter()
        .map(|p| url.matches(p).count())
        .sum::<usize>()
    {
        0 => return Err(Problem::NoPlaceholder),
        1 => {}
        _ => return Err(Problem::TwoPlaceholders),
    }
    if url.contains(char::is_whitespace) {
        return Err(Problem::NotWeb);
    }
    match ::url::Url::parse(&fill(&template(url), "x")) {
        Ok(u)
            if matches!(u.scheme(), "http" | "https")
                && u.host_str().is_some_and(|h| !h.is_empty()) =>
        {
            Ok(())
        }
        _ => Err(Problem::NotWeb),
    }
}

/// Checks an engine: a name, a one-word keyword no other engine has (`taken` is asked with
/// the keyword lower-cased), and a good address ([`check_template`]).
pub fn check(entry: &EngineEntry, taken: impl Fn(&str) -> bool) -> Result<(), Problem> {
    if entry.name.trim().is_empty() {
        return Err(Problem::NoName);
    }
    if !valid_keyword(&entry.keyword) {
        return Err(Problem::BadKeyword);
    }
    check_template(&entry.url)?;
    if taken(&entry.keyword.to_lowercase()) {
        return Err(Problem::KeywordTaken);
    }
    Ok(())
}

/// Whether `id` may be stored as the default: a built-in's id but a keyword engine's
/// ([`Reach::Site`]), or a user's engine's.
pub fn is_engine_id(id: &str) -> bool {
    engines::engine_info(id).is_some_and(|e| e.reach != Reach::Site)
        || id.strip_prefix(CUSTOM_PREFIX).is_some_and(valid_keyword)
}

/// Every engine, resolved from the settings: the one owner the host keeps
/// (`apply::Applied::SearchEngines`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Engines {
    /// `None` until the user chooses (asked on the first search), or when the chosen one is gone.
    pub default: Option<EngineRef>,
    /// The built-ins, then the user's own that check out, in their order: what keywords search
    /// and the default is chosen from.
    pub active: Vec<EngineRef>,
    /// Found on sites, inactive: listed, never searched.
    pub discovered: Vec<EngineEntry>,
    /// The built-ins offered for the country (`settings::SEARCH_REGION`, `engines::region`), by
    /// id, in Chrome's order: the first-run choice, and the head of the default's choices.
    pub region: &'static [&'static str],
}

impl Engines {
    /// The engines of these rows: the default's id, `search.custom`'s and
    /// `search.discovered`'s entries. An entry that does not check out, or whose keyword an
    /// earlier engine has, is left out.
    pub fn resolve(default_id: &str, custom: &[String], discovered: &[String]) -> Engines {
        let mut active: Vec<EngineRef> = ENGINES.iter().map(EngineRef::of_builtin).collect();
        for text in custom {
            let Some(entry) = EngineEntry::parse(text) else {
                continue;
            };
            let ok = check(&entry, |k| {
                active.iter().any(|e| e.keyword.to_lowercase() == k)
            });
            if ok.is_ok() {
                active.push(entry.to_ref());
            }
        }
        let mut found: Vec<EngineEntry> = Vec::new();
        for text in discovered {
            let Some(entry) = EngineEntry::parse(text) else {
                continue;
            };
            let ok = check(&entry, |k| {
                active.iter().any(|e| e.keyword.to_lowercase() == k)
                    || found.iter().any(|e| e.keyword.to_lowercase() == k)
            });
            if ok.is_ok() {
                found.push(entry);
            }
        }
        let default = active
            .iter()
            .find(|e| e.id == default_id && is_engine_id(&e.id))
            .cloned();
        Engines {
            default,
            active,
            discovered: found,
            region: engines::region(""),
        }
    }

    /// The engines of the stored rows (`key -> text`).
    pub fn from_stored(stored: &HashMap<String, String>) -> Engines {
        Engines {
            region: engines::region(crate::apply::value_of(&settings::SEARCH_REGION, stored)),
            ..Engines::resolve(
                crate::apply::value_of(&settings::SEARCH_ENGINE, stored),
                &crate::apply::list(&settings::SEARCH_CUSTOM, stored),
                &crate::apply::list(&settings::SEARCH_DISCOVERED, stored),
            )
        }
    }

    /// An active engine by its stored id.
    pub fn by_id(&self, id: &str) -> Option<&EngineRef> {
        self.active.iter().find(|e| e.id == id)
    }

    /// `input` starts with an active engine's keyword (any case) and a space, and a query
    /// follows: that engine and the query (Chrome's keyword mode).
    pub fn keyword_search<'q>(&self, input: &'q str) -> Option<(&EngineRef, &'q str)> {
        let (word, rest) = input.trim_start().split_once(char::is_whitespace)?;
        let rest = rest.trim();
        if rest.is_empty() {
            return None;
        }
        let word = word.to_lowercase();
        self.active
            .iter()
            .find(|e| e.keyword.to_lowercase() == word)
            .map(|e| (e, rest))
    }

    /// Whether an engine, active or found on a site, has `keyword` (any case).
    pub fn knows_keyword(&self, keyword: &str) -> bool {
        let keyword = keyword.to_lowercase();
        self.active
            .iter()
            .map(|e| &e.keyword)
            .chain(self.discovered.iter().map(|e| &e.keyword))
            .any(|k| k.to_lowercase() == keyword)
    }

    /// The first-run choice: the region's engines, in its order (Chrome's choice screen).
    pub fn first_run(&self) -> impl Iterator<Item = &EngineRef> {
        self.region.iter().filter_map(|id| self.by_id(id))
    }

    /// The default engine's choices, `(stored id, name)`: the [`Reach::Everywhere`] engines in
    /// table order (the same list in every country), the region's own engines, the user's own,
    /// and the default wherever it is from (a region's engine kept after a move). A keyword
    /// engine or a site's is never one.
    pub fn choices(&self) -> Vec<(String, String)> {
        let everywhere = ENGINES
            .iter()
            .filter(|e| e.reach == Reach::Everywhere)
            .map(|e| e.id);
        let mut ids: Vec<&str> = Vec::new();
        let custom = self
            .active
            .iter()
            .filter(|e| !e.is_builtin())
            .map(|e| e.id.as_str());
        let default = self.default.iter().map(|e| e.id.as_str());
        for id in everywhere
            .chain(self.region.iter().copied())
            .chain(custom)
            .chain(default)
        {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.into_iter()
            .filter_map(|id| self.by_id(id))
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect()
    }
}

/// The default stored once the user's engine stored as `old` is edited to `new` (`None`:
/// removed), when that engine is the default `default_id`: the new one's id, or `""` (ask
/// again) once it is gone. `None` when the default is another engine.
pub fn default_after_edit(default_id: &str, old: &str, new: Option<&str>) -> Option<String> {
    let old = EngineEntry::parse(old)?;
    if old.id() != default_id {
        return None;
    }
    let new = new.and_then(EngineEntry::parse).map(|e| e.id());
    match new {
        Some(id) if id == default_id => None,
        Some(id) => Some(id),
        None => Some(String::new()),
    }
}

/// `entries` (stored engine texts) without the one with `keyword`: `None` when none has it.
pub fn without_keyword(entries: &[String], keyword: &str) -> Option<Vec<String>> {
    let keyword = keyword.to_lowercase();
    let kept: Vec<String> = entries
        .iter()
        .filter(|t| EngineEntry::parse(t).is_none_or(|e| e.keyword.to_lowercase() != keyword))
        .cloned()
        .collect();
    (kept.len() != entries.len()).then_some(kept)
}

// -------------------------------------------------------------- discovery

/// The keyword Chrome generates for a site's search: the page's host, lower-cased, without a
/// leading `www.`; only for an http(s) page.
pub fn generate_keyword(page: &str) -> Option<String> {
    let url = ::url::Url::parse(page).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (!host.is_empty()).then(|| host.to_owned())
}

fn is_web(url: &str) -> bool {
    ::url::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

/// The page script's answer (`OPENSEARCH_LINK` in the host's page scripts): the description's
/// absolute address as a JSON string, or `null` when the page links none.
pub fn osdd_link(answer: &str) -> Option<String> {
    serde_json::from_str::<Option<String>>(answer)
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
}

/// The discovery of one run: which descriptions were asked for (each once) and which are out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    asked: BTreeSet<String>,
    /// The description's address to the keyword its site's engine gets.
    pending: BTreeMap<String, String>,
}

impl Discovery {
    /// Whether a description is being fetched.
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A page that finished loading in `tab` links a search description at `osdd`.
    Found {
        tab: TabId,
        page: String,
        osdd: String,
    },
    /// The description came back: its text, or `None` on any failure.
    Fetched { osdd: String, body: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEffect {
    /// Fetch the description at `osdd` in `tab`'s network session, then answer `Fetched`.
    Fetch { tab: TabId, osdd: String },
    /// Add this site's engine to `search.discovered`.
    Add(EngineEntry),
}

/// What the machine reads of the settings.
#[derive(Debug, Clone, Copy)]
pub struct DiscoveryContext<'a> {
    /// `settings::SEARCH_DISCOVER_SITES`: off, nothing is fetched and nothing added.
    pub enabled: bool,
    pub engines: &'a Engines,
}

pub fn transition(
    s: Discovery,
    e: DiscoveryEvent,
    cx: &DiscoveryContext,
) -> (Discovery, Vec<DiscoveryEffect>) {
    let mut s = s;
    match e {
        DiscoveryEvent::Found { tab, page, osdd } => {
            if !cx.enabled || !is_web(&osdd) {
                return (s, vec![]);
            }
            let Some(keyword) = generate_keyword(&page) else {
                return (s, vec![]);
            };
            if cx.engines.knows_keyword(&keyword)
                || s.asked.contains(&osdd)
                || s.pending.values().any(|k| *k == keyword)
            {
                return (s, vec![]);
            }
            s.asked.insert(osdd.clone());
            s.pending.insert(osdd.clone(), keyword);
            (s, vec![DiscoveryEffect::Fetch { tab, osdd }])
        }
        DiscoveryEvent::Fetched { osdd, body } => {
            let Some(keyword) = s.pending.remove(&osdd) else {
                return (s, vec![]);
            };
            if !cx.enabled {
                return (s, vec![]);
            }
            let Some(osdd) = body.as_deref().and_then(parse_osdd) else {
                return (s, vec![]);
            };
            let entry = EngineEntry {
                name: if osdd.name.is_empty() {
                    keyword.clone()
                } else {
                    osdd.name
                },
                keyword,
                url: osdd.search,
                suggest: osdd.suggest,
            };
            match check(&entry, |k| cx.engines.knows_keyword(k)) {
                Ok(()) => (s, vec![DiscoveryEffect::Add(entry)]),
                Err(_) => (s, vec![]),
            }
        }
    }
}

// ------------------------------------------------------------------ OSDD

/// What an OpenSearch description says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Osdd {
    /// `ShortName`, trimmed (may be empty).
    pub name: String,
    /// The `text/html` GET `Url`'s template, with `{q}`.
    pub search: String,
    /// The `application/x-suggestions+json` GET `Url`'s, if any.
    pub suggest: Option<String>,
}

/// Reads an OpenSearch description document: `None` without a usable results page.
pub fn parse_osdd(xml: &str) -> Option<Osdd> {
    let tags = start_tags(xml);
    let name = tags
        .iter()
        .find(|t| t.name == "ShortName")
        .map(|t| decode(t.text).trim().to_owned())
        .unwrap_or_default();
    let url_of = |kind: &str| {
        tags.iter()
            .filter(|t| t.name == "Url")
            .filter(|t| {
                t.attr("type")
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case(kind))
            })
            .filter(|t| {
                t.attr("method")
                    .is_none_or(|m| m.trim().eq_ignore_ascii_case("get"))
            })
            .find_map(osdd_template)
    };
    let search = url_of("text/html")?;
    let suggest = url_of("application/x-suggestions+json");
    Some(Osdd {
        name,
        search,
        suggest,
    })
}

/// A `Url` element's template with its parameters filled as Chrome's parser does:
/// `{searchTerms}` becomes `{q}`, the encodings UTF-8, the language any (`*`), the first index
/// and page their offsets, an optional parameter nothing; any other parameter makes it unusable.
fn osdd_template(tag: &Tag) -> Option<String> {
    let template = decode(tag.attr("template")?);
    let index = tag.attr("indexOffset").unwrap_or("1");
    let page = tag.attr("pageOffset").unwrap_or("1");
    let mut out = String::new();
    let mut rest = template.trim();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = open + rest[open..].find('}')?;
        let param = &rest[open + 1..close];
        let (param, optional) = match param.strip_suffix('?') {
            Some(p) => (p, true),
            None => (param, false),
        };
        match param.rsplit(':').next().unwrap_or(param) {
            "searchTerms" => out.push_str("{q}"),
            "inputEncoding" | "outputEncoding" => out.push_str("UTF-8"),
            "language" => out.push('*'),
            "startIndex" => out.push_str(index),
            "startPage" => out.push_str(page),
            _ if optional => {}
            _ => return None,
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    check_template(&out).ok().map(|()| out)
}

/// A start tag: its local name (no namespace prefix), its attributes, and the text after it.
struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
    text: &'a str,
}

impl Tag<'_> {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }
}

/// The start tags of an XML text, in order; comments, declarations and end tags skipped. A
/// tolerant reader for the few elements an OpenSearch description needs, not a parser.
fn start_tags(xml: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let r = &rest[lt + 1..];
        if let Some(comment) = r.strip_prefix("!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        if r.starts_with(['/', '?', '!']) {
            rest = r.find('>').map_or("", |end| &r[end + 1..]);
            continue;
        }
        let Some(gt) = tag_end(r) else {
            break;
        };
        let inner = r[..gt].trim_end_matches('/');
        let after = &r[gt + 1..];
        let text = &after[..after.find('<').unwrap_or(after.len())];
        let (qname, attrs) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
        out.push(Tag {
            name: qname.rsplit(':').next().unwrap_or(qname),
            attrs: attributes(attrs),
            text,
        });
        rest = after;
    }
    out
}

/// Where a tag's `>` is, outside quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// `name="value"` pairs (either quote), names without their namespace prefix.
fn attributes(s: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim();
        let after = rest[eq + 1..].trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let body = &after[1..];
        let Some(end) = body.find(quote) else {
            break;
        };
        out.push((name.rsplit(':').next().unwrap_or(name), &body[..end]));
        rest = body[end + 1..].trim_start();
    }
    out
}

/// XML's five named entities and numeric references; anything else is kept as written.
fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match tail
            .find(';')
            .filter(|&semi| semi <= 10)
            .and_then(|semi| Some((semi, entity(&tail[1..semi])?)))
        {
            Some((semi, c)) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let code = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => name.strip_prefix('#')?.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn templates_need_one_placeholder_and_the_web() {
        let cases = [
            ("https://a.test/?q=%s", Ok(())),
            ("https://a.test/?q={q}", Ok(())),
            ("http://a.test/s/%s/x", Ok(())),
            ("https://a.test/", Err(Problem::NoPlaceholder)),
            ("https://a.test/?q=%s&r=%s", Err(Problem::TwoPlaceholders)),
            ("https://a.test/?q=%s&r={q}", Err(Problem::TwoPlaceholders)),
            ("ftp://a.test/%s", Err(Problem::NotWeb)),
            ("javascript:alert(%s)", Err(Problem::NotWeb)),
            ("a.test/?q=%s", Err(Problem::NotWeb)),
            ("https://a.test/?q=%s x", Err(Problem::NotWeb)),
        ];
        for (url, want) in cases {
            assert_eq!(check_template(url), want, "{url}");
        }
    }

    #[test]
    fn an_engine_needs_a_name_a_free_keyword_and_a_good_address() {
        let entry = |name: &str, kw: &str, url: &str| EngineEntry {
            name: name.into(),
            keyword: kw.into(),
            url: url.into(),
            suggest: None,
        };
        let free = |_: &str| false;
        assert_eq!(
            check(&entry("Docs", "d", "https://d.test/?q=%s"), free),
            Ok(())
        );
        assert_eq!(
            check(&entry(" ", "d", "https://d.test/?q=%s"), free),
            Err(Problem::NoName)
        );
        assert_eq!(
            check(&entry("Docs", "two words", "https://d.test/?q=%s"), free),
            Err(Problem::BadKeyword)
        );
        assert_eq!(
            check(&entry("Docs", "", "https://d.test/?q=%s"), free),
            Err(Problem::BadKeyword)
        );
        assert_eq!(
            check(&entry("Docs", "D", "https://d.test/?q=%s"), |k| k == "d"),
            Err(Problem::KeywordTaken)
        );
    }

    #[test]
    fn entries_round_trip_and_read_from_the_right() {
        let e = EngineEntry {
            name: "My | docs".into(),
            keyword: "d".into(),
            url: "https://d.test/?q=%s".into(),
            suggest: None,
        };
        assert_eq!(EngineEntry::parse(&e.text()), Some(e.clone()));
        let s = EngineEntry {
            suggest: Some("https://d.test/s?q={q}".into()),
            ..e.clone()
        };
        assert_eq!(EngineEntry::parse(&s.text()), Some(s));
        assert_eq!(EngineEntry::parse("just text"), None);
        assert_eq!(
            EngineEntry::from_cells(&[" Docs ", "d", "https://d.test/?q=%s"]).text(),
            "Docs | d | https://d.test/?q=%s"
        );
    }

    #[test]
    fn engines_resolve_from_the_rows() {
        let e = Engines::resolve(
            "custom:docs",
            &custom(&[
                "Docs | Docs | https://docs.test/?q=%s",
                // its keyword is a built-in's: left out
                "Mine | bing.com | https://mine.test/?q=%s",
                "Bad | bad | https://bad.test/",
                // a second engine with the keyword: left out
                "Docs again | docs | https://again.test/?q=%s",
            ]),
            &custom(&[
                "Shop | shop.test | https://shop.test/find?q={q} | https://shop.test/s?q={q}",
                "Dup | docs | https://dup.test/?q=%s",
            ]),
        );
        let default = e
            .default
            .as_ref()
            .expect("the user's engine is the default");
        assert_eq!(default.name, "Docs");
        assert_eq!(default.search_url("a b"), "https://docs.test/?q=a%20b");
        assert_eq!(
            default.suggest_url("a"),
            None,
            "no endpoint, no live suggestions"
        );
        assert_eq!(e.active.len(), ENGINES.len() + 1);
        assert_eq!(e.discovered.len(), 1);
        assert!(e.knows_keyword("SHOP.test") && e.knows_keyword("bing.com"));
        // a site's engine is listed, never a choice for the default
        let ids: Vec<String> = e.choices().into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&"custom:docs".to_owned()) && ids.contains(&"bing".to_owned()));
        assert!(!ids.iter().any(|id| id.contains("shop")));
        assert_eq!(
            Engines::resolve(
                "custom:shop.test",
                &[],
                &e.discovered
                    .iter()
                    .map(EngineEntry::text)
                    .collect::<Vec<_>>()
            )
            .default,
            None
        );
        // an unknown or gone default asks again
        assert_eq!(Engines::resolve("custom:gone", &[], &[]).default, None);
        assert_eq!(Engines::resolve("", &[], &[]).default, None);
        assert_eq!(
            Engines::resolve("bing", &[], &[]).default.map(|d| d.name),
            Some("Bing".into())
        );
    }

    #[test]
    fn a_keyword_and_a_space_pick_the_engine() {
        let e = Engines::resolve("google", &custom(&["Docs | d | https://d.test/?q=%s"]), &[]);
        let (engine, q) = e.keyword_search("d  rust book ").unwrap();
        assert_eq!((engine.name.as_str(), q), ("Docs", "rust book"));
        assert_eq!(
            e.keyword_search("Wikipedia.org cats").unwrap().0.name,
            "Wikipedia"
        );
        assert_eq!(
            e.keyword_search("d"),
            None,
            "a keyword alone searches nothing"
        );
        assert_eq!(e.keyword_search("d   "), None);
        assert_eq!(e.keyword_search("dd rust"), None);
    }

    #[test]
    fn the_region_offers_its_engines() {
        let stored = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let first = |e: &Engines| -> Vec<String> { e.first_run().map(|e| e.id.clone()).collect() };
        let choices =
            |e: &Engines| -> Vec<String> { e.choices().into_iter().map(|(id, _)| id).collect() };
        let everywhere: Vec<String> = ENGINES
            .iter()
            .filter(|e| e.reach == Reach::Everywhere)
            .map(|e| e.id.to_owned())
            .collect();
        // no region: Chromium's default list, and the Select is the everywhere engines
        let e = Engines::from_stored(&stored(&[]));
        assert_eq!(first(&e), ["google", "bing", "yahoo"]);
        assert_eq!(choices(&e), everywhere);
        // Germany: its list in its order, its Yahoo added to the Select after the others
        let e = Engines::from_stored(&stored(&[("search.region", "DE")]));
        assert_eq!(
            first(&e),
            [
                "google",
                "duckduckgo",
                "brave",
                "ecosia",
                "perplexity",
                "bing",
                "startpage",
                "yahoo_de"
            ]
        );
        let mut want = everywhere.clone();
        want.push("yahoo_de".into());
        assert_eq!(choices(&e), want);
        // the default from another region stays a choice; the user's own follow
        let e = Engines::from_stored(&stored(&[
            ("search.region", "US"),
            ("search.engine", "yandex_ru"),
            ("search.custom", r#"["Docs | d | https://d.test/?q=%s"]"#),
        ]));
        assert_eq!(e.default.as_ref().map(|d| d.id.as_str()), Some("yandex_ru"));
        let mut want = everywhere.clone();
        want.extend(["custom:d".to_owned(), "yandex_ru".to_owned()]);
        assert_eq!(choices(&e), want);
        // a keyword engine is never the default, even if stored as one
        let e = Engines::from_stored(&stored(&[("search.engine", "wikipedia")]));
        assert_eq!(e.default, None);
        assert!(e.keyword_search("youtube.com cats").is_some());
        assert!(e.keyword_search("github.com wubble").is_some());
    }

    #[test]
    fn engine_ids_are_built_ins_or_keywords() {
        assert!(is_engine_id("google") && is_engine_id("yandex_ru"));
        // a keyword engine is searched by its keyword only, never the default
        assert!(!is_engine_id("wikipedia") && !is_engine_id("github"));
        assert!(is_engine_id("custom:d"));
        assert!(!is_engine_id("custom:") && !is_engine_id("custom:a b") && !is_engine_id("x"));
    }

    #[test]
    fn editing_the_default_moves_it() {
        let old = "Docs | d | https://d.test/?q=%s";
        assert_eq!(
            default_after_edit("custom:d", old, Some("Docs | docs | https://d.test/?q=%s")),
            Some("custom:docs".into())
        );
        assert_eq!(
            default_after_edit("custom:d", old, None),
            Some(String::new())
        );
        assert_eq!(
            default_after_edit("custom:d", old, Some("Docs 2 | D | https://e.test/?q=%s")),
            None
        );
        assert_eq!(default_after_edit("google", old, None), None);
        let list = custom(&[old, "Shop | s | https://s.test/?q=%s"]);
        assert_eq!(
            without_keyword(&list, "D"),
            Some(custom(&["Shop | s | https://s.test/?q=%s"]))
        );
        assert_eq!(without_keyword(&list, "x"), None);
    }

    const OSDD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment <Url template="https://no.test/{searchTerms}"/> -->
<OpenSearchDescription xmlns="http://a9.com/-/spec/opensearch/1.1/" xmlns:moz="http://www.mozilla.org/2006/browser/search/">
  <ShortName>Shop &amp; Co</ShortName>
  <InputEncoding>UTF-8</InputEncoding>
  <Url type="application/x-suggestions+json" template="https://shop.test/suggest?q={searchTerms}&amp;l={language}"/>
  <Url type="text/html" method="post" template="https://shop.test/post"/>
  <Url type='text/html' template='https://shop.test/find?q={searchTerms}&amp;p={startPage}&amp;e={inputEncoding}&amp;x={moz:foo?}'/>
</OpenSearchDescription>"#;

    #[test]
    fn a_description_gives_its_name_and_templates() {
        assert_eq!(
            parse_osdd(OSDD),
            Some(Osdd {
                name: "Shop & Co".into(),
                search: "https://shop.test/find?q={q}&p=1&e=UTF-8&x=".into(),
                suggest: Some("https://shop.test/suggest?q={q}&l=*".into()),
            })
        );
        assert_eq!(parse_osdd("<OpenSearchDescription/>"), None);
        assert_eq!(
            parse_osdd(
                r#"<Url type="text/html" template="https://a.test/?q={searchTerms}&n={count}"/>"#
            ),
            None,
            "a parameter Chrome cannot fill"
        );
        assert_eq!(decode("a&#38;b&#x26;c&nbsp;"), "a&b&c&nbsp;");
    }

    #[test]
    fn discovery_fetches_once_and_adds_an_inactive_engine() {
        let engines = Engines::resolve("google", &[], &[]);
        let on = DiscoveryContext {
            enabled: true,
            engines: &engines,
        };
        let found = |page: &str, osdd: &str| DiscoveryEvent::Found {
            tab: TabId(1),
            page: page.into(),
            osdd: osdd.into(),
        };
        let osdd = "https://www.shop.test/opensearch.xml";
        let (s, fx) = transition(
            Discovery::default(),
            found("https://www.shop.test/", osdd),
            &on,
        );
        assert_eq!(
            fx,
            [DiscoveryEffect::Fetch {
                tab: TabId(1),
                osdd: osdd.into()
            }]
        );
        // asked once only
        let (s, fx) = transition(s, found("https://www.shop.test/a", osdd), &on);
        assert!(fx.is_empty());
        let (s, fx) = transition(
            s,
            DiscoveryEvent::Fetched {
                osdd: osdd.into(),
                body: Some(OSDD.into()),
            },
            &on,
        );
        assert_eq!(
            fx,
            [DiscoveryEffect::Add(EngineEntry {
                name: "Shop & Co".into(),
                keyword: "shop.test".into(),
                url: "https://shop.test/find?q={q}&p=1&e=UTF-8&x=".into(),
                suggest: Some("https://shop.test/suggest?q={q}&l=*".into()),
            })]
        );
        assert!(s.is_idle());
        // a late or unknown answer does nothing
        let (_, fx) = transition(
            s,
            DiscoveryEvent::Fetched {
                osdd: osdd.into(),
                body: Some(OSDD.into()),
            },
            &on,
        );
        assert!(fx.is_empty());
    }

    #[test]
    fn discovery_off_or_known_asks_nothing() {
        let engines = Engines::resolve("google", &[], &[]);
        let found = |page: &str, osdd: &str| DiscoveryEvent::Found {
            tab: TabId(1),
            page: page.into(),
            osdd: osdd.into(),
        };
        let off = DiscoveryContext {
            enabled: false,
            engines: &engines,
        };
        let on = DiscoveryContext {
            enabled: true,
            ..off
        };
        let cases = [
            (off, found("https://shop.test/", "https://shop.test/o.xml")),
            // a built-in's keyword
            (
                on,
                found("https://www.bing.com/", "https://www.bing.com/o.xml"),
            ),
            (on, found("wubble://settings", "https://shop.test/o.xml")),
            (on, found("https://shop.test/", "data:text/xml,x")),
            (on, found("file:///tmp/a.html", "file:///tmp/o.xml")),
        ];
        for (cx, event) in cases {
            let (_, fx) = transition(Discovery::default(), event.clone(), &cx);
            assert!(fx.is_empty(), "{event:?}");
        }
        // turned off while the fetch was out: nothing is added
        let (s, _) = transition(
            Discovery::default(),
            found("https://shop.test/", "https://shop.test/o.xml"),
            &on,
        );
        let (_, fx) = transition(
            s,
            DiscoveryEvent::Fetched {
                osdd: "https://shop.test/o.xml".into(),
                body: Some(OSDD.into()),
            },
            &off,
        );
        assert!(fx.is_empty());
        assert_eq!(
            generate_keyword("https://WWW.Shop.test:8443/x"),
            Some("shop.test".into())
        );
        assert_eq!(
            osdd_link("\"https://a.test/o.xml\""),
            Some("https://a.test/o.xml".into())
        );
        assert_eq!(osdd_link("null"), None);
    }
}
