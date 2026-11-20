//! `--smoke extensions` and `--smoke extensions-none`: the extension system on only when an
//! extension is installed. Windowless (no wubble window), on a throwaway profile, run by hand.
//!
//! `extensions`: the bundled test extension (`assets/ext-smoke`: Manifest V3, a content script
//! on `http://127.0.0.1/*` at document start, a service worker answering its message) is
//! installed twice through the browser's own path (`extensions::install`, the store, the
//! settings read at start): once as its directory (a path-made id), once as a .crx built from the
//! same files (a key-made id, unpacked by wubble). A loopback page then must carry each id's mark
//! with its worker's answer `worker:wubble`, the ids must be the ones wubble-core computes, and
//! at least one renderer must be an extension process.
//!
//! `extensions-none`: nothing installed (today's `disable-extensions`); the same page loads with
//! no mark and no extension process.
//!
//! Bounded by counts of engine turns; nothing is timed or asserted on speed.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use wubble_core::extensions::{Installed, crx3_unsigned, id_from_key, id_from_path, zip_stored};
use wubble_core::space::ContainerId;
use wubble_engine::app::{Engine, EngineConfig, EngineError, ViewId};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

use crate::persist::StoreClient;
use crate::profile::Profile;

/// The test extension's files.
const FILES: [(&str, &str); 3] = [
    (
        "manifest.json",
        include_str!("../assets/ext-smoke/manifest.json"),
    ),
    ("content.js", include_str!("../assets/ext-smoke/content.js")),
    ("worker.js", include_str!("../assets/ext-smoke/worker.js")),
];

/// The .crx copy's public key: any bytes do, the signature is not checked (only the id is
/// derived from it, by Chromium and by wubble alike).
const CRX_KEY: &[u8] = b"wubble extensions smoke key";

/// The worker's answer to the content script's `{ping: 'wubble'}`.
const ANSWER: &str = "worker:wubble";

/// Engine turns the page may take to show what it should; only stops a hang.
const MAX_TURNS: u32 = 300_000;
/// Turns between two looks at the page.
const LOOK_EVERY: u32 = 500;
const TURN_PAUSE: Duration = Duration::from_millis(1);

/// The page's load state and every extension mark, `id -> value`.
const LOOK: &str = "JSON.stringify({ready: document.readyState, marks: Object.fromEntries(\
                    [...document.documentElement.attributes]\
                    .filter(a => a.name.startsWith('data-wubble-ext-'))\
                    .map(a => [a.name.slice(16), a.value]))})";

/// Installs the test extension both ways on `profile`; the ids Chromium must give them.
fn install(
    profile: &Profile,
    store: &StoreClient,
    list: Installed,
) -> Result<(Installed, [String; 2]), Box<dyn Error>> {
    let src = profile.dir.join("ext-smoke");
    std::fs::create_dir_all(&src)?;
    for (name, text) in FILES {
        std::fs::write(src.join(name), text)?;
    }
    let crx = profile.dir.join("ext-smoke.crx");
    let zip: Vec<(&str, &[u8])> = FILES.iter().map(|(n, t)| (*n, t.as_bytes())).collect();
    std::fs::write(&crx, crx3_unsigned(CRX_KEY, &zip_stored(&zip)))?;
    let root = profile.extensions_dir();
    let list = crate::extensions::install(store, list, &src, &root)?;
    let list = crate::extensions::install(store, list, &crx, &root)?;
    let ids = [
        id_from_path(&src.canonicalize()?).to_string(),
        id_from_key(CRX_KEY).to_string(),
    ];
    Ok((list, ids))
}

/// The list as the next start reads it: the store's rows after the writes above.
fn stored_list(store: &StoreClient) -> Result<Installed, Box<dyn Error>> {
    let (tx, rx) = std::sync::mpsc::channel();
    store.read_settings(move |rows| {
        let _ = tx.send(rows);
    });
    let rows = rx.recv().map_err(|_| "the store thread ended")?;
    Ok(crate::persist::Settings {
        stored: rows,
        ..Default::default()
    }
    .extensions())
}

pub fn run(
    config: Result<EngineConfig, EngineError>,
    dir: &Path,
    installed: bool,
) -> Result<(), Box<dyn Error>> {
    let profile = Profile { dir: dir.into() };
    std::fs::create_dir_all(&profile.dir)?;
    let (store, loaded) = StoreClient::spawn(
        profile.db_path(),
        Box::new(wubble_vault::FakeKeyRing::default()),
    );
    let loaded = loaded.recv().map_err(|_| "the store thread ended")??;
    let (list, want) = if installed {
        let (list, ids) = install(&profile, &store, loaded.settings.extensions())?;
        (list, ids.to_vec())
    } else {
        (loaded.settings.extensions(), vec![])
    };
    let stored = stored_list(&store);
    store.shutdown();
    let stored = stored?;
    if stored != list {
        return Err(format!("the store kept another list: {}", stored.to_json()).into());
    }
    let paths = wubble_core::extensions::load_paths(&stored);
    println!(
        "extensions: {} installed, loading {:?}",
        stored.extensions().len(),
        paths
    );

    let server = crate::load_server::LoadServer::start(0, false)?;
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let config = config?;
    // The DevTools endpoint lists every target, service workers included: on Linux the
    // zygote's children keep its command line, so `--extension-process` is not to be seen there.
    let port = crate::page_smoke::free_port()?;
    let mut engine = Engine::start(EngineConfig {
        extensions: paths,
        features: wubble_engine::app::PageFeatures {
            remote_debugging_port: Some(port),
            ..config.features
        },
        ..config
    })?;
    let outcome = drive(&mut engine, &gpu, &server.url("/article"), &want)
        .and_then(|marks| Ok((marks, extension_targets(&mut engine, port)?)));
    crate::drain(&mut engine);
    let (marks, targets) = outcome?;
    println!("extensions: marks {marks:?}, extension targets {targets:?}");
    let expected: BTreeMap<String, String> = want
        .iter()
        .map(|id| (id.clone(), ANSWER.to_owned()))
        .collect();
    if marks != expected {
        return Err(format!("the page's marks are {marks:?}, not {expected:?}").into());
    }
    let mut workers: Vec<String> = want
        .iter()
        .map(|id| format!("service_worker chrome-extension://{id}/worker.js"))
        .collect();
    workers.sort();
    let mut distinct = targets.clone();
    distinct.dedup();
    if distinct != workers {
        return Err(format!("the extension targets are {targets:?}, not {workers:?}").into());
    }
    // Each worker shows twice on CEF 156: most likely one copy per Chrome profile in use (CEF's
    // global one and the view's container; docs/research/extensions.md). Counted, not asserted.
    println!(
        "smoke: extensions ok ({} loaded, {} extension service workers running)",
        want.len(),
        targets.len()
    );
    Ok(())
}

/// `type url` of every target of an extension (`chrome-extension://`), sorted, from the
/// DevTools endpoint's `/json/list`. Asked from another thread while this one keeps CEF
/// turning: Chromium builds the list on its UI thread, which is this one.
fn extension_targets(engine: &mut Engine, port: u16) -> Result<Vec<String>, Box<dyn Error>> {
    let fetch = std::thread::spawn(move || -> Result<String, String> {
        use std::io::{BufRead, BufReader, Read, Write};
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
        // Only stops a hang.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
        write!(
            stream,
            "GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
        )
        .map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("the DevTools endpoint closed before its headers ended".into());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().map_err(|_| "a bad Content-Length")?;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).map_err(|e| e.to_string())?;
        String::from_utf8(body).map_err(|e| e.to_string())
    });
    for _ in 0..MAX_TURNS {
        if fetch.is_finished() {
            break;
        }
        engine.tick();
        std::thread::sleep(TURN_PAUSE);
    }
    if !fetch.is_finished() {
        return Err("the DevTools endpoint never answered".into());
    }
    let body = fetch.join().map_err(|_| "the fetch panicked")??;
    let list: Vec<serde_json::Value> =
        serde_json::from_str(&body).map_err(|e| format!("/json/list: {e}: {body:?}"))?;
    let mut out: Vec<String> = list
        .iter()
        .filter_map(|t| {
            let url = t["url"].as_str()?;
            url.starts_with("chrome-extension://")
                .then(|| format!("{} {url}", t["type"].as_str().unwrap_or("?")))
        })
        .collect();
    out.sort();
    Ok(out)
}

/// Opens the page and looks at it until it shows the marks `want` with their answers (none:
/// until it has loaded); the marks it had then.
fn drive(
    engine: &mut Engine,
    gpu: &GpuContext,
    url: &str,
    want: &[String],
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let view = engine.open_view(ViewConfig {
        url: url.into(),
        layout: Layout {
            size: FrameSize::new(800, 600),
            scale: 1.0,
            area: None,
        },
        frame_rate: 30,
        begin_frames: false,
        gpu: gpu.presenter().context().clone(),
        container: ContainerId::DEFAULT,
    })?;
    let mut asked = None;
    let mut last = BTreeMap::new();
    for turn in 0..MAX_TURNS {
        engine.tick();
        std::thread::sleep(TURN_PAUSE);
        for (from, event) in engine.take_page_events() {
            if let Some(answer) = answer_of(view, asked, from, &event) {
                asked = None;
                let (ready, marks) = answer?;
                let done = if want.is_empty() {
                    ready == "complete"
                } else {
                    want.iter()
                        .all(|id| marks.get(id).is_some_and(|v| v == ANSWER))
                        || marks.values().any(|v| v.starts_with("error"))
                };
                last = marks;
                if done {
                    return Ok(last);
                }
            }
        }
        if asked.is_none() && turn % LOOK_EVERY == 0 {
            asked = engine.view(view).ok_or("the view closed")?.eval(LOOK);
        }
    }
    Err(format!("the page never showed its marks; last {last:?}").into())
}

type Look = Result<(String, BTreeMap<String, String>), Box<dyn Error>>;

/// The parsed answer of the look `asked`, if `event` is it.
fn answer_of(
    view: ViewId,
    asked: Option<wubble_engine::cdp::EvalId>,
    from: ViewId,
    event: &PageEvent,
) -> Option<Look> {
    let PageEvent::EvalResult { id, result } = event else {
        return None;
    };
    if from != view || Some(*id) != asked {
        return None;
    }
    Some(
        (|| {
            let text = result.clone()?;
            // The value comes back as JSON text; the script's value is itself JSON text.
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let v: serde_json::Value = match v.as_str() {
                Some(inner) => serde_json::from_str(inner).map_err(|e| e.to_string())?,
                None => v,
            };
            let ready = v["ready"].as_str().unwrap_or_default().to_owned();
            let marks = v["marks"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                .collect();
            Ok::<_, String>((ready, marks))
        })()
        .map_err(Into::into),
    )
}
