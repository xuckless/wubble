//! Boosts: the user's own CSS, JavaScript, tint and font for a site, as data. A [`Boost`] names
//! its site with a [`Pattern`]; [`matching`] picks the Boosts for a page in the order they apply,
//! and [`css_of`] / [`page_scripts`] turn them into what the page is given. The page scripts are
//! the same for every document: they carry each Boost with its pattern as data ([`Pattern::js`]),
//! and the page kit's `matches` (`kit.js`, the same rules as [`Pattern::matches`]) picks the
//! Boosts of the document it runs in, so they run at document start whichever way the page was
//! reached (a link, a redirect, a script).
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
    /// A new, empty Boost for `host`'s pages; a port (`localhost:8080`) is the page's, not the
    /// pattern's, and is dropped.
    pub fn for_host(host: &str) -> Self {
        let host = match host.rsplit_once(':') {
            Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
                name
            }
            _ => host,
        };
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

    /// Whether a cookie (`domain` as CEF's cookie manager names it: a leading dot for a domain
    /// cookie, none for a host-only one; `path`; `secure`) would be sent to some page of this
    /// pattern (RFC 6265 5.1.3 domain-match, 5.1.4 path-match; a secure cookie never over
    /// http). Any host (`*://*/*`) answers no: it names no site. The agent sheet's "Use my
    /// sign-ins" copies exactly these (`agent::shares_cookie`).
    pub fn sends_cookie(&self, domain: &str, path: &str, secure: bool) -> bool {
        if secure && self.scheme == Scheme::Http {
            return false;
        }
        let (host, domain_cookie) = match domain.strip_prefix('.') {
            Some(h) => (h.to_ascii_lowercase(), true),
            None => (domain.to_ascii_lowercase(), false),
        };
        if host.is_empty() {
            return false;
        }
        let under = |sub: &str, parent: &str| {
            sub == parent
                || (sub.ends_with(parent) && sub[..sub.len() - parent.len()].ends_with('.'))
        };
        let host_ok = match &self.host {
            Host::Any => false,
            // a page of `h` is sent a host-only cookie of `h`, and a domain cookie of `h` or a
            // parent of it
            Host::Exact(h) => *h == host || (domain_cookie && under(h, &host)),
            // pages of `d` and its subdomains: also a cookie of a subdomain of `d`
            Host::Domain(d) => under(&host, d) || (domain_cookie && under(d, &host)),
        };
        host_ok && self.sends_path(if path.is_empty() { "/" } else { path })
    }

    /// Whether some page of the pattern path-matches a cookie path `c` (RFC 6265 5.1.4: the
    /// page's path is `c`, or starts with `c` where `c` ends in `/` or the next character is
    /// `/`). Pages of the pattern start with its literal part (before any `*`); with a `*` they
    /// may go on.
    fn sends_path(&self, c: &str) -> bool {
        let literal = self.path.split('*').next().unwrap_or("");
        let open = self.path.contains('*');
        // a page under `c` itself (`c`, or `c/...`, when the pattern allows it)
        if glob(&self.path, c) || glob(&self.path, &format!("{}/", c.trim_end_matches('/'))) {
            return true;
        }
        // `c` above every page of the pattern: a path-prefix of its literal start
        literal.starts_with(c)
            && (c.ends_with('/')
                || literal.as_bytes().get(c.len()) == Some(&b'/')
                || (literal.len() == c.len() && open))
    }

    /// Whether the pattern names a site sign-ins can be shared for: an IP address, or a host
    /// with a registrable domain (not a public suffix such as `com`, `co.uk` or `github.io`,
    /// nor a single label): sharing `*.co.uk` would carry every cookie under it.
    pub fn names_a_site(&self) -> bool {
        let host = match &self.host {
            Host::Any => return false,
            Host::Exact(h) | Host::Domain(h) => h,
        };
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        bare.parse::<std::net::IpAddr>().is_ok() || psl::domain_str(bare).is_some()
    }

    /// The pattern as data for the page kit's `matches` (`kit.js`): `s` the scheme (`web` for
    /// http and https), `h` the kind of host (`any`, `domain`, `exact`) and `n` its name, `p` the
    /// glob over path and query.
    pub fn js(&self) -> serde_json::Value {
        let scheme = match self.scheme {
            Scheme::Web => "web",
            Scheme::Http => "http",
            Scheme::Https => "https",
        };
        let (kind, name) = match &self.host {
            Host::Any => ("any", ""),
            Host::Domain(d) => ("domain", d.as_str()),
            Host::Exact(h) => ("exact", h.as_str()),
        };
        serde_json::json!({ "s": scheme, "h": kind, "n": name, "p": self.path })
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
    if host.is_empty() || host.contains(['*', ':', '/', '@', '?', '#']) {
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
    ordered(boosts)
        .into_iter()
        .filter(|(_, p)| p.matches(url))
        .map(|(b, _)| b)
        .collect()
}

/// Every enabled Boost with a pattern, in the order they apply on a page they all match.
fn ordered(boosts: &[Boost]) -> Vec<(&Boost, Pattern)> {
    let mut all: Vec<(&Boost, Pattern)> = boosts
        .iter()
        .filter(|b| b.enabled)
        .filter_map(|b| Pattern::parse(&b.site).map(|p| (b, p)))
        .collect();
    all.sort_by(|(a, pa), (b, pb)| {
        pa.specificity()
            .cmp(&pb.specificity())
            .then(a.order.cmp(&b.order))
            .then(a.id.cmp(&b.id))
    });
    all
}

/// The rule the zap picker adds for `selector` (`kit.js` `zap`), if it is a plain selector of
/// the kind the picker makes (`#id`, `tag.class`, `tag:nth-child(n)`, ` > `, CSS escapes). The
/// selector comes from the page, which can answer anything: braces, `;`, `@`, quotes or a
/// comment could close the rule and smuggle in declarations (a `url()` that tracks), so any
/// other character refuses it.
pub fn zap_rule(selector: &str) -> Option<String> {
    let selector = selector.trim();
    let plain = |c: char| c.is_ascii_alphanumeric() || " #.>:()-_\\".contains(c) || !c.is_ascii();
    (!selector.is_empty() && selector.len() < 1_000 && selector.chars().all(plain))
        .then(|| format!("{selector} {{ display: none !important; }}\n"))
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

/// The two scripts that apply `boosts` (every stored one) to whichever document they run in,
/// after the page kit (`kit.js`) defined `window[Symbol.for("wubble.kit")]`:
///
/// 1. the CSS, as data: each enabled Boost's with its pattern, in [`matching`]'s order; the ones
///    `k.matches` the document go into one adopted style sheet, which no `style-src` policy
///    blocks;
/// 2. the scripts, each behind its pattern and in its own `try`, at document start or at
///    `DOMContentLoaded`.
///
/// Each runs once per document (a flag on the kit), so running it again is harmless. They are two
/// scripts so a syntax error in a user's script cannot cost the page its CSS.
pub fn page_scripts(boosts: &[Boost]) -> [String; 2] {
    let all = ordered(boosts);
    let open = |flag: &str| format!("(function(k){{if(!k||k.{flag})return;k.{flag}=true;\n");
    let close = "\n})(window[Symbol.for('wubble.kit')]);";
    let rows = serde_json::Value::Array(
        all.iter()
            .map(|(b, p)| serde_json::json!([p.js(), css_of(b)]))
            .collect(),
    );
    let styles = format!(
        "{}k.css({rows}.filter(function(r){{return k.matches(r[0]);}})\
         .map(function(r){{return r[1];}}).join('\\n'));{close}",
        open("boostedCss")
    );
    let mut scripts = open("boostedJs");
    for (b, p) in all.iter().filter(|(b, _)| !b.js.trim().is_empty()) {
        let body = format!(
            "try{{(function(){{\n{}\n}})();}}catch(e){{console.error('wubble boost {}:',e);}}",
            b.js, b.id
        );
        let body = match b.js_at {
            At::Start => body,
            At::End => format!("k.ready(function(){{{body}}});"),
        };
        scripts.push_str(&format!("if(k.matches({})){{{body}}}\n", p.js()));
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
    fn a_pattern_says_which_cookies_its_pages_are_sent() {
        let p = |s: &str| Pattern::parse(s).unwrap();
        let bare = p("github.com");
        // host-only cookies of the site and its subdomains; a parent's domain cookie
        assert!(bare.sends_cookie("github.com", "/", true));
        assert!(bare.sends_cookie("gist.github.com", "/", false));
        assert!(bare.sends_cookie(".github.com", "/", true));
        assert!(!bare.sends_cookie("github.io", "/", false));
        assert!(!bare.sends_cookie("notgithub.com", "/", false));
        // an exact host is sent its own cookies and its parents' domain cookies only
        let exact = p("https://accounts.google.com/*");
        assert!(exact.sends_cookie("accounts.google.com", "/", true));
        assert!(exact.sends_cookie(".google.com", "/", true));
        assert!(
            !exact.sends_cookie("google.com", "/", true),
            "host-only of the parent"
        );
        assert!(!exact.sends_cookie("mail.google.com", "/", true));
        assert!(!exact.sends_cookie(".mail.google.com", "/", true));
        // a secure cookie never goes over http
        assert!(!p("http://a.test/*").sends_cookie("a.test", "/", true));
        assert!(p("http://a.test/*").sends_cookie("a.test", "/", false));
        // paths: a cookie under the pattern's pages, or above them
        let docs = p("https://a.test/docs/*");
        assert!(docs.sends_cookie("a.test", "/", false));
        assert!(docs.sends_cookie("a.test", "/docs", false));
        assert!(docs.sends_cookie("a.test", "/docs/x", false));
        assert!(!docs.sends_cookie("a.test", "/admin", false));
        // RFC 6265 5.1.4: `/d` is not a path-prefix of `/docs/...` (no `/` after it)
        assert!(!docs.sends_cookie("a.test", "/d", false));
        assert!(!docs.sends_cookie("a.test", "/doc", false));
        assert!(docs.sends_cookie("a.test", "/docs/", false));
        let exact_path = p("https://a.test/docs");
        assert!(exact_path.sends_cookie("a.test", "/docs", false));
        assert!(!exact_path.sends_cookie("a.test", "/do", false));
        assert!(!exact_path.sends_cookie("a.test", "/docs/x", false));
        // any host is no site; an empty domain is nothing
        assert!(!p("*://*/*").sends_cookie("a.test", "/", false));
        assert!(!bare.sends_cookie("", "/", false));
        assert!(!bare.sends_cookie(".", "/", false));
    }

    #[test]
    fn only_a_pattern_of_a_registrable_site_names_one() {
        let p = |s: &str| Pattern::parse(s).unwrap();
        for site in [
            "github.com",
            "*.github.com",
            "a.github.io",
            "example.co.uk",
            "127.0.0.1",
            "https://accounts.google.com/*",
            "signin.localhost",
        ] {
            assert!(p(site).names_a_site(), "{site}");
        }
        for suffix in [
            "com",
            "*.co.uk",
            "co.uk",
            "github.io",
            "*.github.io",
            "localhost",
            "*://*/*",
        ] {
            assert!(!p(suffix).names_a_site(), "{suffix}");
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
    fn page_scripts_carry_every_boost_behind_its_pattern() {
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
            ..Boost::for_host("https://b.test/x/*")
        };
        let empty = Boost {
            id: 5,
            ..Boost::for_host("a.test")
        };
        let off = Boost {
            id: 6,
            js: "never()".into(),
            enabled: false,
            ..Boost::for_host("a.test")
        };
        let [css, js] = page_scripts(&[b.clone(), a.clone(), empty, off]);
        // No document's URL in them: one pair serves every navigation, however it began.
        assert!(css.contains("k.boostedCss") && !css.contains("k.at("));
        // Broader first: the domain Boost's rows before the exact host's.
        let rows = css.find(
            r#"[{"h":"domain","n":"a.test","p":"*","s":"web"},"body{background:rgb(1,2,3)}"]"#,
        );
        let later = css.find(r#"[{"h":"exact","n":"b.test","p":"/x/*","s":"https"},"p{}"]"#);
        assert!(rows.is_some() && later.is_some() && rows < later, "{css}");
        assert!(css.contains("k.matches(r[0])"));
        assert!(js.contains("k.boostedJs"));
        assert!(
            js.contains(r#"if(k.matches({"h":"domain","n":"a.test","p":"*","s":"web"})){try{"#)
        );
        assert!(js.contains("document.title = \"it's </script>\";"));
        assert!(js.contains(r#"if(k.matches({"h":"exact","n":"b.test","p":"/x/*","s":"https"})){k.ready(function(){try{(function(){
go()
})();}"#), "{js}");
        assert!(js.contains("wubble boost 3"));
        assert!(
            !js.contains("wubble boost 5"),
            "a Boost without a script adds none"
        );
        assert!(!js.contains("never()"), "a Boost turned off adds nothing");
        // No Boosts: the scripts still mark the document, with nothing to apply.
        let [css, js] = page_scripts(&[]);
        assert!(css.contains("k.css([].filter") && js.contains("k.boostedJs=true;"));
    }

    #[test]
    fn patterns_reach_the_kit_as_data() {
        let js = |t: &str| Pattern::parse(t).map(|p| p.js().to_string());
        assert_eq!(
            js("*://*/*").as_deref(),
            Some(r#"{"h":"any","n":"","p":"/*","s":"web"}"#)
        );
        assert_eq!(
            js("http://Bücher.de").as_deref(),
            Some(r#"{"h":"exact","n":"xn--bcher-kva.de","p":"/*","s":"http"}"#)
        );
        assert_eq!(
            js("*.example.com").as_deref(),
            Some(r#"{"h":"domain","n":"example.com","p":"*","s":"web"}"#)
        );
    }

    #[test]
    fn a_new_boost_for_a_ported_host_is_a_valid_pattern() {
        for (host, site) in [
            ("localhost:8080", "localhost"),
            ("127.0.0.1:3000", "127.0.0.1"),
            ("example.com", "example.com"),
        ] {
            let b = Boost::for_host(host);
            assert_eq!(b.site, site);
            let p = Pattern::parse(&b.site).unwrap_or_else(|| panic!("{host}"));
            assert!(p.matches(&format!("http://{host}/x")), "{host}");
        }
    }

    #[test]
    fn zapped_selectors_must_be_plain() {
        assert_eq!(
            zap_rule("div.ad > p:nth-child(2)").as_deref(),
            Some("div.ad > p:nth-child(2) { display: none !important; }\n")
        );
        assert!(
            zap_rule(r"#\31 23 > span.a\:b").is_some(),
            "CSS.escape output"
        );
        assert!(zap_rule("div.größe").is_some());
        for bad in [
            "",
            "   ",
            "x{} * {background:url(https://t.example/?a)}",
            "x;color:red",
            "@import url(x)",
            "a[href=\"x\"]",
            "a /* rest",
            "a\n{}",
        ] {
            assert_eq!(zap_rule(bad), None, "{bad:?}");
        }
        assert_eq!(zap_rule(&"a".repeat(1_000)), None);
    }

    #[test]
    fn run_at_round_trips_its_index() {
        for at in [At::Start, At::End] {
            assert_eq!(At::from_index(at.index()), at);
        }
        assert_eq!(At::from_index(7), At::Start);
    }

    #[test]
    fn host_with_query_or_fragment_is_rejected() {
        for bad in [
            "example.com?foo=1",
            "example.com#frag",
            "https://example.com?token=1",
            "https://example.com#top",
        ] {
            assert_eq!(Pattern::parse(bad), None, "{bad}");
        }
    }
}
