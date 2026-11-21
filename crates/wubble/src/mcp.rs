//! The MCP server's transport (docs/research/mcp-server.md section 3, "The workflow and
//! transport"). With `agent.mcp_enabled` on, the browser listens on a second private socket in
//! its profile (`instance::Socket::Mcp`: a Unix socket made 0600 before it listens, a named pipe
//! on Windows; never a TCP port). Each connection gets a reader thread, which frames its bytes
//! into lines and answers what needs no browser itself (`wubble_mcp::handle_line`: discovery,
//! the tool list, protocol errors), and a writer thread, which sends whatever the reader or the
//! host queued on the connection's [`Outbox`]; tool calls go to the host (`host/agent.rs`) as
//! [`Event`]s on the Slint thread.
//!
//! `wubble mcp` ([`relay`]) is what a harness starts: it speaks MCP on stdio and copies bytes
//! to and from that socket; it starts no CEF, GPU or store. With no browser on the profile it
//! answers every request with an MCP error saying so.
//!
//! The socket code is `instance.rs`'s (the `interprocess` crate, 0BSD OR Apache-2.0, as its
//! `local_socket` example shows).

use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use wubble_core::agent::manifest::{self, Manifest};

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, Listener, ListenerOptions, Stream};
use serde_json::Value;
use wubble_mcp::{AgentCall, CallId, Incoming, LineFramer, Peer, RpcError, encode_line};

/// One connection to the MCP socket, numbered from 1 in the order they came.
pub type ConnId = u64;

/// What the error says when `wubble mcp` finds no browser to relay to.
pub const NOT_RUNNING: &str = "wubble is not running on this profile, or its MCP server is off: \
     start wubble and turn hosting on in Settings > Agents (MCP)";

/// The replies of one connection, written in order by its writer thread.
#[derive(Clone, Debug)]
pub struct Outbox(mpsc::Sender<Value>);

impl Outbox {
    /// Queues `message`; a connection already gone drops it.
    pub fn send(&self, message: Value) {
        let _ = self.0.send(message);
    }
}

/// What a connection tells the host.
#[derive(Debug)]
pub enum Event {
    Connected {
        conn: ConnId,
        out: Outbox,
        /// The peer as the OS names it (pid and executable), for the approval sheet.
        client: String,
    },
    /// A validated `tools/call`: the host answers with `tool_response(&id, ..)` on `out`.
    Call {
        conn: ConnId,
        id: CallId,
        call: AgentCall,
    },
    /// The peer closed the connection (or it broke).
    Gone(ConnId),
}

/// Binds the MCP socket at `path`. The instance socket already made this the profile's one
/// browser, so a file found here is a crash's and goes.
pub fn bind(path: &Path) -> io::Result<Listener> {
    crate::instance::remove_stale(path)?;
    let options = ListenerOptions::new().name(path.to_fs_name::<GenericFilePath>()?);
    // Only this user may connect (set before `listen`, so no umask race).
    #[cfg(unix)]
    let options = {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        options.mode(0o600)
    };
    options.create_sync()
}

/// A bound, listening MCP socket: [`Hosted::stop`] stops accepting and removes it.
pub struct Hosted {
    path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
}

impl Hosted {
    /// Binds `path` and accepts on it ([`listen`]) until stopped.
    pub fn start(
        path: &Path,
        deliver: impl Fn(Event) + Send + Sync + 'static,
    ) -> io::Result<Hosted> {
        let listener = bind(path)?;
        let stop = Arc::new(AtomicBool::new(false));
        listen_until(listener, Arc::clone(&stop), deliver);
        Ok(Hosted {
            path: path.to_owned(),
            stop,
        })
    }

    /// No new connection is taken, and the socket file goes. Connections already open end
    /// with their next line (the host answers nothing on them meanwhile).
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        // wakes the accept loop, which sees the flag and ends
        if let Ok(name) = self.path.as_path().to_fs_name::<GenericFilePath>() {
            let _ = Stream::connect(name);
        }
        let _ = crate::instance::remove_stale(&self.path);
    }
}

fn listen_until(
    listener: Listener,
    stop: Arc<AtomicBool>,
    deliver: impl Fn(Event) + Send + Sync + 'static,
) {
    let deliver = Arc::new(deliver);
    let spawned = std::thread::Builder::new()
        .name("wubble-mcp".into())
        .spawn(move || {
            for (n, stream) in listener.incoming().filter_map(Result::ok).enumerate() {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let conn = n as ConnId + 1;
                let deliver = Arc::clone(&deliver);
                let stop = Arc::clone(&stop);
                let served = std::thread::Builder::new()
                    .name("wubble-mcp-conn".into())
                    .spawn(move || serve(conn, stream, &stop, &*deliver));
                if let Err(e) = served {
                    eprintln!("wubble: mcp connection {conn}: {e}");
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("wubble: no MCP listener: {e}");
    }
}

/// One connection: reads until it ends, then says so.
fn serve(conn: ConnId, stream: Stream, stop: &AtomicBool, deliver: &(dyn Fn(Event) + Send + Sync)) {
    let client = client_of(&stream);
    // `&Stream` reads and writes: one thread each.
    let stream = Arc::new(stream);
    let send = Arc::clone(&stream);
    let (tx, rx) = mpsc::channel::<Value>();
    let writer = std::thread::Builder::new()
        .name("wubble-mcp-write".into())
        .spawn(move || {
            for message in rx {
                if (&*send)
                    .write_all(encode_line(&message).as_bytes())
                    .is_err()
                {
                    break;
                }
            }
        });
    if let Err(e) = writer {
        eprintln!("wubble: mcp connection {conn}: {e}");
        return;
    }
    let out = Outbox(tx);
    deliver(Event::Connected {
        conn,
        out: out.clone(),
        client,
    });
    let mut framer = LineFramer::default();
    let mut peer = Peer::default();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match (&*stream).read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if stop.load(Ordering::SeqCst) {
            break;
        }
        for line in framer.push(&buf[..n]) {
            match line {
                Err(_) => out.send(RpcError::Parse.response(None)),
                Ok(line) => match peer.handle(&line) {
                    Incoming::Reply(reply) => out.send(reply),
                    Incoming::Call { id, call } => deliver(Event::Call { conn, id, call }),
                    Incoming::Ignore => {}
                },
            }
        }
    }
    deliver(Event::Gone(conn));
}

/// The directory of the hosting browsers' manifests (`wubble_core::agent::manifest`).
pub fn manifest_dir() -> std::path::PathBuf {
    manifest::DIR
        .iter()
        .fold(crate::instance::runtime_dir(), |d, p| d.join(p))
}

/// The manifest of the browser on `profile`.
pub fn manifest_path(profile: &Path) -> std::path::PathBuf {
    let hash = crate::instance::fnv1a(profile.as_os_str().as_encoded_bytes());
    manifest_dir().join(format!("{hash:016x}.json"))
}

/// Writes `m` at `path`, the directory private to the user.
pub fn write_manifest(path: &Path, m: &Manifest) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for d in [dir, dir.parent().unwrap_or(dir)] {
                std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, m.to_json())?;
    std::fs::rename(tmp, path)
}

/// The browsers hosting now: every manifest whose socket answers; a stale one (its browser
/// gone) is removed on the way.
pub fn hosting_browsers() -> Vec<Manifest> {
    let Ok(entries) = std::fs::read_dir(manifest_dir()) else {
        return vec![];
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let m = Manifest::from_json(&std::fs::read_to_string(&p).ok()?);
            let live = m.as_ref().is_some_and(|m| {
                Path::new(&m.socket)
                    .to_fs_name::<GenericFilePath>()
                    .ok()
                    .and_then(|n| Stream::connect(n).ok())
                    .is_some()
            });
            if !live {
                let _ = std::fs::remove_file(&p);
            }
            m.filter(|_| live)
        })
        .collect()
}

/// The peer's pid and executable, as the OS reports them (never the client's own claim).
fn client_of(stream: &Stream) -> String {
    let Some(pid) = stream.peer_creds().ok().and_then(|c| c.pid()) else {
        return "a local program".into();
    };
    #[cfg(target_os = "linux")]
    if let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) {
        return format!("{} (pid {pid})", exe.display());
    }
    format!("pid {pid}")
}

/// How long the relay still passes answers on after its client closed stdin.
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// `wubble mcp`: stdio to the socket at `path` and back, until either side ends. With no
/// browser there, every request on stdin gets the [`NOT_RUNNING`] error.
pub fn relay(path: &Path) -> io::Result<()> {
    let stream = match Stream::connect(path.to_fs_name::<GenericFilePath>()?) {
        Ok(stream) => stream,
        Err(_) => {
            eprintln!("wubble mcp: {NOT_RUNNING}");
            return answer_offline(io::stdin().lock(), io::stdout().lock());
        }
    };
    let stream = Arc::new(stream);
    let send = Arc::clone(&stream);
    std::thread::Builder::new()
        .name("wubble-mcp-stdin".into())
        .spawn(move || {
            let _ = io::copy(&mut io::stdin().lock(), &mut &*send);
            // The harness closed stdin: the session ends with the connection, once the answers
            // to what it already asked (a script piping a few requests in) had time to arrive.
            std::thread::sleep(CLOSE_GRACE);
            std::process::exit(0);
        })?;
    io::copy(&mut &*stream, &mut io::stdout().lock())?;
    Ok(())
}

/// Every request on stdin answered with `why`, until stdin ends.
pub fn answer_offline_with(why: &str) -> io::Result<()> {
    offline(io::stdin().lock(), io::stdout().lock(), why)
}

/// Every request line of `input` answered with the [`NOT_RUNNING`] error on `output`.
fn answer_offline(input: impl BufRead, output: impl Write) -> io::Result<()> {
    offline(input, output, NOT_RUNNING)
}

fn offline(input: impl BufRead, mut output: impl Write, why: &str) -> io::Result<()> {
    for line in input.lines() {
        if let Some(reply) = wubble_mcp::unavailable(&line?, why) {
            output.write_all(encode_line(&reply).as_bytes())?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn without_a_browser_requests_are_answered_with_an_error() {
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\"}\n\
            {\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\"}\n";
        let mut out = vec![];
        answer_offline(&input[..], &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 1, "the notification gets nothing");
        let reply: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["error"]["message"], NOT_RUNNING);
    }

    #[test]
    fn a_connection_answers_discovery_and_hands_calls_to_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::instance::path_of(
            crate::instance::Socket::Mcp,
            dir.path(),
            &crate::instance::runtime_dir(),
        );
        let (tx, rx) = mpsc::channel();
        let hosted = Hosted::start(&path, move |e| tx.send(e).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only this user may connect");
        }
        let mut stream =
            Stream::connect(path.as_path().to_fs_name::<GenericFilePath>().unwrap()).unwrap();
        let Event::Connected { conn, out, .. } = rx.recv().unwrap() else {
            panic!("connected first");
        };
        stream
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\
                   \"params\":{\"name\":\"list_tabs\",\"arguments\":{\"session\":\"s\"}}}\n",
            )
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let discover: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(discover["id"], 1);
        assert!(discover["result"]["supportedVersions"].is_array());
        let Event::Call { conn: c, id, call } = rx.recv().unwrap() else {
            panic!("the call goes to the host");
        };
        assert_eq!((c, call.tool()), (conn, "list_tabs"));
        // the host's answer goes back on the same connection
        out.send(wubble_mcp::tool_response(
            &id,
            &wubble_mcp::ToolResult::text("[]"),
        ));
        line.clear();
        reader.read_line(&mut line).unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(answer["id"], 2);
        assert_eq!(answer["result"]["content"][0]["text"], "[]");
        drop(reader);
        assert!(matches!(rx.recv().unwrap(), Event::Gone(c) if c == conn));
        // stopped: the socket is gone and a new connection is refused
        hosted.stop();
        assert!(!path.exists());
        assert!(Stream::connect(path.to_fs_name::<GenericFilePath>().unwrap()).is_err());
    }
}
