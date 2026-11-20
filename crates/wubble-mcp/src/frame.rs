//! Newline-delimited JSON-RPC framing: one message per line, UTF-8, no embedded newlines.

use serde_json::Value;

/// A line longer than this is dropped (the stream resynchronises at the next newline).
pub const MAX_LINE: usize = 4 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The line exceeded [`MAX_LINE`].
    TooLong,
    /// The line was not valid UTF-8.
    NotUtf8,
}

/// Splits a byte stream into lines. Feed it whatever the socket gave; it keeps the partial tail.
#[derive(Default)]
pub struct LineFramer {
    buf: Vec<u8>,
    skipping: bool,
}

impl LineFramer {
    /// Complete non-empty lines in `bytes` (and any tail kept from before), or the error for a
    /// line that could not be used. A trailing `\r` is dropped.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<String, FrameError>> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                let line = std::mem::take(&mut self.buf);
                if std::mem::take(&mut self.skipping) {
                    out.push(Err(FrameError::TooLong));
                } else if let Some(r) = finish(line) {
                    out.push(r);
                }
            } else if !self.skipping {
                if self.buf.len() >= MAX_LINE {
                    self.buf = Vec::new();
                    self.skipping = true;
                } else {
                    self.buf.push(b);
                }
            }
        }
        out
    }
}

fn finish(mut line: Vec<u8>) -> Option<Result<String, FrameError>> {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    Some(String::from_utf8(line).map_err(|_| FrameError::NotUtf8))
}

/// One message as a wire line, with its terminating newline.
pub fn encode_line(message: &Value) -> String {
    let mut s = message.to_string();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn splits_lines_across_pushes() {
        let mut f = LineFramer::default();
        assert!(f.push(b"{\"a\":").is_empty());
        let got = f.push(b"1}\n{\"b\":2}\r\n\n  \n{\"c\"");
        assert_eq!(got, vec![Ok("{\"a\":1}".into()), Ok("{\"b\":2}".into())]);
        assert_eq!(f.push(b":3}\n"), vec![Ok("{\"c\":3}".into())]);
    }

    #[test]
    fn rejects_bad_utf8_and_continues() {
        let mut f = LineFramer::default();
        let got = f.push(b"\xff\xfe\n{}\n");
        assert_eq!(got, vec![Err(FrameError::NotUtf8), Ok("{}".into())]);
    }

    #[test]
    fn overlong_line_is_dropped_then_resyncs() {
        let mut f = LineFramer::default();
        let big = vec![b'x'; MAX_LINE + 10];
        assert!(f.push(&big).is_empty());
        let got = f.push(b"more\n{}\n");
        assert_eq!(got, vec![Err(FrameError::TooLong), Ok("{}".into())]);
    }

    #[test]
    fn encode_is_one_line() {
        let s = encode_line(&json!({"text": "a\nb"}));
        assert_eq!(s.matches('\n').count(), 1);
        assert!(s.ends_with('\n'));
    }
}
