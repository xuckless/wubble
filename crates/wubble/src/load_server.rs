//! `--measure load`'s loopback server: fixed pages and assets, a sink for the pages' own timings
//! (`assets/probe.js`) and a log of every request, so a run can count what the page asked for.
//! Hand-rolled HTTP/1.1 with keep-alive, like the other loopback servers (`media_smoke.rs`); the
//! bytes are generated, not stored, and the same pages are served to plain Chromium by
//! `scripts/measure-load-chromium.sh` (`--measure load --serve-only`).
//!
//! Every asset is `Cache-Control: max-age=3600`, so a second load of a page (`?w=1`: another
//! document URL, the same assets) finds them in the HTTP cache.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// The pages a run measures, in order.
pub const PAGES: [&str; 5] = ["/article", "/images", "/scripts", "/blockable", "/bigdom"];

/// Requests the blockable page makes (scripts, then images), on URL paths the default lists
/// block.
pub const BLOCKABLE_SCRIPTS: usize = 100;
pub const BLOCKABLE_IMAGES: usize = 100;

/// Nodes the big DOM page builds: in the HTML, then appended by script in batches.
pub const BIGDOM_STATIC: usize = 50_000;
pub const BIGDOM_APPENDED: usize = 5_000;

const IMAGES: usize = 30;
const SCRIPTS: usize = 20;
const PROBE: &str = include_str!("../assets/probe.js");
/// The script the host registers as a document script in every page (`host/boost.rs`).
const KIT: &str = include_str!("../assets/kit.js");

/// What one page load reported about itself, milliseconds from navigation start.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub page: String,
    pub fp: Option<f64>,
    pub fcp: Option<f64>,
    pub dcl: Option<f64>,
    pub load: Option<f64>,
    /// `/bigdom`: how long its script took to build the DOM.
    pub build: Option<f64>,
    /// When the page's first script ran.
    pub s0: Option<f64>,
    /// Set by a `pageshow` that restored the page from the back/forward cache.
    pub persisted: Option<bool>,
}

/// The `POST /__t` body.
pub fn parse_timing(body: &str) -> Option<Timing> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let num = |k: &str| v.get(k).and_then(serde_json::Value::as_f64);
    Some(Timing {
        page: v.get("page")?.as_str()?.to_owned(),
        fp: num("fp"),
        fcp: num("fcp"),
        dcl: num("dcl"),
        load: num("load"),
        build: num("build"),
        s0: num("s0"),
        persisted: v.get("persisted").and_then(serde_json::Value::as_bool),
    })
}

/// `(method, path, query)` of a request line.
pub fn parse_request_line(line: &str) -> Option<(&str, &str, &str)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Some((method, path, query))
}

/// The server and what it saw.
pub struct LoadServer {
    pub port: u16,
    /// `METHOD path?query` of every request except the timing sink, in arrival order.
    log: Mutex<Vec<String>>,
    timings: Mutex<VecDeque<Timing>>,
    connections: AtomicUsize,
    /// Print each timing as a CSV line when it arrives (`--serve-only`).
    echo: bool,
}

impl LoadServer {
    /// Serves on `127.0.0.1:port` (0: a free one) from threads that end with the process.
    pub fn start(port: u16, echo: bool) -> std::io::Result<Arc<Self>> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let server = Arc::new(Self {
            port: listener.local_addr()?.port(),
            log: Mutex::new(Vec::new()),
            timings: Mutex::new(VecDeque::new()),
            connections: AtomicUsize::new(0),
            echo,
        });
        let this = server.clone();
        std::thread::Builder::new()
            .name("load-http".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    this.connections.fetch_add(1, Ordering::Relaxed);
                    let this = this.clone();
                    std::thread::spawn(move || {
                        // A page navigated away from drops its connections.
                        let _ = this.serve(stream);
                    });
                }
            })?;
        Ok(server)
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Requests logged so far.
    pub fn requests(&self) -> usize {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// The log from entry `from` on.
    pub fn log_since(&self, from: usize) -> Vec<String> {
        let log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.get(from..).unwrap_or_default().to_vec()
    }

    /// TCP connections accepted (a `preconnect` opens one without a request).
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    /// The oldest timing a page reported and nobody took yet.
    pub fn take_timing(&self) -> Option<Timing> {
        self.timings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    fn serve(&self, stream: TcpStream) -> std::io::Result<()> {
        // A header and a body in two small writes meet Nagle's algorithm and the client's delayed
        // ACK: 40 ms on every page, which would be in every number measured.
        stream.set_nodelay(true)?;
        let mut out = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            let Some((method, path, query)) = parse_request_line(&line) else {
                return Ok(());
            };
            let (method, path, query) = (method.to_owned(), path.to_owned(), query.to_owned());
            let mut length = 0usize;
            let mut purpose = String::new();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header)? == 0 || header == "\r\n" {
                    break;
                }
                let lower = header.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
                if let Some(v) = lower.strip_prefix("sec-purpose:") {
                    purpose = format!(" [{}]", v.trim());
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            if method == "POST" && path == "/__t" {
                if let Some(t) = parse_timing(&String::from_utf8_lossy(&body)) {
                    if self.echo {
                        let c = |v: Option<f64>| v.map_or_else(String::new, |v| v.to_string());
                        println!(
                            "timing,{},{},{},{},{},{},{},{}",
                            t.page,
                            c(t.fp),
                            c(t.fcp),
                            c(t.dcl),
                            c(t.load),
                            c(t.build),
                            c(t.s0),
                            t.persisted.map_or("", |p| if p { "1" } else { "0" })
                        );
                    }
                    self.timings
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push_back(t);
                }
                out.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")?;
                continue;
            }
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!(
                    "{method} {path}{}{purpose}",
                    if query.is_empty() {
                        String::new()
                    } else {
                        format!("?{query}")
                    }
                ));
            match route(&path) {
                Some((mime, body)) => {
                    let mut reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: max-age=3600\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    if method != "HEAD" {
                        reply.extend_from_slice(&body);
                    }
                    out.write_all(&reply)?;
                }
                None => out.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nCache-Control: max-age=3600\r\n\r\n",
                )?,
            }
        }
    }
}

fn html(body: String) -> Option<(&'static str, Vec<u8>)> {
    Some(("text/html; charset=utf-8", body.into_bytes()))
}

/// A page: the probe inline in the head, then `head` and `body`.
fn page(title: &str, head: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>{title}</title><script>{PROBE}</script>{head}</head><body>{body}</body></html>"
    )
}

/// The answer for `path` (query already cut): MIME type and bytes.
pub fn route(path: &str) -> Option<(&'static str, Vec<u8>)> {
    match path {
        "/blank" => html(page("blank", "", "<h1>blank</h1>")),
        "/article" => html(article()),
        "/images" => html(images()),
        "/scripts" => html(scripts()),
        "/blockable" => html(blockable()),
        "/bigdom" => html(bigdom()),
        "/spec" => html(speculation()),
        "/kit.js" => Some(("text/javascript", KIT.as_bytes().to_vec())),
        "/kitcost" => html(kit_cost()),
        "/spec/a" | "/spec/b" => html(page("speculated", "", "<p>speculated</p>")),
        _ => {
            let n = |prefix: &str, suffix: &str| {
                path.strip_prefix(prefix)?
                    .strip_suffix(suffix)?
                    .parse::<usize>()
                    .ok()
            };
            if let Some(i) = n("/img/", ".bmp") {
                return Some(("image/bmp", bmp(i)));
            }
            if let Some(i) = n("/js/", ".js") {
                return Some(("text/javascript", script(i).into_bytes()));
            }
            // The blockable page's assets: tiny, and only served when nothing blocked them.
            if path.ends_with(".js") {
                return Some(("text/javascript", b"window.__ad=1;".to_vec()));
            }
            if path.ends_with(".gif") {
                return Some(("image/gif", GIF.to_vec()));
            }
            None
        }
    }
}

/// A 1x1 GIF.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 1, 0, 1, 0, 0x80, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0x21,
    0xf9, 4, 1, 0, 0, 0, 0, 0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 1, 0, 0x3b,
];

/// Forty KB of text in 80 paragraphs.
fn article() -> String {
    const WORDS: [&str; 12] = [
        "the", "browser", "draws", "a", "page", "from", "text", "and", "styles", "without", "any",
        "script",
    ];
    let paragraph = |p: usize| {
        let text: Vec<&str> = (0..80)
            .map(|i| WORDS[(i * 7 + p * 3) % WORDS.len()])
            .collect();
        format!("<p>{}.</p>", text.join(" "))
    };
    let body: String = (0..80).map(paragraph).collect();
    page(
        "article",
        "<style>body{font:16px/1.5 sans-serif;max-width:40em;margin:2em auto}</style>",
        &format!("<h1>Article</h1>{body}"),
    )
}

/// 160x120 24-bit bitmap, a gradient that depends on `seed`: 57 654 bytes.
pub fn bmp(seed: usize) -> Vec<u8> {
    const W: usize = 160;
    const H: usize = 120;
    let size = 54 + W * H * 3;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(W as u32).to_le_bytes());
    out.extend_from_slice(&(H as u32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in 0..H {
        for x in 0..W {
            out.extend_from_slice(&[
                (x * 255 / W) as u8,
                (y * 255 / H) as u8,
                ((x + y + seed * 37) % 256) as u8,
            ]);
        }
    }
    out
}

fn images() -> String {
    let imgs: String = (0..IMAGES)
        .map(|i| format!("<img src=\"/img/{i}.bmp\" width=160 height=120>"))
        .collect();
    page("images", "", &format!("<h1>Images</h1>{imgs}"))
}

/// About 30 KB of script that defines functions and runs one of them.
pub fn script(i: usize) -> String {
    let mut s = String::with_capacity(32_000);
    for k in 0..600 {
        s.push_str(&format!("function f{i}_{k}(a){{return a*{k}+{i};}}\n"));
    }
    s.push_str(&format!("window.__s=(window.__s||0)+f{i}_0(1);\n"));
    s
}

fn scripts() -> String {
    let tags: String = (0..SCRIPTS)
        .map(|i| format!("<script src=\"/js/{i}.js\"></script>"))
        .collect();
    page("scripts", &tags, "<h1>Scripts</h1>")
}

/// The URL path of the blockable page's `i`th script and image: shapes the default lists block
/// whatever the host is (checked by `--measure blocker`).
pub fn blockable_script(i: usize) -> String {
    format!("/adserver.js?n={i}")
}

pub fn blockable_image(i: usize) -> String {
    format!("/ads/banner-300x250-{i}.gif")
}

fn blockable() -> String {
    let scripts: String = (0..BLOCKABLE_SCRIPTS)
        .map(|i| format!("<script src=\"{}\"></script>", blockable_script(i)))
        .collect();
    let images: String = (0..BLOCKABLE_IMAGES)
        .map(|i| format!("<img src=\"{}\" width=1 height=1>", blockable_image(i)))
        .collect();
    page(
        "blockable",
        &scripts,
        &format!("<h1>Blockable</h1>{images}"),
    )
}

/// 50 000 elements with ids and classes in the HTML, then 5 000 more appended in batches of
/// 500; the script reports how long it took as `build`.
fn bigdom() -> String {
    let nodes: String = (0..BIGDOM_STATIC)
        .map(|i| format!("<div id=n{i} class=\"c{} box\">x</div>", i % 50))
        .collect();
    let script = format!(
        "<script>window.__pending=1;(()=>{{const t=performance.now();\
         let i=0;const batch=()=>{{const root=document.getElementById('root');for(let k=0;k<500;k++,i++){{const d=document.createElement('div');\
         d.id='a'+i;d.className='added c'+(i%50);d.textContent='y';root.appendChild(d);}}\
         if(i<{BIGDOM_APPENDED}){{setTimeout(batch,0);}}else{{window.__build=performance.now()-t;}}}};\
         addEventListener('DOMContentLoaded',batch);}})();</script>"
    );
    page("bigdom", &script, &format!("<div id=root>{nodes}</div>"))
}

/// How long the kit takes V8 to compile and run at document start: the page fetches the kit's
/// source and times 200 `new Function(source)()` calls (`build` is the mean in ms).
fn kit_cost() -> String {
    page(
        "kit cost",
        "<script>window.__pending=1;addEventListener('load',async()=>{\
         const text=await (await fetch('/kit.js')).text();const t=performance.now();\
         for(let i=0;i<200;i++){new Function(text)();}\
         window.__build=(performance.now()-t)/200;});</script>",
        "<h1>Kit cost</h1>",
    )
}

/// A page that asks for speculative requests itself: wubble must add none.
fn speculation() -> String {
    page(
        "speculation",
        "<link rel=preconnect href=\"/\"><link rel=prefetch href=\"/spec/a\">\
         <script type=speculationrules>{\"prefetch\":[{\"urls\":[\"/spec/b\"]}]}</script>",
        "<h1>Speculation</h1>",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(server: &LoadServer, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .expect("send");
        let mut out = Vec::new();
        // The server keeps the connection open: read the head, then the length it names.
        let mut reader = BufReader::new(s);
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("head");
            if line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().expect("length");
            }
            head.push_str(&line);
        }
        out.resize(length, 0);
        reader.read_exact(&mut out).expect("body");
        format!("{head}\n{}", String::from_utf8_lossy(&out))
    }

    #[test]
    fn request_lines() {
        assert_eq!(
            parse_request_line("GET /a/b?w=1 HTTP/1.1\r\n"),
            Some(("GET", "/a/b", "w=1"))
        );
        assert_eq!(
            parse_request_line("POST /__t HTTP/1.1"),
            Some(("POST", "/__t", ""))
        );
        assert_eq!(parse_request_line(""), None);
    }

    #[test]
    fn timing_bodies() {
        let t = parse_timing(r#"{"page":"/article","fp":10.5,"fcp":null,"dcl":30,"load":40.2}"#)
            .expect("parses");
        assert_eq!(t.page, "/article");
        assert_eq!(
            (t.fp, t.fcp, t.dcl, t.load, t.build),
            (Some(10.5), None, Some(30.0), Some(40.2), None)
        );
        assert_eq!(t.s0, None);
        assert_eq!(parse_timing("nope"), None);
        assert_eq!(parse_timing(r#"{"fp":1}"#), None);
    }

    #[test]
    fn pages_have_the_stated_shape() {
        let blockable = String::from_utf8(route("/blockable").expect("page").1).expect("utf8");
        assert_eq!(blockable.matches("<script src=").count(), BLOCKABLE_SCRIPTS);
        assert_eq!(blockable.matches("<img src=").count(), BLOCKABLE_IMAGES);
        let big = String::from_utf8(route("/bigdom").expect("page").1).expect("utf8");
        assert_eq!(big.matches("<div id=n").count(), BIGDOM_STATIC);
        let images = String::from_utf8(route("/images").expect("page").1).expect("utf8");
        assert_eq!(images.matches("<img src=").count(), IMAGES);
        let scripts = String::from_utf8(route("/scripts").expect("page").1).expect("utf8");
        assert_eq!(scripts.matches("<script src=").count(), SCRIPTS);
        for page in PAGES
            .iter()
            .chain(&["/kitcost", "/kit.js", "/spec", "/blank"])
        {
            assert!(route(page).is_some(), "{page}");
        }
        assert!(route("/nope").is_none());
        // A bitmap is its header's size.
        let bmp = bmp(3);
        assert_eq!(
            u32::from_le_bytes(bmp[2..6].try_into().expect("4")) as usize,
            bmp.len()
        );
        assert_eq!(bmp, super::bmp(3));
    }

    #[test]
    fn the_server_answers_and_logs() {
        let server = LoadServer::start(0, false).expect("bind");
        let reply = get(&server, "/article?w=1");
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
        assert!(reply.contains("Cache-Control: max-age=3600"));
        assert!(get(&server, "/nope").starts_with("HTTP/1.1 404"));
        assert_eq!(server.log_since(0), ["GET /article?w=1", "GET /nope"]);
        assert_eq!(server.take_timing(), None);
        let mut s = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
        let body = r#"{"page":"/article","dcl":1}"#;
        write!(
            s,
            "POST /__t HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .expect("send");
        let mut head = String::new();
        BufReader::new(s).read_line(&mut head).expect("reply");
        assert!(head.starts_with("HTTP/1.1 204"));
        assert_eq!(
            server.take_timing().map(|t| t.page),
            Some("/article".into())
        );
        // The sink is not a page request.
        assert_eq!(server.requests(), 2);
    }
}
