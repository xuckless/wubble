//! The pages' clipboard where wubble carries it itself (Wayland, `crates/wubble/src/clipboard.rs`):
//! what a copy holds and the types it is offered as, which offered types a paste reads and what
//! they hold, and the JSON the page script (wubble-helper `clipboard.js`) sends and is answered
//! with.
//!
//! The types are Chromium's on Linux, names and layout only, no code copied (BSD-3-Clause):
//! `ui/base/clipboard/clipboard_constants.cc` (`text/html`, `chromium/x-web-custom-data`), the
//! HTML header of `ClipboardOzone::WriteHTML` and the UTF-16 HTML `ReadHTML` takes from Firefox,
//! and the `web ` formats of `ClipboardFormatType::WebCustomFormatMap`/`WebCustomFormatName`
//! (a JSON map from each format to the type it is offered as). Files copied in a file manager are
//! `text/uri-list` or GNOME's `x-special/gnome-copied-files` (`copy` or `cut`, then the URIs).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::dnd::{self, UriList};

/// What a copy puts on the clipboard, or a paste takes from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Payload {
    pub text: Option<String>,
    pub html: Option<String>,
    /// As the page gets them: a picture (a copy's own, or the clipboard's, which Chromium names
    /// `image.<ext>`), or the files a file manager copied.
    pub files: Vec<File>,
    /// `clipboardData.setData`'s types other than text and HTML, with their values: Chromium's
    /// custom data.
    pub custom: Vec<(String, String)>,
    /// `navigator.clipboard.write`'s `web ` formats, by their type without the prefix.
    pub web: Vec<(String, Vec<u8>)>,
}

/// A file as the page gets it; `mime` is empty when unknown, as a browser's `File.type` is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Who reads the clipboard, which says the types it takes (Chromium's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// Text only: a field's paste, `readText`, Paste as Plain Text, the primary selection.
    Text,
    /// `navigator.clipboard.read`: text, HTML, a picture and the `web ` formats; never files.
    AsyncApi,
    /// A page's `paste` event: text, HTML, a picture, copied files and custom data.
    Paste,
}

/// What a paste reads a type as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Html,
    Picture,
    /// A file manager's copy: `text/uri-list` or GNOME's list.
    Files,
    CustomData,
    /// The `web ` formats' map, and each format.
    WebMap,
    Web,
}

impl Kind {
    /// The most a paste reads of one type, in bytes.
    pub const fn cap(self) -> u64 {
        match self {
            Kind::Picture => PICTURE_CAP,
            Kind::Files | Kind::WebMap => 1 << 20,
            Kind::Text | Kind::Html | Kind::CustomData | Kind::Web => 8 << 20,
        }
    }
}

/// The largest picture a paste takes: a screenshot of a large screen as PNG fits.
pub const PICTURE_CAP: u64 = 16 << 20;
/// The most a paste reads of the files a file manager copied, all together; a file that would
/// go past it is left out.
pub const FILES_CAP: u64 = 16 << 20;
/// At most this many `web ` formats (Chromium's `kMaxRegisteredClipboardFormats`).
const WEB_MAX: usize = 100;

const HTML: &str = "text/html";
const CUSTOM_DATA: &str = "chromium/x-web-custom-data";
const WEB_MAP: &str = "application/web;type=\"custom/formatmap\"";
/// A `web ` format's type is this, its index, and a closing quote.
const WEB_FORMAT: &str = "application/web;type=\"custom/format";
const GNOME_FILES: &str = "x-special/gnome-copied-files";
/// What Chromium on Linux puts before copied HTML.
const HTML_HEADER: &str = "<meta http-equiv=\"content-type\" content=\"text/html; charset=utf-8\">";

const EVERY: &[Reader] = &[Reader::Text, Reader::AsyncApi, Reader::Paste];
const RICH: &[Reader] = &[Reader::AsyncApi, Reader::Paste];

/// The types a paste reads each kind from, best first, and who reads it; pictures
/// ([`dnd::picture_types`]) are read by [`RICH`], and every offered `web ` format by
/// [`Reader::AsyncApi`]. A new kind is a row here.
const READS: &[(Kind, &[Reader], &[&str])] = &[
    (Kind::Text, EVERY, dnd::TEXT),
    (Kind::Html, RICH, &[HTML]),
    (
        Kind::Files,
        &[Reader::Paste],
        &[dnd::URI_LIST_TYPE, GNOME_FILES],
    ),
    (Kind::CustomData, &[Reader::Paste], &[CUSTOM_DATA]),
    (Kind::WebMap, &[Reader::AsyncApi], &[WEB_MAP]),
];

/// The most pixels a page's copied picture may have (a 8192 x 8192 PNG): the host decodes it
/// to check it, and one that says it is larger is refused before any pixel is read.
pub const PAGE_PICTURE_PIXELS: u64 = 1 << 26;

/// The longest primary selection a page sets (`clipboard.js` caps it too); a longer one is
/// dropped, as no one middle-click pastes a book.
pub const PRIMARY_CAP: usize = 1 << 20;

/// How long a page's read counts as in flight ([`Asker::reading`]). Past it the read is given
/// up for, whatever happened to its answer (dropped while the host was busy, its tab or window
/// closed, the worker stopped), so no tab is left unable to read. Longer than the worker's own
/// deadline for reading the clipboard (1.5 s), so a read that is answered is never overtaken.
pub const READ_IN_FLIGHT: Duration = Duration::from_secs(3);

/// Whether a read started at `started` (the host's clock; `None`: no read) is in flight at
/// `now`.
pub fn in_flight(started: Option<Duration>, now: Duration) -> bool {
    started.is_some_and(|s| now.saturating_sub(s) < READ_IN_FLIGHT)
}

/// What a page asks of the host's clipboard, for [`admit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Puts something on the clipboard: a copy or cut, `write`, `writeText`, an image
    /// document's copy.
    Write,
    /// Sets the primary selection.
    Primary,
    /// Takes the clipboard: a paste, `readText`, `read`.
    Read,
}

/// What the host knows of the frame that asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asker {
    /// Its tab is the page the window's keys go to.
    pub key_page: bool,
    /// That window has the keyboard.
    pub window_focused: bool,
    /// It is CEF's focused frame of that page.
    pub frame_focused: bool,
    /// A read for its tab is still being answered ([`in_flight`]).
    pub reading: bool,
}

/// Whether the host does what a frame asks of the clipboard. Only the frame the user is in
/// reaches it, as Chromium lets only a focused document use the clipboard: never a background
/// tab, an unfocused window or another frame of the page (a cross-origin iframe), whatever the
/// page's script did to the bridge. One read at a time per tab, so no page holds the window
/// waiting on reads.
pub fn admit(ask: Ask, asker: Asker) -> bool {
    let present = asker.key_page && asker.window_focused && asker.frame_focused;
    match ask {
        Ask::Write | Ask::Primary => present,
        Ask::Read => present && !asker.reading,
    }
}

/// The types `reader` reads from a clipboard offering `mimes`, each with its kind, as offered.
pub fn plan<S: AsRef<str>>(mimes: &[S], reader: Reader) -> Vec<(Kind, String)> {
    let reads = |readers: &[Reader]| readers.contains(&reader);
    let offered = |t: &str| {
        mimes
            .iter()
            .map(|m| m.as_ref())
            .find(|m| m.eq_ignore_ascii_case(t))
    };
    let rows = READS
        .iter()
        .filter(|(_, readers, _)| reads(*readers))
        .filter_map(|(kind, _, types)| Some((*kind, types.iter().find_map(|t| offered(*t))?)));
    let picture = dnd::picture_types()
        .find_map(|t| offered(t))
        .filter(|_| reads(RICH))
        .map(|m| (Kind::Picture, m));
    let web = mimes
        .iter()
        .map(|m| m.as_ref())
        .filter(|m| {
            // A format's index, not the map, which shares the prefix.
            reader == Reader::AsyncApi
                && m.strip_prefix(WEB_FORMAT)
                    .and_then(|i| i.strip_suffix('"'))
                    .is_some_and(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter(|_| offered(WEB_MAP).is_some())
        .take(WEB_MAX)
        .map(|m| (Kind::Web, m));
    rows.chain(picture)
        .chain(web)
        .map(|(kind, m)| (kind, m.to_owned()))
        .collect()
}

/// What a paste read (each planned type's kind, type and bytes): the payload, and the local
/// files a file manager's copy names, whose bytes the caller reads into it ([`add_file`]).
pub fn read(parts: Vec<(Kind, String, Vec<u8>)>) -> (Payload, Vec<PathBuf>) {
    let mut payload = Payload::default();
    let mut paths = vec![];
    let map: Vec<(String, String)> = parts
        .iter()
        .find(|(kind, ..)| *kind == Kind::WebMap)
        .and_then(|(_, _, bytes)| serde_json::from_slice::<Map<String, Value>>(bytes).ok())
        .map(|map| {
            map.into_iter()
                .filter_map(|(t, m)| Some((t, m.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    for (kind, mime, bytes) in parts {
        match kind {
            Kind::Text => payload.text = Some(normalize_to_lf(&dnd::text_of(&mime, &bytes))),
            Kind::Html => payload.html = Some(html_text(&bytes)),
            Kind::Picture => payload.files.push(File {
                name: format!("image.{}", dnd::image_extension(&mime).unwrap_or("png")),
                mime: mime.to_ascii_lowercase(),
                bytes,
            }),
            Kind::Files => {
                let text = String::from_utf8_lossy(&bytes);
                // GNOME's list says `copy` or `cut` first.
                let list = if mime.eq_ignore_ascii_case(GNOME_FILES) {
                    text.split_once('\n').map_or("", |(_, uris)| uris)
                } else {
                    &*text
                };
                if let Some(UriList::Files(files)) = dnd::parse_uri_list(list) {
                    paths = files;
                }
            }
            Kind::CustomData => payload.custom = dnd::read_custom_data(&bytes).unwrap_or_default(),
            Kind::WebMap => {}
            Kind::Web => {
                if let Some((t, _)) = map.iter().find(|(_, m)| *m == mime) {
                    payload.web.push((t.clone(), bytes));
                }
            }
        }
    }
    (payload, paths)
}

/// Adds a copied file's bytes to a paste's `payload`, unless they would take its files past
/// [`FILES_CAP`]; its type is its picture type by name, else unknown.
pub fn add_file(payload: &mut Payload, name: String, bytes: Vec<u8>) {
    let used: u64 = payload.files.iter().map(|f| f.bytes.len() as u64).sum();
    if used + bytes.len() as u64 > FILES_CAP {
        return;
    }
    let mime = dnd::image_mime_of(&name).unwrap_or_default().to_owned();
    payload.files.push(File { name, mime, bytes });
}

/// The types a copy is offered as, with the bytes of each: its picture, HTML (with Chromium's
/// header), text, custom data, and the `web ` formats with their map.
pub fn offers(payload: &Payload) -> Vec<(String, Arc<[u8]>)> {
    let bytes = |b: &[u8]| Arc::<[u8]>::from(b);
    let picture = payload
        .files
        .iter()
        .find(|f| dnd::image_extension(&f.mime).is_some())
        .map(|f| (f.mime.clone(), bytes(&f.bytes)));
    let html = payload.html.as_ref().map(|h| {
        (
            HTML.to_owned(),
            bytes(format!("{HTML_HEADER}{h}").as_bytes()),
        )
    });
    let text = payload.text.as_ref().map(|t| bytes(t.as_bytes()));
    let texts = dnd::OFFERED_TEXT
        .iter()
        .filter_map(|mime| Some((mime.to_string(), text.clone()?)));
    let custom = (!payload.custom.is_empty()).then(|| {
        (
            CUSTOM_DATA.to_owned(),
            bytes(&dnd::custom_data(&payload.custom)),
        )
    });
    let web = payload.web.iter().take(WEB_MAX).enumerate();
    let names: Map<String, Value> = web
        .clone()
        .map(|(i, (t, _))| (t.clone(), Value::from(format!("{WEB_FORMAT}{i}\""))))
        .collect();
    let map = (!names.is_empty()).then(|| {
        let names = Value::from(names).to_string();
        (WEB_MAP.to_owned(), bytes(names.as_bytes()))
    });
    let formats = web.map(|(i, (_, b))| (format!("{WEB_FORMAT}{i}\""), bytes(b)));
    picture
        .into_iter()
        .chain(html)
        .chain(texts)
        .chain(custom)
        .chain(map)
        .chain(formats)
        .collect()
}

/// Copy Image's HTML beside the picture, as Chromium writes it (Blink `URLToImageMarkup`).
pub fn image_markup(url: &str) -> String {
    let mut escaped = String::with_capacity(url.len());
    for c in url.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            c => escaped.push(c),
        }
    }
    format!("<img src=\"{escaped}\"/>")
}

/// CR and CRLF as LF (smithay-clipboard `mime.rs` `normalize_to_lf`).
fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Pasted HTML as text: UTF-16 after a byte order mark (Firefox's), else UTF-8; without the
/// trailing NUL some apps send.
fn html_text(bytes: &[u8]) -> String {
    let text = match bytes {
        [0xff, 0xfe, rest @ ..] => {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|u| u16::from_le_bytes([u[0], u[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    };
    text.trim_end_matches('\0').to_owned()
}

impl Payload {
    /// The page script's JSON: `{text, html, files: [{name, type, data}], custom: [[type,
    /// value]], web: [[type, data]]}`, bytes as base64; what is absent is left out.
    pub fn to_json(&self) -> String {
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
        let mut out = Map::new();
        if let Some(text) = &self.text {
            out.insert("text".into(), json!(text));
        }
        if let Some(html) = &self.html {
            out.insert("html".into(), json!(html));
        }
        if !self.files.is_empty() {
            let files = self
                .files
                .iter()
                .map(|f| json!({ "name": f.name, "type": f.mime, "data": b64(&f.bytes) }));
            out.insert("files".into(), Value::Array(files.collect()));
        }
        if !self.custom.is_empty() {
            out.insert("custom".into(), json!(self.custom));
        }
        if !self.web.is_empty() {
            let web = self.web.iter().map(|(t, b)| json!([t, b64(b)]));
            out.insert("web".into(), Value::Array(web.collect()));
        }
        Value::Object(out).to_string()
    }

    /// The page script's JSON ([`Payload::to_json`]); `None` when it is not one. Entries of the
    /// wrong shape are left out.
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        let string = |v: &Value| v.as_str().map(str::to_owned);
        let bytes = |v: &Value| {
            base64::engine::general_purpose::STANDARD
                .decode(v.as_str()?)
                .ok()
        };
        let list = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        };
        Some(Payload {
            text: object.get("text").and_then(string),
            html: object.get("html").and_then(string),
            files: list("files")
                .filter_map(|f| {
                    Some(File {
                        name: string(f.get("name")?)?,
                        mime: string(f.get("type")?)?,
                        bytes: bytes(f.get("data")?)?,
                    })
                })
                .collect(),
            custom: list("custom")
                .filter_map(|p| Some((string(p.get(0)?)?, string(p.get(1)?)?)))
                .collect(),
            web: list("web")
                .filter_map(|p| Some((string(p.get(0)?)?, bytes(p.get(1)?)?)))
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_is_in_flight_until_answered_or_given_up() {
        let s = Duration::from_secs;
        // (started, now, in flight)
        for (started, now, flight) in [
            (None, s(10), false),
            (Some(s(10)), s(10), true),
            (
                Some(s(10)),
                s(10) + READ_IN_FLIGHT - Duration::from_millis(1),
                true,
            ),
            (Some(s(10)), s(10) + READ_IN_FLIGHT, false),
            (Some(s(10)), s(60), false),
            // a clock that read earlier than the start: just started
            (Some(s(10)), s(9), true),
        ] {
            assert_eq!(in_flight(started, now), flight, "{started:?} {now:?}");
        }
    }

    #[test]
    fn only_the_frame_the_user_is_in_reaches_the_clipboard() {
        // Every fact: a write or the primary selection needs the three, a read also no read
        // of its tab in flight.
        for bits in 0u8..16 {
            let asker = Asker {
                key_page: bits & 8 != 0,
                window_focused: bits & 4 != 0,
                frame_focused: bits & 2 != 0,
                reading: bits & 1 != 0,
            };
            for (ask, allowed) in [
                (Ask::Write, bits >> 1 == 0b111),
                (Ask::Primary, bits >> 1 == 0b111),
                (Ask::Read, bits == 0b1110),
            ] {
                assert_eq!(admit(ask, asker), allowed, "{ask:?} {asker:?}");
            }
        }
    }

    #[test]
    fn a_paste_reads_each_kind_once_from_its_best_type() {
        let web0 = "application/web;type=\"custom/format0\"";
        let all = [
            "text/plain",
            "UTF8_STRING",
            "TEXT/HTML",
            "image/jpeg",
            "image/png",
            "x-special/gnome-copied-files",
            "text/uri-list",
            CUSTOM_DATA,
            WEB_MAP,
            web0,
        ];
        let cases: &[(&[&str], Reader, &[(Kind, &str)])] = &[
            (
                &all,
                Reader::Paste,
                &[
                    (Kind::Text, "UTF8_STRING"),
                    (Kind::Html, "TEXT/HTML"),
                    (Kind::Files, "text/uri-list"),
                    (Kind::CustomData, CUSTOM_DATA),
                    (Kind::Picture, "image/png"),
                ],
            ),
            // `navigator.clipboard.read`: no files, no custom data; the `web ` formats.
            (
                &all,
                Reader::AsyncApi,
                &[
                    (Kind::Text, "UTF8_STRING"),
                    (Kind::Html, "TEXT/HTML"),
                    (Kind::WebMap, WEB_MAP),
                    (Kind::Picture, "image/png"),
                    (Kind::Web, web0),
                ],
            ),
            // The primary selection, a field, `readText`: text alone.
            (&all, Reader::Text, &[(Kind::Text, "UTF8_STRING")]),
            (&["image/png"], Reader::Text, &[]),
            // A picture other than PNG; GNOME's file list alone.
            (
                &["image/webp", "x-special/gnome-copied-files"],
                Reader::Paste,
                &[(Kind::Files, GNOME_FILES), (Kind::Picture, "image/webp")],
            ),
            // A `web ` format without its map is not read; nor is an unknown type.
            (&[web0, "application/x-other"], Reader::AsyncApi, &[]),
            (&[], Reader::Paste, &[]),
        ];
        for (mimes, reader, want) in cases {
            let want: Vec<_> = want.iter().map(|(k, m)| (*k, m.to_string())).collect();
            assert_eq!(plan(*mimes, *reader), want, "{mimes:?} {reader:?}");
        }
    }

    #[test]
    fn a_paste_holds_what_was_read() {
        let map = br#"{"text/x-a":"application/web;type=\"custom/format0\""}"#.to_vec();
        let utf16: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain("<b>é</b>\0".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let custom = dnd::custom_data(&[("text/x-c".into(), "c".into())]);
        let part = |kind, mime: &str, bytes: &[u8]| (kind, mime.to_owned(), bytes.to_vec());
        let (payload, paths) = read(vec![
            part(Kind::Text, "text/plain", b"a\r\nb\rc"),
            part(Kind::Html, HTML, &utf16),
            part(Kind::Picture, "IMAGE/JPEG", b"jpg"),
            part(Kind::CustomData, CUSTOM_DATA, &custom),
            part(Kind::WebMap, WEB_MAP, &map),
            part(Kind::Web, "application/web;type=\"custom/format0\"", b"w"),
            part(
                Kind::Web,
                "application/web;type=\"custom/format9\"",
                b"unmapped",
            ),
        ]);
        assert_eq!(
            payload,
            Payload {
                text: Some("a\nb\nc".into()),
                html: Some("<b>é</b>".into()),
                files: vec![File {
                    name: "image.jpg".into(),
                    mime: "image/jpeg".into(),
                    bytes: b"jpg".to_vec(),
                }],
                custom: vec![("text/x-c".into(), "c".into())],
                web: vec![("text/x-a".into(), b"w".to_vec())],
            }
        );
        assert!(paths.is_empty());
        let lists: &[(&str, &[u8], &[&str])] = &[
            (
                "text/uri-list",
                b"file:///tmp/a%20b\r\nfile:///c\r\n",
                &["/tmp/a b", "/c"],
            ),
            (
                GNOME_FILES,
                b"copy\nfile:///tmp/a\nfile:///tmp/b",
                &["/tmp/a", "/tmp/b"],
            ),
            (GNOME_FILES, b"cut\nfile:///tmp/a", &["/tmp/a"]),
            // A link is no file.
            ("text/uri-list", b"https://a.test/", &[]),
            (GNOME_FILES, b"copy", &[]),
        ];
        for (mime, bytes, want) in lists {
            let (_, paths) = read(vec![part(Kind::Files, mime, bytes)]);
            let want: Vec<PathBuf> = want.iter().map(PathBuf::from).collect();
            assert_eq!(paths, want, "{mime} {bytes:?}");
        }
    }

    #[test]
    fn copied_files_stop_at_the_cap() {
        let mut payload = Payload::default();
        add_file(&mut payload, "shot.PNG".into(), vec![1; 10]);
        add_file(&mut payload, "notes.txt".into(), vec![2; 10]);
        add_file(&mut payload, "big.bin".into(), vec![0; FILES_CAP as usize]);
        let got: Vec<_> = payload
            .files
            .iter()
            .map(|f| (f.name.as_str(), f.mime.as_str()))
            .collect();
        assert_eq!(got, [("shot.PNG", "image/png"), ("notes.txt", "")]);
    }

    #[test]
    fn a_copy_is_offered_as_chromium_on_linux_offers_it() {
        let payload = Payload {
            text: Some("t".into()),
            html: Some("<b>t</b>".into()),
            files: vec![File {
                name: "image.png".into(),
                mime: "image/png".into(),
                bytes: b"png".to_vec(),
            }],
            custom: vec![("text/x-c".into(), "c".into())],
            web: vec![("text/x-a".into(), b"w".to_vec())],
        };
        let offered = offers(&payload);
        let types: Vec<_> = offered.iter().map(|(m, _)| m.as_str()).collect();
        let web0 = "application/web;type=\"custom/format0\"";
        assert_eq!(
            types,
            [
                "image/png",
                HTML,
                "text/plain;charset=utf-8",
                "UTF8_STRING",
                "text/plain",
                CUSTOM_DATA,
                WEB_MAP,
                web0
            ]
        );
        let body = |mime: &str| offered.iter().find(|(m, _)| m == mime).unwrap().1.to_vec();
        assert_eq!(body(HTML), format!("{HTML_HEADER}<b>t</b>").into_bytes());
        assert_eq!(body("UTF8_STRING"), b"t");
        assert_eq!(
            dnd::read_custom_data(&body(CUSTOM_DATA)),
            Some(vec![("text/x-c".to_owned(), "c".to_owned())])
        );
        // What is offered reads back as it was copied, each reader its own types.
        let read_back = |reader| {
            let parts = plan(&types, reader)
                .into_iter()
                .map(|(kind, mime)| {
                    let bytes = body(&mime);
                    (kind, mime, bytes)
                })
                .collect();
            read(parts).0
        };
        let html = Some(format!("{HTML_HEADER}<b>t</b>"));
        let pasted = Payload {
            html: html.clone(),
            web: vec![],
            ..payload.clone()
        };
        assert_eq!(read_back(Reader::Paste), pasted);
        let custom = vec![];
        assert_eq!(
            read_back(Reader::AsyncApi),
            Payload {
                html,
                custom,
                ..payload
            }
        );
        // Text alone is three text types; nothing is no offer.
        let text = Payload {
            text: Some("t".into()),
            ..Payload::default()
        };
        assert_eq!(offers(&text).len(), 3);
        assert!(offers(&Payload::default()).is_empty());
    }

    #[test]
    fn the_page_json_round_trips() {
        let cases = [
            Payload::default(),
            Payload {
                text: Some("a\n\"b\"".into()),
                ..Payload::default()
            },
            Payload {
                text: Some(String::new()),
                html: Some("<i>x</i>".into()),
                files: vec![File {
                    name: "a.png".into(),
                    mime: String::new(),
                    bytes: vec![0, 255, 7],
                }],
                custom: vec![("text/x-c".into(), "c".into())],
                web: vec![("text/x-a".into(), vec![1, 2])],
            },
        ];
        for payload in cases {
            assert_eq!(
                Payload::from_json(&payload.to_json()),
                Some(payload.clone())
            );
        }
        assert_eq!(Payload::default().to_json(), "{}");
        // What the page wrote wrong is left out; what is no JSON object is nothing.
        let odd = r#"{"text":1,"files":[{"name":"a"}],"custom":[["t"]],"web":[["t","%%"]]}"#;
        assert_eq!(Payload::from_json(odd), Some(Payload::default()));
        assert_eq!(Payload::from_json("[]"), None);
        assert_eq!(Payload::from_json("text"), None);
    }

    #[test]
    fn copy_image_markup_escapes_the_url() {
        let cases = [
            (
                "https://a.test/cat.png",
                "<img src=\"https://a.test/cat.png\"/>",
            ),
            (
                "https://a.test/?a=1&b=\"<>",
                "<img src=\"https://a.test/?a=1&amp;b=&quot;&lt;&gt;\"/>",
            ),
        ];
        for (url, want) in cases {
            assert_eq!(image_markup(url), want);
        }
    }
}
