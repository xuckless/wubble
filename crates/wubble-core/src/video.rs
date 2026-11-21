//! Video controls on every page: one [`Overlay`] state and one pure [`transition`]. The page
//! kit (`kit.js`) reports the page's main `<video>` as a [`VideoState`]; the overlay shows over
//! it while the pointer is there or just after a control was used, and its controls become
//! [`VideoEffect`]s the host carries out (page kit commands, the per-site rate, a frame
//! screenshot, picture-in-picture, the media downloader's entry).
//!
//! The strip's life is [`Ui`]: hidden, shown, collapsed into its tab, or dismissed. Folded is
//! a state of its own ([`Fold`] while hidden, [`Ui::Collapsed`] while the tab shows), so a
//! collapse outlives the pointer leaving and coming back: only a click on the tab
//! ([`VideoCmd::Expand`]) opens the strip again, never a hover. The pointer on the strip, its
//! tab or its menu ([`Over::Strip`]) leaves no hide time to run out.
//!
//! Times are milliseconds on the host's clock, passed in: nothing here reads a clock.

use std::collections::BTreeMap;

use crate::download::{DownloadEntry, DownloadState};
use crate::media::Refusal;
use crate::media::detect::{Detected, Found};
use crate::zoom;

/// The speeds offered, slowest first (0.25 to 4, as Chromium's own player menu tops out at 2).
pub const RATES: [f64; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];

/// How long the controls stay after the pointer stops moving over the video (they were shown
/// for as long as the pointer was over it, and 2.5 s after it left).
pub const STILL_MS: u64 = 1_200;
/// How long they stay after the pointer leaves the video.
pub const LEAVE_MS: u64 = 250;

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
    /// The element has MediaKeys attached (EME: Widevine, PlayReady, FairPlay, ClearKey): its
    /// stream is protected, whatever the downloader saw of it.
    pub drm: bool,
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
            drm: flag("drm"),
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

/// What a hidden strip comes back as: the strip, or its tab (the user collapsed it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fold {
    #[default]
    Open,
    Tucked,
}

/// What the controls show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ui {
    /// Nothing shows; the next hover brings back the strip or, tucked, its tab.
    Hidden(Fold),
    Shown {
        over: Over,
        /// When the controls go (never while the pointer is on them or the menu is open).
        hide_at: Option<u64>,
        /// The speed menu is open.
        speeds: bool,
    },
    /// Tucked into a tab at the page's right edge; it goes like the strip does, and comes back
    /// tucked. Only a click on the tab opens the strip.
    Collapsed { over: Over, hide_at: Option<u64> },
    /// Closed by the user: hidden until the next navigation.
    Dismissed,
}

impl Ui {
    /// The two states look alike (a changed `hide_at` alone needs no redraw).
    fn looks_like(self, other: Ui) -> bool {
        match (self, other) {
            (Ui::Shown { speeds: a, .. }, Ui::Shown { speeds: b, .. }) => a == b,
            (Ui::Collapsed { .. }, Ui::Collapsed { .. }) => true,
            (Ui::Hidden(_), Ui::Hidden(_)) | (Ui::Dismissed, Ui::Dismissed) => true,
            _ => false,
        }
    }

    /// The strip shown for a while with the pointer `over`, as it was folded.
    fn showing(fold: Fold, over: Over, now: u64) -> Ui {
        match fold {
            Fold::Open => Ui::Shown {
                over,
                hide_at: hide_after(over, false, now),
                speeds: false,
            },
            Fold::Tucked => Ui::Collapsed {
                over,
                hide_at: hide_after(over, false, now),
            },
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
    /// Seek by this many seconds (the picture-in-picture window's back and forward buttons).
    SeekBy(f64),
    Pip,
    Frame,
    /// The save button, its menu anchored at this window point (logical px).
    Save {
        x: f32,
        y: f32,
    },
    /// Tuck the strip into its tab (remembered).
    Collapse,
    /// Open the strip from its tab, by a click (a hover never does).
    Expand,
    /// Hide the strip until the next navigation.
    Dismiss,
    /// Hide it on this site (remembered), and until the next navigation.
    HideSite,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VideoInput {
    /// The page's snapshot: its main video, if any. `site_rate`: the site's remembered speed
    /// (1 when none); `fold`: the remembered fold a new video's strip starts in.
    Snapshot {
        video: Option<VideoState>,
        site_rate: f64,
        fold: Fold,
        now: u64,
    },
    /// The pointer moved (or left).
    Pointer { over: Over, now: u64 },
    /// The host's clock moved on (each poll).
    Tick(u64),
    /// A control was used. `can_save`: the save button is offered ([`Save::offered`]).
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
    /// Pause, whatever it was doing (picture-in-picture closed).
    Pause,
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
    /// The media downloader's entry for the page (its menu, or the default quality at once),
    /// anchored at this window point.
    Save {
        x: f32,
        y: f32,
    },
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
                fold,
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
                Overlay::NoVideo { .. } => Ui::Hidden(fold),
            };
            let ui = expire(ui, now);
            if state.video() != Some(&video) || Some(ui) != current_ui(&state) {
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
            // A hover shows the strip as it was left: collapsed stays collapsed.
            let next = match ui {
                Ui::Hidden(fold) if over == Over::Away => Ui::Hidden(fold),
                Ui::Hidden(fold) => Ui::showing(fold, over, now),
                Ui::Shown { speeds, .. } => Ui::Shown {
                    over,
                    hide_at: hide_after(over, speeds, now),
                    speeds,
                },
                Ui::Collapsed { .. } => Ui::showing(Fold::Tucked, over, now),
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
            // A control used while nothing shows (a key, the smoke) shows the strip, or its
            // tab, for a while.
            let mut ui = match ui {
                Ui::Hidden(fold) => Ui::showing(fold, Over::Away, now),
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
                        ui = Ui::showing(Fold::Tucked, over, now);
                        effects.push(VideoEffect::RememberCollapsed(true));
                    }
                }
                VideoCmd::Expand => {
                    if let Ui::Collapsed { over, .. } = ui {
                        ui = Ui::showing(Fold::Open, over, now);
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
                VideoCmd::SeekBy(delta) if video.duration.is_finite() => {
                    let to = (video.time + delta).clamp(0.0, video.duration);
                    video.time = to;
                    effects.push(VideoEffect::Page(PageCmd::Seek(to)));
                }
                VideoCmd::SeekBy(_) => {}
                VideoCmd::Pip => effects.push(VideoEffect::Pip),
                VideoCmd::Frame => effects.push(VideoEffect::Frame),
                VideoCmd::Save { x, y } if can_save => effects.push(VideoEffect::Save { x, y }),
                VideoCmd::Save { .. } => {}
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

/// The strip's save button: what the media downloader offers for the page and how the latest
/// download started from it stands. Derived at each draw from the downloader's own state (the
/// tab's list of streams, the downloads list's entry); nothing of it is kept here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Save {
    /// No button: nothing the downloader can save, or protected media (DRM).
    Absent,
    Offered,
    /// A download from the page runs (or is paused); its fraction, to the
    /// percent, once the size is known.
    Saving(Option<f32>),
    Saved,
    Failed,
}

impl Save {
    /// The button for `video`, from the tab's streams (`media`: the downloader's list, empty
    /// while it is off or saw nothing) and the downloads entry of the latest download from the page. DRM is
    /// absolute: an element with MediaKeys, or a stream of the page the downloader refused as
    /// protected, takes the button away, whatever else the page offers.
    pub fn of(video: &VideoState, media: &[Detected], latest: Option<&DownloadEntry>) -> Save {
        let protected = video.drm
            || media
                .iter()
                .any(|i| i.found == Found::Refused(Refusal::Protected));
        let ready = media.iter().any(|i| matches!(i.found, Found::Ready(_)));
        if protected || !ready {
            return Save::Absent;
        }
        match latest.map(|e| (&e.state, e.fraction())) {
            Some((DownloadState::Complete, _)) => Save::Saved,
            // An interrupted download waits for a resume in the downloads list; the strip
            // does not pretend it is still saving.
            Some((DownloadState::Failed(_) | DownloadState::Interrupted(_), _)) => Save::Failed,
            Some((state, fraction)) if state.is_active() => {
                Save::Saving(fraction.map(|f| (f * 100.0).round() / 100.0))
            }
            _ => Save::Offered,
        }
    }

    pub fn offered(self) -> bool {
        self != Save::Absent
    }

    /// The button's accessible label (the downloader's word is "save": its pill's "Save media",
    /// its setting "Save videos to").
    pub fn label(self) -> String {
        match self {
            Save::Absent | Save::Offered => "Save video".into(),
            Save::Saving(None) => "Saving video".into(),
            Save::Saving(Some(f)) => format!("Saving video, {:.0}%", f * 100.0),
            Save::Saved => "Video saved".into(),
            Save::Failed => "Saving failed, save the video again".into(),
        }
    }
}

fn current_ui(state: &Overlay) -> Option<Ui> {
    match state {
        Overlay::Video { ui, .. } => Some(*ui),
        Overlay::NoVideo { .. } => None,
    }
}

/// The controls after `now`: gone once their time is up, folded as they were.
fn expire(ui: Ui, now: u64) -> Ui {
    match ui {
        Ui::Shown {
            hide_at: Some(at), ..
        } if now >= at => Ui::Hidden(Fold::Open),
        Ui::Collapsed {
            hide_at: Some(at), ..
        } if now >= at => Ui::Hidden(Fold::Tucked),
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
            drm: false,
        }
    }

    fn snap(state: Overlay, rate: f64, now: u64) -> (Overlay, Vec<VideoEffect>) {
        transition(
            state,
            VideoInput::Snapshot {
                video: Some(video()),
                site_rate: rate,
                fold: Fold::Open,
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
        transition(state, VideoInput::Pointer { over, now })
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
        // Snapshots and controls used meanwhile set no hide time either.
        let (s, _) = snap(s, 1.0, 2_000_000);
        let (s, _) = cmd(s, VideoCmd::PlayPause, 2_000_001);
        assert!(tick(s.clone(), 9_000_000).shown());
        // Leaving the strip for the video starts the still clock.
        let (left, _) = point(s.clone(), Over::Video, 9_000_000);
        assert!(!tick(left, 9_000_000 + STILL_MS).shown());
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

    /// The strip collapsed into its tab at 1100, the pointer on the collapse button.
    fn collapsed() -> Overlay {
        let (s, _) = point(hovered(), Over::Strip, 1_050);
        let (s, e) = cmd(s, VideoCmd::Collapse, 1_100);
        assert!(s.collapsed() && !s.shown());
        assert!(e.contains(&VideoEffect::RememberCollapsed(true)));
        s
    }

    #[test]
    fn collapsed_then_hovered_stays_collapsed() {
        let s = collapsed();
        // On the tab, on the video, anywhere: a hover never opens the strip.
        for (i, over) in [Over::Strip, Over::Video, Over::Strip, Over::Video]
            .into_iter()
            .enumerate()
        {
            let at = 1_200 + 1_000 * i as u64;
            let (next, e) = point(s.clone(), over, at);
            assert!(next.collapsed() && !next.shown(), "{over:?}");
            assert!(!e.contains(&VideoEffect::RememberCollapsed(false)));
        }
    }

    #[test]
    fn only_a_click_expands() {
        let (open, e) = cmd(collapsed(), VideoCmd::Expand, 1_200);
        assert!(open.shown() && !open.collapsed());
        assert!(e.contains(&VideoEffect::RememberCollapsed(false)));
        // An expand with nothing collapsed changes nothing.
        let (s, e) = cmd(hovered(), VideoCmd::Expand, 1_200);
        assert!(s.shown() && !e.contains(&VideoEffect::RememberCollapsed(false)));
    }

    #[test]
    fn the_collapse_outlives_the_pointer_leaving_and_coming_back() {
        // The tab goes like the strip: leaving hides it fast ...
        let (s, _) = point(collapsed(), Over::Away, 3_000);
        assert!(tick(s.clone(), 3_000 + LEAVE_MS - 1).collapsed());
        let s = tick(s, 3_000 + LEAVE_MS);
        assert!(!s.collapsed() && !s.shown());
        // ... and coming back shows the tab again, not the strip, as often as it happens.
        let (s, _) = point(s, Over::Video, 4_000);
        assert!(s.collapsed() && !s.shown());
        let s = tick(s, 4_000 + STILL_MS);
        let (s, _) = snap(s, 1.0, 6_000);
        let (s, _) = point(s, Over::Video, 7_000);
        assert!(s.collapsed());
        // A control used while it is hidden (a media key) shows the tab, not the strip.
        let s = tick(s, 7_000 + STILL_MS);
        let (s, _) = cmd(s, VideoCmd::Mute, 9_000);
        assert!(s.collapsed());
        // Until the page navigates: the next document's strip opens.
        let (s, _) = transition(s, VideoInput::PageChanged);
        let (s, _) = snap(s, 1.0, 10_000);
        let (s, _) = point(s, Over::Video, 10_100);
        assert!(s.shown());
    }

    #[test]
    fn the_pointer_on_the_tab_holds_it() {
        let (s, _) = point(collapsed(), Over::Strip, 1_200);
        assert!(tick(s.clone(), 1_000_000).collapsed());
        // Leaving it starts the clock.
        let (s, _) = point(s, Over::Video, 2_000);
        assert!(tick(s.clone(), 2_000 + STILL_MS - 1).collapsed());
        assert!(!tick(s, 2_000 + STILL_MS).collapsed());
    }

    #[test]
    fn a_remembered_collapse_opens_the_next_hover_as_the_tab() {
        let (s, _) = transition(
            Overlay::default(),
            VideoInput::Snapshot {
                video: Some(video()),
                site_rate: 1.0,
                fold: Fold::Tucked,
                now: 0,
            },
        );
        let (s, e) = point(s, Over::Video, 10);
        assert!(s.collapsed() && !s.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // The remembered fold counts only for a new video: a snapshot does not refold it.
        let (open, _) = cmd(s, VideoCmd::Expand, 20);
        let (open, _) = transition(
            open,
            VideoInput::Snapshot {
                video: Some(video()),
                site_rate: 1.0,
                fold: Fold::Tucked,
                now: 30,
            },
        );
        assert!(open.shown());
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
                fold: Fold::Open,
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
                fold: Fold::Open,
                now: 20,
            },
        );
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        let (_, e) = transition(
            s,
            VideoInput::Snapshot {
                video: Some(new),
                site_rate: 1.5,
                fold: Fold::Open,
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
                fold: Fold::Open,
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
        // By seconds from there, kept inside the video.
        let (s, e) = cmd(s, VideoCmd::SeekBy(-10.0), 4);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::Seek(0.0)));
        let (s, e) = cmd(s, VideoCmd::SeekBy(3.0), 4);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::Seek(3.0)));
        let (s, e) = cmd(s, VideoCmd::SeekBy(30.0), 4);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::Seek(10.0)));
        let (s, e) = cmd(s, VideoCmd::Frame, 5);
        assert_eq!(e[0], VideoEffect::Frame);
        let (s, e) = cmd(s, VideoCmd::Pip, 6);
        assert_eq!(e[0], VideoEffect::Pip);
        // Save only while the button is offered; it neither pauses, seeks nor folds the strip.
        let save = VideoCmd::Save { x: 3.0, y: 4.0 };
        let (s, e) = cmd(s, save, 7);
        assert_eq!(e, vec![VideoEffect::Redraw]);
        let before = s.clone();
        let (s, e) = transition(
            s,
            VideoInput::Command {
                cmd: save,
                now: 8,
                can_save: true,
            },
        );
        assert_eq!(
            e,
            vec![VideoEffect::Save { x: 3.0, y: 4.0 }, VideoEffect::Redraw]
        );
        assert_eq!(s.video(), before.video());
        assert!(s.shown() && !s.collapsed());
        // A live stream cannot be sought.
        let live = VideoState {
            duration: f64::INFINITY,
            ..video()
        };
        let s = Overlay::Video {
            video: live,
            rate_set: true,
            ui: Ui::Hidden(Fold::Open),
        };
        let (_, e) = cmd(s, VideoCmd::Seek(0.5), 9);
        assert_eq!(e, vec![VideoEffect::Redraw]);
    }

    fn media(found: &[Found]) -> Vec<Detected> {
        found
            .iter()
            .enumerate()
            .map(|(i, f)| Detected {
                id: i as u32,
                url: format!("https://a.test/{i}.mp4"),
                kind: crate::media::detect::Kind::File,
                title: None,
                found: f.clone(),
            })
            .collect()
    }

    fn ready() -> Found {
        use crate::media::{Container, Track, Variant};
        Found::Ready(vec![Variant::file(
            "https://a.test/v.mp4",
            Track::Muxed,
            Container::Mp4,
            Some(1_000),
            false,
        )])
    }

    fn entry(state: DownloadState, received: u64, total: Option<u64>) -> DownloadEntry {
        DownloadEntry {
            id: 1,
            url: "https://a.test/v.mp4".into(),
            name: "v.mp4".into(),
            path: "/d/v.mp4".into(),
            received,
            total,
            state,
            started_ms: 0,
        }
    }

    #[test]
    fn the_save_button_is_offered_only_for_what_the_downloader_can_save() {
        let v = video();
        // Nothing seen (or the downloader off), only streams still resolving, live or broken.
        assert_eq!(Save::of(&v, &[], None), Save::Absent);
        let unsaveable = media(&[
            Found::Resolving,
            Found::Refused(Refusal::Live),
            Found::Refused(Refusal::Failed("x".into())),
        ]);
        assert_eq!(Save::of(&v, &unsaveable, None), Save::Absent);
        assert!(!Save::Absent.offered());
        // A stream ready to save.
        let page = media(&[Found::Resolving, ready()]);
        assert_eq!(Save::of(&v, &page, None), Save::Offered);
        assert!(Save::Offered.offered());
    }

    #[test]
    fn protected_media_never_gets_the_save_button() {
        let page = media(&[ready()]);
        // The element has MediaKeys: protected, whatever the downloader saw.
        let drm = VideoState {
            drm: true,
            ..video()
        };
        assert_eq!(Save::of(&drm, &page, None), Save::Absent);
        let running = entry(DownloadState::Running, 1, Some(2));
        assert_eq!(Save::of(&drm, &page, Some(&running)), Save::Absent);
        // A stream of the page the downloader refused as protected, beside a saveable one.
        let mixed = media(&[ready(), Found::Refused(Refusal::Protected)]);
        assert_eq!(Save::of(&video(), &mixed, None), Save::Absent);
        // The snapshot reads the flag.
        let v = serde_json::json!({"x": 0, "y": 0, "w": 1, "h": 1, "drm": true});
        assert!(VideoState::from_json(&v).unwrap().drm);
    }

    #[test]
    fn the_save_button_follows_the_latest_download() {
        let (v, page) = (video(), media(&[ready()]));
        let at = |e: DownloadEntry| Save::of(&v, &page, Some(&e));
        assert_eq!(
            at(entry(DownloadState::Running, 1, Some(3))),
            Save::Saving(Some(0.33))
        );
        assert_eq!(
            at(entry(DownloadState::Paused, 1, Some(2))),
            Save::Saving(Some(0.5))
        );
        assert_eq!(
            at(entry(DownloadState::Interrupted("net".into()), 0, Some(2))),
            Save::Failed
        );
        // Size unknown: saving, no fraction.
        assert_eq!(
            at(entry(DownloadState::Running, 7, None)),
            Save::Saving(None)
        );
        assert_eq!(at(entry(DownloadState::Complete, 2, Some(2))), Save::Saved);
        assert_eq!(
            at(entry(DownloadState::Failed("x".into()), 0, None)),
            Save::Failed
        );
        // Cancelled: offered again.
        assert_eq!(at(entry(DownloadState::Cancelled, 0, None)), Save::Offered);
        assert_eq!(Save::Offered.label(), "Save video");
        assert_eq!(Save::Saving(Some(0.33)).label(), "Saving video, 33%");
        assert_eq!(Save::Saving(None).label(), "Saving video");
        assert_eq!(Save::Saved.label(), "Video saved");
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
