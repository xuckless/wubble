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
    // Only `Hosted::stop` removes the file, before it returns: a listener reclaiming its name on
    // drop would do it later, from the accept thread winding down, and could take the socket a
    // following `start` just bound at the same path.
    let options = || {
        path.to_fs_name::<GenericFilePath>()
            .map(|name| ListenerOptions::new().name(name).reclaim_name(false))
    };
    // Only this user may connect (set before `listen`, so no umask race).
    #[cfg(unix)]
    {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        match options()?.mode(0o600).create_sync() {
            Err(e) if e.kind() == io::ErrorKind::Unsupported => private_by_dir(path),
            other => other,
        }
    }
    #[cfg(not(unix))]
    options()?.create_sync()
}

/// Binds where a socket's mode cannot be set before `bind` (macOS: `fchmod` on a socket fails,
/// which interprocess reports as `Unsupported`): the socket is bound inside a unique `0o700`
/// temporary directory only this user may enter, given its `0o600` mode there, then linked or
/// renamed into place (a bound socket keeps listening under its new name). It is never reachable
/// by another user, and the process's umask is left alone (other threads make files meanwhile).
#[cfg(unix)]
fn private_by_dir(path: &Path) -> io::Result<Listener> {
    private_by_dir_impl(path, false)
}

#[cfg(unix)]
pub(crate) fn private_by_dir_impl(path: &Path, exclusive: bool) -> io::Result<Listener> {
    use std::os::unix::fs::PermissionsExt;
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    // short unique name: a socket path has a small limit, and `path` already fits it;
    // `tempfile::Builder::tempdir_in` creates the directory with mode 0o700 on Unix.
    let dir = tempfile::Builder::new()
        .prefix(".m-")
        .rand_bytes(4)
        .tempdir_in(parent)?;
    let inside = dir.path().join("s");
    let listener = ListenerOptions::new()
        .name(inside.as_path().to_fs_name::<GenericFilePath>()?)
        // the file is moved/linked away: nothing of this name is left to reclaim
        .reclaim_name(false)
        .create_sync()?;
    std::fs::set_permissions(&inside, std::fs::Permissions::from_mode(0o600))?;
    if exclusive {
        std::fs::hard_link(&inside, path).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                io::Error::from(io::ErrorKind::AddrInUse)
            } else {
                e
            }
        })?;
    } else {
        std::fs::rename(&inside, path)?;
    }
    Ok(listener)
}

/// Connection ids, process-wide: a connection left from an earlier hosting run never shares
/// one with a later run's.
static NEXT_CONN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The connections open on one hosting run, to close when it stops.
type Open = Arc<std::sync::Mutex<Vec<(ConnId, Arc<Stream>)>>>;

/// Both directions of `stream` closed: a thread blocked reading it wakes with its end.
fn shut(stream: &Stream) {
    // on Unix a local socket is only ever a Unix domain socket
    #[cfg(unix)]
    {
        let Stream::UdSocket(s) = stream;
        let _ = s.inner().shutdown(std::net::Shutdown::Both);
    }
    #[cfg(not(unix))]
    let _ = stream;
}

/// A bound, listening MCP socket: [`Hosted::stop`] stops accepting, closes the connections it
/// took and removes the socket.
pub struct Hosted {
    path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    open: Open,
}

impl Hosted {
    /// Binds `path` and accepts on it ([`listen`]) until stopped.
    pub fn start(
        path: &Path,
        deliver: impl Fn(Event) + Send + Sync + 'static,
    ) -> io::Result<Hosted> {
        let listener = bind(path)?;
        let stop = Arc::new(AtomicBool::new(false));
        let open: Open = Arc::default();
        listen_until(listener, Arc::clone(&stop), Arc::clone(&open), deliver);
        Ok(Hosted {
            path: path.to_owned(),
            stop,
            open,
        })
    }

    /// No new connection is taken, the open ones are closed (each reports `Gone`), and the
    /// socket file goes.
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        let open = std::mem::take(&mut *self.open.lock().unwrap_or_else(|e| e.into_inner()));
        for (_, stream) in open {
            shut(&stream);
        }
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
    open: Open,
    deliver: impl Fn(Event) + Send + Sync + 'static,
) {
    let deliver = Arc::new(deliver);
    let spawned = std::thread::Builder::new()
        .name("wubble-mcp".into())
        .spawn(move || {
            for stream in listener.incoming().filter_map(Result::ok) {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let conn = NEXT_CONN.fetch_add(1, Ordering::SeqCst);
                let stream = Arc::new(stream);
                open.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((conn, Arc::clone(&stream)));
                let deliver = Arc::clone(&deliver);
                let stop = Arc::clone(&stop);
                let open = Arc::clone(&open);
                let served = std::thread::Builder::new()
                    .name("wubble-mcp-conn".into())
                    .spawn(move || {
                        serve(conn, stream, &stop, &*deliver);
                        open.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .retain(|(c, _)| *c != conn);
                    });
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
fn serve(
    conn: ConnId,
    stream: Arc<Stream>,
    stop: &AtomicBool,
    deliver: &(dyn Fn(Event) + Send + Sync),
) {
    let client = client_of(&stream);
    // `&Stream` reads and writes: one thread each.
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

/// The directory of the hosting browsers' manifests (`wubble_core::agent::manifest`), in the
/// user's private root (`instance::private_root`). The shared temp root is its last resort only
/// (no `HOME`): there the readers' check that it is the user's own and private
/// (`private_to_me`) keeps another user's planted one out.
pub fn manifest_dir() -> std::path::PathBuf {
    manifest::DIR
        .iter()
        .fold(crate::instance::private_root(), |d, p| d.join(p))
}

/// The manifest of the browser on `profile`.
pub fn manifest_path(profile: &Path) -> std::path::PathBuf {
    let hash = crate::instance::fnv1a(profile.as_os_str().as_encoded_bytes());
    manifest_dir().join(format!("{hash:016x}.json"))
}

/// What a path must be for the relay to trust it.
#[derive(Clone, Copy)]
enum Kind {
    Dir,
    File,
    Socket,
}

/// Whether `path` is a `kind` (not a link) owned by this user with no group or other access.
#[cfg(unix)]
fn private_to_me(path: &Path, kind: Kind) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let ft = meta.file_type();
    let right_kind = match kind {
        Kind::Dir => ft.is_dir(),
        Kind::File => ft.is_file(),
        Kind::Socket => ft.is_socket(),
    };
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    right_kind && meta.uid() == me && meta.mode() & 0o077 == 0
}

/// Windows: the manifests sit in the user's own `%LOCALAPPDATA%`; a named pipe is no file.
#[cfg(not(unix))]
fn private_to_me(path: &Path, kind: Kind) -> bool {
    match kind {
        Kind::Dir => path.is_dir(),
        Kind::File => path.is_file(),
        Kind::Socket => true,
    }
}

/// Writes `m` at `path`: the directories 0700, the file 0600 from its creation.
pub fn write_manifest(path: &Path, m: &Manifest) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("a manifest with no directory"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for d in [dir, dir.parent().unwrap_or(dir)] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(m.to_json().as_bytes())?;
    file.sync_all()?;
    std::fs::rename(tmp, path)
}

/// The browsers hosting now: every trusted manifest (`private_to_me`: directory, file and
/// socket this user's own and private) whose socket answers. One whose socket is gone for good
/// (`instance::probe`: refused or not found) is removed; one that could not be checked is only
/// skipped.
pub fn hosting_browsers() -> Vec<Manifest> {
    let dir = manifest_dir();
    if !private_to_me(&dir, Kind::Dir) {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter(|p| private_to_me(p, Kind::File))
        .filter_map(|p| {
            let m = Manifest::from_json(&std::fs::read_to_string(&p).ok()?)?;
            let socket = Path::new(&m.socket);
            if !private_to_me(socket, Kind::Socket) {
                #[cfg(unix)]
                if std::fs::symlink_metadata(socket)
                    .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
                {
                    let _ = std::fs::remove_file(&p);
                }
                return None;
            }
            match crate::instance::probe(socket) {
                crate::instance::Probe::Live => Some(m),
                crate::instance::Probe::Dead => {
                    let _ = std::fs::remove_file(&p);
                    None
                }
                crate::instance::Probe::Unknown => None,
            }
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
    if !private_to_me(path, Kind::Socket) {
        eprintln!("wubble mcp: {NOT_RUNNING}");
        return answer_offline(io::stdin().lock(), io::stdout().lock());
    }
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

    #[cfg(unix)]
    #[test]
    fn a_socket_bound_in_a_private_directory_is_private_and_listens_under_its_name() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.sock");
        let listener = private_by_dir(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            entries,
            [std::ffi::OsStr::new("m.sock")],
            "the directory it was made in is gone"
        );
        Stream::connect(path.as_path().to_fs_name::<GenericFilePath>().unwrap()).unwrap();
        drop(listener);
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

    /// Hosting off and on again while an old client is still connected: the old connection is
    /// closed by the stop, and its id is never one a new connection gets, so its going cannot
    /// end a new client's session.
    #[test]
    fn a_stop_closes_open_connections_and_ids_never_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::instance::path_of(
            crate::instance::Socket::Mcp,
            dir.path(),
            &crate::instance::runtime_dir(),
        );
        let connect = || Stream::connect(path.as_path().to_fs_name::<GenericFilePath>().unwrap());
        let (tx, rx) = mpsc::channel();
        let first = Hosted::start(&path, move |e| tx.send(e).unwrap()).unwrap();
        let old = connect().unwrap();
        let Event::Connected { conn: old_id, .. } = rx.recv().unwrap() else {
            panic!("connected first");
        };
        first.stop();
        // the stop ends the old connection: its reader sees the end
        assert!(matches!(
            rx.recv_timeout(std::time::Duration::from_secs(5)),
            Ok(Event::Gone(c)) if c == old_id
        ));
        let mut buf = [0u8; 8];
        assert_eq!((&old).read(&mut buf).unwrap_or(0), 0);
        let (tx, rx) = mpsc::channel();
        let second = Hosted::start(&path, move |e| tx.send(e).unwrap()).unwrap();
        let _new = connect().unwrap();
        let Event::Connected { conn: new_id, .. } = rx.recv().unwrap() else {
            panic!("connected");
        };
        assert_ne!(new_id, old_id);
        second.stop();
    }

    /// Only a manifest this user owns, private, and no link, is trusted; the manifest wubble
    /// writes is one.
    #[cfg(unix)]
    #[test]
    fn only_a_private_manifest_of_this_user_is_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wubble/mcp/m.json");
        let m = Manifest {
            pid: 1,
            profile: "/p".into(),
            default_profile: true,
            socket: "/p/mcp.sock".into(),
            command: vec!["/w".into(), "mcp".into()],
            version: "0".into(),
        };
        write_manifest(&path, &m).unwrap();
        let mcp_dir = path.parent().unwrap();
        assert!(private_to_me(mcp_dir, Kind::Dir));
        assert!(private_to_me(&path, Kind::File));
        assert!(!private_to_me(&path, Kind::Socket));
        let link = mcp_dir.join("l.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(!private_to_me(&link, Kind::File), "a link is not trusted");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!private_to_me(&path, Kind::File), "readable by others");
        std::fs::set_permissions(mcp_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!private_to_me(mcp_dir, Kind::Dir));
        // nothing listens: dead, the one rule `sweep` and the relay share
        assert_eq!(
            crate::instance::probe(&dir.path().join("none.sock")),
            crate::instance::Probe::Dead
        );
    }
}
