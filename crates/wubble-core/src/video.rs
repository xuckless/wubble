//! Video controls on every page: one [`Overlay`] state and one pure [`transition`]. The page
//! kit (`kit.js`) reports the page's main `<video>` as a [`VideoState`]; the overlay shows over
//! it while the pointer is there or just after a control was used, and its controls become
//! [`VideoEffect`]s the host carries out (page kit commands, the per-site rate, a frame
//! screenshot, picture-in-picture, the media downloader's entry).
//!
//! What the controls show is [`Ui`]: the strip, its tab, or dismissed. Whether and how long
//! they show is the store's video slot (`overlay::Kind::VideoStrip`), fed the cue of [`hold`]
//! for where the pointer is ([`VideoEffect::Hold`]). Collapsed is a state of its own, so a
//! collapse outlives the pointer leaving and coming back: only a click on the tab
//! ([`VideoCmd::Expand`]) opens the strip again, never a hover. The pointer on the strip, its
//! tab or its menu ([`Over::Strip`]) holds the slot.
//!
//! Nothing here reads a clock or keeps a time.

use std::collections::BTreeMap;

use crate::download::{DownloadEntry, DownloadState};
use crate::media::Refusal;
use crate::media::detect::{Detected, Found};
use crate::overlay::Cue;
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
    /// Not a video: the area of an embedded frame the page kit cannot reach into (a
    /// cross-origin player), where only the save control is offered, over what the downloader
    /// saw of the page.
    pub frame: bool,
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
            frame: flag("frame"),
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
    // the order of [`HOLD`]'s rows
    Away,
    Video,
    Strip,
}

/// What a new video's strip starts as: the strip, or its tab (the user collapsed it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fold {
    #[default]
    Open,
    Tucked,
}

/// What the controls show when they show; whether and how long they show is the store's video
/// slot (`overlay::Kind::VideoStrip`), which this machine feeds with [`VideoEffect::Hold`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ui {
    /// The strip; `speeds`: its speed menu is open.
    Strip { speeds: bool },
    /// Tucked into a tab at the page's right edge; only a click on it opens the strip.
    Tab,
    /// Closed by the user: hidden until the next navigation.
    Dismissed,
}

impl Ui {
    fn of(fold: Fold) -> Ui {
        match fold {
            Fold::Open => Ui::Strip { speeds: false },
            Fold::Tucked => Ui::Tab,
        }
    }
}

/// How the strip's slot holds, by where the pointer is (`[over]`) and the menu open or not
/// (`[speeds]`): on the strip, its tab or with the menu open it is held; after a move over the
/// video it goes [`STILL_MS`] later; the pointer gone, the slot's own [`LEAVE_MS`].
const HOLD: [[Cue; 2]; 3] = [
    [Cue::Leave, Cue::Show],
    [Cue::ShowFor(STILL_MS), Cue::Show],
    [Cue::Show, Cue::Show],
];

/// The video slot's cue for the pointer `over`, the menu `speeds` open or not.
pub fn hold(over: Over, speeds: bool) -> Cue {
    HOLD[over as usize][usize::from(speeds)]
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

    /// While the slot shows: the strip.
    pub fn strip(&self) -> bool {
        matches!(
            self,
            Overlay::Video {
                ui: Ui::Strip { .. },
                ..
            }
        )
    }

    /// While the slot shows: the controls tucked into their edge tab.
    pub fn collapsed(&self) -> bool {
        matches!(self, Overlay::Video { ui: Ui::Tab, .. })
    }

    pub fn speeds_open(&self) -> bool {
        matches!(
            self,
            Overlay::Video {
                ui: Ui::Strip { speeds: true },
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
    },
    /// The pointer moved (or left).
    Pointer { over: Over },
    /// A control was used, the pointer `over`. `can_save`: the save button is offered
    /// ([`Save::offered`]).
    Command {
        cmd: VideoCmd,
        over: Over,
        can_save: bool,
    },
    /// The store's slot hid the controls: the menu closes with them.
    Hidden,
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
    /// The store's video slot for this tab: shown at once, for a while, left or gone.
    Hold(Cue),
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
    use VideoEffect::{Hold, Redraw};
    let rate_set = state.rate_set();
    match (state, input) {
        (_, VideoInput::PageChanged) => (Overlay::default(), vec![Redraw]),
        (state, VideoInput::Snapshot { video: None, .. }) => {
            let fx = match state {
                Overlay::Video { .. } => vec![Hold(Cue::Dismiss), Redraw],
                Overlay::NoVideo { .. } => vec![],
            };
            (Overlay::NoVideo { rate_set }, fx)
        }
        (
            state,
            VideoInput::Snapshot {
                video: Some(video),
                site_rate,
                fold,
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
            // the remembered fold counts only for a new video
            let ui = match &state {
                Overlay::Video { ui, .. } => *ui,
                Overlay::NoVideo { .. } => Ui::of(fold),
            };
            if state.video() != Some(&video) {
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
        (state @ Overlay::Video { ui, .. }, VideoInput::Pointer { over }) => {
            // a hover shows the controls as they were left: collapsed stays collapsed
            let fx = match ui {
                Ui::Dismissed => vec![],
                _ => vec![Hold(hold(over, state.speeds_open()))],
            };
            (state, fx)
        }
        (
            Overlay::Video {
                video,
                rate_set,
                ui: Ui::Strip { .. },
            },
            VideoInput::Hidden,
        ) => (
            Overlay::Video {
                video,
                rate_set,
                ui: Ui::Strip { speeds: false },
            },
            vec![Redraw],
        ),
        (state @ Overlay::Video { .. }, VideoInput::Hidden) => (state, vec![Redraw]),
        (
            Overlay::Video {
                mut video,
                rate_set,
                mut ui,
            },
            VideoInput::Command {
                cmd,
                over,
                can_save,
            },
        ) => {
            let mut effects = vec![];
            match cmd {
                VideoCmd::PlayPause => {
                    video.paused = !video.paused;
                    effects.push(VideoEffect::Page(PageCmd::Toggle));
                }
                VideoCmd::Speeds => {
                    if let Ui::Strip { speeds } = &mut ui {
                        *speeds = !*speeds;
                    }
                }
                VideoCmd::Collapse => {
                    if let Ui::Strip { .. } = ui {
                        ui = Ui::Tab;
                        effects.push(VideoEffect::RememberCollapsed(true));
                    }
                }
                VideoCmd::Expand => {
                    if ui == Ui::Tab {
                        ui = Ui::Strip { speeds: false };
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
                    if let Ui::Strip { speeds } = &mut ui {
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
            // A control used shows the controls for a while (a key, the smoke, with nothing
            // shown); the menu opening holds them, closing it starts the clock again; a
            // dismissal ends them.
            let speeds = matches!(ui, Ui::Strip { speeds: true });
            effects.push(Hold(match (ui, hold(over, speeds)) {
                (Ui::Dismissed, _) => Cue::Dismiss,
                (_, Cue::Leave) => Cue::ShowFor(LEAVE_MS),
                (_, cue) => cue,
            }));
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
        // Pointer, controls and hides with no video: nothing to show.
        (state @ Overlay::NoVideo { .. }, _) => (state, vec![]),
    }
}

/// The strip's save control: where the downloader stands for the page and how the latest
/// download started from it stands. Derived at each draw from the downloader's own state (the
/// tab's list of streams, the downloads list's entry); nothing of it is kept here.
///
/// The rule: whenever the strip shows over a video the control is there (the downloader on), and
/// pressing it either saves or says why it cannot (protected, live, YouTube's lookup off, nothing
/// found): a missing control reads as "this browser never saw my video".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Save {
    /// No control: the downloader is off (`media.enabled`).
    Absent,
    Offered,
    /// A download from the page runs (or is paused); its fraction, to the
    /// percent, once the size is known.
    Saving(Option<f32>),
    Saved,
    Failed,
}

impl Save {
    /// The control for the strip over `video`, with the downloads entry of the latest download
    /// from the page. `enabled`: the downloader is on. Protected media (an element with
    /// MediaKeys, or a stream the downloader refused as protected) keeps the control, which
    /// then explains: the menu never offers such a video, whatever else the page lists.
    pub fn of(
        video: &VideoState,
        media: &[Detected],
        latest: Option<&DownloadEntry>,
        enabled: bool,
    ) -> Save {
        let protected = video.drm
            || media
                .iter()
                .any(|i| i.found == Found::Refused(Refusal::Protected));
        if !enabled {
            return Save::Absent;
        }
        if protected {
            return Save::Offered;
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
            frame: false,
        }
    }

    use crate::overlay::{self, Action, Effect, Env, Key, Kind, Layer, Surface};
    use crate::tab::TabId;
    use crate::window::{WindowId, WindowKind};

    /// Nothing of the overlays: the video slot needs no other owner.
    struct NoEnv;
    impl Env for NoEnv {
        fn shown(&self, _: WindowId) -> Option<TabId> {
            None
        }
        fn showing(&self, _: TabId) -> Option<WindowId> {
            None
        }
        fn kind(&self, _: WindowId) -> WindowKind {
            WindowKind::Main
        }
        fn page_has_keys(&self, _: WindowId) -> bool {
            false
        }
        fn others(&self, _: WindowId) -> Vec<Layer> {
            vec![]
        }
        fn machine(&self, _: Kind) -> bool {
            false
        }
    }

    const TAB: TabId = TabId(1);
    const SLOT: Surface = Surface {
        w: WindowId(1),
        kind: Kind::VideoStrip,
    };

    /// The machine with the store's video slot, as the host runs them: each `Hold` a cue for
    /// the slot, each hide of the slot `VideoInput::Hidden`; `over`, the host's pointer.
    #[derive(Clone, Debug, PartialEq)]
    struct Run {
        s: Overlay,
        ui: overlay::Ui,
        over: Over,
    }

    impl Run {
        fn up(&self) -> bool {
            self.ui.shown(SLOT).is_some()
        }
        fn shown(&self) -> bool {
            self.up() && self.s.strip()
        }
        fn collapsed(&self) -> bool {
            self.up() && self.s.collapsed()
        }
        fn speeds_open(&self) -> bool {
            self.up() && self.s.speeds_open()
        }
    }

    fn store(mut r: Run, action: Action) -> Run {
        let (ui, fx) = overlay::reduce(r.ui, &NoEnv, action);
        r.ui = ui;
        if fx.contains(&Effect::Unshow(SLOT)) {
            r.s = transition(r.s, VideoInput::Hidden).0;
        }
        r
    }

    fn step(mut r: Run, input: VideoInput, now: u64) -> (Run, Vec<VideoEffect>) {
        let (s, fx) = transition(r.s, input);
        r.s = s;
        for f in &fx {
            if let VideoEffect::Hold(cue) = *f {
                let (at, key) = (SLOT, Key::Tab(TAB));
                r = store(r, Action::Hover { at, key, cue, now });
            }
        }
        (r, fx)
    }

    fn start() -> Run {
        Run {
            s: Overlay::default(),
            ui: overlay::Ui::default(),
            over: Over::Away,
        }
    }

    fn snap(r: Run, rate: f64, now: u64) -> (Run, Vec<VideoEffect>) {
        snap_with(r, Some(video()), rate, Fold::Open, now)
    }

    fn snap_with(
        r: Run,
        video: Option<VideoState>,
        site_rate: f64,
        fold: Fold,
        now: u64,
    ) -> (Run, Vec<VideoEffect>) {
        let input = VideoInput::Snapshot {
            video,
            site_rate,
            fold,
        };
        step(r, input, now)
    }

    fn cmd(r: Run, cmd: VideoCmd, now: u64) -> (Run, Vec<VideoEffect>) {
        cmd_save(r, cmd, false, now)
    }

    fn cmd_save(r: Run, cmd: VideoCmd, can_save: bool, now: u64) -> (Run, Vec<VideoEffect>) {
        let over = r.over;
        step(
            r,
            VideoInput::Command {
                cmd,
                over,
                can_save,
            },
            now,
        )
    }

    fn point(mut r: Run, over: Over, now: u64) -> (Run, Vec<VideoEffect>) {
        r.over = over;
        step(r, VideoInput::Pointer { over }, now)
    }

    fn tick(r: Run, now: u64) -> Run {
        store(r, Action::Tick { now })
    }

    fn page_changed(r: Run) -> Run {
        let r = step(r, VideoInput::PageChanged, 0).0;
        store(
            r,
            Action::Navigated {
                w: SLOT.w,
                tab: TAB,
            },
        )
    }

    /// A video with the pointer over it at time 1000.
    fn hovered() -> Run {
        let (s, _) = snap(start(), 1.0, 0);
        point(s, Over::Video, 1_000).0
    }

    #[test]
    fn the_slot_holds_by_where_the_pointer_is_and_the_menu() {
        #[rustfmt::skip]
        let rows = [
            (Over::Away, false, Cue::Leave), (Over::Video, false, Cue::ShowFor(STILL_MS)), (Over::Strip, false, Cue::Show),
            (Over::Away, true, Cue::Show), (Over::Video, true, Cue::Show), (Over::Strip, true, Cue::Show),
        ];
        for (over, speeds, want) in rows {
            assert_eq!(hold(over, speeds), want, "{over:?} {speeds}");
        }
    }

    #[test]
    fn a_playing_video_shows_no_controls_without_the_pointer() {
        let (s, e) = snap(start(), 1.0, 0);
        assert!(!s.shown());
        assert_eq!(e, vec![VideoEffect::Redraw]);
        // The same snapshot again changes nothing; time passing neither.
        let (s, e) = snap(s, 1.0, 100);
        assert!(e.is_empty());
        assert!(!tick(s.clone(), 100_000).shown());
        // The pointer off a hidden overlay does nothing.
        let (s2, _) = point(s.clone(), Over::Away, 9);
        assert_eq!((s2.s, s2.ui), (s.s, s.ui));
    }

    #[test]
    fn hover_shows_and_stillness_hides_after_the_delay() {
        let s = hovered();
        assert!(s.shown());
        assert!(tick(s.clone(), 1_000 + STILL_MS - 1).shown());
        assert!(!tick(s.clone(), 1_000 + STILL_MS).shown());
        // Moving again over the video starts the wait over, re-arming nothing and drawing
        // nothing: the slot keeps its timer (the stale fire re-arms) and the machine its state.
        let (ui, fx) = overlay::reduce(
            s.ui.clone(),
            &NoEnv,
            Action::Hover {
                at: SLOT,
                key: Key::Tab(TAB),
                cue: hold(Over::Video, false),
                now: 2_000,
            },
        );
        assert_eq!(fx, vec![], "a later move: no effect, no Arm");
        let (s, e) = point(s, Over::Video, 2_000);
        assert_eq!(
            (&s.ui, &e[..]),
            (&ui, &[VideoEffect::Hold(Cue::ShowFor(STILL_MS))][..])
        );
        let s = tick(s, 1_000 + STILL_MS);
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
    fn the_menu_closes_when_the_controls_hide() {
        let (s, _) = point(hovered(), Over::Strip, 1_100);
        let (s, _) = cmd(s, VideoCmd::Speeds, 1_200);
        let (s, _) = cmd(s, VideoCmd::Speeds, 1_300);
        let (s, _) = cmd(s, VideoCmd::Speeds, 1_400);
        // off the strip with the menu open: held; the menu closed by a pick, the clock runs
        let (s, _) = point(s, Over::Away, 1_500);
        assert!(s.speeds_open());
        let (s, _) = cmd(s, VideoCmd::Rate(2.0), 1_600);
        let s = tick(s, 1_600 + LEAVE_MS);
        assert!(!s.up() && !s.s.speeds_open());
        // shown again, the menu is shut
        let (s, _) = point(s, Over::Video, 3_000);
        assert!(s.shown() && !s.speeds_open());
    }

    #[test]
    fn dismissed_stays_hidden_until_navigation() {
        let (s, e) = cmd(hovered(), VideoCmd::Dismiss, 1_100);
        assert!(!s.shown() && !s.collapsed() && !s.up());
        assert!(e.contains(&VideoEffect::Redraw) && !e.contains(&VideoEffect::HideSite));
        let (s, _) = point(s, Over::Video, 1_200);
        let (s, _) = snap(s, 1.0, 1_300);
        assert!(!s.shown() && !s.collapsed());
        let s = page_changed(s);
        let (s, _) = snap(s, 1.0, 2_000);
        let (s, _) = point(s, Over::Video, 2_100);
        assert!(s.shown());
    }

    #[test]
    fn a_navigation_hides_the_strip() {
        let s = page_changed(hovered());
        assert!(!s.up() && s.ui.idle());
    }

    #[test]
    fn hiding_on_the_site_dismisses_and_says_so() {
        let (s, e) = cmd(hovered(), VideoCmd::HideSite, 1_100);
        assert!(!s.shown());
        assert!(e.contains(&VideoEffect::HideSite));
    }

    /// The strip collapsed into its tab at 1100, the pointer on the collapse button.
    fn collapsed() -> Run {
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
        let (s, _) = point(s, Over::Away, 8_500);
        let (s, _) = cmd(s, VideoCmd::Mute, 9_000);
        assert!(s.collapsed());
        assert!(!tick(s.clone(), 9_000 + LEAVE_MS).up(), "for a while");
        // Until the page navigates: the next document's strip opens.
        let s = page_changed(s);
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
        let (s, _) = snap_with(start(), Some(video()), 1.0, Fold::Tucked, 0);
        let (s, e) = point(s, Over::Video, 10);
        assert!(s.collapsed() && !s.shown());
        assert_eq!(e, vec![VideoEffect::Hold(Cue::ShowFor(STILL_MS))]);
        // The remembered fold counts only for a new video: a snapshot does not refold it.
        let (open, _) = cmd(s, VideoCmd::Expand, 20);
        let (open, _) = snap_with(open, Some(video()), 1.0, Fold::Tucked, 30);
        assert!(open.shown());
    }

    #[test]
    fn the_video_going_away_hides_everything() {
        let (s, _) = snap(start(), 1.0, 0);
        let (s, _) = point(s, Over::Video, 0);
        let (s, e) = snap_with(s, None, 1.0, Fold::Open, 1);
        assert_eq!(s.s, Overlay::NoVideo { rate_set: true });
        assert_eq!(
            e,
            vec![VideoEffect::Hold(Cue::Dismiss), VideoEffect::Redraw]
        );
        assert!(!s.up());
        // Without a video, input is ignored.
        for input in [
            VideoInput::Pointer { over: Over::Video },
            VideoInput::Hidden,
            VideoInput::Command {
                cmd: VideoCmd::Mute,
                over: Over::Strip,
                can_save: true,
            },
        ] {
            let (next, e) = transition(s.s.clone(), input);
            assert_eq!(next, s.s);
            assert!(e.is_empty());
        }
    }

    #[test]
    fn a_rate_is_set_remembered_and_closes_the_menu() {
        let (s, _) = snap(start(), 1.0, 0);
        let (s, _) = cmd(s, VideoCmd::Speeds, 10);
        assert!(s.speeds_open() && s.shown());
        let (s, e) = cmd(s, VideoCmd::Rate(2.0), 20);
        assert!(!s.speeds_open());
        assert_eq!(
            e,
            vec![
                VideoEffect::Page(PageCmd::SetRate(2.0)),
                VideoEffect::RememberRate(2.0),
                VideoEffect::Hold(Cue::ShowFor(LEAVE_MS)),
                VideoEffect::Redraw,
            ]
        );
        assert_eq!(s.s.video().map(|v| v.rate), Some(2.0));
        // Out of range: clamped to the table's ends.
        let (_, e) = cmd(s, VideoCmd::Rate(9.0), 30);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(4.0)));
        // A control shows the controls for a while, as a hover would.
        let (s, _) = snap(start(), 1.0, 0);
        let (s, _) = cmd(s, VideoCmd::Loop, 100);
        assert!(s.shown());
        assert!(!tick(s, 100 + LEAVE_MS).shown());
    }

    #[test]
    fn a_new_document_gets_the_site_rate_again() {
        let (s, _) = snap(start(), 1.5, 0);
        // A reload's late answer from the old document took the reset state's turn ...
        let s = page_changed(s);
        let (s, _) = snap(s, 1.5, 10);
        // ... so the new document, answering next, still gets the speed.
        let new = VideoState {
            doc: 2.0,
            ..video()
        };
        let (s, e) = snap_with(s, Some(new.clone()), 1.5, Fold::Open, 20);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        let (_, e) = snap_with(s, Some(new), 1.5, Fold::Open, 30);
        assert!(!e.contains(&VideoEffect::Page(PageCmd::SetRate(1.5))));
    }

    #[test]
    fn the_site_rate_is_applied_once_per_document() {
        let (s, e) = snap(start(), 1.5, 0);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        // The page set its own speed afterwards: it is not fought.
        let (s, e) = snap(s, 1.5, 10);
        assert!(!e.contains(&VideoEffect::Page(PageCmd::SetRate(1.5))));
        let s = page_changed(s);
        assert_eq!(s.s, Overlay::default());
        let (_, e) = snap(s, 1.5, 20);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(1.5)));
        // A video without metadata yet (duration 0) waits for it.
        let early = VideoState {
            duration: 0.0,
            ..video()
        };
        let (s, e) = snap_with(start(), Some(early), 2.0, Fold::Open, 0);
        assert!(!e.contains(&VideoEffect::Page(PageCmd::SetRate(2.0))));
        let (_, e) = snap(s, 2.0, 1);
        assert_eq!(e[0], VideoEffect::Page(PageCmd::SetRate(2.0)));
    }

    #[test]
    fn controls_become_page_commands() {
        let (s, _) = snap(start(), 1.0, 0);
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
        let held = VideoEffect::Hold(Cue::ShowFor(LEAVE_MS));
        let (s, e) = cmd(s, save, 7);
        assert_eq!(e, vec![held.clone(), VideoEffect::Redraw]);
        let before = s.clone();
        let (s, e) = cmd_save(s, save, true, 8);
        assert_eq!(
            e,
            vec![
                VideoEffect::Save { x: 3.0, y: 4.0 },
                held.clone(),
                VideoEffect::Redraw
            ]
        );
        assert_eq!(s.s.video(), before.s.video());
        assert!(s.shown() && !s.collapsed());
        // A live stream cannot be sought.
        let live = VideoState {
            duration: f64::INFINITY,
            ..video()
        };
        let s = Run {
            s: Overlay::Video {
                video: live,
                rate_set: true,
                ui: Ui::Strip { speeds: false },
            },
            ..start()
        };
        let (_, e) = cmd(s, VideoCmd::Seek(0.5), 9);
        assert_eq!(e, vec![held, VideoEffect::Redraw]);
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
            container: crate::space::ContainerId::DEFAULT,
        }
    }

    #[test]
    fn the_save_control_is_there_whenever_the_downloader_is_on() {
        let v = video();
        // Nothing seen, streams still resolving, live or broken: the control stays, and its
        // menu says why there is nothing to save.
        assert_eq!(Save::of(&v, &[], None, true), Save::Offered);
        let unsaveable = media(&[
            Found::Resolving,
            Found::Refused(Refusal::Live),
            Found::Refused(Refusal::Failed("x".into())),
        ]);
        assert_eq!(Save::of(&v, &unsaveable, None, true), Save::Offered);
        let page = media(&[Found::Resolving, ready()]);
        assert_eq!(Save::of(&v, &page, None, true), Save::Offered);
        // Only the setting takes it away.
        assert_eq!(Save::of(&v, &page, None, false), Save::Absent);
        assert!(!Save::Absent.offered() && Save::Offered.offered());
    }

    #[test]
    fn protected_media_keeps_the_control_but_never_saves() {
        let page = media(&[ready()]);
        // The element has MediaKeys: protected, whatever the downloader saw; a running or
        // finished download of the page does not change that.
        let drm = VideoState {
            drm: true,
            ..video()
        };
        assert_eq!(Save::of(&drm, &page, None, true), Save::Offered);
        let running = entry(DownloadState::Running, 1, Some(2));
        assert_eq!(Save::of(&drm, &page, Some(&running), true), Save::Offered);
        let mixed = media(&[ready(), Found::Refused(Refusal::Protected)]);
        assert_eq!(Save::of(&video(), &mixed, None, true), Save::Offered);
        // The snapshot reads the flag.
        let v = serde_json::json!({"x": 0, "y": 0, "w": 1, "h": 1, "drm": true});
        assert!(VideoState::from_json(&v).unwrap().drm);
    }

    #[test]
    fn the_save_button_follows_the_latest_download() {
        let (v, page) = (video(), media(&[ready()]));
        let at = |e: DownloadEntry| Save::of(&v, &page, Some(&e), true);
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
