//! `--smoke mcp`: an MCP client on the browser's own socket, as a harness's `wubble mcp` would
//! be. On a throwaway profile with `agent.mcp_enabled` on, a thread connects and runs, one line
//! per step: discovery, the tool list, a `start_session` that asks for a secret and is denied on
//! the approval sheet, `start_session` (headless; allowed on the sheet), at once `navigate` to the bundled page on loopback and
//! `read_page` (the new tab's own `about:blank` load must not pass for the page's), that the
//! session sees only its own tab, two clicks on its button by ref (each its own single click)
//! and a wait for what the click wrote, typing into its field and a wait for what the input
//! wrote, `evaluate`, a `screenshot` (a PNG that decodes), a tab opened and closed,
//! `stop_session`, and that the handle is dead afterwards; last a second session is started and
//! left running: the browser's quit must close its Space. The host then checks that the user's
//! recently-closed list and history hold nothing of the agent's.
//! The sheet is the real one: the host reads what its window shows (the client, the secret names,
//! the sites, the window choice) and answers it with the pointer, through the window's event
//! path (`Host::mcp_smoke_answer`): Deny for the first, Allow for the second (headless, as asked),
//! and for the third a press on Windowed, then Allow; the Space each approval opens must be in
//! the mode the sheet showed. Last the host switches between that windowed session's two tabs
//! in the window, and hides the sidebar while the second shows (its pane widens): the first must
//! come back at its pane's new size, not its hidden-tab snapshot nor its old size.
//! Every step waits for its answer: the browser bounds each call by its deadline. Run by hand
//! (it opens a window), under `scripts/headless.sh` and `timeout`.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, Stream};
use serde_json::{Value, json};

/// The page on loopback (`autofill_smoke::serve` serves it at `/mcp`).
pub const PAGE: &str = r##"<!doctype html><html lang="en"><head><title>mcp</title></head>
<body style="font:16px sans-serif">
<h1>Agent page</h1>
<label for="name">Name</label>
<input id="name" oninput="document.getElementById('out').textContent='typed '+this.value">
<button onclick="window.clicks=(window.clicks||'')+event.detail;document.getElementById('out').textContent='pressed';document.title='pressed'">Press</button>
<p id="out">idle</p>
<p>LONG_TEXT</p>
<a href="/mcp/next?from=link">LONG_LINK</a>
<form action="/mcp/next" method="get"><label for="q">Query</label> <input id="q" name="q"></form>
<label for="note">Note</label> <input id="note" value="only in a field">
<button onclick="setTimeout(()=>{history.pushState({},'','/mcp/pushed');document.getElementById('out').textContent='pushed'},800)">Push</button>
<button onclick="setTimeout(()=>document.getElementById('going').remove(),200)">Hide</button>
<p id="going">Going away</p>
<button onclick="window.open('chrome://version')">Open chrome</button>
<button onclick="window.open('file:///etc/passwd')">Open file</button>
<button onclick="location.href='file:///etc/passwd'">Go file</button>
<a href="ELSEWHERE">Elsewhere link</a>
<button onclick="location.href='ELSEWHERE'">Go elsewhere</button>
<button onclick="window.open('ELSEWHERE')">Open elsewhere</button>
<form action="ELSEWHERE" method="get"><label for="far">Far</label> <input id="far" name="q"></form>
</body></html>"##;

/// A paragraph longer than the first tests saw `read_page` keep (about 120 characters).
const LONG_TEXT: &str = "A paragraph of plain prose that runs on well past one hundred and twenty \
     characters, so that a reader of the accessibility tree can tell whether its text arrives \
     whole or cut short somewhere along the way: the end is here.";
const LONG_LINK: &str = "A link whose name also runs past one hundred and twenty characters, \
     to show that a link's text and its target both come through whole: link end";

/// Where the form and the link lead.
const NEXT: &str = r##"<!doctype html><html lang="en"><head><title>next</title></head>
<body style="font:16px sans-serif"><h1>Next page</h1></body></html>"##;

/// GitHub's profile tabs as the first tests found them: a sticky bar under the header, its
/// links in a navigation strip that scrolls sideways (`overflow-x: auto`), each link an icon,
/// a label and a counter; a tall page below with two plain-coloured boxes, one far down.
const STICKY: &str = r##"<!doctype html><html lang="en"><head><title>sticky</title></head>
<body style="margin:0;font:16px sans-serif">
<header style="height:64px;background:#222;color:#fff">Header</header>
<div style="position:sticky;top:0;z-index:2;background:#fff;border-bottom:1px solid #ccc">
<nav aria-label="User" style="overflow-x:auto;overflow-y:hidden;white-space:nowrap;padding:0 16px">
<ul style="display:inline-flex;list-style:none;margin:0;padding:0;min-width:max-content">
<li><a href="#overview" onclick="document.getElementById('tab').textContent='at overview'" style="display:inline-flex;gap:8px;padding:8px 16px;align-items:center"><svg width="16" height="16" aria-hidden="true"><circle cx="8" cy="8" r="6"/></svg><span>Overview</span></a></li>
<li><a href="#repositories" onclick="document.getElementById('tab').textContent='at repositories'" style="display:inline-flex;gap:8px;padding:8px 16px;align-items:center"><svg width="16" height="16" aria-hidden="true"><rect width="12" height="12"/></svg><span>Repositories</span><span style="border-radius:8px;background:#ddd;padding:0 6px">12</span></a></li>
</ul></nav></div>
<main style="padding:16px">
<p id="tab">at start</p>
<button style="display:block;width:200px;height:120px;border:0;padding:0;background:rgb(0,128,255);color:rgb(0,128,255)">Box</button>
<button style="display:block;margin-top:2400px;width:200px;height:60px;border:0;padding:0;background:rgb(255,64,0);color:rgb(255,64,0)">Far</button>
</main></body></html>"##;

/// The pointer tools' page: a hover target, a canvas that logs its pointer, a drag between two
/// buttons, and, far down, a scrolling pane.
const TOOLS: &str = r##"<!doctype html><html lang="en"><head><title>tools</title></head>
<body style="margin:0;font:16px sans-serif">
<p id="log">nothing yet</p>
<button onmouseenter="document.getElementById('log').textContent='hovered'">Hover me</button>
<canvas id="c" width="300" height="200" aria-label="Sketch" style="display:block;border:1px solid #000"></canvas>
<button id="src">Source</button> <button id="dst" style="margin-left:200px">Target</button>
<div style="height:2000px"></div>
<div id="pane" style="height:100px;overflow:auto;border:1px solid #000"><p style="height:600px;margin:0">Inside the pane</p><p>Pane end</p></div>
<p>Bottom</p>
<script>
const log = t => document.getElementById('log').textContent = t;
const c = document.getElementById('c');
let path = [];
// from the canvas's box, as a drawing app reads the pointer
const at = e => { const r = c.getBoundingClientRect(); return Math.round(e.clientX - r.left) + ',' + Math.round(e.clientY - r.top); };
c.addEventListener('pointerdown', e => { path = [at(e)]; });
c.addEventListener('pointermove', e => { if (e.buttons) path.push('m'); });
c.addEventListener('pointerup', e => log('canvas from ' + path[0] + ' to ' + at(e) + ' moved ' + (path.length > 1)));
let from = null;
document.getElementById('src').addEventListener('mousedown', () => { from = 'Source'; });
document.addEventListener('mouseup', e => { if (from) log('dropped ' + from + ' on ' + (e.target.id || e.target.tagName)); from = null; });
</script></body></html>"##;

/// The form tools' page: a `<select>` (a label with a no-break space, a disabled option, two equal
/// labels), one in a disabled fieldset, a popup button, a plain field, an editor stand-in that
/// focuses an off-screen textarea beside it on mousedown (as Monaco, CodeMirror 5 and Ace do),
/// two custom widgets: one that takes focus itself, one that takes none (focus stays where it
/// was), a bare file input and a custom upload button that clicks a hidden one.
const FORMS: &str = r##"<!doctype html><html lang="en"><head><title>forms</title></head>
<body style="font:16px sans-serif">
<label for="city">City</label>
<select id="city" oninput="window.ev=(window.ev||'')+'i'" onchange="window.ev=(window.ev||'')+'c';document.getElementById('picked').textContent='picked '+this.value">
<option value="">Pick one</option>
<option value="yyz">Toronto,&nbsp;ON</option>
<option value="yul">Montreal</option>
<option value="sold" disabled>Sold out</option>
<option value="par1">Paris</option>
<option value="par2">Paris</option>
</select>
<p id="picked">nothing picked</p>
<fieldset disabled><legend>Locked</legend><label for="locked">Size</label> <select id="locked"><option>One</option><option>Two</option></select></fieldset>
<button aria-haspopup="listbox">Colour</button>
<p><label for="a">Field A</label> <input id="a"></p>
<div style="position:relative">
<div id="ed" role="textbox" aria-label="Editor" style="width:300px;height:40px;border:1px solid #000">code</div>
<textarea id="hidden" aria-hidden="true" tabindex="-1" style="position:absolute;left:-9999px;top:0;width:1px;height:1px"></textarea>
</div>
<div id="w1" tabindex="0" role="button" aria-label="Widget" style="width:120px;height:30px;border:1px solid #000">Widget</div>
<div id="w2" role="button" aria-label="Chooser" style="width:120px;height:30px;border:1px solid #000">Chooser</div>
<label for="resume">Resume</label>
<input id="resume" type="file" accept=".pdf">
<input id="hidden-file" type="file" style="display:none" onchange="document.getElementById('custom-out').textContent='attached '+Array.from(this.files).map(f=>f.name).join(',')">
<button id="custom-btn" type="button" onclick="document.getElementById('hidden-file').click()">Upload cover</button>
<p id="custom-out">no cover</p>
<script>
const hidden = document.getElementById('hidden');
document.getElementById('ed').addEventListener('mousedown', e => { e.preventDefault(); hidden.focus(); });
// takes no focus: a press leaves it where it was
document.getElementById('w2').addEventListener('mousedown', e => e.preventDefault());
</script></body></html>"##;

/// The site whose sign-ins the owner shares with a session, and one the owner is signed in to
/// that the session's sites leave out (`.localhost`: loopback).
pub const SIGNIN_HOST: &str = "signin.localhost";
pub const OTHER_HOST: &str = "other.localhost";
/// The owner's saved login on [`SIGNIN_HOST`] (seeded), bound on the sheet.
pub const SIGNIN_USER: &str = "agent-user";
pub const SIGNIN_PASSWORD: &str = "s1gn-in-pass";

/// Where the owner's first tab goes: the server signs the owner in to both sites (cookies set
/// on redirects, so each is the site's own) and ends on the echo page.
pub fn sign_in(port: u16) -> String {
    format!("http://{SIGNIN_HOST}:{port}/mcp/cookies/set")
}

/// The echo page: the cookies its request carried (an httpOnly one too: the server sees it,
/// the page's script does not), and a login form whose input is echoed into the page.
const COOKIES: &str = r##"<!doctype html><html lang="en"><head><title>cookies</title></head>
<body style="font:16px sans-serif">
<p>sent: SENT</p>
<label for="u">User</label> <input id="u" autocomplete="username" oninput="document.getElementById('echo').textContent='typed '+this.value">
<label for="p">Pass</label> <input id="p" type="password" autocomplete="current-password">
<p id="echo">nothing typed</p>
</body></html>"##;

/// The pages and redirects the smoke's server answers under `/mcp`.
pub enum Route {
    Page(String),
    Redirect(String),
    /// A redirect that sets cookies (`Set-Cookie` values).
    SetCookies {
        cookies: Vec<String>,
        to: String,
    },
}

/// What `path` answers (`autofill_smoke::serve`), given the request's `Cookie` header; `None`:
/// not found.
pub fn route(path: &str, port: u16, cookie: &str) -> Option<Route> {
    let page = |p: &str| Some(Route::Page(p.to_owned()));
    // a loopback host the first session's sites leave out until `request_more` adds it
    let elsewhere = format!("http://a.localhost:{port}/mcp/next");
    match path.split_once('?').map_or(path, |(p, _)| p) {
        "/mcp" | "/mcp/pushed" => Some(Route::Page(
            PAGE.replace("LONG_TEXT", LONG_TEXT)
                .replace("LONG_LINK", LONG_LINK)
                .replace("ELSEWHERE", &elsewhere),
        )),
        "/mcp/to-elsewhere" => Some(Route::Redirect(elsewhere)),
        "/mcp/next" => page(NEXT),
        "/mcp/sticky" => page(STICKY),
        "/mcp/tools" => page(TOOLS),
        "/mcp/forms" => page(FORMS),
        // a server's redirect off the web: Chromium must not follow it for a session
        "/mcp/to-file" => Some(Route::Redirect("file:///etc/passwd".into())),
        "/mcp/to-chrome" => Some(Route::Redirect("chrome://version".into())),
        // the owner's sign-ins: a host-only httpOnly cookie and a domain one on the shared
        // site, then an httpOnly one on the other site
        "/mcp/cookies/set" => Some(Route::SetCookies {
            cookies: vec![
                "sid=owner-signin; HttpOnly; Path=/; SameSite=Lax".into(),
                format!("pref=owner-pref; Domain={SIGNIN_HOST}; Path=/; Max-Age=3600"),
            ],
            to: format!("http://{OTHER_HOST}:{port}/mcp/cookies/set-other"),
        }),
        "/mcp/cookies/set-other" => Some(Route::SetCookies {
            cookies: vec!["other=owner-other; HttpOnly; Path=/".into()],
            to: format!("http://{SIGNIN_HOST}:{port}/mcp/cookies"),
        }),
        "/mcp/cookies" => Some(Route::Page(COOKIES.replace(
            "SENT",
            &if cookie.is_empty() {
                "none".to_owned()
            } else {
                cookie.replace(['<', '>', '&'], "")
            },
        ))),
        _ => None,
    }
}

/// How the host answers each approval sheet, in the order they come (`Host::mcp_smoke_answer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Deny,
    /// Allow, in the window mode the client asked for.
    Allow,
    /// Windowed pressed first (the client asked for headless), then Allow.
    WindowedAllow,
    /// A site typed into the Sites editor's draft, Escape (through the host's own key path:
    /// it must drop the draft, not deny), then Allow.
    EscapeDraftThenAllow,
    /// "Use my sign-ins for these sites" pressed on, each secret name bound to the seeded
    /// login (through the pickers' callback), then Allow.
    ShareBindAllow,
}

/// One sheet: what it must show, and the press that answers it.
pub struct Sheet {
    pub more: bool,
    pub secrets: &'static [&'static str],
    pub sites: &'static [&'static str],
    pub headless: bool,
    pub press: Press,
}

pub const SHEETS: &[Sheet] = &[
    Sheet {
        more: false,
        secrets: &["GITHUB_PASSWORD"],
        sites: &["example.com"],
        headless: false,
        press: Press::Deny,
    },
    Sheet {
        more: false,
        secrets: &[],
        sites: &["127.0.0.1"],
        headless: true,
        press: Press::Allow,
    },
    // `request_more`: only what is new (127.0.0.1 is granted already)
    Sheet {
        more: true,
        secrets: &["NPM_TOKEN"],
        sites: &["a.localhost"],
        headless: true,
        press: Press::Deny,
    },
    Sheet {
        more: true,
        secrets: &["NPM_TOKEN"],
        sites: &["a.localhost"],
        headless: true,
        press: Press::EscapeDraftThenAllow,
    },
    // the owner's sign-ins shared and both names bound to the saved login
    Sheet {
        more: false,
        secrets: &["SIGNIN_USER", "SIGNIN_PASSWORD"],
        sites: &[SIGNIN_HOST],
        headless: true,
        press: Press::ShareBindAllow,
    },
    // more: the other site, its sign-ins not shared (the sheet's default)
    Sheet {
        more: true,
        secrets: &[],
        sites: &[OTHER_HOST],
        headless: true,
        press: Press::Allow,
    },
    Sheet {
        more: false,
        secrets: &[],
        sites: &[],
        headless: true,
        press: Press::WindowedAllow,
    },
    // the session the Spaces popup's dustbin stops
    Sheet {
        more: false,
        secrets: &[],
        sites: &[],
        headless: true,
        press: Press::Allow,
    },
];

/// What the smoke types.
const TYPED: &str = "wubble";

/// Runs the client on its own thread; its result goes to the host, which ends the run.
pub fn start(socket: PathBuf, page: String) {
    let spawned = std::thread::Builder::new()
        .name("mcp-smoke".into())
        .spawn(move || {
            // The connection stays open to the end, so the quit, not a disconnect, ends the
            // second session.
            let result = run(&socket, &page).map(|(done, client)| {
                std::mem::forget(client);
                done
            });
            let _ = slint::invoke_from_event_loop(move || {
                crate::with_host(|host| host.mcp_smoke_done(result));
            });
        });
    if let Err(e) = spawned {
        eprintln!("smoke: mcp: no client thread: {e}");
    }
}

struct Client {
    stream: Stream,
    framer: wubble_mcp::LineFramer,
    lines: VecDeque<String>,
    next: i64,
    steps: usize,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stream
            .write_all(wubble_mcp::encode_line(&message).as_bytes())
            .map_err(|e| format!("send {method}: {e}"))?;
        loop {
            let line = self.line().map_err(|e| format!("read {method}: {e}"))?;
            let v: Value = serde_json::from_str(&line).map_err(|e| format!("{method}: {e}"))?;
            if v["id"] == id {
                return match v.get("error") {
                    Some(e) => Err(format!("{method}: {e}")),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    }

    /// The next line from the browser.
    fn line(&mut self) -> Result<String, String> {
        let mut buf = vec![0u8; 64 * 1024];
        while self.lines.is_empty() {
            let n = self.stream.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("the browser closed the connection".into());
            }
            for line in self.framer.push(&buf[..n]) {
                self.lines.push_back(line.map_err(|e| format!("{e:?}"))?);
            }
        }
        self.lines.pop_front().ok_or_else(String::new)
    }

    /// A tool's result, or its error text.
    fn tool(&mut self, name: &str, args: Value) -> Result<Result<Value, String>, String> {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}))?;
        Ok(if r["isError"] == true {
            Err(text(&r))
        } else {
            Ok(r)
        })
    }

    /// A tool that must succeed.
    fn ok(&mut self, name: &str, args: Value) -> Result<Value, String> {
        self.tool(name, args)?
            .map_err(|e| format!("{name} failed: {e}"))
    }

    fn step(&mut self, name: &str, detail: impl std::fmt::Display) {
        self.steps += 1;
        println!("smoke: mcp {name} ok {detail}");
    }
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// A client of an earlier revision (the TypeScript SDK's, 2025-11-25): `initialize`, then
/// `notifications/initialized`, the list, a call, on its own connection.
fn legacy(socket: &Path, c: &mut Client) -> Result<(), String> {
    let name = socket
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| e.to_string())?;
    let mut old = Client {
        stream: Stream::connect(name).map_err(|e| format!("connect: {e}"))?,
        framer: wubble_mcp::LineFramer::default(),
        lines: VecDeque::new(),
        next: 0,
        steps: 0,
    };
    let init = old.request(
        "initialize",
        json!({"protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "an earlier client", "version": "1"}}),
    )?;
    if init["protocolVersion"] != "2025-11-25"
        || init["serverInfo"]["name"] != "wubble"
        || !init["instructions"]
            .as_str()
            .is_some_and(|i| i.contains("start_session"))
    {
        return Err(format!("initialize: {init}"));
    }
    old.stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .map_err(|e| e.to_string())?;
    let l = old.request("tools/list", json!({}))?;
    let tools = l["tools"].as_array().map_or(0, Vec::len);
    if tools != wubble_mcp::tools::TOOLS.len() || l.get("ttlMs").is_some() {
        return Err(format!("tools/list after initialize: {l}"));
    }
    let r = old.request(
        "tools/call",
        json!({"name": "list_tabs", "arguments": {"session": "none"}}),
    )?;
    if r["isError"] != true || r.get("resultType").is_some() {
        return Err(format!("a call after initialize: {r}"));
    }
    c.step(
        "initialize",
        format!("an earlier client (2025-11-25) served its revision: {tools} tools, calls answered in its shape"),
    );
    Ok(())
}

/// The ref on the `read_page` line that starts with `prefix`.
fn ref_of(page: &str, prefix: &str) -> Option<String> {
    let line = page.lines().find(|l| l.starts_with(prefix))?;
    let r = line.rsplit_once("[ref=")?.1.strip_suffix(']')?;
    Some(r.to_owned())
}

/// The JSON a tool answered with.
fn json_of(result: &Value) -> Result<Value, String> {
    serde_json::from_str(&text(result)).map_err(|e| format!("{e}: {}", text(result)))
}

/// A PNG tool result's size and its centre pixel.
fn picture(result: &Value) -> Result<(u32, u32, [u8; 3]), String> {
    let png = result["content"][0]["data"]
        .as_str()
        .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
        .ok_or("no image")?;
    let (w, h, rgba) = crate::shot_io::rgba(&png)?;
    let i = ((h / 2 * w + w / 2) * 4) as usize;
    let px = rgba.get(i..i + 3).ok_or("an empty picture")?;
    Ok((w, h, [px[0], px[1], px[2]]))
}

/// Whether two colours are near each other (the capture's colour handling may move a channel).
fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 12)
}

/// A session's handle and first tab from `start_session`'s answer.
fn started(result: &Value) -> Result<(String, String), String> {
    let v = json_of(result)?;
    let session = v["session"].as_str().unwrap_or_default().to_owned();
    let tab = v["tab"].as_str().unwrap_or_default().to_owned();
    if session.len() != 64 || tab.is_empty() {
        return Err(format!("start_session: {v}"));
    }
    Ok((session, tab))
}

/// What the client hands the host at its end: its step count and the tab the first session
/// left to the owner.
pub struct Done {
    pub steps: usize,
    pub kept: u64,
}

fn run(socket: &Path, page: &str) -> Result<(Done, Client), String> {
    let name = socket
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| e.to_string())?;
    let stream = Stream::connect(name).map_err(|e| format!("connect: {e}"))?;
    let mut c = Client {
        stream,
        framer: wubble_mcp::LineFramer::default(),
        lines: VecDeque::new(),
        next: 0,
        steps: 0,
    };
    let base = page.trim_end_matches("/mcp").to_owned();
    let d = c.request("server/discover", json!({}))?;
    let version = d["supportedVersions"][0]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if version != wubble_mcp::server::PROTOCOL_VERSION {
        return Err(format!("discover: {d}"));
    }
    c.step("discover", &version);
    let l = c.request("tools/list", json!({}))?;
    let tools = l["tools"].as_array().map_or(0, Vec::len);
    if tools != wubble_mcp::tools::TOOLS.len() {
        return Err(format!("tools/list: {tools} tools"));
    }
    c.step("tools/list", format!("{tools} tools"));
    legacy(socket, &mut c)?;

    // The sheet's Deny: the host's answer to the one that asks for a secret (the sheet shows it
    // by name; no value exists to ask for).
    let denied = c.tool(
        "start_session",
        json!({"purpose": "to be denied", "secrets": ["GITHUB_PASSWORD"], "sites": ["example.com"]}),
    )?;
    match denied {
        Err(why) if why.contains("denied") => c.step("start_session", &why),
        other => return Err(format!("the denied session: {other:?}")),
    }
    let (session, tab) = started(&c.ok(
        "start_session",
        json!({"purpose": "the mcp smoke", "headless": true, "sites": ["127.0.0.1"]}),
    )?)?;
    c.step(
        "start_session",
        format!("tab {tab}, headless, on 127.0.0.1, allowed on the sheet"),
    );
    let s = |extra: Value| {
        let mut m = json!({"session": session, "tab": tab});
        for (k, v) in extra.as_object().into_iter().flatten() {
            m[k] = v.clone();
        }
        m
    };

    // The scheme is read without case.
    c.ok(
        "navigate",
        s(json!({"url": page.replacen("http://", "HTTP://", 1)})),
    )?;
    c.step("navigate", format!("{page}, spelt HTTP://"));
    let read = text(&c.ok("read_page", s(json!({})))?);
    let button = ref_of(&read, "- button \"Press\"").ok_or(format!("read_page: {read}"))?;
    let field = ref_of(&read, "- textField \"Name\"").ok_or(format!("read_page: {read}"))?;
    if !read.contains(&format!("{LONG_TEXT:?}")) {
        return Err(format!("read_page cut the long paragraph: {read}"));
    }
    let link = format!("- link {LONG_LINK:?} url=\"{base}/mcp/next?from=link\"");
    if !read.lines().any(|l| l.starts_with(&link)) {
        return Err(format!(
            "read_page has no whole link with its target: {read}"
        ));
    }
    let cut = text(&c.ok("read_page", s(json!({"max_items": 2})))?);
    if cut.lines().count() != 3 || !cut.contains("[2 of ") || !cut.contains("default 500") {
        return Err(format!("read_page cut at 2: {cut}"));
    }
    match c.tool("read_page", s(json!({"ref": "e99.1"})))? {
        Err(e) if e.contains("stale ref") => {}
        other => return Err(format!("read_page with a stale ref: {other:?}")),
    }
    c.step(
        "read_page",
        format!(
            "{} items, text and link whole with its target, a cut list noted, a stale ref an error",
            read.lines().count()
        ),
    );

    let tabs = json_of(&c.ok("list_tabs", json!({"session": session}))?)?;
    if tabs.as_array().map(Vec::len) != Some(1)
        || tabs[0]["tab"] != tab.as_str()
        || tabs[0]["active"] != true
        || tabs[0]["shown"] != false
    {
        return Err(format!("list_tabs: {tabs}"));
    }
    c.step("list_tabs", "1 tab, active, not shown (headless)");
    // Every other tab handle, the user's own tab among them, is unknown to the session.
    let own: u64 = tab.parse().map_err(|_| format!("tab handle {tab}"))?;
    for other in (0..=own + 1).filter(|&t| t != own) {
        let other = json!({"session": session, "tab": other.to_string()});
        for (tool, extra) in [
            ("navigate", json!({"url": page})),
            ("read_page", json!({})),
            ("focus_tab", json!({})),
            ("close_tab", json!({})),
            ("evaluate", json!({"script": "1"})),
        ] {
            let mut args = other.clone();
            for (k, v) in extra.as_object().into_iter().flatten() {
                args[k] = v.clone();
            }
            match c.tool(tool, args)? {
                Err(e) if e.contains("no such tab") => {}
                r => return Err(format!("{tool} on tab {other} outside the session: {r:?}")),
            }
        }
    }
    c.step(
        "isolation",
        format!(
            "tabs 0..={} refused to navigate, read_page, focus_tab, close_tab, evaluate",
            own + 1
        ),
    );

    // twice, as fast as the calls go: each is a click of its own (`event.detail` 1)
    for _ in 0..2 {
        c.ok("click", s(json!({"ref": button})))?;
    }
    c.ok("wait_for", s(json!({"text": "pressed"})))?;
    c.step("click", &button);
    c.ok("type", s(json!({"ref": field, "text": TYPED})))?;
    c.ok("wait_for", s(json!({"text": format!("typed {TYPED}")})))?;
    c.step("type", &field);

    evaluate(&mut c, &s)?;
    waiting(&mut c, &s, &read, &base)?;
    keys(&mut c, &s, page)?;
    shots(&mut c, &s, &base)?;
    pointer(&mut c, &s, &base)?;
    forms(&mut c, &s, &base)?;
    reading(&mut c, &s, &base)?;
    safety(&mut c, &s, &session, page, &base)?;

    let shot = c.ok("screenshot", s(json!({})))?;
    let (w, h, _) = picture(&shot)?;
    c.step("screenshot", format!("{w}x{h} PNG"));

    let opened = json_of(&c.ok("open_tab", json!({"session": session, "url": page}))?)?;
    let second = opened["tab"].as_str().unwrap_or_default().to_owned();
    // it answered once its page had loaded, as navigate does
    let state = text(&c.ok(
        "evaluate",
        json!({"session": session, "tab": second, "script": "document.readyState + ' ' + document.title"}),
    )?);
    if state != "\"complete mcp\"" {
        return Err(format!("open_tab answered before its load: {state}"));
    }
    c.ok("close_tab", json!({"session": session, "tab": second}))?;
    c.step(
        "open_tab",
        format!("tab {second} loaded when answered, then closed"),
    );

    // request_more: Deny keeps the session as it was; asked again, the sheet comes again;
    // Allow widens the grant. 127.0.0.1 is granted already: only a.localhost is new.
    let more = json!({"session": session, "sites": ["a.localhost", "127.0.0.1"], "secrets": ["NPM_TOKEN"]});
    match c.tool("request_more", more.clone())? {
        Err(e) if e.contains("denied") => {}
        r => return Err(format!("request_more, denied: {r:?}")),
    }
    let other_site = format!("{}/mcp", base.replace("127.0.0.1", "a.localhost"));
    match c.tool("navigate", s(json!({"url": other_site})))? {
        Err(e) if e.contains("not a site this session may visit") => {}
        r => return Err(format!("a site denied on the sheet: {r:?}")),
    }
    let granted = json_of(&c.ok("request_more", more.clone())?)?;
    if granted["granted"]["sites"] != json!(["a.localhost"]) {
        return Err(format!("request_more, allowed: {granted}"));
    }
    c.ok("navigate", s(json!({"url": other_site})))?;
    // the link refused before now goes
    c.ok("navigate", s(json!({"url": page})))?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    let link = ref_of(&read, "- link \"Elsewhere link\"").ok_or("no Elsewhere link")?;
    c.ok("click", s(json!({"ref": link})))?;
    c.ok(
        "wait_for",
        s(json!({"url": "a.localhost", "timeout_ms": 5000})),
    )
    .map_err(|e| format!("the link once a.localhost is granted: {e}"))?;
    let nothing = text(&c.ok(
        "request_more",
        json!({"session": session, "sites": ["a.localhost"]}),
    )?);
    if !nothing.contains("nothing new") {
        return Err(format!("request_more for what is granted: {nothing}"));
    }
    c.step(
        "request_more",
        "denied (the session runs on, the site refused), asked again and allowed (only a.localhost listed, then its link goes), nothing new needs no sheet",
    );

    // The hand-off: the session ends, its tab stays for the owner (checked by the host).
    let stopped = text(&c.ok(
        "stop_session",
        json!({"session": session, "keep_tabs": true}),
    )?);
    if !stopped.contains("the user's now") {
        return Err(format!("stop_session keep_tabs: {stopped}"));
    }
    for (tool, args) in [
        ("list_tabs", json!({"session": session})),
        ("read_page", json!({"session": session, "tab": tab})),
        ("stop_session", json!({"session": session})),
    ] {
        if c.tool(tool, args)?.is_ok() {
            return Err(format!("the stopped session still answers {tool}"));
        }
    }
    c.step(
        "stop_session",
        "keep_tabs: the tab stays, the handle is dead (list_tabs, read_page, stop_session refused)",
    );
    signins(&mut c, &base)?;
    // Left running: the browser's quit must end it and close its Space (checked by the host).
    // The sheet makes it windowed; two tabs on the still page, which the host switches between
    // in the window (`Host::mcp_smoke_switch`).
    let (session, tab) = started(&c.ok(
        "start_session",
        json!({"purpose": "ended by the quit", "headless": true}),
    )?)?;
    c.ok(
        "navigate",
        json!({"session": session, "tab": tab, "url": page}),
    )?;
    let opened = json_of(&c.ok("open_tab", json!({"session": session, "url": page}))?)?;
    let second = opened["tab"].as_str().unwrap_or_default().to_owned();
    let shown = |c: &mut Client, which: &str| -> Result<bool, String> {
        let tabs = json_of(&c.ok("list_tabs", json!({"session": session}))?)?;
        let row = tabs
            .as_array()
            .and_then(|a| a.iter().find(|t| t["tab"] == which))
            .ok_or(format!("list_tabs: {tabs}"))?;
        Ok(row["active"] == true && row["shown"] == true)
    };
    c.ok("focus_tab", json!({"session": session, "tab": tab}))?;
    if !shown(&mut c, &tab)? || shown(&mut c, &second)? {
        return Err("focus_tab did not bring the first tab forward".into());
    }
    c.step(
        "focus_tab",
        format!("tab {tab} active and shown, tab {second} not"),
    );
    c.step(
        "start_session",
        "a second one, windowed on the sheet, two tabs, left for the quit",
    );
    // a third, headless, whose Space the host clears with the Spaces popup's dustbin
    let (third, its_tab) = started(&c.ok(
        "start_session",
        json!({"purpose": "cleared by its dustbin", "headless": true}),
    )?)?;
    c.ok(
        "navigate",
        json!({"session": third, "tab": its_tab, "url": page}),
    )?;
    c.step("start_session", "a third one, headless, for the dustbin");
    let kept = own;
    let steps = c.steps;
    Ok((Done { steps, kept }, c))
}

/// "Use my sign-ins": a session on [`SIGNIN_HOST`] with the owner's cookies copied in and its
/// two secret names bound on the sheet. The server sees the copied cookies (the httpOnly one
/// too, which the page's script must not see); the other site's cookie is not copied (it is
/// granted later, unshared, and arrives with none); the bound names fill only on the login's
/// site, and no reply carries a filled value.
fn signins(c: &mut Client, base: &str) -> Result<(), String> {
    let signin = base.replace("127.0.0.1", SIGNIN_HOST);
    let other = base.replace("127.0.0.1", OTHER_HOST);
    let r = c.ok(
        "start_session",
        json!({"purpose": "signed in", "headless": true, "sites": [SIGNIN_HOST],
            "secrets": ["SIGNIN_USER", "SIGNIN_PASSWORD"]}),
    )?;
    let (session, tab) = started(&r)?;
    let v = json_of(&r)?;
    if v["signed_in_sites"] != json!([SIGNIN_HOST])
        || v["secrets"] != json!(["SIGNIN_PASSWORD", "SIGNIN_USER"])
    {
        return Err(format!("start_session with sign-ins: {v}"));
    }
    let s = |extra: Value| {
        let mut m = json!({"session": session, "tab": tab});
        for (k, v) in extra.as_object().into_iter().flatten() {
            m[k] = v.clone();
        }
        m
    };
    c.ok(
        "navigate",
        s(json!({"url": format!("{signin}/mcp/cookies")})),
    )?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    let sent = read
        .lines()
        .find(|l| l.contains("sent: "))
        .ok_or(format!("the echo page: {read}"))?;
    if !sent.contains("sid=owner-signin")
        || !sent.contains("pref=owner-pref")
        || sent.contains("other=")
    {
        return Err(format!("the session's request carried: {sent}"));
    }
    let script = text(&c.ok("evaluate", s(json!({"script": "return document.cookie"})))?);
    if script.contains("sid") || !script.contains("pref=owner-pref") {
        return Err(format!(
            "the page's script sees cookies {script} (sid is httpOnly)"
        ));
    }
    c.step(
        "sign-ins",
        format!("the owner's {SIGNIN_HOST} cookies arrived in the session (the server saw the httpOnly sid, the page's script did not); {OTHER_HOST}'s did not"),
    );
    let field = |c: &mut Client, label: &str| -> Result<String, String> {
        let read = text(&c.ok("read_page", s(json!({})))?);
        ref_of(&read, &format!("- textField \"{label}\""))
            .ok_or(format!("no {label} field: {read}"))
    };
    let user = field(c, "User")?;
    // a password is typed only into a password field
    match c.tool(
        "fill_secret",
        s(json!({"ref": user, "secret": "SIGNIN_PASSWORD"})),
    )? {
        Err(e) if e.contains("only into a password field") => {}
        r => return Err(format!("the password into a text field: {r:?}")),
    }
    c.ok(
        "fill_secret",
        s(json!({"ref": user, "secret": "SIGNIN_USER"})),
    )?;
    let pass = field(c, "Pass")?;
    c.ok(
        "fill_secret",
        s(json!({"ref": pass, "secret": "SIGNIN_PASSWORD"})),
    )?;
    // the page echoes the typed username into its text
    c.ok("wait_for", s(json!({"text": "typed "})))?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    if read.contains(SIGNIN_USER)
        || read.contains(SIGNIN_PASSWORD)
        || !read.contains("typed [secret]")
    {
        return Err(format!("read_page after the fills: {read}"));
    }
    // no script where a secret was typed: this tab, and a new one on the same site
    match c.tool(
        "evaluate",
        s(json!({"script": "return document.getElementById('u').value"})),
    )? {
        Err(e) if e.contains("a secret was filled") => {}
        r => return Err(format!("evaluate in the filled tab: {r:?}")),
    }
    let again = json_of(&c.ok(
        "open_tab",
        json!({"session": session, "url": format!("{signin}/mcp/cookies")}),
    )?)?;
    let again = again["tab"].as_str().unwrap_or_default().to_owned();
    match c.tool(
        "evaluate",
        json!({"session": session, "tab": again, "script": "1"}),
    )? {
        Err(e) if e.contains("a secret was filled") => {}
        r => return Err(format!("evaluate in a new tab on the filled site: {r:?}")),
    }
    // more: the other site, unshared; a bound name is refused there
    let granted = json_of(&c.ok(
        "request_more",
        json!({"session": session, "sites": [OTHER_HOST]}),
    )?)?;
    if granted["granted"]["sites"] != json!([OTHER_HOST]) {
        return Err(format!("request_more {OTHER_HOST}: {granted}"));
    }
    c.ok(
        "navigate",
        s(json!({"url": format!("{other}/mcp/cookies")})),
    )?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    if !read.contains("sent: none") {
        return Err(format!(
            "{OTHER_HOST} unshared, yet the request carried cookies: {read}"
        ));
    }
    let user = field(c, "User")?;
    match c.tool(
        "fill_secret",
        s(json!({"ref": user, "secret": "SIGNIN_USER"})),
    )? {
        Err(e) if e.contains("not the login's site") => {}
        r => return Err(format!("fill_secret on {OTHER_HOST}: {r:?}")),
    }
    // the new tab moved off the filled site: a tab never filled into may run scripts again
    c.ok(
        "navigate",
        json!({"session": session, "tab": again, "url": format!("{other}/mcp/cookies")}),
    )?;
    let two = text(&c.ok(
        "evaluate",
        json!({"session": session, "tab": again, "script": "return 1+1"}),
    )?);
    if two != "2" {
        return Err(format!("evaluate in a tab never filled into: {two}"));
    }
    c.step(
        "fill_secret",
        format!("both names filled on {SIGNIN_HOST} (no reply carries a value: [secret]; the password refused in a text field); evaluate refused in the filled tab and in a new tab on its site, allowed once that tab left it; refused on {OTHER_HOST}, which came unshared and cookieless"),
    );
    c.ok("stop_session", json!({"session": session}))?;
    Ok(())
}

/// `evaluate`: statements, a top-level return, promises, values JSON cannot carry, a throw
/// with its stack, and a script that never ends.
fn evaluate(c: &mut Client, s: &dyn Fn(Value) -> Value) -> Result<(), String> {
    let script = "document.getElementById('name').value + '|' + document.title + '|' + clicks";
    let value = text(&c.ok("evaluate", s(json!({"script": script})))?);
    if value != format!("\"{TYPED}|pressed|11\"") {
        return Err(format!("evaluate: {value}"));
    }
    for (script, want) in [
        ("let a = 2; a * 21", "42"),
        ("const x = 5;\nreturn x + 1", "6"),
        (
            "new Promise(r => setTimeout(() => r({ok: true}), 50))",
            "{\"ok\":true}",
        ),
        ("await Promise.resolve(7)", "7"),
        (
            "({el: document.body, list: new Set([1]), n: 1})",
            "{\"el\":\"[node body]\",\"list\":[1],\"n\":1}",
        ),
        ("undefined", "undefined"),
        ("NaN", "NaN"),
    ] {
        let got = text(&c.ok("evaluate", s(json!({"script": script})))?);
        if got != want {
            return Err(format!("evaluate {script:?}: {got}, not {want}"));
        }
    }
    let node = text(&c.ok("evaluate", s(json!({"script": "document.body"})))?);
    if !node.starts_with("[not JSON] a DOM node (body)") {
        return Err(format!("evaluate of a node: {node}"));
    }
    match c.tool(
        "evaluate",
        s(json!({"script": "function boom() { throw new Error('boom') }\nboom()"})),
    )? {
        Err(e) if e.contains("Error: boom") && e.contains("at boom") => {}
        r => return Err(format!("evaluate of a throw: {r:?}")),
    }
    match c.tool(
        "evaluate",
        s(json!({"script": "new Promise(() => {})", "timeout_ms": 300})),
    )? {
        Err(e) if e.contains("in time") => {}
        r => return Err(format!("evaluate of a promise that never settles: {r:?}")),
    }
    match c.tool(
        "evaluate",
        s(json!({"script": "while (true) {}", "timeout_ms": 300})),
    )? {
        Err(e) if e.contains("is stopped") => {}
        r => return Err(format!("evaluate of an endless loop: {r:?}")),
    }
    // the page runs on
    if text(&c.ok("evaluate", s(json!({"script": "document.title"})))?) != "\"pressed\"" {
        return Err("the page did not run on after a stopped script".into());
    }
    c.step(
        "evaluate",
        "statements, return, promises, await, node named, throw with stack, timeouts",
    );
    Ok(())
}

/// `wait_for`'s conditions, and `type` with `submit` waiting as `navigate` does.
fn waiting(
    c: &mut Client,
    s: &dyn Fn(Value) -> Value,
    read: &str,
    base: &str,
) -> Result<(), String> {
    let click = |c: &mut Client, name: &str| -> Result<(), String> {
        let r = ref_of(read, &format!("- button {name:?}")).ok_or(format!("no {name} button"))?;
        c.ok("click", s(json!({"ref": r}))).map(|_| ())
    };
    // a field's own value is not the page's text, unless asked
    match c.tool(
        "wait_for",
        s(json!({"text": "only in a field", "timeout_ms": 300})),
    )? {
        Err(e) if e.contains("timed out") => {}
        r => return Err(format!("wait_for matched a field's value: {r:?}")),
    }
    c.ok(
        "wait_for",
        s(json!({"text": "only in a field", "in_values": true})),
    )?;
    click(c, "Hide")?;
    c.ok("wait_for", s(json!({"text_gone": "Going away"})))?;
    // an in-page navigation (the history API, a moment after the click)
    click(c, "Push")?;
    let moved = json_of(&c.ok("wait_for", s(json!({"navigation": true})))?)?;
    if !moved["url"]
        .as_str()
        .is_some_and(|u| u.ends_with("/mcp/pushed"))
    {
        return Err(format!("wait_for navigation: {moved}"));
    }
    c.ok("wait_for", s(json!({"url": "/mcp/pushed"})))?;
    c.step(
        "wait_for",
        "field values left out (and matched with in_values), text gone, an in-page navigation, url",
    );
    // a form submit loads the next page before it answers; a field with no form answers that
    // no load followed
    let page = text(&c.ok("read_page", s(json!({})))?);
    let query = ref_of(&page, "- textField \"Query\"").ok_or(format!("read_page: {page}"))?;
    let typed = text(&c.ok(
        "type",
        s(json!({"ref": query, "text": TYPED, "submit": true})),
    )?);
    let next = text(&c.ok(
        "evaluate",
        s(json!({"script": "location.pathname + location.search + ' ' + document.readyState"})),
    )?);
    if !typed.contains("the page loaded") || next != "\"/mcp/next?q=wubble complete\"" {
        return Err(format!("type submit: {typed}; then {next}"));
    }
    c.ok("navigate", s(json!({"url": format!("{base}/mcp")})))?;
    let page = text(&c.ok("read_page", s(json!({})))?);
    let name = ref_of(&page, "- textField \"Name\"").ok_or(format!("read_page: {page}"))?;
    let stayed = text(&c.ok(
        "type",
        s(json!({"ref": name, "text": TYPED, "submit": true})),
    )?);
    if !stayed.contains("no page load followed") {
        return Err(format!("type submit with no form: {stayed}"));
    }
    c.step(
        "type",
        "submit waited for the next page; with no form, said no load followed",
    );
    Ok(())
}

/// `press_key`: wubble's history chords on the session's tab, its window chords refused, the
/// address bar's chord only to the page.
fn keys(c: &mut Client, s: &dyn Fn(Value) -> Value, page: &str) -> Result<(), String> {
    c.ok("navigate", s(json!({"url": format!("{page}/next")})))?;
    c.ok("press_key", s(json!({"key": "Alt+ArrowLeft"})))?;
    let at = text(&c.ok("evaluate", s(json!({"script": "location.pathname"})))?);
    if at != "\"/mcp\"" {
        return Err(format!("Alt+ArrowLeft did not go back: at {at}"));
    }
    let l = text(&c.ok("press_key", s(json!({"key": "Control+L"})))?);
    if !l.contains("does not run for an agent") {
        return Err(format!("Control+L: {l}"));
    }
    match c.tool("press_key", s(json!({"key": "Control+Shift+T"})))? {
        Err(e) if e.contains("not pressed") => {}
        r => return Err(format!("Control+Shift+T: {r:?}")),
    }
    c.step(
        "press_key",
        "Alt+ArrowLeft went back, Control+L said the address bar does not open, Control+Shift+T refused",
    );
    Ok(())
}

/// `screenshot` of an element at any scroll (and of one far down, scrolled to), and of the
/// whole page; `click` on the sticky bar's links at the top and scrolled.
fn shots(c: &mut Client, s: &dyn Fn(Value) -> Value, base: &str) -> Result<(), String> {
    c.ok("navigate", s(json!({"url": format!("{base}/mcp/sticky")})))?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    let find = |prefix: &str| ref_of(&read, prefix).ok_or(format!("no {prefix}: {read}"));
    let (overview, repos) = (find("- link \"Overview\"")?, find("- link \"Repositories")?);
    let (boxed, far) = (find("- button \"Box\"")?, find("- button \"Far\"")?);
    for (y, link, said) in [
        (0, &overview, "at overview"),
        (400, &repos, "at repositories"),
    ] {
        c.ok(
            "evaluate",
            s(json!({"script": format!("scrollTo(0, {y})")})),
        )?;
        c.ok("click", s(json!({"ref": link})))?;
        c.ok("wait_for", s(json!({"text": said})))?;
    }
    c.step(
        "click",
        "the sticky bar's Overview at scroll 0 and Repositories at 400",
    );
    const BLUE: [u8; 3] = [0, 128, 255];
    for y in [0, 50, 150, 300] {
        c.ok(
            "evaluate",
            s(json!({"script": format!("scrollTo(0, {y})")})),
        )?;
        let (w, h, px) = picture(&c.ok("screenshot", s(json!({"ref": boxed})))?)?;
        // the box is 200x120; at 150 and 300 the bar hides its top part
        if !(w.abs_diff(200) <= 1 && (40..=121).contains(&h) && near(px, BLUE)) {
            return Err(format!("the box at scroll {y}: {w}x{h}, centre {px:?}"));
        }
    }
    let (w, h, px) = picture(&c.ok("screenshot", s(json!({"ref": far})))?)?;
    if !(w.abs_diff(200) <= 1 && h.abs_diff(60) <= 1 && near(px, [255, 64, 0])) {
        return Err(format!("the far box: {w}x{h}, centre {px:?}"));
    }
    let (_, full, _) = picture(&c.ok("screenshot", s(json!({"full_page": true})))?)?;
    if full < 2500 {
        return Err(format!("the full page is {full} high"));
    }
    c.step(
        "screenshot",
        format!("the box cropped right at scroll 0, 50, 150, 300; the far one scrolled to; full page {full} high"),
    );
    Ok(())
}

/// `hover`, `drag` (between elements, and inside a canvas by its coordinates) and `scroll`.
fn pointer(c: &mut Client, s: &dyn Fn(Value) -> Value, base: &str) -> Result<(), String> {
    c.ok("navigate", s(json!({"url": format!("{base}/mcp/tools")})))?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    let find = |prefix: &str| ref_of(&read, prefix).ok_or(format!("no {prefix}: {read}"));
    let log = |c: &mut Client| -> Result<String, String> {
        Ok(text(&c.ok(
            "evaluate",
            s(json!({"script": "document.getElementById('log').textContent"})),
        )?))
    };
    c.ok("hover", s(json!({"ref": find("- button \"Hover me\"")?})))?;
    c.ok("wait_for", s(json!({"text": "hovered"})))?;
    let canvas = find("- canvas \"Sketch\"")?;
    c.ok(
        "drag",
        s(json!({"ref": canvas, "x": 20, "y": 30, "to_x": 120, "to_y": 80})),
    )?;
    let drawn = log(c)?;
    if drawn != "\"canvas from 20,30 to 120,80 moved true\"" {
        return Err(format!("drag in the canvas: {drawn}"));
    }
    c.ok(
        "drag",
        s(json!({"ref": find("- button \"Source\"")?, "to_ref": find("- button \"Target\"")?})),
    )?;
    let dropped = log(c)?;
    if dropped != "\"dropped Source on dst\"" {
        return Err(format!("drag between buttons: {dropped}"));
    }
    // coordinates anywhere but in a canvas are refused
    match c.tool(
        "click",
        s(json!({"ref": find("- button \"Source\"")?, "x": 1, "y": 1})),
    )? {
        Err(e) if e.contains("only inside a <canvas>") => {}
        r => return Err(format!("coordinates on a button: {r:?}")),
    }
    match c.tool("drag", s(json!({"ref": canvas, "to_x": 400, "to_y": 10})))? {
        Err(e) if e.contains("outside the canvas") => {}
        r => return Err(format!("a drag out of the canvas: {r:?}")),
    }
    c.step(
        "hover, drag",
        "hovered; drawn in the canvas 20,30 to 120,80; Source dropped on Target; coordinates outside a canvas refused",
    );
    // the wheel over the page's middle, an element scrolled to, the wheel inside a pane
    let until = |what: &str| {
        json!({"script": format!(
            "await new Promise(r => {{ const f = () => ({what}) ? r(true) : setTimeout(f, 20); f() }})"
        ), "timeout_ms": 5000})
    };
    let state = |c: &mut Client| -> String {
        let js = "JSON.stringify([scrollY, document.getElementById('pane').getBoundingClientRect().top])";
        c.ok("evaluate", s(json!({"script": js})))
            .map(|v| text(&v))
            .unwrap_or_default()
    };
    // an element far down scrolled to; the wheel over the page's middle, down and up
    c.ok(
        "scroll",
        s(json!({"ref": find("- staticText \"Bottom\"")?})),
    )?;
    c.ok(
        "evaluate",
        s(until(
            "document.getElementById('pane').getBoundingClientRect().top < innerHeight",
        )),
    )
    .map_err(|e| format!("Bottom into view: {e}; {}", state(c)))?;
    let bottom = text(&c.ok("evaluate", s(json!({"script": "scrollY"})))?);
    c.ok("scroll", s(json!({"direction": "up", "amount": 300})))?;
    c.ok("evaluate", s(until(&format!("scrollY < {bottom}"))))
        .map_err(|e| format!("the wheel up: {e}; {}", state(c)))?;
    let up = text(&c.ok("evaluate", s(json!({"script": "scrollY"})))?);
    c.ok("scroll", s(json!({"direction": "down", "amount": 100})))?;
    c.ok("evaluate", s(until(&format!("scrollY > {up}"))))
        .map_err(|e| format!("the wheel down: {e}; {}", state(c)))?;
    // Not checked: the wheel over a nested scroller (the pane) scrolls it in some runs only;
    // the wheel event reaches the pane, the document takes the scroll (TODO.md, MCP server).
    c.step(
        "scroll",
        "Bottom brought into view, the wheel scrolled the page up and down",
    );
    Ok(())
}

/// `select`, `type` and `upload_file` on form fields ([`FORMS`]).
fn forms(c: &mut Client, s: &dyn Fn(Value) -> Value, base: &str) -> Result<(), String> {
    c.ok("navigate", s(json!({"url": format!("{base}/mcp/forms")})))?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    let find = |prefix: &str| ref_of(&read, prefix).ok_or(format!("no {prefix}: {read}"));
    let js = |c: &mut Client, script: &str| -> Result<String, String> {
        Ok(text(&c.ok("evaluate", s(json!({"script": script})))?))
    };
    let city = find("- comboBoxSelect \"City\"")?;
    let pick =
        |c: &mut Client, option: &str| c.tool("select", s(json!({"ref": city, "option": option})));
    let value = |c: &mut Client| {
        js(
            c,
            "document.getElementById('city').value + ' ' + document.getElementById('picked').textContent",
        )
    };
    // by label, by value, a label's no-break space as a plain one, without case
    for (option, want) in [
        ("Montreal", "\"yul picked yul\""),
        ("yyz", "\"yyz picked yyz\""),
        ("Pick one", "\" picked \""),
        ("Toronto, ON", "\"yyz picked yyz\""),
        ("MONTREAL", "\"yul picked yul\""),
    ] {
        pick(c, option)?.map_err(|e| format!("select {option:?}: {e}"))?;
        if option == "Montreal" && js(c, "window.ev")? != "\"ic\"" {
            return Err(format!(
                "select fired {}, not input then change",
                js(c, "window.ev")?
            ));
        }
        let got = value(c)?;
        if got != want {
            return Err(format!("select {option:?}: {got}, not {want}"));
        }
    }
    // equal labels: the first, said
    let both = pick(c, "Paris")?.map(|r| text(&r));
    if !matches!(&both, Ok(t) if t.contains("2 options match"))
        || value(c)? != "\"par1 picked par1\""
    {
        return Err(format!("select of two equal labels: {both:?}"));
    }
    match pick(c, "Ottawa")? {
        Err(e)
            if e.contains("no option matches \"Ottawa\"")
                && e.contains("\"Montreal\" (value=\"yul\")") => {}
        r => return Err(format!("select with no match: {r:?}")),
    }
    match pick(c, "sold out")? {
        Err(e) if e.contains("option \"Sold out\" is disabled") => {}
        r => return Err(format!("select of a disabled option: {r:?}")),
    }
    let locked = find("- comboBoxSelect \"Size\"")?;
    match c.tool("select", s(json!({"ref": locked, "option": "Two"})))? {
        Err(e) if e.contains("this <select> dropdown is disabled") => {}
        r => return Err(format!("select in a disabled fieldset: {r:?}")),
    }
    let colour = find("- popUpButton \"Colour\"")?;
    match c.tool("type", s(json!({"ref": colour, "text": "x"})))? {
        Err(e) if e.contains("click it") && !e.contains("select tool") => {}
        r => return Err(format!("type on a popup button: {r:?}")),
    }
    match c.tool("select", s(json!({"ref": colour, "option": "x"})))? {
        Err(e) if e.contains("<button> is not a native <select>") && e.contains("click it") => {}
        r => return Err(format!("select on a popup button: {r:?}")),
    }
    c.step(
        "select",
        "by label, value, a no-break space, without case; the first of two equal labels, said; no match lists the options; a disabled option, a disabled fieldset refused; a popup button sent to click by type and select",
    );

    let field = find("- textField \"Field A\"")?;
    c.ok("type", s(json!({"ref": field, "text": TYPED})))?;
    let a = |c: &mut Client| js(c, "document.getElementById('a').value");
    if a(c)? != format!("{TYPED:?}") {
        return Err(format!("type into Field A: {}", a(c)?));
    }
    // an editor that focuses a hidden textarea beside what shows: taken, the text arrives
    let editor = find("- textField \"Editor\"")?;
    c.ok("type", s(json!({"ref": editor, "text": "fn main() {}"})))
        .map_err(|e| format!("type into the editor stand-in: {e}"))?;
    let code = js(c, "document.getElementById('hidden').value")?;
    if code != "\"fn main() {}\"" {
        return Err(format!("the editor's hidden textarea holds {code}"));
    }
    // Field A focused again, then custom widgets: one takes focus itself (not editable), one
    // takes none (focus stays on Field A, unrelated): both refused, Field A untouched
    for (name, why) in [
        ("Widget", "is not editable"),
        ("Chooser", "stayed on an earlier <input>"),
    ] {
        let widget = find(&format!("- button \"{name}\""))?;
        c.ok("type", s(json!({"ref": field, "text": TYPED})))?;
        match c.tool("type", s(json!({"ref": widget, "text": "overwritten"})))? {
            Err(e) if e.contains(why) => {}
            r => return Err(format!("type on the {name} widget: {r:?}")),
        }
        if a(c)? != format!("{TYPED:?}") {
            return Err(format!(
                "Field A after type on the {name} widget: {}",
                a(c)?
            ));
        }
    }
    c.step(
        "type (forms)",
        "a plain field; an editor stand-in's hidden textarea took the text; a widget that takes focus and one that leaves it on Field A refused, Field A unchanged",
    );

    let resume = find("- button \"Resume")?;
    let custom = find("- button \"Upload cover\"")?;
    // `click` on a file input must cancel the OS chooser and tell the agent to use `upload_file`.
    match c.tool("click", s(json!({"ref": resume})))? {
        Err(e) if e.contains("upload_file") => {}
        r => return Err(format!("click on file input: {r:?}")),
    }

    // `upload_file` on a bare `<input type="file">` AND on a custom button; `read_page` must
    // immediately reflect `resume.pdf` (never stale "No file chosen").
    let dir = std::env::temp_dir().join(format!("wubble-mcp-upload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let resume_path = dir.join("resume.pdf");
    let cover_path = dir.join("cover.txt");
    std::fs::write(&resume_path, b"%PDF-1.4\n%wubble-smoke\n").map_err(|e| e.to_string())?;
    std::fs::write(&cover_path, b"hello cover").map_err(|e| e.to_string())?;

    let up = text(&c.ok(
        "upload_file",
        s(json!({"ref": resume, "path": resume_path.to_string_lossy()})),
    )?);
    if !up.contains("resume.pdf") {
        return Err(format!("upload_file bare input: {up}"));
    }
    let after_read = text(&c.ok("read_page", s(json!({})))?);
    if !after_read.contains("resume.pdf") || after_read.contains("No file chosen") {
        return Err(format!("read_page stale after upload_file: {after_read}"));
    }

    let up2 = text(&c.ok(
        "upload_file",
        s(json!({"ref": custom, "path": cover_path.to_string_lossy()})),
    )?);
    if !up2.contains("cover.txt") {
        return Err(format!("upload_file custom button: {up2}"));
    }
    let custom_out = text(&c.ok(
        "evaluate",
        s(json!({"script": "document.getElementById('custom-out').textContent"})),
    )?);
    if custom_out != "\"attached cover.txt\"" {
        return Err(format!("custom upload button result: {custom_out}"));
    }
    let _ = std::fs::remove_dir_all(&dir);

    c.step(
        "upload_file",
        "click on a file input refused with the advice; a bare input and a custom button attached, read_page fresh",
    );
    Ok(())
}

/// `read_text`: an article's prose, capped with a note, and one element's text.
fn reading(c: &mut Client, s: &dyn Fn(Value) -> Value, base: &str) -> Result<(), String> {
    c.ok("navigate", s(json!({"url": format!("{base}/memory")})))?;
    let article = text(&c.ok("read_text", s(json!({})))?);
    if !article.starts_with("The Quiet Physics of Soap Films\nBy A. Bubble")
        || !article.contains("A soap film is a sheet of water")
        || article.contains("Home · About")
    {
        return Err(format!("read_text: {article}"));
    }
    let cut = text(&c.ok("read_text", s(json!({"max_chars": 200})))?);
    if !cut.contains("[cut at 200 of ") {
        return Err(format!("read_text capped: {cut}"));
    }
    let read = text(&c.ok("read_page", s(json!({})))?);
    let heading = ref_of(&read, "- heading 1").ok_or(format!("read_page: {read}"))?;
    let one = text(&c.ok("read_text", s(json!({"ref": heading})))?);
    if one != "The Quiet Physics of Soap Films" {
        return Err(format!("read_text of the heading: {one}"));
    }
    c.step(
        "read_text",
        format!(
            "the article ({} chars) without the nav, capped with a note, one heading",
            article.len()
        ),
    );
    Ok(())
}

/// A session's tab cannot be taken off http(s): a script, a pop-up, a server's redirect.
fn safety(
    c: &mut Client,
    s: &dyn Fn(Value) -> Value,
    session: &str,
    page: &str,
    base: &str,
) -> Result<(), String> {
    let off_web = |c: &mut Client| -> Result<Option<String>, String> {
        let tabs = json_of(&c.ok("list_tabs", json!({"session": session}))?)?;
        Ok(tabs.as_array().into_iter().flatten().find_map(|t| {
            let url = t["url"].as_str().unwrap_or_default();
            let web = ["http://", "https://", "about:"]
                .iter()
                .any(|p| url.starts_with(p));
            (!web).then(|| url.to_owned())
        }))
    };
    c.ok("navigate", s(json!({"url": page})))?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    // a refused navigation is the error of the call that started it, when it is refused
    // before it is sent (a pop-up Chromium refuses itself opens nothing and says nothing)
    for name in ["Open chrome", "Open file", "Go file"] {
        let r = ref_of(&read, &format!("- button {name:?}")).ok_or(format!("no {name}"))?;
        match c.tool("click", s(json!({"ref": r})))? {
            Ok(_) => {}
            Err(e) if e.contains("not a web address") => {}
            Err(e) => return Err(format!("click {name}: {e}")),
        }
    }
    for script in [
        "location.href = 'file:///etc/passwd'",
        "window.open('chrome://version'); 1",
        "location.href = 'chrome://version'",
    ] {
        match c.tool("evaluate", s(json!({"script": script})))? {
            Ok(_) => {}
            Err(e) if e.contains("not a web address") => {}
            Err(e) => return Err(format!("evaluate {script}: {e}")),
        }
    }
    // a navigation off the web would show here (the host checks the pop-ups at the end)
    match c.tool(
        "wait_for",
        s(json!({"navigation": true, "timeout_ms": 1500})),
    )? {
        Err(e) if e.contains("timed out") => {}
        r => return Err(format!("a script moved the session's tab: {r:?}")),
    }
    for to in ["to-file", "to-chrome"] {
        let _ = c.tool("navigate", s(json!({"url": format!("{base}/mcp/{to}")})))?;
        if let Some(url) = off_web(c)? {
            return Err(format!("a redirect took the session's tab to {url}"));
        }
    }
    if let Some(url) = off_web(c)? {
        return Err(format!("the session's tab is at {url}"));
    }
    for url in [
        "file:///etc/passwd",
        "chrome://version",
        "javascript:alert(1)",
        "data:text/html,x",
    ] {
        match c.tool("navigate", s(json!({"url": url})))? {
            Err(e) if e.contains("only http and https") => {}
            r => return Err(format!("navigate {url}: {r:?}")),
        }
    }
    c.step(
        "safety",
        "location and window.open to file: and chrome: (by click and by script), redirects to file: and chrome:, navigate off the web: the tab stayed on http(s)",
    );
    sites(c, s, session, page, base)
}

/// A session's tab stays on its sites: a link, a script's `location`, a form, `window.open`
/// and a server's redirect to a host left out are each refused, as the error of the call,
/// naming the host and `request_more`; the tab stays where it was.
fn sites(
    c: &mut Client,
    s: &dyn Fn(Value) -> Value,
    session: &str,
    page: &str,
    base: &str,
) -> Result<(), String> {
    let refused = |what: &str, r: Result<Value, String>| -> Result<(), String> {
        match r {
            Err(e) if e.contains("a.localhost") && e.contains("request_more") => Ok(()),
            r => Err(format!("{what} to a.localhost: {r:?}")),
        }
    };
    let tabs = |c: &mut Client| -> Result<Vec<String>, String> {
        let tabs = json_of(&c.ok("list_tabs", json!({"session": session}))?)?;
        Ok(tabs
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| t["url"].as_str().unwrap_or_default().to_owned())
            .collect())
    };
    c.ok("navigate", s(json!({"url": page})))?;
    let before = tabs(c)?;
    let read = text(&c.ok("read_page", s(json!({})))?);
    let link = ref_of(&read, "- link \"Elsewhere link\"").ok_or("no Elsewhere link")?;
    refused("a link", c.tool("click", s(json!({"ref": link})))?)?;
    for name in ["Go elsewhere", "Open elsewhere"] {
        let r = ref_of(&read, &format!("- button {name:?}")).ok_or(format!("no {name}"))?;
        refused(name, c.tool("click", s(json!({"ref": r})))?)?;
    }
    let far = ref_of(&read, "- textField \"Far\"").ok_or("no Far field")?;
    refused(
        "a form",
        c.tool("type", s(json!({"ref": far, "text": "x", "submit": true})))?,
    )?;
    refused(
        "location.href",
        c.tool(
            "evaluate",
            s(json!({"script": "location.href = document.querySelector('a[href*=\"a.localhost\"]').href"})),
        )?,
    )?;
    refused(
        "a redirect",
        c.tool(
            "navigate",
            s(json!({"url": format!("{base}/mcp/to-elsewhere")})),
        )?,
    )?;
    let after = tabs(c)?;
    if after != before || after.iter().any(|u| u.contains("a.localhost")) {
        return Err(format!("the tabs were {before:?}, now {after:?}"));
    }
    c.step(
        "sites",
        "a link, location.href, a form, window.open and a redirect to a.localhost (not on the sites) refused, each the call's error naming the host and request_more; the tab stayed",
    );
    Ok(())
}

/// The host's side of the run: the sheets it has allowed, the window choice the next approval
/// must open the session's Space with (`Host::mcp_smoke_answer`), and the tab switch after the
/// client.
#[derive(Default)]
pub struct McpSmoke {
    /// The sheets answered so far ([`SHEETS`]).
    pub sheets: usize,
    pub expect: Option<wubble_core::agent::Mode>,
    /// Pop-ups off the web a session's page asked for, refused.
    pub popups_refused: usize,
    /// Session tabs that reached an address off the web anyway and were blanked.
    pub blanked: usize,
    /// Navigations of session tabs refused for the session's sites (before they were sent,
    /// pop-ups, or taken back).
    pub navigations_refused: usize,
    /// The client's step count, kept while the host switches tabs.
    pub steps: usize,
    /// The run turned hosting off before the quit (`Host::mcp_smoke_off`).
    pub hosting_off: bool,
    /// The Space the first session left its tab in (`stop_session` with `keep_tabs`).
    pub kept_space: Option<wubble_core::space::SpaceId>,
    /// The switch's step and the tab whose frame it waits for (`Host::mcp_smoke_switch`).
    pub switch: Option<(usize, wubble_core::tab::TabId)>,
    /// The panes' frame size before the switch hid the sidebar.
    pub sized: Option<(u32, u32)>,
    /// What each "Use my sign-ins" copy did, in order.
    pub copied: Vec<wubble_engine::storage::CopyReport>,
    /// The owner's [`SIGNIN_HOST`] cookies counted after the copy (the source keeps them).
    pub owner_kept: Option<usize>,
    /// The container the copy went into: gone once its session stopped.
    pub signin_container: Option<wubble_core::space::ContainerId>,
    /// The activity log's copy line was written.
    pub copy_logged: bool,
    /// The origins a session's tab offered the owner's logins on (the key chip).
    pub key_chips: std::collections::BTreeSet<String>,
    /// A [`Press::ShareBindAllow`] sheet waits for the vault's list before it is answered.
    pub waiting_vault: bool,
    pub deadline: slint::Timer,
}

/// The approval sheet on screen, found by its elements (their ids and roles, in a debug build)
/// and pressed as winit's pointer events would press it.
pub mod sheet {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};
    use slint::LogicalPosition;
    use slint::platform::{PointerEventButton, WindowEvent};
    use wubble_ui::AppWindow;

    fn root(window: &AppWindow) -> Result<ElementHandle, String> {
        ElementHandle::find_by_element_id(window, "AppWindow::agent-sheet")
            .next()
            .ok_or_else(|| "no agent sheet in the window".into())
    }

    fn centre(e: &ElementHandle) -> LogicalPosition {
        let (p, s) = (e.absolute_position(), e.size());
        LogicalPosition::new(p.x + s.width / 2.0, p.y + s.height / 2.0)
    }

    /// The centre of the window's button `label`, wherever it is (the Spaces popup's dustbins).
    pub fn labelled(window: &AppWindow, label: &str) -> Result<LogicalPosition, String> {
        ElementHandle::find_by_accessible_label(window, label)
            .find(|e| e.accessible_role() == Some(AccessibleRole::Button))
            .map(|e| centre(&e))
            .ok_or_else(|| format!("no {label:?} button"))
    }

    /// The centre of the sheet's switch `label` ("Use my sign-ins for these sites").
    pub fn switch(window: &AppWindow, label: &str) -> Result<LogicalPosition, String> {
        root(window)?
            .query_descendants()
            .match_accessible_role(AccessibleRole::Switch)
            .match_accessible_label(label)
            .find_first()
            .map(|e| centre(&e))
            .ok_or_else(|| format!("no {label:?} switch on the sheet"))
    }

    /// The centre of the sheet's button `label` (Allow, Deny).
    pub fn button(window: &AppWindow, label: &str) -> Result<LogicalPosition, String> {
        root(window)?
            .query_descendants()
            .match_accessible_role(AccessibleRole::Button)
            .match_accessible_label(label)
            .find_first()
            .map(|e| centre(&e))
            .ok_or_else(|| format!("no {label} button on the sheet"))
    }

    /// The centre of segment `i` of the sheet's Window choice.
    pub fn window_choice(window: &AppWindow, i: usize) -> Result<LogicalPosition, String> {
        root(window)?
            .query_descendants()
            .match_id("SegRow::seg")
            .find_all()
            .get(i)
            .map(centre)
            .ok_or_else(|| format!("no window choice {i} on the sheet"))
    }

    /// The Window choice the sheet shows selected (its tabs' `accessible-item-selected`).
    pub fn chosen(window: &AppWindow) -> Result<Option<String>, String> {
        Ok(root(window)?
            .query_descendants()
            .match_accessible_role(AccessibleRole::Tab)
            .find_all()
            .into_iter()
            .filter(|e| e.accessible_item_selected() == Some(true))
            .filter_map(|e| e.accessible_label().map(|l| l.to_string()))
            // a login picker's options are tabs too
            .find(|l| l == "Windowed" || l == "Headless"))
    }

    /// Every tab on screen, its label and selection: what a failed [`chosen`] saw.
    pub fn tabs(window: &AppWindow) -> String {
        root(window)
            .map(|r| {
                r.query_descendants()
                    .match_accessible_role(AccessibleRole::Tab)
                    .find_all()
                    .into_iter()
                    .map(|e| {
                        format!(
                            "{:?}={:?}",
                            e.accessible_label(),
                            e.accessible_item_selected()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    }

    /// The Sites list editor on the sheet.
    fn sites(window: &AppWindow) -> Result<ElementHandle, String> {
        root(window)?
            .query_descendants()
            .match_accessible_role(AccessibleRole::List)
            .match_accessible_label("Sites")
            .find_first()
            .ok_or_else(|| "no Sites editor on the sheet".into())
    }

    /// The centre of the Sites editor's "+".
    pub fn add_site(window: &AppWindow) -> Result<LogicalPosition, String> {
        sites(window)?
            .query_descendants()
            .match_accessible_role(AccessibleRole::Button)
            .match_accessible_label("Add")
            .find_first()
            .map(|e| centre(&e))
            .ok_or_else(|| "no Add button in the Sites editor".into())
    }

    /// Whether the Sites editor shows a draft (a text field).
    pub fn drafting(window: &AppWindow) -> Result<bool, String> {
        Ok(sites(window)?
            .query_descendants()
            .match_accessible_role(AccessibleRole::TextInput)
            .find_first()
            .is_some())
    }

    /// `text` typed into what has the focus, key by key.
    pub fn type_text(window: &AppWindow, text: &str) {
        let w = slint::ComponentHandle::window(window);
        for c in text.chars() {
            let text: slint::SharedString = c.to_string().into();
            w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            w.dispatch_event(WindowEvent::KeyReleased { text });
        }
    }

    /// Escape pressed and released, to the window.
    pub fn escape(window: &AppWindow) {
        let w = slint::ComponentHandle::window(window);
        let text: slint::SharedString = slint::platform::Key::Escape.into();
        w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        w.dispatch_event(WindowEvent::KeyReleased { text });
    }

    /// A left click at `at`, through the window's event path.
    pub fn click(window: &AppWindow, at: LogicalPosition) {
        let w = slint::ComponentHandle::window(window);
        let button = PointerEventButton::Left;
        w.dispatch_event(WindowEvent::PointerMoved { position: at });
        w.dispatch_event(WindowEvent::PointerPressed {
            position: at,
            button,
        });
        w.dispatch_event(WindowEvent::PointerReleased {
            position: at,
            button,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_read_from_their_lines() {
        let page = "- heading 1 \"Agent page\" [ref=e1.3]\n- button \"Press\" [ref=e1.9]";
        assert_eq!(ref_of(page, "- button \"Press\""), Some("e1.9".into()));
        assert_eq!(ref_of(page, "- textField"), None);
    }
}
