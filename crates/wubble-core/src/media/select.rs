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
        (None, Some(b)) if b > 0 => parts.push(format!("{:.1} Mbit/s", b as f64 / 1e6)),
        _ => {}
    }
    parts.push(container.ext().to_uppercase());
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
    out
}

/// The choice `quality` picks: among choices with a picture, the tallest within the cap (the
/// shortest above it when none fits), or the shortest; the first choice when none has a height.
pub fn choose(choices: &[Choice], quality: Quality) -> Option<usize> {
    let pictured: Vec<(usize, u32)> = choices
        .iter()
        .enumerate()
        .filter_map(|(i, c)| Some((i, c.height?)))
        .collect();
    if pictured.is_empty() {
        return (!choices.is_empty()).then_some(0);
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
        let pick = |q| choose(&c, q).map(|i| c[i].selection);
        assert_eq!(pick(Quality::Best), Some(Selection::Single(0)));
        assert_eq!(pick(Quality::UpTo(720)), Some(Selection::Single(1)));
        assert_eq!(pick(Quality::UpTo(480)), Some(Selection::Single(3)));
        assert_eq!(pick(Quality::UpTo(240)), Some(Selection::Single(3)));
        assert_eq!(pick(Quality::Smallest), Some(Selection::Single(3)));
        // No heights at all (a plain file): the first.
        let plain = choices(&[v(Track::Muxed, Container::Mp4, None, 0)]);
        assert_eq!(choose(&plain, Quality::Smallest), Some(0));
        assert_eq!(choose(&[], Quality::Best), None);
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
