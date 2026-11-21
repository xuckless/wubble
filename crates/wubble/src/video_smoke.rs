//! `--smoke video` and `--smoke boost`: the video built-ins and Boosts through the host's real
//! paths, on pages a loopback server serves (`serve`). Bounded by counts of CEF work runs per
//! step and by the states the host reports, never by time. Run by hand (they open a window).
//!
//! `--smoke video` (`video.html`, a 3 s 64 x 48 VP8 clip, `assets/clip.webm`):
//! 1. the kit's snapshot finds the video (its rect and duration are printed);
//! 2. picture-in-picture spike: `document.pictureInPictureEnabled` and whether the video goes
//!    fullscreen as a user gesture, printed as `pip: api=.. fullscreen=..` (not asserted); then
//!    the overlay's button opens the Pip window: the page goes fullscreen, its frames arrive in
//!    the second window and the Main window shows its card; return puts it back in the Main
//!    window (one window again); the clip restarted and opened once more, its close button ends
//!    it and pauses the clip, which return left playing (`smoke: pip ok`);
//! 3. speed 2 from the controls reads back from the page and is remembered for the site: after a
//!    reload (a new document) the page plays at 2 again;
//! 4. loop on, mute toggled;
//! 5. SponsorBlock against the server's fake endpoint: the request carries only the 4-hex prefix
//!    (no video id), no cookie and no referer; the page jumps over the segment and the host shows
//!    the notice; after a reload (a new document of the same video, its kit without jumps) the
//!    new document reports the jump again; undo (paused) goes back before the segment and the jump is withdrawn
//!    (`smoke: sponsor ok`);
//! 6. a media key (Toggle) through the host's path pauses the playing video;
//! 7. the strip through real pointer events at its buttons' places on screen, the clip playing
//!    (looped): a move over the video shows it; the pointer resting on its play button keeps it
//!    past the hide delay; a click pauses, mute works paused, a click plays; a click on the bar
//!    seeks; the speed button's menu sets 2; mute and loop toggle; the frame button saves a frame
//!    (its notice comes up); the collapse button tucks the strip into its tab: the pointer
//!    resting on the tab, then on the video, does not open it (the tab stays while the pointer
//!    is on it, goes a while after it rests on the video, and comes back as the tab on the next
//!    move), and a click on the tab does (`smoke: strip collapse ok`); the pointer moved off the
//!    video hides the strip; back on it, the frame saved once more, the run ends with the strip
//!    up (`smoke: strip ok`);
//! 8. the save control (the media downloader's entry), always on the strip while the downloader
//!    is on: with nothing to save (the clip is under its size floor) it is there and a click
//!    opens a menu saying so; with the floor at 0, on `drm.html`, whose video has ClearKey
//!    MediaKeys attached (EME), it is there and its menu explains the protection; on
//!    `video.html?save` a click (ask-each-time off: the default quality at once) starts a
//!    download through the downloader, which saves the clip, byte for byte, into the smoke's
//!    downloads folder; the clip plays on and the strip stays open (`smoke: save ok`); on
//!    `frame.html?save`, the clip in an iframe the kit's top-document snapshot does not see, the
//!    strip shows over the frame's area (save, collapse and close only) and a click saves too
//!    (`smoke: frame save ok`), the strip up and the pointer on the save control for `--shot`.
//!
//! `--smoke boost`: a Boost stored through the editor's path (`http://127.0.0.1/boost*`: CSS, a
//! tint, a document-start script) colours `boost.html` and sets its title; `other.html` stays as
//! it is; `boost-csp.html` (`style-src 'none'`), reached by the page's own navigation (as a link
//! is), still gets the CSS and its script ran at document start; the Boost turned off, the page
//! reloads as it was.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use std::path::{Path, PathBuf};
use wubble_core::boost::{At, Boost, Tint};

use wubble_core::video::{Save, VideoState};
use wubble_engine::nav::WebEvent;
use wubble_engine::page::PageEvent;

use crate::media_keys::MediaKey;

/// Runs of CEF work one step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 4_000;

/// The video id the loopback page stands for (11 of `[A-Za-z0-9_-]`).
pub const SMOKE_VIDEO: &str = "wubbleSmoke";
/// The fake answer's skip segment, seconds.
pub const JUMP: (f64, f64) = (0.6, 1.8);
/// The colour a Boost gives `boost.html`, as `getComputedStyle` reads it.
pub const BOOSTED_BG: &str = "rgb(1, 2, 3)";

const CLIP: &[u8] = include_bytes!("../assets/clip.webm");

/// The video page; its clip's URL takes the page's query, so each page's clip is a response of
/// its own (not the HTTP cache's) that the downloader's hook sees. The save steps' pages show the
/// video 480 wide, room for the strip with its save control.
fn video_page(query: &str) -> String {
    let width = if query.is_empty() { 320 } else { 480 };
    format!(
        "<!doctype html><title>video</title>\
         <body style=\"margin:0;background:#202020\">\
         <video src=\"/clip.webm?{query}\" muted autoplay playsinline width=\"{width}\" \
         height=\"240\" style=\"display:block;margin:40px\"></video>"
    )
}

/// The video page with ClearKey MediaKeys attached to its video (EME, which Chromium has built
/// in): protected media, as far as the save button goes, though the clip itself is clear.
const DRM_SCRIPT: &str = "<script>navigator.requestMediaKeySystemAccess('org.w3.clearkey',\
    [{initDataTypes:['webm'],videoCapabilities:[{contentType:'video/webm; codecs=\"vp8\"'}]}])\
    .then(a=>a.createMediaKeys()).then(k=>document.querySelector('video').setMediaKeys(k))\
    .catch(e=>console.error('smoke: eme '+e))</script>";
const BOOST_PAGE: &str = "<!doctype html><title>plain</title>\
    <body style=\"background:rgb(250,250,250)\"><p>A page to boost.</p>";

/// The clip from its start, playing, so a pause is the picture-in-picture's.
const RESTART: &str = "(k=>(k.cmd('seek',0),k.cmd('play'),1))(window[Symbol.for('wubble.kit')])";

/// Plays from just before the segment, so the wait is short; muted, since an unmuted video does
/// not play without a user gesture.
const PLAY_BEFORE_JUMP: &str = "(k=>(k.cmd('muted',true),k.cmd('rate',1),k.cmd('seek',0.5),k.cmd('play'),1))\
     (window[Symbol.for('wubble.kit')])";

/// The clip playing from the start, looped and muted, at normal speed: the strip's stage.
const STRIP_PREP: &str = "(k=>(k.cmd('muted',true),k.cmd('loop',true),k.cmd('rate',1),k.cmd('seek',0),\
     k.cmd('play'),1))(window[Symbol.for('wubble.kit')])";

/// How long the pointer rests on the strip before its click: past both of the strip's hide
/// delays, which must not apply while it is there.
const HOLD_MS: u64 = wubble_core::video::STILL_MS + wubble_core::video::LEAVE_MS + 500;

/// Also marks the document, so the check after the reload can tell the new one from it.
const RATE_CHECK: &str =
    "(window.wubbleMark=1,{check:'rate',rate:document.querySelector('video').playbackRate})";
const SITE_CHECK: &str =
    "({check:'site',rate:document.querySelector('video').playbackRate,fresh:!window.wubbleMark})";
const BOOST_CHECK: &str = "({check:'boost',bg:getComputedStyle(document.body).backgroundColor,\
    title:document.title,after:getComputedStyle(document.documentElement,'::after').position,\
    at:String(window.wubbleAt),kit:!!window[Symbol.for('wubble.kit')],\
    css:!!(window[Symbol.for('wubble.kit')]||{}).boostedCss,href:location.href})";

/// What a request to the fake SponsorBlock endpoint looked like: its line and headers, lowercase.
pub type Requests = Arc<Mutex<Vec<String>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    // --smoke video
    Found,
    PipSpike,
    PipOpen,
    PipReturning,
    /// Back in the tab: the clip restarted (it may have ended), waiting for it to play.
    PipRestart,
    PipReopen,
    PipClosing,
    /// Closed: waiting for the clip to read paused part way through (an ended clip is paused
    /// too, so the check is on the time).
    PipPaused,
    Rate,
    RateChecked,
    Reloading,
    SiteLoading,
    SiteRate,
    SiteChecked,
    Loop,
    Mute {
        before: bool,
    },
    Skipping,
    SponsorReloading,
    SponsorLoading,
    SkippingAgain,
    Undone,
    KeyPlaying,
    KeyPaused,
    /// The clip playing again, looped, for the strip's stage.
    StripPrep,
    StripShown,
    /// The pointer rests on the strip's play button since this time (host ms).
    StripHeld {
        since: u64,
    },
    StripPaused,
    StripPausedMute {
        before: bool,
    },
    StripPlaying,
    /// A click on the bar seeks to this time (ms).
    StripSeek {
        to: u64,
    },
    StripSpeeds,
    StripRate,
    StripMute {
        before: bool,
    },
    StripLoopOff,
    StripLoopOn,
    StripFrame {
        serial: i32,
    },
    /// The collapse button clicked: waiting for the tab.
    StripCollapse,
    /// The pointer rests on the tab since this time.
    TabHeld {
        since: u64,
    },
    /// The pointer rests on the video beside the tab: waiting for the tab to go.
    TabVideo,
    /// Moved on the video again: the tab must come back, not the strip.
    TabBack,
    /// The tab clicked: waiting for the strip.
    TabClicked,
    StripLeft,
    StripBack,
    StripLastFrame {
        serial: i32,
    },
    /// The save control pressed with nothing found: waiting for its explaining menu.
    SaveNothingMenu,
    /// The menu dismissed by a click beside the video.
    SaveNothingClosed,
    /// The floor at 0, `drm.html` loading.
    SaveDrmLoading,
    /// Waiting for the downloader to list the clip and the page to attach its MediaKeys.
    SaveDrm,
    /// The pointer on the protected video: waiting for the strip and its save control.
    SaveDrmShown,
    SaveDrmMenu,
    SaveDrmClosed,
    /// `frame.html` (the clip in an iframe the page kit's snapshot does not reach) loading.
    FrameLoading,
    FrameFound,
    FrameShown {
        before: usize,
    },
    FrameClicked {
        before: usize,
    },
    FrameChecking,
    SaveLoading,
    /// Loaded: waiting for the new document's video and its controls on screen (the pointer
    /// finds them there).
    SaveFound,
    /// Waiting for the button.
    SaveShown,
    /// The button clicked: waiting for the download to start, then for its file.
    SaveClicked,
    SaveChecking,
    /// Passed: the window quits.
    Done,
    // --smoke boost
    BoostLoaded,
    BoostSaving,
    BoostReloaded,
    BoostChecked,
    OtherLoaded,
    OtherChecked,
    CspLoaded,
    CspChecked,
    Disabling,
    DisabledReloaded,
    DisabledChecked,
}

#[derive(Clone)]
pub struct VideoSmoke {
    step: Step,
    runs: u32,
    /// The loopback server's base, `http://127.0.0.1:<port>`.
    base: String,
    requests: Requests,
    boost: Option<i64>,
    /// The video's time at the last snapshot: while it moves, a step waiting on playback is
    /// not stuck.
    time: f64,
    /// Downloads from the tab before the iframe page's click.
    frame_runs: usize,
}

impl VideoSmoke {
    pub fn video(base: String, requests: Requests) -> Self {
        Self {
            step: Step::Found,
            runs: 0,
            base,
            requests,
            boost: None,
            time: 0.0,
            frame_runs: 0,
        }
    }

    pub fn boost(base: String, requests: Requests) -> Self {
        Self {
            step: Step::BoostLoaded,
            ..Self::video(base, requests)
        }
    }
}

/// What the smoke reads of the host at a snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    pub video: Option<VideoState>,
    /// The jumps the page was last given.
    pub jumps: usize,
    /// The "skipped" notice is up.
    pub notice: bool,
    /// Jumps the page reported taking since the last snapshot.
    pub fired: usize,
    pub pip: PipSeen,
    /// The host's clock, ms.
    pub now: u64,
    /// The window lays the video's controls over it (its `VideoView` has a size): only then
    /// can the pointer find them.
    pub controls: bool,
    /// The strip is open (not hidden, not tucked away).
    pub strip: bool,
    /// The strip is tucked into its tab, and the tab shows.
    pub collapsed: bool,
    /// Its speed menu is open.
    pub speeds: bool,
    /// A menu of the shell is open (the save control's explanation, a list of qualities).
    pub menu: bool,
    /// The window's notice: its serial and text.
    pub toast: (i32, String),
    /// The strip's save button, as the host derives it from the downloader.
    pub save: Option<Save>,
    /// The downloader lists a stream of the page it can save.
    pub ready: bool,
    /// Downloads the downloader started from the tab.
    pub runs: usize,
    /// The saved file of the tab's latest finished download.
    pub saved: Option<PathBuf>,
}

/// Picture-in-picture as the host has it at a snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PipSeen {
    /// `pip::Pip::On`: the page went fullscreen for it.
    pub on: bool,
    /// The model has a Pip window and its platform window is open.
    pub open: bool,
    /// Frames of the tab (not a stand-in snapshot) in the Pip window at the window's size, and
    /// that size.
    pub framed: Option<(u32, u32)>,
    /// The Main window shows the card for its tab.
    pub card: bool,
    /// Open platform windows.
    pub windows: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum Input<'a> {
    Worked,
    Snapshot(&'a Seen),
    Web(&'a WebEvent),
    Page(&'a PageEvent),
    BoostSaved(i64),
    PipSpiked(&'a str),
}

/// A place on the strip or around it, found on screen when the event is sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    /// The middle of the video's lower half (under the strip).
    Video,
    /// A little left of `Video`: a move that is a move.
    VideoNear,
    /// Beside the video, on the page.
    Away,
    /// The centre of the strip's element with this id (`VideoControls::<id>`).
    Button(&'static str),
    /// The progress bar at this fraction.
    Bar(f32),
    /// The speed menu's row (`video::RATES` index).
    Speed(usize),
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Load(String),
    Reload,
    Eval(String),
    /// A control of the video overlay (`Host::video_command`).
    Video(&'static str, f32),
    Setting(&'static str, &'static str),
    PipSpike,
    /// A button of the Pip window's controls (`Host::pip_command`).
    Pip(&'static str),
    /// The pointer moves to the spot (a window event, as winit's).
    PointTo(Spot),
    /// A left click at the spot: moved there, pressed, released.
    Click(Spot),
    Undo,
    Key(MediaKey),
    PutBoost(Box<Boost>),
    ToggleBoost(i64),
    Log(String),
    /// Check the saved file against the clip (`check_saved`).
    CheckSaved(PathBuf),
    Pass(String),
    Fail(String),
}

/// A tagged check's answer.
fn check(event: &PageEvent, tag: &str) -> Option<Value> {
    let PageEvent::EvalResult {
        result: Ok(text), ..
    } = event
    else {
        return None;
    };
    let v: Value = serde_json::from_str(text).ok()?;
    (v.get("check").and_then(Value::as_str) == Some(tag)).then_some(v)
}

fn loaded(event: &WebEvent) -> bool {
    matches!(event, WebEvent::LoadingChanged { loading: false, .. })
}

/// The Boost `--smoke boost` stores.
pub fn smoke_boost() -> Boost {
    Boost {
        name: "smoke".into(),
        css: "body{background:rgb(1,2,3)!important}".into(),
        js: "window.wubbleAt = document.readyState;\
             document.addEventListener('DOMContentLoaded', function () { document.title = 'boosted'; });"
            .into(),
        js_at: At::Start,
        tint: Some(Tint {
            rgb: 0x10_20_30,
            strength: 10,
        }),
        ..Boost::for_host("http://127.0.0.1/boost*")
    }
}

/// Whether the fake endpoint saw exactly one lookup, by prefix only, with no cookie or referer.
pub fn private_lookup(requests: &[String]) -> Result<(), String> {
    let prefix = wubble_core::sponsor::hash_prefix(SMOKE_VIDEO);
    let [request] = requests else {
        return Err(format!("{} lookups, not one", requests.len()));
    };
    let line = request.lines().next().unwrap_or_default();
    if !line.starts_with(&format!("get /api/skipsegments/{prefix}?")) {
        return Err(format!("the lookup was {line:?}"));
    }
    if request.contains(&SMOKE_VIDEO.to_lowercase()) {
        return Err("the lookup named the video".into());
    }
    if request
        .lines()
        .any(|h| h.starts_with("cookie:") || h.starts_with("referer:"))
    {
        return Err(format!("the lookup sent {request:?}"));
    }
    Ok(())
}

pub fn next(mut s: VideoSmoke, input: Input) -> (VideoSmoke, Vec<Action>) {
    use {Input::*, Step::*};
    let fail = |s: VideoSmoke, why: String| (s, vec![Action::Fail(why)]);
    if let Worked = input {
        s.runs += 1;
        if s.runs > MAX_STEP_RUNS {
            let step = s.step;
            return fail(
                s,
                format!("stuck at {step:?} for {MAX_STEP_RUNS} runs of CEF work"),
            );
        }
        return (s, vec![]);
    }
    let video = match input {
        Snapshot(seen) => seen.video.clone(),
        _ => None,
    };
    if let Some(v) = &video
        && v.time != s.time
    {
        s.time = v.time;
        s.runs = 0;
    }
    let to = |mut s: VideoSmoke, step: Step, actions: Vec<Action>| {
        s.step = step;
        s.runs = 0;
        (s, actions)
    };
    let url = |s: &VideoSmoke, page: &str| format!("{}/{page}", s.base);
    match (s.step, input) {
        // ----- video
        (Found, Snapshot(_))
            if video
                .as_ref()
                .is_some_and(|v| v.duration.is_finite() && v.duration > 1.0 && v.rect.w > 0.0) =>
        {
            let v = video.unwrap_or_default();
            let line = format!(
                "smoke: video {}x{} at {},{} duration {:.1}s",
                v.rect.w, v.rect.h, v.rect.x, v.rect.y, v.duration
            );
            to(s, PipSpike, vec![Action::Log(line), Action::PipSpike])
        }
        (PipSpike, PipSpiked(text)) => to(
            s,
            PipOpen,
            vec![
                Action::Log(format!("pip: {text}")),
                Action::Video("pip", 0.0),
            ],
        ),
        (PipOpen, Snapshot(seen))
            if seen.pip.on && seen.pip.open && seen.pip.card && seen.pip.windows == 2 =>
        {
            let Some((w, h)) = seen.pip.framed else {
                return (s, vec![]);
            };
            let line = format!("smoke: pip window {w}x{h}");
            to(
                s,
                PipReturning,
                vec![Action::Log(line), Action::Pip("return")],
            )
        }
        (PipReturning, Snapshot(seen))
            if !seen.pip.on && !seen.pip.open && !seen.pip.card && seen.pip.windows == 1 =>
        {
            to(s, PipRestart, vec![Action::Eval(RESTART.into())])
        }
        // return left the clip as it was, and it plays on
        (PipRestart, Snapshot(_))
            if video
                .as_ref()
                .is_some_and(|v| !v.paused && v.time < v.duration / 2.0) =>
        {
            to(s, PipReopen, vec![Action::Video("pip", 0.0)])
        }
        (PipReopen, Snapshot(seen)) if seen.pip.on && seen.pip.framed.is_some() => {
            to(s, PipClosing, vec![Action::Pip("close")])
        }
        (PipClosing, Snapshot(seen)) if !seen.pip.on && !seen.pip.open && seen.pip.windows == 1 => {
            to(s, PipPaused, vec![])
        }
        // closing pauses it (as Chrome's does), before the clip ends on its own
        (PipPaused, Snapshot(_))
            if video
                .as_ref()
                .is_some_and(|v| v.paused && v.time < v.duration - 0.5) =>
        {
            to(
                s,
                Rate,
                vec![
                    Action::Log("smoke: pip ok".into()),
                    // RATES[7] is 2.
                    Action::Video("rate", 7.0),
                ],
            )
        }
        (Rate, Snapshot(_)) if video.as_ref().is_some_and(|v| v.rate == 2.0) => {
            to(s, RateChecked, vec![Action::Eval(RATE_CHECK.into())])
        }
        (RateChecked, Page(e)) if check(e, "rate").is_some() => {
            let v = check(e, "rate").unwrap_or_default();
            if v["rate"] != 2 {
                return fail(s, format!("the page plays at {}", v["rate"]));
            }
            to(s, Reloading, vec![Action::Reload])
        }
        (Reloading, Web(WebEvent::LoadStarted)) => to(s, SiteLoading, vec![]),
        // An eval sent while the old document goes away is never answered: wait for the new one.
        (SiteLoading, Web(e)) if loaded(e) => to(s, SiteRate, vec![]),
        (SiteRate, Snapshot(_)) if video.as_ref().is_some_and(|v| v.rate == 2.0) => {
            to(s, SiteChecked, vec![Action::Eval(SITE_CHECK.into())])
        }
        (SiteChecked, Page(e)) if check(e, "site").is_some() => {
            let v = check(e, "site").unwrap_or_default();
            if v["fresh"] != true {
                // An answer of the old document: ask the new one.
                return (s, vec![Action::Eval(SITE_CHECK.into())]);
            }
            if v["rate"] != 2 {
                return fail(s, format!("the site's speed was not applied: {v}"));
            }
            to(s, Loop, vec![Action::Video("loop", 0.0)])
        }
        (Loop, Snapshot(_)) if video.as_ref().is_some_and(|v| v.looped) => {
            let before = video.is_some_and(|v| v.muted);
            to(s, Mute { before }, vec![Action::Video("mute", 0.0)])
        }
        (Mute { before }, Snapshot(_)) if video.as_ref().is_some_and(|v| v.muted != before) => to(
            s,
            Skipping,
            vec![
                Action::Setting("sponsor.enabled", "1"),
                Action::Eval(PLAY_BEFORE_JUMP.into()),
            ],
        ),
        (Skipping, Snapshot(seen)) if seen.notice => {
            let requests = s.requests.lock().map(|r| r.clone()).unwrap_or_default();
            if let Err(e) = private_lookup(&requests) {
                return fail(s, e);
            }
            to(s, SponsorReloading, vec![Action::Reload])
        }
        (SponsorReloading, Web(WebEvent::LoadStarted)) => to(s, SponsorLoading, vec![]),
        (SponsorLoading, Web(e)) if loaded(e) => to(
            s,
            SkippingAgain,
            vec![Action::Eval(PLAY_BEFORE_JUMP.into())],
        ),
        // The first notice may still be up: the new document's own report of the jump counts.
        (SkippingAgain, Snapshot(seen)) if seen.fired > 0 => {
            let pause = "window[Symbol.for('wubble.kit')].cmd('pause')";
            to(s, Undone, vec![Action::Eval(pause.into()), Action::Undo])
        }
        (Undone, Snapshot(seen))
            if seen.jumps == 0
                && video
                    .as_ref()
                    .is_some_and(|v| v.paused && v.time < JUMP.1 - 0.1) =>
        {
            let play = "window[Symbol.for('wubble.kit')].cmd('play')";
            to(
                s,
                KeyPlaying,
                vec![
                    Action::Log("smoke: sponsor ok".into()),
                    Action::Eval(play.into()),
                ],
            )
        }
        (KeyPlaying, Snapshot(_)) if video.as_ref().is_some_and(|v| !v.paused) => {
            to(s, KeyPaused, vec![Action::Key(MediaKey::Toggle)])
        }
        (KeyPaused, Snapshot(_)) if video.as_ref().is_some_and(|v| v.paused) => to(
            s,
            StripPrep,
            vec![
                Action::Setting("screenshot.output", "file"),
                Action::Eval(STRIP_PREP.into()),
            ],
        ),

        // ----- the strip, through the pointer
        (StripPrep, Snapshot(seen))
            if seen.controls && video.as_ref().is_some_and(|v| !v.paused && v.looped) =>
        {
            to(s, StripShown, vec![Action::PointTo(Spot::Video)])
        }
        (StripShown, Snapshot(seen)) if seen.strip => to(
            s,
            StripHeld { since: seen.now },
            vec![Action::PointTo(Spot::Button("play"))],
        ),
        (StripHeld { .. }, Snapshot(seen)) if !seen.strip => {
            fail(s, "the strip went while the pointer was on it".into())
        }
        (StripHeld { since }, Snapshot(seen)) if seen.now > since + HOLD_MS => {
            to(s, StripPaused, vec![Action::Click(Spot::Button("play"))])
        }
        (StripPaused, Snapshot(seen)) if video.as_ref().is_some_and(|v| v.paused) => {
            let before = video.is_some_and(|v| v.muted);
            if !seen.strip {
                return fail(s, "the strip went after its pause click".into());
            }
            to(
                s,
                StripPausedMute { before },
                vec![Action::Click(Spot::Button("mute"))],
            )
        }
        (StripPausedMute { before }, Snapshot(_))
            if video
                .as_ref()
                .is_some_and(|v| v.paused && v.muted != before) =>
        {
            to(s, StripPlaying, vec![Action::Click(Spot::Button("play"))])
        }
        (StripPlaying, Snapshot(_)) if video.as_ref().is_some_and(|v| !v.paused) => {
            let v = video.unwrap_or_default();
            // far from where it plays, so the jump shows
            let fraction = if v.time < v.duration / 2.0 { 0.7 } else { 0.1 };
            let to_time = (f64::from(fraction) * v.duration * 1000.0) as u64;
            to(
                s,
                StripSeek { to: to_time },
                vec![Action::Click(Spot::Bar(fraction))],
            )
        }
        (StripSeek { to: ms }, Snapshot(_))
            if let t = ms as f64 / 1000.0
                && video
                    .as_ref()
                    .is_some_and(|v| !v.paused && v.time >= t - 0.15 && v.time < t + 0.6) =>
        {
            to(
                s,
                StripSpeeds,
                vec![Action::Click(Spot::Button("rate-button"))],
            )
        }
        (StripSpeeds, Snapshot(seen)) if seen.speeds => {
            // RATES[7] is 2.
            to(s, StripRate, vec![Action::Click(Spot::Speed(7))])
        }
        (StripRate, Snapshot(_)) if video.as_ref().is_some_and(|v| v.rate == 2.0) => {
            let before = video.is_some_and(|v| v.muted);
            to(
                s,
                StripMute { before },
                vec![Action::Click(Spot::Button("mute"))],
            )
        }
        (StripMute { before }, Snapshot(_))
            if video.as_ref().is_some_and(|v| v.muted != before) =>
        {
            to(s, StripLoopOff, vec![Action::Click(Spot::Button("loop"))])
        }
        (StripLoopOff, Snapshot(_)) if video.as_ref().is_some_and(|v| !v.looped) => {
            to(s, StripLoopOn, vec![Action::Click(Spot::Button("loop"))])
        }
        (StripLoopOn, Snapshot(seen)) if video.as_ref().is_some_and(|v| v.looped) => {
            let serial = seen.toast.0;
            to(
                s,
                StripFrame { serial },
                vec![Action::Click(Spot::Button("frame"))],
            )
        }
        (StripFrame { serial }, Snapshot(seen)) if seen.toast.0 != serial => {
            let line = format!("smoke: strip frame notice {:?}", seen.toast.1);
            to(
                s,
                StripCollapse,
                vec![Action::Log(line), Action::Click(Spot::Button("collapse"))],
            )
        }
        (StripCollapse, Snapshot(seen)) if seen.collapsed && !seen.strip => to(
            s,
            TabHeld { since: seen.now },
            vec![Action::PointTo(Spot::Button("tab"))],
        ),
        (TabHeld { .. } | TabVideo | TabBack, Snapshot(seen)) if seen.strip => {
            fail(s, "a hover opened the collapsed strip".into())
        }
        (TabHeld { .. }, Snapshot(seen)) if !seen.collapsed => {
            fail(s, "the tab went while the pointer was on it".into())
        }
        (TabHeld { since }, Snapshot(seen)) if seen.now > since + HOLD_MS => {
            to(s, TabVideo, vec![Action::PointTo(Spot::Video)])
        }
        (TabVideo, Snapshot(seen)) if !seen.collapsed => {
            to(s, TabBack, vec![Action::PointTo(Spot::VideoNear)])
        }
        (TabBack, Snapshot(seen)) if seen.collapsed => {
            to(s, TabClicked, vec![Action::Click(Spot::Button("tab"))])
        }
        (TabClicked, Snapshot(seen)) if seen.strip && !seen.collapsed => to(
            s,
            StripLeft,
            vec![
                Action::Log("smoke: strip collapse ok".into()),
                Action::PointTo(Spot::Away),
            ],
        ),
        (StripLeft, Snapshot(seen)) if !seen.strip => {
            to(s, StripBack, vec![Action::PointTo(Spot::Video)])
        }
        (StripBack, Snapshot(seen)) if seen.strip => {
            let serial = seen.toast.0;
            to(
                s,
                StripLastFrame { serial },
                vec![
                    Action::PointTo(Spot::Button("frame")),
                    Action::Click(Spot::Button("frame")),
                ],
            )
        }
        (StripLastFrame { serial }, Snapshot(seen)) if seen.toast.0 != serial && seen.strip => {
            // The clip is under the downloader's size floor: it offers nothing, and the control
            // is there anyway, saying so when pressed.
            if seen.save != Some(Save::Offered) || seen.ready {
                return fail(
                    s,
                    format!("the save control with nothing to save: {:?}", seen.save),
                );
            }
            to(
                s,
                SaveNothingMenu,
                vec![
                    Action::Log("smoke: strip ok".into()),
                    Action::PointTo(Spot::Button("download")),
                    Action::Click(Spot::Button("download")),
                ],
            )
        }
        (SaveNothingMenu, Snapshot(seen)) if seen.menu => to(
            s,
            SaveNothingClosed,
            vec![
                Action::Log("smoke: save control explains nothing found".into()),
                Action::Click(Spot::Away),
            ],
        ),
        (SaveNothingClosed, Snapshot(seen)) if !seen.menu => {
            let drm = url(&s, "drm.html");
            to(
                s,
                SaveDrmLoading,
                vec![
                    Action::Setting("media.min_bytes", "0"),
                    Action::Setting("media.ask_each_time", "0"),
                    Action::Load(drm),
                ],
            )
        }
        (SaveDrmLoading, Web(e)) if loaded(e) => to(s, SaveDrm, vec![]),
        (SaveDrm, Snapshot(seen))
            if seen.ready && seen.controls && video.as_ref().is_some_and(|v| v.drm) =>
        {
            to(s, SaveDrmShown, vec![Action::PointTo(Spot::Video)])
        }
        // Protected media keeps the control (a missing one reads as "never saw my video"); it
        // says why nothing is offered.
        (SaveDrmShown, Snapshot(seen)) if seen.strip => {
            if seen.save != Some(Save::Offered) {
                return fail(s, format!("protected video, save control {:?}", seen.save));
            }
            to(
                s,
                SaveDrmMenu,
                vec![
                    Action::PointTo(Spot::Button("download")),
                    Action::Click(Spot::Button("download")),
                ],
            )
        }
        (SaveDrmMenu, Snapshot(seen)) if seen.menu => to(
            s,
            SaveDrmClosed,
            vec![
                Action::Log("smoke: save control explains protected media".into()),
                Action::Click(Spot::Away),
            ],
        ),
        (SaveDrmClosed, Snapshot(seen)) if !seen.menu => {
            let page = url(&s, "video.html?save");
            to(s, SaveLoading, vec![Action::Load(page)])
        }
        // A load ends before the new document's kit reports its video: until then the window
        // has no controls over it for the pointer to find.
        (SaveLoading, Web(e)) if loaded(e) => to(s, SaveFound, vec![]),
        (SaveFound, Snapshot(seen))
            if seen.controls && video.as_ref().is_some_and(|v| !v.drm && !v.paused) =>
        {
            to(s, SaveShown, vec![Action::PointTo(Spot::Video)])
        }
        (SaveShown, Snapshot(seen))
            if seen.strip
                && seen.save == Some(Save::Offered)
                && video.as_ref().is_some_and(|v| !v.paused) =>
        {
            to(
                s,
                SaveClicked,
                vec![
                    Action::PointTo(Spot::Button("download")),
                    Action::Click(Spot::Button("download")),
                ],
            )
        }
        (SaveClicked | SaveChecking, Snapshot(seen)) if !seen.strip || seen.collapsed => {
            fail(s, "the save click took the strip away".into())
        }
        (SaveClicked | SaveChecking, Snapshot(_)) if video.as_ref().is_some_and(|v| v.paused) => {
            fail(s, "the save click paused the video".into())
        }
        (SaveClicked, Snapshot(seen)) if seen.runs > 0 => match &seen.saved {
            Some(path) if seen.save == Some(Save::Saved) => {
                let line = format!("smoke: save started {} download(s)", seen.runs);
                let path = path.clone();
                to(
                    s,
                    SaveChecking,
                    vec![Action::Log(line), Action::CheckSaved(path)],
                )
            }
            _ => (s, vec![]),
        },
        (SaveChecking, Snapshot(seen)) if seen.runs != 1 => {
            fail(s, format!("one click, {} downloads", seen.runs))
        }
        (SaveChecking, Snapshot(seen)) => {
            let page = url(&s, "frame.html?save");
            let before = seen.runs;
            let mut s = to(
                s,
                FrameLoading,
                vec![Action::Log("smoke: save ok".into()), Action::Load(page)],
            );
            s.0.frame_runs = before;
            s
        }
        // A player in an iframe (most embedded players): the kit's snapshot of the top document
        // has no video, so the strip is the save control over the frame's area.
        (FrameLoading, Web(e)) if loaded(e) => to(s, FrameFound, vec![]),
        (FrameFound, Snapshot(seen))
            if seen.controls && seen.ready && video.as_ref().is_some_and(|v| v.frame) =>
        {
            let before = s.frame_runs;
            to(s, FrameShown { before }, vec![Action::PointTo(Spot::Video)])
        }
        (FrameShown { before }, Snapshot(seen)) if seen.strip => {
            if seen.save != Some(Save::Offered) {
                return fail(
                    s,
                    format!("the frame's strip, save control {:?}", seen.save),
                );
            }
            to(
                s,
                FrameClicked { before },
                vec![
                    Action::PointTo(Spot::Button("download")),
                    Action::Click(Spot::Button("download")),
                ],
            )
        }
        (FrameClicked { before }, Snapshot(seen)) if seen.runs > before => match &seen.saved {
            Some(path) if seen.save == Some(Save::Saved) => {
                let path = path.clone();
                to(
                    s,
                    FrameChecking,
                    vec![
                        Action::Log("smoke: save from a frame started".into()),
                        Action::CheckSaved(path),
                    ],
                )
            }
            _ => (s, vec![]),
        },
        (FrameChecking, Snapshot(_)) => to(
            s,
            Done,
            vec![
                Action::Log("smoke: frame save ok".into()),
                Action::Pass("video".into()),
            ],
        ),

        // ----- boost
        (BoostLoaded, Web(e)) if loaded(e) => to(
            s,
            BoostSaving,
            vec![Action::PutBoost(Box::new(smoke_boost()))],
        ),
        (BoostSaving, BoostSaved(id)) => {
            s.boost = Some(id);
            // Storing it reloads the page.
            to(s, BoostReloaded, vec![])
        }
        (BoostReloaded, Web(e)) if loaded(e) => {
            to(s, BoostChecked, vec![Action::Eval(BOOST_CHECK.into())])
        }
        (BoostChecked, Page(e)) if check(e, "boost").is_some() => {
            let v = check(e, "boost").unwrap_or_default();
            if v["bg"] != BOOSTED_BG || v["title"] != "boosted" || v["after"] != "fixed" {
                return fail(s, format!("the Boost did not apply: {v}"));
            }
            let line = format!("smoke: boost ran at readyState={}", v["at"]);
            let other = url(&s, "other.html");
            to(s, OtherLoaded, vec![Action::Log(line), Action::Load(other)])
        }
        (OtherLoaded, Web(e)) if loaded(e) => {
            to(s, OtherChecked, vec![Action::Eval(BOOST_CHECK.into())])
        }
        (OtherChecked, Page(e)) if check(e, "boost").is_some() => {
            let v = check(e, "boost").unwrap_or_default();
            if v["bg"] == BOOSTED_BG || v["title"] == "boosted" {
                return fail(s, format!("a page the Boost does not match changed: {v}"));
            }
            // The page navigates itself, as a link click does: the host hears of it only once
            // it committed.
            let go = format!("location.href = {:?}", url(&s, "boost-csp.html"));
            to(s, CspLoaded, vec![Action::Eval(go)])
        }
        (CspLoaded, Web(e)) if loaded(e) => {
            to(s, CspChecked, vec![Action::Eval(BOOST_CHECK.into())])
        }
        (CspChecked, Page(e)) if check(e, "boost").is_some() => {
            let v = check(e, "boost").unwrap_or_default();
            if v["bg"] != BOOSTED_BG {
                return fail(s, format!("style-src 'none' kept the Boost's CSS out: {v}"));
            }
            if v["at"] != "loading" {
                return fail(
                    s,
                    format!("a page's own navigation ran the Boost late: {v}"),
                );
            }
            match s.boost {
                Some(id) => to(s, Disabling, vec![Action::ToggleBoost(id)]),
                None => fail(s, "no Boost id".into()),
            }
        }
        (Disabling, BoostSaved(_)) => to(s, DisabledReloaded, vec![]),
        (DisabledReloaded, Web(e)) if loaded(e) => {
            to(s, DisabledChecked, vec![Action::Eval(BOOST_CHECK.into())])
        }
        (DisabledChecked, Page(e)) if check(e, "boost").is_some() => {
            let v = check(e, "boost").unwrap_or_default();
            if v["bg"] == BOOSTED_BG {
                return fail(s, format!("the Boost stayed after it was turned off: {v}"));
            }
            to(s, DisabledChecked, vec![Action::Pass("boost".into())])
        }
        _ => (s, vec![]),
    }
}

/// The saved file is the clip, byte for byte, and lies in `dir` (the smoke's downloads folder).
pub fn check_saved(path: &Path, dir: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes != CLIP {
        return Err(format!(
            "{} holds {} bytes, not the clip's {}",
            path.display(),
            bytes.len(),
            CLIP.len()
        ));
    }
    if path.parent() != Some(dir) {
        return Err(format!("{} is not in {}", path.display(), dir.display()));
    }
    Ok(format!("smoke: save wrote {}", path.display()))
}

/// Serves the smoke's pages, the clip and the fake SponsorBlock endpoint on `127.0.0.1` at a free
/// port, from a thread that ends with the process. Loopback only: nothing is fetched from
/// outside.
pub fn serve() -> std::io::Result<(u16, Requests)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let requests = Requests::default();
    let log = Arc::clone(&requests);
    std::thread::Builder::new()
        .name("media-smoke-http".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Err(e) = answer(stream, &log) {
                    eprintln!("smoke: media page server: {e}");
                }
            }
        })?;
    Ok((port, requests))
}

/// The fake endpoint's answer: our video and another under the same prefix.
fn segments_answer() -> String {
    serde_json::json!([
        {
            "videoID": "zzzzzzzzzzz",
            "segments": [{"segment": [0.0, 2.0], "UUID": "other", "category": "sponsor",
                          "actionType": "skip", "videoDuration": 3.0}]
        },
        {
            "videoID": SMOKE_VIDEO,
            "segments": [{"segment": [JUMP.0, JUMP.1], "UUID": "smoke", "category": "sponsor",
                          "actionType": "skip", "videoDuration": 3.0}]
        }
    ])
    .to_string()
}

fn answer(mut stream: TcpStream, log: &Requests) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut head = String::new();
    reader.read_line(&mut head)?;
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        head.push_str(&header);
    }
    let (kind, body, extra): (&str, Vec<u8>, &str) = match path.split('?').next().unwrap_or("") {
        "/video.html" => {
            let query = path.split_once('?').map_or("", |(_, q)| q);
            ("text/html; charset=utf-8", video_page(query).into(), "")
        }
        "/frame.html" => (
            "text/html; charset=utf-8",
            "<!doctype html><title>frame</title><body style=\"margin:0;background:#202020\">\
             <iframe src=\"/video.html?save\" width=\"560\" height=\"330\" \
             style=\"border:0;display:block;margin:20px\"></iframe>"
                .into(),
            "",
        ),
        "/drm.html" => (
            "text/html; charset=utf-8",
            (video_page("drm") + DRM_SCRIPT).into(),
            "",
        ),
        "/clip.webm" => ("video/webm", CLIP.to_vec(), ""),
        "/boost.html" | "/other.html" => ("text/html; charset=utf-8", BOOST_PAGE.into(), ""),
        "/boost-csp.html" => (
            "text/html; charset=utf-8",
            BOOST_PAGE.into(),
            "Content-Security-Policy: style-src 'none'; script-src 'none'\r\n",
        ),
        p if p.starts_with("/api/skipSegments/") => {
            if let Ok(mut log) = log.lock() {
                log.push(head.replace("\r\n", "\n").to_lowercase());
            }
            ("application/json", segments_answer().into_bytes(), "")
        }
        _ => {
            return stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(video: Option<VideoState>) -> Seen {
        Seen {
            video,
            ..Seen::default()
        }
    }

    fn playing() -> VideoState {
        VideoState {
            rect: wubble_core::video::Rect {
                x: 40.0,
                y: 40.0,
                w: 320.0,
                h: 240.0,
            },
            time: 0.2,
            duration: 3.0,
            rate: 1.0,
            ..VideoState::default()
        }
    }

    fn answer(json: &str) -> PageEvent {
        PageEvent::EvalResult {
            id: wubble_engine::cdp::EvalId(1),
            result: Ok(json.into()),
        }
    }

    #[test]
    fn the_video_smoke_walks_its_steps() {
        let s = VideoSmoke::video("http://127.0.0.1:1".into(), Requests::default());
        let (s, a) = next(s, Input::Snapshot(&seen(None)));
        assert!(a.is_empty());
        let (s, a) = next(s, Input::Snapshot(&seen(Some(playing()))));
        assert_eq!(a[1], Action::PipSpike);
        let (s, a) = next(s, Input::PipSpiked("api=false fullscreen=ok"));
        assert_eq!(a[1], Action::Video("pip", 0.0));
        let open = PipSeen {
            on: true,
            open: true,
            framed: None,
            card: true,
            windows: 2,
        };
        let pip = |p: &PipSeen| Seen {
            pip: p.clone(),
            ..seen(Some(playing()))
        };
        // Frames first.
        let (s, a) = next(s, Input::Snapshot(&pip(&open)));
        assert!(a.is_empty());
        let framed = PipSeen {
            framed: Some((400, 225)),
            ..open.clone()
        };
        let (s, a) = next(s, Input::Snapshot(&pip(&framed)));
        assert_eq!(a[1], Action::Pip("return"));
        let back = PipSeen {
            windows: 1,
            ..PipSeen::default()
        };
        let (s, a) = next(s, Input::Snapshot(&pip(&back)));
        assert_eq!(a, vec![Action::Eval(RESTART.into())]);
        // Waits for the clip to play, then opens again.
        let (s, a) = next(
            s,
            Input::Snapshot(&Seen {
                video: Some(VideoState {
                    paused: true,
                    ..playing()
                }),
                ..pip(&back)
            }),
        );
        assert!(a.is_empty());
        let (s, a) = next(s, Input::Snapshot(&pip(&back)));
        assert_eq!(a, vec![Action::Video("pip", 0.0)]);
        let (s, a) = next(s, Input::Snapshot(&pip(&framed)));
        assert_eq!(a, vec![Action::Pip("close")]);
        // Closed, still reading playing: waits for the pause.
        let (s, a) = next(s, Input::Snapshot(&pip(&back)));
        assert!(a.is_empty());
        // Paused at its end is not the picture-in-picture's doing.
        let ended = VideoState {
            paused: true,
            time: 3.0,
            ..playing()
        };
        let (s, a) = next(
            s,
            Input::Snapshot(&Seen {
                video: Some(ended),
                ..pip(&back)
            }),
        );
        assert!(a.is_empty());
        let paused = VideoState {
            paused: true,
            time: 1.0,
            ..playing()
        };
        let (s, a) = next(
            s,
            Input::Snapshot(&Seen {
                video: Some(paused),
                ..pip(&back)
            }),
        );
        assert_eq!(a[1], Action::Video("rate", 7.0));
        let fast = VideoState {
            rate: 2.0,
            ..playing()
        };
        let (s, a) = next(s, Input::Snapshot(&seen(Some(fast.clone()))));
        assert_eq!(a, vec![Action::Eval(RATE_CHECK.into())]);
        let (s, a) = next(s, Input::Page(&answer(r#"{"check":"rate","rate":2}"#)));
        assert_eq!(a, vec![Action::Reload]);
        let (s, _) = next(s, Input::Web(&WebEvent::LoadStarted));
        let done = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        let (s, _) = next(s, Input::Web(&done));
        let (s, _) = next(s, Input::Snapshot(&seen(Some(fast))));
        // The old document's answer is asked again, the new one's passes.
        let (s, a) = next(
            s,
            Input::Page(&answer(r#"{"check":"site","rate":2,"fresh":false}"#)),
        );
        assert_eq!(a, vec![Action::Eval(SITE_CHECK.into())]);
        let (s, a) = next(
            s,
            Input::Page(&answer(r#"{"check":"site","rate":2,"fresh":true}"#)),
        );
        assert_eq!(a, vec![Action::Video("loop", 0.0)]);
        assert_eq!(s.step, Step::Loop);
    }

    #[test]
    fn the_save_steps_expect_the_control_everywhere_the_strip_shows() {
        let done = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        let mut s = VideoSmoke::video("http://127.0.0.1:1".into(), Requests::default());
        s.step = Step::StripLastFrame { serial: 0 };
        let strip = |save, ready| Seen {
            controls: true,
            strip: true,
            toast: (1, String::new()),
            save: Some(save),
            ready,
            ..seen(Some(playing()))
        };
        let menu = |m: &Seen| Seen {
            menu: true,
            ..m.clone()
        };
        // No control fails, whatever the downloader found; so does a downloader that found
        // something where nothing is expected.
        let (_, a) = next(s.clone(), Input::Snapshot(&strip(Save::Absent, false)));
        assert!(matches!(&a[..], [Action::Fail(_)]));
        let (_, a) = next(s.clone(), Input::Snapshot(&strip(Save::Offered, true)));
        assert!(matches!(&a[..], [Action::Fail(_)]));
        // Nothing found: the control is pressed and its menu must come up.
        let (s, a) = next(s, Input::Snapshot(&strip(Save::Offered, false)));
        assert_eq!(a.last(), Some(&Action::Click(Spot::Button("download"))));
        let (s, a) = next(s, Input::Snapshot(&strip(Save::Offered, false)));
        assert!(a.is_empty(), "no menu yet");
        let (s, a) = next(s, Input::Snapshot(&menu(&strip(Save::Offered, false))));
        assert_eq!(a.last(), Some(&Action::Click(Spot::Away)));
        let (s, a) = next(s, Input::Snapshot(&menu(&strip(Save::Offered, false))));
        assert!(a.is_empty(), "waits for the menu to close");
        let (s, a) = next(s, Input::Snapshot(&strip(Save::Offered, false)));
        assert_eq!(
            a.last(),
            Some(&Action::Load("http://127.0.0.1:1/drm.html".into()))
        );
        let (s, _) = next(s, Input::Web(&done));
        // Protected: waits for the downloader's list and the MediaKeys, then the control must
        // be there and explain.
        let drm = Seen {
            video: Some(VideoState {
                drm: true,
                ..playing()
            }),
            ..strip(Save::Offered, true)
        };
        let (_, a) = next(s.clone(), Input::Snapshot(&strip(Save::Offered, true)));
        assert!(a.is_empty(), "MediaKeys not attached yet");
        let (s, a) = next(s, Input::Snapshot(&drm));
        assert_eq!(a, vec![Action::PointTo(Spot::Video)]);
        let (_, a) = next(
            s.clone(),
            Input::Snapshot(&Seen {
                save: Some(Save::Absent),
                ..drm.clone()
            }),
        );
        assert!(matches!(&a[..], [Action::Fail(_)]));
        let (s, a) = next(s, Input::Snapshot(&drm));
        assert_eq!(a.last(), Some(&Action::Click(Spot::Button("download"))));
        let (s, a) = next(s, Input::Snapshot(&menu(&drm)));
        assert_eq!(a.last(), Some(&Action::Click(Spot::Away)));
        let (s, a) = next(s, Input::Snapshot(&drm));
        assert_eq!(
            a.last(),
            Some(&Action::Load("http://127.0.0.1:1/video.html?save".into()))
        );
        // Loaded: the pointer waits for the new document's video under its controls (the old
        // one's, or a video the window has no controls over yet, is no place to point at).
        let (s, a) = next(s, Input::Web(&done));
        assert!(
            a.is_empty(),
            "the load ends before the kit reports the video"
        );
        let (s, a) = next(s, Input::Snapshot(&drm));
        assert!(a.is_empty(), "the old document's video");
        let bare = Seen {
            controls: false,
            ..strip(Save::Offered, true)
        };
        let (s, a) = next(s, Input::Snapshot(&bare));
        assert!(a.is_empty(), "no controls on screen yet");
        let (s, a) = next(s, Input::Snapshot(&strip(Save::Offered, true)));
        assert_eq!(a, vec![Action::PointTo(Spot::Video)]);
        let (s, a) = next(s, Input::Snapshot(&strip(Save::Offered, true)));
        assert_eq!(a.last(), Some(&Action::Click(Spot::Button("download"))));
        // The click must leave the video playing and the strip open.
        let paused = Seen {
            video: Some(VideoState {
                paused: true,
                ..playing()
            }),
            ..strip(Save::Offered, true)
        };
        let (_, a) = next(s.clone(), Input::Snapshot(&paused));
        assert!(matches!(&a[..], [Action::Fail(_)]));
        let saving = Seen {
            runs: 1,
            ..strip(Save::Saving(Some(0.5)), true)
        };
        let (s, a) = next(s, Input::Snapshot(&saving));
        assert!(a.is_empty());
        let saved = Seen {
            runs: 1,
            saved: Some("/d/video.webm".into()),
            ..strip(Save::Saved, true)
        };
        let (s, a) = next(s, Input::Snapshot(&saved));
        assert_eq!(a.last(), Some(&Action::CheckSaved("/d/video.webm".into())));
        // Then the iframe page: the strip is over the frame's area and saves from there.
        let (s, a) = next(s, Input::Snapshot(&saved));
        assert_eq!(
            a.last(),
            Some(&Action::Load("http://127.0.0.1:1/frame.html?save".into()))
        );
        let (s, _) = next(s, Input::Web(&done));
        let framed = Seen {
            video: Some(VideoState {
                frame: true,
                paused: true,
                ..playing()
            }),
            ..strip(Save::Offered, true)
        };
        let (s, a) = next(s, Input::Snapshot(&strip(Save::Offered, true)));
        assert!(a.is_empty(), "a video of the top document, not the frame");
        let (s, a) = next(s, Input::Snapshot(&framed));
        assert_eq!(a, vec![Action::PointTo(Spot::Video)]);
        let (s, a) = next(s, Input::Snapshot(&framed));
        assert_eq!(a.last(), Some(&Action::Click(Spot::Button("download"))));
        let (s, a) = next(
            s,
            Input::Snapshot(&Seen {
                runs: 1,
                saved: Some("/d/old.webm".into()),
                ..Seen {
                    save: Some(Save::Saved),
                    ..framed.clone()
                }
            }),
        );
        assert!(a.is_empty(), "the earlier download is not this one");
        let again = Seen {
            runs: 2,
            saved: Some("/d/video (1).webm".into()),
            save: Some(Save::Saved),
            ..framed
        };
        let (s, a) = next(s, Input::Snapshot(&again));
        assert_eq!(
            a.last(),
            Some(&Action::CheckSaved("/d/video (1).webm".into()))
        );
        let (_, a) = next(s, Input::Snapshot(&again));
        assert_eq!(a.last(), Some(&Action::Pass("video".into())));
    }

    #[test]
    fn the_saved_file_must_be_the_clip_in_the_folder() {
        let dir = std::env::temp_dir().join(format!("wubble-viddl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("video.webm");
        std::fs::write(&file, CLIP).unwrap();
        assert!(check_saved(&file, &dir).is_ok());
        assert!(check_saved(&file, Path::new("/elsewhere")).is_err());
        std::fs::write(&file, b"short").unwrap();
        assert!(check_saved(&file, &dir).is_err());
        assert!(check_saved(&dir.join("none"), &dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_lookup_must_be_private() {
        let prefix = wubble_core::sponsor::hash_prefix(SMOKE_VIDEO);
        let good =
            format!("get /api/skipsegments/{prefix}?categories=x http/1.1\nhost: 127.0.0.1\n");
        assert_eq!(private_lookup(std::slice::from_ref(&good)), Ok(()));
        assert!(private_lookup(&[]).is_err());
        assert!(private_lookup(&[good.clone(), good.clone()]).is_err());
        assert!(private_lookup(&[format!("{good}cookie: a=b\n")]).is_err());
        assert!(private_lookup(&[format!("{good}referer: http://x/\n")]).is_err());
        assert!(private_lookup(&["get /api/skipsegments/wubblesmoke? http/1.1\n".into()]).is_err());
    }

    #[test]
    fn the_boost_smoke_checks_each_page() {
        let s = VideoSmoke::boost("http://127.0.0.1:1".into(), Requests::default());
        let done = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        let (s, a) = next(s, Input::Web(&done));
        assert!(matches!(&a[..], [Action::PutBoost(_)]));
        let (s, _) = next(s, Input::BoostSaved(4));
        let (s, a) = next(s, Input::Web(&done));
        assert_eq!(a, vec![Action::Eval(BOOST_CHECK.into())]);
        let wrong = answer(
            r#"{"check":"boost","bg":"rgb(250, 250, 250)","title":"plain","after":"static"}"#,
        );
        let right = answer(
            r#"{"check":"boost","bg":"rgb(1, 2, 3)","title":"boosted","after":"fixed","at":"loading"}"#,
        );
        let (s, a) = next(s, Input::Page(&right));
        assert_eq!(a[1], Action::Load("http://127.0.0.1:1/other.html".into()));
        let (s, _) = next(s, Input::Web(&done));
        let (s, a) = next(s, Input::Page(&wrong));
        assert_eq!(
            a,
            vec![Action::Eval(
                "location.href = \"http://127.0.0.1:1/boost-csp.html\"".into()
            )]
        );
        let (s, _) = next(s, Input::Web(&done));
        let late = answer(
            r#"{"check":"boost","bg":"rgb(1, 2, 3)","title":"boosted","after":"fixed","at":"interactive"}"#,
        );
        let (_, a) = next(s.clone(), Input::Page(&late));
        assert!(matches!(&a[..], [Action::Fail(_)]), "a late script fails");
        let (s, a) = next(s, Input::Page(&right));
        assert_eq!(a, vec![Action::ToggleBoost(4)]);
        let (s, _) = next(s, Input::BoostSaved(4));
        let (s, _) = next(s, Input::Web(&done));
        let (_, a) = next(s, Input::Page(&wrong));
        assert_eq!(a, vec![Action::Pass("boost".into())]);
    }

    #[test]
    fn the_sponsor_skip_must_happen_again_after_a_reload() {
        let mut s = VideoSmoke::video(String::new(), Requests::default());
        s.step = Step::Skipping;
        let notice = |fired| Seen {
            notice: true,
            fired,
            ..seen(Some(playing()))
        };
        // The fake endpoint saw no lookup in this test: the check fails, which proves it runs.
        let (_, a) = next(s.clone(), Input::Snapshot(&notice(1)));
        assert!(matches!(&a[..], [Action::Fail(_)]));
        if let Ok(mut r) = s.requests.lock() {
            let prefix = wubble_core::sponsor::hash_prefix(SMOKE_VIDEO);
            r.push(format!("get /api/skipsegments/{prefix}?c http/1.1\n"));
        }
        let (s, a) = next(s, Input::Snapshot(&notice(1)));
        assert_eq!(a, vec![Action::Reload]);
        let (s, _) = next(s, Input::Web(&WebEvent::LoadStarted));
        let done = WebEvent::LoadingChanged {
            loading: false,
            can_go_back: false,
            can_go_forward: false,
        };
        let (s, a) = next(s, Input::Web(&done));
        assert_eq!(a, vec![Action::Eval(PLAY_BEFORE_JUMP.into())]);
        // The first skip's notice still up is not a new skip.
        let (s, a) = next(s, Input::Snapshot(&notice(0)));
        assert!(a.is_empty());
        assert_eq!(s.step, Step::SkippingAgain);
        let (s, a) = next(s, Input::Snapshot(&notice(1)));
        assert_eq!(a.last(), Some(&Action::Undo));
        assert_eq!(s.step, Step::Undone);
    }

    #[test]
    fn a_boost_that_did_not_apply_fails() {
        let s = VideoSmoke {
            step: Step::BoostChecked,
            ..VideoSmoke::boost(String::new(), Requests::default())
        };
        let wrong = answer(r#"{"check":"boost","bg":"rgb(250, 250, 250)","title":"plain"}"#);
        let (_, a) = next(s, Input::Page(&wrong));
        assert!(matches!(&a[..], [Action::Fail(_)]));
    }

    #[test]
    fn a_playing_video_is_not_stuck() {
        let mut s = VideoSmoke::video(String::new(), Requests::default());
        s.step = Step::Skipping;
        for i in 0..3 {
            for _ in 0..MAX_STEP_RUNS {
                (s, _) = next(s, Input::Worked);
            }
            let moved = VideoState {
                time: 0.5 + f64::from(i) / 100.0,
                ..playing()
            };
            let a;
            (s, a) = next(s, Input::Snapshot(&seen(Some(moved))));
            assert!(a.is_empty());
        }
        assert_eq!(s.step, Step::Skipping);
    }

    #[test]
    fn a_stuck_step_fails() {
        let mut s = VideoSmoke::video(String::new(), Requests::default());
        let mut last = vec![];
        for _ in 0..=MAX_STEP_RUNS {
            (s, last) = next(s, Input::Worked);
        }
        assert!(matches!(&last[..], [Action::Fail(e)] if e.contains("Found")));
    }

    #[test]
    fn the_clip_is_a_small_webm() {
        assert!(CLIP.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]), "EBML header");
        assert!(CLIP.len() < 16 * 1024);
        assert_eq!(
            wubble_core::sponsor::video_id(&format!("https://youtu.be/{SMOKE_VIDEO}")).as_deref(),
            Some(SMOKE_VIDEO)
        );
    }
}
