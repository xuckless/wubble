//! Why a tab must stay awake: one set of [`Reason`]s per tab.
//!
//! Any reason protects a hidden tab from being frozen or discarded (TODO.md "Decided": busy tabs
//! are left alone). The model derives `lifecycle::Activity` from whether the set is empty, so the
//! lifecycle machine and the sleep policy only ever see "busy" or "idle".
//!
//! The reason list is a subset, adapted with no code copied, of Chromium
//! `components/performance_manager/freezing/freezing_policy.cc` `CannotFreezeReason`
//! (BSD-3-Clause; audible, WebRTC, capturing, loading) and Firefox
//! `browser/components/tabbrowser/TabUnloader.sys.mjs` (MPL-2.0; playing media, WebRTC,
//! picture-in-picture are never unloaded), plus what wubble itself knows (downloads, unsaved
//! input, DevTools, the user's "never sleep").

use std::collections::BTreeMap;

/// One reason a tab is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    /// The page is loading.
    Loading,
    /// The tab holds a running download (a discard would end it).
    Download,
    /// A media element plays sound.
    Audible,
    /// The page captures the camera, microphone or screen (a live track).
    Capture,
    /// The page holds a WebRTC connection.
    WebRtc,
    /// A text field holds input the user typed and has not submitted.
    UnsavedInput,
    /// The page asks before it is left (a `beforeunload` handler after a user gesture).
    BeforeUnload,
    /// DevTools is open on the page.
    DevTools,
    /// The page shows picture-in-picture.
    PictureInPicture,
    /// The user asked this tab never to sleep (Task manager).
    NeverSleep,
}

/// Every reason with its wire key (the probes script's) and its label (the UI's), in display
/// order. The one table: [`Reason::key`], [`Reason::label`] and [`Reason::from_key`] read it.
pub const REASONS: [(Reason, &str, &str); 10] = [
    (Reason::Loading, "loading", "Loading"),
    (Reason::Download, "download", "Downloading"),
    (Reason::Audible, "audible", "Playing audio"),
    (
        Reason::Capture,
        "capture",
        "Using camera, microphone or screen",
    ),
    (Reason::WebRtc, "webrtc", "In a call"),
    (Reason::UnsavedInput, "input", "Unsaved input"),
    (Reason::BeforeUnload, "unload", "Asks before leaving"),
    (Reason::DevTools, "devtools", "DevTools open"),
    (Reason::PictureInPicture, "pip", "Picture-in-picture"),
    (Reason::NeverSleep, "never", "Never sleep"),
];

/// The reasons only the renderer-side probes report (`wubble.probe`); the host owns the rest.
pub const PROBED: [Reason; 5] = [
    Reason::Capture,
    Reason::WebRtc,
    Reason::UnsavedInput,
    Reason::BeforeUnload,
    Reason::PictureInPicture,
];

impl Reason {
    fn row(self) -> &'static (Reason, &'static str, &'static str) {
        REASONS
            .iter()
            .find(|(r, _, _)| *r == self)
            .expect("REASONS lists every reason")
    }

    pub fn key(self) -> &'static str {
        self.row().1
    }

    pub fn label(self) -> &'static str {
        self.row().2
    }

    pub fn from_key(key: &str) -> Option<Reason> {
        REASONS
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(r, _, _)| *r)
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// A set of [`Reason`]s. Empty: the tab is idle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Busy(u16);

impl Busy {
    pub const NONE: Busy = Busy(0);

    pub fn of(reasons: &[Reason]) -> Busy {
        reasons.iter().fold(Busy::NONE, |b, &r| b.with(r, true))
    }

    /// The set with `reason` added (`on`) or removed.
    #[must_use]
    pub fn with(self, reason: Reason, on: bool) -> Busy {
        if on {
            Busy(self.0 | reason.bit())
        } else {
            Busy(self.0 & !reason.bit())
        }
    }

    pub fn contains(self, reason: Reason) -> bool {
        self.0 & reason.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub fn union(self, other: Busy) -> Busy {
        Busy(self.0 | other.0)
    }

    /// The reasons in the set, in [`REASONS`] order.
    pub fn iter(self) -> impl Iterator<Item = Reason> {
        REASONS
            .iter()
            .map(|(r, _, _)| *r)
            .filter(move |&r| self.contains(r))
    }

    /// The labels of the reasons in the set, in display order.
    pub fn labels(self) -> Vec<&'static str> {
        self.iter().map(Reason::label).collect()
    }

    /// A probe report: the reasons named by the comma-separated `keys`, kept to [`PROBED`]
    /// (a page cannot claim the host's reasons). Unknown keys are ignored.
    pub fn from_probe(keys: &str) -> Busy {
        let reasons: Vec<Reason> = keys
            .split(',')
            .filter_map(|k| Reason::from_key(k.trim()))
            .filter(|r| PROBED.contains(r))
            .collect();
        Busy::of(&reasons)
    }
}

/// What each frame of one tab's page last reported through the probes. The tab's probed reasons
/// are the union over its frames ([`Frames::fold`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frames(BTreeMap<String, Busy>);

impl Frames {
    /// The frame `id` now reports `busy`. An empty report forgets the frame (it left).
    pub fn report(&mut self, id: &str, busy: Busy) {
        if busy.is_empty() {
            self.0.remove(id);
        } else {
            self.0.insert(id.to_owned(), busy);
        }
    }

    /// The main frame committed a new page, or the renderer is gone: no frame holds anything.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn fold(&self) -> Busy {
        self.0.values().fold(Busy::NONE, |a, &b| a.union(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_operations() {
        let b = Busy::NONE.with(Reason::Audible, true);
        assert!(b.contains(Reason::Audible));
        assert!(!b.contains(Reason::Loading));
        assert!(!b.is_empty());
        let b = b.with(Reason::Loading, true).with(Reason::Audible, false);
        assert_eq!(b, Busy::of(&[Reason::Loading]));
        assert!(b.with(Reason::Loading, false).is_empty());
        assert_eq!(
            Busy::of(&[Reason::NeverSleep, Reason::Loading])
                .iter()
                .collect::<Vec<_>>(),
            [Reason::Loading, Reason::NeverSleep]
        );
        assert_eq!(
            Busy::of(&[Reason::Capture]).union(Busy::of(&[Reason::WebRtc])),
            Busy::of(&[Reason::Capture, Reason::WebRtc])
        );
    }

    #[test]
    fn the_table_covers_every_reason_once() {
        let all = [
            Reason::Loading,
            Reason::Download,
            Reason::Audible,
            Reason::Capture,
            Reason::WebRtc,
            Reason::UnsavedInput,
            Reason::BeforeUnload,
            Reason::DevTools,
            Reason::PictureInPicture,
            Reason::NeverSleep,
        ];
        assert_eq!(all.len(), REASONS.len());
        for r in all {
            assert_eq!(Reason::from_key(r.key()), Some(r));
            assert!(!r.label().is_empty());
            assert_eq!(REASONS.iter().filter(|(x, _, _)| *x == r).count(), 1);
        }
        assert_eq!(Busy::of(&all).labels().len(), all.len());
    }

    #[test]
    fn a_probe_names_only_probed_reasons() {
        assert_eq!(
            Busy::from_probe("capture, webrtc,never,devtools,bogus,"),
            Busy::of(&[Reason::Capture, Reason::WebRtc])
        );
        assert!(Busy::from_probe("").is_empty());
    }

    #[test]
    fn frames_fold_to_their_union() {
        let mut f = Frames::default();
        f.report("main", Busy::of(&[Reason::UnsavedInput]));
        f.report("child", Busy::of(&[Reason::Capture]));
        assert_eq!(f.fold(), Busy::of(&[Reason::UnsavedInput, Reason::Capture]));
        // A frame that clears drops only its own.
        f.report("child", Busy::NONE);
        assert_eq!(f.fold(), Busy::of(&[Reason::UnsavedInput]));
        // Another frame sets it again; a main commit clears all.
        f.report("other", Busy::of(&[Reason::WebRtc]));
        f.clear();
        assert!(f.fold().is_empty());
    }
}
