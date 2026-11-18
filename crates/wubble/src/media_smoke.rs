//! `--smoke media`: a page served from a loopback port plays a file, and its script fetches an
//! HLS master playlist and a DRM-marked one. The response hook must list the file, the HLS
//! stream with its two variants (one plain, one AES-128 with a fetchable key) and the DRM
//! stream as protected. The smoke then saves the file and both variants, as menu rows would,
//! and checks each saved file's size and SHA-256 against the fixtures (the AES one must equal
//! the plain segments joined). Every fixture the downloads fetch answers 403 without the
//! cookie the page set, so a pass also proves the downloads ran in the tab's session.
//! Bounded by counts of runs of work, never by time.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

/// Runs of work a step may take before the smoke gives up.
pub const MAX_STEP_RUNS: u32 = 4_000;

/// The cookie the page sets and every download must send back.
const COOKIE: &str = "wubble_smoke=media";

/// Segments of the HLS fixture.
const SEGMENTS: usize = 3;

/// The progressive file: over the downloader's size floor.
const CLIP_BYTES: usize = 200_000;

/// The AES-128 key of the encrypted variant.
const KEY: [u8; 16] = *b"wubble-smoke-key";

/// Deterministic bytes: `len` of a pattern seeded by `seed`.
fn pattern(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

pub fn clip() -> Vec<u8> {
    pattern(7, CLIP_BYTES)
}

/// Segment `i` in the clear.
fn segment(i: usize) -> Vec<u8> {
    pattern(100 + i as u8, 20_000 + i * 1000)
}

/// The plain segments joined: what both HLS variants must save as.
pub fn joined_segments() -> Vec<u8> {
    (0..SEGMENTS).flat_map(segment).collect()
}

/// Segment `i` sealed with [`KEY`] and the IV of its sequence number (no `IV` attribute).
fn sealed(i: usize) -> Vec<u8> {
    use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&(i as u64).to_be_bytes());
    cbc::Encryptor::<aes::Aes128>::new(&KEY.into(), &iv.into())
        .encrypt_padded_vec::<Pkcs7>(&segment(i))
}

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>Media Smoke</title>\
    <video src=\"/clip.mp4\" preload=auto muted></video>\
    <script>fetch('/hls/master.m3u8');fetch('/drm.m3u8');</script>";

const MASTER: &str = "#EXTM3U\n\
    #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nplain.m3u8\n\
    #EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\naes.m3u8\n";

const DRM: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n\
    #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://smoke\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n\
    #EXTINF:2,\nd0.ts\n#EXT-X-ENDLIST\n";

fn media_playlist(prefix: &str, key: bool) -> String {
    let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n");
    if key {
        text.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n");
    }
    for i in 0..SEGMENTS {
        text.push_str(&format!("#EXTINF:2,\n{prefix}{i}.ts\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Waiting for the list: the file, the HLS stream's two variants, the DRM stream.
    Detect,
    /// Saving three downloads: waiting for them to end.
    Download,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaSmoke {
    At { step: Step, runs: u32 },
    Done,
}

impl Default for MediaSmoke {
    fn default() -> Self {
        Self::At {
            step: Step::Detect,
            runs: 0,
        }
    }
}

/// What the visible tab's list and the downloads show after a run of work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub file: bool,
    pub hls_variants: usize,
    pub protected: bool,
    /// Downloads that completed.
    pub complete: usize,
    /// The first download that failed, with its reason.
    pub failed: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Save the file and both HLS variants.
    SaveAll,
    /// Check the saved files.
    Check,
    Fail(String),
}

pub fn next(state: MediaSmoke, seen: &Seen) -> (MediaSmoke, Vec<Action>) {
    let MediaSmoke::At { step, runs } = state else {
        return (state, vec![]);
    };
    let to = |step| MediaSmoke::At { step, runs: 0 };
    match step {
        _ if seen.failed.is_some() => (
            MediaSmoke::Done,
            vec![Action::Fail(format!(
                "a download failed: {}",
                seen.failed.clone().unwrap_or_default()
            ))],
        ),
        Step::Detect if seen.file && seen.hls_variants == 2 && seen.protected => {
            (to(Step::Download), vec![Action::SaveAll])
        }
        Step::Download if seen.complete == 3 => (MediaSmoke::Done, vec![Action::Check]),
        _ if runs >= MAX_STEP_RUNS => (
            MediaSmoke::Done,
            vec![Action::Fail(format!(
                "stuck at {step:?} for {MAX_STEP_RUNS} runs ({seen:?})"
            ))],
        ),
        _ => (
            MediaSmoke::At {
                step,
                runs: runs + 1,
            },
            vec![],
        ),
    }
}

/// The saved files' (size, SHA-256) must be the fixtures', in any order.
pub fn check(saved: &[Vec<u8>]) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let digest = |b: &[u8]| (b.len(), Sha256::digest(b).to_vec());
    let mut want = vec![
        digest(&clip()),
        digest(&joined_segments()),
        digest(&joined_segments()),
    ];
    let mut got: Vec<_> = saved.iter().map(|b| digest(b)).collect();
    want.sort();
    got.sort();
    if want == got {
        Ok(())
    } else {
        let sizes: Vec<usize> = got.iter().map(|(n, _)| *n).collect();
        Err(format!(
            "saved sizes {sizes:?}, want {CLIP_BYTES} and twice {} with matching SHA-256",
            joined_segments().len()
        ))
    }
}

/// Serves the fixtures on `127.0.0.1` at a free port, from a thread that ends with the process;
/// the port. Loopback only.
pub fn serve() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::Builder::new()
        .name("media-smoke-http".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    // The page's <video> may drop its connection mid-body.
                    if let Err(e) = answer(stream)
                        && !matches!(
                            e.kind(),
                            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                        )
                    {
                        eprintln!("smoke: media server: {e}");
                    }
                });
            }
        })?;
    Ok(port)
}

/// The response for `path`: status, MIME type, body; `None` for 404.
fn route(path: &str, cookie: bool) -> Option<(u16, &'static str, Vec<u8>)> {
    let body = |mime, bytes: Vec<u8>| Some((200, mime, bytes));
    let hls = "application/vnd.apple.mpegurl";
    let guarded = |mime, bytes: Vec<u8>| {
        if cookie {
            Some((200, mime, bytes))
        } else {
            Some((403, "text/plain", b"no cookie".to_vec()))
        }
    };
    match path {
        "/" => body("text/html; charset=utf-8", PAGE.as_bytes().to_vec()),
        "/clip.mp4" => guarded("video/mp4", clip()),
        "/hls/master.m3u8" => body(hls, MASTER.as_bytes().to_vec()),
        "/hls/plain.m3u8" => guarded(hls, media_playlist("p", false).into_bytes()),
        "/hls/aes.m3u8" => guarded(hls, media_playlist("e", true).into_bytes()),
        "/hls/key.bin" => guarded("application/octet-stream", KEY.to_vec()),
        "/drm.m3u8" => body(hls, DRM.as_bytes().to_vec()),
        p => {
            let name = p.strip_prefix("/hls/")?.strip_suffix(".ts")?;
            let (kind, i) = name.split_at(1);
            let i: usize = i.parse().ok().filter(|&i| i < SEGMENTS)?;
            match kind {
                "p" => guarded("video/mp2t", segment(i)),
                "e" => guarded("video/mp2t", sealed(i)),
                _ => None,
            }
        }
    }
}

/// `bytes=a-b` / `bytes=a-` of a body of `len` bytes, inclusive.
fn parse_range(header: &str, len: usize) -> Option<(usize, usize)> {
    let (a, b) = header.trim().strip_prefix("bytes=")?.split_once('-')?;
    let start: usize = a.trim().parse().ok()?;
    let end = match b.trim() {
        "" => len.checked_sub(1)?,
        b => b.parse::<usize>().ok()?.min(len.checked_sub(1)?),
    };
    (start <= end).then_some((start, end))
}

/// One request, one answer, then the connection closes.
fn answer(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let path = path.split('?').next().unwrap_or("/").to_owned();
    let (mut cookie, mut range) = (false, None);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("cookie:") {
            cookie |= v.split(';').any(|c| c.trim() == COOKIE);
        }
        if let Some(v) = lower.strip_prefix("range:") {
            range = Some(v.trim().to_owned());
        }
    }
    let Some((status, mime, body)) = route(&path, cookie) else {
        return stream.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let set_cookie = if path == "/" {
        format!("Set-Cookie: {COOKIE}; Path=/\r\n")
    } else {
        String::new()
    };
    let ranged = range
        .as_deref()
        .filter(|_| status == 200)
        .and_then(|r| parse_range(r, body.len()));
    let (status, extra, slice) = match ranged {
        Some((a, b)) => (
            206,
            format!("Content-Range: bytes {a}-{b}/{}\r\n", body.len()),
            &body[a..=b],
        ),
        None => (status, String::new(), &body[..]),
    };
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        _ => "Forbidden",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
         Accept-Ranges: bytes\r\nCache-Control: no-store\r\n{set_cookie}{extra}Connection: close\r\n\r\n",
        slice.len()
    )?;
    stream.write_all(slice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::media::hls;

    #[test]
    fn the_fixtures_parse_as_the_downloader_expects() {
        let base = "http://127.0.0.1:1/hls/master.m3u8";
        let variants = hls::variants(MASTER, base).unwrap();
        assert_eq!(variants.len(), 2);
        let drm = hls::variants(DRM, "http://127.0.0.1:1/drm.m3u8");
        assert_eq!(drm, Err(wubble_core::media::Refusal::Protected));
        let plan = hls::parse_media(
            &media_playlist("e", true),
            "http://127.0.0.1:1/hls/aes.m3u8",
        )
        .unwrap();
        let key = plan.segments[1].key.as_ref().unwrap();
        assert_eq!(key.uri, "http://127.0.0.1:1/hls/key.bin");
        assert_eq!(hls::decrypt(&KEY, &key.iv, &sealed(1)).unwrap(), segment(1));
    }

    #[test]
    fn routes_guard_downloads_with_the_cookie() {
        assert_eq!(route("/clip.mp4", false).unwrap().0, 403);
        assert_eq!(route("/clip.mp4", true).unwrap().2.len(), CLIP_BYTES);
        assert_eq!(route("/hls/p2.ts", true).unwrap().2, segment(2));
        assert!(route("/hls/p3.ts", true).is_none());
        assert_eq!(route("/hls/master.m3u8", false).unwrap().0, 200);
        assert_eq!(parse_range("bytes=0-", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=2-99", 10), Some((2, 9)));
        assert_eq!(parse_range("bytes=5-1", 10), None);
    }

    #[test]
    fn the_steps_and_the_check() {
        let (s, a) = next(
            MediaSmoke::default(),
            &Seen {
                file: true,
                hls_variants: 2,
                protected: true,
                ..Seen::default()
            },
        );
        assert_eq!(a, vec![Action::SaveAll]);
        let (s, a) = next(
            s,
            &Seen {
                complete: 2,
                ..Seen::default()
            },
        );
        assert!(a.is_empty());
        let (_, a) = next(
            s,
            &Seen {
                complete: 3,
                ..Seen::default()
            },
        );
        assert_eq!(a, vec![Action::Check]);
        let (_, a) = next(
            s,
            &Seen {
                failed: Some("HTTP 403".into()),
                ..Seen::default()
            },
        );
        assert!(matches!(&a[..], [Action::Fail(r)] if r.contains("403")));
        assert_eq!(
            check(&[joined_segments(), clip(), joined_segments()]),
            Ok(())
        );
        assert!(check(&[clip(), clip(), joined_segments()]).is_err());
    }
}
