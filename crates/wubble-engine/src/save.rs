//! Where a media download's bytes go: a writer thread that puts each track's segments into its
//! part file in order (decrypting HLS AES-128 segments with their key), cuts a file back to a
//! segment boundary for a resume, and joins the finished tracks into the saved file. Nothing
//! here touches the UI thread: commands go in over a channel, [`WriteEvent`]s come back through
//! a queue and a wake-up callback.
//!
//! Joining: one track is renamed into place (HLS MPEG-TS stays `.ts`, fragmented MP4 init and
//! segments are already one playable file). Separate video and audio need a muxer; no small
//! pure-Rust remuxer exists (`mp4` writes from samples, `mp4-atom` is a box codec), so the
//! system `ffmpeg`, found on the `PATH` (or a usual install folder) at run time, copies both
//! streams into one file without re-encoding. Without it the two tracks are kept as two files.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use wubble_core::media::{Container, hls};

/// A job, as the host numbers them (its downloads-list entry id).
pub type JobId = i64;

/// An AES-128 key and IV for one segment.
pub type SegmentKey = (Vec<u8>, [u8; 16]);

#[derive(Debug)]
pub enum WriteCmd {
    /// A segment is being fetched into the track's file at `path`; with a key it is decrypted
    /// once whole.
    Begin {
        job: JobId,
        track: usize,
        index: usize,
        path: PathBuf,
        key: Option<SegmentKey>,
    },
    Chunk {
        job: JobId,
        track: usize,
        index: usize,
        bytes: Vec<u8>,
    },
    /// The segment is whole.
    End {
        job: JobId,
        track: usize,
        index: usize,
    },
    /// Keep the track's first `keep` segments, drop the rest (a resume).
    Rewind {
        job: JobId,
        track: usize,
        keep: usize,
    },
    /// The track is complete: flush and close its file.
    Close {
        job: JobId,
        track: usize,
    },
    Join(Join),
    /// Forget the job and delete these files.
    Discard {
        job: JobId,
        paths: Vec<PathBuf>,
    },
}

/// How a job's tracks become the saved file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub job: JobId,
    /// The part files, video first.
    pub parts: Vec<PathBuf>,
    pub output: PathBuf,
    pub container: Container,
    /// Where each part goes when the tracks cannot be joined (two files).
    pub fallback: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteEvent {
    /// The saved files (two when the tracks could not be joined).
    Joined {
        job: JobId,
        result: Result<Vec<PathBuf>, String>,
    },
    /// Writing failed (a full disk, a segment that would not decrypt).
    Failed { job: JobId, error: String },
}

/// One track's file being written.
#[derive(Default)]
struct TrackFile {
    path: PathBuf,
    file: Option<File>,
    /// The file's length after each written segment.
    ends: Vec<u64>,
    /// Segments not yet written: their key and bytes, and whether they are whole.
    held: BTreeMap<usize, (Option<SegmentKey>, Vec<u8>, bool)>,
}

impl TrackFile {
    fn next(&self) -> usize {
        self.ends.len()
    }

    fn len(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }

    fn file(&mut self) -> std::io::Result<&mut File> {
        if self.file.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&self.path)?;
            file.set_len(self.len())?;
            self.file = Some(file);
        }
        let file = self.file.as_mut().expect("opened above");
        Ok(file)
    }

    fn append(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let file = self.file()?;
        file.seek(SeekFrom::End(0))?;
        file.write_all(bytes)
    }

    /// Writes every held segment that is next in line and whole.
    fn flush(&mut self) -> Result<(), String> {
        loop {
            let next = self.next();
            let Some((_, _, true)) = self.held.get(&next) else {
                break;
            };
            let (key, bytes, _) = self.held.remove(&next).expect("checked above");
            let bytes = match key {
                Some((key, iv)) => hls::decrypt(&key, &iv, &bytes)?,
                None => bytes,
            };
            self.append(&bytes).map_err(|e| e.to_string())?;
            let end = self
                .file()
                .and_then(|f| f.metadata())
                .map_err(|e| e.to_string())?
                .len();
            self.ends.push(end);
        }
        Ok(())
    }

    fn rewind(&mut self, keep: usize) -> std::io::Result<()> {
        self.held.retain(|&i, _| i < keep);
        self.ends.truncate(keep);
        let len = self.len();
        self.file()?.set_len(len)
    }
}

/// The writer thread's state.
#[derive(Default)]
struct Files {
    tracks: HashMap<(JobId, usize), TrackFile>,
}

impl Files {
    fn apply(&mut self, cmd: WriteCmd) -> Option<WriteEvent> {
        let failed = |job, e: String| Some(WriteEvent::Failed { job, error: e });
        match cmd {
            WriteCmd::Begin {
                job,
                track,
                index,
                path,
                key,
            } => {
                let t = self.tracks.entry((job, track)).or_default();
                t.path = path;
                if index >= t.next() {
                    t.held.insert(index, (key, Vec::new(), false));
                }
                None
            }
            WriteCmd::Chunk {
                job,
                track,
                index,
                bytes,
            } => {
                let t = self.tracks.get_mut(&(job, track))?;
                let (key, held, _) = t.held.get(&index)?;
                // The next segment in line streams to the file (its held body stays empty)
                // unless it must be decrypted whole or already began in memory.
                if index == t.next() && key.is_none() && held.is_empty() {
                    return t
                        .append(&bytes)
                        .err()
                        .and_then(|e| failed(job, e.to_string()));
                }
                t.held.get_mut(&index)?.1.extend_from_slice(&bytes);
                None
            }
            WriteCmd::End { job, track, index } => {
                let t = self.tracks.get_mut(&(job, track))?;
                if let Some(entry) = t.held.get_mut(&index) {
                    entry.2 = true;
                }
                t.flush().err().and_then(|e| failed(job, e))
            }
            WriteCmd::Rewind { job, track, keep } => {
                let t = self.tracks.get_mut(&(job, track))?;
                t.rewind(keep)
                    .err()
                    .and_then(|e| failed(job, e.to_string()))
            }
            WriteCmd::Close { job, track } => {
                let t = self.tracks.get_mut(&(job, track))?;
                let result = match t.file() {
                    Ok(f) => f.sync_all().map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                };
                t.file = None;
                result.err().and_then(|e| failed(job, e))
            }
            WriteCmd::Join(join) => {
                self.tracks.retain(|(j, _), _| *j != join.job);
                Some(WriteEvent::Joined {
                    job: join.job,
                    result: run_join(&join),
                })
            }
            WriteCmd::Discard { job, paths } => {
                self.tracks.retain(|(j, _), _| *j != job);
                for p in paths {
                    let _ = std::fs::remove_file(p);
                }
                None
            }
        }
    }
}

/// The writer thread and what it reported.
pub struct Writer {
    commands: Sender<WriteCmd>,
    events: Arc<Mutex<Vec<WriteEvent>>>,
    thread: std::thread::JoinHandle<()>,
}

impl Writer {
    /// Starts the thread; `wake` runs on it after each event is queued (the host schedules
    /// a look at [`Self::take_events`] on its own thread).
    pub fn start(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (commands, rx): (Sender<WriteCmd>, Receiver<WriteCmd>) = mpsc::channel();
        let events = Arc::new(Mutex::new(Vec::new()));
        let queue = events.clone();
        let thread = std::thread::Builder::new()
            .name("wubble-media-writer".into())
            .spawn(move || {
                wubble_sysmem::lower_current_thread();
                let mut files = Files::default();
                for cmd in rx {
                    if let Some(event) = files.apply(cmd) {
                        queue.lock().unwrap_or_else(|e| e.into_inner()).push(event);
                        wake();
                    }
                }
            })?;
        Ok(Self {
            commands,
            events,
            thread,
        })
    }

    pub fn send(&self, cmd: WriteCmd) {
        // The thread ends only in `finish`: a send cannot fail before that.
        let _ = self.commands.send(cmd);
    }

    /// Runs every command sent (a join already asked for finishes) and ends the thread: at
    /// shutdown, so the files are deleted or saved before the process exits.
    pub fn finish(self) {
        drop(self.commands);
        let _ = self.thread.join();
    }

    pub fn take_events(&self) -> Vec<WriteEvent> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Writer")
    }
}

/// Folders searched for `ffmpeg` after the `PATH` (macOS GUI apps get a short `PATH`).
const FFMPEG_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

/// The system `ffmpeg`, if one is installed.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(FFMPEG_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// `ffmpeg`'s arguments to copy a video and an audio track into `output` without re-encoding.
pub fn ffmpeg_args(
    video: &Path,
    audio: &Path,
    output: &Path,
    container: Container,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.push(video.into());
    args.push("-i".into());
    args.push(audio.into());
    for a in ["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"] {
        args.push(a.into());
    }
    if container == Container::Mp4 {
        args.extend(["-movflags", "+faststart"].map(OsString::from));
    }
    args.push(output.into());
    args
}

fn rename(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| format!("cannot save {}: {e}", to.display()))
}

fn run_join(join: &Join) -> Result<Vec<PathBuf>, String> {
    match join.parts.as_slice() {
        [one] => rename(one, &join.output).map(|()| vec![join.output.clone()]),
        [video, audio] => {
            if let Some(ffmpeg) = find_ffmpeg() {
                let status = std::process::Command::new(ffmpeg)
                    .args(ffmpeg_args(video, audio, &join.output, join.container))
                    .stdin(std::process::Stdio::null())
                    .status()
                    .map_err(|e| format!("ffmpeg did not run: {e}"))?;
                if status.success() {
                    let _ = std::fs::remove_file(video);
                    let _ = std::fs::remove_file(audio);
                    return Ok(vec![join.output.clone()]);
                }
                let _ = std::fs::remove_file(&join.output);
            }
            // No ffmpeg (or it failed): the two tracks as two files.
            for (part, to) in join.parts.iter().zip(&join.fallback) {
                rename(part, to)?;
            }
            Ok(join.fallback.clone())
        }
        _ => Err("nothing to join".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wubble-save-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(files: &mut Files, cmds: Vec<WriteCmd>) -> Vec<WriteEvent> {
        cmds.into_iter().filter_map(|c| files.apply(c)).collect()
    }

    #[test]
    fn segments_land_in_order_and_rewind_to_a_boundary() {
        let d = dir();
        let part = d.join("v.part0");
        let mut f = Files::default();
        let begin = |index| WriteCmd::Begin {
            job: 1,
            track: 0,
            index,
            path: part.clone(),
            key: None,
        };
        let chunk = |index, b: &[u8]| WriteCmd::Chunk {
            job: 1,
            track: 0,
            index,
            bytes: b.to_vec(),
        };
        let end = |index| WriteCmd::End {
            job: 1,
            track: 0,
            index,
        };
        let events = run(
            &mut f,
            vec![
                begin(0),
                begin(1),
                begin(2),
                chunk(1, b"BB"),
                chunk(0, b"A"),
                chunk(2, b"CC"),
                chunk(0, b"a"),
                end(2),
                end(1),
            ],
        );
        assert!(events.is_empty());
        // Segment 0 streams; 1 and 2 wait for it.
        assert_eq!(std::fs::read(&part).unwrap(), b"Aa");
        run(&mut f, vec![end(0)]);
        assert_eq!(std::fs::read(&part).unwrap(), b"AaBBCC");
        // A resume keeps two segments and refetches the third.
        run(
            &mut f,
            vec![
                WriteCmd::Rewind {
                    job: 1,
                    track: 0,
                    keep: 2,
                },
                begin(2),
                chunk(2, b"cc"),
                end(2),
            ],
        );
        assert_eq!(std::fs::read(&part).unwrap(), b"AaBBcc");
        let out = d.join("v.mp4");
        let joined = run(
            &mut f,
            vec![
                WriteCmd::Close { job: 1, track: 0 },
                WriteCmd::Join(Join {
                    job: 1,
                    parts: vec![part.clone()],
                    output: out.clone(),
                    container: Container::Mp4,
                    fallback: vec![],
                }),
            ],
        );
        assert_eq!(
            joined,
            vec![WriteEvent::Joined {
                job: 1,
                result: Ok(vec![out.clone()])
            }]
        );
        assert_eq!(std::fs::read(&out).unwrap(), b"AaBBcc");
        assert!(!part.exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn keyed_segments_are_decrypted_whole() {
        use aes::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
        let d = dir();
        let part = d.join("k.part0");
        let (key, iv) = ([3u8; 16], [5u8; 16]);
        let sealed = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_vec::<Pkcs7>(b"plain bytes");
        let mut f = Files::default();
        let (a, b) = sealed.split_at(7);
        let events = run(
            &mut f,
            vec![
                WriteCmd::Begin {
                    job: 2,
                    track: 0,
                    index: 0,
                    path: part.clone(),
                    key: Some((key.to_vec(), iv)),
                },
                WriteCmd::Chunk {
                    job: 2,
                    track: 0,
                    index: 0,
                    bytes: a.to_vec(),
                },
                WriteCmd::Chunk {
                    job: 2,
                    track: 0,
                    index: 0,
                    bytes: b.to_vec(),
                },
                WriteCmd::End {
                    job: 2,
                    track: 0,
                    index: 0,
                },
            ],
        );
        assert!(events.is_empty());
        assert_eq!(std::fs::read(&part).unwrap(), b"plain bytes");
        // A wrong key fails the job instead of writing garbage.
        let bad = run(
            &mut f,
            vec![
                WriteCmd::Begin {
                    job: 2,
                    track: 0,
                    index: 1,
                    path: part.clone(),
                    key: Some((vec![9; 16], iv)),
                },
                WriteCmd::Chunk {
                    job: 2,
                    track: 0,
                    index: 1,
                    bytes: sealed.clone(),
                },
                WriteCmd::End {
                    job: 2,
                    track: 0,
                    index: 1,
                },
            ],
        );
        assert!(matches!(
            bad.as_slice(),
            [WriteEvent::Failed { job: 2, .. }]
        ));
        run(
            &mut f,
            vec![WriteCmd::Discard {
                job: 2,
                paths: vec![part.clone()],
            }],
        );
        assert!(!part.exists());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn ffmpeg_copies_both_streams() {
        let args = ffmpeg_args(
            Path::new("/t/v.part0"),
            Path::new("/t/a.part1"),
            Path::new("/t/T.mp4"),
            Container::Mp4,
        );
        let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-i",
                "/t/v.part0",
                "-i",
                "/t/a.part1",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c",
                "copy",
                "-movflags",
                "+faststart",
                "/t/T.mp4",
            ]
        );
        let webm = ffmpeg_args(
            Path::new("v"),
            Path::new("a"),
            Path::new("o.webm"),
            Container::Webm,
        );
        assert!(!webm.iter().any(|a| a == "-movflags"));
    }

    #[test]
    fn finish_runs_what_was_sent_before_returning() {
        let d = dir().join("finish");
        std::fs::create_dir_all(&d).unwrap();
        let part = d.join("v.mp4.part0");
        std::fs::write(&part, b"partial").unwrap();
        let writer = Writer::start(|| {}).unwrap();
        writer.send(WriteCmd::Discard {
            job: 7,
            paths: vec![part.clone()],
        });
        writer.finish();
        assert!(!part.exists());
    }
}
