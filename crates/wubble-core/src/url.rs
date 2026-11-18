//! URL text helpers.

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

#[cfg(test)]
mod tests {
    use super::*;

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
