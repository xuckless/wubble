//! One browser per profile. The first start listens on a local socket in its profile (a Unix
//! domain socket; a named pipe on Windows); a second start on the same profile connects, hands
//! over its URL in one line and exits before it opens the store, the GPU or CEF. The socket
//! doubles as the lock: binding it is atomic, so of two racing starts one binds and the other
//! forwards. Nothing leaves the machine.
//!
//! CEF's own process singleton (`on_already_running_app_relaunch`) answers only inside
//! `cef::initialize`, after a second start would already have read the settings and opened the
//! database, so it cannot serve here. The socket is the `interprocess` crate's
//! (0BSD OR Apache-2.0), used as its `local_socket` example shows.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericFilePath, Listener, ListenerOptions, Stream};

/// The profile's sockets: this one, and the MCP server's (`mcp.rs`), a second socket because
/// this one takes unauthenticated "open this URL" lines from anyone who can reach it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Socket {
    Instance,
    Mcp,
}

impl Socket {
    /// Its file in the profile, and the stem of its hashed name elsewhere.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Socket::Instance => ("instance.sock", "wubble"),
            Socket::Mcp => ("mcp.sock", "wubble-mcp"),
        }
    }
}

/// The instance socket's file in the profile.
#[cfg(all(test, not(windows)))]
const SOCKET_FILE: &str = "instance.sock";
/// The longest message read; anything longer is dropped.
pub const MAX_MESSAGE: usize = 8 * 1024;
/// The longest socket path used as is. `sockaddr_un.sun_path` holds 108 bytes on Linux and 104
/// on macOS, with the trailing NUL; longer profile paths use a hashed name instead.
#[cfg(not(windows))]
const MAX_SOCKET_PATH: usize = 100;

/// What a second start asks of the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// A link from another app: a Little window on it.
    Open(String),
    /// Started without a URL: bring the browser forward.
    Focus,
}

impl Message {
    /// The one line on the wire.
    pub fn encode(&self) -> String {
        match self {
            Message::Open(url) => format!("open {url}\n"),
            Message::Focus => "focus\n".into(),
        }
    }
}

/// The message in `bytes`: one UTF-8 line (`open <url>` or `focus`), the newline optional, at
/// most [`MAX_MESSAGE`] bytes. Anything else is `None`.
pub fn parse_message(bytes: &[u8]) -> Option<Message> {
    if bytes.len() > MAX_MESSAGE {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains(['\n', '\r', '\0']) {
        return None;
    }
    match line.split_once(' ') {
        Some(("open", url)) if !url.trim().is_empty() && url.trim() == url => {
            Some(Message::Open(url.into()))
        }
        None if line == "focus" => Some(Message::Focus),
        _ => None,
    }
}

/// Where the instance socket of `profile` lives ([`path_of`]).
pub fn socket_path(profile: &Path, runtime: &Path) -> PathBuf {
    path_of(Socket::Instance, profile, runtime)
}

/// Where `socket` of `profile` lives: in it when the path is short enough, else a name hashed
/// from it in `runtime` (`$XDG_RUNTIME_DIR`, else the temp directory). On Windows a named pipe
/// ([`pipe_of`]): interprocess takes only `\\.\pipe\` paths there.
#[cfg(windows)]
pub fn path_of(socket: Socket, profile: &Path, _runtime: &Path) -> PathBuf {
    pipe_of(socket, profile)
}

/// The instance's named pipe of `profile`, named by a hash of its path.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn pipe_path(profile: &Path) -> PathBuf {
    pipe_of(Socket::Instance, profile)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn pipe_of(socket: Socket, profile: &Path) -> PathBuf {
    PathBuf::from(format!(
        r"\\.\pipe\{}-{:016x}",
        socket.names().1,
        fnv1a(profile.as_os_str().as_encoded_bytes())
    ))
}

#[cfg(not(windows))]
pub fn path_of(socket: Socket, profile: &Path, runtime: &Path) -> PathBuf {
    let (file, stem) = socket.names();
    let inside = profile.join(file);
    if inside.as_os_str().len() <= MAX_SOCKET_PATH {
        return inside;
    }
    runtime.join(format!(
        "{stem}-{:016x}.sock",
        fnv1a(profile.as_os_str().as_encoded_bytes())
    ))
}

/// FNV-1a, 64 bit: a hash that stays the same across Rust releases (`DefaultHasher` does not).
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The directory for a hashed socket name.
pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// How the start went.
pub enum Claim {
    /// This is the browser: it listens for later starts.
    Primary(Listener),
    /// Another browser runs on this profile and has the message.
    Forwarded,
}

/// Binds the socket at `path`, or hands `message` to the browser already bound to it. A socket
/// file nobody listens on (left by a crash) is removed and bound once more.
pub fn claim(path: &Path, message: &Message) -> io::Result<Claim> {
    for _ in 0..2 {
        match ListenerOptions::new()
            .name(path.to_fs_name::<GenericFilePath>()?)
            .create_sync()
        {
            Ok(listener) => return Ok(Claim::Primary(listener)),
            Err(e) if taken(&e) => {}
            Err(e) => return Err(e),
        }
        match Stream::connect(path.to_fs_name::<GenericFilePath>()?) {
            Ok(mut stream) => {
                stream.write_all(message.encode().as_bytes())?;
                return Ok(Claim::Forwarded);
            }
            // nobody listens: a stale file
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                remove_stale(path)?;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "the instance socket stays taken",
    ))
}

/// Whether binding failed because another start holds the name. A named pipe that exists
/// fails interprocess's first-instance create with `ERROR_ACCESS_DENIED`, which std reads as
/// `PermissionDenied`; a Unix socket file that exists gives `AddrInUse`.
fn taken(e: &io::Error) -> bool {
    let kind = if cfg!(windows) {
        io::ErrorKind::PermissionDenied
    } else {
        io::ErrorKind::AddrInUse
    };
    e.kind() == kind
}

#[cfg(unix)]
pub(crate) fn remove_stale(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// A named pipe leaves no file behind.
#[cfg(not(unix))]
pub(crate) fn remove_stale(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Removes the sockets in `runtime` that dead browsers left behind (a hashed name: a profile
/// path too long for the socket address), instance and MCP alike, and the manifests of
/// browsers that no longer host: a socket nobody listens on refuses a connection. A live one
/// is only connected to and let go (it reads no message from an empty connection).
#[cfg(unix)]
pub fn sweep(runtime: &Path) {
    let Ok(entries) = std::fs::read_dir(runtime) else {
        return;
    };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !(name.starts_with("wubble-") && name.ends_with(".sock")) {
            continue;
        }
        let refused = path
            .as_path()
            .to_fs_name::<GenericFilePath>()
            .map(|n| Stream::connect(n))
            .is_ok_and(|r| {
                r.is_err_and(|e| {
                    matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                    )
                })
            });
        if refused {
            let _ = std::fs::remove_file(&path);
        }
    }
    // the manifests: `mcp::hosting_browsers` removes the stale ones as it reads them
    let _ = crate::mcp::hosting_browsers();
}

/// A named pipe leaves no file behind.
#[cfg(not(unix))]
pub fn sweep(_runtime: &Path) {}

/// Accepts later starts on `listener` until the process ends, handing each one's message to
/// `deliver`. On its own thread: `accept` blocks.
pub fn listen(listener: Listener, deliver: impl Fn(Message) + Send + 'static) {
    std::thread::Builder::new()
        .name("wubble-instance".into())
        .spawn(move || {
            for stream in listener.incoming().filter_map(Result::ok) {
                if let Some(message) = read_message(stream) {
                    deliver(message);
                }
            }
        })
        .map(drop)
        .unwrap_or_else(|e| eprintln!("wubble: no instance listener: {e}"));
}

/// One message from `stream`: at most one line of [`MAX_MESSAGE`] bytes.
fn read_message(stream: Stream) -> Option<Message> {
    let mut line = vec![];
    BufReader::new(stream)
        .take(MAX_MESSAGE as u64 + 1)
        .read_until(b'\n', &mut line)
        .ok()?;
    parse_message(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_parse() {
        let open = Message::Open("https://a.test/?q=1 2".into());
        assert_eq!(parse_message(open.encode().as_bytes()), Some(open));
        assert_eq!(parse_message(b"focus\n"), Some(Message::Focus));
        assert_eq!(parse_message(b"focus"), Some(Message::Focus));
        assert_eq!(
            parse_message(b"open https://a.test/"),
            Some(Message::Open("https://a.test/".into()))
        );
        for junk in [
            &b"open \n"[..],
            b"open https://a.test/\nopen https://b.test/\n",
            b"close https://a.test/\n",
            b"open  https://a.test/",
            b"",
            b"\n",
            b"focus please",
            b"open https://a.test/\r\n",
        ] {
            assert_eq!(parse_message(junk), None, "{junk:?}");
        }
        assert_eq!(parse_message(b"open https://\xff.test/"), None, "not UTF-8");
        let long = format!("open https://a.test/{}", "x".repeat(MAX_MESSAGE));
        assert_eq!(parse_message(long.as_bytes()), None, "oversize");
    }

    #[test]
    fn a_windows_profile_names_a_pipe() {
        let a = pipe_path(Path::new(r"C:\Users\u\AppData\Local\wubble"));
        let s = a.to_str().unwrap();
        assert!(s.starts_with(r"\\.\pipe\wubble-"), "{s}");
        assert_eq!(s.len(), r"\\.\pipe\wubble-".len() + 16);
        assert_ne!(a, pipe_path(Path::new(r"C:\other")));
        let mcp = pipe_of(Socket::Mcp, Path::new(r"C:\Users\u\AppData\Local\wubble"));
        assert!(mcp.to_str().unwrap().starts_with(r"\\.\pipe\wubble-mcp-"));
        #[cfg(windows)]
        assert_eq!(
            socket_path(Path::new(r"C:\p"), &runtime_dir()),
            pipe_path(Path::new(r"C:\p"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn a_long_profile_path_hashes_its_socket_into_the_runtime_dir() {
        let runtime = Path::new("/run/user/1000");
        let short = Path::new("/home/u/.local/share/wubble");
        assert_eq!(socket_path(short, runtime), short.join(SOCKET_FILE));
        let long = PathBuf::from(format!("/home/u/{}", "deep/".repeat(30)));
        let hashed = socket_path(&long, runtime);
        assert_eq!(hashed.parent(), Some(runtime));
        assert!(hashed.as_os_str().len() <= MAX_SOCKET_PATH);
        assert_eq!(hashed, socket_path(&long, runtime), "stable");
        let other = socket_path(&long.join("x"), runtime);
        assert_ne!(hashed, other);
        // the MCP socket beside it, never the same name
        assert_eq!(path_of(Socket::Mcp, short, runtime), short.join("mcp.sock"));
        let mcp = path_of(Socket::Mcp, &long, runtime);
        assert_eq!(mcp.parent(), Some(runtime));
        assert!(mcp.as_os_str().len() <= MAX_SOCKET_PATH);
        assert_ne!(mcp, hashed);
    }

    #[test]
    fn a_second_start_forwards_and_a_stale_socket_is_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path(), &runtime_dir());
        let Claim::Primary(listener) = claim(&path, &Message::Focus).unwrap() else {
            panic!("the first start binds");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        listen(listener, move |m| tx.send(m).unwrap());
        let url = Message::Open("https://a.test/".into());
        assert!(matches!(claim(&path, &url).unwrap(), Claim::Forwarded));
        assert_eq!(rx.recv().unwrap(), url);
        // a crash leaves the file with nobody listening (a named pipe leaves nothing)
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let path = socket_path(dir.path(), &runtime_dir());
            drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
            assert!(path.exists());
            assert!(matches!(
                claim(&path, &Message::Focus).unwrap(),
                Claim::Primary(_)
            ));
        }
    }
}
