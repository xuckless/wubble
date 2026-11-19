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
//! 6. a media key (Toggle) through the host's path pauses the playing video.
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
use wubble_core::boost::{At, Boost, Tint};
use wubble_core::video::VideoState;
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

const VIDEO_PAGE: &str = "<!doctype html><title>video</title>\
    <body style=\"margin:0;background:#202020\">\
    <video src=\"/clip.webm\" muted autoplay playsinline width=\"320\" height=\"240\" \
    style=\"display:block;margin:40px\"></video>";
const BOOST_PAGE: &str = "<!doctype html><title>plain</title>\
    <body style=\"background:rgb(250,250,250)\"><p>A page to boost.</p>";

/// The clip from its start, playing, so a pause is the picture-in-picture's.
const RESTART: &str = "(k=>(k.cmd('seek',0),k.cmd('play'),1))(window[Symbol.for('wubble.kit')])";

/// Plays from just before the segment, so the wait is short; muted, since an unmuted video does
/// not play without a user gesture.
const PLAY_BEFORE_JUMP: &str = "(k=>(k.cmd('muted',true),k.cmd('rate',1),k.cmd('seek',0.5),k.cmd('play'),1))\
     (window[Symbol.for('wubble.kit')])";

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
    Undo,
    Key(MediaKey),
    PutBoost(Box<Boost>),
    ToggleBoost(i64),
    Log(String),
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
        (KeyPaused, Snapshot(_)) if video.as_ref().is_some_and(|v| v.paused) => {
            to(s, KeyPaused, vec![Action::Pass("video".into())])
        }

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
        "/video.html" => ("text/html; charset=utf-8", VIDEO_PAGE.into(), ""),
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
