//! Which responses are media, and the list of what a tab's page offers. The response hook sees
//! every response's headers ([`Response`]); [`classify`] keeps whole files (`video/*`,
//! `audio/*`, or a media extension), HLS playlists and DASH manifests, and drops stream
//! segments (`.ts`, `.m4s`, MPEG-TS and ISO segment MIME types), small files (ad pings, UI
//! sounds: under the `media.min_bytes` floor), and failed answers. A ranged answer (206) counts
//! the whole file from its `Content-Range`. YouTube's own player traffic is skipped: it is
//! SABR/UMP, not a file ([`super::youtube`] lists YouTube's formats instead).
//!
//! [`TabMedia`] is one tab's list, with one pure [`TabMedia::update`]: a seen candidate joins
//! once (by URL without its fragment), up to [`MAX_ITEMS`]; a manifest is resolved once
//! ([`MediaEffect::Resolve`]); a navigation to another page clears the list.
//!
//! A service worker's own fetch has no tab: the page's request went to the worker, which
//! answered it from a fetch of its own (or its cache). [`worker_tab`] credits such a response
//! to a tab of the worker's container showing a page of the worker's origin.

use super::{Container, Refusal, Resolution, Source, Track, Variant};
use crate::space::ContainerId;
use crate::tab::TabId;

/// Most streams one tab lists.
pub const MAX_ITEMS: usize = 32;

/// A response, as the response hook reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Response<'a> {
    pub url: &'a str,
    /// Lower case, without parameters.
    pub mime: &'a str,
    pub status: i32,
    pub content_length: Option<u64>,
    pub content_range: &'a str,
    pub accepts_ranges: bool,
}

/// What kind of stream a candidate is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A whole file.
    File,
    Hls,
    Dash,
    /// A YouTube video, listed through the player API.
    YouTube,
}

/// A response worth listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub url: String,
    pub kind: Kind,
    /// [`Kind::File`]: what the file is.
    pub track: Track,
    pub container: Container,
    pub size: Option<u64>,
    pub ranges: bool,
}

const HLS_MIMES: [&str; 4] = [
    "application/vnd.apple.mpegurl",
    "application/x-mpegurl",
    "audio/mpegurl",
    "audio/x-mpegurl",
];
const DASH_MIMES: [&str; 1] = ["application/dash+xml"];
/// Stream segments, never listed on their own.
const SEGMENT_MIMES: [&str; 3] = ["video/mp2t", "video/iso.segment", "audio/iso.segment"];
const SEGMENT_EXTS: [&str; 7] = ["ts", "m4s", "m4f", "cmfv", "cmfa", "aac", "vtt"];

/// The total size of a `Content-Range: bytes a-b/total` answer.
pub fn range_total(content_range: &str) -> Option<u64> {
    let (_, total) = content_range
        .trim()
        .strip_prefix("bytes")?
        .rsplit_once('/')?;
    total.trim().parse().ok()
}

/// The response as a candidate, or `None` when it is not worth listing. `min_bytes`: files
/// known to be smaller are dropped.
pub fn classify(r: &Response, min_bytes: u64) -> Option<Candidate> {
    if !matches!(r.status, 200 | 206) || !r.url.starts_with("http") {
        return None;
    }
    let host = ::url::Url::parse(r.url)
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    if host.ends_with("googlevideo.com") || r.mime == "application/vnd.yt-ump" {
        return None;
    }
    let ext = super::path_ext(r.url).unwrap_or_default();
    let url = r.url.split('#').next().unwrap_or_default().to_owned();
    let stream = |kind| Candidate {
        url: url.clone(),
        kind,
        track: Track::Muxed,
        container: Container::Other,
        size: None,
        ranges: false,
    };
    if HLS_MIMES.contains(&r.mime) || ext == "m3u8" {
        return Some(stream(Kind::Hls));
    }
    if DASH_MIMES.contains(&r.mime) || ext == "mpd" {
        return Some(stream(Kind::Dash));
    }
    if SEGMENT_MIMES.contains(&r.mime) || SEGMENT_EXTS.contains(&ext.as_str()) {
        return None;
    }
    let by_mime = r.mime.starts_with("video/") || r.mime.starts_with("audio/");
    let container = Container::of_mime(r.mime).or_else(|| Container::of_url(r.url));
    let by_ext = !r.mime.starts_with("text/") && Container::of_url(r.url).is_some();
    if !by_mime && !by_ext {
        return None;
    }
    let size = match r.status {
        206 => range_total(r.content_range),
        _ => r.content_length,
    };
    if size.is_some_and(|s| s < min_bytes) {
        return None;
    }
    let audio = r.mime.starts_with("audio/")
        || (matches!(
            container,
            Some(Container::M4a | Container::Mp3 | Container::Ogg)
        ) && !r.mime.starts_with("video/"));
    Some(Candidate {
        url,
        kind: Kind::File,
        track: if audio { Track::Audio } else { Track::Muxed },
        container: container.unwrap_or(Container::Other),
        size,
        ranges: r.status == 206 || r.accepts_ranges,
    })
}

/// A tab with a page, as [`worker_tab`] weighs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveTab<'a> {
    pub tab: TabId,
    pub container: ContainerId,
    /// The page it shows (its committed address).
    pub url: &'a str,
    pub last_active_ms: i64,
}

/// The tab a service worker's response is for: of the worker's `container`, the tabs whose
/// page has the worker's origin (`initiator`, the origin that made the fetch), the one used
/// last. `None` when no tab shows that site (a worker running for a page since closed).
pub fn worker_tab(initiator: &str, container: ContainerId, tabs: &[LiveTab]) -> Option<TabId> {
    let origin = crate::site::origin_of(initiator)?;
    tabs.iter()
        .filter(|t| {
            t.container == container && crate::site::origin_of(t.url).as_ref() == Some(&origin)
        })
        .max_by_key(|t| t.last_active_ms)
        .map(|t| t.tab)
}

/// What is known of a listed stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// Its manifest is being fetched.
    Resolving,
    Ready(Vec<Variant>),
    Refused(Refusal),
}

/// One stream the page offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detected {
    /// Unique within the tab's list (never reused while the list lives).
    pub id: u32,
    pub url: String,
    pub kind: Kind,
    /// The stream's own title, when its source names one.
    pub title: Option<String>,
    /// What the menu says of it besides its rows ([`Resolution::note`]).
    pub note: Option<String>,
    pub found: Found,
}

impl Detected {
    /// Whether the menu lists it (and the glyph shows for it): not a manifest that failed to
    /// read, unless it is a YouTube lookup, whose failure the user must be told.
    pub fn is_shown(&self) -> bool {
        match &self.found {
            Found::Refused(Refusal::Failed(_)) => self.kind == Kind::YouTube,
            _ => true,
        }
    }

    /// The URL is a part of this stream: a segment or a playlist of it.
    fn contains(&self, url: &str) -> bool {
        matches!(&self.found, Found::Ready(variants) if variants.iter().any(|v| match &v.source {
            Source::File { url: u, .. } | Source::HlsPlaylist(u) => u == url,
            Source::Planned(plan) => plan.segments.iter().any(|s| s.url == url),
        }))
    }
}

/// The input of [`TabMedia::update`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaEvent {
    Seen(Candidate),
    /// A stream that would be looked up but may not be: listed as [`Refusal::LookupOff`], no
    /// request made (a YouTube page with `media.youtube` off).
    Declined(Candidate),
    /// A manifest fetched and parsed (or not).
    Resolved {
        id: u32,
        result: Result<Resolution, Refusal>,
    },
    /// The main frame committed another page.
    Navigated,
}

/// What the host does for the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaEffect {
    /// Fetch and parse the stream's manifest (or ask the site's player API), then report
    /// [`MediaEvent::Resolved`].
    Resolve { id: u32, url: String, kind: Kind },
}

/// One tab's streams.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabMedia {
    pub items: Vec<Detected>,
    next_id: u32,
}

impl TabMedia {
    pub fn update(mut self, event: MediaEvent) -> (TabMedia, Vec<MediaEffect>) {
        match event {
            MediaEvent::Navigated => (
                TabMedia {
                    items: vec![],
                    ..self
                },
                vec![],
            ),
            MediaEvent::Resolved { id, result } => {
                let Some(at) = self
                    .items
                    .iter()
                    .position(|i| i.id == id && i.found == Found::Resolving)
                else {
                    return (self, vec![]);
                };
                match result {
                    Ok(r) => {
                        let item = &mut self.items[at];
                        item.title = r.title;
                        item.note = r.note;
                        item.found = Found::Ready(r.variants);
                        // The player loads a master's variant and rendition playlists while
                        // the master resolves: they are listed until now, as streams of
                        // their own.
                        let master = self.items[at].clone();
                        self.items
                            .retain(|i| i.id == master.id || !master.contains(&i.url));
                    }
                    Err(refusal) => self.items[at].found = Found::Refused(refusal),
                }
                (self, vec![])
            }
            MediaEvent::Declined(c) => {
                if self.items.len() >= MAX_ITEMS || self.knows(&c.url) {
                    return (self, vec![]);
                }
                let id = self.next_id;
                self.next_id += 1;
                self.items.push(Detected {
                    id,
                    url: c.url,
                    kind: c.kind,
                    title: None,
                    note: None,
                    found: Found::Refused(Refusal::LookupOff),
                });
                (self, vec![])
            }
            MediaEvent::Seen(c) => {
                if self.items.len() >= MAX_ITEMS || self.knows(&c.url) {
                    return (self, vec![]);
                }
                let id = self.next_id;
                self.next_id += 1;
                let (found, effects) = match c.kind {
                    Kind::File => (
                        Found::Ready(vec![Variant::file(
                            &c.url,
                            c.track,
                            c.container,
                            c.size,
                            c.ranges,
                        )]),
                        vec![],
                    ),
                    kind => (
                        Found::Resolving,
                        vec![MediaEffect::Resolve {
                            id,
                            url: c.url.clone(),
                            kind,
                        }],
                    ),
                };
                self.items.push(Detected {
                    id,
                    url: c.url,
                    kind: c.kind,
                    title: None,
                    note: None,
                    found,
                });
                (self, effects)
            }
        }
    }

    /// The URL is listed, or is a part of a listed stream (a segment or a playlist of it).
    fn knows(&self, url: &str) -> bool {
        self.items
            .iter()
            .any(|item| item.url == url || item.contains(url))
    }

    /// Something to show: a stream to save, or one to explain (protected, live, a lookup that
    /// is off or failed). What the page's own manifest reads as nothing (a failed fetch, a shape
    /// not known) shows nothing; a YouTube video, which the user asked about, always says why.
    pub fn has_media(&self) -> bool {
        self.items.iter().any(Detected::is_shown)
    }

    pub fn get(&self, id: u32) -> Option<&Detected> {
        self.items.iter().find(|i| i.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(url: &'static str, mime: &'static str) -> Response<'static> {
        Response {
            url,
            mime,
            status: 200,
            content_length: Some(1 << 20),
            content_range: "",
            accepts_ranges: false,
        }
    }

    #[test]
    fn classify_table() {
        let kind = |r: Response| classify(&r, 65_536).map(|c| (c.kind, c.track, c.container));
        let file = |t, c| Some((Kind::File, t, c));
        let cases = [
            (
                resp("https://a.test/v.mp4", "video/mp4"),
                file(Track::Muxed, Container::Mp4),
            ),
            (
                resp("https://a.test/videoplayback?x=1", "video/webm"),
                file(Track::Muxed, Container::Webm),
            ),
            (
                resp("https://a.test/song", "audio/mpeg"),
                file(Track::Audio, Container::Mp3),
            ),
            (
                resp("https://a.test/clip.m4a", "application/octet-stream"),
                file(Track::Audio, Container::M4a),
            ),
            (
                resp("https://a.test/clip.mov", ""),
                file(Track::Muxed, Container::Mp4),
            ),
            (
                resp("https://a.test/x/master.m3u8?tok=1", "text/plain"),
                Some((Kind::Hls, Track::Muxed, Container::Other)),
            ),
            (
                resp("https://a.test/live", "application/vnd.apple.mpegurl"),
                Some((Kind::Hls, Track::Muxed, Container::Other)),
            ),
            (
                resp("https://a.test/m.mpd", "application/xml"),
                Some((Kind::Dash, Track::Muxed, Container::Other)),
            ),
            (
                resp("https://a.test/m", "application/dash+xml"),
                Some((Kind::Dash, Track::Muxed, Container::Other)),
            ),
            (resp("https://a.test/seg1.ts", "video/mp2t"), None),
            (resp("https://a.test/seg1", "video/mp2t"), None),
            (resp("https://a.test/c/seg-3.m4s", "video/mp4"), None),
            (resp("https://a.test/page.html", "text/html"), None),
            (resp("https://a.test/a.mp4", "text/html"), None),
            (resp("https://a.test/i.png", "image/png"), None),
            (
                resp(
                    "https://rr3---sn.googlevideo.com/videoplayback?itag=18",
                    "video/mp4",
                ),
                None,
            ),
            (resp("https://a.test/x", "application/vnd.yt-ump"), None),
            (resp("data:video/mp4;base64,AA", "video/mp4"), None),
        ];
        for (r, want) in cases {
            assert_eq!(kind(r), want, "{}", r.url);
        }
    }

    #[test]
    fn a_service_workers_response_goes_to_its_sites_tab() {
        let (home, work) = (ContainerId(0), ContainerId(1));
        let tab = |id, container, url, last_active_ms| LiveTab {
            tab: TabId(id),
            container,
            url,
            last_active_ms,
        };
        let tabs = [
            tab(1, home, "https://a.test/watch/1", 10),
            tab(2, home, "https://a.test/other", 30),
            tab(3, work, "https://a.test/watch/9", 50),
            tab(4, home, "https://b.test/", 90),
            tab(5, home, "https://a.test:8443/x", 99),
            tab(6, home, "about:blank", 100),
        ];
        let cases = [
            (
                "the newest of the site's tabs",
                "https://a.test",
                home,
                Some(2),
            ),
            ("another container's own", "https://a.test", work, Some(3)),
            (
                "a trailing slash and case",
                "HTTPS://A.TEST/",
                home,
                Some(2),
            ),
            (
                "another port is another origin",
                "https://a.test:8443",
                home,
                Some(5),
            ),
            ("no tab of the site", "https://c.test", home, None),
            ("no tab of the container", "https://b.test", work, None),
            ("an opaque initiator", "null", home, None),
            ("no initiator", "", home, None),
        ];
        for (name, initiator, container, want) in cases {
            assert_eq!(
                worker_tab(initiator, container, &tabs),
                want.map(TabId),
                "{name}"
            );
        }
        assert_eq!(worker_tab("https://a.test", home, &[]), None);
    }

    #[test]
    fn sizes_floors_and_ranges() {
        let small = Response {
            content_length: Some(1000),
            ..resp("https://a.test/ding.mp3", "audio/mpeg")
        };
        assert_eq!(classify(&small, 65_536), None);
        assert!(classify(&small, 0).is_some());
        let unknown = Response {
            content_length: None,
            ..small
        };
        assert_eq!(classify(&unknown, 65_536).unwrap().size, None);
        let ranged = Response {
            status: 206,
            content_length: Some(100),
            content_range: "bytes 0-99/5000000",
            ..resp("https://a.test/v.mp4", "video/mp4")
        };
        let c = classify(&ranged, 65_536).unwrap();
        assert_eq!((c.size, c.ranges), (Some(5_000_000), true));
        let failed = Response {
            status: 404,
            ..resp("https://a.test/v.mp4", "video/mp4")
        };
        assert_eq!(classify(&failed, 0), None);
        assert_eq!(range_total("bytes 0-1/*"), None);
        assert_eq!(range_total("bytes */77"), Some(77));
        let accepts = Response {
            accepts_ranges: true,
            ..resp("https://a.test/v.mp4#t=3", "video/mp4")
        };
        let c = classify(&accepts, 0).unwrap();
        assert_eq!((c.url.as_str(), c.ranges), ("https://a.test/v.mp4", true));
    }

    fn seen(url: &str, kind: Kind) -> MediaEvent {
        MediaEvent::Seen(Candidate {
            url: url.into(),
            kind,
            track: Track::Muxed,
            container: Container::Mp4,
            size: Some(9),
            ranges: false,
        })
    }

    #[test]
    fn the_list_dedupes_resolves_and_clears() {
        let m = TabMedia::default();
        let (m, fx) = m.update(seen("https://a.test/v.mp4", Kind::File));
        assert!(fx.is_empty());
        assert!(matches!(&m.items[0].found, Found::Ready(v) if v.len() == 1));
        let (m, fx) = m.update(seen("https://a.test/v.mp4", Kind::File));
        assert!(fx.is_empty());
        assert_eq!(m.items.len(), 1);
        let (m, fx) = m.update(seen("https://a.test/hls.m3u8", Kind::Hls));
        assert_eq!(
            fx,
            vec![MediaEffect::Resolve {
                id: 1,
                url: "https://a.test/hls.m3u8".into(),
                kind: Kind::Hls
            }]
        );
        let plan = super::super::Plan {
            segments: vec![super::super::Segment::whole("https://a.test/s1.mp4")],
            container: Container::Ts,
            periods: vec![],
        };
        let variant = Variant {
            source: Source::Planned(plan),
            ..Variant::file("x", Track::Muxed, Container::Ts, None, false)
        };
        let resolved = Resolution {
            variants: vec![variant],
            title: Some("T".into()),
            note: None,
        };
        let (m, _) = m.update(MediaEvent::Resolved {
            id: 1,
            result: Ok(resolved.clone()),
        });
        assert_eq!(m.get(1).unwrap().title.as_deref(), Some("T"));
        // A segment of the resolved stream is not a stream of its own.
        let (m, _) = m.update(seen("https://a.test/s1.mp4", Kind::File));
        assert_eq!(m.items.len(), 2);
        // A late answer for a resolved item changes nothing.
        let (m2, _) = m.clone().update(MediaEvent::Resolved {
            id: 1,
            result: Err(Refusal::Live),
        });
        assert_eq!(m2, m);
        assert!(m.has_media());
        let (m, _) = m.update(MediaEvent::Navigated);
        assert!(m.items.is_empty() && !m.has_media());
        // Ids are never reused.
        let (m, _) = m.update(seen("https://a.test/v.mp4", Kind::File));
        assert_eq!(m.items[0].id, 2);
    }

    #[test]
    fn playlists_seen_while_the_master_resolves_fold_into_it() {
        let m = TabMedia::default();
        let (m, _) = m.update(seen("https://a.test/master.m3u8", Kind::Hls));
        let (m, _) = m.update(seen("https://a.test/v720.m3u8", Kind::Hls));
        let (m, _) = m.update(seen("https://a.test/audio.m3u8", Kind::Hls));
        let (m, _) = m.update(seen("https://a.test/other.mp4", Kind::File));
        assert_eq!(m.items.len(), 4);
        let playlist = |url: &str, track| Variant {
            source: Source::HlsPlaylist(url.into()),
            ..Variant::file("x", track, Container::Ts, None, false)
        };
        let (m, _) = m.update(MediaEvent::Resolved {
            id: 0,
            result: Ok(Resolution {
                variants: vec![
                    playlist("https://a.test/v720.m3u8", Track::Video),
                    playlist("https://a.test/audio.m3u8", Track::Audio),
                ],
                title: None,
                note: None,
            }),
        });
        let urls: Vec<&str> = m.items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            vec!["https://a.test/master.m3u8", "https://a.test/other.mp4"]
        );
        // The variant's own resolve answering late finds nothing.
        let (m2, _) = m.clone().update(MediaEvent::Resolved {
            id: 1,
            result: Err(Refusal::Live),
        });
        assert_eq!(m2, m);
    }

    #[test]
    fn the_list_is_capped_and_failures_hide_the_glyph() {
        let mut m = TabMedia::default();
        for i in 0..MAX_ITEMS + 5 {
            m = m
                .update(seen(&format!("https://a.test/{i}.mp4"), Kind::File))
                .0;
        }
        assert_eq!(m.items.len(), MAX_ITEMS);
        let (m, _) = TabMedia::default().update(seen("https://a.test/d.mpd", Kind::Dash));
        let (failed, _) = m.clone().update(MediaEvent::Resolved {
            id: 0,
            result: Err(Refusal::Failed("x".into())),
        });
        assert!(!failed.has_media());
        // A YouTube lookup that failed says so; one that is off says so, with no request.
        let (yt, fx) =
            TabMedia::default().update(seen("https://youtu.be/dQw4w9WgXcQ", Kind::YouTube));
        assert_eq!(fx.len(), 1);
        let (yt, _) = yt.update(MediaEvent::Resolved {
            id: 0,
            result: Err(Refusal::Failed("Sign in".into())),
        });
        assert!(yt.has_media());
        let declined = MediaEvent::Declined(Candidate {
            url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            kind: Kind::YouTube,
            track: Track::Muxed,
            container: Container::Mp4,
            size: None,
            ranges: false,
        });
        let (off, fx) = TabMedia::default().update(declined);
        assert!(fx.is_empty() && off.has_media());
        assert_eq!(off.items[0].found, Found::Refused(Refusal::LookupOff));
        let (protected, _) = m.update(MediaEvent::Resolved {
            id: 0,
            result: Err(Refusal::Protected),
        });
        assert!(protected.has_media());
    }
}
