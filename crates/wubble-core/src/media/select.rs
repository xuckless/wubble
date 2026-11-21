//! Which variants make a download: the menu's choices (each muxed stream, each video stream
//! paired with its best audio, each audio stream alone) and the one a default quality picks.
//! Pairing follows yt-dlp's default `bestvideo+bestaudio` in spirit (yt-dlp, Unlicense): the
//! audio of the video's HLS audio group, else of a container that joins with it (MP4 with
//! M4A, WebM with WebM), the highest bitrate first.

use super::{Container, Track, Variant};
use crate::download::format_bytes;

/// The `media.quality` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Best,
    /// The tallest picture up to this height.
    UpTo(u32),
    Smallest,
}

/// The setting's words, in the order the settings page shows them.
pub const QUALITY_WORDS: [&str; 5] = ["best", "1080p", "720p", "480p", "smallest"];

impl Quality {
    pub fn from_word(word: &str) -> Quality {
        match word {
            "best" => Quality::Best,
            "smallest" => Quality::Smallest,
            w => w
                .strip_suffix('p')
                .and_then(|h| h.parse().ok())
                .map_or(Quality::Best, Quality::UpTo),
        }
    }
}

/// The variants a download takes, by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Single(usize),
    /// Separate video and audio, joined after the download.
    Pair {
        video: usize,
        audio: usize,
    },
}

impl Selection {
    /// The indices, video first.
    pub fn parts(self) -> Vec<usize> {
        match self {
            Selection::Single(i) => vec![i],
            Selection::Pair { video, audio } => vec![video, audio],
        }
    }
}

/// One row of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub selection: Selection,
    /// Bytes, when every part's size is known.
    pub size: Option<u64>,
    /// The container of the saved file (a pair saves as the video's).
    pub container: Container,
    pub height: Option<u32>,
    /// Bits per second of the picture (or the sound, for audio only), when the manifest says.
    pub bandwidth: Option<u64>,
    /// Separate video and audio: one file only with a muxer, else two.
    pub pair: bool,
    /// Sound without a picture.
    pub audio_only: bool,
}

fn joins(video: Container, audio: Container) -> bool {
    matches!(
        (video, audio),
        (Container::Mp4, Container::M4a | Container::Mp4)
            | (Container::Webm, Container::Webm)
            | (Container::Ts, _)
    )
}

/// The best audio for `video`: its HLS group's, else a joinable container's, highest bitrate.
fn audio_for(variants: &[Variant], video: &Variant) -> Option<usize> {
    let audio = variants
        .iter()
        .enumerate()
        .filter(|(_, a)| a.track == Track::Audio);
    let fits: Vec<(usize, &Variant)> = match &video.group {
        Some(g) => audio.filter(|(_, a)| a.group.as_ref() == Some(g)).collect(),
        None => audio
            .filter(|(_, a)| joins(video.container, a.container))
            .collect(),
    };
    fits.into_iter()
        .max_by_key(|(_, a)| a.bandwidth.unwrap_or(0))
        .map(|(i, _)| i)
}

/// A bit rate in words: `41 kbit/s`, `2.1 Mbit/s`.
fn rate(bits: u64) -> String {
    if bits >= 1_000_000 {
        format!("{:.1} Mbit/s", bits as f64 / 1e6)
    } else {
        format!("{} kbit/s", bits.div_ceil(1000))
    }
}

/// The codec family a `codecs` string names, for telling two rows of one height apart.
fn codec_name(codecs: &str) -> Option<&'static str> {
    const FAMILIES: [(&str, &str); 6] = [
        ("avc1", "H.264"),
        ("hvc1", "HEVC"),
        ("hev1", "HEVC"),
        ("dvh1", "Dolby Vision"),
        ("av01", "AV1"),
        ("vp09", "VP9"),
    ];
    let first = codecs.split(',').next()?.trim().to_ascii_lowercase();
    FAMILIES
        .iter()
        .find(|(prefix, _)| first.starts_with(prefix))
        .map(|(_, name)| *name)
        .or_else(|| first.starts_with("vp9").then_some("VP9"))
}

fn label(
    height: Option<u32>,
    bandwidth: Option<u64>,
    container: Container,
    size: Option<u64>,
    note: &str,
) -> String {
    let mut parts = Vec::new();
    match (height, bandwidth) {
        (Some(h), _) => parts.push(format!("{h}p")),
        (None, Some(b)) if b > 0 => parts.push(rate(b)),
        _ => {}
    }
    if container != Container::Other {
        parts.push(container.ext().to_uppercase());
    }
    if let Some(s) = size {
        parts.push(format_bytes(s));
    }
    if !note.is_empty() {
        parts.push(note.to_owned());
    }
    parts.join(" \u{b7} ")
}

/// Every way to save `variants`, tallest first (then highest bitrate, muxed before a pair).
pub fn choices(variants: &[Variant]) -> Vec<Choice> {
    let mut out: Vec<Choice> = variants
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (selection, size, note) = match v.track {
                Track::Video => match audio_for(variants, v) {
                    Some(a) => (
                        Selection::Pair { video: i, audio: a },
                        v.size.zip(variants[a].size).map(|(x, y)| x + y),
                        "",
                    ),
                    None => (Selection::Single(i), v.size, "no sound"),
                },
                Track::Audio => (Selection::Single(i), v.size, "audio only"),
                Track::Muxed => (Selection::Single(i), v.size, ""),
            };
            Choice {
                label: label(v.height, v.bandwidth, v.container, size, note),
                selection,
                size,
                container: v.container,
                height: v.height,
                bandwidth: v.bandwidth,
                pair: matches!(selection, Selection::Pair { .. }),
                audio_only: v.track == Track::Audio,
            }
        })
        .collect();
    let rank = |c: &Choice| {
        let parts = c.selection.parts();
        let v = &variants[parts[0]];
        (
            v.track != Track::Audio,
            c.height.unwrap_or(0),
            v.bandwidth.unwrap_or(0),
            parts.len() == 1,
        )
    };
    out.sort_by_key(|c| std::cmp::Reverse(rank(c)));
    tell_apart(&mut out, variants);
    out
}

/// Rows with one label (the same height from two codecs or two bit rates) get the codec when
/// the codecs differ, then the bit rate, so a menu never lists two rows that read alike.
fn tell_apart(rows: &mut [Choice], variants: &[Variant]) {
    let family = |c: &Choice| {
        let (Selection::Single(v) | Selection::Pair { video: v, .. }) = c.selection;
        codec_name(&variants[v].codecs)
    };
    for by_codec in [true, false] {
        let labels: Vec<String> = rows.iter().map(|c| c.label.clone()).collect();
        for i in 0..rows.len() {
            let twins: Vec<usize> = (0..rows.len()).filter(|&j| labels[j] == labels[i]).collect();
            if twins.len() < 2 {
                continue;
            }
            let extra = if by_codec {
                let families: Vec<_> = twins.iter().map(|&j| family(&rows[j])).collect();
                let differ = families.iter().any(|f| *f != families[0]);
                family(&rows[i]).filter(|_| differ).map(str::to_owned)
            } else {
                rows[i].bandwidth.filter(|b| *b > 0).map(rate)
            };
            if let Some(extra) = extra {
                rows[i].label = match rows[i].label.split_once(" \u{b7} ") {
                    Some((first, rest)) => format!("{first} \u{b7} {extra} \u{b7} {rest}"),
                    None => format!("{} \u{b7} {extra}", rows[i].label),
                };
            }
        }
    }
}

/// Typical bit rate of an H.264 ladder rung by picture height, for a manifest that names no
/// resolution (Apple's HLS Authoring Specification recommends 0.4, 1.1, 2.0, 4.5 and 7.8 Mbit/s
/// for 240p, 360p, 480p, 720p and 1080p; rounded up a little).
const LADDER_BITS: [(u32, u64); 5] = [
    (240, 600_000),
    (360, 1_200_000),
    (480, 2_200_000),
    (720, 5_000_000),
    (1080, 9_000_000),
];

/// The choice `quality` picks: among choices with a picture, the tallest within the cap (the
/// shortest above it when none fits), or the shortest. `can_join`: a muxer is there; without
/// one a pair saves as two files, so a stream with its sound inside is taken when there is one.
/// A manifest that names no heights is judged by its bit rates (the ladder above); a plain
/// file, with neither, is the first choice.
pub fn choose(choices: &[Choice], quality: Quality, can_join: bool) -> Option<usize> {
    let whole = choices.iter().any(|c| !c.pair && !c.audio_only);
    let usable = |c: &Choice| (can_join || !whole || !c.pair) && !c.audio_only;
    let pool: Vec<usize> = (0..choices.len()).filter(|&i| usable(&choices[i])).collect();
    // Only audio on offer: the first of them.
    let first = pool
        .first()
        .copied()
        .or_else(|| (!choices.is_empty()).then_some(0))?;
    let pictured: Vec<(usize, u32)> = pool
        .iter()
        .filter_map(|&i| Some((i, choices[i].height?)))
        .collect();
    if pictured.is_empty() {
        let rated: Vec<(usize, u64)> = pool
            .iter()
            .filter_map(|&i| Some((i, choices[i].bandwidth?)))
            .collect();
        if rated.is_empty() {
            return Some(first);
        }
        let lowest = || rated.iter().min_by_key(|(_, b)| *b).map(|(i, _)| *i);
        let top = || rated.iter().max_by_key(|(_, b)| *b).map(|(i, _)| *i);
        return match quality {
            Quality::Best => top(),
            Quality::Smallest => lowest(),
            Quality::UpTo(cap) => {
                let limit = LADDER_BITS
                    .iter()
                    .find(|(h, _)| *h >= cap)
                    .map_or(u64::MAX, |(_, b)| *b);
                rated
                    .iter()
                    .filter(|(_, b)| *b <= limit)
                    .max_by_key(|(_, b)| *b)
                    .map(|(i, _)| *i)
                    .or_else(lowest)
            }
        };
    }
    // `choices` is sorted tallest first, so the first match of a height is its best stream.
    let tallest = |cap: u32| pictured.iter().find(|(_, h)| *h <= cap).map(|(i, _)| *i);
    let shortest = || {
        let min = pictured.iter().map(|(_, h)| *h).min()?;
        pictured.iter().find(|(_, h)| *h == min).map(|(i, _)| *i)
    };
    match quality {
        Quality::Best => Some(pictured[0].0),
        Quality::UpTo(cap) => tallest(cap).or_else(shortest),
        Quality::Smallest => shortest(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Source;

    fn v(track: Track, container: Container, height: Option<u32>, bandwidth: u64) -> Variant {
        Variant {
            track,
            container,
            height,
            bandwidth: Some(bandwidth),
            codecs: String::new(),
            size: Some(100),
            source: Source::HlsPlaylist(String::new()),
            group: None,
            headers: vec![],
        }
    }

    #[test]
    fn quality_words() {
        assert_eq!(Quality::from_word("720p"), Quality::UpTo(720));
        assert_eq!(Quality::from_word("smallest"), Quality::Smallest);
        assert_eq!(Quality::from_word("junk"), Quality::Best);
        assert!(
            QUALITY_WORDS
                .iter()
                .all(|w| *w == "best" || Quality::from_word(w) != Quality::Best)
        );
    }

    #[test]
    fn videos_pair_with_the_best_joinable_audio() {
        let list = vec![
            v(Track::Video, Container::Mp4, Some(1080), 4_000_000),
            v(Track::Video, Container::Webm, Some(1080), 3_000_000),
            v(Track::Audio, Container::M4a, None, 128_000),
            v(Track::Audio, Container::M4a, None, 64_000),
            v(Track::Audio, Container::Webm, None, 160_000),
            v(Track::Muxed, Container::Mp4, Some(360), 500_000),
        ];
        let c = choices(&list);
        assert_eq!(c[0].selection, Selection::Pair { video: 0, audio: 2 });
        assert_eq!(c[0].size, Some(200));
        assert_eq!(c[0].label, "1080p \u{b7} MP4 \u{b7} 200 B");
        assert_eq!(c[1].selection, Selection::Pair { video: 1, audio: 4 });
        assert_eq!(c[2].selection, Selection::Single(5));
        // Audio rows last.
        assert!(c[3..].iter().all(|c| c.label.ends_with("audio only")));
    }

    #[test]
    fn hls_groups_pair_and_lonely_video_says_so() {
        let mut video = v(Track::Video, Container::Ts, Some(720), 1);
        video.group = Some("a".into());
        let mut other = v(Track::Audio, Container::Ts, None, 9);
        other.group = Some("b".into());
        let mut own = v(Track::Audio, Container::Ts, None, 1);
        own.group = Some("a".into());
        let c = choices(&[video.clone(), other, own]);
        assert_eq!(c[0].selection, Selection::Pair { video: 0, audio: 2 });
        let alone = choices(&[v(Track::Video, Container::Webm, Some(480), 1)]);
        assert!(alone[0].label.ends_with("no sound"));
    }

    #[test]
    fn default_quality_picks_within_the_cap() {
        let list = vec![
            v(Track::Muxed, Container::Mp4, Some(1080), 3),
            v(Track::Muxed, Container::Mp4, Some(720), 2),
            v(Track::Muxed, Container::Mp4, Some(720), 1),
            v(Track::Muxed, Container::Mp4, Some(360), 1),
        ];
        let c = choices(&list);
        let pick = |q| choose(&c, q, true).map(|i| c[i].selection);
        assert_eq!(pick(Quality::Best), Some(Selection::Single(0)));
        assert_eq!(pick(Quality::UpTo(720)), Some(Selection::Single(1)));
        assert_eq!(pick(Quality::UpTo(480)), Some(Selection::Single(3)));
        assert_eq!(pick(Quality::UpTo(240)), Some(Selection::Single(3)));
        assert_eq!(pick(Quality::Smallest), Some(Selection::Single(3)));
        // No heights at all (a plain file): the first.
        let plain = choices(&[v(Track::Muxed, Container::Mp4, None, 0)]);
        assert_eq!(choose(&plain, Quality::Smallest, true), Some(0));
        assert_eq!(choose(&[], Quality::Best, true), None);
    }

    fn rated(track: Track, height: Option<u32>, bandwidth: u64, codecs: &str) -> Variant {
        Variant {
            codecs: codecs.into(),
            ..v(track, Container::Other, height, bandwidth)
        }
    }

    /// An HLS master without resolutions (Apple's bipbop basic): the quality setting still
    /// picks by bit rate; an audio-only variant is never "the smallest video".
    #[test]
    fn without_heights_the_bit_rate_decides() {
        let list = vec![
            rated(Track::Muxed, None, 1_900_000, ""),
            rated(Track::Muxed, None, 1_000_000, ""),
            rated(Track::Muxed, None, 650_000, ""),
            rated(Track::Muxed, None, 230_000, ""),
            rated(Track::Audio, None, 41_000, ""),
        ];
        let c = choices(&list);
        let bits = |q| choose(&c, q, true).and_then(|i| c[i].bandwidth);
        assert_eq!(bits(Quality::Best), Some(1_900_000));
        assert_eq!(bits(Quality::Smallest), Some(230_000));
        assert_eq!(bits(Quality::UpTo(720)), Some(1_900_000));
        assert_eq!(bits(Quality::UpTo(360)), Some(1_000_000));
        assert_eq!(bits(Quality::UpTo(240)), Some(230_000));
        assert!(c.last().is_some_and(|c| c.audio_only));
    }

    #[test]
    fn a_pair_without_a_muxer_gives_way_to_a_stream_with_its_sound() {
        let list = vec![
            v(Track::Video, Container::Mp4, Some(720), 5),
            v(Track::Audio, Container::M4a, None, 1),
            v(Track::Muxed, Container::Mp4, Some(360), 1),
        ];
        let c = choices(&list);
        let pick = |join| choose(&c, Quality::UpTo(720), join).map(|i| (c[i].pair, c[i].height));
        assert_eq!(pick(true), Some((true, Some(720))));
        assert_eq!(pick(false), Some((false, Some(360))));
        // Nothing but pairs: taken as they are.
        let only = choices(&list[..2]);
        assert_eq!(
            choose(&only, Quality::Best, false).map(|i| only[i].pair),
            Some(true)
        );
        // Only sound: the sound.
        let song = choices(&[v(Track::Audio, Container::Mp3, None, 1)]);
        assert_eq!(choose(&song, Quality::Best, false), Some(0));
    }

    #[test]
    fn rows_that_read_alike_are_told_apart() {
        let list = vec![
            rated(Track::Muxed, Some(1080), 8_000_000, "avc1.640028"),
            rated(Track::Muxed, Some(1080), 6_000_000, "hvc1.2.4.L120"),
            rated(Track::Muxed, Some(720), 3_000_000, "avc1.64001f"),
            rated(Track::Muxed, Some(720), 2_000_000, "avc1.64001f"),
            rated(Track::Muxed, Some(480), 1_000_000, "avc1.64001e"),
        ];
        let labels: Vec<String> = choices(&list).into_iter().map(|c| c.label).collect();
        assert_eq!(
            labels,
            [
                "1080p \u{b7} H.264 \u{b7} 100 B",
                "1080p \u{b7} HEVC \u{b7} 100 B",
                "720p \u{b7} 3.0 Mbit/s \u{b7} 100 B",
                "720p \u{b7} 2.0 Mbit/s \u{b7} 100 B",
                "480p \u{b7} 100 B"
            ]
        );
        assert_eq!(rate(41_457), "42 kbit/s");
        assert_eq!(codec_name("vp9"), Some("VP9"));
        assert_eq!(codec_name("mp4a.40.2"), None);
    }

    #[test]
    fn muxed_wins_a_tie_with_a_pair() {
        let list = vec![
            v(Track::Video, Container::Mp4, Some(720), 5),
            v(Track::Audio, Container::M4a, None, 1),
            v(Track::Muxed, Container::Mp4, Some(720), 5),
        ];
        let c = choices(&list);
        assert_eq!(c[0].selection, Selection::Single(2));
    }
}
