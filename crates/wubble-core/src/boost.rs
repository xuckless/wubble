//! Boosts: the user's own CSS, JavaScript, tint and font for a site, as data. A [`Boost`] names
//! its site with a [`Pattern`]; [`matching`] picks the Boosts for a page in the order they apply,
//! and [`css_of`] / [`page_scripts`] turn them into what the page is given. Matching happens here
//! only: the page scripts carry the URL they were matched for and run on no other document.
//!
//! The pattern grammar is a subset of Chrome's extension match patterns
//! (`scheme://host/path`), design reference only: Chromium `extensions/common/url_pattern.cc`
//! (BSD-3-Clause). A bare host (`example.com`) means that host and its subdomains, as Stylus's
//! "URLs on the domain".

use std::cmp::Ordering;

/// When a Boost's script runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum At {
    /// Before the page's own scripts (document start).
    #[default]
    Start,
    /// Once the document is parsed (`DOMContentLoaded`).
    End,
}

impl At {
    pub fn index(self) -> i64 {
        match self {
            At::Start => 0,
            At::End => 1,
        }
    }

    pub fn from_index(i: i64) -> Self {
        if i == 1 { At::End } else { At::Start }
    }
}

/// A colour laid over the page: `rgb` as `0xRRGGBB`, `strength` 0 to 100 percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tint {
    pub rgb: u32,
    pub strength: u8,
}

/// One Boost, as stored (`wubble-store` `boosts`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boost {
    /// The store's row id; 0 for one not stored yet.
    pub id: i64,
    pub name: String,
    /// A [`Pattern`]'s text.
    pub site: String,
    pub css: String,
    pub js: String,
    pub js_at: At,
    pub tint: Option<Tint>,
    /// A font family every element of the page is shown in.
    pub font: Option<String>,
    pub enabled: bool,
    /// The user's order among Boosts of the same specificity.
    pub order: i64,
}

impl Boost {
    /// A new, empty Boost for `host`'s pages.
    pub fn for_host(host: &str) -> Self {
        Self {
            id: 0,
            name: host.to_owned(),
            site: host.to_owned(),
            css: String::new(),
            js: String::new(),
            js_at: At::Start,
            tint: None,
            font: None,
            enabled: true,
            order: 0,
        }
    }
}

/// Which schemes a pattern accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    /// `*`: http and https.
    Web,
    Http,
    Https,
}

/// Which hosts a pattern accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Host {
    /// `*`.
    Any,
    /// This host and its subdomains (`*.x.y`, or a bare `x.y`).
    Domain(String),
    /// This host only (`scheme://x.y/...`).
    Exact(String),
}

/// A parsed site pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    scheme: Scheme,
    host: Host,
    /// A glob over the path and query; `*` is any run of characters.
    path: String,
}

impl Pattern {
    /// `example.com`, `*.example.com`, `https://example.com/docs/*`, `*://*/*`. `None` for
    /// anything else (an unknown scheme, a `*` inside a host, spaces, an empty text).
    pub fn parse(text: &str) -> Option<Pattern> {
        let text = text.trim();
        if text.is_empty() || text.chars().any(char::is_whitespace) {
            return None;
        }
        let Some((scheme, rest)) = text.split_once("://") else {
            // A bare host (or `*.host`): that site, every scheme, every path.
            let host = text.strip_prefix("*.").unwrap_or(text);
            return Some(Pattern {
                scheme: Scheme::Web,
                host: Host::Domain(ascii_host(host)?),
                path: "*".into(),
            });
        };
        let scheme = match scheme {
            "*" => Scheme::Web,
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return None,
        };
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/*"),
        };
        let host = match host {
            "*" => Host::Any,
            h => match h.strip_prefix("*.") {
                Some(domain) => Host::Domain(ascii_host(domain)?),
                None => Host::Exact(ascii_host(h)?),
            },
        };
        Some(Pattern {
            scheme,
            host,
            path: path.to_owned(),
        })
    }

    /// Whether the page at `url` is one of the pattern's.
    pub fn matches(&self, url: &str) -> bool {
        let Ok(url) = url::Url::parse(url) else {
            return false;
        };
        let scheme_ok = match self.scheme {
            Scheme::Web => matches!(url.scheme(), "http" | "https"),
            Scheme::Http => url.scheme() == "http",
            Scheme::Https => url.scheme() == "https",
        };
        let Some(host) = url.host_str() else {
            return false;
        };
        let host_ok = match &self.host {
            Host::Any => true,
            Host::Exact(h) => host == h,
            Host::Domain(d) => {
                host == d
                    || (host.ends_with(d.as_str()) && host[..host.len() - d.len()].ends_with('.'))
            }
        };
        let mut path = url.path().to_owned();
        if let Some(q) = url.query() {
            path.push('?');
            path.push_str(q);
        }
        scheme_ok && host_ok && glob(&self.path, &path)
    }

    /// How narrow the pattern is: an exact host over a domain over any host, a longer host and
    /// then a longer literal path narrower.
    fn specificity(&self) -> (u8, usize, usize) {
        let (class, len) = match &self.host {
            Host::Any => (0, 0),
            Host::Domain(d) => (1, d.len()),
            Host::Exact(h) => (2, h.len()),
        };
        (class, len, self.path.chars().filter(|&c| c != '*').count())
    }
}

/// A host as the `url` crate writes it (lowercase, punycode), refusing wildcards and ports.
fn ascii_host(host: &str) -> Option<String> {
    if host.is_empty() || host.contains(['*', ':', '/', '@']) {
        return None;
    }
    let url = url::Url::parse(&format!("http://{host}/")).ok()?;
    let parsed = url.host_str()?;
    (!parsed.is_empty() && !parsed.starts_with('.') && !parsed.ends_with('.'))
        .then(|| parsed.to_owned())
}

/// Whether `text` matches `pattern`, where `*` stands for any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    // The classic two-pointer match with one backtrack point.
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < t.len() {
        if i < p.len() && p[i] != '*' && p[i] == t[j] {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == '*')
}

/// The enabled Boosts for the page at `url`, in the order they apply: broader patterns first, so
/// a narrower one's CSS wins; then the user's order, then the oldest first.
pub fn matching<'a>(boosts: &'a [Boost], url: &str) -> Vec<&'a Boost> {
    let mut hits: Vec<(&Boost, Pattern)> = boosts
        .iter()
        .filter(|b| b.enabled)
        .filter_map(|b| Pattern::parse(&b.site).map(|p| (b, p)))
        .filter(|(_, p)| p.matches(url))
        .collect();
    hits.sort_by(|(a, pa), (b, pb)| {
        pa.specificity()
            .cmp(&pb.specificity())
            .then(a.order.cmp(&b.order))
            .then(a.id.cmp(&b.id))
    });
    hits.into_iter().map(|(b, _)| b).collect()
}

/// The CSS a Boost gives its pages: the user's, then the tint over everything, then the font.
pub fn css_of(boost: &Boost) -> String {
    let mut css = boost.css.trim().to_owned();
    if let Some(tint) = boost.tint.filter(|t| t.strength > 0) {
        let [_, r, g, b] = tint.rgb.to_be_bytes();
        let alpha = f32::from(tint.strength.min(100)) / 100.0;
        css.push_str(&format!(
            "\nhtml::after{{content:\"\";position:fixed;inset:0;pointer-events:none;\
             z-index:2147483647;background:rgba({r},{g},{b},{alpha});mix-blend-mode:multiply}}"
        ));
    }
    if let Some(font) = boost
        .font
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        css.push_str(&format!(
            "\n*{{font-family:{}!important}}",
            css_string(font)
        ));
    }
    css
}

/// `text` as a CSS string: quotes and backslashes escaped, line breaks as escapes, so a font
/// name cannot end the rule.
fn css_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' | '\u{c}' => out.push_str("\\a "),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JavaScript string literal of `text` (JSON is a subset).
fn js_string(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

/// `url` as the page script compares it with `location.href`: parsed, without its fragment.
pub fn document_key(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            u.set_fragment(None);
            u.into()
        }
        Err(_) => url.split('#').next().unwrap_or_default().to_owned(),
    }
}

/// The two scripts that apply `boosts` (from [`matching`]) to the document at `url`, after the
/// page kit (`kit.js`) defined `window[Symbol.for("wubble.kit")]`:
///
/// 1. the CSS, as data: one adopted style sheet, which no `style-src` policy blocks;
/// 2. the scripts, each in its own `try`, at document start or at `DOMContentLoaded`.
///
/// Each runs once per document (a flag on the kit) and only on the document at `url`, so a
/// script registered for one page never touches another, and running it again is harmless.
/// They are two scripts so a syntax error in a user's script cannot cost the page its CSS.
pub fn page_scripts(boosts: &[&Boost], url: &str) -> [String; 2] {
    let key = js_string(&document_key(url));
    let css: Vec<String> = boosts.iter().map(|b| css_of(b)).collect();
    let css = js_string(&css.join("\n"));
    let open =
        |flag: &str| format!("(function(k){{if(!k||k.{flag}||!k.at({key}))return;k.{flag}=true;\n");
    let close = "\n})(window[Symbol.for('wubble.kit')]);";
    let styles = format!("{}k.css({css});{close}", open("boostedCss"));
    let mut scripts = open("boostedJs");
    for b in boosts.iter().filter(|b| !b.js.trim().is_empty()) {
        let body = format!(
            "try{{(function(){{\n{}\n}})();}}catch(e){{console.error('wubble boost {}:',e);}}",
            b.js, b.id
        );
        match b.js_at {
            At::Start => scripts.push_str(&body),
            At::End => scripts.push_str(&format!("k.ready(function(){{{body}}});")),
        }
        scripts.push('\n');
    }
    scripts.push_str(close);
    [styles, scripts]
}

/// Orders by `order`, then id: the editor's list.
pub fn by_order(a: &Boost, b: &Boost) -> Ordering {
    a.order.cmp(&b.order).then(a.id.cmp(&b.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boost(id: i64, site: &str, order: i64) -> Boost {
        Boost {
            id,
            order,
            ..Boost::for_host(site)
        }
    }

    #[test]
    fn patterns_match_their_pages() {
        let cases: &[(&str, &str, bool)] = &[
            ("example.com", "https://example.com/", true),
            ("example.com", "http://example.com/a?b=1", true),
            ("example.com", "https://www.example.com/x", true),
            ("example.com", "https://example.com.evil.org/", false),
            ("example.com", "https://notexample.com/", false),
            ("example.com", "ftp://example.com/", false),
            ("Example.COM", "https://example.com/", true),
            ("*.example.com", "https://a.b.example.com/", true),
            ("*.example.com", "https://example.com/", true),
            ("https://*.example.com/*", "http://example.com/", false),
            ("https://example.com/*", "https://example.com/x", true),
            ("https://example.com/*", "https://www.example.com/x", false),
            (
                "http://example.com/docs/*",
                "http://example.com/docs/a/b",
                true,
            ),
            (
                "http://example.com/docs/*",
                "http://example.com/blog",
                false,
            ),
            (
                "*://example.com/*?q=*",
                "https://example.com/s?q=cats",
                true,
            ),
            ("*://*/*", "https://anything.test/x", true),
            ("*://*/*", "about:blank", false),
            ("https://example.com", "https://example.com/deep/page", true),
            // Ports belong to the page, not the pattern.
            ("localhost", "http://localhost:8080/", true),
            // Internationalised names compare as punycode.
            ("bücher.de", "https://xn--bcher-kva.de/", true),
            ("bücher.de", "https://bücher.de/", true),
        ];
        for &(pattern, url, want) in cases {
            let p = Pattern::parse(pattern).unwrap_or_else(|| panic!("{pattern} parses"));
            assert_eq!(p.matches(url), want, "{pattern} vs {url}");
        }
    }

    #[test]
    fn bad_patterns_are_refused() {
        for bad in [
            "",
            "   ",
            "a b.com",
            "ftp://example.com/*",
            "http://*example.com/*",
            "http://example.*/*",
            "http://ex*ample.com/",
            "*.",
            "http:///path",
            "http://example.com:8080/*",
            "user@example.com",
        ] {
            assert_eq!(Pattern::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn globs() {
        assert!(glob("*", ""));
        assert!(glob("/a*c", "/abbbc"));
        assert!(glob("/a*b*c", "/aXbYc"));
        assert!(!glob("/a*b", "/acc"));
        assert!(glob("/a", "/a"));
        assert!(!glob("/a", "/ab"));
    }

    #[test]
    fn broader_boosts_apply_first_then_order_then_age() {
        let boosts = vec![
            boost(1, "https://www.example.com/docs/*", 0),
            boost(2, "*://*/*", 0),
            boost(3, "example.com", 5),
            boost(4, "example.com", 1),
            boost(5, "www.example.com", 1),
            boost(6, "other.test", 0),
            Boost {
                enabled: false,
                ..boost(7, "example.com", 0)
            },
            boost(8, "not a pattern", 0),
            boost(9, "example.com", 1),
            boost(10, "https://www.example.com/*", 0),
        ];
        let ids: Vec<i64> = matching(&boosts, "https://www.example.com/docs/x")
            .iter()
            .map(|b| b.id)
            .collect();
        assert_eq!(ids, vec![2, 4, 9, 3, 5, 10, 1]);
    }

    #[test]
    fn tint_and_font_are_css_after_the_users() {
        let b = Boost {
            css: " p{color:red} ".into(),
            tint: Some(Tint {
                rgb: 0x10_20_30,
                strength: 25,
            }),
            font: Some("Ev\"il}; \\ Font\n".into()),
            ..Boost::for_host("a.test")
        };
        let css = css_of(&b);
        assert!(css.starts_with("p{color:red}\nhtml::after{"), "{css}");
        assert!(css.contains("background:rgba(16,32,48,0.25)"), "{css}");
        assert!(css.contains("mix-blend-mode:multiply"));
        assert!(
            css.ends_with("*{font-family:\"Ev\\\"il}; \\\\ Font\"!important}"),
            "{css}"
        );
        // Nothing to add: only the user's CSS.
        let plain = Boost {
            css: "a{}".into(),
            tint: Some(Tint {
                rgb: 0,
                strength: 0,
            }),
            font: Some("  ".into()),
            ..Boost::for_host("a.test")
        };
        assert_eq!(css_of(&plain), "a{}");
    }

    #[test]
    fn page_scripts_carry_their_document_and_escape_sources() {
        let a = Boost {
            id: 3,
            css: "body{background:rgb(1,2,3)}".into(),
            js: "document.title = \"it's </script>\";".into(),
            ..Boost::for_host("a.test")
        };
        let b = Boost {
            id: 4,
            css: "p{}".into(),
            js: "go()".into(),
            js_at: At::End,
            ..Boost::for_host("a.test")
        };
        let empty = Boost {
            id: 5,
            ..Boost::for_host("a.test")
        };
        let [css, js] = page_scripts(&[&a, &b, &empty], "https://a.test/p?q#frag");
        assert!(css.contains("k.at(\"https://a.test/p?q\")"), "{css}");
        assert!(css.contains("k.boostedCss"));
        assert!(
            css.contains(r#"k.css("body{background:rgb(1,2,3)}\np{}\n")"#),
            "{css}"
        );
        assert!(js.contains("k.boostedJs"));
        assert!(js.contains("document.title = \"it's </script>\";"));
        assert!(js.contains("k.ready(function(){try{(function(){\ngo()\n})();}"));
        assert!(js.contains("wubble boost 3"));
        assert!(
            !js.contains("wubble boost 5"),
            "a Boost without a script adds none"
        );
        // Nothing matches: the scripts still mark the document, with nothing to apply.
        let [css, js] = page_scripts(&[], "https://a.test/");
        assert!(css.contains("k.css(\"\")") && js.contains("k.boostedJs=true;"));
    }

    #[test]
    fn document_keys_drop_the_fragment_and_normalise() {
        assert_eq!(document_key("https://A.test/x#y"), "https://a.test/x");
        assert_eq!(document_key("https://a.test"), "https://a.test/");
        assert_eq!(document_key("not a url#x"), "not a url");
    }

    #[test]
    fn run_at_round_trips_its_index() {
        for at in [At::Start, At::End] {
            assert_eq!(At::from_index(at.index()), at);
        }
        assert_eq!(At::from_index(7), At::Start);
    }
}
