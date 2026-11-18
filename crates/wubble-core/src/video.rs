//! Video controls on every page: one [`Overlay`] state and one pure [`transition`]. The page
//! kit (`kit.js`) reports the page's main `<video>` as a [`VideoState`]; the overlay shows over
//! it while the pointer is there or just after a control was used, and its controls become
//! [`VideoEffect`]s the host carries out (page kit commands, the per-site rate, a frame
//! screenshot, picture-in-picture, the media downloader's seam).
//!
//! Times are milliseconds on the host's clock, passed in: nothing here reads a clock.

use std::collections::BTreeMap;

use crate::zoom;

/// The speeds offered, slowest first (0.25 to 4, as Chromium's own player menu tops out at 2).
pub const RATES: [f64; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];

/// How long the controls stay after the pointer leaves the video or a control was used.
pub const LINGER_MS: u64 = 2_500;

/// A site's remembered speed: host to rate in hundredths (`settings::VIDEO_RATES`); sites at
/// normal speed are absent.
pub type RateTable = BTreeMap<String, u32>;

/// A rectangle in CSS px of the page's viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// The rectangle in the window's logical px: the page is shown at `zoom_percent`, its pane
    /// starts at `origin` (logical px).
    pub fn on_window(&self, zoom_percent: u32, origin: (f64, f64)) -> Rect {
        let f = f64::from(zoom_percent) / 100.0;
        Rect {
            x: origin.0 + self.x * f,
            y: origin.1 + self.y * f,
            w: self.w * f,
            h: self.h * f,
        }
    }
}

/// The page's main video, as its last snapshot reported it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoState {
    pub rect: Rect,
    /// Seconds; `duration` is infinite for a live stream.
    pub time: f64,
    pub duration: f64,
    pub paused: bool,
    pub rate: f64,
    pub looped: bool,
    pub muted: bool,
    /// The document that answered (its `performance.timeOrigin`): an answer of the document
    /// a navigation replaces can still arrive after the navigation began.
    pub doc: f64,
}

impl VideoState {
    /// A snapshot's `v` object (`kit.js` `snapshot()`), its numbers checked: a page could hand
    /// anything back.
    pub fn from_json(v: &serde_json::Value) -> Option<VideoState> {
        let num = |k: &str| v.get(k).and_then(serde_json::Value::as_f64);
        let flag = |k: &str| {
            v.get(k)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        let finite = |x: f64| x.is_finite().then_some(x);
        let rect = Rect {
            x: finite(num("x")?)?,
            y: finite(num("y")?)?,
            w: finite(num("w")?)?.max(0.0),
            h: finite(num("h")?)?.max(0.0),
        };
        // `duration` may be NaN (no metadata yet) or infinite (live); both read as live.
        let duration = num("dur").filter(|d| *d >= 0.0).unwrap_or(f64::INFINITY);
        Some(VideoState {
            rect,
            time: finite(num("t").unwrap_or(0.0)).unwrap_or(0.0).max(0.0),
            duration,
            paused: flag("paused"),
            rate: num("rate")
                .filter(|r| r.is_finite() && *r > 0.0)
                .unwrap_or(1.0),
            looped: flag("loop"),
            muted: flag("muted"),
            doc: finite(num("doc").unwrap_or(0.0)).unwrap_or(0.0),
        })
    }

    /// How far along the video is, 0 to 1 (0 for a live stream).
    pub fn progress(&self) -> f64 {
        if self.duration.is_finite() && self.duration > 0.0 {
            (self.time / self.duration).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// What the controls show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ui {
    Hidden,
    Shown {
        /// The pointer is over the video (or the controls).
        hover: bool,
        /// When the controls go, if the pointer is not over them.
        hide_at: Option<u64>,
        /// The speed menu is open.
        speeds: bool,
    },
}

/// The video controls of one page.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    /// No video on the page yet. `rate_set`: the site's speed was given to this document.
    NoVideo { rate_set: bool },
    Video {
        video: VideoState,
        rate_set: bool,
        ui: Ui,
    },
}

impl Default for Overlay {
    fn default() -> Self {
        Overlay::NoVideo { rate_set: false }
    }
}

impl Overlay {
    pub fn video(&self) -> Option<&VideoState> {
        match self {
            Overlay::Video { video, .. } => Some(video),
            Overlay::NoVideo { .. } => None,
        }
    }

    pub fn shown(&self) -> bool {
        matches!(
            self,
            Overlay::Video {
                ui: Ui::Shown { .. },
                ..
            }
        )
    }

    pub fn speeds_open(&self) -> bool {
        matches!(
            self,
            Overlay::Video {
                ui: Ui::Shown { speeds: true, .. },
                ..
            }
        )
    }

    fn rate_set(&self) -> bool {
        match self {
            Overlay::NoVideo { rate_set } | Overlay::Video { rate_set, .. } => *rate_set,
        }
    }
}

/// A control of the overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VideoCmd {
    PlayPause,
    /// Opens or closes the speed menu.
    Speeds,
    Rate(f64),
    Loop,
    Mute,
    /// Seek to this fraction of the video.
    Seek(f64),
    Pip,
    Frame,
    Save,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VideoInput {
    /// The page's snapshot: its main video, if any. `site_rate`: the site's remembered speed
    /// (1 when none).
    Snapshot {
        video: Option<VideoState>,
        site_rate: f64,
        now: u64,
    },
    /// The pointer moved; whether it is over the video's rectangle or the controls.
    Pointer { over: bool, now: u64 },
    /// The host's clock moved on (each poll).
    Tick(u64),
    /// A control was used. `can_save`: the media downloader's seam is connected.
    Command {
        cmd: VideoCmd,
        now: u64,
        can_save: bool,
    },
    /// A new document: nothing of the old one's video is left.
    PageChanged,
}

/// A command for the page's video (`kit.js` `cmd`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageCmd {
    SetRate(f64),
    SetLoop(bool),
    SetMuted(bool),
    Toggle,
    Seek(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum VideoEffect {
    Page(PageCmd),
    /// Remember this speed for the site (1 forgets it).
    RememberRate(f64),
    /// Save the frame shown now.
    Frame,
    Pip,
    /// The media downloader's menu (the seam).
    Save,
    /// The controls changed: draw them.
    Redraw,
}

/// The nearest offered speed to `rate`, so a page's odd value still marks a menu row.
pub fn nearest_rate(rate: f64) -> f64 {
    RATES
        .into_iter()
        .min_by(|a, b| (a - rate).abs().total_cmp(&(b - rate).abs()))
        .unwrap_or(1.0)
}

/// `rate` within the offered range.
pub fn clamp_rate(rate: f64) -> f64 {
    if rate.is_finite() {
        rate.clamp(RATES[0], RATES[RATES.len() - 1])
    } else {
        1.0
    }
}

/// The site's remembered speed for the page at `url` (1 when none).
pub fn site_rate(table: &RateTable, url: &str) -> f64 {
    zoom::host_of(url)
        .and_then(|h| table.get(&h).copied())
        .map_or(1.0, |hundredths| clamp_rate(f64::from(hundredths) / 100.0))
}

/// The table after the user chose `rate` on `host`: normal speed removes the row.
pub fn with_rate(table: RateTable, host: &str, rate: f64) -> RateTable {
    let hundredths = (clamp_rate(rate) * 100.0).round() as u32;
    zoom::next_table(table, host, hundredths, 100)
}

fn shown(hover: bool, now: u64) -> Ui {
    Ui::Shown {
        hover,
        hide_at: (!hover).then_some(now + LINGER_MS),
        speeds: false,
    }
}

pub fn transition(state: Overlay, input: VideoInput) -> (Overlay, Vec<VideoEffect>) {
    use VideoEffect::Redraw;
    let rate_set = state.rate_set();
    match (state, input) {
        (_, VideoInput::PageChanged) => (Overlay::default(), vec![Redraw]),
        (state, VideoInput::Snapshot { video: None, .. }) => {
            let redraw = matches!(state, Overlay::Video { .. });
            (
                Overlay::NoVideo { rate_set },
                if redraw { vec![Redraw] } else { vec![] },
            )
        }
        (
            state,
            VideoInput::Snapshot {
                video: Some(video),
                site_rate,
                now,
            },
        ) => {
            let mut effects = vec![];
            // The site's speed, once per document. Another document answering (the one a
            // reload replaced, late) does not count for this one.
            let rate_set = rate_set && state.video().is_none_or(|v| v.doc == video.doc);
            let rate_set = if rate_set {
                true
            } else if (site_rate - 1.0).abs() > f64::EPSILON && video.duration > 0.0 {
                effects.push(VideoEffect::Page(PageCmd::SetRate(site_rate)));
                true
            } else {
                video.duration > 0.0
            };
            let ui = match &state {
                Overlay::Video { ui, .. } => *ui,
                Overlay::NoVideo { .. } => Ui::Hidden,
            };
            let ui = expire(ui, now);
            if state.video() != Some(&video) || ui != current_ui(&state) {
                effects.push(Redraw);
            }
            (
                Overlay::Video {
                    video,
                    rate_set,
                    ui,
                },
                effects,
            )
        }
        (
            Overlay::Video {
                video,
                rate_set,
                ui,
            },
            VideoInput::Pointer { over, now },
        ) => {
            let next = match (ui, over) {
                (Ui::Hidden, false) => Ui::Hidden,
                (Ui::Hidden, true) => shown(true, now),
                (Ui::Shown { speeds, .. }, true) => Ui::Shown {
                    hover: true,
                    hide_at: None,
                    speeds,
                },
                (
                    Ui::Shown {
                        hover: true,
                        speeds,
                        ..
                    },
                    false,
                ) => Ui::Shown {
                    hover: false,
                    hide_at: Some(now + LINGER_MS),
                    speeds,
                },
                (ui @ Ui::Shown { hover: false, .. }, false) => ui,
            };
            let effects = if next == ui { vec![] } else { vec![Redraw] };
            (
                Overlay::Video {
                    video,
                    rate_set,
                    ui: next,
                },
                effects,
            )
        }
        (
            Overlay::Video {
                video,
                rate_set,
                ui,
            },
            VideoInput::Tick(now),
        ) => {
            let next = expire(ui, now);
            let effects = if next == ui { vec![] } else { vec![Redraw] };
            (
                Overlay::Video {
                    video,
                    rate_set,
                    ui: next,
                },
                effects,
            )
        }
        (
            Overlay::Video {
                mut video,
                rate_set,
                ui,
            },
            VideoInput::Command { cmd, now, can_save },
        ) => {
            let hover = matches!(ui, Ui::Shown { hover: true, .. });
            let mut ui = match ui {
                Ui::Shown { speeds, .. } => Ui::Shown {
                    hover,
                    hide_at: (!hover).then_some(now + LINGER_MS),
                    speeds,
                },
                Ui::Hidden => shown(false, now),
            };
            let mut effects = vec![];
            match cmd {
                VideoCmd::PlayPause => {
                    video.paused = !video.paused;
                    effects.push(VideoEffect::Page(PageCmd::Toggle));
                }
                VideoCmd::Speeds => {
                    if let Ui::Shown { speeds, .. } = &mut ui {
                        *speeds = !*speeds;
                    }
                }
                VideoCmd::Rate(rate) => {
                    let rate = clamp_rate(rate);
                    video.rate = rate;
                    if let Ui::Shown { speeds, .. } = &mut ui {
                        *speeds = false;
                    }
                    effects.push(VideoEffect::Page(PageCmd::SetRate(rate)));
                    effects.push(VideoEffect::RememberRate(rate));
                }
                VideoCmd::Loop => {
                    video.looped = !video.looped;
                    effects.push(VideoEffect::Page(PageCmd::SetLoop(video.looped)));
                }
                VideoCmd::Mute => {
                    video.muted = !video.muted;
                    effects.push(VideoEffect::Page(PageCmd::SetMuted(video.muted)));
                }
                VideoCmd::Seek(fraction) if video.duration.is_finite() => {
                    let to = fraction.clamp(0.0, 1.0) * video.duration;
                    video.time = to;
                    effects.push(VideoEffect::Page(PageCmd::Seek(to)));
                }
                VideoCmd::Seek(_) => {}
                VideoCmd::Pip => effects.push(VideoEffect::Pip),
                VideoCmd::Frame => effects.push(VideoEffect::Frame),
                VideoCmd::Save if can_save => effects.push(VideoEffect::Save),
                VideoCmd::Save => {}
            }
            effects.push(Redraw);
            (
                Overlay::Video {
                    video,
                    rate_set,
                    ui,
                },
                effects,
            )
        }
        // Pointer, ticks and controls with no video: nothing to show.
        (state @ Overlay::NoVideo { .. }, _) => (state, vec![]),
    }
}

fn current_ui(state: &Overlay) -> Ui {
    match state {
        Overlay::Video { ui, .. } => *ui,
        Overlay::NoVideo { .. } => Ui::Hidden,
    }
}

/// The controls after `now`: gone once their time is up.
fn expire(ui: Ui, now: u64) -> Ui {
    match ui {
        Ui::Shown {
            hide_at: Some(at), ..
        } if now >= at => Ui::Hidden,
        ui => ui,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video() -> VideoState {
        VideoState {
            rect: Rect {
                x: 10.0,
                y: 20.0,
                w: 320.0,
                h: 180.0,
            },
            time: 1.0,
            duration: 10.0,
            paused: false,
            rate: 1.0,
            looped: false,
            muted: false,
            doc: 1.0,
        }
    }

    fn snap(state: Overlay, rate: f64, now: u64) -> (Overlay, Vec<VideoEffect>) {
        transition(
            state,
            VideoInput::Snapshot {
                video: Some(video()),
                site_rate: rate,
                now,
            },
        )
    }

    fn cmd(state: Overlay, cmd: VideoCmd, now: u64) -> (Overlay, Vec<VideoEffect>) {
        transition(
            state,
            VideoInput::Command {
                cmd,
                now,
                can_save: false,
            },
        )
    }

    #[test]
    fn a_video_shows_controls_only_under_the_pointer_and_they_linger() {
        let (s, e) = snap(Overlay::default(), 1.0, 0);
        assert!(!s.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // The same snapshot again changes nothing.
        let (s, e) = snap(s, 1.0, 100);
        assert!(e.is_empty());
        let (s, _) = transition(
            s,
            VideoInput::Pointer {
                over: true,
                now: 200,
            },
        );
        assert!(s.shown());
        // Under the pointer they never go.
        let (s, e) = transition(s, VideoInput::Tick(100_000));
        assert!(s.shown() && e.is_empty());
        let (s, _) = transition(
            s,
            VideoInput::Pointer {
                over: false,
                now: 1_000,
            },
        );
        let (s, _) = transition(s, VideoInput::Tick(1_000 + LINGER_MS - 1));
        assert!(s.shown());
        let (s, e) = transition(s, VideoInput::Tick(1_000 + LINGER_MS));
        assert!(!s.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // The pointer moving off a hidden overlay does nothing.
        let (s2, e) = transition(
            s.clone(),
            VideoInput::Pointer {
                over: false,
                now: 9,
            },
        );
        assert_eq!((s2, e), (s, vec![]));
    }

    #[test]
    fn the_video_going_away_hides_everything() {
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        let (s, _) = transition(s, VideoInput::Pointer { over: true, now: 0 });
        let (s, e) = transition(
            s,
            VideoInput::Snapshot {
                video: None,
                site_rate: 1.0,
                now: 1,
            },
        );
        assert_eq!(s, Overlay::NoVideo { rate_set: true });
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // Without a video, input is ignored.
        for input in [
            VideoInput::Pointer { over: true, now: 2 },
            VideoInput::Tick(3),
            VideoInput::Command {
                cmd: VideoCmd::Mute,
                now: 4,
                can_save: true,
            },
        ] {
            let (next, e) = transition(s.clone(), input);
            assert_eq!(next, s);
            assert!(e.is_empty());
        }
    }

    #[test]
    fn a_rate_is_set_remembered_and_closes_the_menu() {
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        let (s, _) = cmd(s, VideoCmd::Speeds, 10);
        assert!(s.speeds_open() && s.shown());
        let (s, e) = cmd(s, VideoCmd::Rate(2.0), 20);
        assert!(!s.speeds_open());
        assert_eq!(
            e,
            vec![
                VideoEffect::Page(PageCmd::SetRate(2.0)),
                VideoEffect::RememberRate(2.0),
                VideoEffect::Redraw,
            ]
        );
        assert_eq!(s.video().map(|v| v.rate), Some(2.0));
        // Out of range: clamped to the table's ends.
        let (_, e) = cmd(s, VideoCmd::Rate(9.0), 30);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(4.0)));
        // A control shows the controls for a while, as a hover would.
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        let (s, _) = cmd(s, VideoCmd::Loop, 100);
        let (s, _) = transition(s, VideoInput::Tick(100 + LINGER_MS));
        assert!(!s.shown());
    }

    #[test]
    fn a_new_document_gets_the_site_rate_again() {
        let (s, _) = snap(Overlay::default(), 1.5, 0);
        // A reload's late answer from the old document took the reset state's turn ...
        let (s, _) = transition(s, VideoInput::PageChanged);
        let (s, _) = snap(s, 1.5, 10);
        // ... so the new document, answering next, still gets the speed.
        let new = VideoState {
            doc: 2.0,
            ..video()
        };
        let (s, e) = transition(
            s,
            VideoInput::Snapshot {
                video: Some(new.clone()),
                site_rate: 1.5,
                now: 20,
            },
        );
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        let (_, e) = transition(
            s,
            VideoInput::Snapshot {
                video: Some(new),
                site_rate: 1.5,
                now: 30,
            },
        );
        assert!(!e.contains(&VideoEffect::Page(PageCmd::SetRate(1.5))));
    }

    #[test]
    fn the_site_rate_is_applied_once_per_document() {
        let (s, e) = snap(Overlay::default(), 1.5, 0);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        // The page set its own speed afterwards: it is not fought.
        let (s, e) = snap(s, 1.5, 10);
        assert!(!e.contains(&VideoEffect::Page(PageCmd::SetRate(1.5))));
        let (s, _) = transition(s, VideoInput::PageChanged);
        assert_eq!(s, Overlay::default());
        let (_, e) = snap(s, 1.5, 20);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        // A video without metadata yet (duration 0) waits for it.
        let early = VideoState {
            duration: 0.0,
            ..video()
        };
        let (s, e) = transition(
            Overlay::default(),
            VideoInput::Snapshot {
                video: Some(early),
                site_rate: 2.0,
                now: 0,
            },
        );
        assert!(!e.contains(&VideoEffect::Page(PageCmd::SetRate(2.0))));
        let (_, e) = snap(s, 2.0, 1);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(2.0)));
    }

    #[test]
    fn controls_become_page_commands() {
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        let (s, e) = cmd(s, VideoCmd::Mute, 1);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetMuted(true)));
        let (s, e) = cmd(s, VideoCmd::Loop, 2);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetLoop(true)));
        let (s, e) = cmd(s, VideoCmd::PlayPause, 3);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::Toggle));
        let (s, e) = cmd(s, VideoCmd::Seek(0.5), 4);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::Seek(5.0)));
        let (s, e) = cmd(s, VideoCmd::Frame, 5);
        assert_eq!(e[0], VideoEffect::Frame);
        let (s, e) = cmd(s, VideoCmd::Pip, 6);
        assert_eq!(e[0], VideoEffect::Pip);
        // Save only with the downloader's seam.
        let (s, e) = cmd(s, VideoCmd::Save, 7);
        assert_eq!(e, vec![VideoEffect::Redraw]);
        let (_, e) = transition(
            s,
            VideoInput::Command {
                cmd: VideoCmd::Save,
                now: 8,
                can_save: true,
            },
        );
        assert_eq!(e[0], VideoEffect::Save);
        // A live stream cannot be sought.
        let live = VideoState {
            duration: f64::INFINITY,
            ..video()
        };
        let s = Overlay::Video {
            video: live,
            rate_set: true,
            ui: Ui::Hidden,
        };
        let (_, e) = cmd(s, VideoCmd::Seek(0.5), 9);
        assert_eq!(e, vec![VideoEffect::Redraw]);
    }

    #[test]
    fn rects_reach_the_window_through_zoom_and_the_pane() {
        let r = Rect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 50.0,
        };
        // At 125 % in the second pane of a split, which starts 400 px in.
        assert_eq!(
            r.on_window(125, (400.0, 8.0)),
            Rect {
                x: 412.5,
                y: 33.0,
                w: 125.0,
                h: 62.5,
            }
        );
        assert!(r.contains((10.0, 20.0)) && !r.contains((110.0, 20.0)));
    }

    #[test]
    fn rates_span_a_quarter_to_four_and_are_remembered_per_site() {
        assert_eq!(RATES[0], 0.25);
        assert_eq!(RATES[RATES.len() - 1], 4.0);
        assert!(RATES.windows(2).all(|w| w[0] < w[1]));
        assert!(RATES.contains(&1.0));
        assert_eq!(nearest_rate(1.1), 1.0);
        assert_eq!(nearest_rate(2.6), 3.0);
        assert_eq!(clamp_rate(f64::NAN), 1.0);
        let t = with_rate(RateTable::new(), "a.test", 1.5);
        assert_eq!(t.get("a.test"), Some(&150));
        assert_eq!(site_rate(&t, "https://a.test/v"), 1.5);
        assert_eq!(site_rate(&t, "https://b.test/v"), 1.0);
        assert!(with_rate(t, "a.test", 1.0).is_empty());
        // A stored value out of range is read within it.
        let odd = RateTable::from([("a.test".to_string(), 1_000)]);
        assert_eq!(site_rate(&odd, "https://a.test/"), 4.0);
    }

    #[test]
    fn snapshots_are_read_defensively() {
        let v = serde_json::json!({
            "x": 1, "y": 2, "w": 3, "h": 4, "t": 5, "dur": 60,
            "paused": true, "rate": 2, "loop": true, "muted": false, "doc": 7.5
        });
        let s = VideoState::from_json(&v).unwrap();
        assert_eq!(s.doc, 7.5);
        assert_eq!(s.rect.w, 3.0);
        assert!(s.paused && s.looped && !s.muted);
        assert_eq!((s.time, s.duration, s.rate), (5.0, 60.0, 2.0));
        assert!((s.progress() - 5.0 / 60.0).abs() < 1e-9);
        let live = serde_json::json!({"x": 0, "y": 0, "w": -5, "h": 1, "dur": null, "rate": -1});
        let s = VideoState::from_json(&live).unwrap();
        assert_eq!((s.rect.w, s.rate, s.progress()), (0.0, 1.0, 0.0));
        assert!(s.duration.is_infinite());
        assert_eq!(VideoState::from_json(&serde_json::json!({"x": "a"})), None);
    }
}
