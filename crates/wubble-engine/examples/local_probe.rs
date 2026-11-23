//! Windowless check that local servers load. Run by hand:
//!
//!     cargo run -p wubble-engine --example local_probe
//!
//! (after `cargo build --workspace`; on macOS bundled with `wubble-bundle --exe`, see AGENTS.md).
//! It serves a page from servers bound the ways dev servers bind (IPv4 loopback only, IPv6
//! loopback only, both, and every interface for the machine's LAN address) and loads each by
//! `localhost`, `127.0.0.1`, `[::1]` and the LAN address in a view of the default container
//! (a persistent shared request context, as the browser's tabs). Each page then checks what dev
//! servers need: a same-origin `fetch`, a cookie set by script and sent back, a WebSocket
//! (`ws://` on the same host, the shape of a hot-reload channel) and a service worker
//! (localhost is a secure context; a LAN address is not, so not there). It prints one line per URL and `smoke: local ok` when every
//! loopback URL loaded with all four checks; the LAN row is reported, not asserted (macOS asks
//! for Local Network access, and a machine may have no LAN address). Nothing leaves the
//! machine: the LAN address is read from a UDP socket that is never sent on (or given as
//! `LOCAL_PROBE_LAN=IP`). Bounded by counts of
//! pump turns; the page's own timers only stop a hang.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::process::ExitCode;
use std::time::Duration;

use wubble_core::space::ContainerId;
use wubble_engine::app::{Engine, EngineConfig};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

/// Pump turns a loopback URL may take (a turn is a millisecond on Linux, a frame on macOS);
/// only stops a hang.
const TURNS: u32 = 60_000;
/// Pump turns for the LAN row, reported only: on macOS a connection the Local Network permission
/// holds back never fails, it hangs.
const LAN_TURNS: u32 = 3_000;

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><title>local</title><script>
(async () => {
  const r = { fetch: false, cookie: false, ws: false, sw: false };
  try { r.fetch = (await (await fetch('/ping')).text()) === 'pong'; } catch (e) { r.fetchErr = String(e); }
  try {
    document.cookie = 'local_probe=1; path=/';
    r.cookie = (await (await fetch('/cookie')).text()).includes('local_probe=1');
  } catch (e) { r.cookieErr = String(e); }
  r.ws = await new Promise((done) => {
    try {
      const ws = new WebSocket('ws://' + location.host + '/ws');
      ws.onopen = () => { ws.close(); done(true); };
      ws.onerror = () => done(false);
      setTimeout(() => done(false), 5000);
    } catch (e) { done(false); }
  });
  r.secure = window.isSecureContext;
  if (navigator.serviceWorker) {
    try {
      await navigator.serviceWorker.register('/sw.js');
      await Promise.race([navigator.serviceWorker.ready, new Promise((_, j) => setTimeout(() => j('timeout'), 5000))]);
      r.sw = true;
    } catch (e) { r.swErr = String(e); }
  } else { r.swErr = 'no navigator.serviceWorker (not a secure context)'; }
  console.log('local ' + JSON.stringify(r));
})();
</script>"#;

fn main() -> ExitCode {
    match run() {
        Ok(true) => {
            println!("smoke: local ok");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            eprintln!("smoke: local FAILED");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("smoke: local FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The machine's LAN address: the source a route to a TEST-NET address would use. `connect`
/// on UDP sends nothing.
fn lan_address() -> Option<IpAddr> {
    // A VPN's tunnel can own that route (the Mac's utun did): `LOCAL_PROBE_LAN=IP` names the LAN
    // address instead.
    if let Some(ip) = std::env::var("LOCAL_PROBE_LAN")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Some(ip);
    }
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn run() -> Result<bool, String> {
    let v4 = serve("127.0.0.1:0").map_err(|e| format!("127.0.0.1: {e}"))?;
    let v6 = serve("[::1]:0").map_err(|e| format!("[::1]: {e}"))?;
    // Both families on one port, as a server bound to `localhost` by name may be.
    let both = serve("127.0.0.1:0").map_err(|e| e.to_string())?;
    let both_v6 = serve(&format!("[::1]:{both}")).is_ok();
    let any = serve("0.0.0.0:0").map_err(|e| e.to_string())?;
    let mut rows: Vec<(String, bool)> = vec![
        (format!("http://localhost:{v4}/"), true),
        (format!("http://127.0.0.1:{v4}/"), true),
        (format!("http://[::1]:{v6}/"), true),
        (format!("http://localhost:{v6}/"), true),
        (format!("http://localhost:{both}/"), both_v6),
        (format!("http://sub.localhost:{v4}/"), true),
    ];
    if let Some(ip) = lan_address() {
        let host = match ip {
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) => format!("[{ip}]"),
        };
        rows.push((format!("http://{host}:{any}/"), false));
    } else {
        println!("local: no LAN address, LAN row skipped");
    }

    let dir = std::env::temp_dir().join(format!("wubble-local-probe-{}", std::process::id()));
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let mut engine =
        Engine::start(EngineConfig::for_example(dir.join("cef")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut all = true;
    for (url, asserted) in &rows {
        let seen = load(
            &mut engine,
            &gpu,
            url,
            if *asserted { TURNS } else { LAN_TURNS },
        );
        // A LAN address is no secure context, so it has no service workers (as in Chrome).
        let checks: &[&str] = if *asserted {
            &["fetch", "cookie", "ws", "sw"]
        } else {
            &["fetch", "cookie", "ws"]
        };
        let pass = seen
            .as_deref()
            .is_ok_and(|r| checks.iter().all(|k| r.contains(&format!("\"{k}\":true"))));
        let mark = match (pass, asserted) {
            (true, _) => "ok",
            (false, true) => "FAILED",
            (false, false) => "failed (reported only)",
        };
        println!("local: {url} {mark}: {}", seen.unwrap_or_else(|e| e));
        all &= pass || !asserted;
    }
    engine.request_shutdown();
    while !engine.is_stopped() {
        engine.tick();
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(all)
}

/// Loads `url` in a new view: the page's report, or why there is none.
fn load(engine: &mut Engine, gpu: &GpuContext, url: &str, turns: u32) -> Result<String, String> {
    let view = engine
        .open_view(ViewConfig {
            url: url.into(),
            layout: Layout {
                size: FrameSize::new(320, 200),
                scale: 1.0,
                area: None,
            },
            frame_rate: 30,
            begin_frames: false,
            gpu: gpu.presenter().context().clone(),
            container: ContainerId::DEFAULT,
            storage: wubble_core::space::Storage::Disk,
        })
        .map_err(|e| e.to_string())?;
    for _ in 0..turns {
        engine.tick();
        for (id, event) in engine.take_page_events() {
            if id != view {
                continue;
            }
            match event {
                PageEvent::Console { message, .. } if message.starts_with("local ") => {
                    return Ok(message["local ".len()..].to_owned());
                }
                PageEvent::LoadFailed { code, text, .. } => {
                    return Err(format!("load failed {code} {text}"));
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Err("no report (hang)".into())
}

/// Serves [`PAGE`], `/ping`, `/cookie`, `/sw.js` and a WebSocket at `/ws` on `addr`; the port.
fn serve(addr: &str) -> std::io::Result<u16> {
    let listener = TcpListener::bind(addr.parse::<SocketAddr>().map_err(std::io::Error::other)?)?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = answer(stream);
            });
        }
    });
    Ok(port)
}

fn answer(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut request = String::new();
        if reader.read_line(&mut request)? == 0 {
            return Ok(());
        }
        let (mut cookie, mut ws_key) = (String::new(), None);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let value = value.trim().to_owned();
                if name.eq_ignore_ascii_case("cookie") {
                    cookie = value;
                } else if name.eq_ignore_ascii_case("sec-websocket-key") {
                    ws_key = Some(value);
                }
            }
        }
        let path = request.split_whitespace().nth(1).unwrap_or("/");
        if let (true, Some(key)) = (path == "/ws", ws_key) {
            let digest =
                sha1_smol::Sha1::from(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11"))
                    .digest();
            let accept =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, digest.bytes());
            write!(
                stream,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {accept}\r\n\r\n"
            )?;
            stream.flush()?;
            // Hold the socket until the page closes it.
            let mut sink = [0u8; 256];
            while stream.read(&mut sink)? > 0 {}
            return Ok(());
        }
        let (mime, body) = match path {
            "/" => ("text/html; charset=utf-8", PAGE.to_owned()),
            "/ping" => ("text/plain", "pong".to_owned()),
            "/cookie" => ("text/plain", cookie),
            "/sw.js" => (
                "text/javascript",
                "self.addEventListener('fetch', () => {});".to_owned(),
            ),
            _ => ("text/plain", String::new()),
        };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()?;
    }
}
