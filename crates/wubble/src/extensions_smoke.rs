//! `--smoke extensions` and `--smoke extensions-none`: the extension system on only when an
//! extension is installed. Windowless (no wubble window), on a throwaway profile, run by hand.
//!
//! `extensions`: the bundled test extension (`assets/ext-smoke`: Manifest V3, a content script
//! on `http://127.0.0.1/*` at document start, a service worker answering its message) is
//! installed twice through the browser's own path (`extensions::install`, the store, the
//! settings read at start): once as its directory (a path-made id), once as a .crx built from the
//! same files (a key-made id, unpacked by wubble; installed twice, so the start's sweep must
//! leave only the second copy). A loopback page then must carry each id's mark
//! with its worker's answer `worker:wubble`, the ids must be the ones wubble-core computes, and
//! at least one renderer must be an extension process.
//!
//! On a CEF carrying wubble's patches (`patches/chromium`, `Engine::ext_apis`), the same run also
//! drives the tab, window and action APIs on wubble's tabs: the page is the directory copy's
//! `?apis=<id>`, opened as a tab of a wubble model whose requests and projection go through the
//! host's own code (`host::ext_tabs::serve` and `push`, the model's commands, `host::call` for
//! the views; only the window is missing). The worker reports
//! `windows.getCurrent/getAll/getLastFocused`, `tabs.query({active, currentWindow})`, a
//! `tabs.create({active: false})` (and its `tabs.onUpdated` `complete`), an active
//! `tabs.create` (and its `tabs.onActivated`), a `tabs.create({})` (wubble's start page, no tab
//! object), a `tabs.sendMessage` to the page and `action.setBadgeText('7')`; then the smoke
//! reads the badge, clicks the action (the worker's `action.onClicked` sets `hit` on the
//! clicked tab) and waits for the change. On a stock CEF it says so and skips that.
//!
//! `extensions-none`: nothing installed (today's `disable-extensions`); the same page loads with
//! no mark and no extension process.
//!
//! Bounded by counts of engine turns; nothing is timed or asserted on speed.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use std::time::Instant;

use wubble_core::extensions::{
    Installed, Source, crx3_unsigned, id_from_key, id_from_path, zip_stored,
};
use wubble_core::model::{Command, Model, apply};
use wubble_core::space::ContainerId;
use wubble_core::tab::TabId;
use wubble_engine::app::{Engine, EngineConfig, EngineError, ViewId};
use wubble_engine::ext_api::{ActionClick, ExtRequest};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

use crate::host::ext_tabs::{self, ExtHost};
use crate::host::{Call, call};
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

/// The page's load state, every extension mark (`id -> value`) and the tab APIs' report.
const LOOK: &str = "JSON.stringify({ready: document.readyState, marks: Object.fromEntries(\
                    [...document.documentElement.attributes]\
                    .filter(a => a.name.startsWith('data-wubble-ext-'))\
                    .map(a => [a.name.slice(16), a.value])), \
                    apis: document.documentElement.getAttribute('data-wubble-apis')})";

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
    if installed {
        // Two copies of the .crx on disk, the list names the second: the sweep the browser
        // runs at start leaves only it.
        let root = profile.extensions_dir();
        crate::extensions::sweep(&root, &stored);
        let left = wubble_engine::extensions::copies(&root);
        let named: Vec<_> = stored
            .extensions()
            .iter()
            .filter(|e| matches!(e.source, Source::Crx { .. }))
            .map(|e| e.dir().to_path_buf())
            .collect();
        if left != named || named.len() != 1 {
            return Err(format!("the sweep left {left:?}, the list names {named:?}").into());
        }
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
    // The directory copy drives the tab APIs, on a CEF that has them.
    let patched = engine.ext_apis();
    let apis = patched.then(|| want.first().cloned()).flatten();
    let page = match &apis {
        Some(id) => server.url(&format!("/article?apis={id}")),
        None => server.url("/article"),
    };
    let outcome = drive(&mut engine, &gpu, &page, &want, apis.is_some()).and_then(|look| {
        let tabs = match (&apis, &look.apis) {
            (Some(id), Some(report)) => Some(check_apis(&mut engine, id, &page, &look, report)?),
            _ => None,
        };
        Ok((look.marks, tabs, extension_targets(&mut engine, port)?))
    });
    crate::drain(&mut engine);
    let (marks, tabs, targets) = outcome?;
    match tabs {
        Some(summary) => println!("extensions: tab APIs ok ({summary})"),
        None if patched => println!("extensions: tab APIs: skipped (no extension installed)"),
        None => println!("extensions: tab APIs: skipped (CEF without the wubble patches)"),
    }
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

/// What the page showed when [`drive`] stopped looking.
struct Seen {
    marks: BTreeMap<String, String>,
    /// The tab APIs' report (`data-wubble-apis`), JSON.
    apis: Option<String>,
    /// The page's view, and the views of the tabs the worker opened, in the order opened.
    page: ViewId,
    opened: Vec<ViewId>,
    /// The URLs of the model's tabs.
    urls: Vec<String>,
}

/// The host as far as the extension tab APIs see it, without a window: a model, its tabs'
/// views, and the host's own effect-to-engine map (`host::call`) for opening and closing
/// them. Requests go through the host's own `ext_tabs::serve` and `push`.
struct Stand<'a> {
    engine: &'a mut Engine,
    gpu: &'a GpuContext,
    model: Model,
    views: BTreeMap<TabId, ViewId>,
    /// A view that failed to open.
    failed: Option<EngineError>,
}

impl ExtHost for Stand<'_> {
    fn model(&self) -> &Model {
        &self.model
    }

    fn views(&self) -> &BTreeMap<TabId, ViewId> {
        &self.views
    }

    fn tab_of(&self, view: ViewId) -> Option<TabId> {
        self.views
            .iter()
            .find_map(|(&tab, &v)| (v == view).then_some(tab))
    }

    fn engine(&mut self) -> &mut Engine {
        self.engine
    }

    fn run(&mut self, command: Command) {
        let model = std::mem::replace(&mut self.model, Model::new(Instant::now()));
        let (model, effects) = apply(model, command);
        self.model = model;
        for effect in &effects {
            match call(effect, &self.model) {
                Call::OpenView { tab, url } if !self.views.contains_key(&tab) => {
                    let container = self.model.tabs.get(&tab).map(|t| t.container);
                    let config = ViewConfig {
                        url,
                        layout: Layout {
                            size: FrameSize::new(800, 600),
                            scale: 1.0,
                            area: None,
                        },
                        frame_rate: 30,
                        begin_frames: false,
                        gpu: self.gpu.presenter().context().clone(),
                        container: container.unwrap_or(ContainerId::DEFAULT),
                    };
                    match self.engine.open_view(config) {
                        Ok(view) => {
                            self.views.insert(tab, view);
                        }
                        Err(e) => self.failed = Some(e),
                    }
                }
                Call::CloseView(tab) => {
                    if let Some(view) = self.views.remove(&tab) {
                        self.engine.close_view(view);
                    }
                }
                _ => {}
            }
        }
        ext_tabs::push(self);
    }
}

/// Opens the page as the model's tab and looks at it until it shows the marks `want` with
/// their answers (none: until it has loaded) and, with `apis`, the tab APIs' report,
/// meanwhile serving the extensions' requests through the host's code ([`Stand`]).
fn drive(
    engine: &mut Engine,
    gpu: &GpuContext,
    url: &str,
    want: &[String],
    apis: bool,
) -> Result<Seen, Box<dyn Error>> {
    let mut stand = Stand {
        engine,
        gpu,
        model: Model::new(Instant::now()),
        views: BTreeMap::new(),
        failed: None,
    };
    stand.run(Command::NewTab {
        url: url.into(),
        container: None,
        opener: None,
    });
    let view = *stand.views.values().next().ok_or("the page got no view")?;
    let mut asked = None;
    let mut seen = Seen {
        marks: BTreeMap::new(),
        apis: None,
        page: view,
        opened: vec![],
        urls: vec![],
    };
    for turn in 0..MAX_TURNS {
        stand.engine.tick();
        std::thread::sleep(TURN_PAUSE);
        ext_tabs::serve(&mut stand);
        if let Some(e) = stand.failed.take() {
            return Err(e.into());
        }
        for (from, event) in stand.engine.take_page_events() {
            if let Some(answer) = answer_of(view, asked, from, &event) {
                asked = None;
                let (ready, marks, report) = answer?;
                let done = if want.is_empty() {
                    ready == "complete"
                } else {
                    want.iter()
                        .all(|id| marks.get(id).is_some_and(|v| v == ANSWER))
                        || marks.values().any(|v| v.starts_with("error"))
                };
                seen.marks = marks;
                seen.apis = report;
                if done && (!apis || seen.apis.is_some()) {
                    // Tab ids grow, so the model's order is the order opened.
                    seen.opened = stand
                        .views
                        .values()
                        .copied()
                        .filter(|v| *v != view)
                        .collect();
                    seen.urls = stand.model.tabs.values().map(|t| t.url.clone()).collect();
                    return Ok(seen);
                }
            }
        }
        if asked.is_none() && turn % LOOK_EVERY == 0 {
            asked = stand.engine.view(view).ok_or("the view closed")?.eval(LOOK);
        }
    }
    Err(format!(
        "the page never showed its marks; last {:?}, tab APIs {:?}",
        seen.marks, seen.apis
    )
    .into())
}

/// Checks the worker's report, then the action: badge `7`, a click, its change, badge `hit`.
fn check_apis(
    engine: &mut Engine,
    id: &str,
    page_url: &str,
    seen: &Seen,
    report: &str,
) -> Result<String, Box<dyn Error>> {
    let r: serde_json::Value =
        serde_json::from_str(report).map_err(|e| format!("tab APIs report {report:?}: {e}"))?;
    let bad = |what: &str| format!("tab APIs: {what}; report {report}");
    if let Some(error) = r["error"].as_str() {
        return Err(bad(&format!("the worker failed: {error}")).into());
    }
    let w = &r["windows"];
    let current = w["current"]
        .as_i64()
        .ok_or_else(|| bad("no current window"))?;
    if w["last"].as_i64() != Some(current)
        || w["all"] != serde_json::json!([current])
        || w["type"] != "normal"
    {
        return Err(bad("windows.getCurrent, getAll and getLastFocused disagree").into());
    }
    let q = &r["query"];
    if q["index"] != 0
        || q["url"].as_str() != Some(page_url)
        || q["window"].as_i64() != Some(current)
        || q["id"] != r["sender"]
    {
        return Err(bad("tabs.query({active, currentWindow}) is not the page at index 0").into());
    }
    let c = &r["created"];
    if c["index"] != 1 || c["active"] != false || c["window"].as_i64() != Some(current) {
        return Err(bad("tabs.create's tab is not the inactive second tab").into());
    }
    let opened = seen
        .opened
        .first()
        .copied()
        .filter(|v| engine.has_view(*v))
        .ok_or_else(|| bad("tabs.create opened no view"))?;
    let shown = &r["shown"];
    if shown["active"] != true || shown["index"] != 2 || seen.opened.len() != 2 {
        return Err(bad("the active tabs.create is not the shown third tab with a view").into());
    }
    if r["start"] != "none" || !seen.urls.iter().any(|u| u == "wubble://start") {
        return Err(bad(&format!(
            "tabs.create({{}}) did not open the start page without a tab object; tabs {:?}",
            seen.urls
        ))
        .into());
    }
    if r["echo"] != "tab" {
        return Err(bad("tabs.sendMessage did not reach the page").into());
    }
    let badge = |engine: &Engine| engine.action_state(id, seen.page).map(|s| s.badge);
    if badge(engine).as_deref() != Some("7") {
        return Err(bad(&format!("the badge is {:?}, not 7", badge(engine))).into());
    }
    engine.take_ext_requests();
    let click = engine.action_click(id, seen.page);
    if click != ActionClick::Clicked {
        return Err(bad(&format!("the action click did {click:?}")).into());
    }
    let mut changed = false;
    for _ in 0..MAX_TURNS {
        engine.tick();
        std::thread::sleep(TURN_PAUSE);
        changed |= engine
            .take_ext_requests()
            .iter()
            .any(|r| matches!(r, ExtRequest::ActionChanged { extension } if extension == id));
        if changed && badge(engine).as_deref() == Some("hit") {
            return Ok(format!(
                "window {current}, page tab {}, opened tab {} as view {opened:?}, shown tab {} \
                 activated, the start page opened, badge 7 then hit",
                r["sender"], c["id"], shown["id"]
            ));
        }
    }
    Err(bad(&format!(
        "after the click: changed {changed}, badge {:?}",
        badge(engine)
    ))
    .into())
}

/// The page's load state, its marks and the tab APIs' report.
type Look = Result<(String, BTreeMap<String, String>, Option<String>), Box<dyn Error>>;

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
            let apis = v["apis"].as_str().map(str::to_owned);
            Ok::<_, String>((ready, marks, apis))
        })()
        .map_err(Into::into),
    )
}
