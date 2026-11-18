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

#[cfg(test)]
mod tests {
    use super::*;

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
