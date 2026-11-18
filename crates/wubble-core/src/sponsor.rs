//! SponsorBlock, built in: which YouTube video a page plays, the privacy-preserving lookup
//! (only the first four hex digits of the SHA-256 of the video id leave the machine), the
//! answer's segments, and the policy: which segments are skipped, which are only marked on the
//! progress bar ([`plan`]), and the "skipped: undo" notice ([`Skips`]).
//!
//! The page carries out jumps only (`kit.js` `jumps`); every decision is here.
//!
//! Ported policy, not copied line by line: the SponsorBlock extension
//! (github.com/ajayyy/SponsorBlock, GPL-3.0-or-later) `src/content.ts` (`getNextSkipIndex`,
//! `getStartTimes`, `skipToTime`: sorted by start, overlapping skips merged, a segment shorter
//! than half a second ignored, segments dropped when the video's duration differs) and the
//! category colours of `src/config.ts`. The data comes from the SponsorBlock API
//! (sponsor.ajay.app) under CC BY-NC-SA 4.0: it is fetched at run time, never bundled, and the
//! attribution is [`ATTRIBUTION`].

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

/// The public API's address.
pub const DEFAULT_ENDPOINT: &str = "https://sponsor.ajay.app";

/// The attribution the database's licence asks for, shown beside the setting.
pub const ATTRIBUTION: &str = "Uses SponsorBlock data from https://sponsor.ajay.app/";

/// A segment shorter than this is ignored (seconds).
pub const MIN_SEGMENT: f64 = 0.5;

/// How long the "skipped" notice can be undone (ms).
pub const TOAST_MS: u64 = 6_000;

/// The answer's duration may differ from the page's by this much (seconds) before the segments
/// are taken to be for another cut of the video (an ad, a re-upload).
pub const DURATION_SLACK: f64 = 2.0;

/// What to do with a category's segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Skip,
    /// Marked on the progress bar only.
    Show,
    Off,
}

impl Act {
    /// As stored in `settings::SPONSOR_ACTIONS` (category to number).
    pub fn code(self) -> u32 {
        match self {
            Act::Skip => 0,
            Act::Show => 1,
            Act::Off => 2,
        }
    }

    pub fn from_code(code: u32) -> Option<Act> {
        match code {
            0 => Some(Act::Skip),
            1 => Some(Act::Show),
            2 => Some(Act::Off),
            _ => None,
        }
    }
}

/// One category: its API id, its name, its colour on the bar and what wubble does by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub colour: u32,
    pub default: Act,
}

/// The categories wubble asks for (SponsorBlock `src/config.ts` colours).
pub const CATEGORIES: [Category; 8] = [
    Category {
        id: "sponsor",
        label: "Sponsor",
        colour: 0x00d400,
        default: Act::Skip,
    },
    Category {
        id: "selfpromo",
        label: "Self-promotion",
        colour: 0xffff00,
        default: Act::Skip,
    },
    Category {
        id: "interaction",
        label: "Interaction reminder",
        colour: 0xcc00ff,
        default: Act::Show,
    },
    Category {
        id: "intro",
        label: "Intro",
        colour: 0x00ffff,
        default: Act::Show,
    },
    Category {
        id: "outro",
        label: "Endcards",
        colour: 0x0202ed,
        default: Act::Show,
    },
    Category {
        id: "preview",
        label: "Preview",
        colour: 0x008fd6,
        default: Act::Show,
    },
    Category {
        id: "music_offtopic",
        label: "Non-music section",
        colour: 0xff9900,
        default: Act::Show,
    },
    Category {
        id: "filler",
        label: "Filler",
        colour: 0x7300ff,
        default: Act::Show,
    },
];

pub fn category(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id)
}

/// The stored per-category choices (category id to [`Act::code`]); absent ones are the
/// category's default.
pub type ActTable = BTreeMap<String, u32>;

pub fn act_of(table: &ActTable, id: &str) -> Act {
    table
        .get(id)
        .and_then(|&c| Act::from_code(c))
        .or_else(|| category(id).map(|c| c.default))
        .unwrap_or(Act::Off)
}

/// The YouTube video a page plays: `youtube.com/watch?v=`, `/shorts/`, `/embed/`, `/live/`,
/// `youtu.be/<id>`, `music.youtube.com`. An id is 11 of `[A-Za-z0-9_-]`.
pub fn video_id(page: &str) -> Option<String> {
    let url = url::Url::parse(page).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let host = host.strip_prefix("m.").unwrap_or(host);
    let id = match host {
        "youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            let mut segments = url.path_segments()?;
            match segments.next()? {
                "watch" => url
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned())?,
                "shorts" | "embed" | "live" | "v" => segments.next()?.to_owned(),
                _ => return None,
            }
        }
        "youtu.be" => url.path_segments()?.next()?.to_owned(),
        _ => return None,
    };
    is_video_id(&id).then_some(id)
}

fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The first four hex digits of the SHA-256 of `id`: all the lookup sends.
pub fn hash_prefix(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    digest[..2].iter().map(|b| format!("{b:02x}")).collect()
}

/// The lookup's URL on `endpoint`: every category, skip segments only, YouTube.
pub fn request_url(endpoint: &str, prefix: &str) -> String {
    let categories: Vec<&str> = CATEGORIES.iter().map(|c| c.id).collect();
    let categories = serde_json::to_string(&categories).unwrap_or_default();
    let mut out = format!(
        "{}/api/skipSegments/{prefix}?categories=",
        endpoint.trim_end_matches('/')
    );
    crate::url::percent_encode_into(&categories, &mut out);
    out.push_str("&actionTypes=");
    crate::url::percent_encode_into(r#"["skip"]"#, &mut out);
    out.push_str("&service=YouTube");
    out
}

/// One segment of a video.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub uuid: String,
    pub category: String,
    pub from: f64,
    pub to: f64,
    /// The duration of the video the segment was submitted on; 0 when unknown.
    pub video_duration: f64,
}

/// The segments of video `id` in an answer (every video under the prefix is in it). A body that
/// is no such answer (the API's 404 "Not Found", say) has none.
pub fn parse(body: &str, id: &str) -> Vec<Segment> {
    let Ok(serde_json::Value::Array(videos)) = serde_json::from_str(body) else {
        return vec![];
    };
    let Some(ours) = videos
        .iter()
        .find(|v| v.get("videoID").and_then(serde_json::Value::as_str) == Some(id))
    else {
        return vec![];
    };
    let Some(segments) = ours.get("segments").and_then(serde_json::Value::as_array) else {
        return vec![];
    };
    segments
        .iter()
        .filter_map(|s| {
            let pair = s.get("segment")?.as_array()?;
            let (from, to) = (pair.first()?.as_f64()?, pair.get(1)?.as_f64()?);
            let category = s.get("category")?.as_str()?;
            (from.is_finite() && to.is_finite() && from >= 0.0 && to > from && category.len() < 64)
                .then(|| Segment {
                    uuid: s
                        .get("UUID")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .chars()
                        .take(128)
                        .collect(),
                    category: category.to_owned(),
                    from,
                    to,
                    video_duration: s
                        .get("videoDuration")
                        .and_then(serde_json::Value::as_f64)
                        .filter(|d| d.is_finite() && *d > 0.0)
                        .unwrap_or(0.0),
                })
        })
        .collect()
}

/// A skip the page carries out: from `from` to `to`, seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Jump {
    /// Stable for one plan: the notice and undo name it.
    pub id: usize,
    pub from: f64,
    pub to: f64,
    /// The first merged segment's category.
    pub category: &'static str,
}

/// A segment drawn on the progress bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub from: f64,
    pub to: f64,
    pub colour: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub jumps: Vec<Jump>,
    pub marks: Vec<Mark>,
}

/// What to do with `segments` on a video of `duration` seconds: skip segments merged where they
/// overlap or touch, every shown or skipped segment marked. Nothing on a live stream, nothing
/// from an answer for another cut of the video, nothing shorter than [`MIN_SEGMENT`].
pub fn plan(segments: &[Segment], acts: &ActTable, duration: f64) -> Plan {
    if !duration.is_finite() || duration <= 0.0 {
        return Plan::default();
    }
    let mut kept: Vec<(&Segment, &'static Category, Act)> = segments
        .iter()
        .filter(|s| s.to - s.from >= MIN_SEGMENT)
        .filter(|s| {
            s.video_duration == 0.0 || (s.video_duration - duration).abs() <= DURATION_SLACK
        })
        .filter_map(|s| {
            let c = category(&s.category)?;
            let act = act_of(acts, c.id);
            (act != Act::Off).then_some((s, c, act))
        })
        .collect();
    kept.sort_by(|a, b| {
        a.0.from
            .total_cmp(&b.0.from)
            .then(a.0.to.total_cmp(&b.0.to))
    });
    let marks = kept
        .iter()
        .map(|(s, c, _)| Mark {
            from: s.from,
            to: s.to.min(duration),
            colour: c.colour,
        })
        .collect();
    let mut jumps: Vec<Jump> = vec![];
    for (s, c, _) in kept.iter().filter(|(_, _, act)| *act == Act::Skip) {
        match jumps.last_mut() {
            Some(last) if s.from <= last.to => last.to = last.to.max(s.to),
            _ => jumps.push(Jump {
                id: jumps.len(),
                from: s.from,
                to: s.to,
                category: c.id,
            }),
        }
    }
    for j in &mut jumps {
        j.to = j.to.min(duration);
    }
    Plan { jumps, marks }
}

/// The "skipped" notice on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub jump: usize,
    pub category: &'static str,
    /// Where undo goes back to.
    pub from: f64,
    pub until: u64,
}

impl Notice {
    pub fn text(&self) -> String {
        let label = category(self.category).map_or("segment", |c| c.label);
        format!("Skipped {}", label.to_lowercase())
    }
}

/// The skips of one video: which jumps the user undid (the page is not given them again) and
/// the notice of the last one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skips {
    undone: BTreeSet<usize>,
    notice: Option<Notice>,
}

impl Skips {
    /// The jumps of `plan` the page carries out.
    pub fn table<'a>(&self, plan: &'a Plan) -> Vec<&'a Jump> {
        plan.jumps
            .iter()
            .filter(|j| !self.undone.contains(&j.id))
            .collect()
    }

    /// The page reports it took jump `id` at `now`: the notice to show, unless it shows already
    /// or `id` is no jump the page was given.
    pub fn fired(&mut self, plan: &Plan, id: usize, now: u64) -> Option<&Notice> {
        let jump = self.table(plan).into_iter().find(|j| j.id == id)?.clone();
        if self.notice(now).is_some_and(|n| n.jump == id) {
            return None;
        }
        self.notice = Some(Notice {
            jump: jump.id,
            category: jump.category,
            from: jump.from,
            until: now + TOAST_MS,
        });
        self.notice.as_ref()
    }

    /// The notice still on screen at `now`.
    pub fn notice(&self, now: u64) -> Option<&Notice> {
        self.notice.as_ref().filter(|n| now < n.until)
    }

    /// Undo the last skip: the time to seek back to; its jump is not taken again.
    pub fn undo(&mut self, now: u64) -> Option<f64> {
        let notice = self.notice(now)?.clone();
        self.undone.insert(notice.jump);
        self.notice = None;
        Some(notice.from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(category: &str, from: f64, to: f64) -> Segment {
        Segment {
            uuid: format!("{category}-{from}"),
            category: category.into(),
            from,
            to,
            video_duration: 0.0,
        }
    }

    #[test]
    fn video_ids_from_every_youtube_address() {
        let id = "dQw4w9WgXcQ";
        for page in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=3",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ?autoplay=1",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?t=42",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=x",
        ] {
            assert_eq!(video_id(page).as_deref(), Some(id), "{page}");
        }
        for page in [
            "https://www.youtube.com/watch?v=dQw4w9WgXc",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQQ",
            "https://www.youtube.com/watch?v=dQw4w9WgX!Q",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/",
            "https://www.youtube.com/channel/dQw4w9WgXcQ",
            "https://vimeo.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ",
            "file:///watch?v=dQw4w9WgXcQ",
            "not a url",
        ] {
            assert_eq!(video_id(page), None, "{page}");
        }
    }

    #[test]
    fn only_four_hex_digits_of_the_hash_are_sent() {
        // `printf %s dQw4w9WgXcQ | sha256sum` starts with 5f6b0b4e.
        assert_eq!(hash_prefix("dQw4w9WgXcQ"), "5f6b");
        let url = request_url("https://sponsor.ajay.app/", "5f6b");
        assert_eq!(
            url,
            "https://sponsor.ajay.app/api/skipSegments/5f6b?categories=\
             %5B%22sponsor%22%2C%22selfpromo%22%2C%22interaction%22%2C%22intro%22%2C%22outro%22%2C\
             %22preview%22%2C%22music_offtopic%22%2C%22filler%22%5D\
             &actionTypes=%5B%22skip%22%5D&service=YouTube"
        );
        assert!(!url.contains("dQw4w9WgXcQ"));
    }

    #[test]
    fn answers_are_read_for_our_video_only() {
        let body = include_str!("../tests/fixtures/sponsor_skipsegments.json");
        let ours = parse(body, "dQw4w9WgXcQ");
        assert_eq!(
            ours.iter()
                .map(|s| (s.category.as_str(), s.from, s.to))
                .collect::<Vec<_>>(),
            vec![
                ("sponsor", 10.0, 20.5),
                ("selfpromo", 20.0, 30.0),
                ("intro", 0.0, 4.0),
            ]
        );
        assert_eq!(ours[0].uuid, "u1");
        assert_eq!(ours[0].video_duration, 212.0);
        let other = parse(body, "aaaaaaaaaaa");
        assert_eq!(other.len(), 1);
        assert!(parse(body, "bbbbbbbbbbb").is_empty());
        for junk in ["Not Found", "", "{}", "[1]", r#"[{"videoID":"x"}]"#] {
            assert!(parse(junk, "x").is_empty(), "{junk}");
        }
    }

    #[test]
    fn plans_merge_skips_and_mark_what_is_shown() {
        let segments = vec![
            seg("sponsor", 10.0, 20.0),
            seg("selfpromo", 19.0, 25.0),
            // Touching: merged too.
            seg("sponsor", 25.0, 30.0),
            seg("intro", 0.0, 5.0),
            seg("filler", 40.0, 41.0),
            seg("sponsor", 50.0, 50.4),
            seg("unknown", 60.0, 70.0),
            seg("sponsor", 90.0, 120.0),
        ];
        let acts = ActTable::from([("filler".into(), Act::Off.code())]);
        let p = plan(&segments, &acts, 100.0);
        assert_eq!(
            p.jumps,
            vec![
                Jump {
                    id: 0,
                    from: 10.0,
                    to: 30.0,
                    category: "sponsor",
                },
                Jump {
                    id: 1,
                    from: 90.0,
                    to: 100.0,
                    category: "sponsor",
                },
            ]
        );
        let marks: Vec<(f64, f64, u32)> =
            p.marks.iter().map(|m| (m.from, m.to, m.colour)).collect();
        assert_eq!(
            marks,
            vec![
                (0.0, 5.0, 0x00ffff),
                (10.0, 20.0, 0x00d400),
                (19.0, 25.0, 0xffff00),
                (25.0, 30.0, 0x00d400),
                (90.0, 100.0, 0x00d400),
            ]
        );
        // A live stream or a video without duration: nothing.
        assert_eq!(plan(&segments, &acts, f64::INFINITY), Plan::default());
        assert_eq!(plan(&segments, &acts, 0.0), Plan::default());
    }

    #[test]
    fn segments_of_another_cut_are_ignored() {
        let mut s = seg("sponsor", 1.0, 2.0);
        s.video_duration = 300.0;
        assert!(
            plan(std::slice::from_ref(&s), &ActTable::new(), 200.0)
                .jumps
                .is_empty()
        );
        assert_eq!(plan(&[s], &ActTable::new(), 301.5).jumps.len(), 1);
    }

    #[test]
    fn a_skip_notice_once_then_undo_with_a_fake_clock() {
        let p = plan(
            &[seg("sponsor", 1.0, 2.0), seg("selfpromo", 5.0, 6.0)],
            &ActTable::new(),
            10.0,
        );
        let mut skips = Skips::default();
        assert_eq!(skips.table(&p).len(), 2);
        let n = skips.fired(&p, 0, 1_000).cloned().unwrap();
        assert_eq!((n.jump, n.from, n.until), (0, 1.0, 1_000 + TOAST_MS));
        assert_eq!(n.text(), "Skipped sponsor");
        // Reported twice in one pass: one notice.
        assert!(skips.fired(&p, 0, 1_100).is_none());
        // An id the page was never given.
        assert!(skips.fired(&p, 7, 1_200).is_none());
        assert!(skips.notice(1_000 + TOAST_MS - 1).is_some());
        assert!(skips.notice(1_000 + TOAST_MS).is_none());
        assert_eq!(skips.undo(1_000 + TOAST_MS), None, "too late");
        let _ = skips.fired(&p, 0, 20_000);
        assert_eq!(skips.undo(20_500), Some(1.0));
        assert_eq!(
            skips.table(&p).iter().map(|j| j.id).collect::<Vec<_>>(),
            vec![1]
        );
        // The undone jump is not taken again, even if the page reports it.
        assert!(skips.fired(&p, 0, 21_000).is_none());
        let n = skips.fired(&p, 1, 22_000).unwrap();
        assert_eq!(n.text(), "Skipped self-promotion");
    }

    #[test]
    fn the_category_table_has_the_eight_ids_and_their_colours() {
        let ids: Vec<&str> = CATEGORIES.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                "sponsor",
                "selfpromo",
                "interaction",
                "intro",
                "outro",
                "preview",
                "music_offtopic",
                "filler"
            ]
        );
        assert_eq!(category("outro").map(|c| c.colour), Some(0x0202ed));
        assert_eq!(act_of(&ActTable::new(), "sponsor"), Act::Skip);
        assert_eq!(act_of(&ActTable::new(), "intro"), Act::Show);
        assert_eq!(
            act_of(&ActTable::from([("intro".into(), 0)]), "intro"),
            Act::Skip
        );
        assert_eq!(
            act_of(&ActTable::from([("intro".into(), 9)]), "intro"),
            Act::Show
        );
        assert_eq!(act_of(&ActTable::new(), "nope"), Act::Off);
        for a in [Act::Skip, Act::Show, Act::Off] {
            assert_eq!(Act::from_code(a.code()), Some(a));
        }
    }
}
