//! `--smoke data`: the built-in pages and the data behind them, driven on a throwaway profile
//! against a loopback server. Two runs of the same script (`host/data.rs`): one ends with a
//! clean quit, one with a SIGKILL once the store has answered its last read. After each the
//! parent opens the database and compares it with what the pages showed at the end of the run
//! (`summary.txt`), so what a page shows is what a start finds.
//!
//! This file is the edge: the loopback server (pages, cookies that are echoed back, a counted
//! cacheable page, downloads held open until released) and the parent that runs the halves.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Runs of CEF work one step may take before the smoke gives up (a hang, never a speed).
pub const MAX_STEP_RUNS: u32 = 4_000;

/// The size of every file the server offers.
pub const FILE_BYTES: usize = 64 * 1024;
/// What a held file sends before it waits for [`Server::release`].
const FIRST_CHUNK: usize = 16 * 1024;

/// The title of `/uni`: Cyrillic and a Latin letter with a diacritic, in capitals as titles are.
pub const UNI_TITLE: &str = "Привет мир ÄRGER";

/// The net log the quit run writes (`net_audit`), in its profile directory.
pub const NET_LOG: &str = "net-log.json";

/// The session bus address the runs get: nothing listens there, so a download's Show in folder
/// (a D-Bus call to the file manager) fails and falls back to the opener's stand-in.
pub const NO_BUS: &str = "unix:path=/nonexistent/wubble-data-smoke-bus";

/// How a run ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The window closes and the store shuts down.
    Quit,
    /// SIGKILL, once the store has answered a read sent after the last write.
    Kill,
    /// `--smoke private`'s script (`host/data/private.rs`) in place of the pages', ending in a
    /// quit; the parent then looks for the private pages everywhere ([`private_leaks`]).
    Private,
}

/// The hosts (on loopback: Chromium resolves `*.localhost` there) of `--smoke private`'s normal
/// and private pages, so each has its own site and every trace of one is found by its name.
pub const NORMAL_HOST: &str = "normalmark";
pub const PRIVATE_HOST: &str = "privmark";

/// The loopback server's state, shared with the host's steps.
#[derive(Debug, Default)]
pub struct Server {
    pub port: u16,
    /// Files wait before their last bytes while this is false.
    release: AtomicBool,
    /// Requests per path (`/count`, `/file`).
    counted: AtomicU32,
    files: AtomicU32,
    fails: AtomicU32,
}

impl Server {
    pub fn release(&self, on: bool) {
        self.release.store(on, Ordering::SeqCst);
    }

    /// How many times `/count` was served (a cached load does not reach the server).
    pub fn counted(&self) -> u32 {
        self.counted.load(Ordering::SeqCst)
    }

    /// How many times `/fail` was asked for (a resume asks again).
    pub fn fails(&self) -> u32 {
        self.fails.load(Ordering::SeqCst)
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// `path` on `host.localhost` ([`NORMAL_HOST`], [`PRIVATE_HOST`]).
    pub fn url_at(&self, host: &str, path: &str) -> String {
        format!("http://{host}.localhost:{}{path}", self.port)
    }
}

/// Serves on a free loopback port, each connection on its own thread (Chromium opens spare
/// connections and holds them idle).
pub fn serve() -> std::io::Result<Arc<Server>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = Arc::new(Server {
        port,
        ..Server::default()
    });
    let shared = Arc::clone(&server);
    std::thread::Builder::new()
        .name("data-smoke-http".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let server = Arc::clone(&shared);
                let _ = std::thread::Builder::new()
                    .name("data-smoke-http-conn".into())
                    .spawn(move || {
                        // a client that went away mid-file is the point of some steps
                        let _ = answer(stream, &server);
                    });
            }
        })?;
    Ok(server)
}

fn page(title: &str) -> String {
    format!("<!doctype html><meta charset=utf-8><title>{title}</title><p>{title}")
}

fn html(stream: &mut TcpStream, extra: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n{extra}\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The head of a file answer: an attachment of `FILE_BYTES`.
fn attachment(stream: &mut TcpStream, name: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{name}\"\r\n\
         Content-Length: {FILE_BYTES}\r\nConnection: close\r\n\r\n"
    )
}

fn answer(mut stream: TcpStream, server: &Server) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut cookie = None;
    let mut ranged = false;
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        ranged |= lower.starts_with("range:");
        if lower.starts_with("cookie:") {
            cookie = Some(header["cookie:".len()..].trim().to_owned());
        }
    }
    reader.take(length).read_to_end(&mut Vec::new())?;
    match path.as_str() {
        "/page" => html(&mut stream, "", &page("Page One")),
        "/other" => html(&mut stream, "", &page("Page Two")),
        "/uni" => html(&mut stream, "", &page(UNI_TITLE)),
        // `--smoke private`'s pages: their titles carry their host's mark too
        "/a" => html(&mut stream, "", &page("normalmark A")),
        "/b" => html(&mut stream, "", &page("privmark B")),
        "/b2" => html(&mut stream, "", &page("privmark B2")),
        // a sign-in form and a textarea: what Chromium's own autofill and spelling services ask
        // their servers about (`net_audit`)
        "/form" => html(
            &mut stream,
            "",
            "<!doctype html><meta charset=utf-8><title>Form</title>\
             <form action=/form method=post><input name=user autocomplete=username>\
             <input name=pass type=password autocomplete=current-password></form>\
             <textarea spellcheck=true>teh wrod</textarea>",
        ),
        // a persistent cookie for the whole site
        "/set" => html(
            &mut stream,
            "Set-Cookie: sid=abc123; Path=/; Max-Age=3600\r\n",
            &page("cookie set"),
        ),
        // the title is what the server got: the cookie a tab sends
        "/echo" => html(
            &mut stream,
            "Cache-Control: no-store\r\n",
            &page(&format!("cookie={}", cookie.as_deref().unwrap_or("none"))),
        ),
        // cacheable for an hour, numbered by how many times it was served
        "/count" => {
            let n = server.counted.fetch_add(1, Ordering::SeqCst) + 1;
            html(
                &mut stream,
                "Cache-Control: public, max-age=3600\r\n",
                &page(&format!("n={n}")),
            )
        }
        "/small" => {
            attachment_whole(&mut stream, "small.bin")?;
            Ok(())
        }
        // closes after 8 KiB of 64: an interrupted download
        "/fail" => {
            server.fails.fetch_add(1, Ordering::SeqCst);
            // a resume is refused, so the interruption stands
            if ranged {
                return stream.write_all(
                    b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\n\
                      Connection: close\r\n\r\n",
                );
            }
            attachment(&mut stream, "fail.bin")?;
            stream.write_all(&vec![7; 8 * 1024])?;
            stream.flush()
        }
        p if p.starts_with("/file/") => {
            server.files.fetch_add(1, Ordering::SeqCst);
            attachment(&mut stream, &p["/file/".len()..])?;
            stream.write_all(&vec![1; FIRST_CHUNK])?;
            stream.flush()?;
            // hold the rest until released; a write error (the client cancelled) ends it
            let mut waited = 0;
            while !server.release.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(20));
                waited += 1;
                if waited > 3_000 {
                    return Ok(());
                }
            }
            stream.write_all(&vec![2; FILE_BYTES - FIRST_CHUNK])?;
            stream.flush()
        }
        _ => stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}

fn attachment_whole(stream: &mut TcpStream, name: &str) -> std::io::Result<()> {
    let body = [3u8; 100];
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{name}\"\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)
}

/// The summary a run writes before it ends: what its pages showed, one line each.
pub const SUMMARY: &str = "summary.txt";

/// What a database holds, in [`SUMMARY`]'s lines (sorted): the parent's side of the check.
pub fn stored_lines(db: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    use wubble_store::Store;
    let store = pollster::block_on(Store::open(db))?;
    let mut lines = Vec::new();
    for v in pollster::block_on(store.list_visits(10_000, None, ""))? {
        lines.push(format!("history {}|{}", v.url, v.title));
    }
    for b in pollster::block_on(store.all_bookmarks())? {
        lines.push(format!(
            "bookmark {}|{}",
            b.title,
            b.url.unwrap_or_else(|| "(folder)".into())
        ));
    }
    for d in pollster::block_on(store.downloads(10_000))? {
        lines.push(format!("download {}|{}", d.name, d.state.as_str()));
    }
    for a in pollster::block_on(store.list_archive(10_000, None, ""))? {
        lines.push(format!("archive {}|{}", a.url, a.title));
    }
    lines.sort();
    Ok(lines)
}

/// Compares what the run's pages showed with what the database holds. A download a kill left
/// running reads as failed on the next start, not in the file: `download` lines are compared by
/// name only for a kill.
pub fn compare(shown: &str, db: &Path, ending: Ending) -> Result<(), String> {
    let mut shown: Vec<String> = shown.lines().map(str::to_owned).collect();
    shown.sort();
    let stored = stored_lines(db).map_err(|e| format!("cannot read the database: {e}"))?;
    let strip = |lines: Vec<String>| -> Vec<String> {
        lines
            .into_iter()
            .map(|l| match (ending, l.strip_prefix("download ")) {
                (Ending::Kill, Some(rest)) => {
                    format!("download {}", rest.split('|').next().unwrap_or_default())
                }
                _ => l,
            })
            .collect()
    };
    let (shown, stored) = (strip(shown), strip(stored));
    if shown == stored {
        return Ok(());
    }
    let missing: Vec<&String> = shown.iter().filter(|l| !stored.contains(l)).collect();
    let extra: Vec<&String> = stored.iter().filter(|l| !shown.contains(l)).collect();
    Err(format!(
        "after a {ending:?} the database differs from what the pages showed: \
         shown but not stored {missing:?}; stored but not shown {extra:?}"
    ))
}

/// `--smoke private`'s check after its run quit, on its profile `dir`: the store holds the
/// normal page (history, session) and nothing of the private one (history, closed tabs, the
/// session, downloads, site permissions, settings: zoom and themes among them); no file under
/// the profile holds the private host's name (as UTF-8 or UTF-16) while the normal one's is
/// found (so the scan can see); and no container directory is left of a container the session
/// does not have (the private windows' in-memory one).
pub fn private_leaks(dir: &Path, db: &Path) -> Result<(), String> {
    use wubble_store::Store;
    let fail = |e: wubble_store::Error| format!("cannot read the database: {e}");
    let store = pollster::block_on(Store::open(db)).map_err(fail)?;
    let visits: Vec<String> = pollster::block_on(store.list_visits(10_000, None, ""))
        .map_err(fail)?
        .into_iter()
        .map(|v| format!("{}|{}", v.url, v.title))
        .collect();
    let closed: Vec<String> = pollster::block_on(store.recently_closed())
        .map_err(fail)?
        .into_iter()
        .map(|c| format!("{c:?}"))
        .collect();
    let session =
        pollster::block_on(store.load_session(std::time::Instant::now())).map_err(fail)?;
    let tabs: Vec<String> = session
        .tabs
        .values()
        .map(|t| format!("{}|{}", t.url, t.title))
        .collect();
    let downloads: Vec<String> = pollster::block_on(store.downloads(10_000))
        .map_err(fail)?
        .into_iter()
        .map(|d| format!("{d:?}"))
        .collect();
    let permissions: Vec<String> = pollster::block_on(store.site_permissions())
        .map_err(fail)?
        .into_iter()
        .map(|p| format!("{p:?}"))
        .collect();
    let settings: Vec<String> = pollster::block_on(store.all_settings())
        .map_err(fail)?
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    drop(store);
    // (what, its lines, whether the normal page must be there to prove the check sees it)
    let tables: [(&str, &[String], bool); 6] = [
        ("history", &visits, true),
        ("closed tabs", &closed, false),
        ("the session's tabs", &tabs, true),
        ("downloads", &downloads, false),
        ("site permissions", &permissions, false),
        ("settings", &settings, true),
    ];
    for (what, lines, proves) in tables {
        if let Some(leak) = lines.iter().find(|l| l.contains(PRIVATE_HOST)) {
            return Err(format!("{what} holds the private page: {leak}"));
        }
        if proves && !lines.iter().any(|l| l.contains(NORMAL_HOST)) {
            return Err(format!("{what} lacks the normal page: {lines:?}"));
        }
    }
    let utf16 = |s: &str| -> Vec<u8> { s.encode_utf16().flat_map(u16::to_le_bytes).collect() };
    let marks = [PRIVATE_HOST.as_bytes().to_vec(), utf16(PRIVATE_HOST)];
    let found = |bytes: &[u8], mark: &[u8]| bytes.windows(mark.len()).any(|w| w == mark);
    let mut seen_normal = false;
    let mut left = vec![dir.to_path_buf()];
    while let Some(at) = left.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                if let Some(id) = name
                    .to_str()
                    .and_then(|n| n.strip_prefix("container-"))
                    .and_then(|n| n.parse::<u32>().ok())
                    && !session.containers.iter().any(|c| c.id.0 == id)
                {
                    return Err(format!(
                        "{} is left of a container the session does not have",
                        path.display()
                    ));
                }
                left.push(path);
            } else if kind.is_file()
                && let Ok(bytes) = std::fs::read(&path)
            {
                if let Some(mark) = marks.iter().find(|m| found(&bytes, m)) {
                    let how = if mark.len() == PRIVATE_HOST.len() {
                        "UTF-8"
                    } else {
                        "UTF-16"
                    };
                    return Err(format!(
                        "{} holds the private host's name ({how})",
                        path.display()
                    ));
                }
                seen_normal |= found(&bytes, NORMAL_HOST.as_bytes());
            }
        }
    }
    if !seen_normal {
        return Err("no file holds the normal host's name: the scan sees nothing".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kill_compares_downloads_by_name_only() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("wubble.db");
        {
            let mut store = pollster::block_on(wubble_store::Store::open(&db)).unwrap();
            pollster::block_on(store.add_download(&wubble_core::download::DownloadEntry {
                id: 1,
                url: "http://a.test/f".into(),
                name: "f.bin".into(),
                path: "/d/f.bin".into(),
                received: 1,
                total: Some(2),
                state: wubble_core::download::DownloadState::Running,
                started_ms: 0,
                container: wubble_core::space::ContainerId::DEFAULT,
            }))
            .unwrap();
        }
        assert!(compare("download f.bin|complete", &db, Ending::Kill).is_ok());
        assert!(compare("download f.bin|complete", &db, Ending::Quit).is_err());
        assert!(compare("download g.bin|complete", &db, Ending::Kill).is_err());
    }
}
