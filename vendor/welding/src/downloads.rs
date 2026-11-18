// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Downloads: CEF offers one, `welding` picks where it lands, the host watches
//! and steers it.
//!
//! CEF asks `OnBeforeDownload` where to put the file and will cancel the
//! download unless the callback is answered *during* the call. There is no time
//! to ask the host and wait for a reply — on Linux and macOS the host thread is
//! CEF's UI thread, so waiting would stop the loop carrying the answer, the
//! same reason cookies are request-then-poll. So the destination is policy,
//! set once on `CefSurfaceConfig::download_dir`, and the host steers afterwards
//! with `cancel_download` / `pause_download` / `resume_download`.
//!
//! wubble change: those three act at once. CEF's `DownloadItemCallback`
//! (`CefDownloadItemCallbackImpl`, libcef `download_manager_delegate_impl.cc`)
//! keeps only a weak download-manager pointer and the id and posts each call
//! to the UI thread, so the latest one from `OnDownloadUpdated` is kept per
//! download and used directly. Upstream recorded the request and applied it
//! on the next update, which a paused download never sends, so a resume after
//! a pause was never delivered.
//!
//! wubble change: with `CefSurfaceConfig::ask_download_path` the host chooses
//! the destination instead. `OnBeforeDownload`'s callback may be answered
//! later (a pending one cancels the download when it is dropped), so it is
//! held in a [`crate::held::Held`] and the host answers with
//! `answer_download_path`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Identifies one download for as long as the producer lives.
pub type DownloadId = u32;

/// wubble change: what steers one running download (CEF's
/// `DownloadItemCallback`; a fake in tests).
pub(crate) trait DownloadControl {
    fn apply(&self, op: DownloadOp);
}

/// What the host asked to do with an in-flight download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadOp {
    Cancel,
    Pause,
    Resume,
}

/// Progress is reported at most this often per download, plus always once more
/// when it finishes. CEF updates far more often than any progress bar needs,
/// and an unthrottled event per update is a log flood waiting to happen.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(crate) struct Downloads {
    inner: Mutex<Inner>,
    /// Destinations the host has been asked for (wubble change).
    #[cfg(feature = "cef-runtime")]
    pub(crate) asked: Mutex<crate::held::Held<crate::handlers::AskedPath>>,
}

#[derive(Default)]
struct Inner {
    dir: Option<PathBuf>,
    /// wubble change: the host picks each destination.
    ask: bool,
    /// wubble change: the latest control per running download.
    controls: HashMap<DownloadId, Box<dyn DownloadControl>>,
    /// wubble change: ops for a started download CEF has not handed a
    /// control for yet, applied in order when the first one arrives.
    queued: HashMap<DownloadId, Vec<DownloadOp>>,
    last_progress: HashMap<DownloadId, Instant>,
    started: HashSet<DownloadId>,
}

impl Downloads {
    pub(crate) fn set_dir(&self, dir: Option<PathBuf>) {
        self.inner.lock().unwrap().dir = dir;
    }

    /// Where a download called `suggested_name` should be written, or `None`
    /// when no download directory is configured and the download is refused.
    ///
    /// The name is attacker-influenced: it arrives from the server's
    /// `Content-Disposition` or the URL. Only its final component is used, so a
    /// suggestion of `../../.bashrc` or `C:\Windows\evil.dll` lands inside the
    /// configured directory as `.bashrc` / `evil.dll` rather than escaping it.
    pub(crate) fn destination_for(&self, suggested_name: &str) -> Option<PathBuf> {
        let dir = self.inner.lock().unwrap().dir.clone()?;
        Some(native_path(dir.join(safe_file_name(suggested_name))))
    }

    /// wubble change: ask the host for each destination instead of using
    /// the download directory.
    pub(crate) fn set_ask(&self, ask: bool) {
        self.inner.lock().unwrap().ask = ask;
    }

    pub(crate) fn asks_host(&self) -> bool {
        self.inner.lock().unwrap().ask
    }

    pub(crate) fn is_enabled(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.dir.is_some() || inner.ask
    }

    /// wubble change: keep the newest control CEF handed over for `id`, and
    /// apply any ops the host sent before the first one arrived.
    pub(crate) fn set_control(&self, id: DownloadId, control: Box<dyn DownloadControl>) {
        let mut inner = self.inner.lock().unwrap();
        for op in inner.queued.remove(&id).unwrap_or_default() {
            control.apply(op);
        }
        inner.controls.insert(id, control);
    }

    /// wubble change: steer a running download now, or as soon as CEF hands
    /// over its control (a host may pause in the same tick it answered the
    /// path). False when the download is not running (not started, finished
    /// or cancelled).
    pub(crate) fn steer(&self, id: DownloadId, op: DownloadOp) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if let Some(control) = inner.controls.get(&id) {
            control.apply(op);
        } else if inner.started.contains(&id) {
            inner.queued.entry(id).or_default().push(op);
        } else {
            return false;
        }
        true
    }

    /// True the first time this id is seen, so `DownloadStarted` is emitted
    /// once even though CEF keeps updating the same item.
    pub(crate) fn mark_started(&self, id: DownloadId) -> bool {
        self.inner.lock().unwrap().started.insert(id)
    }

    /// Whether `DownloadStarted` has already gone out for this id.
    ///
    /// CEF updates an item before it asks where to put it, so without this the
    /// host would hear about progress on a download it has not been told
    /// exists yet.
    pub(crate) fn has_started(&self, id: DownloadId) -> bool {
        self.inner.lock().unwrap().started.contains(&id)
    }

    /// True when this download is due another progress event.
    pub(crate) fn due_for_progress(&self, id: DownloadId, now: Instant) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.last_progress.get(&id) {
            Some(prev) if now.duration_since(*prev) < PROGRESS_INTERVAL => false,
            _ => {
                inner.last_progress.insert(id, now);
                true
            },
        }
    }

    /// Drop everything remembered about a finished or cancelled download.
    pub(crate) fn forget(&self, id: DownloadId) {
        let mut inner = self.inner.lock().unwrap();
        inner.controls.remove(&id);
        inner.queued.remove(&id);
        inner.last_progress.remove(&id);
        inner.started.remove(&id);
    }
}

/// The final path component of `name`, with anything that would leave the
/// download directory removed. Falls back to `download` when nothing usable is
/// left, so a hostile or empty suggestion still produces a writable path.
fn safe_file_name(name: &str) -> String {
    // Split on both separators: a Windows-style name can arrive on Linux and
    // `Path::file_name` would keep the backslashes as part of the name there.
    let last = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('.');
    let candidate = Path::new(last)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if candidate.is_empty() {
        "download".to_owned()
    } else {
        candidate.to_owned()
    }
}

/// Give CEF the platform's own separators.
///
/// CEF on Windows **silently discards** a download whose destination contains
/// forward slashes. `on_before_download` is answered, the transfer runs to
/// completion and reports every byte, and then nothing is written: no file, no
/// `.crdownload` partial, `is_complete` never true and `full_path` empty for
/// the life of the item. There is no error anywhere to notice.
///
/// A host is well within its rights to hand over `C:/downloads`, and one under
/// Git Bash or any POSIX-shaped config will. Normalising here costs nothing and
/// removes a failure mode whose only symptom is a file that never appears.
#[cfg(windows)]
fn native_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if s.contains('/') {
        PathBuf::from(s.replace('/', "\\"))
    } else {
        path
    }
}

#[cfg(not(windows))]
fn native_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggested_name_cannot_escape_the_download_directory() {
        let d = Downloads::default();
        d.set_dir(Some(PathBuf::from("/downloads")));
        for hostile in [
            "../../.bashrc",
            "../.bashrc",
            "/etc/passwd",
            "..\\..\\evil.dll",
            "C:\\Windows\\evil.dll",
        ] {
            let path = d.destination_for(hostile).expect("dir is set");
            assert_eq!(
                path.parent(),
                Some(Path::new("/downloads")),
                "{hostile} escaped to {path:?}"
            );
        }
    }

    #[test]
    fn an_unusable_name_still_produces_a_path() {
        let d = Downloads::default();
        d.set_dir(Some(PathBuf::from("/downloads")));
        for empty in ["", "   ", "..", "/", "\\"] {
            assert_eq!(
                d.destination_for(empty).unwrap(),
                Path::new("/downloads/download"),
                "{empty:?} produced no usable name"
            );
        }
    }

    #[test]
    fn an_ordinary_name_is_left_alone() {
        let d = Downloads::default();
        d.set_dir(Some(PathBuf::from("/downloads")));
        assert_eq!(
            d.destination_for("report.pdf").unwrap(),
            Path::new("/downloads/report.pdf")
        );
    }

    #[test]
    fn no_directory_means_no_destination() {
        let d = Downloads::default();
        assert!(!d.is_enabled());
        assert_eq!(d.destination_for("report.pdf"), None);
    }

    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Default)]
    struct Recorder(Rc<RefCell<Vec<(u8, DownloadOp)>>>);

    struct Control(u8, Recorder);

    impl DownloadControl for Control {
        fn apply(&self, op: DownloadOp) {
            self.1.0.borrow_mut().push((self.0, op));
        }
    }

    #[test]
    fn a_host_op_reaches_the_newest_control_at_once() {
        let d = Downloads::default();
        let seen = Recorder::default();
        assert!(!d.steer(7, DownloadOp::Pause), "nothing to steer before an update");
        d.set_control(7, Box::new(Control(1, seen.clone())));
        d.set_control(7, Box::new(Control(2, seen.clone())));
        assert!(d.steer(7, DownloadOp::Pause));
        // A paused download sends no update, so resume must not wait for one.
        assert!(d.steer(7, DownloadOp::Resume));
        assert_eq!(
            *seen.0.borrow(),
            vec![(2, DownloadOp::Pause), (2, DownloadOp::Resume)],
            "an older control was used, or an op waited for an update"
        );
    }

    #[test]
    fn an_op_before_the_first_control_waits_for_it() {
        let d = Downloads::default();
        let seen = Recorder::default();
        d.mark_started(7);
        // Answered the path and paused in the same tick: CEF has not sent
        // an update with a control yet.
        assert!(d.steer(7, DownloadOp::Pause), "a started download refused an op");
        assert!(d.steer(7, DownloadOp::Resume));
        assert!(seen.0.borrow().is_empty());
        d.set_control(7, Box::new(Control(1, seen.clone())));
        d.set_control(7, Box::new(Control(2, seen.clone())));
        assert_eq!(
            *seen.0.borrow(),
            vec![(1, DownloadOp::Pause), (1, DownloadOp::Resume)],
            "queued ops were lost, reordered or applied twice"
        );
    }

    #[test]
    fn a_finished_download_has_no_control() {
        let d = Downloads::default();
        let seen = Recorder::default();
        d.set_control(7, Box::new(Control(1, seen.clone())));
        d.forget(7);
        assert!(!d.steer(7, DownloadOp::Cancel));
        assert!(seen.0.borrow().is_empty());
    }

    #[test]
    fn asking_the_host_enables_downloads_without_a_directory() {
        let d = Downloads::default();
        assert!(!d.is_enabled());
        d.set_ask(true);
        assert!(d.is_enabled() && d.asks_host());
        // The directory policy is untouched by asking.
        assert_eq!(d.destination_for("report.pdf"), None);
    }

    #[test]
    fn not_asking_keeps_the_directory_policy() {
        let d = Downloads::default();
        d.set_dir(Some(PathBuf::from("/downloads")));
        assert!(!d.asks_host());
        assert_eq!(
            d.destination_for("report.pdf").unwrap(),
            Path::new("/downloads/report.pdf")
        );
    }

    #[test]
    fn started_is_reported_once_per_download() {
        let d = Downloads::default();
        assert!(d.mark_started(1));
        assert!(!d.mark_started(1));
        assert!(d.mark_started(2));
    }

    #[test]
    fn nothing_is_reported_before_the_download_is_accepted() {
        let d = Downloads::default();
        assert!(!d.has_started(1), "CEF updates an item before offering it");
        d.mark_started(1);
        assert!(d.has_started(1));
        d.forget(1);
        assert!(!d.has_started(1));
    }

    #[test]
    fn progress_is_throttled_then_allowed_again() {
        let d = Downloads::default();
        let t0 = Instant::now();
        assert!(d.due_for_progress(1, t0), "first update always reports");
        assert!(!d.due_for_progress(1, t0 + Duration::from_millis(10)));
        assert!(d.due_for_progress(1, t0 + PROGRESS_INTERVAL));
        // Throttling is per download, not global.
        assert!(d.due_for_progress(2, t0 + Duration::from_millis(10)));
    }

    #[test]
    fn forgetting_a_download_clears_its_state() {
        let d = Downloads::default();
        d.set_control(3, Box::new(Control(1, Recorder::default())));
        d.mark_started(3);
        d.due_for_progress(3, Instant::now());
        d.forget(3);
        assert!(!d.steer(3, DownloadOp::Cancel));
        assert!(d.mark_started(3), "state survived forget()");
    }
}
