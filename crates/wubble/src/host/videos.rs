//! The host's half of the video built-ins: the video controls (`wubble_core::video`),
//! SponsorBlock (`wubble_core::sponsor`), picture-in-picture (`wubble_core::pip`) and the media
//! keys (`media_keys.rs`). They share one way to hear from the page: a poll of the page kit's
//! snapshot (`kit.js`) through `View::eval`, one in flight per view, for the tabs on screen, the
//! one in picture-in-picture and the ones playing sound. Every answer is checked here before it
//! is believed: numbers through `VideoState::from_json`, jump ids against the jumps the page was
//! given, strings capped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;
use wubble_core::model::Command;
use wubble_core::overlay::Kind;
use wubble_core::pip::{self, Pip, PipEffect, PipInput};
use wubble_core::settings as defs;
use wubble_core::sponsor::{self, ActTable, Plan, Segment, Skips};
use wubble_core::tab::TabId;
use wubble_core::video::{
    self, Fold, Over, Overlay, PageCmd, RateTable, VideoCmd, VideoEffect, VideoInput, VideoState,
};
use wubble_engine::input::Point;
use wubble_engine::page::PageEvent;
use wubble_ui::{AppWindow, VideoMark, VideoSave, VideoView};

use super::boost::KIT_REF;
use super::{Host, now_ms};
use crate::media_keys::{Keys, MediaKey, NowPlaying};
use crate::sponsor_net::SponsorNet;
use crate::video_smoke as smoke;

/// Rounds of the poll after which an unanswered snapshot is given up (`Videos::round`).
const POLL_GIVE_UP: u64 = 8;

/// How long a SponsorBlock lookup that got no answer (offline, a timeout, a server error) waits
/// before it is asked again (ms).
const SPONSOR_RETRY_MS: u64 = 30_000;

/// The kit's snapshot, as the poll asks for it.
const SNAPSHOT: &str =
    "(function(){var k=window[Symbol.for('wubble.kit')];return k?k.snapshot():null})()";

/// What the toast's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastAct {
    /// Undo the last skip in this tab.
    Undo(TabId),
    /// The one-time "Skip sponsor segments?" question, asked over this tab's video: it stays
    /// with the tab (`wubble_core::overlay::Kind::Notice`).
    AskSponsor(TabId),
    /// "wubble <v> is ready: restart to update" (`host/updates.rs`).
    RestartToUpdate,
}

/// The question's text and button.
pub(super) const ASK_SPONSOR: (&str, &str) =
    ("Skip sponsor segments? (uses sponsor.ajay.app)", "Turn on");

/// A DevTools call of picture-in-picture in flight.
#[derive(Clone, Copy, Debug)]
enum PipCall {
    Enter,
    /// `--smoke video`'s look at what the engine allows.
    Spike,
}

/// The jumps the page carries out: id, from, to (`kit.js` `jumps`).
type JumpTable = Vec<(usize, f64, f64)>;

/// SponsorBlock for the video one tab plays.
#[derive(Default)]
struct SponsorTab {
    video: String,
    /// `None` until the lookup answered.
    segments: Option<Vec<Segment>>,
    requested: bool,
    /// A lookup that got no answer is not asked again before this (ms).
    retry_at: u64,
    plan: Plan,
    skips: Skips,
    /// The jumps last given to the page, and the document that answered the snapshot they
    /// followed (`VideoState::doc`): a later document (a reload, the same URL again) starts
    /// without them.
    sent: Option<(f64, JumpTable)>,
}

impl SponsorTab {
    /// Whether to look the segments up now (then a lookup is under way).
    fn wants_lookup(&mut self, now: u64) -> bool {
        if self.segments.is_some() || self.requested || now < self.retry_at {
            return false;
        }
        self.requested = true;
        true
    }

    /// The lookup got no answer: asked again after a while.
    fn lookup_failed(&mut self, now: u64) {
        self.requested = false;
        self.retry_at = now + SPONSOR_RETRY_MS;
    }

    /// The jump table the page should carry out.
    fn table(&self) -> JumpTable {
        self.skips
            .table(&self.plan)
            .iter()
            .map(|j| (j.id, j.from, j.to))
            .collect()
    }

    /// The table, if document `doc` (the one that answered the snapshot) does not have it yet.
    /// An answer of a document since replaced changes nothing.
    fn unsent(&mut self, doc: f64) -> Option<JumpTable> {
        let table = self.table();
        match &self.sent {
            Some((d, _)) if doc < *d => None,
            Some((d, t)) if doc == *d && *t == table => None,
            _ => {
                self.sent = Some((doc, table.clone()));
                Some(table)
            }
        }
    }
}

pub(super) struct Videos {
    controls: bool,
    rates: RateTable,
    overlays: BTreeMap<TabId, Overlay>,
    /// Snapshot polls in flight (eval id to tab and the round it went out in), at most one per
    /// tab.
    polls: BTreeMap<u64, (TabId, u64)>,
    /// Rounds of `poll_media`. A poll unanswered for `POLL_GIVE_UP` rounds is given up: an eval
    /// sent while the frame navigates is never answered.
    round: u64,
    /// Polls sent to a document that has since gone: their answers are dropped (and may never
    /// come, so they do not hold up the next poll).
    stale: BTreeSet<u64>,
    /// Frame screenshots in flight.
    frames: BTreeMap<u64, TabId>,
    /// The pointer over the visible page's video, and over the controls.
    over_video: bool,
    over_controls: bool,
    /// Hosts where the controls stay hidden (`video.hidden_sites`), and whether a new video's
    /// strip starts folded into its tab (`video.collapsed`).
    hidden: Vec<String>,
    collapsed: bool,
    /// Whether each tab's main video played at its last snapshot.
    playing: BTreeMap<TabId, bool>,
    last_visible: Option<TabId>,
    // SponsorBlock
    sponsor_on: bool,
    sponsor_asked: bool,
    acts: ActTable,
    endpoint: String,
    /// `--smoke video`: the video id its loopback page stands for.
    smoke_video: Option<String>,
    sponsor_tabs: BTreeMap<TabId, SponsorTab>,
    /// Answers of this run, by video id (never stored).
    found: BTreeMap<String, Vec<Segment>>,
    net: Option<SponsorNet>,
    asking: bool,
    // picture-in-picture
    pip: Pip,
    pip_auto: bool,
    pip_calls: BTreeMap<u64, (TabId, PipCall)>,
    // media keys
    keys_on: bool,
    keys: Option<Keys>,
    keys_failed: bool,
    toast: Option<ToastAct>,
}

impl Default for Videos {
    fn default() -> Self {
        Self {
            controls: true,
            rates: RateTable::new(),
            overlays: BTreeMap::new(),
            polls: BTreeMap::new(),
            round: 0,
            stale: BTreeSet::new(),
            frames: BTreeMap::new(),
            over_video: false,
            over_controls: false,
            hidden: Vec::new(),
            collapsed: false,
            playing: BTreeMap::new(),
            last_visible: None,
            sponsor_on: false,
            sponsor_asked: false,
            acts: ActTable::new(),
            endpoint: sponsor::DEFAULT_ENDPOINT.into(),
            smoke_video: None,
            sponsor_tabs: BTreeMap::new(),
            found: BTreeMap::new(),
            net: None,
            asking: false,
            pip: Pip::Off,
            pip_auto: false,
            pip_calls: BTreeMap::new(),
            keys_on: true,
            keys: None,
            keys_failed: false,
            toast: None,
        }
    }
}

impl Videos {
    /// `--smoke video`: SponsorBlock asks `endpoint` (a loopback fake) and takes the loopback page
    /// for video `id`.
    pub fn for_smoke(endpoint: String, id: String) -> Self {
        Self {
            endpoint,
            smoke_video: Some(id),
            ..Self::default()
        }
    }
}

/// The kit's command for `cmd`.
fn page_script(cmd: PageCmd) -> String {
    let (name, arg) = match cmd {
        PageCmd::SetRate(r) => ("rate", r.to_string()),
        PageCmd::SetLoop(on) => ("loop", on.to_string()),
        PageCmd::SetMuted(on) => ("muted", on.to_string()),
        PageCmd::Toggle => ("toggle", "0".into()),
        PageCmd::Pause => ("pause", "0".into()),
        PageCmd::Seek(t) => ("seek", t.to_string()),
    };
    format!("{KIT_REF}&&{KIT_REF}.cmd('{name}',{arg})")
}

/// The kit's jump table.
fn jumps_script(table: &[(usize, f64, f64)]) -> String {
    let rows: Vec<Value> = table
        .iter()
        .map(|&(id, from, to)| serde_json::json!({ "id": id, "from": from, "to": to }))
        .collect();
    format!("{KIT_REF}&&{KIT_REF}.jumps({})", Value::Array(rows))
}

/// "1.5×".
fn rate_label(rate: f64) -> String {
    let r = (rate * 100.0).round() / 100.0;
    format!("{r}\u{d7}")
}

impl Host {
    // ----- settings

    pub(super) fn set_video_settings(
        &mut self,
        controls: bool,
        rates: RateTable,
        hidden: Vec<String>,
        collapsed: bool,
    ) {
        self.videos.controls = controls;
        self.videos.rates = rates;
        self.videos.hidden = hidden;
        self.videos.collapsed = collapsed;
        self.redraw_video();
    }

    pub(super) fn set_media_keys(&mut self, on: bool) {
        self.videos.keys_on = on;
        if !on {
            // Dropping the controls leaves the system's media controls.
            self.videos.keys = None;
        }
    }

    pub(super) fn set_pip_auto(&mut self, on: bool) {
        self.videos.pip_auto = on;
    }

    pub(super) fn set_sponsor(&mut self, enabled: bool, asked: bool, acts: ActTable) {
        let m = &mut self.videos;
        let was = m.sponsor_on;
        m.sponsor_on = enabled;
        m.sponsor_asked = asked;
        m.acts = acts;
        if was && !enabled {
            // Off: no page keeps skipping.
            let tabs: Vec<TabId> = self.videos.sponsor_tabs.keys().copied().collect();
            for tab in tabs {
                if let Some(view) = self.tab_view(tab) {
                    view.eval(&jumps_script(&[]));
                }
            }
            self.videos.sponsor_tabs.clear();
        }
        // The plans follow the categories at the next snapshot.
        for t in self.videos.sponsor_tabs.values_mut() {
            t.plan = Plan::default();
        }
        self.redraw_video();
    }

    // ----- the poll

    /// Asks the page kit of each tab that matters for its snapshot: the tabs on screen, the one
    /// in picture-in-picture, the ones playing sound. Also the overlay's clock.
    pub fn poll_media(&mut self) {
        let now = now_ms() as u64;
        // The docked DevTools panel is a pane but no page of the user's.
        let mut tabs: BTreeSet<TabId> = self
            .shown_tabs()
            .into_iter()
            .filter(|t| *t != super::PANEL_TAB)
            .collect();
        tabs.extend(self.videos.pip.tab());
        tabs.extend(self.audible_tabs());
        self.videos.round += 1;
        let round = self.videos.round;
        let lost: Vec<u64> = self
            .videos
            .polls
            .iter()
            .filter(|(_, (_, at))| round - at > POLL_GIVE_UP)
            .map(|(id, _)| *id)
            .collect();
        for id in lost {
            self.videos.polls.remove(&id);
            self.videos.stale.insert(id);
        }
        let polling: BTreeSet<TabId> = self.videos.polls.values().map(|(t, _)| *t).collect();
        for tab in tabs.difference(&polling).copied().collect::<Vec<_>>() {
            if let Some(id) = self.tab_view(tab).and_then(|v| v.eval(SNAPSHOT)) {
                self.videos.polls.insert(id.0, (tab, round));
            }
        }
        let shown: Vec<TabId> = self.videos.overlays.keys().copied().collect();
        for tab in shown {
            self.video_event(tab, VideoInput::Tick(now));
        }
    }

    /// An answer to one of the media calls; whether it was one.
    pub(super) fn media_answer(&mut self, tab: TabId, event: &PageEvent) -> bool {
        match event {
            PageEvent::EvalResult { id, result } => {
                if self.videos.stale.remove(&id.0) {
                    return true;
                }
                if self.videos.polls.remove(&id.0).is_some() {
                    let json = result
                        .as_deref()
                        .ok()
                        .and_then(|t| serde_json::from_str::<Value>(t).ok());
                    self.media_snapshot(tab, json.unwrap_or(Value::Null));
                    return true;
                }
                if self.videos.frames.remove(&id.0).is_some() {
                    let png = result
                        .as_deref()
                        .ok()
                        .and_then(|t| serde_json::from_str::<Option<String>>(t).ok())
                        .flatten();
                    match png {
                        Some(png) => self.save_shot(tab, png, true),
                        None => self.toast_text("This video's frames cannot be saved"),
                    }
                    return true;
                }
                false
            }
            PageEvent::CdpReply { id, result } => {
                let Some((tab, call)) = self.videos.pip_calls.remove(&id.0) else {
                    return false;
                };
                // `Runtime.evaluate` answers with the value, or its exception.
                let outcome = result
                    .clone()
                    .and_then(|v| match v.get("exceptionDetails") {
                        Some(e) => Err(e
                            .pointer("/exception/description")
                            .or_else(|| e.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("refused")
                            .chars()
                            .take(200)
                            .collect()),
                        None => Ok(v),
                    });
                match call {
                    PipCall::Enter => {
                        self.pip_event(PipInput::Fullscreen(outcome.map(|_| ())));
                    }
                    PipCall::Spike => {
                        let text = outcome
                            .as_ref()
                            .ok()
                            .and_then(|v| v.pointer("/result/value"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("{outcome:?}"));
                        let _ = tab;
                        self.video_smoke_step(smoke::Input::PipSpiked(&text));
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn media_snapshot(&mut self, tab: TabId, json: Value) {
        let now = now_ms() as u64;
        // A frame's area is a strip only over what the downloader saw of the page.
        let video = json
            .get("v")
            .and_then(VideoState::from_json)
            .filter(|v| !v.frame || self.media_has_shown(tab));
        self.videos
            .playing
            .insert(tab, video.as_ref().is_some_and(VideoState::plays_aloud));
        let url = self
            .model
            .tabs
            .get(&tab)
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let site_rate = video::site_rate(&self.videos.rates, &url);
        let fold = if self.videos.collapsed {
            Fold::Tucked
        } else {
            Fold::Open
        };
        self.video_event(
            tab,
            VideoInput::Snapshot {
                video: video.clone(),
                site_rate,
                fold,
                now,
            },
        );
        let fired: Vec<usize> = json
            .get("fired")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_u64)
                    .map(|i| i as usize)
                    .collect()
            })
            .unwrap_or_default();
        self.sponsor_snapshot(tab, &url, video.as_ref(), &fired, now);
        if let Some(zap) = json.get("zap").filter(|z| z.is_object()) {
            let on = zap.get("on").and_then(Value::as_bool).unwrap_or(false);
            let selector = zap.get("selector").and_then(Value::as_str);
            self.boost_zapped(tab, on, selector);
        }
        if self.key_target() == Some(tab)
            && let Some(keys) = self.videos.keys.as_mut()
        {
            match json.get("meta").filter(|m| m.is_object()) {
                Some(meta) => keys.show(&NowPlaying::from_json(meta)),
                None => keys.stop(),
            }
        }
        if self.checks.video_smoke.is_some() {
            let seen = self.video_smoke_seen(fired.len());
            self.video_smoke_step(smoke::Input::Snapshot(&seen));
        }
    }

    // ----- the video controls

    fn video_event(&mut self, tab: TabId, input: VideoInput) {
        let state = self.videos.overlays.remove(&tab).unwrap_or_default();
        let (next, effects) = video::transition(state, input);
        if next != Overlay::default() {
            self.videos.overlays.insert(tab, next);
        }
        for effect in effects {
            match effect {
                VideoEffect::Page(cmd) => {
                    // As the user's own action: the controls' clicks and the keys are, and a
                    // page lets only such a call start a video with its sound on (Chromium's
                    // autoplay policy); `View::eval` is none, so play after unmute did nothing.
                    let params = serde_json::json!({
                        "expression": page_script(cmd),
                        "userGesture": true,
                    });
                    if let Some(view) = self.tab_view(tab) {
                        view.cdp("Runtime.evaluate", &params);
                    }
                }
                VideoEffect::RememberRate(rate) => {
                    let url = self.model.tabs.get(&tab).map(|t| t.url.clone());
                    if let Some(host) = url.as_deref().and_then(wubble_core::zoom::host_of) {
                        let table = video::with_rate(self.videos.rates.clone(), &host, rate);
                        self.write_setting(
                            defs::VIDEO_RATES.key,
                            &wubble_store::text_of::<wubble_store::VideoRates>(&table),
                        );
                    }
                }
                VideoEffect::RememberCollapsed(collapsed) => {
                    self.write_setting(
                        defs::VIDEO_COLLAPSED.key,
                        if collapsed { "1" } else { "0" },
                    );
                }
                VideoEffect::HideSite => {
                    let url = self.model.tabs.get(&tab).map(|t| t.url.clone());
                    if let Some(host) = url.as_deref().and_then(wubble_core::zoom::host_of)
                        && let Some(text) = wubble_core::apply::with_entry(
                            defs::VIDEO_HIDDEN_SITES.key,
                            &self.settings.stored,
                            &host,
                        )
                    {
                        self.write_setting(defs::VIDEO_HIDDEN_SITES.key, &text);
                        self.toast_text(&format!(
                            "Video controls hidden on {host}. Show them again in Settings, under \"{}\".",
                            defs::VIDEO_HIDDEN_SITES.label
                        ));
                    }
                }
                VideoEffect::Frame => {
                    let script = format!("{KIT_REF}?{KIT_REF}.frame():null");
                    if let Some(id) = self.tab_view(tab).and_then(|v| v.eval(&script)) {
                        self.videos.frames.insert(id.0, tab);
                    }
                }
                VideoEffect::Pip => self.pip_event(PipInput::Request(tab)),
                VideoEffect::Save { x, y } => self.media_entry(tab, x, y),
                VideoEffect::Redraw => {
                    if self.shown() == Some(tab) {
                        self.redraw_video();
                    }
                }
            }
        }
    }

    /// The controls' intents (`video-controls.slint`).
    pub(crate) fn video_command(&mut self, name: &str, value: f32) {
        let Some(tab) = self.shown() else {
            return;
        };
        let cmd = match name {
            "play" => VideoCmd::PlayPause,
            "speeds" => VideoCmd::Speeds,
            "rate" => match usize::try_from(value as i64)
                .ok()
                .and_then(|i| video::RATES.get(i))
            {
                Some(&r) => VideoCmd::Rate(r),
                None => return,
            },
            "loop" => VideoCmd::Loop,
            "mute" => VideoCmd::Mute,
            "seek" => VideoCmd::Seek(f64::from(value)),
            "seek-by" => VideoCmd::SeekBy(f64::from(value)),
            "pip" => VideoCmd::Pip,
            "frame" => VideoCmd::Frame,
            "collapse" => VideoCmd::Collapse,
            "expand" => VideoCmd::Expand,
            "dismiss" => VideoCmd::Dismiss,
            "hide-site" => VideoCmd::HideSite,
            _ => return,
        };
        self.video_cmd(tab, cmd);
    }

    /// The strip's save button, its bottom left at `x`, `y` (window px).
    pub(crate) fn video_save(&mut self, x: f32, y: f32) {
        if let Some(tab) = self.shown() {
            self.video_cmd(tab, VideoCmd::Save { x, y });
        }
    }

    fn video_cmd(&mut self, tab: TabId, cmd: VideoCmd) {
        let can_save = self.video_save_state(tab).offered();
        self.video_event(
            tab,
            VideoInput::Command {
                cmd,
                now: now_ms() as u64,
                can_save,
            },
        );
    }

    /// The shown tab's save button, to tell whether a change of the downloader moved it.
    pub(super) fn video_save_now(&self) -> Option<(TabId, video::Save)> {
        let tab = self.shown()?;
        Some((tab, self.video_save_state(tab)))
    }

    /// Draws the strip again when the downloader moved its save button from `before`.
    pub(super) fn video_save_redraw(&mut self, before: Option<(TabId, video::Save)>) {
        if before != self.video_save_now() {
            self.redraw_video();
        }
    }

    /// The tab's main video carries MediaKeys (EME): protected, whatever the downloader saw.
    pub(super) fn video_is_protected(&self, tab: TabId) -> bool {
        self.videos
            .overlays
            .get(&tab)
            .and_then(Overlay::video)
            .is_some_and(|v| v.drm)
    }

    /// `--smoke media=probe`: what the strip knows of the tab's video.
    pub(super) fn video_overlay_debug(&self, tab: TabId) -> String {
        match self.videos.overlays.get(&tab).and_then(Overlay::video) {
            Some(v) => format!("{v:?}"),
            None => "no video seen by the page kit".into(),
        }
    }

    /// The tab's save button, from the downloader's state (`video::Save::of`).
    pub(super) fn video_save_state(&self, tab: TabId) -> video::Save {
        let Some(v) = self.videos.overlays.get(&tab).and_then(Overlay::video) else {
            return video::Save::Absent;
        };
        let latest = self.media_latest(tab).and_then(|id| self.downloads.get(id));
        video::Save::of(v, self.media_items(tab), latest, self.media_enabled())
    }

    pub(crate) fn video_hovered(&mut self, over: bool) {
        self.videos.over_controls = over;
        // Off the controls: on the video or not by where the pointer is now. The pane heard
        // nothing while the controls had the pointer, and a strip that goes from under a still
        // pointer (collapsed, a click away) gives it no move: its last word, "left", is stale.
        if !over && let Some(window) = self.w.window.upgrade() {
            let video = self.video_on_window(&window).map(|(r, _)| r);
            self.videos.over_video = self
                .input
                .pointer
                .zip(video)
                .is_some_and(|(p, r)| r.contains((f64::from(p.x), f64::from(p.y))));
        }
        self.video_pointer_changed();
    }

    /// The pointer moved over pane `pane` at `point` (logical px in the pane).
    pub(super) fn media_pointer(&mut self, pane: i32, point: Option<Point>) {
        let Some(tab) = self.shown() else {
            return;
        };
        if crate::shell::pane_id(tab) != pane {
            return;
        }
        let zoom = self.tab_view(tab).map_or(100, |v| v.zoom_percent());
        let f = f64::from(zoom) / 100.0;
        let over = point.is_some_and(|p| {
            self.videos
                .overlays
                .get(&tab)
                .and_then(Overlay::video)
                .is_some_and(|v| v.rect.contains((f64::from(p.x) / f, f64::from(p.y) / f)))
        });
        // Every move over the video counts: the controls go a while after the last one.
        if over || over != self.videos.over_video {
            self.videos.over_video = over;
            self.video_pointer_changed();
        }
    }

    fn video_pointer_changed(&mut self) {
        if let Some(tab) = self.shown() {
            let over = match (self.videos.over_controls, self.videos.over_video) {
                (true, _) => Over::Strip,
                (false, true) => Over::Video,
                (false, false) => Over::Away,
            };
            let now = now_ms() as u64;
            self.video_event(tab, VideoInput::Pointer { over, now });
        }
    }

    /// The overlay's clock between polls, so the controls go on time (`LEAVE_MS` is shorter than
    /// a poll).
    pub fn tick_video(&mut self) {
        let now = now_ms() as u64;
        let tabs: Vec<TabId> = self.videos.overlays.keys().copied().collect();
        for tab in tabs {
            self.video_event(tab, VideoInput::Tick(now));
        }
    }

    /// A new document in `tab`: its video is gone.
    pub(super) fn media_page_changed(&mut self, tab: TabId) {
        let old: Vec<u64> = self
            .videos
            .polls
            .iter()
            .filter(|(_, (t, _))| *t == tab)
            .map(|(id, _)| *id)
            .collect();
        for id in old {
            self.videos.polls.remove(&id);
            self.videos.stale.insert(id);
        }
        self.video_event(tab, VideoInput::PageChanged);
        self.videos.playing.remove(&tab);
    }

    pub(super) fn media_view_gone(&mut self, tab: TabId) {
        self.videos.overlays.remove(&tab);
        self.videos.polls.retain(|_, (t, _)| *t != tab);
        self.videos.frames.retain(|_, t| *t != tab);
        self.videos.pip_calls.retain(|_, (t, _)| *t != tab);
        self.videos.playing.remove(&tab);
        self.videos.sponsor_tabs.remove(&tab);
        self.pip_event(PipInput::TabClosed(tab));
        self.redraw_video();
    }

    /// After every change to the model: a tab switch is picture-in-picture's cue.
    pub(super) fn media_after_change(&mut self) {
        // the focused Main window's tab: the Pip window's own is no switch
        let visible = self.model.window_tab(self.model.main_window());
        let last = std::mem::replace(&mut self.videos.last_visible, visible);
        if last == visible {
            return;
        }
        self.videos.over_video = false;
        self.videos.over_controls = false;
        if let Some(from) = last {
            // Playing with sound the tab makes: not a muted or silent clip.
            let playing = self.videos.playing.get(&from).copied().unwrap_or(false)
                && self.audible_tabs().any(|t| t == from);
            let auto = self.videos.pip_auto;
            self.pip_event(PipInput::TabSwitched {
                from,
                auto,
                playing,
            });
        }
        if let Some(to) = visible {
            self.pip_event(PipInput::TabActivated(to));
        }
    }

    fn redraw_video(&mut self) {
        if let Some(window) = self.w.window.upgrade() {
            self.render_video(&window);
        }
    }

    /// Whether the active tab has an open or collapsed video overlay (the only consumer of
    /// `pointer-in`/`pointer-x`/`pointer-y` on `AppWindow`).
    pub(super) fn video_overlay_active(&self) -> bool {
        let Some(tab) = self.model.window_key_tab(self.w.id) else {
            return false;
        };
        self.videos.controls
            && !self.site_hides_video(tab)
            && self
                .videos
                .overlays
                .get(&tab)
                .is_some_and(|o| o.shown() || o.collapsed())
    }

    /// The visible page's controls.
    pub(super) fn render_video(&mut self, window: &AppWindow) {
        if slint::Model::row_count(&window.get_video_rates()) != video::RATES.len() {
            let rates: Vec<slint::SharedString> =
                video::RATES.iter().map(|&r| rate_label(r).into()).collect();
            window.set_video_rates(slint::ModelRc::new(slint::VecModel::from(rates)));
        }
        let prev = window.get_video();
        // a tab in picture-in-picture has its controls there, not under the Main window's card
        let tab = self.model.window_key_tab(self.w.id);
        let on_window = self.video_on_window(window);
        let overlay = tab.and_then(|t| self.videos.overlays.get(&t));
        let view = match (overlay, tab, on_window) {
            (Some(o), Some(tab), Some((r, pane)))
                if self.videos.controls && !self.site_hides_video(tab) =>
            {
                o.video().map(|v| {
                    let marks = self.sponsor_marks(tab, v.duration);
                    let same_marks = slint::Model::row_count(&prev.marks) == marks.len()
                        && slint::Model::iter(&prev.marks)
                            .zip(&marks)
                            .all(|(a, b)| a == *b);
                    let marks = if same_marks {
                        prev.marks.clone()
                    } else {
                        slint::ModelRc::new(slint::VecModel::from(marks))
                    };
                    let save = self.video_save_state(tab);
                    let nearest = video::nearest_rate(v.rate);
                    VideoView {
                        open: o.shown(),
                        collapsed: o.collapsed(),
                        pane_right: (pane.x + pane.w) as f32,
                        pane_bottom: (pane.y + pane.h) as f32,
                        x: r.x as f32,
                        y: r.y as f32,
                        w: r.w as f32,
                        h: r.h as f32,
                        paused: v.paused,
                        looped: v.looped,
                        muted: v.muted,
                        progress: v.progress() as f32,
                        rate: rate_label(v.rate).into(),
                        rate_index: video::RATES
                            .iter()
                            .position(|&r| r == nearest)
                            .map_or(-1, |i| i as i32),
                        speeds: o.speeds_open(),
                        can_pip: self.model.can_pip(tab),
                        frame: v.frame,
                        save: match save {
                            video::Save::Absent => VideoSave::Absent,
                            video::Save::Offered => VideoSave::Offered,
                            video::Save::Saving(_) => VideoSave::Saving,
                            video::Save::Saved => VideoSave::Saved,
                            video::Save::Failed => VideoSave::Failed,
                        },
                        save_fraction: match save {
                            video::Save::Saving(Some(f)) => f,
                            _ => -1.0,
                        },
                        save_label: save.label().into(),
                        marks,
                    }
                })
            }
            _ => None,
        };
        let next = view.unwrap_or_default();
        if next != prev {
            if (next.open || next.collapsed) && !(prev.open || prev.collapsed) {
                self.sync_pointer(window);
            }
            window.set_video(next);
        }
    }

    /// The window's tab's video in the window (logical px), cut to its pane, and the pane's
    /// viewport: the controls never reach over another pane or off the page.
    fn video_on_window(&mut self, window: &AppWindow) -> Option<(video::Rect, video::Rect)> {
        let tab = self.model.window_key_tab(self.w.id)?;
        let zoom = self.tab_view(tab).map_or(100, |v| v.zoom_percent());
        let rect = self
            .videos
            .overlays
            .get(&tab)
            .and_then(Overlay::video)?
            .rect;
        let (x, y) = self.content_origin(window);
        let pane = video::Rect {
            x: f64::from(x),
            y: f64::from(y),
            w: self
                .pane_rects(window)
                .into_iter()
                .find(|p| p.tab == tab)
                .map_or(0.0, |p| p.width),
            h: f64::from(window.get_content_height()) - f64::from(y),
        };
        let r = rect
            .on_window(zoom, (f64::from(x), f64::from(y)))
            .intersect(&pane)?;
        Some((r, pane))
    }

    /// The tab's site is one the user hid the controls on.
    fn site_hides_video(&self, tab: TabId) -> bool {
        self.model
            .tabs
            .get(&tab)
            .and_then(|t| wubble_core::zoom::host_of(&t.url))
            .is_some_and(|host| self.videos.hidden.contains(&host))
    }

    // ----- SponsorBlock

    fn sponsor_snapshot(
        &mut self,
        tab: TabId,
        url: &str,
        video: Option<&VideoState>,
        fired: &[usize],
        now: u64,
    ) {
        let smoke_page = self.videos.smoke_video.is_some() && url.starts_with("http://127.0.0.1:");
        let id = sponsor::video_id(url).or_else(|| {
            smoke_page
                .then(|| self.videos.smoke_video.clone())
                .flatten()
        });
        let Some(id) = id else {
            self.videos.sponsor_tabs.remove(&tab);
            return;
        };
        if !self.videos.sponsor_on {
            if !self.videos.sponsor_asked
                && !self.videos.asking
                && video.is_some()
                && self.shown() == Some(tab)
                && self.overlay_open(Kind::Notice, Some(tab))
            {
                self.videos.asking = true;
                self.toast_with(ASK_SPONSOR.0, ASK_SPONSOR.1, true);
                self.videos.toast = Some(ToastAct::AskSponsor(tab));
            }
            return;
        }
        let entry = self.videos.sponsor_tabs.entry(tab).or_default();
        if entry.video != id {
            *entry = SponsorTab {
                video: id.clone(),
                ..SponsorTab::default()
            };
        }
        if entry.segments.is_none()
            && let Some(found) = self.videos.found.get(&id)
        {
            entry.segments = Some(found.clone());
        }
        if entry.wants_lookup(now) {
            if self.videos.net.is_none() {
                self.videos.net = SponsorNet::spawn(|tab, video, segments| {
                    let _ = slint::invoke_from_event_loop(move || {
                        crate::with_host(|host| host.sponsor_found(tab, video, segments));
                    });
                });
            }
            if let Some(net) = &self.videos.net {
                net.lookup(tab, &id, &self.videos.endpoint);
            }
        }
        let entry = self.videos.sponsor_tabs.entry(tab).or_default();
        let (Some(segments), Some(video)) = (&entry.segments, video) else {
            return;
        };
        let plan = sponsor::plan(segments, &self.videos.acts, video.duration);
        if plan != entry.plan {
            entry.plan = plan;
        }
        let mut notice = None;
        for &f in fired {
            if let Some(n) = entry.skips.fired(&entry.plan, f, now) {
                notice = Some(n.text());
            }
        }
        if let Some(table) = entry.unsent(video.doc)
            && let Some(view) = self.tab_view(tab)
        {
            view.eval(&jumps_script(&table));
        }
        if let Some(text) = notice
            && self.shown() == Some(tab)
        {
            self.toast_with(&text, "Undo", false);
            self.videos.toast = Some(ToastAct::Undo(tab));
        }
    }

    /// The lookup thread's answer for `video` in `tab`. Only a real answer is kept for the run;
    /// a lookup that got none is asked again later.
    fn sponsor_found(&mut self, tab: TabId, video: String, segments: Result<Vec<Segment>, String>) {
        let entry = self
            .videos
            .sponsor_tabs
            .get_mut(&tab)
            .filter(|e| e.video == video);
        match segments {
            Ok(segments) => {
                if let Some(entry) = entry {
                    entry.segments = Some(segments.clone());
                }
                self.videos.found.insert(video, segments);
            }
            Err(_) => {
                if let Some(entry) = entry {
                    entry.lookup_failed(now_ms() as u64);
                }
            }
        }
    }

    /// The progress bar's segments of `tab`'s video.
    fn sponsor_marks(&self, tab: TabId, duration: f64) -> Vec<VideoMark> {
        let Some(entry) = self.videos.sponsor_tabs.get(&tab) else {
            return vec![];
        };
        if !duration.is_finite() || duration <= 0.0 {
            return vec![];
        }
        entry
            .plan
            .marks
            .iter()
            .map(|m| {
                let [_, r, g, b] = m.colour.to_be_bytes();
                VideoMark {
                    from: (m.from / duration) as f32,
                    to: (m.to / duration) as f32,
                    colour: slint::Color::from_rgb_u8(r, g, b),
                }
            })
            .collect()
    }

    /// Undo of the last skip in `tab`: back to where it began, and that jump is not taken again.
    pub(crate) fn sponsor_undo(&mut self, tab: TabId) {
        let now = now_ms() as u64;
        let Some(entry) = self.videos.sponsor_tabs.get_mut(&tab) else {
            return;
        };
        let Some(from) = entry.skips.undo(now) else {
            return;
        };
        let table = entry.table();
        let doc = entry.sent.as_ref().map_or(0.0, |(d, _)| *d);
        entry.sent = Some((doc, table.clone()));
        if let Some(view) = self.tab_view(tab) {
            // `eval` takes one expression.
            view.eval(&format!(
                "({}),({})",
                jumps_script(&table),
                page_script(PageCmd::Seek(from))
            ));
        }
    }

    // ----- the toast's button

    pub(crate) fn toast_acted(&mut self) {
        match self.videos.toast.take() {
            Some(ToastAct::Undo(tab)) => self.sponsor_undo(tab),
            Some(ToastAct::AskSponsor(_)) => {
                self.videos.asking = false;
                self.write_setting(defs::SPONSOR_ASKED.key, "1");
                self.write_setting(defs::SPONSOR_ENABLED.key, "1");
            }
            Some(ToastAct::RestartToUpdate) => self.updates_restart(),
            None => {}
        }
    }

    pub(crate) fn toast_closed(&mut self) {
        if let Some(ToastAct::AskSponsor(_)) = self.videos.toast.take() {
            self.videos.asking = false;
            self.write_setting(defs::SPONSOR_ASKED.key, "1");
        }
    }

    /// Another notice replaced whatever the toast's button was for. An unanswered question is
    /// not asked again in this run (`asking` stays), only in the next.
    pub(super) fn toast_replaced(&mut self) {
        self.videos.toast = None;
    }

    /// The tab the question is asked over, while it waits.
    pub(super) fn notice_tab(&self) -> Option<TabId> {
        match self.videos.toast {
            Some(ToastAct::AskSponsor(tab)) => Some(tab),
            _ => None,
        }
    }

    /// The question goes unanswered (its page went): as replaced, not asked again this run.
    pub(super) fn notice_close(&mut self) {
        if self.notice_tab().is_some() {
            self.toast_replaced();
            self.toast_hide();
        }
    }

    /// Its tab left (false) or came back (true): the question hides or shows again, waiting.
    pub(super) fn notice_shown(&mut self, shown: bool) {
        let Some(act @ ToastAct::AskSponsor(_)) = self.videos.toast else {
            return;
        };
        if shown {
            self.toast_with(ASK_SPONSOR.0, ASK_SPONSOR.1, true);
            self.videos.toast = Some(act);
        } else {
            self.toast_hide();
        }
    }

    /// The update is ready: a notice with "Restart" that runs `updates_restart`.
    pub(super) fn toast_restart(&mut self, text: &str) {
        self.toast_with(text, wubble_core::update::RESTART_ACTION, false);
        self.videos.toast = Some(ToastAct::RestartToUpdate);
    }

    fn toast_text(&mut self, text: &str) {
        if let Some(window) = self.w.window.upgrade() {
            self.toast(&window, text);
        }
    }

    // ----- picture-in-picture

    fn pip_event(&mut self, input: PipInput) {
        let (next, effects) = pip::transition(self.videos.pip, input);
        self.videos.pip = next;
        for effect in effects {
            match effect {
                PipEffect::EnterFullscreen(tab) => {
                    let params = serde_json::json!({
                        "expression": format!("{KIT_REF}.pip()"),
                        "userGesture": true,
                        "awaitPromise": true,
                        "returnByValue": true,
                    });
                    match self
                        .tab_view(tab)
                        .and_then(|v| v.cdp("Runtime.evaluate", &params))
                    {
                        Some(id) => {
                            self.videos.pip_calls.insert(id.0, (tab, PipCall::Enter));
                        }
                        None => self.pip_event(PipInput::Fullscreen(Err("no page".into()))),
                    }
                }
                PipEffect::ExitFullscreen(tab) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.exit_fullscreen();
                    }
                }
                PipEffect::Pause(tab) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.eval(&page_script(PageCmd::Pause));
                    }
                }
                PipEffect::OpenWindow(tab) => {
                    self.dispatch(Command::OpenPip { tab });
                    self.pip_event(PipInput::WindowOpened(self.model.pip_tab() == Some(tab)));
                }
                // the model has pruned it already when the tab went
                PipEffect::CloseWindow(tab) if self.model.pip_tab() == Some(tab) => {
                    self.dispatch(Command::ClosePip);
                }
                PipEffect::CloseWindow(_) => {}
                PipEffect::Toast(text) => self.toast_text(text),
            }
        }
    }

    /// The model's Pip window closed (its close button, return to tab, the window manager, or
    /// this machine's own `CloseWindow`).
    pub(super) fn pip_window_closed(&mut self) {
        self.pip_event(PipInput::WindowClosed);
    }

    /// The user closed the Pip window (its close button, the window manager): the video pauses,
    /// as in Chrome, and this machine closes the window. False when nothing is out (the caller
    /// closes the window itself).
    pub(super) fn pip_user_closed(&mut self) -> bool {
        let out = self.videos.pip.tab().is_some();
        if out {
            self.pip_event(PipInput::Close);
        }
        out
    }

    /// The tab whose page fullscreen belongs to picture-in-picture, not to the window.
    pub(super) fn pip_tab(&self) -> Option<TabId> {
        self.videos.pip.tab()
    }

    /// `tab`'s page left fullscreen.
    pub(super) fn pip_fullscreen_left(&mut self, tab: TabId) {
        self.pip_event(PipInput::FullscreenLeft(tab));
    }

    /// `--smoke video`: what the engine allows (logged, not asserted): the page's
    /// `document.pictureInPictureEnabled`, and whether the main video goes fullscreen when asked
    /// as a user gesture (then it leaves again).
    pub(crate) fn pip_spike(&mut self) {
        let Some(tab) = self.shown() else {
            return;
        };
        let expression = format!(
            "(async()=>{{const api=!!document.pictureInPictureEnabled;let fs;\
             try{{await {KIT_REF}.pip();fs='ok';await document.exitFullscreen();}}\
             catch(e){{fs=String(e&&e.message||e);}}return 'api='+api+' fullscreen='+fs;}})()"
        );
        let params = serde_json::json!({
            "expression": expression,
            "userGesture": true,
            "awaitPromise": true,
            "returnByValue": true,
        });
        if let Some(id) = self
            .tab_view(tab)
            .and_then(|v| v.cdp("Runtime.evaluate", &params))
        {
            self.videos.pip_calls.insert(id.0, (tab, PipCall::Spike));
        }
    }

    /// The Pip window's controls (`pip-controls.slint`) and the Main window's card
    /// (`pip-card.slint`): play, seek-by (seconds), return, close.
    pub(crate) fn pip_command(&mut self, name: &str, value: f32) {
        match name {
            "return" => self.dispatch(Command::ReturnPip),
            "close" => {
                if !self.pip_user_closed() {
                    self.dispatch(Command::ClosePip);
                }
            }
            "play" | "seek-by" => {
                if let Some(tab) = self.model.pip_tab() {
                    let here = self.w.id;
                    if let Some(w) = self.model.window_of(tab)
                        && self.enter(w)
                    {
                        self.video_command(name, value);
                    }
                    self.enter(here);
                }
            }
            _ => {}
        }
    }

    /// The Main window's card for its tab in picture-in-picture, and the Pip window's controls.
    pub(super) fn render_pip(&mut self, window: &AppWindow) {
        let card = self.model.pip_tab().filter(|&t| {
            self.w.kind == wubble_core::window::WindowKind::Main && self.shown() == Some(t)
        });
        window.set_pip_card(card.is_some());
        let title = card
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.title.clone())
            .unwrap_or_default();
        window.set_pip_title(title.into());
    }

    // ----- media keys

    /// A tab plays sound: the system's media controls start (once).
    pub(super) fn media_audible(&mut self, audible: bool) {
        let m = &mut self.videos;
        if !audible || !m.keys_on || m.keys.is_some() || m.keys_failed {
            return;
        }
        match Keys::start(|key| {
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host(|host| host.media_key(key));
            });
        }) {
            Ok(keys) => m.keys = Some(keys),
            Err(e) => {
                m.keys_failed = true;
                eprintln!("wubble: media keys are off: {e}");
            }
        }
    }

    /// The tab the media keys act on: one that plays sound, else the one in picture-in-picture,
    /// else the visible one.
    /// The tabs playing sound (`Reason::Audible` in the model's busy set).
    fn audible_tabs(&self) -> impl Iterator<Item = TabId> + '_ {
        self.model
            .tabs
            .values()
            .filter(|t| t.busy.contains(wubble_core::busy::Reason::Audible))
            .map(|t| t.id)
    }

    fn key_target(&self) -> Option<TabId> {
        self.audible_tabs()
            .next()
            .or(self.videos.pip.tab())
            .or(self.model.visible_tab())
    }

    pub(crate) fn media_key(&mut self, key: MediaKey) {
        let Some(tab) = self.key_target() else {
            return;
        };
        let script = format!("{KIT_REF}&&{KIT_REF}.act('{}')", key.action());
        if let Some(view) = self.tab_view(tab) {
            view.eval(&script);
        }
    }

    // ----- `--smoke video`

    fn video_smoke_seen(&mut self, fired: usize) -> smoke::Seen {
        // the Main window's tab, wherever its frames show
        let tab = self.model.window_tab(self.model.main_window());
        let video = tab
            .and_then(|t| self.videos.overlays.get(&t))
            .and_then(Overlay::video)
            .cloned();
        let pip = self.pip_smoke_seen();
        let sponsor = tab.and_then(|t| self.videos.sponsor_tabs.get(&t));
        let overlay = tab.and_then(|t| self.videos.overlays.get(&t));
        let window = self.w.window.upgrade();
        let toast = window
            .as_ref()
            .map(|w| (w.get_toast_serial(), w.get_toast_text().to_string()))
            .unwrap_or_default();
        // what the pointer (`video_smoke_pointer`, the same window) finds: `VideoControls`
        // shows only over a video with a size
        let controls = window.is_some_and(|w| w.get_video().w > 0.0);
        smoke::Seen {
            video,
            jumps: sponsor.map_or(0, |s| s.sent.as_ref().map_or(0, |(_, t)| t.len())),
            notice: sponsor.is_some_and(|s| s.skips.notice(now_ms() as u64).is_some()),
            fired,
            pip,
            now: now_ms() as u64,
            controls,
            strip: overlay.is_some_and(Overlay::shown),
            collapsed: overlay.is_some_and(Overlay::collapsed),
            speeds: overlay.is_some_and(Overlay::speeds_open),
            menu: tab.is_some_and(|t| {
                self.surfaces
                    .get(&t)
                    .is_some_and(|s| s.menu != crate::menu::Menu::Closed)
            }),
            toast,
            save: tab.map(|t| self.video_save_state(t)),
            ready: tab.is_some_and(|t| {
                self.media_items(t)
                    .iter()
                    .any(|i| matches!(i.found, wubble_core::media::detect::Found::Ready(_)))
            }),
            runs: tab.map_or(0, |t| self.media_runs_from(t)),
            saved: tab.and_then(|t| self.media_saved(t)),
        }
    }

    /// `--smoke video`'s pointer: window events at a spot of the strip, found on screen (the
    /// elements' ids, in a debug build), sent once the host is free, as winit's would be.
    fn video_smoke_pointer(&self, spot: smoke::Spot, click: bool) {
        use i_slint_backend_testing::ElementHandle;
        use slint::ComponentHandle;
        use slint::platform::{PointerEventButton, WindowEvent};
        let weak = self.w.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = weak.upgrade() else { return };
            let find = |id: &str| {
                ElementHandle::find_by_element_id(&window, &format!("VideoControls::{id}"))
                    .collect::<Vec<_>>()
            };
            let centre = |e: &ElementHandle, fx: f32| {
                let (p, size) = (e.absolute_position(), e.size());
                slint::LogicalPosition::new(p.x + size.width * fx, p.y + size.height / 2.0)
            };
            let video = find("root");
            let Some(video) = video.first() else {
                return crate::with_host(|h| h.fail("no video controls on screen".into()));
            };
            let (vp, vs) = (video.absolute_position(), video.size());
            let at = match spot {
                smoke::Spot::Video => Some(slint::LogicalPosition::new(
                    vp.x + vs.width / 2.0,
                    vp.y + vs.height * 0.75,
                )),
                smoke::Spot::VideoNear => Some(slint::LogicalPosition::new(
                    vp.x + vs.width * 0.4,
                    vp.y + vs.height * 0.75,
                )),
                smoke::Spot::Away => Some(slint::LogicalPosition::new(
                    vp.x + vs.width + 40.0,
                    vp.y + vs.height * 0.75,
                )),
                smoke::Spot::Button(id) => find(id).first().map(|e| centre(e, 0.5)),
                smoke::Spot::Bar(f) => find("bar").first().map(|e| centre(e, f)),
                smoke::Spot::Speed(i) => find("speed").get(i).map(|e| centre(e, 0.5)),
            };
            let Some(position) = at else {
                return crate::with_host(|h| h.fail(format!("{spot:?} is not on screen")));
            };
            crate::with_host(|h| h.set_pointer(&window, Some(position)));
            let w = window.window();
            w.dispatch_event(WindowEvent::PointerMoved { position });
            if click {
                let button = PointerEventButton::Left;
                w.dispatch_event(WindowEvent::PointerPressed { position, button });
                w.dispatch_event(WindowEvent::PointerReleased { position, button });
            }
        });
    }

    /// Picture-in-picture as `--smoke video` checks it: the machine, the model's Pip window and
    /// what each platform window shows.
    fn pip_smoke_seen(&mut self) -> smoke::PipSeen {
        let here = self.w.id;
        let pip = self.model.pip_window();
        let open = pip.is_some_and(|w| self.enter(w));
        // a frame at the window's own size: the view was laid out for it (no gap, no chrome)
        let size = self
            .w
            .window
            .upgrade()
            .map(|w| slint::ComponentHandle::window(&w).size());
        let framed = match (self.model.pip_tab(), size) {
            (Some(tab), Some(size)) if open && !self.snapshots.standing_in(tab) => self
                .w
                .frames
                .get(&tab)
                .map(|f| (f.clip.width, f.clip.height))
                .filter(|&(w, h)| w.abs_diff(size.width) <= 2 && h.abs_diff(size.height) <= 2),
            _ => None,
        };
        let card = self.enter(self.model.main_window())
            && self.w.window.upgrade().is_some_and(|w| w.get_pip_card());
        self.enter(here);
        smoke::PipSeen {
            on: matches!(self.videos.pip, Pip::On { .. }),
            open,
            framed,
            card,
            windows: self.window_ids().len(),
        }
    }

    pub(super) fn video_smoke_worked(&mut self) {
        if self.checks.video_smoke.is_some() {
            self.poll_media();
            self.video_smoke_step(smoke::Input::Worked);
        }
    }

    pub(super) fn video_smoke_boost_saved(&mut self, id: i64) {
        self.video_smoke_step(smoke::Input::BoostSaved(id));
    }

    pub(super) fn video_smoke_step(&mut self, input: smoke::Input) {
        let Some(state) = self.checks.video_smoke.take() else {
            return;
        };
        let (state, actions) = smoke::next(state, input);
        self.checks.video_smoke = Some(state);
        for action in actions {
            match action {
                smoke::Action::Load(url) => {
                    if let Some(tab) = self.shown() {
                        self.dispatch(wubble_core::model::Command::Navigate { tab, url });
                    }
                }
                smoke::Action::Reload => {
                    if let Some(view) = self.view() {
                        view.reload();
                    }
                }
                smoke::Action::Eval(script) => {
                    if let Some(view) = self.view() {
                        view.eval(&script);
                    }
                }
                smoke::Action::Video(name, value) => {
                    // the overlay of the window the tab is on screen in
                    let tab = self.model.window_tab(self.model.main_window());
                    if let Some(w) = tab.and_then(|t| self.model.window_of(t)) {
                        self.enter(w);
                    }
                    self.video_command(name, value);
                }
                smoke::Action::Setting(key, value) => {
                    self.write_setting(key, value);
                }
                smoke::Action::PipSpike => self.pip_spike(),
                smoke::Action::Pip(name) => self.pip_command(name, 0.0),
                smoke::Action::PointTo(spot) => self.video_smoke_pointer(spot, false),
                smoke::Action::Click(spot) => self.video_smoke_pointer(spot, true),
                smoke::Action::Undo => {
                    if let Some(tab) = self.shown() {
                        self.sponsor_undo(tab);
                    }
                }
                smoke::Action::Key(key) => self.media_key(key),
                smoke::Action::PutBoost(b) => self.boost_put_for_smoke(*b),
                smoke::Action::ToggleBoost(id) => self.toggle_boost(id),
                smoke::Action::Log(line) => println!("{line}"),
                smoke::Action::CheckSaved(path) => {
                    let dir = self
                        .downloads
                        .dir()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    match smoke::check_saved(&path, &dir) {
                        Ok(line) => println!("{line}"),
                        Err(e) => self.fail(e),
                    }
                }
                smoke::Action::Pass(what) => {
                    self.video_passed = Some(what);
                    crate::quit();
                }
                smoke::Action::Fail(reason) => self.fail(reason),
            }
        }
    }

    /// A page event for `--smoke video` and `--smoke boost` (their checks' answers).
    pub(super) fn video_smoke_page(&mut self, event: &PageEvent) {
        if self.checks.video_smoke.is_some() {
            self.video_smoke_step(smoke::Input::Page(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_reach_the_kit_as_scripts() {
        assert_eq!(
            page_script(PageCmd::SetRate(1.5)),
            format!("{KIT_REF}&&{KIT_REF}.cmd('rate',1.5)")
        );
        assert!(page_script(PageCmd::SetMuted(true)).ends_with("cmd('muted',true)"));
        assert!(page_script(PageCmd::Seek(0.5)).ends_with("cmd('seek',0.5)"));
        assert_eq!(
            jumps_script(&[(0, 0.5, 1.5)]),
            format!("{KIT_REF}&&{KIT_REF}.jumps([{{\"from\":0.5,\"id\":0,\"to\":1.5}}])")
        );
        assert_eq!(rate_label(1.5), "1.5\u{d7}");
        assert_eq!(rate_label(1.0), "1\u{d7}");
        assert_eq!(rate_label(0.25), "0.25\u{d7}");
    }

    fn tab_with_a_jump() -> SponsorTab {
        let segments = vec![Segment {
            uuid: "a".into(),
            category: "sponsor".into(),
            from: 1.0,
            to: 5.0,
            video_duration: 0.0,
        }];
        SponsorTab {
            video: "v".into(),
            plan: sponsor::plan(&segments, &ActTable::new(), 60.0),
            segments: Some(segments),
            ..SponsorTab::default()
        }
    }

    #[test]
    fn a_new_document_is_given_the_jumps_again() {
        let mut t = tab_with_a_jump();
        let table = t.unsent(100.0).expect("the first document gets the table");
        assert_eq!(table, vec![(0, 1.0, 5.0)]);
        assert_eq!(t.unsent(100.0), None, "given once");
        // A reload, or the same URL entered again: the new kit starts without jumps.
        assert_eq!(t.unsent(200.0), Some(table.clone()));
        assert_eq!(t.unsent(200.0), None);
        // A late answer of the replaced document changes nothing.
        assert_eq!(t.unsent(100.0), None);
        // The table changes (an undo): the same document gets the new one.
        t.skips.fired(&t.plan, 0, 0);
        t.skips.undo(1);
        assert_eq!(t.unsent(200.0), Some(vec![]));
    }

    #[test]
    fn a_lookup_without_an_answer_is_asked_again_later() {
        let mut t = SponsorTab::default();
        assert!(t.wants_lookup(1_000));
        assert!(!t.wants_lookup(1_001), "one at a time");
        t.lookup_failed(2_000);
        assert!(!t.wants_lookup(2_000 + SPONSOR_RETRY_MS - 1));
        assert!(t.wants_lookup(2_000 + SPONSOR_RETRY_MS));
        // An answer, even "no segments", is not asked again.
        let mut answered = tab_with_a_jump();
        assert!(!answered.wants_lookup(u64::MAX));
    }
}
