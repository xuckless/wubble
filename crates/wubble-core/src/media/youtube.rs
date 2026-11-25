//! YouTube, the one site-specific and fragile part, kept in this module alone. The page's own
//! player cannot be watched for its streams: the web client fetches only SABR formats (POSTed
//! protobuf, no plain itag/range requests), and most other clients need a PO token for the
//! video hosts (yt-dlp's PO Token Guide). So wubble asks the Innertube player API itself, as
//! the `android_vr` client, which needs no PO token and no JavaScript player (no signature or
//! `n` challenge), and lists the formats it answers with direct URLs. Formats that need a
//! signature (`signatureCipher`) are skipped; formats with `drmFamilies` (or a player answer
//! with `drmParams`) are protected and never fetched.
//!
//! Ported from yt-dlp (yt-dlp/yt-dlp, Unlicense: public domain, credited anyway):
//! `yt_dlp/extractor/youtube/_base.py` `INNERTUBE_CLIENTS['android_vr']` (the client context,
//! as data in [`CLIENT`]) and `_video.py` `_extract_formats_and_subtitles` (the
//! `streamingData.formats` / `adaptiveFormats` fields read here). The request is wubble's own
//! network request, made only with the `media.youtube` setting on (off by default). When the
//! answer no longer has the shape read here, the result is [`Refusal::SiteChanged`], which the
//! menu shows; the rest of the downloader is unaffected.

use serde_json::Value;

use super::{Container, Refusal, Resolution, Source, Track, Variant};

/// The `android_vr` client, from yt-dlp `INNERTUBE_CLIENTS`. Bump together when YouTube
/// retires the version.
pub struct Client {
    pub name: &'static str,
    /// `X-YouTube-Client-Name`.
    pub id: u32,
    pub version: &'static str,
    pub user_agent: &'static str,
    pub device_make: &'static str,
    pub device_model: &'static str,
    pub sdk: u32,
    pub os_name: &'static str,
    pub os_version: &'static str,
}

pub const CLIENT: Client = Client {
    name: "ANDROID_VR",
    id: 28,
    version: "1.65.10",
    user_agent: "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
    device_make: "Oculus",
    device_model: "Quest 3",
    sdk: 32,
    os_name: "Android",
    os_version: "12L",
};

/// The player API's endpoint.
pub const PLAYER_URL: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";

/// The video id of a watch, shorts, embed or youtu.be URL.
pub fn video_id(url: &str) -> Option<String> {
    let parsed = ::url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(&host);
    let id = match host {
        "youtube.com" | "music.youtube.com" => {
            let mut segments = parsed.path_segments()?;
            match segments.next()? {
                "watch" => parsed
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned())?,
                "shorts" | "embed" | "live" => segments.next()?.to_owned(),
                _ => return None,
            }
        }
        "youtu.be" => parsed.path_segments()?.next()?.to_owned(),
        _ => return None,
    };
    let valid = id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then_some(id)
}

/// The player request for `video_id`: its JSON body and headers.
pub fn player_request(video_id: &str) -> (String, Vec<(String, String)>) {
    let c = &CLIENT;
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": c.name,
                "clientVersion": c.version,
                "deviceMake": c.device_make,
                "deviceModel": c.device_model,
                "androidSdkVersion": c.sdk,
                "userAgent": c.user_agent,
                "osName": c.os_name,
                "osVersion": c.os_version,
                "hl": "en",
            }
        },
        "videoId": video_id,
        "playbackContext": { "contentPlaybackContext": { "html5Preference": "HTML5_PREF_WANTS" } },
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("X-YouTube-Client-Name".into(), c.id.to_string()),
        ("X-YouTube-Client-Version".into(), c.version.into()),
        ("User-Agent".into(), c.user_agent.into()),
        ("Origin".into(), "https://www.youtube.com".into()),
    ];
    (body.to_string(), headers)
}

/// The headers each format fetch sends (the client the URLs were issued to).
fn fetch_headers() -> Vec<(String, String)> {
    vec![("User-Agent".into(), CLIENT.user_agent.into())]
}

fn changed(what: &str) -> Refusal {
    Refusal::SiteChanged(format!(
        "YouTube formats unavailable ({what}, client {})",
        CLIENT.name
    ))
}

/// The formats of a player answer.
pub fn parse_player(json: &str) -> Result<Resolution, Refusal> {
    let root: Value = serde_json::from_str(json).map_err(|_| changed("not JSON"))?;
    let status = root
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str);
    match status {
        Some("OK") => {}
        Some(other) => {
            let reason = root
                .pointer("/playabilityStatus/reason")
                .and_then(Value::as_str)
                .unwrap_or(other);
            return Err(Refusal::Failed(format!("YouTube: {reason}")));
        }
        None => return Err(changed("no playability status")),
    }
    let streaming = root
        .get("streamingData")
        .ok_or_else(|| changed("no streaming data"))?;
    if streaming.get("drmParams").is_some() {
        return Err(Refusal::Protected);
    }
    let title = root
        .pointer("/videoDetails/title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let list = |key: &str| {
        streaming
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let (muxed, adaptive) = (list("formats"), list("adaptiveFormats"));
    if muxed.is_empty() && adaptive.is_empty() {
        if streaming.get("hlsManifestUrl").is_some() {
            return Err(Refusal::Live);
        }
        return Err(changed("no formats"));
    }
    let (mut protected, mut ciphered) = (0, 0);
    let mut variants = Vec::new();
    for (format, is_muxed) in muxed
        .iter()
        .map(|f| (f, true))
        .chain(adaptive.iter().map(|f| (f, false)))
    {
        if format.get("drmFamilies").is_some() {
            protected += 1;
            continue;
        }
        let Some(url) = format.get("url").and_then(Value::as_str) else {
            ciphered += 1;
            continue;
        };
        let mime = format
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(container) = Container::of_mime(mime) else {
            continue;
        };
        let track = match (is_muxed, mime.starts_with("audio/")) {
            (true, _) => Track::Muxed,
            (false, true) => Track::Audio,
            (false, false) => Track::Video,
        };
        let number = |key: &str| {
            let v = format.get(key)?;
            v.as_u64().or_else(|| v.as_str()?.parse().ok())
        };
        let size = number("contentLength");
        let codecs = mime
            .split_once("codecs=")
            .map(|(_, c)| c.trim_matches('"').to_owned())
            .unwrap_or_default();
        variants.push(Variant {
            track,
            container,
            height: number("height").and_then(|h| u32::try_from(h).ok()),
            bandwidth: number("bitrate"),
            codecs,
            size,
            source: Source::File {
                url: url.to_owned(),
                size,
                ranges: size.is_some(),
            },
            group: None,
            headers: fetch_headers(),
        });
    }
    match (variants.is_empty(), protected, ciphered) {
        (false, ..) => Ok(Resolution {
            variants,
            title,
            note: None,
        }),
        (true, p, _) if p > 0 => Err(Refusal::Protected),
        (true, _, c) if c > 0 => Err(changed("every format needs a signature")),
        _ => Err(changed("no usable formats")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: &str = r#"{
      "playabilityStatus": {"status": "OK"},
      "videoDetails": {"videoId": "dQw4w9WgXcQ", "title": "A Song"},
      "streamingData": {
        "formats": [
          {"itag": 18, "url": "https://rr1.googlevideo.test/videoplayback?itag=18", "mimeType": "video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"", "bitrate": 500000, "height": 360, "contentLength": "1000"}
        ],
        "adaptiveFormats": [
          {"itag": 137, "url": "https://rr1.googlevideo.test/videoplayback?itag=137", "mimeType": "video/mp4; codecs=\"avc1.640028\"", "bitrate": 4000000, "height": 1080, "contentLength": "90000"},
          {"itag": 248, "signatureCipher": "s=abc&url=x", "mimeType": "video/webm; codecs=\"vp9\"", "height": 1080},
          {"itag": 999, "url": "https://x.test/drm", "mimeType": "video/mp4", "drmFamilies": ["WIDEVINE"]},
          {"itag": 140, "url": "https://rr1.googlevideo.test/videoplayback?itag=140", "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "bitrate": 130000, "contentLength": 3000}
        ]
      }
    }"#;

    #[test]
    fn a_player_answer_lists_direct_formats() {
        let r = parse_player(PLAYER).unwrap();
        assert_eq!(r.title.as_deref(), Some("A Song"));
        let v = &r.variants;
        assert_eq!(v.len(), 3, "the ciphered and the DRM format are dropped");
        assert_eq!(
            (v[0].track, v[0].height, v[0].size),
            (Track::Muxed, Some(360), Some(1000))
        );
        assert_eq!(
            (v[1].track, v[1].height, v[1].container),
            (Track::Video, Some(1080), Container::Mp4)
        );
        assert_eq!(
            (v[2].track, v[2].container, v[2].size),
            (Track::Audio, Container::M4a, Some(3000))
        );
        assert_eq!(v[1].codecs, "avc1.640028");
        assert!(matches!(v[2].source, Source::File { ranges: true, .. }));
        assert_eq!(v[0].headers[0].1, CLIENT.user_agent);
    }

    #[test]
    fn unknown_shapes_say_the_site_changed() {
        assert!(matches!(parse_player("[]"), Err(Refusal::SiteChanged(_))));
        assert!(matches!(parse_player("nope"), Err(Refusal::SiteChanged(_))));
        assert!(matches!(
            parse_player(r#"{"playabilityStatus":{"status":"OK"}}"#),
            Err(Refusal::SiteChanged(_))
        ));
        let ciphered = r#"{"playabilityStatus":{"status":"OK"},"streamingData":{"adaptiveFormats":[{"signatureCipher":"s","mimeType":"video/mp4"}]}}"#;
        let Err(Refusal::SiteChanged(text)) = parse_player(ciphered) else {
            panic!()
        };
        assert!(text.contains("signature") && text.contains("ANDROID_VR"));
    }

    #[test]
    fn drm_and_unplayable_are_refused() {
        let drm = r#"{"playabilityStatus":{"status":"OK"},"streamingData":{"drmParams":"x","adaptiveFormats":[]}}"#;
        assert_eq!(parse_player(drm), Err(Refusal::Protected));
        let only_drm = r#"{"playabilityStatus":{"status":"OK"},"streamingData":{"adaptiveFormats":[{"url":"u","mimeType":"video/mp4","drmFamilies":["PLAYREADY"]}]}}"#;
        assert_eq!(parse_player(only_drm), Err(Refusal::Protected));
        let login = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#;
        assert_eq!(
            parse_player(login),
            Err(Refusal::Failed(
                "YouTube: Sign in to confirm your age".into()
            ))
        );
    }

    #[test]
    fn video_ids_from_urls() {
        let id = Some("dQw4w9WgXcQ".to_owned());
        assert_eq!(
            video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1"),
            id
        );
        assert_eq!(
            video_id("https://m.youtube.com/watch?feature=x&v=dQw4w9WgXcQ"),
            id
        );
        assert_eq!(video_id("https://youtu.be/dQw4w9WgXcQ?si=1"), id);
        assert_eq!(video_id("https://www.youtube.com/shorts/dQw4w9WgXcQ"), id);
        assert_eq!(video_id("https://www.youtube.com/watch?v=short"), None);
        assert_eq!(video_id("https://www.youtube.com/feed/trending"), None);
        assert_eq!(video_id("https://notyoutube.com/watch?v=dQw4w9WgXcQ"), None);
    }

    #[test]
    fn the_request_names_the_client_and_video() {
        let (body, headers) = player_request("dQw4w9WgXcQ");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["videoId"], "dQw4w9WgXcQ");
        assert_eq!(v["context"]["client"]["clientName"], "ANDROID_VR");
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "X-YouTube-Client-Name" && v == "28")
        );
    }
}
