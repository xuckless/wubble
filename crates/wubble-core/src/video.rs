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

/// How long the controls stay after the pointer stops moving over the video (they were shown
/// for as long as the pointer was over it, and 2.5 s after it left).
pub const STILL_MS: u64 = 1_200;
/// How long they stay after the pointer leaves the video.
pub const LEAVE_MS: u64 = 250;
/// After the strip tucks into its tab, hovering the tab does not open it again for this long:
/// the pointer is still where the collapse button was.
pub const EXPAND_GUARD_MS: u64 = 600;

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

    /// The part of the rectangle inside `other`, if any.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        (right > x && bottom > y).then_some(Rect {
            x,
            y,
            w: right - x,
            h: bottom - y,
        })
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

    /// Whether it plays with its sound on: what picture-in-picture on a tab switch waits for
    /// (Chrome's automatic picture-in-picture asks for audible playback), not a muted or
    /// decorative clip.
    pub fn plays_aloud(&self) -> bool {
        !self.paused && !self.muted
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

/// Where the pointer is: off the video, on it, or on the controls (which hold them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Over {
    Away,
    Video,
    Strip,
}

/// What the controls show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ui {
    Hidden,
    Shown {
        over: Over,
        /// When the controls go (never while the pointer is on them or the menu is open).
        hide_at: Option<u64>,
        /// The speed menu is open.
        speeds: bool,
    },
    /// Tucked into a tab at the page's right edge; it goes like the strip does.
    Collapsed {
        over: Over,
        hide_at: Option<u64>,
        /// Hovering the tab opens the strip from this time on.
        armed_at: u64,
    },
    /// Closed by the user: hidden until the next navigation.
    Dismissed,
}

impl Ui {
    /// The two states look alike (a changed `hide_at` alone needs no redraw).
    fn looks_like(self, other: Ui) -> bool {
        match (self, other) {
            (Ui::Shown { speeds: a, .. }, Ui::Shown { speeds: b, .. }) => a == b,
            (Ui::Collapsed { .. }, Ui::Collapsed { .. }) => true,
            (Ui::Hidden, Ui::Hidden) | (Ui::Dismissed, Ui::Dismissed) => true,
            _ => false,
        }
    }
}

/// When the controls go, the pointer being `over` and the menu `speeds` open or not.
fn hide_after(over: Over, speeds: bool, now: u64) -> Option<u64> {
    match over {
        _ if speeds => None,
        Over::Strip => None,
        Over::Video => Some(now + STILL_MS),
        Over::Away => Some(now + LEAVE_MS),
    }
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

    /// The controls are tucked into their edge tab (and shown).
    pub fn collapsed(&self) -> bool {
        matches!(
            self,
            Overlay::Video {
                ui: Ui::Collapsed { .. },
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
    /// Tuck the strip into its tab (remembered).
    Collapse,
    /// Open the strip from its tab by a click.
    Expand,
    /// The pointer came onto the tab: opens it once the collapse's guard has passed.
    Peek,
    /// Hide the strip until the next navigation.
    Dismiss,
    /// Hide it on this site (remembered), and until the next navigation.
    HideSite,
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
    /// The pointer moved (or left); `collapsed`: the remembered state the strip opens in.
    Pointer {
        over: Over,
        now: u64,
        collapsed: bool,
    },
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
    /// Remember the strip folded (true) or open (false).
    RememberCollapsed(bool),
    /// Hide the controls on this tab's site from now on.
    HideSite,
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
            VideoInput::Pointer {
                over,
                now,
                collapsed,
            },
        ) => {
            let next = match ui {
                Ui::Hidden if over == Over::Away => Ui::Hidden,
                Ui::Hidden if collapsed => Ui::Collapsed {
                    over,
                    hide_at: hide_after(over, false, now),
                    armed_at: now,
                },
                Ui::Hidden => Ui::Shown {
                    over,
                    hide_at: hide_after(over, false, now),
                    speeds: false,
                },
                Ui::Shown { speeds, .. } => Ui::Shown {
                    over,
                    hide_at: hide_after(over, speeds, now),
                    speeds,
                },
                Ui::Collapsed { armed_at, .. } => Ui::Collapsed {
                    over,
                    hide_at: hide_after(over, false, now),
                    armed_at,
                },
                Ui::Dismissed => Ui::Dismissed,
            };
            let effects = if next.looks_like(ui) {
                vec![]
            } else {
                vec![Redraw]
            };
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
            let effects = if next.looks_like(ui) {
                vec![]
            } else {
                vec![Redraw]
            };
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
            // A control used while nothing shows (a key, the smoke) shows the strip for a while.
            let mut ui = match ui {
                Ui::Hidden => Ui::Shown {
                    over: Over::Away,
                    hide_at: hide_after(Over::Away, false, now),
                    speeds: false,
                },
                ui => ui,
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
                VideoCmd::Collapse => {
                    if let Ui::Shown { over, .. } = ui {
                        ui = Ui::Collapsed {
                            over,
                            hide_at: hide_after(over, false, now),
                            armed_at: now + EXPAND_GUARD_MS,
                        };
                        effects.push(VideoEffect::RememberCollapsed(true));
                    }
                }
                VideoCmd::Expand | VideoCmd::Peek => {
                    if let Ui::Collapsed { over, armed_at, .. } = ui
                        && (cmd == VideoCmd::Expand || now >= armed_at)
                    {
                        ui = Ui::Shown {
                            over,
                            hide_at: hide_after(over, false, now),
                            speeds: false,
                        };
                        effects.push(VideoEffect::RememberCollapsed(false));
                    }
                }
                VideoCmd::Dismiss => ui = Ui::Dismissed,
                VideoCmd::HideSite => {
                    ui = Ui::Dismissed;
                    effects.push(VideoEffect::HideSite);
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
            // The menu opening holds the strip, closing it starts the clock again.
            if let Ui::Shown {
                over,
                speeds,
                hide_at,
            } = &mut ui
            {
                *hide_at = hide_after(*over, *speeds, now);
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
        }
        | Ui::Collapsed {
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

    fn point(state: Overlay, over: Over, now: u64) -> (Overlay, Vec<VideoEffect>) {
        transition(
            state,
            VideoInput::Pointer {
                over,
                now,
                collapsed: false,
            },
        )
    }

    fn tick(state: Overlay, now: u64) -> Overlay {
        transition(state, VideoInput::Tick(now)).0
    }

    /// A video with the pointer over it at time 1000.
    fn hovered() -> Overlay {
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        point(s, Over::Video, 1_000).0
    }

    #[test]
    fn a_playing_video_shows_no_controls_without_the_pointer() {
        let (s, e) = snap(Overlay::default(), 1.0, 0);
        assert!(!s.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // The same snapshot again changes nothing; time passing neither.
        let (s, e) = snap(s, 1.0, 100);
        assert!(e.is_empty());
        assert!(!tick(s.clone(), 100_000).shown());
        // The pointer off a hidden overlay does nothing.
        let (s2, e) = point(s.clone(), Over::Away, 9);
        assert_eq!((s2, e), (s, vec![]));
    }

    #[test]
    fn hover_shows_and_stillness_hides_after_the_delay() {
        let s = hovered();
        assert!(s.shown());
        assert!(tick(s.clone(), 1_000 + STILL_MS - 1).shown());
        let (gone, e) = transition(s.clone(), VideoInput::Tick(1_000 + STILL_MS));
        assert!(!gone.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // Moving again over the video starts the wait over, and a move redraws nothing.
        let (s, e) = point(s, Over::Video, 2_000);
        assert!(e.is_empty());
        assert!(tick(s.clone(), 2_000 + STILL_MS - 1).shown());
        assert!(!tick(s, 2_000 + STILL_MS).shown());
    }

    #[test]
    fn leaving_the_video_hides_fast() {
        let (s, _) = point(hovered(), Over::Away, 1_500);
        assert!(tick(s.clone(), 1_500 + LEAVE_MS - 1).shown());
        assert!(!tick(s, 1_500 + LEAVE_MS).shown());
        const { assert!(LEAVE_MS < STILL_MS) };
    }

    #[test]
    fn the_pointer_on_the_strip_or_an_open_menu_holds_it() {
        let (s, _) = point(hovered(), Over::Strip, 1_100);
        assert!(tick(s.clone(), 1_000_000).shown());
        // The menu holds it even with the pointer gone.
        let (s, _) = cmd(s, VideoCmd::Speeds, 1_200);
        let (s, _) = point(s, Over::Away, 1_300);
        assert!(s.speeds_open() && tick(s.clone(), 1_000_000).shown());
        // Closing the menu starts the clock.
        let (s, _) = cmd(s, VideoCmd::Speeds, 2_000);
        assert!(!tick(s, 2_000 + LEAVE_MS).shown());
    }

    #[test]
    fn dismissed_stays_hidden_until_navigation() {
        let (s, e) = cmd(hovered(), VideoCmd::Dismiss, 1_100);
        assert!(!s.shown() && !s.collapsed());
        assert!(e.contains(&VideoEffect::Redraw) && !e.contains(&VideoEffect::HideSite));
        let (s, _) = point(s, Over::Video, 1_200);
        let (s, _) = snap(s, 1.0, 1_300);
        assert!(!s.shown() && !s.collapsed());
        let (s, _) = transition(s, VideoInput::PageChanged);
        let (s, _) = snap(s, 1.0, 2_000);
        let (s, _) = point(s, Over::Video, 2_100);
        assert!(s.shown());
    }

    #[test]
    fn hiding_on_the_site_dismisses_and_says_so() {
        let (s, e) = cmd(hovered(), VideoCmd::HideSite, 1_100);
        assert!(!s.shown());
        assert!(e.contains(&VideoEffect::HideSite));
    }

    #[test]
    fn collapsed_shows_only_the_tab_and_expands_by_click_or_a_late_hover() {
        let (s, e) = cmd(hovered(), VideoCmd::Collapse, 1_100);
        assert!(s.collapsed() && !s.shown());
        assert!(e.contains(&VideoEffect::RememberCollapsed(true)));
        // The pointer still where the button was: hover does not reopen at once ...
        let (still, e) = cmd(s.clone(), VideoCmd::Peek, 1_200);
        assert!(still.collapsed() && !e.contains(&VideoEffect::RememberCollapsed(false)));
        // ... a click does, and so does a hover after the guard.
        let (open, e) = cmd(s.clone(), VideoCmd::Expand, 1_200);
        assert!(open.shown() && e.contains(&VideoEffect::RememberCollapsed(false)));
        let (open, _) = cmd(s.clone(), VideoCmd::Peek, 1_100 + EXPAND_GUARD_MS);
        assert!(open.shown());
        // The tab goes like the strip: leaving hides it fast.
        let (s, _) = point(s, Over::Away, 3_000);
        assert!(s.collapsed());
        assert!(!tick(s, 3_000 + LEAVE_MS).collapsed());
    }

    #[test]
    fn a_remembered_collapse_opens_the_next_hover_as_the_tab() {
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        let (s, e) = transition(
            s,
            VideoInput::Pointer {
                over: Over::Video,
                now: 10,
                collapsed: true,
            },
        );
        assert!(s.collapsed() && !s.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
    }

    #[test]
    fn the_video_going_away_hides_everything() {
        let (s, _) = snap(Overlay::default(), 1.0, 0);
        let (s, _) = point(s, Over::Video, 0);
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
            VideoInput::Pointer {
                over: Over::Video,
                now: 2,
                collapsed: false,
            },
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
        let (s, _) = transition(s, VideoInput::Tick(100 + LEAVE_MS));
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
    fn a_video_is_clipped_to_its_pane() {
        let pane = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        // Wider than the left pane of a split, and taller than the viewport.
        let video = Rect {
            x: 100.0,
            y: 200.0,
            w: 500.0,
            h: 400.0,
        };
        assert_eq!(
            video.intersect(&pane),
            Some(Rect {
                x: 100.0,
                y: 200.0,
                w: 300.0,
                h: 100.0,
            })
        );
        let below = Rect { y: 300.0, ..video };
        assert_eq!(below.intersect(&pane), None, "scrolled out of view");
    }

    #[test]
    fn only_a_playing_video_with_sound_plays_aloud() {
        let v = VideoState::default();
        assert!(v.plays_aloud());
        assert!(
            !VideoState {
                muted: true,
                ..v.clone()
            }
            .plays_aloud()
        );
        assert!(!VideoState { paused: true, ..v }.plays_aloud());
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
