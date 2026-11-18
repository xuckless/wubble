//! `data:` URLs for bundled pages, so nothing is fetched from the network.

/// `data:text/html;charset=utf-8,<html>` with every byte outside RFC 3986's
/// unreserved set percent-encoded (so `#`, `%`, `?` and newlines survive).
pub fn html(html: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut url = String::from("data:text/html;charset=utf-8,");
    for &byte in html.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            url.push('%');
            url.push(char::from(HEX[usize::from(byte >> 4)]));
            url.push(char::from(HEX[usize::from(byte & 0xF)]));
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(url: &str) -> String {
        let body = url
            .strip_prefix("data:text/html;charset=utf-8,")
            .expect("data URL prefix");
        let mut bytes = Vec::new();
        let mut rest = body.as_bytes();
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
    fn round_trips_reserved_bytes_and_utf8() {
        for page in [
            "",
            "<p id=\"a\">100% #1 ?q=1&b=2</p>\n\t<b>ok</b>",
            "über 🫧 wubble",
            include_str!("../assets/smoke.html"),
        ] {
            let url = html(page);
            assert!(!url.contains(['#', ' ', '\n', '<', '"']), "{url}");
            assert_eq!(decode(&url), page);
        }
    }

    #[test]
    fn unreserved_bytes_stay_readable() {
        assert_eq!(html("aZ09-._~"), "data:text/html;charset=utf-8,aZ09-._~");
        assert_eq!(html("#%\n"), "data:text/html;charset=utf-8,%23%25%0A");
    }
}
