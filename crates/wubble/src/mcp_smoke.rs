//! `--smoke mcp`: an MCP client on the browser's own socket, as a harness's `wubble mcp` would
//! be. On a throwaway profile with `agent.mcp_enabled` on, a thread connects and runs, one line
//! per step: discovery, the tool list, `start_session` (headless; the smoke approves it,
//! `Host::approve_for_smoke`), that the session sees only its own tab, `navigate` to the
//! bundled page on loopback, `read_page`, a click on its button by ref and a wait for what the
//! click wrote, typing into its field and a wait for what the input wrote, `evaluate`, a
//! `screenshot` (a PNG that decodes), `stop_session`, and that the handle is dead afterwards;
//! last a second session is started and left running: the browser's quit must close its Space.
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
<button onclick="document.getElementById('out').textContent='pressed';document.title='pressed'">Press</button>
<p id="out">idle</p>
</body></html>"##;

/// What the smoke types.
const TYPED: &str = "wubble";

/// Runs the client on its own thread; its result goes to the host, which ends the run.
pub fn start(socket: PathBuf, page: String) {
    let spawned = std::thread::Builder::new()
        .name("mcp-smoke".into())
        .spawn(move || {
            // The connection stays open to the end, so the quit, not a disconnect, ends the
            // second session.
            let result = run(&socket, &page).map(|(steps, client)| {
                std::mem::forget(client);
                steps
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

/// The ref on the `read_page` line that starts with `prefix`.
fn ref_of(page: &str, prefix: &str) -> Option<String> {
    let line = page.lines().find(|l| l.starts_with(prefix))?;
    let r = line.rsplit_once("[ref=")?.1.strip_suffix(']')?;
    Some(r.to_owned())
}

fn run(socket: &Path, page: &str) -> Result<(usize, Client), String> {
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

    let started = c.ok(
        "start_session",
        json!({"purpose": "the mcp smoke", "headless": true}),
    )?;
    let started: Value = serde_json::from_str(&text(&started)).map_err(|e| e.to_string())?;
    let session = started["session"].as_str().unwrap_or_default().to_owned();
    let tab = started["tab"].as_str().unwrap_or_default().to_owned();
    if session.len() != 64 || tab.is_empty() {
        return Err(format!("start_session: {started}"));
    }
    c.step("start_session", format!("tab {tab}, headless"));

    let tabs = c.ok("list_tabs", json!({"session": session}))?;
    let tabs: Value = serde_json::from_str(&text(&tabs)).map_err(|e| e.to_string())?;
    if tabs.as_array().map(Vec::len) != Some(1) || tabs[0]["tab"] != tab.as_str() {
        return Err(format!("list_tabs: {tabs}"));
    }
    // Every other tab handle, the user's own tab among them, is unknown to the session.
    let own: u64 = tab.parse().map_err(|_| format!("tab handle {tab}"))?;
    for other in (0..=own + 1).filter(|&t| t != own) {
        let r = c.tool(
            "navigate",
            json!({"session": session, "tab": other.to_string(), "url": page}),
        )?;
        if r.is_ok() {
            return Err(format!("tab {other} outside the session was reachable"));
        }
    }
    c.step("isolation", format!("1 tab; tabs 0..={} refused", own + 1));

    c.ok(
        "navigate",
        json!({"session": session, "tab": tab, "url": page}),
    )?;
    c.step("navigate", page);
    let read = c.ok("read_page", json!({"session": session, "tab": tab}))?;
    let read = text(&read);
    let button = ref_of(&read, "- button \"Press\"").ok_or(format!("read_page: {read}"))?;
    let field = ref_of(&read, "- textField \"Name\"").ok_or(format!("read_page: {read}"))?;
    c.step("read_page", format!("{} items", read.lines().count()));

    c.ok(
        "click",
        json!({"session": session, "tab": tab, "ref": button}),
    )?;
    c.ok(
        "wait_for",
        json!({"session": session, "tab": tab, "text": "pressed"}),
    )?;
    c.step("click", &button);
    c.ok(
        "type",
        json!({"session": session, "tab": tab, "ref": field, "text": TYPED}),
    )?;
    c.ok(
        "wait_for",
        json!({"session": session, "tab": tab, "text": format!("typed {TYPED}")}),
    )?;
    c.step("type", &field);
    let script = "document.getElementById('name').value + '|' + document.title";
    let value = c.ok(
        "evaluate",
        json!({"session": session, "tab": tab, "script": script}),
    )?;
    let value = text(&value);
    if value != format!("\"{TYPED}|pressed\"") {
        return Err(format!("evaluate: {value}"));
    }
    c.step("evaluate", &value);
    let shot = c.ok("screenshot", json!({"session": session, "tab": tab}))?;
    let png = shot["content"][0]["data"]
        .as_str()
        .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok())
        .ok_or("screenshot: no image")?;
    let (w, h) = crate::shot_io::png_size(&png).ok_or("screenshot: not a PNG")?;
    if w == 0 || h == 0 {
        return Err(format!("screenshot: {w}x{h}"));
    }
    c.step("screenshot", format!("{w}x{h} PNG, {} bytes", png.len()));

    c.ok("stop_session", json!({"session": session}))?;
    if c.tool("list_tabs", json!({"session": session}))?.is_ok() {
        return Err("the stopped session still answers".into());
    }
    c.step("stop_session", "the handle is dead");
    // Left running: the browser's quit must end it and close its Space (checked by the host).
    c.ok(
        "start_session",
        json!({"purpose": "ended by the quit", "headless": true}),
    )?;
    c.step("start_session", "a second one, left for the quit");
    Ok((c.steps, c))
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
