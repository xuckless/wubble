//! `--smoke codecs` and `--smoke codecs=bare`: what plays, and which layer stops what. Windowless
//! (no wubble window), run by hand. A page on one loopback origin plays media from a second one
//! (another port), so every source is cross-origin as on the web:
//!
//! - support: `canPlayType`, `MediaSource.isTypeSupported` and `MediaCapabilities.decodingInfo`
//!   for every codec of note, including X's `avc1.64001f, mp4a.40.2`;
//! - playback: each clip in a `<video>`/`<audio>`, muted, through `play()`: `playing`, time
//!   advancing, else the element's error, `readyState` and `networkState`; Range answered with
//!   206 (`/m/r*`) and ignored (`/m/n`), CORS headers with and without `crossorigin`, `blob:`,
//!   `data:`, Media Source Extensions fed fragmented MP4 (X's H.264 + AAC shape) and WebM, native
//!   HLS (fMP4 and MPEG-TS), a video in a cross-origin iframe;
//! - the autoplay policy: `autoplay` with and without `muted`, `play()` with sound without a user
//!   gesture, then with one (a click the smoke sends);
//! - cosmetic filtering: videos inside common player wrappers stay visible.
//!
//! `codecs` runs with the browser's wiring: wubble-blocker on its bundled default lists (the
//! `Blocking` the app builds), the media downloader's response observer and the page kit as a
//! document script (the helper's own frame scripts run in both modes). `codecs=bare` runs with
//! none of them, so a row that differs between the two is wubble's doing. `codecs` also prints
//! the blocker's decisions for X's and YouTube's media URL shapes and fails if any is blocked
//! or rewritten.
//!
//! The clips come from `WUBBLE_CODEC_CLIPS=DIR` (written by `scripts/codec-clips.sh`); without
//! it only the bundled VP8 clip runs (cross-origin, Range, no Range, CORS, `blob:`, `data:`,
//! iframe, autoplay), which is the regression check that always applies. Rows for codecs prebuilt
//! CEF does not have (H.264, AAC, MPEG-TS, native HLS, HEVC: no `proprietary_codecs`) are
//! reported, not asserted, unless the build says it plays H.264 (a codec build from
//! `scripts/build-cef.sh`): then all but HEVC must play. HEVC is always reported only.
//! `WUBBLE_PROBE_URL=URL` (`codecs` only) also loads that page (an ordinary page load), then
//! prints its media elements' state and what the blocker blocked there.
//!
//! Bounded by counts of pump turns; the page bounds each attempt with its own timer, which only
//! stops a hang (nothing is timed or asserted on speed).

use std::collections::BTreeMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use slint::winit_030::winit::keyboard::ModifiersState;
use wubble_core::space::ContainerId;
use wubble_engine::app::{Engine, EngineConfig, EngineError, ViewId};
use wubble_engine::filter::{ContentFilter, RequestAction, RequestInfo, RequestKind};
use wubble_engine::input::{Buttons, Modifiers, MouseButton, Point};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

use crate::blocking::{BlockerSettings, Blocking};

/// Pump turns the whole page may take (each turn sleeps [`TURN_PAUSE`]); only stops a hang.
const MAX_TURNS: u32 = 600_000;
/// Pump turns a `WUBBLE_PROBE_URL` page gets before its media is looked at.
const LIVE_TURNS: u32 = 15_000;
const TURN_PAUSE: Duration = Duration::from_millis(1);

const BUNDLED: &[u8] = include_bytes!("../assets/clip.webm");

/// Rows that must play in every mode: a wubble blockage, never the build.
const MUST_ALWAYS: &[&str] = &[
    "bundled range+cors",
    "bundled no-range",
    "bundled range, no cors",
    "bundled crossorigin+cors",
    "bundled blob",
    "bundled data",
    "bundled autoplay muted",
    "iframe bundled",
];

/// Rows that must play when the clips are there: the codecs prebuilt CEF has.
const MUST_WITH_CLIPS: &[&str] = &[
    "vp8-vorbis.webm",
    "vp9-opus.webm",
    "vp9-opus.mp4",
    "av1.mp4",
    "av1-opus.webm",
    "audio.mp3",
    "audio.ogg",
    "audio.opus",
    "audio.flac",
    "audio.wav",
    "mse vp9 webm",
    "vp9-opus.webm no-range",
];

/// The support row that tells a build with proprietary codecs (`proprietary_codecs=true
/// ffmpeg_branding=Chrome`, `scripts/build-cef.sh`) from one without.
const H264_SUPPORT: &str = "support video/mp4; codecs=\"avc1.42E01E\"";

/// Rows that must play when the build has proprietary codecs: H.264, AAC, MPEG-TS, native HLS.
/// HEVC stays reported only: on Linux it needs a VA-API decoder, so it is the machine's, not
/// the build's.
const MUST_WITH_CODECS: &[&str] = &[
    "support video/mp4; codecs=\"avc1.64001f, mp4a.40.2\"",
    "support video/mp4; codecs=\"avc1.64001f\"",
    "support audio/mp4; codecs=\"mp4a.40.2\"",
    "support video/mp2t; codecs=\"avc1.42E01E, mp4a.40.2\"",
    "support application/vnd.apple.mpegurl",
];

/// Rows that must play when the build has proprietary codecs and the clips are there.
const MUST_WITH_CODECS_AND_CLIPS: &[&str] = &[
    "h264-aac.mp4",
    "h264.mp4",
    "audio.m4a",
    "mse h264+aac fmp4 (X)",
    "native hls h264 fmp4",
    "native hls h264 ts",
    "native hls vp9 fmp4",
];

/// The clips the page plays when present, with the element they go in.
const CLIPS: &[(&str, &str)] = &[
    ("h264-aac.mp4", "video"),
    ("h264.mp4", "video"),
    ("hevc.mp4", "video"),
    ("vp8-vorbis.webm", "video"),
    ("vp9-opus.webm", "video"),
    ("vp9-opus.mp4", "video"),
    ("av1.mp4", "video"),
    ("av1-opus.webm", "video"),
    // Audio files play in a `<video>` too: Chromium lets only a video element play muted
    // without a user gesture (`AutoplayPolicy::IsEligibleForAutoplayMuted`); a muted `<audio>`
    // is refused like one with sound, which is the policy, not the codec.
    ("audio.mp3", "video"),
    ("audio.m4a", "video"),
    ("audio.ogg", "video"),
    ("audio.opus", "video"),
    ("audio.flac", "video"),
    ("audio.wav", "video"),
];

/// Media URL shapes of X and YouTube, with their page: none may be blocked or rewritten.
const SITE_URLS: &[(&str, &str, RequestKind)] = &[
    (
        "https://video.twimg.com/ext_tw_video/1850000000000000000/pu/vid/avc1/1280x720/AbCdEfGh.mp4?tag=12",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Media,
    ),
    (
        "https://video.twimg.com/ext_tw_video/1850000000000000000/pu/pl/AbCdEfGh.m3u8?tag=12&v=4a5",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Xhr,
    ),
    (
        "https://video.twimg.com/ext_tw_video/1850000000000000000/pu/pl/avc1/1280x720/AbCd.m3u8",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Xhr,
    ),
    (
        "https://video.twimg.com/ext_tw_video/1850000000000000000/pu/vid/avc1/0/3000/1280x720/AbCd.m4s",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Xhr,
    ),
    (
        "https://video.twimg.com/amplify_video/1850000000000000000/vid/avc1/1280x720/AbCd.mp4?tag=16",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Media,
    ),
    (
        "https://video.twimg.com/tweet_video/GaBcDeFgHiJ.mp4",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Media,
    ),
    (
        "https://pbs.twimg.com/ext_tw_video_thumb/1850000000000000000/pu/img/AbCd.jpg",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Image,
    ),
    (
        "https://abs.twimg.com/responsive-web/client-web/main.0123abcd.js",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Script,
    ),
    (
        "https://abs.twimg.com/responsive-web/client-web/bundle.Videos.0123abcd.js",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Script,
    ),
    (
        "https://x.com/i/api/graphql/AbCdEf/TweetResultByRestId?variables=%7B%22tweetId%22%3A%221%22%7D&features=%7B%7D",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Xhr,
    ),
    (
        "https://api.x.com/graphql/AbCdEf/TweetResultByRestId?variables=%7B%7D&features=%7B%7D",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Xhr,
    ),
    (
        "https://api.x.com/1.1/videos/tweet/config/1850000000000000000.json",
        "https://x.com/someone/status/1850000000000000000",
        RequestKind::Xhr,
    ),
    (
        "https://rr3---sn-abcdef.googlevideo.com/videoplayback?expire=1&ei=x&ip=1.2.3.4&id=o-AB&itag=243&source=youtube&requiressl=yes&mime=video%2Fwebm&sig=AbC&lsig=XyZ&range=0-1000",
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        RequestKind::Xhr,
    ),
    (
        "https://d111111abcdef8.cloudfront.net/video/clip.mp4?Expires=1700000000&Signature=AbC~&Key-Pair-Id=K2JCJMDEHXQW5F",
        "https://news.example/article",
        RequestKind::Media,
    ),
    (
        "https://cdn.example.net/hls/master.m3u8?token=abc&expires=1700000000&utm_source=newsletter",
        "https://news.example/article",
        RequestKind::Xhr,
    ),
];

/// A dev server's requests on its own page: printed, not asserted (the lists filter local pages
/// as uBO does; the site allowlist turns them off for a host).
const LOCAL_URLS: &[(&str, RequestKind)] = &[
    ("http://localhost:5173/@vite/client", RequestKind::Script),
    ("http://localhost:5173/src/main.tsx", RequestKind::Script),
    (
        "http://localhost:5173/node_modules/.vite/deps/react.js?v=1a2b",
        RequestKind::Script,
    ),
    (
        "http://localhost:5173/src/components/AdBanner.tsx",
        RequestKind::Script,
    ),
    (
        "http://localhost:5173/src/ads/AdSlot.tsx?t=1",
        RequestKind::Script,
    ),
    (
        "http://localhost:5173/api/analytics/track",
        RequestKind::Xhr,
    ),
    (
        "http://localhost:5173/assets/banner-ad.png",
        RequestKind::Image,
    ),
    ("ws://localhost:5173/?token=abc", RequestKind::Other),
    (
        "http://localhost:3000/_next/static/chunks/pages/_app.js",
        RequestKind::Script,
    ),
    ("http://localhost:3000/?utm_source=x", RequestKind::Document),
];

/// One answer the loopback servers gave, for the Range check.
#[derive(Clone, Debug)]
struct Served {
    path: String,
    range: Option<String>,
    status: u16,
}

type Log = Arc<Mutex<Vec<Served>>>;

pub fn run(
    config: Result<EngineConfig, EngineError>,
    profile: &Path,
    wired: bool,
) -> Result<(), Box<dyn Error>> {
    let clips = std::env::var_os("WUBBLE_CODEC_CLIPS").map(PathBuf::from);
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert("bundled.webm".into(), BUNDLED.to_vec());
    if let Some(dir) = &clips {
        read_tree(dir, dir, &mut files)?;
    }
    let have = |name: &str| files.contains_key(name);
    let clip_rows: Vec<(&str, &str)> = CLIPS.iter().copied().filter(|(f, _)| have(f)).collect();
    let missing: Vec<&str> = CLIPS.iter().map(|c| c.0).filter(|f| !have(f)).collect();
    if clips.is_some() && !missing.is_empty() {
        println!("codecs: clips missing (not run): {}", missing.join(" "));
    }
    let log: Log = Arc::default();
    let files = Arc::new(files);
    let media_port = serve(files.clone(), log.clone(), None)?;
    let page = page_html(media_port, &clip_rows, &files);
    let frame = frame_html(media_port);
    let page_port = serve(
        files,
        log.clone(),
        Some(Arc::new(
            [("/".to_owned(), page), ("/frame.html".to_owned(), frame)].into(),
        )),
    )?;

    let blocking = wired.then(|| {
        let b = Blocking::new();
        if let Some(t) = b.start_with(profile, &BlockerSettings::default(), |_| {
            Err("no network".into())
        }) {
            // Compiling the bundled lists; only the first rebuild, no update loop (off).
            let _ = t.join();
        }
        b
    });
    if let Some(b) = &blocking {
        site_urls(b)?;
    }
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let mut engine = Engine::start(EngineConfig {
        content_filter: blocking.clone().map(|b| b as Arc<dyn ContentFilter>),
        response_observer: wired.then(|| crate::host::MediaObserver::new() as _),
        ..config?
    })?;
    let outcome = drive(
        &mut engine,
        &gpu,
        page_port,
        wired,
        &log,
        clip_rows.len(),
        blocking.as_deref(),
    );
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
    let rows = outcome?;
    verdict(&rows, &log, media_port, clips.is_some(), wired)
}

fn read_tree(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), Box<dyn Error>> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            read_tree(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.insert(
                rel.to_string_lossy().replace('\\', "/"),
                std::fs::read(&path)?,
            );
        }
    }
    Ok(())
}

/// The blocker's decision for each of [`SITE_URLS`]; any block or rewrite is an error.
fn site_urls(b: &Blocking) -> Result<(), Box<dyn Error>> {
    let mut bad = vec![];
    for (url, top, kind) in SITE_URLS {
        let action = b.check(&RequestInfo {
            url,
            top_url: top,
            kind: *kind,
            browser_id: 0,
        });
        let short: String = url.chars().take(90).collect();
        println!("codecs: blocker {kind:?} {short} -> {action:?}");
        let fine = match &action {
            RequestAction::Allow => true,
            // URL Tracking Protection: only `utm_*` may go, every other parameter stays.
            RequestAction::Redirect(to) => only_tracking_dropped(url, to),
            _ => false,
        };
        if !fine {
            bad.push(format!("{url}: {action:?}"));
        }
    }
    for (url, kind) in LOCAL_URLS {
        let action = b.check(&RequestInfo {
            url,
            top_url: "http://localhost:5173/",
            kind: *kind,
            browser_id: 0,
        });
        println!("codecs: blocker on a dev server {kind:?} {url} -> {action:?}");
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("the blocker touches media URLs: {}", bad.join("; ")).into())
    }
}

/// Whether `to` is `from` with only `utm_*` query parameters removed.
fn only_tracking_dropped(from: &str, to: &str) -> bool {
    let split = |u: &str| -> (String, Vec<String>) {
        let (base, query) = u.split_once('?').unwrap_or((u, ""));
        let params = query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect();
        (base.to_owned(), params)
    };
    let ((from_base, from_params), (to_base, to_params)) = (split(from), split(to));
    let kept: Vec<String> = from_params
        .into_iter()
        .filter(|p| !p.starts_with("utm_"))
        .collect();
    from_base == to_base && kept == to_params
}

fn layout() -> Layout {
    Layout {
        size: FrameSize::new(800, 600),
        scale: 1.0,
        area: None,
    }
}

fn drive(
    engine: &mut Engine,
    gpu: &GpuContext,
    port: u16,
    wired: bool,
    log: &Log,
    clips: usize,
    blocking: Option<&Blocking>,
) -> Result<Vec<Value>, Box<dyn Error>> {
    let open = |engine: &mut Engine, url: &str| {
        engine.open_view(ViewConfig {
            url: url.into(),
            layout: layout(),
            frame_rate: 30,
            begin_frames: false,
            gpu: gpu.presenter().context().clone(),
            container: ContainerId::DEFAULT,
            storage: wubble_core::space::Storage::Disk,
        })
    };
    let view = open(engine, "about:blank")?;
    if wired {
        for _ in 0..MAX_TURNS {
            if engine.view(view).is_some_and(|v| v.is_open()) {
                break;
            }
            engine.tick();
            std::thread::sleep(TURN_PAUSE);
        }
        let v = engine.view(view).ok_or("the view closed")?;
        v.set_document_scripts(&[crate::host::KIT.to_owned()]);
        // The engine keeps the registration's DevTools reply to itself; some turns let it land
        // before the page loads (the probe checks below that the kit ran).
        for _ in 0..2_000 {
            engine.tick();
            std::thread::sleep(TURN_PAUSE);
        }
    }
    let _ = log;
    engine
        .view(view)
        .ok_or("the view closed")?
        .load(&format!("http://127.0.0.1:{port}/"));
    println!("codecs: page http://127.0.0.1:{port}/ ({clips} clips, wired={wired})");
    let mut rows = vec![];
    let mut done = (false, false);
    let mut clicked = false;
    for _ in 0..MAX_TURNS {
        engine.tick();
        for (id, event) in engine.take_page_events() {
            let PageEvent::Console { message, .. } = &event else {
                continue;
            };
            if id != view {
                continue;
            }
            match message.as_str() {
                "probe done" => done.0 = true,
                "probe frame done" => done.1 = true,
                "probe click me" if !clicked => {
                    clicked = true;
                    click(engine, view, Point { x: 400.0, y: 300.0 })?;
                }
                m => {
                    if let Some(json) = m.strip_prefix("probe ") {
                        match serde_json::from_str::<Value>(json) {
                            Ok(row) => {
                                println!("codecs: {}", row);
                                rows.push(row);
                            }
                            Err(_) => println!("codecs: page said {m}"),
                        }
                    }
                }
            }
        }
        if done == (true, true) {
            break;
        }
        std::thread::sleep(TURN_PAUSE);
    }
    if done != (true, true) {
        return Err(format!(
            "the page never finished (main {}, frame {})",
            done.0, done.1
        )
        .into());
    }
    if let (Some(b), Some(url)) = (blocking, std::env::var("WUBBLE_PROBE_URL").ok()) {
        live(engine, &open, b, &url)?;
    }
    Ok(rows)
}

/// `WUBBLE_PROBE_URL`: the page as a user would load it, then its media and what was blocked.
fn live(
    engine: &mut Engine,
    open: &dyn Fn(&mut Engine, &str) -> Result<ViewId, EngineError>,
    b: &Blocking,
    url: &str,
) -> Result<(), Box<dyn Error>> {
    let view = open(engine, url)?;
    for _ in 0..LIVE_TURNS {
        engine.tick();
        for (id, event) in engine.take_page_events() {
            if id == view
                && let PageEvent::Console { message, .. } = event
                && message.contains("rror")
            {
                println!(
                    "codecs: live console: {}",
                    message.chars().take(200).collect::<String>()
                );
            }
        }
        std::thread::sleep(TURN_PAUSE);
    }
    let script = "JSON.stringify([...document.querySelectorAll('video,audio')].map(v=>({src:(v.currentSrc||'').slice(0,80),\
                  t:v.currentTime,paused:v.paused,rs:v.readyState,ns:v.networkState,err:v.error&&[v.error.code,v.error.message]})))\
                  +' mse-avc-aac='+MediaSource.isTypeSupported('video/mp4; codecs=\"avc1.64001f, mp4a.40.2\"')";
    let v = engine.view(view).ok_or("the live view closed")?;
    let browser = v.browser_id();
    let id = v.eval(script).ok_or("no main frame")?;
    for _ in 0..LIVE_TURNS {
        engine.tick();
        for (from, event) in engine.take_page_events() {
            if from == view
                && let PageEvent::EvalResult { id: got, result } = &event
                && *got == id
            {
                println!("codecs: live {url}: media {result:?}");
                println!(
                    "codecs: live blocked {} requests: {:?}",
                    b.blocked(browser),
                    b.blocked_hosts(browser)
                );
                return Ok(());
            }
        }
        std::thread::sleep(TURN_PAUSE);
    }
    Err("the live page never answered".into())
}

fn click(engine: &mut Engine, view: ViewId, at: Point) -> Result<(), Box<dyn Error>> {
    let v = engine.view(view).ok_or("the view closed")?;
    let up = Modifiers {
        keys: ModifiersState::empty(),
        buttons: Buttons::default(),
        locks: Default::default(),
    };
    let down = Modifiers {
        buttons: Buttons::default().with(MouseButton::Left, true),
        ..up
    };
    v.focus(true);
    v.mouse_move(at, &up, Duration::ZERO);
    v.mouse_button(MouseButton::Left, true, at, &down, Duration::ZERO);
    v.mouse_button(MouseButton::Left, false, at, &up, Duration::ZERO);
    Ok(())
}

fn verdict(
    rows: &[Value],
    log: &Log,
    media_port: u16,
    clips: bool,
    wired: bool,
) -> Result<(), Box<dyn Error>> {
    let played = |name: &str| rows.iter().any(|r| r["name"] == name && r["ok"] == true);
    let mut must: Vec<&str> = MUST_ALWAYS.to_vec();
    if clips {
        must.extend(MUST_WITH_CLIPS);
    }
    let codecs = played(H264_SUPPORT);
    if codecs {
        must.extend(MUST_WITH_CODECS);
        if clips {
            must.extend(MUST_WITH_CODECS_AND_CLIPS);
        }
    }
    println!(
        "codecs: proprietary codecs in this build: {}",
        if codecs {
            "yes, asserted"
        } else {
            "no, reported"
        }
    );
    if wired {
        must.push("page kit");
    }
    let failed: Vec<&str> = must.into_iter().filter(|n| !played(n)).collect();
    // Range: the media origin answered at least one ranged request with 206.
    let log = log.lock().unwrap_or_else(PoisonError::into_inner);
    let ranged = log
        .iter()
        .filter(|s| s.range.is_some() && s.status == 206 && s.path.starts_with("/m/r"))
        .count();
    let ignored = log
        .iter()
        .filter(|s| s.range.is_some() && s.path.starts_with("/m/n"))
        .count();
    println!(
        "codecs: media origin :{media_port} answered {} requests, {ranged} ranged with 206, {ignored} ranged ones as 200 (no-range server)",
        log.len()
    );
    let hidden: Vec<String> = rows
        .iter()
        .filter(|r| r["hidden"] == true)
        .map(|r| r["name"].to_string())
        .collect();
    let mut problems = vec![];
    if !failed.is_empty() {
        problems.push(format!("did not play: {}", failed.join(", ")));
    }
    if ranged == 0 {
        problems.push("no Range request reached the media origin".into());
    }
    if !hidden.is_empty() {
        problems.push(format!("hidden by a style: {}", hidden.join(", ")));
    }
    let build: Vec<String> = rows
        .iter()
        .filter(|r| r["ok"] == false && r["build"] == true)
        .map(|r| r["name"].as_str().unwrap_or_default().to_owned())
        .collect();
    if !build.is_empty() {
        println!(
            "codecs: not in this Chromium build (proprietary codecs): {}",
            build.join(", ")
        );
    }
    let mode = if wired { "codecs" } else { "codecs=bare" };
    if problems.is_empty() {
        println!("smoke: {mode} ok");
        Ok(())
    } else {
        Err(problems.join("; ").into())
    }
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "mp4" | "m4s" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "m3u8" => "application/vnd.apple.mpegurl",
        "ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

/// A loopback server on its own port: `pages` (path → HTML) when given, else media under
/// `/m/<flags>/<file>`: `r` answers Range with 206, `c` adds CORS headers (`n`: neither).
fn serve(
    files: Arc<BTreeMap<String, Vec<u8>>>,
    log: Log,
    pages: Option<Arc<BTreeMap<String, String>>>,
) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (files, log, pages) = (files.clone(), log.clone(), pages.clone());
            std::thread::spawn(move || {
                let _ = answer(stream, &files, &log, pages.as_deref());
            });
        }
    });
    Ok(port)
}

fn answer(
    mut stream: TcpStream,
    files: &BTreeMap<String, Vec<u8>>,
    log: &Log,
    pages: Option<&BTreeMap<String, String>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }
        let mut range = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("range")
            {
                range = Some(value.trim().to_owned());
            }
        }
        let path = request.split_whitespace().nth(1).unwrap_or("/");
        let path = path.split('?').next().unwrap_or(path).to_owned();
        let mut headers = String::from("Cache-Control: no-store\r\n");
        let (status, mime, body): (u16, &str, &[u8]) = if let Some(pages) = pages {
            match pages.get(&path) {
                Some(html) => (200, "text/html; charset=utf-8", html.as_bytes()),
                None => (404, "text/plain", b"not found"),
            }
        } else {
            let mut parts = path.trim_start_matches('/').splitn(3, '/');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("m"), Some(flags), Some(file)) if files.contains_key(file) => {
                    if flags.contains('c') {
                        headers.push_str("Access-Control-Allow-Origin: *\r\n");
                    }
                    (200, mime_of(file), &files[file][..])
                }
                _ => (404, "text/plain", b"not found"),
            }
        };
        let honour = path.starts_with("/m/r");
        let (status, body) = match (
            honour,
            status,
            range.as_deref().and_then(|r| byte_range(r, body.len())),
        ) {
            (true, 200, Some((a, b))) => {
                headers.push_str(&format!("Content-Range: bytes {a}-{b}/{}\r\n", body.len()));
                (206, &body[a..=b])
            }
            _ => (status, body),
        };
        if honour {
            headers.push_str("Accept-Ranges: bytes\r\n");
        }
        log.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Served {
                path: path.clone(),
                range,
                status,
            });
        let reason = match status {
            200 => "OK",
            206 => "Partial Content",
            _ => "Not Found",
        };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\n{headers}Content-Length: {}\r\n\r\n",
            body.len()
        )?;
        stream.write_all(body)?;
        stream.flush()?;
    }
}

/// `bytes=a-b` / `bytes=a-` / `bytes=-n` against a body of `len`, inclusive; `None` when it
/// does not fit (then the whole body is sent).
fn byte_range(spec: &str, len: usize) -> Option<(usize, usize)> {
    let (a, b) = spec.strip_prefix("bytes=")?.split_once('-')?;
    let last = len.checked_sub(1)?;
    let (a, b) = match (a.trim(), b.trim()) {
        ("", n) => (len.saturating_sub(n.parse().ok()?), last),
        (a, "") => (a.parse().ok()?, last),
        (a, b) => (a.parse().ok()?, b.parse::<usize>().ok()?.min(last)),
    };
    (a <= b).then_some((a, b))
}

/// The probe's script, shared by the page and its cross-origin frame.
const RUNNER: &str = r#"
const log = (o) => console.log('probe ' + JSON.stringify(o));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
// One attempt: `make` returns the element (not yet in the page), `setup` may feed it.
async function attempt(name, make, opts = {}) {
  const el = make();
  el.style.cssText = 'width:160px;height:90px;display:block';
  (opts.parent || document.getElementById('stage') || document.body).appendChild(el);
  const r = { name, ok: false };
  let playErr = null;
  el.addEventListener('playing', () => { r.playing = true; });
  try { if (opts.setup) await opts.setup(el); } catch (e) { r.setup = String(e); }
  if (opts.play !== false) {
    try { const p = el.play(); if (p) p.catch((e) => { playErr = e.name + ': ' + e.message; }); }
    catch (e) { playErr = String(e); }
  }
  for (let i = 0; i < 80; i++) {
    await wait(100);
    if (el.currentTime > 0.3 || el.error || (playErr && !opts.keepTrying)) break;
  }
  r.ok = el.currentTime > 0.3;
  r.t = Math.round(el.currentTime * 100) / 100;
  r.paused = el.paused; r.rs = el.readyState; r.ns = el.networkState;
  if (el.error) { r.err = el.error.code; r.msg = el.error.message; }
  if (playErr) r.play = playErr;
  const box = opts.check || el;
  r.hidden = getComputedStyle(box).display === 'none' || box.getBoundingClientRect().height === 0;
  if (opts.build) r.build = true;
  el.pause(); el.removeAttribute('src'); el.load(); el.remove();
  log(r);
  return r;
}
const media = (tag, src, extra = {}) => () => {
  const el = document.createElement(tag);
  el.muted = extra.muted !== false;
  if (extra.crossorigin) el.crossOrigin = 'anonymous';
  if (extra.autoplay) el.autoplay = true;
  if (extra.mutedAttr) el.setAttribute('muted', '');
  if (src) el.src = src;
  return el;
};
"#;

/// The cross-origin iframe's page: one attempt on the bundled clip.
fn frame_html(media_port: u16) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>frame</title><body>\
         <script>{RUNNER}\n(async () => {{ await attempt('iframe bundled', media('video', 'http://127.0.0.1:{media_port}/m/rc/bundled.webm')); \
         console.log('probe frame done'); }})();</script>"
    )
}

fn page_html(media_port: u16, clips: &[(&str, &str)], files: &BTreeMap<String, Vec<u8>>) -> String {
    let clips: Vec<Value> = clips
        .iter()
        .map(|(f, tag)| {
            let build = ["h264", "hevc", "m4a"].iter().any(|p| f.contains(p));
            serde_json::json!({ "file": f, "tag": tag, "build": build })
        })
        .collect();
    let have = |f: &str| files.contains_key(f);
    let flags = serde_json::json!({
        "hlsFmp4": have("hls-fmp4/media.m3u8"),
        "hlsTs": have("hls-ts/media.m3u8"),
        "hlsVp9": have("hls-vp9/media.m3u8"),
        "mseH264": have("mse-h264-aac.mp4"),
        "mseVp9": have("mse-vp9.webm"),
        "vp9": have("vp9-opus.webm"),
    });
    let data_url = format!(
        "data:video/webm;base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, BUNDLED)
    );
    format!(
        r#"<!doctype html><meta charset=utf-8><title>codecs</title>
<body style="margin:0">
<div id="stage" style="position:fixed;left:0;top:0;z-index:5"></div>
<div style="height:120px"></div>
<div class="video-js vjs-default-skin" id="w1"><video class="vjs-tech" muted></video></div>
<div class="jwplayer jw-reset" id="player"><video class="jw-video" muted></video></div>
<div class="html5-video-player" id="movie_player"><video class="video-stream html5-main-video" muted></video></div>
<div class="plyr" data-testid="videoPlayer"><video class="plyr__video" muted></video></div>
<script>{RUNNER}
const B = 'http://127.0.0.1:{media_port}';
const CLIPS = {clips};
const HAVE = {flags};
const DATA_URL = '{data_url}';
const SUPPORT = [
  ['video/mp4; codecs="avc1.64001f, mp4a.40.2"', null, true],
  ['video/mp4; codecs="avc1.42E01E"', 'video', true],
  ['video/mp4; codecs="avc1.64001f"', 'video', true],
  ['audio/mp4; codecs="mp4a.40.2"', 'audio', true],
  ['video/mp4; codecs="hvc1.1.6.L93.B0"', 'video', true],
  ['video/mp2t; codecs="avc1.42E01E, mp4a.40.2"', null, true],
  ['application/vnd.apple.mpegurl', null, true],
  ['video/webm; codecs="vp8, vorbis"', null, false],
  ['video/webm; codecs="vp8"', 'video', false],
  ['video/webm; codecs="vp9"', 'video', false],
  ['video/webm; codecs="vp09.00.10.08"', 'video', false],
  ['video/mp4; codecs="vp09.00.10.08"', 'video', false],
  ['video/webm; codecs="av01.0.04M.08"', 'video', false],
  ['video/mp4; codecs="av01.0.04M.08"', 'video', false],
  ['audio/webm; codecs="opus"', 'audio', false],
  ['audio/mp4; codecs="opus"', 'audio', false],
  ['audio/ogg; codecs="vorbis"', 'audio', false],
  ['audio/mpeg', 'audio', false],
  ['audio/flac', 'audio', false],
  ['audio/wav; codecs="1"', 'audio', false],
];
async function support() {{
  for (const [type, kind, build] of SUPPORT) {{
    const r = {{ name: 'support ' + type, canPlay: document.createElement('video').canPlayType(type) || '""',
      mse: window.MediaSource ? MediaSource.isTypeSupported(type) : 'no MSE' }};
    if (kind) {{
      const conf = kind === 'video'
        ? {{ type: 'file', video: {{ contentType: type, width: 1280, height: 720, bitrate: 2e6, framerate: 30 }} }}
        : {{ type: 'file', audio: {{ contentType: type, channels: '2', bitrate: 128000, samplerate: 48000 }} }};
      try {{ const d = await navigator.mediaCapabilities.decodingInfo(conf);
        r.decode = (d.supported ? 'supported' : 'no') + (d.smooth ? ' smooth' : '') + (d.powerEfficient ? ' hw' : ''); }}
      catch (e) {{ r.decode = String(e); }}
    }}
    // A playlist is never an MSE type: native HLS is `canPlayType`'s answer alone.
    r.ok = type === 'application/vnd.apple.mpegurl' ? r.canPlay !== '""' : r.mse === true;
    if (build) r.build = true;
    log(r);
  }}
}}
// MSE as hls.js / X feed it: the whole file appended to one SourceBuffer.
function mseSetup(url, type) {{
  return async (el) => {{
    if (!MediaSource.isTypeSupported(type)) throw 'isTypeSupported false';
    const ms = new MediaSource();
    el.src = URL.createObjectURL(ms);
    await new Promise((r) => ms.addEventListener('sourceopen', r, {{ once: true }}));
    const buf = await (await fetch(url)).arrayBuffer();
    const sb = ms.addSourceBuffer(type);
    await new Promise((r, j) => {{ sb.addEventListener('updateend', r, {{ once: true }});
      sb.addEventListener('error', () => j('appendBuffer error'), {{ once: true }}); sb.appendBuffer(buf); }});
    ms.endOfStream();
  }};
}}
async function blobUrl(src) {{ return URL.createObjectURL(await (await fetch(src)).blob()); }}
(async () => {{
  log({{ name: 'page kit', ok: !!window[Symbol.for('wubble.kit')] }});
  await support();
  // The bundled VP8 clip: the request path, whatever the codecs.
  await attempt('bundled range+cors', media('video', B + '/m/rc/bundled.webm'));
  await attempt('bundled range, no cors', media('video', B + '/m/r/bundled.webm'));
  await attempt('bundled no-range', media('video', B + '/m/n/bundled.webm'));
  await attempt('bundled crossorigin+cors', media('video', B + '/m/rc/bundled.webm', {{ crossorigin: true }}));
  await attempt('bundled crossorigin, no cors (spec: fails)', media('video', B + '/m/r/bundled.webm', {{ crossorigin: true }}));
  const blob = await blobUrl(B + '/m/rc/bundled.webm');
  await attempt('bundled blob', media('video', blob));
  await attempt('bundled data', media('video', DATA_URL));
  for (const c of CLIPS) {{
    await attempt(c.file, media(c.tag, B + '/m/rc/' + c.file), {{ build: c.build }});
  }}
  if (HAVE.vp9) await attempt('vp9-opus.webm no-range', media('video', B + '/m/n/vp9-opus.webm'));
  if (HAVE.mseH264) await attempt('mse h264+aac fmp4 (X)', media('video'),
    {{ setup: mseSetup(B + '/m/rc/mse-h264-aac.mp4', 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"'), build: true }});
  if (HAVE.mseVp9) await attempt('mse vp9 webm', media('video'),
    {{ setup: mseSetup(B + '/m/rc/mse-vp9.webm', 'video/webm; codecs="vp9"') }});
  if (HAVE.hlsFmp4) await attempt('native hls h264 fmp4', media('video', B + '/m/rc/hls-fmp4/media.m3u8'), {{ build: true }});
  if (HAVE.hlsTs) await attempt('native hls h264 ts', media('video', B + '/m/rc/hls-ts/media.m3u8'), {{ build: true }});
  if (HAVE.hlsVp9) await attempt('native hls vp9 fmp4', media('video', B + '/m/rc/hls-vp9/media.m3u8'), {{ build: true }});
  // Cosmetic filtering: the player wrappers on the page since its start.
  for (const w of document.querySelectorAll('body > div[class]')) {{
    const s = getComputedStyle(w);
    log({{ name: 'wrapper .' + w.className.split(' ')[0], ok: s.display !== 'none' && w.getBoundingClientRect().height > 0,
      hidden: s.display === 'none' || s.visibility === 'hidden' || w.getBoundingClientRect().height === 0 }});
  }}
  // The autoplay policy, no user gesture yet.
  await attempt('bundled autoplay muted', media('video', B + '/m/rc/bundled.webm', {{ autoplay: true, mutedAttr: true }}), {{ play: false }});
  await attempt('autoplay with sound, no gesture', media('video', B + '/m/rc/bundled.webm', {{ autoplay: true, muted: false }}), {{ play: false }});
  await attempt('play() with sound, no gesture', media('video', B + '/m/rc/bundled.webm', {{ muted: false }}));
  // A cross-origin iframe (`localhost` is another site than `127.0.0.1`).
  const frame = document.createElement('iframe');
  frame.src = 'http://localhost:' + location.port + '/frame.html';
  document.body.appendChild(frame);
  // Then a click: play() with sound inside the gesture.
  const btn = document.createElement('button');
  btn.style.cssText = 'position:fixed;inset:0;width:100%;height:100%;z-index:9';
  btn.textContent = 'click';
  document.body.appendChild(btn);
  await new Promise((r) => {{ btn.onclick = () => {{
    const v = media('video', B + '/m/rc/bundled.webm', {{ muted: false }})();
    attempt('play() with sound, after a click', () => v).then(r);
  }}; console.log('probe click me'); }});
  btn.remove();
  console.log('probe done');
}})();
</script>"#,
        clips = Value::Array(clips),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_ranges() {
        assert_eq!(byte_range("bytes=0-", 10), Some((0, 9)));
        assert_eq!(byte_range("bytes=2-4", 10), Some((2, 4)));
        assert_eq!(byte_range("bytes=2-40", 10), Some((2, 9)));
        assert_eq!(byte_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(byte_range("bytes=11-", 10), None);
        assert_eq!(byte_range("items=0-", 10), None);
        assert_eq!(byte_range("bytes=0-", 0), None);
    }

    #[test]
    fn tracking_only() {
        assert!(only_tracking_dropped(
            "https://a/b?t=1&utm_source=x",
            "https://a/b?t=1"
        ));
        assert!(!only_tracking_dropped(
            "https://a/b?t=1&sig=2",
            "https://a/b?t=1"
        ));
        assert!(!only_tracking_dropped("https://a/b?t=1", "https://c/b?t=1"));
    }
}
