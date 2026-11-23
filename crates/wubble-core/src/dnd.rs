//! Drops from other apps: which of the offered MIME types to take, and what
//! a `text/uri-list` holds. Drags out to other apps: which types to offer, and
//! the file a dragged picture lands as.
//!
//! The order is Firefox's and Chromium's on Linux (files and URLs first, then
//! HTML, then plain text); the uri-list rules are RFC 2483 section 5 (one URI
//! per line, CRLF, `#` comment lines). Written new, no code copied. The
//! picture's file name follows Chromium's `DropData::GetSafeFilenameForImageFileContents`
//! (`content/public/common/drop_data.cc`, BSD-3-Clause: the page's name with the
//! extension forced to the picture's type). Chromium's custom data (`custom_data`) is the
//! layout of `ui/base/clipboard/custom_data_helper.cc` in a `base::Pickle`
//! (`base/pickle.cc`, BSD-3-Clause); layout only, no code copied.

use std::path::{Path, PathBuf};

/// What a drop is read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `text/uri-list`: files, or a link when not every entry is a file.
    UriList,
    Html,
    Text,
}

/// PNG's type: the picture a page's copy may carry, what a paste takes first.
pub const PNG: &str = "image/png";
/// A list of URIs: files, or links.
pub const URI_LIST_TYPE: &str = "text/uri-list";

/// The MIME types each kind is offered as, in preference order.
const URI_LIST: &[&str] = &[URI_LIST_TYPE];
const HTML: &[&str] = &["text/html"];
pub(crate) const TEXT: &[&str] = &[
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "STRING",
    "TEXT",
];

/// The text types that are Latin-1, not UTF-8 (X11's ICCCM `STRING`, and `TEXT`, which an
/// owner answers in its own encoding: Latin-1 is what it may be read as).
const LATIN1_TEXT: &[&str] = &["STRING", "TEXT"];

/// What a text type's bytes say: Latin-1 for [`LATIN1_TEXT`], else UTF-8 (a bad sequence a
/// replacement character).
pub fn text_of(mime: &str, bytes: &[u8]) -> String {
    if LATIN1_TEXT.contains(&mime) {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// The text types a drag out (and a copy, `crate::clipboard`) offers: the UTF-8 ones of [`TEXT`].
pub(crate) const OFFERED_TEXT: &[&str] = TEXT.split_at(3).0;

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

/// The `file:` URI of a local path, each byte outside RFC 3986's unreserved
/// set and `/` percent-encoded ("/a b#c" is "file:///a%20b%23c"); what
/// [`parse_uri_list`] reads back.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for (i, segment) in path.to_string_lossy().split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        crate::url::percent_encode_into(segment, &mut out);
    }
    out
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

/// What a drag out offers under a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// `text/uri-list`: the picture's file, else the files and the link.
    UriList,
    /// `text/x-moz-url`: the link's URL, a line feed, its title (Chromium's Wayland form,
    /// UTF-8: `WaylandExchangeDataProvider::ExtractData`); what browsers' address bars and
    /// GNOME's text fields take as a link with its name.
    MozUrl,
    /// The picture's bytes.
    Picture,
    Html,
    /// The text, else the link's URL.
    Text,
}

/// What a drag out of the window carries, for [`offers`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outgoing {
    /// Files, a link or a picture's file.
    pub uris: bool,
    /// A link (or a dragged picture's URL).
    pub link: bool,
    /// The picture's MIME type, when it has bytes of a known type.
    pub picture: Option<&'static str>,
    pub html: bool,
    pub text: bool,
}

/// The types a drag out offers, best first: files and links, the link with its title, the
/// picture itself, HTML, then text. Chromium on Wayland offers `text/x-moz-url`, `text/html`
/// and the plain texts for a link (`BuildMimeTypesList` in
/// ui/ozone/platform/wayland/host/wayland_exchange_data_provider.cc); `text/uri-list` is kept
/// first, as Firefox offers it, for file managers and the desktop. No `_NETSCAPE_URL`: that is
/// X11's (Chromium offers it only there).
pub fn offers(out: &Outgoing) -> Vec<(&'static str, Part)> {
    let uris = out.uris.then_some((URI_LIST_TYPE, Part::UriList));
    let moz = out.link.then_some(("text/x-moz-url", Part::MozUrl));
    let picture = out.picture.map(|mime| (mime, Part::Picture));
    let html = out.html.then_some(("text/html", Part::Html));
    let text = OFFERED_TEXT
        .iter()
        .filter(|_| out.text)
        .map(|mime| (*mime, Part::Text));
    uris.into_iter()
        .chain(moz)
        .chain(picture)
        .chain(html)
        .chain(text)
        .collect()
}

/// Picture types: MIME type, file extension, other extensions it goes by.
const PICTURES: &[(&str, &str, &[&str])] = &[
    (PNG, "png", &[]),
    ("image/jpeg", "jpg", &["jpeg", "jpe", "jfif"]),
    ("image/gif", "gif", &[]),
    ("image/webp", "webp", &[]),
    ("image/avif", "avif", &[]),
    ("image/bmp", "bmp", &[]),
    ("image/tiff", "tif", &["tiff"]),
    ("image/x-icon", "ico", &[]),
    ("image/svg+xml", "svg", &[]),
];

/// The picture type of `bytes` from their first bytes (the WHATWG MIME
/// sniffing standard's image patterns, plus AVIF's `ftyp` brand and SVG's
/// opening tag); `None` when they are no picture known here.
pub fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    Some(if at(0, b"\x89PNG\r\n\x1a\n") {
        PNG
    } else if at(0, &[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        "image/gif"
    } else if at(0, b"RIFF") && at(8, b"WEBPVP") {
        "image/webp"
    } else if at(4, b"ftypavif") || at(4, b"ftypavis") {
        "image/avif"
    } else if at(0, b"BM") {
        "image/bmp"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "image/tiff"
    } else if at(0, &[0, 0, 1, 0]) {
        "image/x-icon"
    } else if is_svg(bytes) {
        "image/svg+xml"
    } else {
        return None;
    })
}

/// An SVG document: its first KiB opens with `<svg`, or is XML with an `<svg`.
fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// The picture types known here, PNG first (what a paste takes first, as Chromium reads PNG).
pub fn picture_types() -> impl Iterator<Item = &'static str> {
    PICTURES.iter().map(|(mime, ..)| *mime)
}

/// The picture type a file name's extension names; `None` for another extension.
pub fn image_mime_of(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    PICTURES
        .iter()
        .find(|(_, own, others)| {
            ext.eq_ignore_ascii_case(own) || others.iter().any(|o| ext.eq_ignore_ascii_case(o))
        })
        .map(|(mime, ..)| *mime)
}

/// The `PICTURES` row of a picture type.
fn picture(mime: &str) -> Option<&'static (&'static str, &'static str, &'static [&'static str])> {
    PICTURES.iter().find(|(m, ..)| m.eq_ignore_ascii_case(mime))
}

/// The file extension of a picture type; `None` for another type.
pub fn image_extension(mime: &str) -> Option<&'static str> {
    picture(mime).map(|(_, ext, _)| *ext)
}

/// The longest a dragged picture's file name gets, in bytes.
const NAME_MAX: usize = 100;

/// The file a dragged picture lands as: the page's name for it (`name`, from
/// Chromium) made safe as one path component, its extension that of `mime`
/// when the type is known; `image` when nothing of the name is left.
pub fn drag_file_name(name: Option<&str>, mime: Option<&str>) -> String {
    let base = name.unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let clean = clean.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let (stem, ext) = match clean.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.trim_end_matches(['.', ' ']), ext),
        _ => (clean, ""),
    };
    let ext = match mime.and_then(picture) {
        Some((_, own, others)) => {
            let fits =
                ext.eq_ignore_ascii_case(own) || others.iter().any(|o| ext.eq_ignore_ascii_case(o));
            if fits { ext } else { own }
        }
        None => ext,
    };
    let stem = if stem.is_empty() { "image" } else { stem };
    let room = NAME_MAX.saturating_sub(ext.len() + 1).max(1);
    let mut cut = stem.len().min(room);
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    let stem = &stem[..cut];
    if ext.is_empty() {
        stem.to_owned()
    } else {
        format!("{stem}.{ext}")
    }
}

/// The per-drag folders (named by their drag's number) to remove when a new
/// picture drag starts: all but the `keep` newest. Other names are not ours.
pub fn stale_drag_dirs<S: AsRef<str>>(names: &[S], keep: usize) -> Vec<String> {
    let mut ours: Vec<(u64, &str)> = names
        .iter()
        .filter_map(|n| Some((n.as_ref().parse().ok()?, n.as_ref())))
        .collect();
    ours.sort_unstable_by_key(|&(n, _)| std::cmp::Reverse(n));
    ours.into_iter()
        .skip(keep)
        .map(|(_, n)| n.to_owned())
        .collect()
}

/// Chromium's custom data (`chromium/x-web-custom-data`): the types a page's `setData` named
/// other than text and HTML, with their values. A `base::Pickle`: a u32 payload size, then a u32
/// count and each type and value as a string16 (an i32 length in UTF-16 units, then the units,
/// zero-padded to 4 bytes); little endian.
pub fn custom_data(entries: &[(String, String)]) -> Vec<u8> {
    let mut payload = (entries.len() as u32).to_le_bytes().to_vec();
    for text in entries.iter().flat_map(|(t, v)| [t, v]) {
        let units: Vec<u16> = text.encode_utf16().collect();
        payload.extend((units.len() as i32).to_le_bytes());
        payload.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        payload.resize(payload.len().next_multiple_of(4), 0);
    }
    let mut out = (payload.len() as u32).to_le_bytes().to_vec();
    out.extend(payload);
    out
}

/// The entries of Chromium's custom data ([`custom_data`]); `None` when the bytes are not one.
pub fn read_custom_data(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    let mut at = 0;
    let u32_at = |at: &mut usize| -> Option<u32> {
        let word = bytes.get(*at..*at + 4)?;
        *at += 4;
        Some(u32::from_le_bytes(word.try_into().ok()?))
    };
    let size = u32_at(&mut at)? as usize;
    if size > bytes.len() - 4 {
        return None;
    }
    let count = u32_at(&mut at)?;
    let string16 = |at: &mut usize| -> Option<String> {
        let len = u32_at(at)? as usize;
        let raw = bytes.get(*at..*at + len.checked_mul(2)?)?;
        *at += (len * 2).next_multiple_of(4);
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|u| u16::from_le_bytes([u[0], u[1]]))
            .collect();
        // A lone surrogate (a page's string cut mid-pair) is a replacement character, as
        // Chromium's own UTF-16 strings tolerate it.
        Some(String::from_utf16_lossy(&units))
    };
    (0..count)
        .map(|_| Some((string16(&mut at)?, string16(&mut at)?)))
        .collect()
}

/// macOS: the pasteboard types a drag out writes and a drop in reads, as Chromium's
/// `ui/base/dragdrop/os_exchange_data_provider_mac.mm` and `web_contents_view_mac.mm` use them
/// (BSD-3-Clause; names and order only, no code copied): the link as `public.url` with its title
/// under `public.url-name` (`PasteboardItemsFromUrls`), HTML as `public.html`, text as
/// `public.utf8-plain-text` (`NSPasteboardTypeString`), files as `public.file-url`, and a dragged
/// picture as a promised file of its type (`NSFilePromiseProvider`, as Chromium's
/// `WebDragSource` promises one).
pub mod pasteboard {
    use super::{Kind, Outgoing};

    pub const FILE_URL: &str = "public.file-url";
    pub const URL: &str = "public.url";
    pub const URL_NAME: &str = "public.url-name";
    pub const HTML: &str = "public.html";
    pub const TEXT: &str = "public.utf8-plain-text";

    /// One thing a drag out puts on the dragging pasteboard.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Item {
        /// Each file, as its own item (`NSURL`).
        Files,
        /// A string under this type on the drag's main item.
        String(&'static str, Source),
        /// The picture, a file promised with this type, written when the target asks.
        Promise(&'static str),
    }

    /// Where a string item's value comes from.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Source {
        Link,
        /// The link's title, else the URL.
        Title,
        Html,
        /// The text, else the link's URL.
        Text,
    }

    /// What a drag out writes, best first. Files are file URLs and no link (Chromium writes
    /// either); a picture with bytes of a known type is promised as a file.
    pub fn writes(out: &Outgoing) -> Vec<Item> {
        let mut items = vec![];
        if out.uris && !out.link {
            items.push(Item::Files);
        }
        if out.link {
            items.push(Item::String(URL, Source::Link));
            items.push(Item::String(URL_NAME, Source::Title));
        }
        if let Some(uti) = out.picture.and_then(image_uti) {
            items.push(Item::Promise(uti));
        }
        if out.html {
            items.push(Item::String(HTML, Source::Html));
        }
        if out.text {
            items.push(Item::String(TEXT, Source::Text));
        }
        items
    }

    /// The UTI of a picture type.
    pub fn image_uti(mime: &str) -> Option<&'static str> {
        const UTIS: &[(&str, &str)] = &[
            (super::PNG, "public.png"),
            ("image/jpeg", "public.jpeg"),
            ("image/gif", "com.compuserve.gif"),
            ("image/webp", "org.webmproject.webp"),
            ("image/avif", "public.avif"),
            ("image/bmp", "com.microsoft.bmp"),
            ("image/tiff", "public.tiff"),
            ("image/x-icon", "com.microsoft.ico"),
            ("image/svg+xml", "public.svg-image"),
        ];
        UTIS.iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(mime))
            .map(|(_, u)| *u)
    }

    /// The kinds to read for a drop offering `types`, each with the type to read, best first
    /// (files, then a link, then HTML, then text); empty: nothing a page takes.
    pub fn plan<S: AsRef<str>>(types: &[S]) -> Vec<(Kind, &'static str)> {
        let has = |t: &str| types.iter().any(|m| m.as_ref() == t);
        let uri = [FILE_URL, URL].into_iter().find(|t| has(t));
        uri.map(|t| (Kind::UriList, t))
            .into_iter()
            .chain(has(HTML).then_some((Kind::Html, HTML)))
            .chain(has(TEXT).then_some((Kind::Text, TEXT)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_read_in_its_types_encoding() {
        for (mime, bytes, text) in [
            ("UTF8_STRING", "café".as_bytes(), "café"),
            ("text/plain;charset=utf-8", "café".as_bytes(), "café"),
            ("STRING", b"caf\xe9".as_slice(), "café"),
            ("TEXT", b"caf\xe9".as_slice(), "café"),
            ("text/plain", b"caf\xe9".as_slice(), "caf\u{fffd}"),
        ] {
            assert_eq!(text_of(mime, bytes), text, "{mime}");
        }
    }

    #[test]
    fn file_uris_encode_and_read_back() {
        let path = Path::new("/home/a/My Doc #1.pdf");
        let uri = file_uri(path);
        assert_eq!(uri, "file:///home/a/My%20Doc%20%231.pdf");
        assert_eq!(
            parse_uri_list(&uri),
            Some(UriList::Files(vec![path.to_path_buf()]))
        );
        assert_eq!(file_uri(Path::new("/tmp/ü")), "file:///tmp/%C3%BC");
    }

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

    #[test]
    fn pictures_are_known_by_their_first_bytes() {
        let cases: &[(&[u8], Option<&str>)] = &[
            (b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", Some("image/png")),
            (&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10], Some("image/jpeg")),
            (b"GIF89a\x01\0", Some("image/gif")),
            (b"RIFF\x24\0\0\0WEBPVP8 ", Some("image/webp")),
            (b"\0\0\0\x1cftypavif\0\0", Some("image/avif")),
            (b"BM\x36\0\0\0", Some("image/bmp")),
            (b"II*\0\x08\0", Some("image/tiff")),
            (&[0, 0, 1, 0, 1, 0], Some("image/x-icon")),
            (b"\xef\xbb\xbf  <svg xmlns='x'/>", Some("image/svg+xml")),
            (b"<?xml version='1.0'?>\n<svg/>", Some("image/svg+xml")),
            (b"<?xml version='1.0'?><feed/>", None),
            (b"RIFF\x24\0\0\0WAVEfmt ", None),
            (b"<html>", None),
            (b"", None),
        ];
        for (bytes, mime) in cases {
            assert_eq!(sniff_image_mime(bytes), *mime, "{bytes:?}");
        }
        assert_eq!(image_extension("image/JPEG"), Some("jpg"));
        assert_eq!(image_extension("text/html"), None);
    }

    #[test]
    fn a_picture_file_name_is_one_safe_path_component() {
        let png = Some("image/png");
        let jpeg = Some("image/jpeg");
        assert_eq!(drag_file_name(Some("cat.png"), png), "cat.png");
        assert_eq!(drag_file_name(Some("../../etc/passwd"), png), "passwd.png");
        assert_eq!(drag_file_name(Some("a\\b\\..\\c.png"), png), "c.png");
        assert_eq!(drag_file_name(Some(".."), png), "image.png");
        assert_eq!(drag_file_name(Some(".hidden.png"), png), "hidden.png");
        assert_eq!(drag_file_name(Some("a\nb\u{7}c:*?.png"), png), "abc.png");
        assert_eq!(drag_file_name(Some("  photo . "), jpeg), "photo.jpg");
        // The type wins over the name; its other extensions are kept.
        assert_eq!(drag_file_name(Some("cat.png"), jpeg), "cat.jpg");
        assert_eq!(drag_file_name(Some("cat.JPEG"), jpeg), "cat.JPEG");
        assert_eq!(drag_file_name(Some("cat"), Some("image/webp")), "cat.webp");
        // Unknown type: the name as it is.
        assert_eq!(drag_file_name(Some("cat.heic"), None), "cat.heic");
        assert_eq!(drag_file_name(None, None), "image");
        assert_eq!(drag_file_name(Some(""), png), "image.png");
        assert_eq!(
            drag_file_name(Some("кот 猫.gif"), Some("image/gif")),
            "кот 猫.gif"
        );
        // A long name is cut on a character, keeping its extension.
        let long = "猫".repeat(100) + ".png";
        let name = drag_file_name(Some(&long), png);
        assert!(name.len() <= NAME_MAX && name.ends_with(".png"), "{name}");
        assert!(name.starts_with("猫猫"));
    }

    #[test]
    fn a_drag_out_offers_files_the_picture_html_then_text() {
        let all = Outgoing {
            uris: true,
            link: true,
            picture: Some("image/png"),
            html: true,
            text: true,
        };
        assert_eq!(
            offers(&all),
            [
                ("text/uri-list", Part::UriList),
                ("text/x-moz-url", Part::MozUrl),
                ("image/png", Part::Picture),
                ("text/html", Part::Html),
                ("text/plain;charset=utf-8", Part::Text),
                ("UTF8_STRING", Part::Text),
                ("text/plain", Part::Text),
            ]
        );
        let jpeg = Outgoing {
            picture: Some("image/jpeg"),
            ..all
        };
        let types: Vec<_> = offers(&jpeg).into_iter().map(|(m, _)| m).collect();
        assert_eq!(types[2], "image/jpeg");
        assert!(!types.contains(&"image/png"));
        let link = Outgoing {
            uris: true,
            link: true,
            text: true,
            ..Outgoing::default()
        };
        assert_eq!(offers(&link).len(), 5);
        assert!(offers(&Outgoing::default()).is_empty());
    }

    #[test]
    fn custom_data_is_chromiums_pickle() {
        let one = vec![("a".to_owned(), "b".to_owned())];
        #[rustfmt::skip]
        let bytes: &[u8] = &[
            20, 0, 0, 0, // payload size
            1, 0, 0, 0, // one entry
            1, 0, 0, 0, b'a', 0, 0, 0, // "a", padded to 4
            1, 0, 0, 0, b'b', 0, 0, 0,
        ];
        assert_eq!(custom_data(&one), bytes);
        let cases: &[&[(&str, &str)]] = &[
            &[],
            &[("text/x-a", "")],
            &[
                ("application/x-b", "猫 cat"),
                ("text/uri-list", "https://a.test/"),
            ],
            &[("ab", "abc"), ("é", "😀")],
        ];
        for entries in cases {
            let owned: Vec<_> = entries
                .iter()
                .map(|(t, v)| (t.to_string(), v.to_string()))
                .collect();
            assert_eq!(read_custom_data(&custom_data(&owned)), Some(owned));
        }
        // Too short, a size past the end, a string past the end: none.
        let broken: &[&[u8]] = &[
            &[],
            &[1, 0],
            &[200, 0, 0, 0, 0, 0, 0, 0],
            &[8, 0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0],
        ];
        for bytes in broken {
            assert_eq!(read_custom_data(bytes), None, "{bytes:?}");
        }
        // A lone surrogate in a value (a page's string cut mid-pair): a replacement character.
        let lone = [
            20, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x61, 0, 0, 0, 1, 0, 0, 0, 0, 0xd8, 0, 0,
        ];
        assert_eq!(
            read_custom_data(&lone),
            Some(vec![("a".into(), "\u{fffd}".into())])
        );
    }

    #[test]
    fn a_file_names_its_picture_type_by_extension() {
        let cases = [
            ("shot.png", Some("image/png")),
            ("a.b.JPEG", Some("image/jpeg")),
            ("x.jfif", Some("image/jpeg")),
            ("notes.txt", None),
            ("png", None),
        ];
        for (name, mime) in cases {
            assert_eq!(image_mime_of(name), mime, "{name}");
        }
        assert_eq!(picture_types().next(), Some("image/png"));
    }

    #[test]
    fn only_the_newest_drag_folders_are_kept() {
        let names = ["3", "10", "notes", "1", "2", "x-4"];
        assert_eq!(stale_drag_dirs(&names, 2), ["2", "1"]);
        assert!(stale_drag_dirs(&names, 8).is_empty());
        assert_eq!(stale_drag_dirs(&names, 0), ["10", "3", "2", "1"]);
        assert!(stale_drag_dirs::<&str>(&[], 2).is_empty());
    }

    #[test]
    fn a_mac_drag_out_writes_the_link_with_its_title_the_picture_html_and_text() {
        use pasteboard::{Item, Source, writes};
        let all = Outgoing {
            uris: true,
            link: true,
            picture: Some("image/jpeg"),
            html: true,
            text: true,
        };
        assert_eq!(
            writes(&all),
            [
                Item::String(pasteboard::URL, Source::Link),
                Item::String(pasteboard::URL_NAME, Source::Title),
                Item::Promise("public.jpeg"),
                Item::String(pasteboard::HTML, Source::Html),
                Item::String(pasteboard::TEXT, Source::Text),
            ]
        );
        let files = Outgoing {
            uris: true,
            ..Outgoing::default()
        };
        assert_eq!(writes(&files), [Item::Files]);
        // an unknown picture type is no promise
        let odd = Outgoing {
            picture: Some("image/heic"),
            ..Outgoing::default()
        };
        assert!(writes(&odd).is_empty());
        assert_eq!(pasteboard::image_uti("IMAGE/PNG"), Some("public.png"));
    }

    #[test]
    fn a_mac_drop_reads_files_then_a_link_then_html_then_text() {
        use pasteboard::{FILE_URL, HTML, TEXT, URL, plan};
        assert_eq!(
            plan(&[TEXT, URL, HTML, FILE_URL]),
            [
                (Kind::UriList, FILE_URL),
                (Kind::Html, HTML),
                (Kind::Text, TEXT)
            ]
        );
        assert_eq!(
            plan(&[TEXT, URL]),
            [(Kind::UriList, URL), (Kind::Text, TEXT)]
        );
        assert!(plan(&["com.apple.flat-rtfd"]).is_empty());
    }
}
