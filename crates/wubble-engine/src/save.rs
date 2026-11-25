//! Where a media download's bytes go: a writer thread that puts each track's segments into its
//! part file in order (decrypting HLS AES-128 segments with their key), cuts a file back to a
//! segment boundary for a resume, and joins the finished tracks into the saved file. Nothing
//! here touches the UI thread: commands go in over a channel, [`WriteEvent`]s come back through
//! a queue and a wake-up callback.
//!
//! Joining: one track is renamed into place (HLS MPEG-TS stays `.ts`, fragmented MP4 init and
//! segments are already one playable file). Separate video and audio need a muxer; no small
//! pure-Rust remuxer exists (`mp4` writes from samples, `mp4-atom` is a box codec), so an
//! `ffmpeg` copies both streams into one file without re-encoding: the one bundled beside the
//! executable (`wubble-ffmpeg`, a build of only the demuxers, muxers and parsers a stream copy
//! needs: `scripts/release/build-ffmpeg.sh`), else one on the `PATH` (or a usual install
//! folder). HLS MPEG-TS is remuxed to MP4 the same way. A track of several DASH periods (each
//! with its own init segment: one fragmented MP4 holds one) is first cut at the periods and put
//! back together by ffmpeg's concat demuxer, still without re-encoding. When joining cannot be
//! done the tracks are kept as they came (two files, or the `.ts`) and the event says why.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use wubble_core::media::{Container, hls, subtitle};

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
    /// Where each part goes when the tracks cannot be joined (two files; with `remux`, the one
    /// part as it came).
    pub fallback: Vec<PathBuf>,
    /// One MPEG-TS part (HLS) to remux into `output` (an MP4), not rename.
    pub remux: bool,
    /// Per part, the segments where a later DASH period begins (`Plan::periods`): the part is
    /// cut there and concatenated before anything else.
    pub periods: Vec<Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteEvent {
    /// The saved files (two when the tracks could not be joined). `note`: what to tell the
    /// user when the files are not what was asked for (the reason, and what was kept).
    Joined {
        job: JobId,
        result: Result<Vec<PathBuf>, String>,
        note: Option<String>,
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
                // The byte offsets of the period starts: where the segment before each ended.
                let cuts: Vec<Vec<u64>> = join
                    .periods
                    .iter()
                    .enumerate()
                    .map(|(track, starts)| {
                        let ends = self
                            .tracks
                            .get(&(join.job, track))
                            .map_or(&[][..], |t| t.ends.as_slice());
                        starts
                            .iter()
                            .filter_map(|&s| ends.get(s.checked_sub(1)?).copied())
                            .collect()
                    })
                    .collect();
                self.tracks.retain(|(j, _), _| *j != join.job);
                let (result, note) = match run_join(&join, &cuts, find_ffmpeg()) {
                    Ok((files, note)) => (Ok(files), note),
                    Err(e) => (Err(e), None),
                };
                Some(WriteEvent::Joined {
                    job: join.job,
                    result,
                    note,
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

/// The bundled `ffmpeg`'s file name, beside the executable (Contents/MacOS in the Mac app).
pub const BUNDLED_FFMPEG: &str = if cfg!(windows) {
    "wubble-ffmpeg.exe"
} else {
    "wubble-ffmpeg"
};

/// The `ffmpeg` to use. `WUBBLE_FFMPEG` first when set (a Mac app's `PATH` is short; empty says
/// to use none: a test of the fallback), then the one bundled beside the executable, then one
/// on the `PATH` or in a usual install folder.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    let path = std::env::var_os("PATH").unwrap_or_default();
    pick_ffmpeg(
        std::env::var_os("WUBBLE_FFMPEG"),
        exe_dir.as_deref(),
        &std::env::split_paths(&path).collect::<Vec<_>>(),
    )
}

fn pick_ffmpeg(
    chosen: Option<OsString>,
    exe_dir: Option<&Path>,
    path: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(chosen) = chosen {
        return Some(PathBuf::from(chosen)).filter(|p| p.is_file());
    }
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    exe_dir
        .map(|d| d.join(BUNDLED_FFMPEG))
        .into_iter()
        .chain(
            path.iter()
                .cloned()
                .chain(FFMPEG_DIRS.iter().map(PathBuf::from))
                .map(|dir| dir.join(name)),
        )
        .find(|p| p.is_file())
}

/// The arguments to copy a video and an audio track into `output` without re-encoding.
pub fn ffmpeg_args(
    video: &Path,
    audio: &Path,
    output: &Path,
    container: Container,
) -> Vec<OsString> {
    let mut args = ffmpeg_base();
    args.push("-i".into());
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

/// The arguments to copy one MPEG-TS stream's pictures and sound into an MP4 (its timed
/// metadata streams, which an MP4 cannot hold, are left out).
pub fn remux_args(input: &Path, output: &Path) -> Vec<OsString> {
    let mut args = ffmpeg_base();
    args.push("-i".into());
    args.push(input.into());
    for a in [
        "-map",
        "0:v?",
        "-map",
        "0:a?",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
    ] {
        args.push(a.into());
    }
    args.push(output.into());
    args
}

/// The arguments to concatenate the files `list` names (ffmpeg's concat demuxer) into `output`
/// as `format`, copying the streams.
pub fn concat_args(list: &Path, output: &Path, format: &str) -> Vec<OsString> {
    let mut args = ffmpeg_base();
    for a in ["-f", "concat", "-safe", "0", "-i"] {
        args.push(a.into());
    }
    args.push(list.into());
    for a in ["-map", "0", "-c", "copy", "-f", format] {
        args.push(a.into());
    }
    args.push(output.into());
    args
}

/// A concat demuxer list of `files` (`file '...'` lines, a quote written `'\''`).
fn concat_list(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|f| {
            let path = f.to_string_lossy().replace('\'', "'\\''");
            format!("file '{path}'\n")
        })
        .collect()
}

/// `part` cut at the byte offsets `cuts` (each period with its own init segment) and the pieces
/// concatenated into `<part>.periods` by ffmpeg; the pieces and the list are removed after.
fn join_periods(
    part: &Path,
    cuts: &[u64],
    ffmpeg: &Path,
    container: Container,
) -> Result<PathBuf, String> {
    let with = |suffix: &str| {
        let mut name = part.file_name().unwrap_or_default().to_os_string();
        name.push(suffix);
        part.with_file_name(name)
    };
    let data = std::fs::read(part).map_err(|e| format!("cannot read {}: {e}", part.display()))?;
    let bounds: Vec<usize> = std::iter::once(0)
        .chain(cuts.iter().map(|&c| (c as usize).min(data.len())))
        .chain(std::iter::once(data.len()))
        .collect();
    let pieces: Vec<PathBuf> = (0..bounds.len() - 1)
        .map(|i| with(&format!(".p{i}")))
        .collect();
    let list = with(".list");
    let output = with(".periods");
    let result = (|| {
        for (piece, w) in pieces.iter().zip(bounds.windows(2)) {
            std::fs::write(piece, &data[w[0]..w[1].max(w[0])]).map_err(|e| e.to_string())?;
        }
        std::fs::write(&list, concat_list(&pieces)).map_err(|e| e.to_string())?;
        let format = if container == Container::Webm {
            "webm"
        } else {
            "mp4"
        };
        run_ffmpeg(ffmpeg, &concat_args(&list, &output, format))
    })();
    for f in pieces.iter().chain([&list]) {
        let _ = std::fs::remove_file(f);
    }
    match result {
        Ok(()) => {
            let _ = std::fs::remove_file(part);
            Ok(output)
        }
        Err(why) => {
            let _ = std::fs::remove_file(&output);
            Err(why)
        }
    }
}

fn ffmpeg_base() -> Vec<OsString> {
    ["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// Runs `ffmpeg` with `args`: `Ok` when it exits 0, else its last line of complaint.
fn run_ffmpeg(ffmpeg: &Path, args: &[OsString]) -> Result<(), String> {
    let out = std::process::Command::new(ffmpeg)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg did not run: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let line = said.lines().rev().find(|l| !l.trim().is_empty());
    let line: String = line.unwrap_or("it failed").chars().take(160).collect();
    Err(line)
}

fn rename(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| format!("cannot save {}: {e}", to.display()))
}

/// The saved files and, when they are not what was asked for, what to tell the user.
type Joined = (Vec<PathBuf>, Option<String>);

fn run_join(join: &Join, cuts: &[Vec<u64>], ffmpeg: Option<PathBuf>) -> Result<Joined, String> {
    // A track of several periods becomes one file first; one that cannot be kept as it came.
    let mut parts = join.parts.clone();
    let mut kept_whole = None;
    for (part, cuts) in parts.iter_mut().zip(cuts) {
        if cuts.is_empty() {
            continue;
        }
        let why = match &ffmpeg {
            None => "no ffmpeg".to_owned(),
            Some(ffmpeg) => match join_periods(part, cuts, ffmpeg, join.container) {
                Ok(joined) => {
                    *part = joined;
                    continue;
                }
                Err(why) => why,
            },
        };
        kept_whole = Some(format!(
            "could not join the stream's periods ({why}): saved as one file, which may stop after the first"
        ));
    }
    let (files, note) = join_parts(join, &parts, ffmpeg)?;
    let note = match (note, kept_whole) {
        (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        (a, b) => a.or(b),
    };
    Ok((files, note))
}

/// The tracks (`parts`, each one file now) joined into the saved file.
fn join_parts(join: &Join, parts: &[PathBuf], ffmpeg: Option<PathBuf>) -> Result<Joined, String> {
    match parts {
        [one] if join.remux => {
            let kept = |why: String| -> Result<Joined, String> {
                let _ = std::fs::remove_file(&join.output);
                let to = join.fallback.first().ok_or("nothing to keep")?;
                rename(one, to)?;
                Ok((
                    vec![to.clone()],
                    why_note(&why, "could not convert to MP4", "saved as it came"),
                ))
            };
            match ffmpeg {
                // No muxer: the stream as it came, as before.
                None => kept(String::new()),
                Some(ffmpeg) => match run_ffmpeg(&ffmpeg, &remux_args(one, &join.output)) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(one);
                        Ok((vec![join.output.clone()], None))
                    }
                    Err(why) => kept(why),
                },
            }
        }
        // WebVTT segments joined as they came: one header, the cues once.
        [one] if join.container == Container::Vtt => {
            let text =
                std::fs::read(one).map_err(|e| format!("cannot read {}: {e}", one.display()))?;
            let merged = subtitle::merge_webvtt(&String::from_utf8_lossy(&text));
            std::fs::write(&join.output, merged)
                .map_err(|e| format!("cannot save {}: {e}", join.output.display()))?;
            let _ = std::fs::remove_file(one);
            Ok((vec![join.output.clone()], None))
        }
        [one] => rename(one, &join.output).map(|()| (vec![join.output.clone()], None)),
        [video, audio] => {
            let why = match ffmpeg {
                None => "no ffmpeg to join them".to_owned(),
                Some(ffmpeg) => {
                    match run_ffmpeg(
                        &ffmpeg,
                        &ffmpeg_args(video, audio, &join.output, join.container),
                    ) {
                        Ok(()) => {
                            let _ = std::fs::remove_file(video);
                            let _ = std::fs::remove_file(audio);
                            return Ok((vec![join.output.clone()], None));
                        }
                        Err(why) => {
                            let _ = std::fs::remove_file(&join.output);
                            format!("joining failed: {why}")
                        }
                    }
                }
            };
            for (part, to) in parts.iter().zip(&join.fallback) {
                rename(part, to)?;
            }
            Ok((
                join.fallback.clone(),
                Some(format!("{why}: video and audio saved as two files")),
            ))
        }
        _ => Err("nothing to join".into()),
    }
}

/// "`what` (`why`): `kept`", or nothing when there is no reason to say (no ffmpeg at all).
fn why_note(why: &str, what: &str, kept: &str) -> Option<String> {
    (!why.is_empty()).then(|| format!("{what} ({why}): {kept}"))
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
                    remux: false,
                    periods: vec![],
                }),
            ],
        );
        assert_eq!(
            joined,
            vec![WriteEvent::Joined {
                job: 1,
                result: Ok(vec![out.clone()]),
                note: None,
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
    fn the_ffmpeg_comes_from_the_setting_then_the_bundle_then_the_path() {
        let d = dir().join("pick");
        let (bin, path) = (d.join("bin"), d.join("path"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        let (bundled, onpath, chosen) = (
            bin.join(BUNDLED_FFMPEG),
            path.join("ffmpeg"),
            d.join("mine"),
        );
        for f in [&bundled, &onpath, &chosen] {
            std::fs::write(f, b"").unwrap();
        }
        let dirs = [path.clone()];
        let pick = |env: Option<&str>| pick_ffmpeg(env.map(OsString::from), Some(&bin), &dirs);
        assert_eq!(pick(Some(chosen.to_str().unwrap())), Some(chosen));
        // Named but missing, or empty: none, whatever else is there.
        assert_eq!(pick(Some("/nowhere/ffmpeg")), None);
        assert_eq!(pick(Some("")), None);
        assert_eq!(pick(None), Some(bundled.clone()));
        std::fs::remove_file(&bundled).unwrap();
        assert_eq!(pick(None), Some(onpath));
        assert!(!pick_ffmpeg(None, Some(&bin), &[]).is_some_and(|p| p.starts_with(&bin)));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn ffmpeg_remuxes_the_pictures_and_sound_only() {
        let args = remux_args(Path::new("/t/s.part0"), Path::new("/t/T.mp4"));
        let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            &args[5..],
            [
                "-i",
                "/t/s.part0",
                "-map",
                "0:v?",
                "-map",
                "0:a?",
                "-c",
                "copy",
                "-movflags",
                "+faststart",
                "/t/T.mp4",
            ]
        );
    }

    /// A stand-in `ffmpeg`: a script that writes its last argument, or says why it cannot.
    #[cfg(unix)]
    fn fake_ffmpeg(d: &Path, ok: bool) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = d.join(if ok { "fake-ok" } else { "fake-bad" });
        let body = if ok {
            "#!/bin/sh\nfor a; do last=$a; done\necho joined > \"$last\"\n"
        } else {
            "#!/bin/sh\necho 'Invalid data found when processing input' >&2\nexit 1\n"
        };
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn a_join_that_fails_keeps_the_tracks_and_says_why() {
        let d = dir().join("join");
        std::fs::create_dir_all(&d).unwrap();
        let join = |remux: bool, parts: &[&str], fallback: &[&str]| {
            let path = |n: &str| d.join(n);
            for p in parts {
                std::fs::write(path(p), b"track").unwrap();
            }
            Join {
                job: 1,
                parts: parts.iter().map(|p| path(p)).collect(),
                output: path(if remux { "o.mp4" } else { "o2.mp4" }),
                container: Container::Mp4,
                fallback: fallback.iter().map(|p| path(p)).collect(),
                remux,
                periods: vec![],
            }
        };
        let (ok, bad) = (fake_ffmpeg(&d, true), fake_ffmpeg(&d, false));
        // A pair joined: one file, no note, the parts gone.
        let pair = join(false, &["v0", "a0"], &["v.mp4", "a.m4a"]);
        let (files, note) = run_join(&pair, &[], Some(ok.clone())).unwrap();
        assert_eq!((files, note), (vec![pair.output.clone()], None));
        assert!(!d.join("v0").exists());
        // A pair that ffmpeg refuses: two files, and the reason.
        let pair = join(false, &["v1", "a1"], &["v1.mp4", "a1.m4a"]);
        let (files, note) = run_join(&pair, &[], Some(bad.clone())).unwrap();
        assert_eq!(files, pair.fallback);
        let note = note.unwrap();
        assert!(note.contains("Invalid data found") && note.ends_with("saved as two files"));
        assert!(!pair.output.exists());
        // No ffmpeg at all.
        let pair = join(false, &["v2", "a2"], &["v2.mp4", "a2.m4a"]);
        let (files, note) = run_join(&pair, &[], None).unwrap();
        assert_eq!(files, pair.fallback);
        assert!(note.unwrap().starts_with("no ffmpeg"));
        // One TS remuxed; refused, it stays a `.ts` with the reason; with no ffmpeg, silently.
        let one = join(true, &["s0"], &["s0.ts"]);
        assert_eq!(
            run_join(&one, &[], Some(ok)).unwrap(),
            (vec![one.output.clone()], None)
        );
        let one = join(true, &["s1"], &["s1.ts"]);
        let (files, note) = run_join(&one, &[], Some(bad)).unwrap();
        assert_eq!(files, one.fallback);
        assert!(
            note.unwrap()
                .starts_with("could not convert to MP4 (Invalid")
        );
        let one = join(true, &["s2"], &["s2.ts"]);
        assert_eq!(
            run_join(&one, &[], None).unwrap(),
            (one.fallback.clone(), None)
        );
        let _ = std::fs::remove_dir_all(d);
    }

    /// A track of two DASH periods: cut where the second's init begins, concatenated by
    /// ffmpeg; when ffmpeg refuses, kept whole with the reason.
    #[cfg(unix)]
    #[test]
    fn periods_are_cut_and_concatenated() {
        let d = dir().join("periods");
        std::fs::create_dir_all(&d).unwrap();
        let (ok, bad) = (fake_ffmpeg(&d, true), fake_ffmpeg(&d, false));
        let one = |name: &str| {
            std::fs::write(d.join(name), b"INIT1seg1INIT2seg2").unwrap();
            Join {
                job: 1,
                parts: vec![d.join(name)],
                output: d.join(format!("{name}.mp4")),
                container: Container::Mp4,
                fallback: vec![],
                remux: false,
                periods: vec![vec![2]],
            }
        };
        let join = one("t0");
        let (files, note) = run_join(&join, &[vec![9]], Some(ok)).unwrap();
        assert_eq!((files, note), (vec![join.output.clone()], None));
        assert_eq!(std::fs::read(&join.output).unwrap(), b"joined\n");
        let left: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("t0") && n != "t0.mp4")
            .collect();
        assert!(left.is_empty(), "{left:?}");
        let join = one("t1");
        let (files, note) = run_join(&join, &[vec![9]], Some(bad)).unwrap();
        assert_eq!(files, vec![join.output.clone()]);
        assert!(
            note.unwrap()
                .starts_with("could not join the stream's periods (Invalid")
        );
        assert_eq!(std::fs::read(&join.output).unwrap(), b"INIT1seg1INIT2seg2");
        let join = one("t2");
        let (_, note) = run_join(&join, &[vec![9]], None).unwrap();
        assert!(note.unwrap().contains("(no ffmpeg)"));
        assert_eq!(
            concat_list(&[PathBuf::from("/t/a.p0"), PathBuf::from("/t/it's.p1")]),
            "file '/t/a.p0'\nfile '/t/it'\\''s.p1'\n"
        );
        let args = concat_args(Path::new("/t/l"), Path::new("/t/o"), "mp4");
        let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            &args[5..],
            [
                "-f", "concat", "-safe", "0", "-i", "/t/l", "-map", "0", "-c", "copy", "-f", "mp4",
                "/t/o"
            ]
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn webvtt_segments_are_saved_as_one_file() {
        let d = dir().join("vtt");
        std::fs::create_dir_all(&d).unwrap();
        let part = d.join("s.part0");
        std::fs::write(
            &part,
            "WEBVTT\n\n00:00.000 --> 00:01.000\nA\n\nWEBVTT\n\n00:01.000 --> 00:02.000\nB\n",
        )
        .unwrap();
        let join = Join {
            job: 1,
            parts: vec![part.clone()],
            output: d.join("T.en.vtt"),
            container: Container::Vtt,
            fallback: vec![],
            remux: false,
            periods: vec![],
        };
        assert_eq!(
            run_join(&join, &[], None).unwrap(),
            (vec![join.output.clone()], None)
        );
        assert_eq!(
            std::fs::read_to_string(&join.output).unwrap(),
            "WEBVTT\n\n00:00.000 --> 00:01.000\nA\n\n00:01.000 --> 00:02.000\nB\n"
        );
        assert!(!part.exists());
        let _ = std::fs::remove_dir_all(d);
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
