//! The media downloader's pure half: which responses are media ([`detect`]), what a manifest
//! offers ([`hls`], [`dash`], [`youtube`]), which variant to take ([`select`]), the life of one
//! download ([`job`]) and its file name ([`name`]). The engine fetches and writes; the host
//! routes. Nothing here decrypts DRM: a stream that needs a DRM system (EME: Widevine,
//! PlayReady, FairPlay, ClearKey) is [`Refusal::Protected`] and never fetched. Plain HLS
//! AES-128 with a fetchable key is transport encryption and is handled ([`hls::decrypt`]).

pub mod dash;
pub mod detect;
pub mod hls;
pub mod job;
pub mod name;
pub mod select;
pub mod youtube;

/// The file format of a variant's bytes, which names the saved file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    Mp4,
    Webm,
    /// MPEG-TS (HLS segments joined as they are).
    Ts,
    M4a,
    Mp3,
    Ogg,
    Other,
}

/// MIME type and file extension of each container, one table.
const CONTAINERS: &[(Container, &str, &[&str])] = &[
    (
        Container::Mp4,
        "mp4",
        &["video/mp4", "video/quicktime", "video/x-m4v"],
    ),
    (Container::Webm, "webm", &["video/webm", "audio/webm"]),
    (Container::Ts, "ts", &["video/mp2t"]),
    (
        Container::M4a,
        "m4a",
        &["audio/mp4", "audio/x-m4a", "audio/aac"],
    ),
    (Container::Mp3, "mp3", &["audio/mpeg", "audio/mp3"]),
    (
        Container::Ogg,
        "ogg",
        &["audio/ogg", "video/ogg", "audio/opus"],
    ),
];

impl Container {
    /// The saved file's extension.
    pub fn ext(self) -> &'static str {
        CONTAINERS
            .iter()
            .find(|(c, ..)| *c == self)
            .map_or("bin", |(_, ext, _)| ext)
    }

    /// The container of a MIME type (parameters ignored).
    pub fn of_mime(mime: &str) -> Option<Container> {
        let mime = mime.split(';').next()?.trim().to_ascii_lowercase();
        CONTAINERS
            .iter()
            .find(|(.., mimes)| mimes.contains(&mime.as_str()))
            .map(|(c, ..)| *c)
    }

    /// The container a URL's path extension names (`mov` and `m4v` are MP4).
    pub fn of_url(url: &str) -> Option<Container> {
        Some(match path_ext(url)?.as_str() {
            "mov" | "m4v" => Container::Mp4,
            ext => CONTAINERS.iter().find(|(_, e, _)| *e == ext)?.0,
        })
    }
}

/// The lower-cased extension of a URL's path (no query, no fragment).
pub fn path_ext(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    (!ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// `relative` resolved against `base` (an absolute URL); `relative` itself when it does not
/// resolve.
pub fn join_url(base: &str, relative: &str) -> String {
    ::url::Url::parse(base)
        .and_then(|b| b.join(relative))
        .map_or_else(|_| relative.to_owned(), String::from)
}

/// What a variant carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Track {
    /// Video and sound in one stream.
    Muxed,
    Video,
    Audio,
}

/// Part of a resource: `len` bytes from `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub len: u64,
}

impl ByteRange {
    /// The `Range` header's value.
    pub fn header(self) -> String {
        format!("bytes={}-{}", self.start, self.start + self.len.max(1) - 1)
    }
}

/// An HLS `METHOD=AES-128` key: fetched from `uri`, applied with `iv`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AesKey {
    pub uri: String,
    pub iv: [u8; 16],
}

/// One fetch of a download, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub url: String,
    pub range: Option<ByteRange>,
    pub key: Option<AesKey>,
}

impl Segment {
    pub fn whole(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            range: None,
            key: None,
        }
    }
}

/// The segments of one track and what they join into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub segments: Vec<Segment>,
    pub container: Container,
}

/// Bytes per ranged fetch of a single file (its segments).
pub const FILE_CHUNK: u64 = 8 << 20;

/// How a variant's bytes are fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// One file: fetched in [`FILE_CHUNK`] ranges when its size is known and the server takes
    /// ranges (so a download resumes and runs in parallel), else in one piece.
    File {
        url: String,
        size: Option<u64>,
        ranges: bool,
    },
    /// An HLS media playlist, fetched and expanded ([`hls::parse_media`]) when the download
    /// starts.
    HlsPlaylist(String),
    /// Segments known from the manifest itself.
    Planned(Plan),
}

impl Source {
    /// The plan of a source that needs no further fetch (`None` for an HLS playlist).
    pub fn plan(&self, container: Container) -> Option<Plan> {
        match self {
            Source::File { url, size, ranges } => Some(Plan {
                segments: file_segments(url, *size, *ranges),
                container,
            }),
            Source::HlsPlaylist(_) => None,
            Source::Planned(plan) => Some(plan.clone()),
        }
    }
}

/// A single file's fetches: [`FILE_CHUNK`] ranges when it can be ranged, else the whole file.
pub fn file_segments(url: &str, size: Option<u64>, ranges: bool) -> Vec<Segment> {
    match size.filter(|&s| ranges && s > 0) {
        Some(size) => (0..size.div_ceil(FILE_CHUNK))
            .map(|i| {
                let start = i * FILE_CHUNK;
                Segment {
                    url: url.to_owned(),
                    range: Some(ByteRange {
                        start,
                        len: FILE_CHUNK.min(size - start),
                    }),
                    key: None,
                }
            })
            .collect(),
        None => vec![Segment::whole(url)],
    }
}

/// ISO BMFF box types only an encrypted (CENC/CBCS) stream carries: DRM system headers, the
/// track encryption box, encrypted sample entries (ISO/IEC 23001-7). The second guard after
/// the manifest's own markers: an MP4 stream's first bytes (its init segment) that hold one
/// are protected, and the download stops.
const ENCRYPTED_BOXES: [&[u8; 4]; 4] = [b"pssh", b"tenc", b"encv", b"enca"];

/// Whether the start of an MP4 stream names encryption.
pub fn looks_encrypted(init: &[u8]) -> bool {
    init.windows(4)
        .any(|w| ENCRYPTED_BOXES.iter().any(|b| w == *b))
}

/// One way to save a detected stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub track: Track,
    pub container: Container,
    /// Picture height in pixels, when the manifest says.
    pub height: Option<u32>,
    /// Bits per second, when the manifest says.
    pub bandwidth: Option<u64>,
    pub codecs: String,
    /// Bytes, when known.
    pub size: Option<u64>,
    pub source: Source,
    /// HLS: the audio group a video names, or an audio rendition's own group; pairs them.
    pub group: Option<String>,
    /// Headers every fetch of this variant sends (a site's client identity).
    pub headers: Vec<(String, String)>,
}

impl Variant {
    /// A single file as the response hook saw it.
    pub fn file(
        url: &str,
        track: Track,
        container: Container,
        size: Option<u64>,
        ranges: bool,
    ) -> Self {
        Self {
            track,
            container,
            height: None,
            bandwidth: None,
            codecs: String::new(),
            size,
            source: Source::File {
                url: url.to_owned(),
                size,
                ranges,
            },
            group: None,
            headers: vec![],
        }
    }
}

/// Why a stream offers nothing to save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Needs a DRM system: never fetched, never decrypted.
    Protected,
    /// A live stream (no end): recording is not supported.
    Live,
    /// A manifest shape this version does not take (multi-period DASH, ...).
    Unsupported(String),
    /// The site's answer no longer has the shape the parser knows.
    SiteChanged(String),
    /// Fetching or parsing failed.
    Failed(String),
    /// A YouTube video, whose formats wubble asks YouTube for only when the user allowed it
    /// (`media.youtube`, off by default): nothing was asked.
    LookupOff,
}

impl Refusal {
    /// The menu's line for it.
    pub fn message(&self) -> String {
        match self {
            Refusal::Protected => "Protected, cannot be saved".into(),
            Refusal::Live => "Live stream, cannot be saved".into(),
            Refusal::Unsupported(what) => format!("Not supported: {what}"),
            Refusal::SiteChanged(what) => format!("This site changed: {what}"),
            Refusal::Failed(why) => format!("Cannot read the stream: {why}"),
            Refusal::LookupOff => {
                "Saving a YouTube video asks YouTube for its formats: off in Settings".into()
            }
        }
    }
}

/// What resolving a manifest gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub variants: Vec<Variant>,
    /// The stream's own title (a site's player data), over the page title.
    pub title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containers_from_mime_and_url() {
        assert_eq!(
            Container::of_mime("Video/MP4; codecs=x"),
            Some(Container::Mp4)
        );
        assert_eq!(Container::of_mime("audio/mp4"), Some(Container::M4a));
        assert_eq!(Container::of_mime("text/html"), None);
        assert_eq!(
            Container::of_url("https://a.test/v/clip.MOV?x=1#t"),
            Some(Container::Mp4)
        );
        assert_eq!(
            Container::of_url("https://a.test/a.webm"),
            Some(Container::Webm)
        );
        assert_eq!(Container::of_url("https://a.test/dir.v/"), None);
        assert_eq!(Container::of_url("https://a.test"), None);
        assert_eq!(Container::Ts.ext(), "ts");
        assert_eq!(Container::Other.ext(), "bin");
    }

    #[test]
    fn urls_join_against_the_base() {
        assert_eq!(
            join_url("https://a.test/hls/master.m3u8", "v1/x.m3u8"),
            "https://a.test/hls/v1/x.m3u8"
        );
        assert_eq!(
            join_url("https://a.test/hls/m.m3u8", "/k.bin"),
            "https://a.test/k.bin"
        );
        assert_eq!(
            join_url("https://a.test/m", "https://b.test/s.ts"),
            "https://b.test/s.ts"
        );
        assert_eq!(join_url("not a url", "s.ts"), "s.ts");
    }

    #[test]
    fn encrypted_init_segments_are_seen() {
        let clear = [&b"\0\0\0\x18ftypiso6"[..], b"\0\0\0\x10moov\0\0\0\x08avc1"].concat();
        assert!(!looks_encrypted(&clear));
        let sealed = [&clear[..], b"\0\0\0\x20pssh\0\0"].concat();
        assert!(looks_encrypted(&sealed));
        assert!(looks_encrypted(b"....encv...."));
        assert!(!looks_encrypted(b""));
    }

    #[test]
    fn a_ranged_file_splits_into_chunks() {
        let segs = file_segments("u", Some(FILE_CHUNK * 2 + 5), true);
        assert_eq!(segs.len(), 3);
        assert_eq!(
            segs[2].range,
            Some(ByteRange {
                start: FILE_CHUNK * 2,
                len: 5
            })
        );
        assert_eq!(
            segs[0].range.unwrap().header(),
            format!("bytes=0-{}", FILE_CHUNK - 1)
        );
        assert_eq!(
            file_segments("u", Some(10), false),
            vec![Segment::whole("u")]
        );
        assert_eq!(file_segments("u", None, true), vec![Segment::whole("u")]);
        assert_eq!(file_segments("u", Some(0), true), vec![Segment::whole("u")]);
    }
}
