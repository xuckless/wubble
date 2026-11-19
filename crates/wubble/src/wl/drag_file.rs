//! A picture dragged out of a page, as the file another app receives through
//! `text/uri-list` (GNOME Files, chat apps and other browsers take a dropped
//! `file://` URI as the file). It is written only when a drop target asks for
//! the URI list, on the thread that sends it, into a folder of its own per
//! drag under `<CEF cache>/wubble-drag/`. The whole folder goes when the
//! Wayland worker starts and stops; a new picture drag keeps only the newest
//! few drags' folders, since a target may read a drop after it finished.
//!
//! Written new: Chromium offers a picture on Wayland as
//! `application/octet-stream;name=...` bytes, not as a file
//! (`ui/ozone/platform/wayland/host/wayland_exchange_data_provider.cc`).

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use wubble_core::dnd;

/// The folder of the drags' files inside CEF's cache.
const DIR: &str = "wubble-drag";

/// How many drags' folders a new picture drag leaves.
const KEEP: usize = 8;

/// Where the drags' files go for a run whose CEF cache is `cache_dir`.
pub fn base_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join(DIR)
}

/// The drags' folders under one base folder; the process's only user of it
/// (CEF locks the cache for one browser).
pub struct Folders {
    base: PathBuf,
    count: u64,
}

impl Folders {
    pub fn new(base: PathBuf) -> Self {
        Self { base, count: 0 }
    }

    /// Removes every drag's file (at start, a crashed run's; at exit, this run's).
    pub fn clear(&self) {
        match std::fs::remove_dir_all(&self.base) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                eprintln!("wubble: could not remove {}: {e}", self.base.display());
            }
            _ => {}
        }
    }

    /// A new drag's folder (made when its file is written), after removing all
    /// but the newest older ones.
    pub fn next(&mut self) -> PathBuf {
        let names: Vec<String> = std::fs::read_dir(&self.base)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        for stale in dnd::stale_drag_dirs(&names, KEEP - 1) {
            let _ = std::fs::remove_dir_all(self.base.join(stale));
        }
        self.count += 1;
        self.base.join(self.count.to_string())
    }
}

/// A dragged picture's file, written once, when first asked for.
pub struct LazyFile {
    path: PathBuf,
    bytes: Arc<[u8]>,
    written: OnceLock<bool>,
}

impl LazyFile {
    /// The file `name` in `dir` holding `bytes`; nothing is written yet.
    pub fn new(dir: PathBuf, name: &str, bytes: Arc<[u8]>) -> Self {
        Self {
            path: dir.join(name),
            bytes,
            written: OnceLock::new(),
        }
    }

    /// The `text/uri-list` naming the file.
    pub fn uri_list(&self) -> Vec<u8> {
        (dnd::file_uri(&self.path) + "\r\n").into_bytes()
    }

    /// Writes the file unless it was: whether it is there. A second caller
    /// meanwhile waits for the first.
    pub fn ensure_written(&self) -> bool {
        *self.written.get_or_init(|| {
            self.write()
                .inspect_err(|e| eprintln!("wubble: drag file {}: {e}", self.path.display()))
                .is_ok()
        })
    }

    /// The folder (and the base) private to the user, the file new and private.
    fn write(&self) -> std::io::Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new("."));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.path)?
            .write_all(&self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_picture_file_is_written_once_when_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let mut folders = Folders::new(base_dir(tmp.path()));
        let dir = folders.next();
        let file = LazyFile::new(
            dir.clone(),
            "cat 1.jpg",
            Arc::from(&b"\xff\xd8\xffjpeg"[..]),
        );
        let path = dir.join("cat 1.jpg");
        assert_eq!(
            String::from_utf8(file.uri_list()).unwrap(),
            dnd::file_uri(&path) + "\r\n"
        );
        assert!(!path.exists(), "nothing is written before a target asks");
        assert!(file.ensure_written());
        assert_eq!(std::fs::read(&path).unwrap(), b"\xff\xd8\xffjpeg");
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);
        // Asked again: there, not rewritten (a new write would fail on the existing file).
        assert!(file.ensure_written());
        folders.clear();
        assert!(!base_dir(tmp.path()).exists());
        folders.clear();
    }

    #[test]
    fn a_file_that_cannot_be_written_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"").unwrap();
        let file = LazyFile::new(blocker.join("1"), "a.png", Arc::from(&b"x"[..]));
        assert!(!file.ensure_written());
    }

    #[test]
    fn a_new_drag_keeps_the_newest_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let base = base_dir(tmp.path());
        let mut folders = Folders::new(base.clone());
        let mut dirs = vec![];
        for _ in 0..KEEP + 2 {
            let dir = folders.next();
            std::fs::create_dir_all(&dir).unwrap();
            dirs.push(dir);
        }
        std::fs::create_dir_all(base.join("other")).unwrap();
        let next = folders.next();
        let left = std::fs::read_dir(&base).unwrap().count();
        // KEEP - 1 older drags, the stranger; the new one is not made yet.
        assert_eq!(left, KEEP);
        assert!(!dirs[0].exists() && dirs.last().unwrap().exists());
        assert!(!next.exists());
    }
}
