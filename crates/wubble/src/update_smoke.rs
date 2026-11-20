//! `--smoke update`, windowless: a fake release of the next version, signed with a key pair
//! made for this run, served from loopback; the updater checks, downloads, verifies and stages
//! it into a temporary install root, and the staged bundle is the version served. Then the same
//! release with a tampered archive is refused before anything is unpacked. Nothing leaves the
//! machine, nothing touches the real install or `~/.local`: the key, the install and the server
//! are this smoke's own, handed to [`Updater::with`] in this process (the browser's updater
//! takes none of them from outside).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use wubble_core::update::install::{Layout, Os};
use wubble_core::update::manifest::{RELEASE_FILE, ReleaseInfo};
use wubble_core::update::{self, Channel, Context, Event, State, Version};

use crate::updater::Updater;
use crate::updater::install::{self, Install};
use crate::updater::net::Net;

/// A `.tar.xz` of `dir`'s entry `top` (tar, then lzma-rs's encoder).
pub fn pack(dir: &Path, top: &str, to: &Path) -> Result<(), String> {
    let mut tar_bytes = vec![];
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        builder
            .append_dir_all(top, dir.join(top))
            .and_then(|()| builder.finish())
            .map_err(|e| e.to_string())?;
    }
    let mut xz = vec![];
    lzma_rs::xz_compress(&mut &tar_bytes[..], &mut xz).map_err(|e| e.to_string())?;
    std::fs::write(to, xz).map_err(|e| e.to_string())
}

/// Serves `files` by path on `listener` (loopback), one request per connection, from threads
/// that end with the process.
fn serve(listener: TcpListener, files: HashMap<String, Vec<u8>>) -> std::io::Result<()> {
    let files = Arc::new(files);
    std::thread::Builder::new()
        .name("update-smoke-http".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let files = files.clone();
                std::thread::spawn(move || {
                    let _ = answer(stream, &files);
                });
            }
        })?;
    Ok(())
}

fn answer(mut stream: TcpStream, files: &HashMap<String, Vec<u8>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
    }
    match files.get(&path) {
        Some(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)
        }
        None => stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}

/// A bundle of `version` for `platform` at `at`, in the OS's shape.
fn fake_bundle(at: &Path, version: &Version, platform: &str, mac: bool) -> Result<(), String> {
    let (program, release) = if mac {
        (
            at.join("Contents/MacOS/wubble"),
            at.join("Contents/Resources"),
        )
    } else {
        (at.join("wubble"), at.to_path_buf())
    };
    let info = ReleaseInfo {
        version: version.clone(),
        platform: platform.to_owned(),
        commit: "smoke".into(),
    };
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(program.parent().unwrap_or(at)).map_err(io)?;
    std::fs::create_dir_all(&release).map_err(io)?;
    std::fs::write(&program, format!("#!/bin/sh\necho wubble {version}\n")).map_err(io)?;
    std::fs::write(release.join(RELEASE_FILE), info.to_json()).map_err(io)
}

/// This OS's install, rooted in `root`, running `current`.
fn install_in(root: &Path, current: &Version, platform: &str) -> Result<Arc<dyn Install>, String> {
    if cfg!(target_os = "macos") {
        let app = root.join("Applications").join(update::install::MAC_APP);
        fake_bundle(&app, current, platform, true)?;
        Ok(Arc::new(install::Mac(Layout {
            os: Os::Mac,
            root: root.join("support"),
            bundle: app,
        })))
    } else {
        let bundle = root.join("versions").join(current.to_string());
        fake_bundle(&bundle, current, platform, false)?;
        Ok(Arc::new(install::Linux(Layout {
            os: Os::Linux,
            root: root.to_path_buf(),
            bundle,
        })))
    }
}

/// Runs the updater from "Check now" until it settles.
fn check(updater: &mut Updater, events: &mpsc::Receiver<Event>) -> Result<State, String> {
    updater.event(Event::CheckNow);
    loop {
        match updater.state() {
            State::Ready(_) | State::Failed(_) | State::UpToDate => {
                return Ok(updater.state().clone());
            }
            _ => {}
        }
        let event = events
            .recv_timeout(Duration::from_secs(60))
            .map_err(|_| format!("the updater stalled in {:?}", updater.state()))?;
        updater.event(event);
    }
}

pub fn run() -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("wubble-update-smoke-{}", std::process::id()));
    let result = run_in(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run_in(dir: &Path) -> Result<String, String> {
    let io = |e: std::io::Error| e.to_string();
    let platform = update::PLATFORM.unwrap_or("linux-x86_64");
    let mac = cfg!(target_os = "macos");
    let current = update::current_version();
    let next = Version::new(current.major, current.minor, current.patch + 1);
    let key = wubble_release::Key::ephemeral()?;
    let trusted = update::key::parse(&key.public_base64()).ok_or("the smoke's key is no key")?;

    // the release: an archive of the next version, its signature, the signed manifest
    let release = dir.join("release");
    let top = if mac {
        update::install::MAC_APP.to_owned()
    } else {
        format!("wubble-{next}-{platform}")
    };
    fake_bundle(&release.join("src").join(&top), &next, platform, mac)?;
    let name = wubble_release::archive_name(&next, platform);
    let archive = release.join(&name);
    pack(&release.join("src"), &top, &archive)?;
    let bytes = std::fs::read(&archive).map_err(io)?;
    std::fs::write(
        release.join(format!("{name}.minisig")),
        key.sign(&bytes[..], &name)?,
    )
    .map_err(io)?;
    // the manifest names the server's port
    let listener = TcpListener::bind("127.0.0.1:0").map_err(io)?;
    let port = listener.local_addr().map_err(io)?.port();
    let base = format!("http://127.0.0.1:{port}");
    let manifest = wubble_release::make_manifest(
        &next,
        &format!("{base}/v{next}"),
        "smoke",
        &release,
        "2026-11-19T00:00:00Z",
    )?;
    let text = manifest.to_json().into_bytes();
    let sig = key.sign(&text[..], "preview.json")?;
    // the tampered release: the same signed manifest's entry, pointing at an archive with one
    // byte changed
    let mut tampered = bytes.clone();
    let middle = tampered.len() / 2;
    tampered[middle] ^= 0x01;
    let mut bad = manifest.clone();
    for a in bad.platforms.values_mut() {
        a.url = format!("{base}/tampered/{name}");
    }
    let bad_text = bad.to_json().into_bytes();
    let bad_sig = key.sign(&bad_text[..], "bad.json")?;
    let files = HashMap::from([
        ("/updates/preview.json".to_owned(), text),
        ("/updates/preview.json.minisig".to_owned(), sig.into_bytes()),
        (format!("/v{next}/{name}"), bytes),
        ("/bad/preview.json".to_owned(), bad_text),
        ("/bad/preview.json.minisig".to_owned(), bad_sig.into_bytes()),
        (format!("/tampered/{name}"), tampered),
    ]);
    serve(listener, files).map_err(io)?;

    let ctx = |manifest_url: String| Context {
        current: current.clone(),
        platform,
        channel: Channel::Preview,
        key: trusted.clone(),
        skip: None,
        policy: update::POLICY,
        manifest_url,
    };
    let net = Net {
        https_only: false,
        env_proxy: false,
    };

    // check, download, verify, stage
    let root = dir.join("install");
    let install = install_in(&root, &current, platform)?;
    let (tx, rx) = mpsc::channel();
    let mut updater = Updater::with(
        ctx(Channel::Preview.manifest_url(&format!("{base}/updates/"))),
        install.clone(),
        net,
        move |e| {
            let _ = tx.send(e);
        },
    );
    let state = check(&mut updater, &rx)?;
    if state != State::Ready(next.clone()) {
        return Err(format!("the update ended {state:?}, not ready with {next}"));
    }
    let staged = staged_version(install.as_ref(), mac)?;
    if staged != next {
        return Err(format!("staged {staged}, served {next}"));
    }
    let record = install::read_record(install.layout());
    if record.staged.as_ref() != Some(&next) {
        return Err(format!("the launch record has {:?}", record.staged));
    }

    // the tampered archive is refused and nothing is staged
    let root = dir.join("install-tampered");
    let install = install_in(&root, &current, platform)?;
    let (tx, rx) = mpsc::channel();
    let mut updater = Updater::with(
        ctx(Channel::Preview.manifest_url(&format!("{base}/bad/"))),
        install.clone(),
        net,
        move |e| {
            let _ = tx.send(e);
        },
    );
    let state = check(&mut updater, &rx)?;
    let State::Failed(reason) = &state else {
        return Err(format!("the tampered archive ended {state:?}"));
    };
    if staged_version(install.as_ref(), mac).is_ok() || install.installed().len() != 1 {
        return Err("the tampered archive was staged".into());
    }
    let leftovers = std::fs::read_dir(install.layout().staging())
        .map(|d| d.count())
        .unwrap_or(0);
    if leftovers != 0 {
        return Err("the tampered download was kept".into());
    }
    Ok(format!(
        "smoke: update ok staged={next} tampered refused ({reason})"
    ))
}

/// The version the next launch runs: Linux `current`'s bundle, macOS the staged app.
fn staged_version(install: &dyn Install, mac: bool) -> Result<Version, String> {
    let l = install.layout();
    let bundle: PathBuf = if mac {
        l.staged_app()
    } else {
        std::fs::canonicalize(l.current()).map_err(|e| e.to_string())?
    };
    let text = std::fs::read_to_string(install.release_file(&bundle)).map_err(|e| e.to_string())?;
    ReleaseInfo::parse(&text)
        .map(|i| i.version)
        .map_err(|r| r.to_string())
}
