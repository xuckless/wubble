//! Drops from other apps: which of the offered MIME types to take, and what
//! a `text/uri-list` holds.
//!
//! The order is Firefox's and Chromium's on Linux (files and URLs first, then
//! HTML, then plain text); the uri-list rules are RFC 2483 section 5 (one URI
//! per line, CRLF, `#` comment lines). Written new, no code copied.

use std::path::PathBuf;

/// What a drop is read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `text/uri-list`: files, or a link when not every entry is a file.
    UriList,
    Html,
    Text,
}

/// The MIME types each kind is offered as, in preference order.
const URI_LIST: &[&str] = &["text/uri-list"];
const HTML: &[&str] = &["text/html"];
const TEXT: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// The kinds to read for a drop offering `mimes`, each with the type to ask
/// for, best first (the first is what the drop is accepted as). Empty: the
/// drop has nothing a page takes.
pub fn plan<S: AsRef<str>>(mimes: &[S]) -> Vec<(Kind, &'static str)> {
    let offered = |names: &'static [&'static str]| {
        names
            .iter()
            .find(|n| mimes.iter().any(|m| m.as_ref().eq_ignore_ascii_case(n)))
            .copied()
    };
    [
        (Kind::UriList, URI_LIST),
        (Kind::Html, HTML),
        (Kind::Text, TEXT),
    ]
    .into_iter()
    .filter_map(|(kind, names)| offered(names).map(|m| (kind, m)))
    .collect()
}

/// A `text/uri-list` read: local files, or else the first URI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UriList {
    Files(Vec<PathBuf>),
    Link(String),
}

/// What a `text/uri-list` holds: every entry a local file gives `Files`;
/// otherwise the first entry as a `Link`. `None` when it lists nothing.
pub fn parse_uri_list(text: &str) -> Option<UriList> {
    let uris: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let first = *uris.first()?;
    let files: Option<Vec<PathBuf>> = uris.iter().map(|u| file_path(u)).collect();
    Some(match files {
        Some(files) => UriList::Files(files),
        None => UriList::Link(first.to_owned()),
    })
}

/// The local path of a `file:` URI ("file:///a%20b", "file://localhost/a");
/// `None` for another scheme or another host.
fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(at) if &rest[..at] == "localhost" => &rest[at..],
        _ => return None,
    };
    let bytes = percent_decode(path)?;
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            let hex = std::str::from_utf8(&hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            out.push(b);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_and_links_come_before_html_and_text() {
        let all = ["text/plain", "text/html", "text/uri-list"];
        assert_eq!(
            plan(&all),
            [
                (Kind::UriList, "text/uri-list"),
                (Kind::Html, "text/html"),
                (Kind::Text, "text/plain")
            ]
        );
        assert_eq!(
            plan(&["UTF8_STRING", "text/plain;charset=utf-8"]),
            [(Kind::Text, "text/plain;charset=utf-8")]
        );
        assert_eq!(plan(&["TEXT/HTML"]), [(Kind::Html, "text/html")]);
    }

    #[test]
    fn unknown_types_are_not_taken() {
        assert!(plan(&["image/png", "application/x-moz-file"]).is_empty());
        assert!(plan::<&str>(&[]).is_empty());
        assert!(plan(&["x-special/gnome-copied-files"]).is_empty());
    }

    #[test]
    fn a_uri_list_of_files_is_files() {
        let list = "# from Files\r\nfile:///home/a/My%20Doc.txt\r\nfile://localhost/tmp/b\r\n";
        assert_eq!(
            parse_uri_list(list),
            Some(UriList::Files(vec![
                "/home/a/My Doc.txt".into(),
                "/tmp/b".into()
            ]))
        );
    }

    #[test]
    fn anything_but_local_files_is_a_link() {
        assert_eq!(
            parse_uri_list("https://a.test/x\nfile:///tmp/b\n"),
            Some(UriList::Link("https://a.test/x".into()))
        );
        // Another host's file is not ours to read.
        assert_eq!(
            parse_uri_list("file://other/tmp/b"),
            Some(UriList::Link("file://other/tmp/b".into()))
        );
        // A broken escape is not a path.
        assert_eq!(
            parse_uri_list("file:///a%2"),
            Some(UriList::Link("file:///a%2".into()))
        );
        assert_eq!(parse_uri_list("# only a comment\n\n"), None);
        assert_eq!(parse_uri_list(""), None);
    }
}
