//! Windowless check that a container is the unit of web storage. Run by hand:
//!
//!     cargo run -p wubble-engine --example cookie_smoke
//!
//! (after `cargo build --workspace`, so `wubble-helper` sits next to the
//! binaries). The parent serves two pages on a loopback port and runs itself
//! twice on one throwaway profile, each run its own process (CEF starts once
//! per process):
//!
//! 1. `write`: a view in container 1 loads `/set`, which answers with a
//!    persistent and a session `Set-Cookie` and stores a `localStorage` item.
//!    A second view in container 1 loads `/check` (the page shows the
//!    `Cookie` header the server got and the item): both cookies and the item
//!    must be there. Views in container 2 and in the default container must
//!    see neither. Then every view closes and CEF shuts down.
//!    Container 3 gets the same cookies and item, then
//!    `Engine::clear_browsing_data` on it: its cookies are gone, its item stays.
//! 2. `read`: on the same profile, container 1 still sends both cookies and
//!    has the item; container 2 still has nothing. Then `View::forget_site`
//!    on the page's origin in container 1: cookies and item are gone.
//!
//! It prints `smoke: cookies ok`. No window is shown; nothing leaves the
//! loopback. The deadline only bounds a hang; nothing here measures speed.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use wubble_core::space::ContainerId;
use wubble_engine::app::{Engine, EngineConfig, ViewId};
use wubble_engine::page::PageEvent;
use wubble_engine::view::{Layout, ViewConfig};
use wubble_gfx::{FrameSize, GpuContext};

/// How long one page may take before the smoke gives up (a hang, not a speed).
const DEADLINE: Duration = Duration::from_secs(60);

const WORK: ContainerId = ContainerId(1);
const OTHER: ContainerId = ContainerId(2);
const CLEARED: ContainerId = ContainerId(3);

/// What `/check` must report in container 1: both cookies and the item.
const SHARED: Seen = Seen {
    persistent: true,
    session: true,
    item: true,
};
const NOTHING: Seen = Seen {
    persistent: false,
    session: false,
    item: false,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    persistent: bool,
    session: bool,
    item: bool,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [] => parent(),
        [phase, dir, port] => port
            .parse()
            .map_err(|e| format!("port {port}: {e}"))
            .and_then(|port| child(phase, Path::new(dir), port)),
        _ => Err(format!("unexpected arguments {args:?}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("smoke: cookies FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parent() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("wubble-cookie-smoke-{}", std::process::id()));
    let port = serve().map_err(|e| format!("cannot serve: {e}"))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let result = (|| {
        for phase in ["write", "read"] {
            let status = std::process::Command::new(&exe)
                .arg(phase)
                .arg(&dir)
                .arg(port.to_string())
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("phase {phase} exited with {status}"));
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result?;
    println!("smoke: cookies ok");
    Ok(())
}

fn child(phase: &str, dir: &Path, port: u16) -> Result<(), String> {
    let gpu = GpuContext::new().map_err(|e| format!("no GPU device: {e}"))?;
    let mut engine = Engine::start(EngineConfig {
        containers: vec![WORK],
        ..EngineConfig::for_example(dir.join("cef")).map_err(|e| e.to_string())?
    })
    .map_err(|e| e.to_string())?;
    let mut smoke = Smoke {
        engine: &mut engine,
        gpu: &gpu,
        base: format!("http://127.0.0.1:{port}"),
    };
    let outcome = match phase {
        "write" => smoke.write(),
        "read" => smoke.read(),
        _ => Err(format!("unknown phase {phase}")),
    };
    // Every view closes before CEF shuts down, which writes the cookie store.
    engine.request_shutdown();
    // The shutdown's budget is a count of turns, so no sleep between them.
    while !engine.is_stopped() {
        engine.tick();
    }
    if engine.abandoned() {
        return Err("the shutdown gave up on its browsers".into());
    }
    outcome?;
    println!("smoke: cookies {phase} ok");
    Ok(())
}

struct Smoke<'a> {
    engine: &'a mut Engine,
    gpu: &'a GpuContext,
    base: String,
}

impl Smoke<'_> {
    fn write(&mut self) -> Result<(), String> {
        let set = self.open("/set", WORK)?;
        self.wait(set, |e| console(e).filter(|m| *m == "smoke set").map(drop))?;
        self.expect(WORK, SHARED, "a second tab in the same container")?;
        self.expect(OTHER, NOTHING, "a tab in another container")?;
        self.expect(
            ContainerId::DEFAULT,
            NOTHING,
            "a tab in the default container",
        )?;
        let set = self.open("/set", CLEARED)?;
        self.wait(set, |e| console(e).filter(|m| *m == "smoke set").map(drop))?;
        self.engine
            .clear_browsing_data(&[CLEARED])
            .map_err(|e| e.to_string())?;
        let item_only = Seen {
            item: true,
            ..NOTHING
        };
        self.expect(CLEARED, item_only, "a container after clearing its cookies")
            .map(drop)
    }

    fn read(&mut self) -> Result<(), String> {
        let view = self.expect(WORK, SHARED, "the same container after a restart")?;
        self.expect(OTHER, NOTHING, "another container after a restart")?;
        let call = self
            .engine
            .view(view)
            .and_then(|v| v.forget_site(&self.base))
            .ok_or("the view refused to forget the site")?;
        let result = self.wait(view, |e| match e {
            PageEvent::CdpReply { id, result } if *id == call => Some(result.clone()),
            _ => None,
        })?;
        result.map_err(|e| format!("forgetting the site: {e}"))?;
        self.expect(
            WORK,
            NOTHING,
            "the same container after forgetting the site",
        )?;
        Ok(())
    }

    /// Opens `/check` in `container` and compares what the page saw; the view.
    fn expect(&mut self, container: ContainerId, want: Seen, who: &str) -> Result<ViewId, String> {
        let view = self.open("/check", container)?;
        let got = self.wait(view, |e| {
            let rest = console(e)?.strip_prefix("smoke check ")?;
            let (cookie, item) = rest.split_once(" | ")?;
            Some(Seen {
                persistent: cookie.contains("keep=1"),
                session: cookie.contains("sess=1"),
                item: item == "v",
            })
        })?;
        if got != want {
            return Err(format!(
                "{who} (container {}): saw {got:?}, want {want:?}",
                container.0
            ));
        }
        println!("smoke: cookies: {who}: {got:?}");
        Ok(view)
    }

    fn open(&mut self, path: &str, container: ContainerId) -> Result<ViewId, String> {
        self.engine
            .open_view(ViewConfig {
                url: format!("{}{path}", self.base),
                layout: Layout {
                    size: FrameSize::new(320, 200),
                    scale: 1.0,
                    area: None,
                },
                frame_rate: 30,
                begin_frames: false,
                gpu: self.gpu.presenter().context().clone(),
                container,
            })
            .map_err(|e| e.to_string())
    }

    /// Pumps CEF until `view` reports a page event `read` accepts.
    fn wait<T>(
        &mut self,
        view: ViewId,
        read: impl Fn(&PageEvent) -> Option<T>,
    ) -> Result<T, String> {
        let start = Instant::now();
        loop {
            if start.elapsed() > DEADLINE {
                return Err(format!("view {view:?} never reported"));
            }
            self.engine.tick();
            for (id, event) in self.engine.take_page_events() {
                if let Some(found) = (id == view).then(|| read(&event)).flatten() {
                    return Ok(found);
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// The text of a console line.
fn console(event: &PageEvent) -> Option<&str> {
    match event {
        PageEvent::Console { message, .. } => Some(message),
        _ => None,
    }
}

/// Serves `/set` and `/check` on `127.0.0.1` from a thread that ends with the
/// process; the port.
fn serve() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = answer(stream);
        }
    });
    Ok(port)
}

fn answer(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut cookie = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("cookie")
        {
            cookie = value.trim().to_owned();
        }
    }
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let (headers, body) = match path {
        "/set" => (
            "Set-Cookie: keep=1; Max-Age=86400; Path=/\r\nSet-Cookie: sess=1; Path=/\r\n",
            "<script>localStorage.setItem('k', 'v'); console.log('smoke set')</script>".to_owned(),
        ),
        "/check" => (
            "",
            // The server's view of the cookies, not `document.cookie`: they were sent.
            format!(
                "<script>console.log('smoke check {} | ' + localStorage.getItem('k'))</script>",
                cookie.replace(['\'', '\\', '<'], "")
            ),
        ),
        _ => ("", String::new()),
    };
    let status = if body.is_empty() {
        "404 Not Found"
    } else {
        "200 OK"
    };
    let page = format!("<!doctype html><meta charset=utf-8><title>cookies</title>{body}");
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nCache-Control: no-store\r\n{headers}\
         Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    )?;
    stream.flush()
}
