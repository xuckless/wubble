//! HLS playlists: a master playlist's variants and a media playlist's segments, parsed by
//! `m3u8-rs` (MIT). The rules the crate leaves to its caller follow RFC 8216 and hls.js
//! (video-dev/hls.js, Apache-2.0; rules ported, no code copied): an `EXT-X-KEY` and an
//! `EXT-X-MAP` hold for every later segment until the next one (section 4.3.2.4/4.3.2.5;
//! hls.js `m3u8-parser.ts`), a `BYTERANGE` without an offset starts where the previous
//! segment's range ended (4.3.2.2), and an AES-128 key without an `IV` uses the segment's
//! media sequence number as a big-endian 128-bit IV (5.2; hls.js `level-key.ts`
//! `createInitializationVector`).
//!
//! DRM is refused, never handled: `METHOD=SAMPLE-AES` / `SAMPLE-AES-CTR` / anything not
//! `AES-128`, or a `KEYFORMAT` other than `identity` (FairPlay `com.apple.streamingkeydelivery`,
//! Widevine and PlayReady `urn:uuid:...`), on a segment or as a master's `EXT-X-SESSION-KEY`,
//! makes the stream [`Refusal::Protected`]. Plain `AES-128` with the `identity` key format is
//! ordinary transport encryption: the key is fetched and the segment decrypted ([`decrypt`]).

use m3u8_rs::{AlternativeMediaType, Key, KeyMethod, MasterPlaylist, MediaPlaylist, Playlist};

use super::{
    AesKey, ByteRange, Container, Plan, Refusal, Segment, Source, Track, Variant, join_url,
};

/// What a playlist turned out to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    /// A master playlist's variants (each a media playlist to fetch later).
    Master(Vec<Variant>),
    /// A media playlist, already expanded.
    Media(Plan),
}

/// Parses an HLS playlist fetched from `base`.
pub fn parse(text: &str, base: &str) -> Result<Parsed, Refusal> {
    match m3u8_rs::parse_playlist_res(text.as_bytes()) {
        Ok(Playlist::MasterPlaylist(m)) => master(&m, base).map(Parsed::Master),
        Ok(Playlist::MediaPlaylist(m)) => media(&m, base).map(Parsed::Media),
        Err(_) => Err(Refusal::Failed("not an HLS playlist".into())),
    }
}

/// A media playlist's plan; a master playlist here is an error.
pub fn parse_media(text: &str, base: &str) -> Result<Plan, Refusal> {
    match parse(text, base)? {
        Parsed::Media(plan) => Ok(plan),
        Parsed::Master(_) => Err(Refusal::Failed("expected a media playlist".into())),
    }
}

/// The variants a detected playlist offers: a master's streams, or a media playlist as one.
pub fn variants(text: &str, base: &str) -> Result<Vec<Variant>, Refusal> {
    match parse(text, base)? {
        Parsed::Master(v) => Ok(v),
        Parsed::Media(plan) => Ok(vec![Variant {
            track: Track::Muxed,
            container: plan.container,
            height: None,
            bandwidth: None,
            codecs: String::new(),
            size: None,
            source: Source::Planned(plan),
            group: None,
            headers: vec![],
        }]),
    }
}

/// A key a DRM system has to open, or a method other than plain AES-128.
fn is_drm(key: &Key) -> bool {
    let format_drm = key
        .keyformat
        .as_deref()
        .is_some_and(|f| !f.eq_ignore_ascii_case("identity"));
    match &key.method {
        KeyMethod::None => false,
        KeyMethod::AES128 => format_drm,
        KeyMethod::SampleAES | KeyMethod::Other(_) => true,
    }
}

fn master(m: &MasterPlaylist, base: &str) -> Result<Vec<Variant>, Refusal> {
    if m.session_key.iter().any(|k| is_drm(&k.0)) {
        return Err(Refusal::Protected);
    }
    // Audio renditions in their own playlists.
    let audio: Vec<Variant> = m
        .alternatives
        .iter()
        .filter(|a| a.media_type == AlternativeMediaType::Audio)
        .filter_map(|a| {
            let uri = a.uri.as_deref()?;
            Some(Variant {
                track: Track::Audio,
                container: Container::of_url(uri).unwrap_or(Container::Ts),
                height: None,
                bandwidth: None,
                codecs: a.name.clone(),
                size: None,
                source: Source::HlsPlaylist(join_url(base, uri)),
                group: Some(a.group_id.clone()),
                headers: vec![],
            })
        })
        .collect();
    let mut out: Vec<Variant> = m
        .variants
        .iter()
        .filter(|v| !v.is_i_frame)
        .map(|v| {
            let separate = v
                .audio
                .as_deref()
                .is_some_and(|g| audio.iter().any(|a| a.group.as_deref() == Some(g)));
            // A variant whose codecs name no picture and which has no resolution is sound only
            // (Apple's `gear0`), not "the smallest video".
            let sound_only = v.resolution.is_none()
                && v.codecs
                    .as_deref()
                    .is_some_and(|c| !c.trim().is_empty() && !names_picture(c));
            Variant {
                track: if sound_only {
                    Track::Audio
                } else if separate {
                    Track::Video
                } else {
                    Track::Muxed
                },
                container: Container::of_url(&v.uri).unwrap_or(Container::Ts),
                height: v.resolution.and_then(|r| u32::try_from(r.height).ok()),
                bandwidth: Some(v.average_bandwidth.unwrap_or(v.bandwidth)),
                codecs: v.codecs.clone().unwrap_or_default(),
                size: None,
                source: Source::HlsPlaylist(join_url(base, &v.uri)),
                group: v.audio.clone().filter(|_| separate && !sound_only),
                headers: vec![],
            }
        })
        .collect();
    out.extend(audio);
    if out.is_empty() {
        return Err(Refusal::Failed("the playlist lists no streams".into()));
    }
    Ok(out)
}

/// Whether a `CODECS` list names a video codec (RFC 6381 four-character codes).
fn names_picture(codecs: &str) -> bool {
    const VIDEO: [&str; 9] = [
        "avc1", "avc3", "hvc1", "hev1", "dvh1", "dvhe", "av01", "vp09", "vp8",
    ];
    codecs.split(',').any(|c| {
        let c = c.trim().to_ascii_lowercase();
        VIDEO.iter().any(|p| c.starts_with(p)) || c.starts_with("vp9")
    })
}

/// The variants of a master, now that one of its media playlists has shown what the segments
/// are (a master names no container: MPEG-TS or fragmented MP4 is only seen in a playlist,
/// by its `EXT-X-MAP`): the same for every variant of a stream.
pub fn with_container(mut variants: Vec<Variant>, plan: Container) -> Vec<Variant> {
    for v in &mut variants {
        if matches!(v.source, Source::HlsPlaylist(_)) {
            v.container = match (v.track, plan) {
                (Track::Audio, Container::Mp4) => Container::M4a,
                _ => plan,
            };
        }
    }
    variants
}

/// `0x` followed by 32 hex digits.
fn parse_iv(text: &str) -> Option<[u8; 16]> {
    let hex = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))?;
    if hex.len() != 32 {
        return None;
    }
    let mut iv = [0u8; 16];
    for (i, byte) in iv.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(iv)
}

/// The IV of segment number `sequence` when the key gives none.
fn sequence_iv(sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&sequence.to_be_bytes());
    iv
}

fn media(m: &MediaPlaylist, base: &str) -> Result<Plan, Refusal> {
    if !m.end_list {
        return Err(Refusal::Live);
    }
    let mut segments = Vec::with_capacity(m.segments.len() + 1);
    let mut key: Option<&Key> = None;
    let mut map: Option<&m3u8_rs::Map> = None;
    let mut last_end: Option<(String, u64)> = None;
    for (i, s) in m.segments.iter().enumerate() {
        if let Some(k) = &s.key {
            key = Some(k);
        }
        // m3u8-rs 6.0.1 rejects `METHOD=NONE` without an IV (its check is inverted) and keeps
        // the line as an unknown tag; it still ends the key.
        if s.unknown_tags.iter().any(|t| {
            t.tag == "X-KEY" && t.rest.as_deref().is_some_and(|r| r.contains("METHOD=NONE"))
        }) {
            key = None;
        }
        if key.is_some_and(is_drm) {
            return Err(Refusal::Protected);
        }
        if let Some(new) = &s.map
            && map != Some(new)
        {
            map = Some(new);
            segments.push(Segment {
                url: join_url(base, &new.uri),
                range: new.byte_range.as_ref().map(|r| ByteRange {
                    start: r.offset.unwrap_or(0),
                    len: r.length,
                }),
                key: None,
            });
        }
        let url = join_url(base, &s.uri);
        let range = s.byte_range.as_ref().map(|r| {
            let follows = last_end
                .as_ref()
                .filter(|(u, _)| *u == url)
                .map(|(_, e)| *e);
            ByteRange {
                start: r.offset.or(follows).unwrap_or(0),
                len: r.length,
            }
        });
        last_end = range.map(|r| (url.clone(), r.start + r.len));
        let aes = match key {
            Some(k) if k.method == KeyMethod::AES128 => {
                let uri = k
                    .uri
                    .as_deref()
                    .ok_or_else(|| Refusal::Failed("an AES-128 key without a URI".into()))?;
                let iv = match k.iv.as_deref() {
                    Some(text) => parse_iv(text)
                        .ok_or_else(|| Refusal::Failed("an AES-128 key with a bad IV".into()))?,
                    None => sequence_iv(m.media_sequence + i as u64),
                };
                Some(AesKey {
                    uri: join_url(base, uri),
                    iv,
                })
            }
            _ => None,
        };
        segments.push(Segment {
            url,
            range,
            key: aes,
        });
    }
    if segments.is_empty() {
        return Err(Refusal::Failed("the playlist has no segments".into()));
    }
    let container = if map.is_some() {
        Container::Mp4
    } else {
        match m.segments.first().and_then(|s| Container::of_url(&s.uri)) {
            Some(Container::Mp4 | Container::M4a) | None => Container::Ts,
            Some(other) => other,
        }
    };
    Ok(Plan {
        segments,
        container,
    })
}

/// Decrypts one AES-128-CBC segment (PKCS#7 padding), HLS `METHOD=AES-128`.
pub fn decrypt(key: &[u8], iv: &[u8; 16], data: &[u8]) -> Result<Vec<u8>, String> {
    use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
    let key: &[u8; 16] = key
        .try_into()
        .map_err(|_| format!("an AES-128 key has 16 bytes, not {}", key.len()))?;
    cbc::Decryptor::<aes::Aes128>::new(key.into(), iv.into())
        .decrypt_padded_vec::<Pkcs7>(data)
        .map_err(|_| "the segment does not decrypt with its key".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://cdn.test/hls/master.m3u8";

    const MASTER: &str = "#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"English\",DEFAULT=YES,URI=\"audio/en.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,CODECS=\"avc1.640028,mp4a.40.2\",AUDIO=\"aud\"
v1080/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1200000,AVERAGE-BANDWIDTH=1000000,RESOLUTION=854x480
v480/index.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=90000,URI=\"iframes.m3u8\"
";

    #[test]
    fn sound_only_variants_are_audio_and_a_playlist_names_the_container() {
        let text = "#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=2000000,CODECS=\"mp4a.40.2, avc1.4d401f\"
gear4/prog_index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=41457,CODECS=\"mp4a.40.2\"
gear0/prog_index.m3u8
";
        let Parsed::Master(v) = parse(text, BASE).unwrap() else {
            panic!("not a master")
        };
        assert_eq!((v[0].track, v[1].track), (Track::Muxed, Track::Audio));
        let fmp4 = with_container(v, Container::Mp4);
        assert_eq!(
            (fmp4[0].container, fmp4[1].container),
            (Container::Mp4, Container::M4a)
        );
        assert!(names_picture("mp4a.40.2, hvc1.2.4.L93") && !names_picture("mp4a.40.5,ec-3"));
    }

    #[test]
    fn a_master_lists_its_streams_and_audio() {
        let Parsed::Master(v) = parse(MASTER, BASE).unwrap() else {
            panic!("not a master")
        };
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].track, Track::Video);
        assert_eq!(v[0].height, Some(1080));
        assert_eq!(v[0].group.as_deref(), Some("aud"));
        assert_eq!(
            v[0].source,
            Source::HlsPlaylist("https://cdn.test/hls/v1080/index.m3u8".into())
        );
        assert_eq!(
            (v[1].track, v[1].bandwidth, v[1].height),
            (Track::Muxed, Some(1_000_000), Some(480))
        );
        assert_eq!(v[2].track, Track::Audio);
        assert_eq!(
            v[2].source,
            Source::HlsPlaylist("https://cdn.test/hls/audio/en.m3u8".into())
        );
    }

    #[test]
    fn a_media_playlist_carries_map_ranges_and_keys() {
        let text = "#EXTM3U
#EXT-X-TARGETDURATION:4
#EXT-X-MEDIA-SEQUENCE:7
#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"700@0\"
#EXT-X-KEY:METHOD=AES-128,URI=\"/keys/k1\"
#EXTINF:4,
#EXT-X-BYTERANGE:1000@700
media.mp4
#EXTINF:4,
#EXT-X-BYTERANGE:500
media.mp4
#EXT-X-KEY:METHOD=AES-128,URI=\"k2\",IV=0x000102030405060708090A0B0C0D0E0F
#EXTINF:4,
#EXT-X-BYTERANGE:300
media.mp4
#EXT-X-KEY:METHOD=NONE
#EXTINF:4,
other.mp4
#EXT-X-ENDLIST
";
        let plan = parse_media(text, "https://cdn.test/v/index.m3u8").unwrap();
        assert_eq!(plan.container, Container::Mp4);
        let s = &plan.segments;
        assert_eq!(s.len(), 5);
        assert_eq!(s[0].url, "https://cdn.test/v/init.mp4");
        assert_eq!(s[0].range, Some(ByteRange { start: 0, len: 700 }));
        assert_eq!(
            s[1].range,
            Some(ByteRange {
                start: 700,
                len: 1000
            })
        );
        assert_eq!(
            s[2].range,
            Some(ByteRange {
                start: 1700,
                len: 500
            })
        );
        assert_eq!(
            s[3].range,
            Some(ByteRange {
                start: 2200,
                len: 300
            })
        );
        let k1 = s[1].key.as_ref().unwrap();
        assert_eq!(k1.uri, "https://cdn.test/keys/k1");
        assert_eq!(k1.iv, sequence_iv(7));
        // The key holds for the next segment, with that segment's own number.
        assert_eq!(s[2].key.as_ref().unwrap().iv, sequence_iv(8));
        let k2 = s[3].key.as_ref().unwrap();
        assert_eq!(k2.uri, "https://cdn.test/v/k2");
        assert_eq!(k2.iv, core::array::from_fn(|i| i as u8));
        assert_eq!((s[4].key.as_ref(), s[4].range), (None, None));
    }

    #[test]
    fn ts_segments_join_as_ts() {
        let text = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n#EXTINF:2,\nb.ts\n#EXT-X-ENDLIST\n";
        let plan = parse_media(text, BASE).unwrap();
        assert_eq!(plan.container, Container::Ts);
        assert_eq!(plan.segments[1].url, "https://cdn.test/hls/b.ts");
        let v = variants(text, BASE).unwrap();
        assert_eq!((v.len(), v[0].track), (1, Track::Muxed));
        assert!(matches!(v[0].source, Source::Planned(_)));
    }

    #[test]
    fn drm_keys_are_protected() {
        let with = |key: &str| {
            format!("#EXTM3U\n#EXT-X-TARGETDURATION:2\n{key}\n#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n")
        };
        for key in [
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k\",KEYFORMAT=\"com.apple.streamingkeydelivery\"",
            "#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"data:x\",KEYFORMAT=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"",
            "#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMAT=\"urn:uuid:9a04f079-9840-4286-ab92-e65be0885f95\"",
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"",
        ] {
            assert_eq!(parse(&with(key), BASE), Err(Refusal::Protected), "{key}");
        }
        let plain = with("#EXT-X-KEY:METHOD=AES-128,URI=\"k\",KEYFORMAT=\"identity\"");
        assert!(parse_media(&plain, BASE).unwrap().segments[0].key.is_some());
        let session = "#EXTM3U\n#EXT-X-SESSION-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXT-X-STREAM-INF:BANDWIDTH=1\nv.m3u8\n";
        assert_eq!(parse(session, BASE), Err(Refusal::Protected));
    }

    #[test]
    fn live_and_junk_are_refused() {
        let live = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n";
        assert_eq!(parse(live, BASE), Err(Refusal::Live));
        assert!(matches!(parse("<html>", BASE), Err(Refusal::Failed(_))));
        let bad_iv = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\",IV=0x12\n#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n";
        assert!(matches!(parse(bad_iv, BASE), Err(Refusal::Failed(_))));
    }

    #[test]
    fn aes_128_round_trips() {
        use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
        let key = [7u8; 16];
        let iv = sequence_iv(3);
        let plain = b"a segment of transport stream bytes".to_vec();
        let sealed = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_vec::<Pkcs7>(&plain);
        assert_eq!(decrypt(&key, &iv, &sealed).unwrap(), plain);
        assert!(decrypt(&key[..8], &iv, &sealed).is_err());
        assert!(decrypt(&[9u8; 16], &iv, &sealed).is_err());
    }
}
