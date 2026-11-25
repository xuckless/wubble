//! Which variants make a download: the menu's choices (each muxed stream, each video stream
//! paired with its best audio, each audio stream alone) and the one a default quality picks.
//! Pairing follows yt-dlp's default `bestvideo+bestaudio` in spirit (yt-dlp, Unlicense): the
//! audio of the video's HLS audio group, else of a container that joins with it (MP4 with
//! M4A, WebM with WebM), the highest bitrate first. At one height (or bit rate) the default
//! takes the codec most players open ([`CODEC_ORDER`]): H.264 before VP9, AV1 and HEVC, AAC
//! before Opus.

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
    /// The codec's place in [`CODEC_ORDER`] (the picture's, or the sound's for audio only):
    /// lower is taken first at one height.
    pub preference: u8,
}

/// The default's codec preference, most widely played first: H.264 opens in every player and
/// editor; VP9 and AV1 in browsers and current players; HEVC and Dolby Vision only where the
/// system licenses them. Sound: AAC, then Opus, then the rest. A `codecs` string naming none of
/// these (or nothing: most unlabelled streams are H.264 and AAC) ranks [`UNLISTED_CODEC`].
/// Matched on the first codec of the string, by prefix; a new codec is a row here, read by
/// [`choose`] and by the pairing of video with audio.
const CODEC_ORDER: [(&str, u8); 15] = [
    ("avc1", 0),
    ("avc3", 0),
    ("vp09", 2),
    ("vp9", 2),
    ("vp8", 2),
    ("av01", 3),
    ("hvc1", 4),
    ("hev1", 4),
    ("dvh1", 5),
    ("dvhe", 5),
    ("mp4a", 0),
    ("opus", 2),
    ("vorbis", 3),
    ("ac-3", 4),
    ("ec-3", 4),
];

/// The rank of a codec [`CODEC_ORDER`] does not name: after H.264 and AAC, before the rest.
const UNLISTED_CODEC: u8 = 1;

/// A `codecs` string's place in [`CODEC_ORDER`].
fn codec_rank(codecs: &str) -> u8 {
    let first = codecs
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    CODEC_ORDER
        .iter()
        .find(|(prefix, _)| first.starts_with(prefix))
        .map_or(UNLISTED_CODEC, |(_, rank)| *rank)
}

fn joins(video: Container, audio: Container) -> bool {
    matches!(
        (video, audio),
        (Container::Mp4, Container::M4a | Container::Mp4)
            | (Container::Webm, Container::Webm)
            | (Container::Ts, _)
    )
}

/// The best audio for `video`: its HLS group's, else a joinable container's; the preferred codec,
/// then the highest bitrate.
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
        .max_by_key(|(_, a)| {
            (
                std::cmp::Reverse(codec_rank(&a.codecs)),
                a.bandwidth.unwrap_or(0),
            )
        })
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
                preference: codec_rank(&v.codecs),
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
    // The codec family, the bit rate, then what the manifest calls it (an HLS rendition's name).
    for pass in 0..3 {
        let labels: Vec<String> = rows.iter().map(|c| c.label.clone()).collect();
        for i in 0..rows.len() {
            let twins: Vec<usize> = (0..rows.len())
                .filter(|&j| labels[j] == labels[i])
                .collect();
            if twins.len() < 2 {
                continue;
            }
            let extra = match pass {
                0 => {
                    let families: Vec<_> = twins.iter().map(|&j| family(&rows[j])).collect();
                    let differ = families.iter().any(|f| *f != families[0]);
                    family(&rows[i]).filter(|_| differ).map(str::to_owned)
                }
                1 => rows[i].bandwidth.filter(|b| *b > 0).map(rate),
                _ => {
                    let (Selection::Single(v) | Selection::Pair { video: v, .. }) =
                        rows[i].selection;
                    let named = variants[v].codecs.trim();
                    (!named.is_empty()).then(|| named.to_owned())
                }
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
/// shortest above it when none fits), or the shortest; of that height, the preferred codec
/// ([`CODEC_ORDER`]), then the first (the highest bit rate). `can_join`: a muxer is there;
/// without one a pair saves as two files, so a stream with its sound inside is taken when there
/// is one. A manifest that names no heights is judged by its bit rates (the ladder above); a
/// plain file, with neither, is the first choice; only sound, the preferred codec.
pub fn choose(choices: &[Choice], quality: Quality, can_join: bool) -> Option<usize> {
    let whole = choices.iter().any(|c| !c.pair && !c.audio_only);
    let usable = |c: &Choice| (can_join || !whole || !c.pair) && !c.audio_only;
    let pool: Vec<usize> = (0..choices.len())
        .filter(|&i| usable(&choices[i]))
        .collect();
    // Only audio on offer: the preferred codec, the first (highest bit rate) of it.
    let first = pool
        .first()
        .copied()
        .or_else(|| (0..choices.len()).min_by_key(|&i| choices[i].preference))?;
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
    // `choices` is sorted tallest first, then by bit rate: of one height, the preferred codec's
    // first row is its best stream.
    let at = |height: u32| {
        pictured
            .iter()
            .filter(|(_, h)| *h == height)
            .min_by_key(|(i, _)| choices[*i].preference)
            .map(|(i, _)| *i)
    };
    let tallest = |cap: u32| pictured.iter().map(|(_, h)| *h).find(|h| *h <= cap);
    let shortest = || pictured.iter().map(|(_, h)| *h).min();
    let height = match quality {
        Quality::Best => Some(pictured[0].1),
        Quality::UpTo(cap) => tallest(cap).or_else(shortest),
        Quality::Smallest => shortest(),
    };
    at(height?)
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
        // Renditions that differ only by name (an HLS audio group's `NAME`, kept in `codecs`).
        let named = |name: &str| rated(Track::Audio, None, 0, name);
        let labels: Vec<String> = choices(&[named("English"), named("English 5.1")])
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert!(
            labels.iter().any(|l| l.contains("English 5.1")),
            "{labels:?}"
        );
        assert_ne!(labels[0], labels[1]);
        assert_eq!(codec_name("vp9"), Some("VP9"));
        assert_eq!(codec_name("mp4a.40.2"), None);
    }

    /// At one height the default takes H.264 over VP9, AV1 and HEVC, whatever their bit rates;
    /// the menu's rows and labels stay as they were.
    #[test]
    fn the_default_prefers_the_codec_most_players_open() {
        let video = |height, bits, codecs: &str| Variant {
            container: Container::Mp4,
            ..rated(Track::Video, Some(height), bits, codecs)
        };
        let audio = |bits, codecs: &str| Variant {
            container: Container::M4a,
            ..rated(Track::Audio, None, bits, codecs)
        };
        let muxed = |height, bits, codecs: &str| Variant {
            container: Container::Mp4,
            ..rated(Track::Muxed, Some(height), bits, codecs)
        };
        // (name, variants, quality, can_join, the codecs of the picked row's first part)
        let cases: Vec<(&str, Vec<Variant>, Quality, bool, &str)> = vec![
            (
                "hevc above h264 at 1080p",
                vec![
                    video(1080, 6, "hvc1.2.4.L120"),
                    video(1080, 5, "avc1.640028"),
                    audio(1, "mp4a.40.2"),
                ],
                Quality::Best,
                true,
                "avc1.640028",
            ),
            (
                "av1 and vp9 at 720p: vp9",
                vec![
                    video(720, 3, "av01.0.05M.08"),
                    video(720, 2, "vp09.00.31.08"),
                    audio(1, "mp4a.40.2"),
                ],
                Quality::UpTo(720),
                true,
                "vp09.00.31.08",
            ),
            (
                "only hevc at the cap: hevc, not a shorter h264",
                vec![
                    video(1080, 6, "hvc1.1"),
                    video(720, 3, "avc1.4d"),
                    audio(1, "mp4a.40.2"),
                ],
                Quality::UpTo(1080),
                true,
                "hvc1.1",
            ),
            (
                "smallest: the preferred codec of the shortest",
                vec![
                    video(360, 2, "av01.0"),
                    video(360, 1, "avc1.42"),
                    video(720, 3, "avc1.4d"),
                    audio(1, "mp4a"),
                ],
                Quality::Smallest,
                true,
                "avc1.42",
            ),
            (
                "unlabelled before hevc",
                vec![muxed(720, 9, "hev1.1"), muxed(720, 1, "")],
                Quality::Best,
                true,
                "",
            ),
            (
                "no muxer: a muxed stream over a pair, whatever its codec",
                vec![
                    video(1080, 6, "avc1.64"),
                    audio(1, "mp4a"),
                    muxed(1080, 5, "hvc1.1"),
                ],
                Quality::Best,
                false,
                "hvc1.1",
            ),
        ];
        for (name, list, quality, join, want) in cases {
            let c = choices(&list);
            let pick = choose(&c, quality, join).expect(name);
            let first = c[pick].selection.parts()[0];
            assert_eq!(list[first].codecs, want, "{name}");
        }
        // Audio: AAC over Opus for the pair and for sound alone, even at a higher Opus rate.
        let list = vec![
            video(720, 3, "avc1.4d"),
            Variant {
                container: Container::Mp4,
                ..audio(160, "opus")
            },
            audio(128, "mp4a.40.2"),
        ];
        let c = choices(&list);
        assert_eq!(c[0].selection, Selection::Pair { video: 0, audio: 2 });
        let songs = choices(&[audio(160, "opus"), audio(96, "mp4a.40.2")]);
        let pick = choose(&songs, Quality::Best, true).unwrap();
        assert_eq!(songs[pick].selection, Selection::Single(1));
        // The labels still name the codecs that tell rows apart.
        let rows = choices(&[muxed(1080, 2, "hvc1.1"), muxed(1080, 1, "avc1.64")]);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "1080p \u{b7} HEVC \u{b7} MP4 \u{b7} 100 B",
                "1080p \u{b7} H.264 \u{b7} MP4 \u{b7} 100 B"
            ]
        );
        assert_eq!(
            (
                codec_rank("AVC1.64"),
                codec_rank("weird"),
                codec_rank("dvh1.05")
            ),
            (0, UNLISTED_CODEC, 5)
        );
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
