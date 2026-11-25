//! The built-in search engines and the regions they are offered in, as data.
//!
//! The engines' names, keywords and search and suggestion addresses, and the per-country
//! engine lists ([`REGIONS`], [`ALIASES`]), are copied from Chromium's
//! `third_party/search_engines_data/resources/definitions/prepopulated_engines.json` and
//! `regional_settings.json` (CEF 156's Chromium, BSD-3-Clause). The addresses are written with
//! `{q}` for `{searchTerms}`, UTF-8 for `{inputEncoding}` and `{outputEncoding}`, and without
//! Chrome's own parameters (`{google:...}`, `{yandex:...}`, `{language}`, the partner codes
//! `fr=crmas*`, `sm=chr_hty`, `DA=JU*`, the Yahoo Japan app id). Kagi is not in Chromium's list;
//! its addresses, and YouTube's, GitHub's and Wikipedia's (keyword engines), come from each
//! site's own OpenSearch description.
//!
//! Adding an engine is a row in [`ENGINES`]; a country's list is a row in [`REGIONS`].

/// Where a built-in engine is offered as the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// In every region's choice (the engines wubble names: Chrome's main ones and the private
    /// ones).
    Everywhere,
    /// Only where its region lists it ([`REGIONS`]): a country's own engine or its Yahoo or
    /// Yandex.
    Regional,
    /// Never the default: searched by its keyword only (`wikipedia.org rust`), as Chrome's
    /// site search.
    Site,
}

/// One built-in search engine. Templates contain exactly one `{q}`.
#[derive(Debug, PartialEq, Eq)]
pub struct EngineInfo {
    /// The text form stored in settings; never rename a shipped one.
    pub id: &'static str,
    pub name: &'static str,
    /// Typed before a query (`bing.com cats`), it searches this engine (Chrome's keyword).
    pub keyword: &'static str,
    pub search: &'static str,
    /// OpenSearch suggestions JSON (`["query", ["a", "b", ...]]`); `None`: the engine has none.
    pub suggest: Option<&'static str>,
    /// Search by image URL, for the page menu's "Search <engine> for Image"; `None` for an
    /// engine with no public one by URL.
    pub image: Option<&'static str>,
    pub reach: Reach,
}

const fn engine(
    id: &'static str,
    name: &'static str,
    keyword: &'static str,
    search: &'static str,
    suggest: Option<&'static str>,
    reach: Reach,
) -> EngineInfo {
    EngineInfo {
        id,
        name,
        keyword,
        search,
        suggest,
        image: None,
        reach,
    }
}

use Reach::{Everywhere, Regional, Site};

/// Every built-in engine. The [`Reach::Everywhere`] ones come first, in the order the Select
/// lists them after a region's own.
pub const ENGINES: [EngineInfo; 40] = [
    EngineInfo {
        id: "google",
        name: "Google",
        keyword: "google.com",
        search: "https://www.google.com/search?q={q}",
        suggest: Some("https://suggestqueries.google.com/complete/search?client=firefox&q={q}"),
        // Google Lens by URL: `searchbyimage` was retired for it.
        image: Some("https://lens.google.com/uploadbyurl?url={q}"),
        reach: Everywhere,
    },
    engine(
        "duckduckgo",
        "DuckDuckGo",
        "duckduckgo.com",
        "https://duckduckgo.com/?q={q}",
        Some("https://duckduckgo.com/ac/?type=list&q={q}"),
        Everywhere,
    ),
    engine(
        "bing",
        "Bing",
        "bing.com",
        "https://www.bing.com/search?q={q}",
        Some("https://www.bing.com/osjson.aspx?query={q}"),
        Everywhere,
    ),
    engine(
        "brave",
        "Brave Search",
        "search.brave.com",
        "https://search.brave.com/search?q={q}",
        Some("https://search.brave.com/api/suggest?q={q}"),
        Everywhere,
    ),
    engine(
        "kagi",
        "Kagi",
        "kagi.com",
        "https://kagi.com/search?q={q}",
        Some("https://kagi.com/api/autosuggest?q={q}"),
        Everywhere,
    ),
    engine(
        "startpage",
        "Startpage",
        "startpage.com",
        "https://www.startpage.com/sp/search?q={q}",
        Some("https://www.startpage.com/osuggestions?q={q}"),
        Everywhere,
    ),
    engine(
        "ecosia",
        "Ecosia",
        "ecosia.org",
        "https://www.ecosia.org/search?q={q}",
        Some("https://ac.ecosia.org/autocomplete?q={q}&type=list"),
        Everywhere,
    ),
    engine(
        "qwant",
        "Qwant",
        "qwant.com",
        "https://www.qwant.com/?q={q}",
        Some("https://api.qwant.com/api/suggest/?q={q}&client=opensearch"),
        Everywhere,
    ),
    engine(
        "perplexity",
        "Perplexity",
        "perplexity.ai",
        "https://www.perplexity.ai/search?q={q}",
        Some("https://suggest.perplexity.ai/suggest?q={q}"),
        Everywhere,
    ),
    engine(
        "yahoo",
        "Yahoo!",
        "yahoo.com",
        "https://search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Everywhere,
    ),
    engine(
        "yandex_com",
        "Yandex",
        "yandex.com",
        "https://yandex.com/search/?text={q}",
        Some("https://suggest.yandex.com/suggest-ff.cgi?part={q}"),
        Everywhere,
    ),
    engine(
        "baidu",
        "百度",
        "baidu.com",
        "https://www.baidu.com/#ie=UTF-8&wd={q}",
        Some("https://suggestion.baidu.com/su?wd={q}&action=opensearch&ie=UTF-8"),
        Everywhere,
    ),
    engine(
        "naver",
        "네이버",
        "naver.com",
        "https://search.naver.com/search.naver?ie=UTF-8&query={q}",
        Some("https://ac.search.naver.com/nx/ac?of=os&ie=UTF-8&q={q}&oe=UTF-8"),
        Everywhere,
    ),
    // a country's own engines
    engine(
        "coccoc",
        "Cốc Cốc",
        "coccoc.com",
        "https://coccoc.com/search#query={q}",
        None,
        Regional,
    ),
    engine(
        "daum",
        "Daum",
        "daum.net",
        "https://search.daum.net/search?w=tot&q={q}",
        Some("https://suggest.search.daum.net/sushi/opensearch/pc?q={q}"),
        Regional,
    ),
    engine(
        "privacywall",
        "PrivacyWall",
        "privacywall.org",
        "https://www.privacywall.org/search/secure/?q={q}",
        Some("https://search.privacywall.org/suggest.php?q={q}"),
        Regional,
    ),
    engine(
        "seznam",
        "Seznam.cz",
        "seznam",
        "https://search.seznam.cz/?q={q}",
        Some("https://suggest.seznam.cz/fulltext_ff?phrase={q}"),
        Regional,
    ),
    engine(
        "so_360",
        "360",
        "so.com",
        "https://www.so.com/s?ie=UTF-8&q={q}",
        Some("https://sug.so.360.cn/suggest?encodein=UTF-8&encodeout=UTF-8&format=opensearch&word={q}"),
        Regional,
    ),
    // Sogou's suggestions are not OpenSearch JSON
    engine(
        "sogou",
        "搜狗",
        "sogou.com",
        "https://www.sogou.com/web?ie=UTF-8&query={q}",
        None,
        Regional,
    ),
    engine(
        "yep",
        "Yep",
        "yep.com",
        "https://yep.com/web?q={q}",
        Some("https://api.yep.com/ac/?query={q}&os=true"),
        Regional,
    ),
    engine(
        "you",
        "You.com",
        "you.com",
        "https://you.com/search?tbm=youchat&q={q}",
        Some("https://you.com/api/ac?domain=default&q={q}"),
        Regional,
    ),
    // the regional Yahoos and Yandexes
    engine(
        "yahoo_at",
        "Yahoo Search",
        "at.yahoo.com",
        "https://at.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://at.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_de",
        "Yahoo Search",
        "de.yahoo.com",
        "https://de.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://de.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_dk",
        "Yahoo Search",
        "dk.yahoo.com",
        "https://dk.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://dk.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_emea",
        "Yahoo Search",
        "emea.yahoo.com",
        "https://emea.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://emea.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_es",
        "Yahoo Búsquedas",
        "es.yahoo.com",
        "https://es.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://es.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_fr",
        "Yahoo Recherche",
        "fr.yahoo.com",
        "https://fr.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://fr.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_in",
        "Yahoo! India",
        "in.yahoo.com",
        "https://in.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://in.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_it",
        "Ricerca di Yahoo",
        "it.yahoo.com",
        "https://it.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://it.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    // Yahoo Japan's suggestions need Chrome's app id: none here
    engine(
        "yahoo_jp",
        "Yahoo! JAPAN",
        "yahoo.co.jp",
        "https://search.yahoo.co.jp/search?ei=UTF-8&p={q}",
        None,
        Regional,
    ),
    engine(
        "yahoo_se",
        "Yahoo Search",
        "se.yahoo.com",
        "https://se.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://se.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_tw",
        "Yahoo!奇摩",
        "tw.yahoo.com",
        "https://tw.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://tw.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yahoo_uk",
        "Yahoo Search",
        "uk.yahoo.com",
        "https://uk.search.yahoo.com/search?ei=UTF-8&p={q}",
        Some("https://uk.search.yahoo.com/sugg/chrome?output=fxjson&command={q}"),
        Regional,
    ),
    engine(
        "yandex_by",
        "Яндекс",
        "yandex.by",
        "https://yandex.by/search/?text={q}",
        Some("https://suggest.yandex.by/suggest-ff.cgi?part={q}"),
        Regional,
    ),
    engine(
        "yandex_kz",
        "Яндекс",
        "yandex.kz",
        "https://yandex.kz/search/?text={q}",
        Some("https://suggest.yandex.kz/suggest-ff.cgi?part={q}"),
        Regional,
    ),
    engine(
        "yandex_ru",
        "Яндекс",
        "yandex.ru",
        "https://yandex.ru/search/?text={q}",
        Some("https://suggest.yandex.ru/suggest-ff.cgi?part={q}"),
        Regional,
    ),
    engine(
        "yandex_tr",
        "Yandex",
        "yandex.com.tr",
        "https://yandex.com.tr/search/?text={q}",
        Some("https://suggest.yandex.com.tr/suggest-ff.cgi?part={q}"),
        Regional,
    ),
    // keyword engines (site search)
    engine(
        "wikipedia",
        "Wikipedia",
        "wikipedia.org",
        "https://en.wikipedia.org/wiki/Special:Search?search={q}",
        Some("https://en.wikipedia.org/w/api.php?action=opensearch&search={q}"),
        Site,
    ),
    engine(
        "youtube",
        "YouTube",
        "youtube.com",
        "https://www.youtube.com/results?search_query={q}",
        Some("https://suggestqueries.google.com/complete/search?client=firefox&ds=yt&q={q}"),
        Site,
    ),
    engine(
        "github",
        "GitHub",
        "github.com",
        "https://github.com/search?q={q}",
        None,
        Site,
    ),
];

/// The built-in engine stored as `id`.
pub fn engine_info(id: &str) -> Option<&'static EngineInfo> {
    ENGINES.iter().find(|e| e.id == id)
}

/// The region no listed country maps to: Chromium's default list.
pub const DEFAULT_REGION: &str = "ZZ";

/// Each country's engines, in the order Chrome's choice screen shows them (the first is the
/// country's default in Chrome): `regional_settings.json`'s `elements`.
#[rustfmt::skip]
pub const REGIONS: [(&str, &[&str]); 134] = [
    ("AE", &["google", "brave", "bing", "duckduckgo", "yandex_ru"]),
    ("AF", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("AL", &["google", "brave", "yep", "duckduckgo", "bing"]),
    ("AM", &["google", "yandex_ru", "brave", "bing", "duckduckgo"]),
    ("AO", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("AR", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("AT", &["google", "brave", "duckduckgo", "perplexity", "ecosia", "bing", "yahoo_at", "qwant"]),
    ("AU", &["google", "brave", "duckduckgo", "bing", "ecosia"]),
    ("AZ", &["google", "yandex_ru", "brave", "bing", "duckduckgo"]),
    ("BA", &["google", "yep", "brave", "bing", "duckduckgo"]),
    ("BD", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("BE", &["google", "brave", "duckduckgo", "ecosia", "perplexity", "bing", "qwant", "privacywall"]),
    ("BF", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("BG", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "yep", "qwant"]),
    ("BJ", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("BO", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("BR", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("BY", &["google", "yandex_by", "bing", "brave", "duckduckgo"]),
    ("CA", &["google", "brave", "duckduckgo", "bing", "ecosia"]),
    ("CD", &["google", "bing", "brave", "duckduckgo", "you"]),
    ("CG", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("CH", &["google", "brave", "duckduckgo", "bing", "ecosia"]),
    ("CI", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("CL", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("CM", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("CN", &["baidu", "bing", "sogou", "so_360", "google"]),
    ("CO", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("CR", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("CU", &["google", "brave", "duckduckgo", "bing", "yandex_com"]),
    ("CY", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "qwant", "privacywall"]),
    ("CZ", &["google", "seznam", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "privacywall"]),
    ("DE", &["google", "duckduckgo", "brave", "ecosia", "perplexity", "bing", "startpage", "yahoo_de"]),
    ("DK", &["google", "brave", "duckduckgo", "perplexity", "bing", "ecosia", "qwant", "yahoo_dk"]),
    ("DO", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("DZ", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("EC", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("EE", &["google", "brave", "duckduckgo", "bing", "perplexity", "ecosia", "privacywall", "qwant"]),
    ("EG", &["google", "brave", "bing", "duckduckgo", "yandex_com"]),
    ("ES", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "yahoo_es", "privacywall"]),
    ("ET", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("FI", &["google", "brave", "duckduckgo", "perplexity", "bing", "ecosia", "qwant", "privacywall"]),
    ("FR", &["google", "brave", "ecosia", "perplexity", "qwant", "duckduckgo", "bing", "yahoo_fr"]),
    ("GB", &["google", "duckduckgo", "brave", "bing", "ecosia"]),
    ("GE", &["google", "yandex_com", "brave", "bing", "duckduckgo"]),
    ("GH", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("GM", &["google", "bing", "duckduckgo", "brave", "ecosia"]),
    ("GN", &["google", "bing", "duckduckgo", "brave", "ecosia"]),
    ("GR", &["google", "brave", "perplexity", "duckduckgo", "bing", "yahoo_emea", "ecosia", "qwant"]),
    ("GT", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("HK", &["google", "baidu", "brave", "bing", "naver"]),
    ("HN", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("HR", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "yep", "privacywall"]),
    ("HU", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "qwant", "yep"]),
    ("ID", &["google", "brave", "bing", "duckduckgo", "naver"]),
    ("IE", &["google", "duckduckgo", "brave", "perplexity", "bing", "ecosia", "yahoo_uk", "privacywall"]),
    ("IL", &["google", "brave", "bing", "duckduckgo", "yandex_ru"]),
    ("IN", &["google", "brave", "bing", "yahoo_in", "duckduckgo"]),
    ("IQ", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("IR", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("IS", &["google", "duckduckgo", "brave", "bing", "perplexity", "ecosia", "privacywall", "qwant"]),
    ("IT", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "yahoo_it", "qwant"]),
    ("JO", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("JP", &["google", "yahoo_jp", "bing", "duckduckgo", "ecosia"]),
    ("KE", &["google", "brave", "bing", "duckduckgo", "you"]),
    ("KG", &["google", "yandex_ru", "brave", "bing", "duckduckgo"]),
    ("KH", &["google", "brave", "bing", "naver", "baidu"]),
    ("KR", &["google", "naver", "daum", "brave", "bing"]),
    ("KZ", &["google", "yandex_kz", "brave", "bing", "duckduckgo"]),
    ("LA", &["google", "brave", "bing", "naver", "baidu"]),
    ("LB", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("LI", &["google", "duckduckgo", "brave", "ecosia", "perplexity", "bing", "startpage", "qwant"]),
    ("LK", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("LT", &["google", "brave", "duckduckgo", "bing", "perplexity", "ecosia", "qwant", "yahoo_emea"]),
    ("LU", &["google", "brave", "duckduckgo", "perplexity", "bing", "ecosia", "qwant", "yahoo_emea"]),
    ("LV", &["google", "brave", "duckduckgo", "bing", "perplexity", "ecosia", "privacywall", "qwant"]),
    ("LY", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("MA", &["google", "brave", "bing", "duckduckgo", "you"]),
    ("MD", &["google", "yandex_ru", "brave", "bing", "duckduckgo"]),
    ("ME", &["google", "yep", "brave", "duckduckgo", "bing"]),
    ("MG", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("MK", &["google", "yep", "brave", "bing", "duckduckgo"]),
    ("ML", &["google", "bing", "duckduckgo", "brave", "ecosia"]),
    ("MM", &["google", "bing", "brave", "baidu", "duckduckgo"]),
    ("MN", &["google", "brave", "naver", "bing", "daum"]),
    ("MO", &["google", "baidu", "brave", "bing", "naver"]),
    ("MT", &["google", "brave", "bing", "perplexity", "duckduckgo", "ecosia", "privacywall", "qwant"]),
    ("MX", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("MY", &["google", "brave", "bing", "baidu", "duckduckgo"]),
    ("MZ", &["google", "bing", "brave", "you", "duckduckgo"]),
    ("NE", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("NG", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("NL", &["google", "duckduckgo", "brave", "perplexity", "bing", "ecosia", "privacywall", "qwant"]),
    ("NO", &["google", "duckduckgo", "brave", "bing", "perplexity", "ecosia", "yahoo_emea", "qwant"]),
    ("NP", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("NZ", &["google", "brave", "duckduckgo", "bing", "ecosia"]),
    ("PE", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("PH", &["google", "brave", "bing", "duckduckgo", "naver"]),
    ("PK", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("PL", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "privacywall", "yahoo_emea"]),
    ("PT", &["google", "brave", "perplexity", "duckduckgo", "bing", "ecosia", "qwant", "yep"]),
    ("RO", &["google", "brave", "perplexity", "duckduckgo", "bing", "yahoo_emea", "ecosia", "qwant"]),
    ("RS", &["google", "brave", "yep", "duckduckgo", "bing"]),
    ("RU", &["google", "yandex_ru", "duckduckgo", "brave", "bing"]),
    ("SA", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("SD", &["google", "brave", "duckduckgo", "bing", "ecosia"]),
    ("SE", &["google", "duckduckgo", "brave", "perplexity", "bing", "ecosia", "yahoo_se", "qwant"]),
    ("SG", &["google", "brave", "baidu", "bing", "duckduckgo"]),
    ("SI", &["google", "brave", "duckduckgo", "bing", "perplexity", "ecosia", "qwant", "yep"]),
    ("SK", &["google", "brave", "perplexity", "duckduckgo", "bing", "seznam", "ecosia", "privacywall"]),
    ("SL", &["google", "bing", "brave", "you", "duckduckgo"]),
    ("SN", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("SO", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("SV", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("TD", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("TG", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("TH", &["google", "brave", "bing", "naver", "duckduckgo"]),
    ("TJ", &["google", "yandex_ru", "brave", "bing", "duckduckgo"]),
    ("TM", &["google", "yandex_ru", "brave", "duckduckgo", "bing"]),
    ("TN", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("TR", &["google", "yandex_tr", "brave", "bing", "duckduckgo"]),
    ("TW", &["google", "brave", "baidu", "bing", "yahoo_tw"]),
    ("TZ", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("UA", &["google", "yandex_ru", "brave", "bing", "duckduckgo"]),
    ("UG", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("US", &["google", "duckduckgo", "brave", "bing", "ecosia"]),
    ("UY", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("UZ", &["google", "yandex_ru", "brave", "bing", "naver"]),
    ("VE", &["google", "bing", "brave", "you", "duckduckgo"]),
    ("VN", &["google", "coccoc", "brave", "bing", "naver"]),
    ("XK", &["google", "yep", "brave", "bing", "duckduckgo"]),
    ("YE", &["google", "bing", "brave", "duckduckgo", "ecosia"]),
    ("ZA", &["google", "brave", "bing", "duckduckgo", "ecosia"]),
    ("ZW", &["google", "bing", "brave", "you", "duckduckgo"]),
    ("ZZ", &["google", "bing", "yahoo"]),
];

/// Countries that use another's list: `regional_settings.json`'s `map_aliases`.
#[rustfmt::skip]
pub const ALIASES: [(&str, &str); 119] = [
    ("AD", "ZZ"), ("AG", "ZZ"), ("AI", "GB"), ("AQ", "ZZ"), ("AS", "US"), ("AW", "NL"),
    ("AX", "FI"), ("BB", "ZZ"), ("BH", "ZZ"), ("BI", "ZZ"), ("BL", "FR"), ("BM", "GB"),
    ("BN", "ZZ"), ("BQ", "NL"), ("BS", "ZZ"), ("BT", "ZZ"), ("BV", "NO"), ("BW", "ZZ"),
    ("BZ", "ZZ"), ("CC", "AU"), ("CF", "ZZ"), ("CK", "ZZ"), ("CQ", "GB"), ("CV", "ZZ"),
    ("CW", "NL"), ("CX", "AU"), ("DJ", "ZZ"), ("DM", "ZZ"), ("EA", "ES"), ("EH", "ZZ"),
    ("ER", "ZZ"), ("FJ", "ZZ"), ("FK", "GB"), ("FM", "ZZ"), ("FO", "DK"), ("GA", "ZZ"),
    ("GD", "ZZ"), ("GF", "FR"), ("GG", "GB"), ("GI", "GB"), ("GL", "DK"), ("GP", "FR"),
    ("GQ", "ZZ"), ("GS", "GB"), ("GU", "US"), ("GW", "ZZ"), ("GY", "ZZ"), ("HM", "AU"),
    ("HT", "ZZ"), ("IC", "ES"), ("IM", "GB"), ("IO", "GB"), ("JE", "GB"), ("JM", "ZZ"),
    ("KI", "ZZ"), ("KM", "ZZ"), ("KN", "ZZ"), ("KP", "ZZ"), ("KW", "ZZ"), ("KY", "GB"),
    ("LC", "ZZ"), ("LR", "ZZ"), ("LS", "ZZ"), ("MC", "ZZ"), ("MF", "FR"), ("MH", "ZZ"),
    ("MP", "US"), ("MQ", "FR"), ("MR", "ZZ"), ("MS", "GB"), ("MU", "ZZ"), ("MV", "ZZ"),
    ("MW", "ZZ"), ("NA", "ZZ"), ("NC", "FR"), ("NF", "AU"), ("NI", "ZZ"), ("NR", "ZZ"),
    ("NU", "ZZ"), ("OM", "ZZ"), ("PA", "ZZ"), ("PF", "FR"), ("PG", "ZZ"), ("PM", "FR"),
    ("PN", "GB"), ("PR", "US"), ("PS", "ZZ"), ("PW", "ZZ"), ("PY", "ZZ"), ("QA", "ZZ"),
    ("RE", "FR"), ("RW", "ZZ"), ("SB", "ZZ"), ("SC", "ZZ"), ("SH", "GB"), ("SJ", "NO"),
    ("SM", "IT"), ("SR", "ZZ"), ("ST", "ZZ"), ("SX", "NL"), ("SY", "ZZ"), ("SZ", "ZZ"),
    ("TC", "GB"), ("TF", "FR"), ("TK", "NZ"), ("TL", "ZZ"), ("TO", "ZZ"), ("TT", "ZZ"),
    ("TV", "ZZ"), ("UM", "US"), ("VA", "IT"), ("VC", "ZZ"), ("VG", "GB"), ("VI", "US"),
    ("VU", "ZZ"), ("WF", "FR"), ("WS", "ZZ"), ("YT", "FR"), ("ZM", "ZZ"),
];

/// The engines offered in `country` (an ISO 3166 code, any case), by id: its own list, the
/// list of the country it is an alias of, or [`DEFAULT_REGION`]'s.
pub fn region(country: &str) -> &'static [&'static str] {
    let country = country.to_ascii_uppercase();
    let code = ALIASES
        .iter()
        .find(|(c, _)| *c == country)
        .map_or(country.as_str(), |(_, to)| to);
    let list = |code: &str| REGIONS.iter().find(|(c, _)| *c == code).map(|(_, l)| *l);
    list(code)
        .or_else(|| list(DEFAULT_REGION))
        .unwrap_or_default()
}

/// The country of a POSIX locale (`en_US.UTF-8`, `pt_BR`, `de_DE@euro`): its territory, upper
/// case; `None` without one (`C`, `en`).
pub fn country_of_locale(locale: &str) -> Option<String> {
    let base = locale.split(['.', '@']).next()?;
    let (_, territory) = base.split_once(['_', '-'])?;
    (territory.len() == 2 && territory.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| territory.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_table() {
        for (i, e) in ENGINES.iter().enumerate() {
            assert_eq!(engine_info(e.id), Some(e));
            assert!(ENGINES[..i].iter().all(|o| o.id != e.id), "dup id {}", e.id);
            assert!(
                ENGINES[..i].iter().all(|o| o.keyword != e.keyword),
                "dup keyword {}",
                e.keyword
            );
            assert!(!e.keyword.contains(char::is_whitespace), "{}", e.id);
            let templates = [Some(e.search), e.suggest, e.image];
            for t in templates.into_iter().flatten() {
                assert_eq!(t.matches("{q}").count(), 1, "{}: {t}", e.id);
                assert!(t.starts_with("https://"), "{}: {t}", e.id);
                assert!(!t.contains("{google:") && !t.contains("{yandex:"), "{}", e.id);
            }
        }
        // Everywhere first: the Select's order after a region's own
        let first_other = ENGINES.iter().position(|e| e.reach != Everywhere).unwrap();
        assert!(ENGINES[first_other..].iter().all(|e| e.reach != Everywhere));
        // the shipped ids never change
        assert_eq!(engine_info("google").map(|e| e.name), Some("Google"));
        assert_eq!(engine_info("duckduckgo").map(|e| e.name), Some("DuckDuckGo"));
        assert_eq!(engine_info(""), None);
        assert_eq!(engine_info("altavista"), None);
    }

    #[test]
    fn the_engines_wubble_names() {
        let cases = [
            ("google", Everywhere),
            ("bing", Everywhere),
            ("duckduckgo", Everywhere),
            ("kagi", Everywhere),
            ("brave", Everywhere),
            ("startpage", Everywhere),
            ("ecosia", Everywhere),
            ("qwant", Everywhere),
            ("yahoo", Everywhere),
            ("yandex_com", Everywhere),
            ("baidu", Everywhere),
            ("naver", Everywhere),
            ("perplexity", Everywhere),
            ("wikipedia", Site),
            ("youtube", Site),
            ("github", Site),
            ("yahoo_de", Regional),
            ("seznam", Regional),
        ];
        for (id, reach) in cases {
            assert_eq!(engine_info(id).map(|e| e.reach), Some(reach), "{id}");
        }
    }

    #[test]
    fn regions_name_known_engines_never_a_site_search() {
        for (country, list) in &REGIONS {
            assert!(!list.is_empty(), "{country}");
            for id in *list {
                let e = engine_info(id).unwrap_or_else(|| panic!("{country}: {id}"));
                assert_ne!(e.reach, Site, "{country}: {id}");
            }
            assert!(REGIONS.iter().filter(|(c, _)| c == country).count() == 1);
        }
        for (country, to) in &ALIASES {
            assert!(REGIONS.iter().any(|(c, _)| c == to), "{country} -> {to}");
            assert!(REGIONS.iter().all(|(c, _)| c != country), "{country} is both");
        }
    }

    #[test]
    fn a_country_gets_its_list() {
        let cases: [(&str, &[&str]); 7] = [
            ("US", &["google", "duckduckgo", "brave", "bing", "ecosia"]),
            ("us", &["google", "duckduckgo", "brave", "bing", "ecosia"]),
            (
                "DE",
                &["google", "duckduckgo", "brave", "ecosia", "perplexity", "bing", "startpage", "yahoo_de"],
            ),
            // an alias: Puerto Rico uses the US list
            ("PR", &["google", "duckduckgo", "brave", "bing", "ecosia"]),
            // an alias of the default list, an unknown code and none at all
            ("AD", &["google", "bing", "yahoo"]),
            ("XX", &["google", "bing", "yahoo"]),
            ("", &["google", "bing", "yahoo"]),
        ];
        for (country, want) in cases {
            assert_eq!(region(country), want, "{country}");
        }
        assert!(region("KR").contains(&"naver"));
        assert!(region("RU").contains(&"yandex_ru"));
        assert!(region("CN").contains(&"baidu"));
    }

    #[test]
    fn locales_name_their_country() {
        let cases = [
            ("en_US.UTF-8", Some("US")),
            ("pt_BR", Some("BR")),
            ("de_DE@euro", Some("DE")),
            ("sr_RS.UTF-8@latin", Some("RS")),
            ("en-gb", Some("GB")),
            ("C", None),
            ("C.UTF-8", None),
            ("POSIX", None),
            ("en", None),
            ("", None),
            ("zh_Hans_CN", None),
        ];
        for (locale, want) in cases {
            assert_eq!(country_of_locale(locale).as_deref(), want, "{locale}");
        }
    }
}
