//! DASH manifests (static, one period): each `Representation` as a [`Variant`] with its
//! segments expanded. Parsed by `dash-mpd-core` (MIT); the expansion rules are DASH-IF IOP and
//! dash.js (Dash-Industry-Forum/dash.js, BSD-3-Clause; rules ported, no code copied):
//! `SegmentTemplate` identifiers `$RepresentationID$`, `$Number$`, `$Time$`, `$Bandwidth$` and
//! their `%0Nd` width (dash.js `SegmentsUtils.js` `replaceTokenForTemplate`), `SegmentTimeline`
//! `S@t/@d/@r` with `r=-1` repeating to the period's end (`TimelineSegmentsGetter.js`),
//! duration-based templates counted over the presentation's duration, `SegmentList`, and a
//! single `BaseURL` file (`SegmentBase`). `BaseURL` resolves MPD > Period > AdaptationSet >
//! Representation; the first template found on the Representation, AdaptationSet or Period is
//! used whole (no attribute-level inheritance).
//!
//! Any `ContentProtection` (Widevine, PlayReady, ClearKey or the bare `mp4protection` CENC
//! signal) marks the representation [`Refusal::Protected`]: EME streams are never fetched.

use dash_mpd_core::{AdaptationSet, Period, Representation, SegmentTemplate};

use super::{ByteRange, Container, Plan, Refusal, Segment, Source, Track, Variant, join_url};

/// Most segments one representation may expand to (a malformed manifest's runaway count).
const MAX_SEGMENTS: u64 = 100_000;

/// The variants a detected MPD offers.
pub fn variants(text: &str, base: &str) -> Result<Vec<Variant>, Refusal> {
    let mpd = dash_mpd_core::parse(text)
        .map_err(|e| Refusal::Failed(format!("not a DASH manifest ({e})")))?;
    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(Refusal::Live);
    }
    let [period] = mpd.periods.as_slice() else {
        return Err(Refusal::Unsupported(
            "a manifest with several periods".into(),
        ));
    };
    let base = mpd
        .base_url
        .first()
        .map_or_else(|| base.to_owned(), |b| join_url(base, &b.base));
    let base = period
        .BaseURL
        .first()
        .map_or(base.clone(), |b| join_url(&base, &b.base));
    let seconds = period
        .duration
        .or(mpd.mediaPresentationDuration)
        .map(|d| d.as_secs_f64());
    let mpd_protected = !mpd.ContentProtection.is_empty() || !period.ContentProtection.is_empty();
    let mut out = Vec::new();
    let mut protected = 0;
    for set in &period.adaptations {
        let set_base = set
            .BaseURL
            .first()
            .map_or(base.clone(), |b| join_url(&base, &b.base));
        for rep in &set.representations {
            let Some(track) = track_of(set, rep) else {
                continue;
            };
            if mpd_protected
                || !set.ContentProtection.is_empty()
                || !rep.ContentProtection.is_empty()
            {
                protected += 1;
                continue;
            }
            let rep_base = rep
                .BaseURL
                .first()
                .map_or(set_base.clone(), |b| join_url(&set_base, &b.base));
            let segments = segments_of(period, set, rep, &rep_base, seconds)?;
            let mime = rep
                .mimeType
                .as_deref()
                .or(set.mimeType.as_deref())
                .unwrap_or_default();
            let container = match Container::of_mime(mime) {
                Some(Container::M4a) if track != Track::Audio => Container::Mp4,
                Some(Container::Mp4) if track == Track::Audio => Container::M4a,
                Some(c) => c,
                None if track == Track::Audio => Container::M4a,
                None => Container::Mp4,
            };
            out.push(Variant {
                track,
                container,
                height: rep
                    .height
                    .or(set.height)
                    .and_then(|h| u32::try_from(h).ok()),
                bandwidth: rep.bandwidth,
                codecs: rep
                    .codecs
                    .clone()
                    .or_else(|| set.codecs.clone())
                    .unwrap_or_default(),
                size: None,
                source: Source::Planned(Plan {
                    segments,
                    container,
                }),
                group: None,
                headers: vec![],
            });
        }
    }
    match (out.is_empty(), protected) {
        (true, 0) => Err(Refusal::Failed(
            "the manifest lists no audio or video".into(),
        )),
        (true, _) => Err(Refusal::Protected),
        (false, _) => Ok(out),
    }
}

/// Video, audio or both; `None` for text, images and the rest.
fn track_of(set: &AdaptationSet, rep: &Representation) -> Option<Track> {
    let kind = rep
        .contentType
        .as_deref()
        .or(set.contentType.as_deref())
        .or_else(|| {
            rep.mimeType
                .as_deref()
                .or(set.mimeType.as_deref())?
                .split('/')
                .next()
        })?;
    let codecs = rep
        .codecs
        .as_deref()
        .or(set.codecs.as_deref())
        .unwrap_or_default();
    let has_audio = codecs.split(',').any(|c| {
        let c = c.trim();
        c.starts_with("mp4a")
            || c.starts_with("opus")
            || c.starts_with("vorbis")
            || c.starts_with("ac-3")
            || c.starts_with("ec-3")
    });
    match kind {
        "video" if has_audio && codecs.contains(',') => Some(Track::Muxed),
        "video" => Some(Track::Video),
        "audio" => Some(Track::Audio),
        _ => None,
    }
}

/// `a-b` (inclusive) as a range.
fn parse_range(text: &str) -> Option<ByteRange> {
    let (a, b) = text.trim().split_once('-')?;
    let (start, end): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (end >= start).then_some(ByteRange {
        start,
        len: end - start + 1,
    })
}

fn segments_of(
    period: &Period,
    set: &AdaptationSet,
    rep: &Representation,
    base: &str,
    seconds: Option<f64>,
) -> Result<Vec<Segment>, Refusal> {
    let template = rep
        .SegmentTemplate
        .as_ref()
        .or(set.SegmentTemplate.as_ref())
        .or(period.SegmentTemplate.as_ref());
    if let Some(t) = template {
        return template_segments(t, rep, base, seconds);
    }
    let list = rep
        .SegmentList
        .as_ref()
        .or(set.SegmentList.as_ref())
        .or(period.SegmentList.as_ref());
    if let Some(list) = list {
        let init = list.Initialization.as_ref().and_then(|i| {
            Some(Segment {
                url: join_url(base, i.sourceURL.as_deref()?),
                range: i.range.as_deref().and_then(parse_range),
                key: None,
            })
        });
        let media = list.segment_urls.iter().map(|s| Segment {
            url: s
                .media
                .as_deref()
                .map_or_else(|| base.to_owned(), |m| join_url(base, m)),
            range: s.mediaRange.as_deref().and_then(parse_range),
            key: None,
        });
        return Ok(init.into_iter().chain(media).collect());
    }
    // SegmentBase or a bare BaseURL: the whole file holds init, index and media.
    Ok(vec![Segment::whole(base)])
}

/// A template with its identifiers filled (`$$` is a dollar sign).
fn fill(template: &str, rep: &Representation, number: u64, time: u64) -> String {
    let mut out = String::with_capacity(template.len());
    let mut parts = template.split('$');
    out.push_str(parts.next().unwrap_or_default());
    let mut inside = true;
    for part in parts {
        if !inside {
            out.push_str(part);
            inside = true;
            continue;
        }
        inside = false;
        let (name, width) = match part.split_once('%') {
            Some((n, f)) => (
                n,
                f.trim_start_matches('0')
                    .trim_end_matches('d')
                    .parse::<usize>()
                    .ok(),
            ),
            None => (part, None),
        };
        let value = match name {
            "" => "$".to_owned(),
            "RepresentationID" => rep.id.clone().unwrap_or_default(),
            "Number" => number.to_string(),
            "Time" => time.to_string(),
            "Bandwidth" => rep.bandwidth.unwrap_or(0).to_string(),
            other => format!("${other}$"),
        };
        match width {
            Some(w) => out.push_str(&format!("{value:0>w$}")),
            None => out.push_str(&value),
        }
    }
    out
}

fn template_segments(
    t: &SegmentTemplate,
    rep: &Representation,
    base: &str,
    seconds: Option<f64>,
) -> Result<Vec<Segment>, Refusal> {
    let media = t
        .media
        .as_deref()
        .ok_or_else(|| Refusal::Failed("a segment template without media".into()))?;
    let start = t.startNumber.unwrap_or(1);
    let timescale = t.timescale.unwrap_or(1).max(1);
    let init = t
        .initialization
        .as_deref()
        .or(t
            .Initialization
            .as_ref()
            .and_then(|i| i.sourceURL.as_deref()))
        .map(|i| Segment::whole(&join_url(base, &fill(i, rep, start, 0))));
    let mut out: Vec<Segment> = init.into_iter().collect();
    let too_many = || Refusal::Unsupported("more segments than a download takes".into());
    if let Some(timeline) = &t.SegmentTimeline {
        let end = seconds.map(|s| (s * timescale as f64) as u64);
        let mut time = 0u64;
        let mut number = start;
        for (i, s) in timeline.segments.iter().enumerate() {
            time = s.t.unwrap_or(time);
            let repeat = match s.r {
                Some(r) if r < 0 => {
                    // Up to the next S's start, else the period's end.
                    let until = timeline
                        .segments
                        .get(i + 1)
                        .and_then(|n| n.t)
                        .or(end)
                        .ok_or_else(|| {
                            Refusal::Failed("an open timeline repeat without a duration".into())
                        })?;
                    until
                        .saturating_sub(time)
                        .div_ceil(s.d.max(1))
                        .saturating_sub(1)
                }
                Some(r) => r as u64,
                None => 0,
            };
            if number - start + repeat >= MAX_SEGMENTS {
                return Err(too_many());
            }
            for _ in 0..=repeat {
                out.push(Segment::whole(&join_url(
                    base,
                    &fill(media, rep, number, time),
                )));
                time += s.d;
                number += 1;
            }
        }
        return Ok(out);
    }
    let duration = t.duration.filter(|d| *d > 0.0).ok_or_else(|| {
        Refusal::Failed("a segment template without a duration or timeline".into())
    })?;
    let seconds = seconds.ok_or_else(|| Refusal::Failed("a manifest without a duration".into()))?;
    let count = (seconds * timescale as f64 / duration).ceil() as u64;
    let count = match t.endNumber {
        Some(end) if end >= start => count.min(end - start + 1),
        _ => count,
    };
    if count > MAX_SEGMENTS {
        return Err(too_many());
    }
    for n in 0..count {
        let time = (n as f64 * duration) as u64;
        out.push(Segment::whole(&join_url(
            base,
            &fill(media, rep, start + n, time),
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://cdn.test/dash/manifest.mpd";

    fn mpd(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
             mediaPresentationDuration=\"PT10S\" profiles=\"urn:mpeg:dash:profile:isoff-live:2011\">\
             <Period>{body}</Period></MPD>"
        )
    }

    const AV: &str = "<AdaptationSet contentType=\"video\" mimeType=\"video/mp4\">\
        <SegmentTemplate initialization=\"$RepresentationID$/init.mp4\" media=\"$RepresentationID$/seg-$Number%03d$.m4s\" startNumber=\"1\" duration=\"4\" timescale=\"1\"/>\
        <Representation id=\"v720\" bandwidth=\"3000000\" width=\"1280\" height=\"720\" codecs=\"avc1.64001f\"/>\
        <Representation id=\"v360\" bandwidth=\"800000\" width=\"640\" height=\"360\" codecs=\"avc1.64001e\"/>\
        </AdaptationSet>\
        <AdaptationSet contentType=\"audio\" mimeType=\"audio/mp4\">\
        <Representation id=\"a\" bandwidth=\"128000\" codecs=\"mp4a.40.2\">\
        <SegmentTemplate initialization=\"a/init.mp4\" media=\"a/$Time$.m4s\" timescale=\"1000\">\
        <SegmentTimeline><S t=\"0\" d=\"4000\" r=\"1\"/><S d=\"2000\"/></SegmentTimeline></SegmentTemplate>\
        </Representation></AdaptationSet>";

    #[test]
    fn separate_audio_and_video_expand_their_templates() {
        let v = variants(&mpd(AV), BASE).unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(
            (v[0].track, v[0].height, v[0].container),
            (Track::Video, Some(720), Container::Mp4)
        );
        let Source::Planned(plan) = &v[0].source else {
            panic!()
        };
        let urls: Vec<&str> = plan.segments.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.test/dash/v720/init.mp4",
                "https://cdn.test/dash/v720/seg-001.m4s",
                "https://cdn.test/dash/v720/seg-002.m4s",
                "https://cdn.test/dash/v720/seg-003.m4s",
            ]
        );
        assert_eq!((v[2].track, v[2].container), (Track::Audio, Container::M4a));
        let Source::Planned(plan) = &v[2].source else {
            panic!()
        };
        let urls: Vec<&str> = plan.segments.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.test/dash/a/init.mp4",
                "https://cdn.test/dash/a/0.m4s",
                "https://cdn.test/dash/a/4000.m4s",
                "https://cdn.test/dash/a/8000.m4s",
            ]
        );
    }

    #[test]
    fn an_open_repeat_runs_to_the_end() {
        let body = "<AdaptationSet mimeType=\"audio/mp4\"><Representation id=\"a\" bandwidth=\"1\">\
            <SegmentTemplate media=\"$Number$.m4s\" timescale=\"10\"><SegmentTimeline><S t=\"0\" d=\"30\" r=\"-1\"/></SegmentTimeline></SegmentTemplate>\
            </Representation></AdaptationSet>";
        let v = variants(&mpd(body), BASE).unwrap();
        let Source::Planned(plan) = &v[0].source else {
            panic!()
        };
        // 10 s at 3 s a segment: 4 segments, the last one short.
        assert_eq!(plan.segments.len(), 4);
        assert_eq!(plan.segments[3].url, "https://cdn.test/dash/4.m4s");
    }

    #[test]
    fn a_single_file_and_a_segment_list() {
        let body = "<BaseURL>media/</BaseURL><AdaptationSet mimeType=\"video/webm\">\
            <Representation id=\"1\" bandwidth=\"1\" height=\"240\" codecs=\"vp9,opus\"><BaseURL>clip.webm</BaseURL><SegmentBase indexRange=\"0-99\"/></Representation>\
            <Representation id=\"2\" bandwidth=\"1\" height=\"480\" codecs=\"vp9\"><SegmentList><Initialization sourceURL=\"i.webm\" range=\"0-9\"/>\
            <SegmentURL media=\"s1.webm\" mediaRange=\"10-19\"/><SegmentURL media=\"s2.webm\"/></SegmentList></Representation>\
            </AdaptationSet>";
        let v = variants(&mpd(body), BASE).unwrap();
        assert_eq!(
            (v[0].track, v[0].container),
            (Track::Muxed, Container::Webm)
        );
        assert_eq!(
            v[0].source,
            Source::Planned(Plan {
                segments: vec![Segment::whole("https://cdn.test/dash/media/clip.webm")],
                container: Container::Webm,
            })
        );
        let Source::Planned(plan) = &v[1].source else {
            panic!()
        };
        assert_eq!(plan.segments.len(), 3);
        assert_eq!(
            plan.segments[0].range,
            Some(ByteRange { start: 0, len: 10 })
        );
        assert_eq!(
            plan.segments[1].range,
            Some(ByteRange { start: 10, len: 10 })
        );
        assert_eq!(plan.segments[2].url, "https://cdn.test/dash/media/s2.webm");
    }

    #[test]
    fn content_protection_is_protected() {
        for scheme in [
            "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed",
            "urn:uuid:9a04f079-9840-4286-ab92-e65be0885f95",
            "urn:uuid:e2719d58-a985-b3c9-781a-b030af78d30e",
            "urn:mpeg:dash:mp4protection:2011",
        ] {
            let body = format!(
                "<AdaptationSet mimeType=\"video/mp4\"><ContentProtection schemeIdUri=\"{scheme}\"/>\
                 <Representation id=\"v\" bandwidth=\"1\"><BaseURL>v.mp4</BaseURL></Representation></AdaptationSet>"
            );
            assert_eq!(
                variants(&mpd(&body), BASE),
                Err(Refusal::Protected),
                "{scheme}"
            );
        }
        // A protected video next to a clear audio: only the audio is offered.
        let mixed = "<AdaptationSet mimeType=\"video/mp4\"><ContentProtection schemeIdUri=\"urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed\"/>\
            <Representation id=\"v\" bandwidth=\"1\"><BaseURL>v.mp4</BaseURL></Representation></AdaptationSet>\
            <AdaptationSet mimeType=\"audio/mp4\"><Representation id=\"a\" bandwidth=\"1\"><BaseURL>a.m4a</BaseURL></Representation></AdaptationSet>";
        let v = variants(&mpd(mixed), BASE).unwrap();
        assert_eq!((v.len(), v[0].track), (1, Track::Audio));
    }

    #[test]
    fn live_multi_period_and_junk_are_refused() {
        let live = "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\"><Period/></MPD>";
        assert_eq!(variants(live, BASE), Err(Refusal::Live));
        let two = "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\"><Period/><Period/></MPD>";
        assert!(matches!(variants(two, BASE), Err(Refusal::Unsupported(_))));
        assert!(matches!(
            variants("not xml at all", BASE),
            Err(Refusal::Failed(_))
        ));
    }

    #[test]
    fn templates_fill_identifiers() {
        let rep = Representation {
            id: Some("r1".into()),
            bandwidth: Some(5),
            ..Default::default()
        };
        assert_eq!(
            fill(
                "$RepresentationID$/$Number%05d$-$Time$-$Bandwidth$$$.m4s",
                &rep,
                42,
                9
            ),
            "r1/00042-9-5$.m4s"
        );
        assert_eq!(fill("plain.mp4", &rep, 1, 0), "plain.mp4");
    }
}
