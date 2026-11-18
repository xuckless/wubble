//! URL text helpers.

/// Only web pages are history and bookmarks; internal and file URLs never are.
pub fn is_recordable(url: &str) -> bool {
    ["http://", "https://"]
        .iter()
        .any(|p| url.len() > p.len() && url[..p.len()].eq_ignore_ascii_case(p))
}

/// Match key: lowercase, scheme and a leading `www.` removed (Places autofill idea), so
/// typing `exa` finds `https://www.example.com/`.
pub fn url_key(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
    rest.strip_prefix("www.").unwrap_or(rest).to_string()
}

/// Appends `input` to `out` with every byte outside RFC 3986's unreserved set
/// percent-encoded, so the result is safe in a query value or a `data:` body.
pub fn percent_encode_into(input: &str, out: &mut String) {
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
}

/// Schemes kept as typed although no `//` follows them.
const OPAQUE_SCHEMES: &[&str] = &["about", "data", "view-source", "mailto", "blob"];

/// The URL a command-line argument names: anything with a scheme as given,
/// a bare `host/path` as `https://host/path`.
pub fn from_cli_arg(arg: &str) -> String {
    let arg = arg.trim();
    let has_scheme = arg.split_once(':').is_some_and(|(scheme, rest)| {
        let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        valid
            && (rest.starts_with("//")
                || OPAQUE_SCHEMES
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(scheme)))
    });
    if has_scheme {
        arg.to_string()
    } else {
        format!("https://{arg}")
    }
}

/// The two halves the address pill shows: the host (with the scheme for
/// URLs without one, such as `about:`) and everything after it.
pub fn address_parts(url: &str) -> (&str, &str) {
    if let Some((_, rest)) = url.split_once("://") {
        rest.split_at(rest.find('/').unwrap_or(rest.len()))
    } else if let Some(colon) = url.find(':') {
        url.split_at(colon + 1)
    } else {
        (url, "")
    }
}

/// The key favicons (and other per-site caches) are filed under:
/// `scheme://host[:port]`, lowercase, default port elided. Only `http` and
/// `https` pages have one; the userinfo, path, query and fragment are dropped.
/// `www.` stays: icons differ per host.
pub fn origin_key(url: &str) -> Option<String> {
    let url = ::url::Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

/// The `view-source:` URL for `page_url`: only for pages Chromium can show
/// the source of (`http`, `https`, `file`, `ftp`); never for a `view-source:`
/// URL itself or an internal page.
pub fn view_source(page_url: &str) -> Option<String> {
    let url = ::url::Url::parse(page_url).ok()?;
    matches!(url.scheme(), "http" | "https" | "file" | "ftp")
        .then(|| format!("view-source:{page_url}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_key_strips_scheme_and_www() {
        assert_eq!(url_key("https://www.Example.com/A"), "example.com/a");
        assert_eq!(url_key("exa"), "exa");
        assert_eq!(url_key("www.go"), "go");
    }

    #[test]
    fn only_web_urls_are_recordable() {
        assert!(is_recordable("HTTP://a.com"));
        assert!(!is_recordable("https://"));
        assert!(!is_recordable("wubble://history"));
        assert!(!is_recordable("about:blank"));
    }

    #[test]
    fn origin_keys() {
        let cases = [
            ("https://Example.com/a/b?q=1#f", Some("https://example.com")),
            ("http://example.com:80/", Some("http://example.com")),
            ("https://example.com:443", Some("https://example.com")),
            (
                "https://example.com:8443/x",
                Some("https://example.com:8443"),
            ),
            (
                "http://u:p@www.example.com:8080/x",
                Some("http://www.example.com:8080"),
            ),
            ("about:blank", None),
            ("data:text/html,x", None),
            ("file:///tmp/x.html", None),
            ("view-source:https://example.com/", None),
            ("not a url", None),
            ("", None),
        ];
        for (url, want) in cases {
            assert_eq!(origin_key(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn view_source_urls() {
        let cases = [
            (
                "https://example.com/a?b",
                Some("view-source:https://example.com/a?b"),
            ),
            ("file:///tmp/x.html", Some("view-source:file:///tmp/x.html")),
            ("view-source:https://example.com/", None),
            ("about:blank", None),
            ("data:text/html,x", None),
            ("garbage", None),
        ];
        for (url, want) in cases {
            assert_eq!(view_source(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn address_parts_split_host_and_path() {
        let cases = [
            (
                "https://example.com/a/b?q=1#f",
                ("example.com", "/a/b?q=1#f"),
            ),
            ("https://example.com", ("example.com", "")),
            ("http://localhost:8080/", ("localhost:8080", "/")),
            ("file:///tmp/x.html", ("", "/tmp/x.html")),
            ("about:blank", ("about:", "blank")),
            ("data:text/html,x", ("data:", "text/html,x")),
            ("", ("", "")),
        ];
        for (url, parts) in cases {
            assert_eq!(address_parts(url), parts, "{url}");
        }
    }

    #[test]
    fn cli_args_with_a_scheme_are_kept() {
        for url in [
            "https://example.com/a?b#c",
            "http://localhost:8080/",
            "file:///tmp/x.html",
            "about:blank",
            "data:text/html,<p>hi",
            "view-source:https://example.com/",
            "HTTPS://EXAMPLE.COM",
            "chrome://version",
        ] {
            assert_eq!(from_cli_arg(url), url);
        }
    }

    #[test]
    fn bare_hosts_get_https() {
        let cases = [
            ("example.com", "https://example.com"),
            ("example.com/a/b?q=1", "https://example.com/a/b?q=1"),
            ("localhost:8080/x", "https://localhost:8080/x"),
            ("  example.org  ", "https://example.org"),
            ("1.2.3.4:80", "https://1.2.3.4:80"),
        ];
        for (arg, url) in cases {
            assert_eq!(from_cli_arg(arg), url, "{arg}");
        }
    }

    fn encode(input: &str) -> String {
        let mut out = String::new();
        percent_encode_into(input, &mut out);
        out
    }

    fn decode(encoded: &str) -> String {
        let mut bytes = Vec::new();
        let mut rest = encoded.as_bytes();
        while let Some((&b, tail)) = rest.split_first() {
            if b == b'%' {
                let hex = std::str::from_utf8(&tail[..2]).expect("ascii hex");
                bytes.push(u8::from_str_radix(hex, 16).expect("hex digit"));
                rest = &tail[2..];
            } else {
                bytes.push(b);
                rest = tail;
            }
        }
        String::from_utf8(bytes).expect("utf-8")
    }

    #[test]
    fn unreserved_bytes_stay_readable() {
        assert_eq!(encode("aZ09-._~"), "aZ09-._~");
        assert_eq!(encode("#%\n /"), "%23%25%0A%20%2F");
    }

    #[test]
    fn round_trips_reserved_bytes_and_utf8() {
        for input in [
            "",
            "<p id=\"a\">100% #1 ?q=1&b=2</p>\n\t<b>ok</b>",
            "über 🫧 wubble",
        ] {
            let encoded = encode(input);
            assert!(!encoded.contains(['#', ' ', '\n', '<', '"']), "{encoded}");
            assert_eq!(decode(&encoded), input);
        }
    }
}
