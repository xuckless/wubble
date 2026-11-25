//! DASH manifests (static): each `Representation` as a [`Variant`] with its segments expanded.
//! Parsed by `dash-mpd-core` (MIT); the expansion rules are DASH-IF IOP and dash.js
//! (Dash-Industry-Forum/dash.js, BSD-3-Clause; rules ported, no code copied):
//! `SegmentTemplate` identifiers `$RepresentationID$`, `$Number$`, `$Time$`, `$Bandwidth$` and
//! their `%0Nd` width (dash.js `SegmentsUtils.js` `replaceTokenForTemplate`), `SegmentTimeline`
//! `S@t/@d/@r` with `r=-1` repeating to the period's end (`TimelineSegmentsGetter.js`),
//! duration-based templates counted over the period's duration from its
//! `presentationTimeOffset`, `SegmentList`, and a single `BaseURL` file (`SegmentBase`).
//! `BaseURL` resolves MPD > Period > AdaptationSet > Representation; the first template found on
//! the Representation, AdaptationSet or Period is used whole (no attribute-level inheritance).
//!
//! Several periods: a period's start is its `@start`, else where the previous one ended; its
//! length its `@duration`, else up to the next period's `@start` (or the presentation's end), as
//! dash.js `DashManifestModel.getRegularPeriods` reads them. The longest period is the main
//! content; each of its representations is followed through the other periods to its match
//! there (same kind, then the same `@id`, then the same height and codecs at the nearest bit
//! rate: dash.js keeps a track across periods by its id, then its attributes), and its variant's
//! segments are every matched period's in order, each period with its own init ([`Plan::periods`]
//! marks where one begins). A period nothing matches (a pre-roll ad) is left out and the menu says
//! so; a representation that misses a period others match is not offered.
//!
//! Subtitles: a text `AdaptationSet` of WebVTT or TTML files (`text/vtt`,
//! `application/ttml+xml`) is a [`Track::Subtitle`] variant with the set's `@lang` and `Label`;
//! subtitles inside fragmented MP4 (`application/mp4`, `stpp`/`wvtt`) are not files and are left
//! out, and so are subtitles of a programme in several periods (each period's file starts its
//! cues at zero again).
//!
//! Any `ContentProtection` (Widevine, PlayReady, ClearKey or the bare `mp4protection` CENC
//! signal) marks the representation [`Refusal::Protected`]: EME streams are never fetched.

use dash_mpd_core::{AdaptationSet, MPD, Period, Representation, SegmentTemplate};

use super::{
    ByteRange, Container, Plan, Refusal, Resolution, Segment, Source, Track, Variant, join_url,
};

/// Most segments one representation may expand to (a malformed manifest's runaway count).
const MAX_SEGMENTS: u64 = 100_000;
/// Widest zero-padded template identifier (`%0Nd`): stops a crafted manifest from allocating GiB.
const MAX_FORMAT_WIDTH: usize = 32;

/// What the menu says when periods were left out.
pub const ADS_LEFT_OUT: &str = "ad periods left out";

/// One representation of one period: its `@id` (for matching across periods) and its variant.
type Rep = (Option<String>, Variant);

/// What a detected MPD offers.
pub fn variants(text: &str, base: &str) -> Result<Resolution, Refusal> {
    let mpd = dash_mpd_core::parse(text)
        .map_err(|e| Refusal::Failed(format!("not a DASH manifest ({e})")))?;
    if mpd.mpdtype.as_deref() == Some("dynamic") {
        return Err(Refusal::Live);
    }
    let base = mpd
        .base_url
        .first()
        .map_or_else(|| base.to_owned(), |b| join_url(base, &b.base));
    let lengths = period_lengths(&mpd);
    let mut periods: Vec<(Vec<Rep>, usize)> = Vec::with_capacity(mpd.periods.len());
    for (period, seconds) in mpd.periods.iter().zip(&lengths) {
        periods.push(period_reps(&mpd, period, &base, *seconds)?);
    }
    // The longest period (the first of equals: `rev` makes `max_by` keep the earliest).
    let main = (0..periods.len())
        .rev()
        .max_by(|&a, &b| {
            lengths[a]
                .unwrap_or(0.0)
                .total_cmp(&lengths[b].unwrap_or(0.0))
        })
        .ok_or_else(|| Refusal::Failed("the manifest has no period".into()))?;
    let (main_reps, protected) = &periods[main];
    if main_reps.is_empty() {
        return Err(match protected {
            0 => Refusal::Failed("the manifest lists no audio or video".into()),
            _ => Refusal::Protected,
        });
    }
    // Per main representation, its match in each period.
    let chains: Vec<Vec<Option<usize>>> = (0..main_reps.len())
        .map(|m| {
            periods
                .iter()
                .enumerate()
                .map(|(p, (reps, _))| {
                    if p == main {
                        Some(m)
                    } else {
                        matching(&main_reps[m], reps)
                    }
                })
                .collect()
        })
        .collect();
    let content: Vec<bool> = (0..periods.len())
        .map(|p| chains.iter().any(|chain| chain[p].is_some()))
        .collect();
    let out: Vec<Variant> = main_reps
        .iter()
        .zip(&chains)
        .filter(|((_, v), chain)| {
            let spans = chain.iter().zip(&content).all(|(m, c)| m.is_some() || !c);
            let periods = content.iter().filter(|c| **c).count();
            spans && (v.track != Track::Subtitle || periods == 1)
        })
        .map(|((_, v), chain)| {
            let mut segments = Vec::new();
            let mut starts = Vec::new();
            for (p, at) in chain.iter().enumerate() {
                let Some(at) = at else { continue };
                if let Source::Planned(plan) = &periods[p].0[*at].1.source {
                    if !segments.is_empty() {
                        starts.push(segments.len());
                    }
                    segments.extend(plan.segments.iter().cloned());
                }
            }
            Variant {
                source: Source::Planned(Plan {
                    segments,
                    container: v.container,
                    periods: starts,
                }),
                ..v.clone()
            }
        })
        .collect();
    if out.is_empty() {
        return Err(Refusal::Unsupported(
            "periods whose streams do not line up".into(),
        ));
    }
    let left_out = content.iter().any(|c| !c);
    Ok(Resolution {
        variants: out,
        title: None,
        note: left_out.then(|| ADS_LEFT_OUT.to_owned()),
    })
}

/// Each period's length in seconds, when the manifest gives it.
fn period_lengths(mpd: &MPD) -> Vec<Option<f64>> {
    let secs = |d: std::time::Duration| d.as_secs_f64();
    let total = mpd.mediaPresentationDuration.map(secs);
    let mut start = Some(0.0);
    let mut out = Vec::with_capacity(mpd.periods.len());
    for (i, period) in mpd.periods.iter().enumerate() {
        start = period.start.map(secs).or(start);
        let end = match mpd.periods.get(i + 1) {
            Some(next) => next.start.map(secs),
            None => total,
        };
        let length = period
            .duration
            .map(secs)
            .or_else(|| Some(end? - start?).filter(|l| *l >= 0.0));
        start = start.zip(length).map(|(s, l)| s + l);
        out.push(length);
    }
    out
}

/// A period's clear representations with their segments, and how many were protected.
fn period_reps(
    mpd: &MPD,
    period: &Period,
    base: &str,
    seconds: Option<f64>,
) -> Result<(Vec<Rep>, usize), Refusal> {
    let base = period
        .BaseURL
        .first()
        .map_or(base.to_owned(), |b| join_url(base, &b.base));
    let period_protected =
        !mpd.ContentProtection.is_empty() || !period.ContentProtection.is_empty();
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
            if period_protected
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
            let container = match (track, Container::of_mime(mime)) {
                (Track::Subtitle, Some(c)) if c.is_subtitle() => c,
                // Subtitles in fragmented MP4, or of a type not known: no file to save.
                (Track::Subtitle, _) => continue,
                (Track::Audio, Some(Container::Mp4) | None) => Container::M4a,
                (Track::Audio, Some(c)) => c,
                (_, Some(Container::M4a) | None) => Container::Mp4,
                (_, Some(c)) => c,
            };
            let lang = rep.lang.clone().or_else(|| set.lang.clone());
            // A subtitle's name: the set's label, else its language.
            let name = set.Label.first().map(|l| l.content.trim().to_owned());
            let variant = Variant {
                track,
                container,
                height: rep
                    .height
                    .or(set.height)
                    .and_then(|h| u32::try_from(h).ok()),
                bandwidth: rep.bandwidth,
                codecs: match track {
                    Track::Subtitle => name.or_else(|| lang.clone()).unwrap_or_default(),
                    _ => rep
                        .codecs
                        .clone()
                        .or_else(|| set.codecs.clone())
                        .unwrap_or_default(),
                },
                size: None,
                source: Source::Planned(Plan {
                    segments,
                    container,
                    periods: vec![],
                }),
                group: None,
                headers: vec![],
                lang,
            };
            out.push((rep.id.clone(), variant));
        }
    }
    Ok((out, protected))
}

/// `want`'s representation among another period's `reps`: of the same kind, the same `@id`,
/// else the same height and codecs at the nearest bit rate.
fn matching(want: &Rep, reps: &[Rep]) -> Option<usize> {
    let (id, v) = want;
    let kind = |r: &Variant| r.track == v.track;
    let by_id = id.as_ref().and_then(|id| {
        reps.iter()
            .position(|(rid, r)| kind(r) && rid.as_ref() == Some(id))
    });
    by_id.or_else(|| {
        reps.iter()
            .enumerate()
            .filter(|(_, (_, r))| kind(r) && r.height == v.height && r.codecs == v.codecs)
            .min_by_key(|(_, (_, r))| r.bandwidth.unwrap_or(0).abs_diff(v.bandwidth.unwrap_or(0)))
            .map(|(i, _)| i)
    })
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
        // `text/vtt`'s major type, `application/ttml+xml`'s, or a set that says so.
        "text" | "application" => Some(Track::Subtitle),
        _ => None,
    }
}

/// `a-b` (inclusive) as a range.
fn parse_range(text: &str) -> Option<ByteRange> {
    let (a, b) = text.trim().split_once('-')?;
    let (start, end): (u64, u64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    if end < start {
        return None;
    }
    Some(ByteRange {
        start,
        len: (end - start).checked_add(1)?,
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
        // An `Initialization` without a `sourceURL` is a byte range of the representation's
        // own file (ISO/IEC 23009-1 5.3.9.2; ffmpeg's `-single_file 1` writes it so).
        let init = list.Initialization.as_ref().map(|i| Segment {
            url: i
                .sourceURL
                .as_deref()
                .map_or_else(|| base.to_owned(), |u| join_url(base, u)),
            range: i.range.as_deref().and_then(parse_range),
            key: None,
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
                    .ok()
                    .map(|w| w.min(MAX_FORMAT_WIDTH)),
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
    // Media time of the period's start: `$Time$` and the timeline's end count from it.
    let offset = t.presentationTimeOffset.unwrap_or(0);
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
        let end = seconds.map(|s| offset.saturating_add((s * timescale as f64) as u64));
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
            if number.saturating_sub(start).saturating_add(repeat) >= MAX_SEGMENTS {
                return Err(too_many());
            }
            for _ in 0..=repeat {
                out.push(Segment::whole(&join_url(
                    base,
                    &fill(media, rep, number, time),
                )));
                time = time.saturating_add(s.d);
                number = number.saturating_add(1);
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
        Some(end) if end >= start => count.min((end - start).saturating_add(1)),
        _ => count,
    };
    if count > MAX_SEGMENTS {
        return Err(too_many());
    }
    for n in 0..count {
        let time = offset.saturating_add((n as f64 * duration) as u64);
        out.push(Segment::whole(&join_url(
            base,
            &fill(media, rep, start.saturating_add(n), time),
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
        let v = variants(&mpd(AV), BASE).unwrap().variants;
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
        let v = variants(&mpd(body), BASE).unwrap().variants;
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
        let v = variants(&mpd(body), BASE).unwrap().variants;
        assert_eq!(
            (v[0].track, v[0].container),
            (Track::Muxed, Container::Webm)
        );
        assert_eq!(
            v[0].source,
            Source::Planned(Plan {
                segments: vec![Segment::whole("https://cdn.test/dash/media/clip.webm")],
                container: Container::Webm,
                periods: vec![],
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

    /// One file per representation, its init and media segments byte ranges of it (the
    /// on-demand shape, ffmpeg's `-single_file 1`): the init range is the file's first bytes,
    /// and without it the saved file has no `moov` and does not play.
    #[test]
    fn a_segment_list_in_one_file_starts_with_its_init_range() {
        let body = "<AdaptationSet mimeType=\"video/mp4\"><Representation id=\"1\" bandwidth=\"1\" height=\"360\">\
            <BaseURL>v.mp4</BaseURL><SegmentList duration=\"2\"><Initialization range=\"0-819\"/>\
            <SegmentURL mediaRange=\"820-1000\"/><SegmentURL mediaRange=\"1001-2000\"/></SegmentList>\
            </Representation></AdaptationSet>";
        let v = variants(&mpd(body), BASE).unwrap().variants;
        let Source::Planned(plan) = &v[0].source else {
            panic!()
        };
        let got: Vec<(&str, Option<ByteRange>)> = plan
            .segments
            .iter()
            .map(|s| (s.url.as_str(), s.range))
            .collect();
        let file = "https://cdn.test/dash/v.mp4";
        assert_eq!(
            got,
            [
                (file, Some(ByteRange { start: 0, len: 820 })),
                (
                    file,
                    Some(ByteRange {
                        start: 820,
                        len: 181
                    })
                ),
                (
                    file,
                    Some(ByteRange {
                        start: 1001,
                        len: 1000
                    })
                ),
            ]
        );
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
        let v = variants(&mpd(mixed), BASE).unwrap().variants;
        assert_eq!((v.len(), v[0].track), (1, Track::Audio));
    }

    #[test]
    fn live_empty_and_junk_are_refused() {
        let live = "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"dynamic\"><Period/></MPD>";
        assert_eq!(variants(live, BASE), Err(Refusal::Live));
        let two = "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\"><Period/><Period/></MPD>";
        assert!(matches!(variants(two, BASE), Err(Refusal::Failed(_))));
        assert!(matches!(
            variants("not xml at all", BASE),
            Err(Refusal::Failed(_))
        ));
    }

    /// A manifest of `periods` (each `(attributes, body)`), ten seconds long.
    fn periods(periods: &[(&str, &str)]) -> String {
        let body: String = periods
            .iter()
            .map(|(attrs, body)| format!("<Period {attrs}>{body}</Period>"))
            .collect();
        format!(
            "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" \
             mediaPresentationDuration=\"PT10S\">{body}</MPD>"
        )
    }

    /// A period's video and audio: representation ids `vid` and `vid` + `a` under `dir`, two-second
    /// segments, the video `height` high in `vcodec`, the audio in `acodec`.
    fn av(dir: &str, vid: &str, height: u32, vcodec: &str, acodec: &str) -> String {
        format!(
            "<AdaptationSet contentType=\"video\" mimeType=\"video/mp4\">\
             <SegmentTemplate initialization=\"{dir}/$RepresentationID$-init.mp4\" media=\"{dir}/$RepresentationID$-$Number$.m4s\" duration=\"2\"/>\
             <Representation id=\"{vid}\" bandwidth=\"1000\" height=\"{height}\" codecs=\"{vcodec}\"/></AdaptationSet>\
             <AdaptationSet contentType=\"audio\" mimeType=\"audio/mp4\">\
             <SegmentTemplate initialization=\"{dir}/a-init.mp4\" media=\"{dir}/a-$Number$.m4s\" duration=\"2\"/>\
             <Representation id=\"{vid}a\" bandwidth=\"128\" codecs=\"{acodec}\"/></AdaptationSet>"
        )
    }

    fn urls(v: &Variant) -> (Vec<String>, Vec<usize>) {
        let Source::Planned(plan) = &v.source else {
            panic!("not planned")
        };
        let names = plan
            .segments
            .iter()
            .map(|s| {
                s.url
                    .trim_start_matches("https://cdn.test/dash/")
                    .to_owned()
            })
            .collect();
        (names, plan.periods.clone())
    }

    /// A pre-roll ad, then the programme in two periods around a mid-roll that repeats its
    /// streams: the ad is left out, the programme's periods are chained, each with its init.
    #[test]
    fn periods_chain_by_id_and_ads_are_left_out() {
        let text = periods(&[
            (
                "id=\"ad\" duration=\"PT2S\"",
                &*av("ad", "adv", 480, "avc1.4d401e", "mp4a.40.5"),
            ),
            (
                "id=\"p1\" start=\"PT2S\" duration=\"PT4S\"",
                &*av("p1", "v", 720, "avc1.64001f", "mp4a.40.2"),
            ),
            (
                "id=\"p2\" start=\"PT6S\"",
                &*av("p2", "v", 720, "avc1.64001f", "mp4a.40.2"),
            ),
        ]);
        let r = variants(&text, BASE).unwrap();
        assert_eq!(r.note.as_deref(), Some(ADS_LEFT_OUT));
        assert_eq!(r.variants.len(), 2);
        assert_eq!(
            (r.variants[0].track, r.variants[0].height),
            (Track::Video, Some(720))
        );
        let (names, starts) = urls(&r.variants[0]);
        assert_eq!(
            names,
            [
                "p1/v-init.mp4",
                "p1/v-1.m4s",
                "p1/v-2.m4s",
                "p2/v-init.mp4",
                "p2/v-1.m4s",
                "p2/v-2.m4s",
            ]
        );
        assert_eq!(starts, [3]);
        let (names, starts) = urls(&r.variants[1]);
        assert_eq!((names.len(), starts), (6, vec![3]));
    }

    /// Without ids in common, the same height and codecs match, at the nearest bit rate; a
    /// period's length comes from the next one's start, the last one's from the presentation's.
    #[test]
    fn periods_match_by_attributes_and_time_their_segments() {
        let text = periods(&[
            (
                "start=\"PT0S\"",
                &*av("x", "v1", 720, "avc1.64001f", "mp4a.40.2"),
            ),
            (
                "start=\"PT6S\"",
                &*av("y", "v9", 720, "avc1.64001f", "mp4a.40.2"),
            ),
        ]);
        let r = variants(&text, BASE).unwrap();
        assert_eq!(r.note, None);
        let (names, starts) = urls(&r.variants[0]);
        // 6 s and 4 s at 2 s a segment: 3 and 2, each after its init.
        assert_eq!(
            names,
            [
                "x/v1-init.mp4",
                "x/v1-1.m4s",
                "x/v1-2.m4s",
                "x/v1-3.m4s",
                "y/v9-init.mp4",
                "y/v9-1.m4s",
                "y/v9-2.m4s"
            ]
        );
        assert_eq!(starts, [4]);
        // One period alone has no breaks.
        let one = variants(&mpd(AV), BASE).unwrap();
        assert!(one.variants.iter().all(|v| urls(v).1.is_empty()));
        assert_eq!(one.note, None);
    }

    /// A representation that misses a period others match is not offered; when none spans
    /// the programme, the manifest is refused. A protected programme after a clear ad is
    /// protected, not the ad.
    #[test]
    fn periods_that_do_not_line_up_are_refused() {
        let video = |dir: &str| {
            format!(
                "<AdaptationSet contentType=\"video\" mimeType=\"video/mp4\"><Representation id=\"v\" bandwidth=\"1\" height=\"720\" codecs=\"avc1\"><BaseURL>{dir}.mp4</BaseURL></Representation></AdaptationSet>"
            )
        };
        let audio = |dir: &str| {
            format!(
                "<AdaptationSet contentType=\"audio\" mimeType=\"audio/mp4\"><Representation id=\"a\" bandwidth=\"1\" codecs=\"mp4a.40.2\"><BaseURL>{dir}.m4a</BaseURL></Representation></AdaptationSet>"
            )
        };
        let main = format!("{}{}", video("m"), audio("m"));
        // The second period has video only: the audio is not offered, the video is.
        let text = periods(&[
            ("duration=\"PT6S\"", &*main),
            ("duration=\"PT4S\"", &*video("b")),
        ]);
        let r = variants(&text, BASE).unwrap();
        assert_eq!(r.variants.len(), 1);
        assert_eq!(urls(&r.variants[0]).0, ["m.mp4", "b.mp4"]);
        // Video only in one period, audio only in the other: nothing spans both.
        let text = periods(&[
            ("duration=\"PT6S\"", &*main),
            ("duration=\"PT2S\"", &*video("b")),
            ("duration=\"PT2S\"", &*audio("c")),
        ]);
        assert!(matches!(
            variants(&text, BASE),
            Err(Refusal::Unsupported(_))
        ));
        let sealed = main.replace(
            "<Representation id=\"v\"",
            "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Representation id=\"v\"",
        ).replace(
            "<Representation id=\"a\"",
            "<ContentProtection schemeIdUri=\"urn:mpeg:dash:mp4protection:2011\"/><Representation id=\"a\"",
        );
        let text = periods(&[
            ("duration=\"PT2S\"", &*main),
            ("duration=\"PT8S\"", &*sealed),
        ]);
        assert_eq!(variants(&text, BASE), Err(Refusal::Protected));
    }

    /// WebVTT and TTML text sets are subtitles with their language and label; subtitles in
    /// fragmented MP4 are left out, and so are a programme's of several periods.
    #[test]
    fn text_sets_are_subtitles() {
        let text = format!(
            "{AV}<AdaptationSet contentType=\"text\" mimeType=\"text/vtt\" lang=\"en\"><Label>English</Label>\
             <Representation id=\"s1\" bandwidth=\"1\"><BaseURL>subs/en.vtt</BaseURL></Representation></AdaptationSet>\
             <AdaptationSet mimeType=\"application/ttml+xml\" lang=\"fr\">\
             <Representation id=\"s2\" bandwidth=\"1\"><BaseURL>subs/fr.ttml</BaseURL></Representation></AdaptationSet>\
             <AdaptationSet contentType=\"text\" mimeType=\"application/mp4\" lang=\"de\">\
             <Representation id=\"s3\" bandwidth=\"1\" codecs=\"stpp\"><BaseURL>subs/de.mp4</BaseURL></Representation></AdaptationSet>"
        );
        let v = variants(&mpd(&text), BASE).unwrap().variants;
        let subs: Vec<(Container, Option<&str>, &str)> = v
            .iter()
            .filter(|v| v.track == Track::Subtitle)
            .map(|v| (v.container, v.lang.as_deref(), v.codecs.as_str()))
            .collect();
        assert_eq!(
            subs,
            [
                (Container::Vtt, Some("en"), "English"),
                (Container::Ttml, Some("fr"), "fr")
            ]
        );
        let sub = v.iter().find(|v| v.track == Track::Subtitle).unwrap();
        assert_eq!(urls(sub).0, ["subs/en.vtt"]);
        // The audio stays M4A, the video MP4.
        assert!(
            v.iter()
                .any(|v| v.track == Track::Audio && v.container == Container::M4a)
        );
        let track = "<AdaptationSet contentType=\"text\" mimeType=\"text/vtt\" lang=\"en\"><Representation id=\"s\" bandwidth=\"1\"><BaseURL>en.vtt</BaseURL></Representation></AdaptationSet>";
        let one = format!("{}{track}", av("p", "v", 720, "avc1", "mp4a.40.2"));
        let two = periods(&[("duration=\"PT5S\"", &*one), ("duration=\"PT5S\"", &*one)]);
        let r = variants(&two, BASE).unwrap();
        assert!(!r.variants.is_empty());
        assert!(r.variants.iter().all(|v| v.track != Track::Subtitle));
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
        // Huge format width is capped rather than allocating GiB.
        let capped = fill("$Number%099999999999d$.m4s", &rep, 7, 0);
        assert_eq!(capped.len(), MAX_FORMAT_WIDTH + 4);
        assert!(capped.ends_with("7.m4s"));
    }

    #[test]
    fn crafted_ranges_and_timeline_overflows_do_not_panic() {
        assert_eq!(parse_range("0-18446744073709551615"), None);
        assert_eq!(
            parse_range("1-18446744073709551615"),
            Some(ByteRange {
                start: 1,
                len: u64::MAX
            })
        );
        // Second timeline entry with r=-1 and huge next t must hit MAX_SEGMENTS, not wrap.
        let crafted = "<AdaptationSet contentType=\"video\" mimeType=\"video/mp4\">\
            <SegmentTemplate media=\"s-$Number$.m4s\" startNumber=\"1\" timescale=\"1\">\
              <SegmentTimeline>\
                <S t=\"0\" d=\"1\" r=\"1\"/>\
                <S t=\"2\" d=\"1\" r=\"-1\"/>\
                <S t=\"18446744073709551615\" d=\"1\"/>\
              </SegmentTimeline>\
            </SegmentTemplate>\
            <Representation id=\"v\" bandwidth=\"1\"/>\
          </AdaptationSet>";
        assert!(matches!(
            variants(&mpd(crafted), BASE),
            Err(Refusal::Unsupported(_))
        ));
    }
}
