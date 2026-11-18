//! Why each tab must stay awake, gathered into the model's one busy set
//! (`wubble_core::busy`), and the memory pressure the lifecycle tick runs under.
//!
//! Sources, one writer each, all through `Command::Busy`: loading and downloads
//! (`Host::report_busy`), sound (the engine's audible probe), DevTools (read on the tick), the
//! renderer-side probes (`wubble-helper/src/probes.js`: capture, WebRTC, unsaved input,
//! beforeunload, picture-in-picture) and the Task manager's "never sleep".

use std::collections::BTreeMap;

use wubble_core::busy::{Busy, Frames, PROBED, Reason};
use wubble_core::devtools::Dock;
use wubble_core::model::Command;
use wubble_core::pressure::Gauge;
use wubble_core::sleep::Pressure;
use wubble_core::tab::TabId;
use wubble_engine::page::FrameInfo;
use wubble_sysmem::PressureSource;

use super::Host;

/// The prefix of a probe report among the context script's frame messages.
pub(super) const PROBE_PREFIX: &str = "wubble.probe ";

/// What each live tab's frames last reported.
#[derive(Default)]
pub(super) struct BusyFrames(BTreeMap<TabId, Frames>);

impl BusyFrames {
    /// Frame `frame` of `tab` reported `keys`.
    fn report(&mut self, tab: TabId, frame: &str, keys: &str) {
        self.0
            .entry(tab)
            .or_default()
            .report(frame, Busy::from_probe(keys));
    }

    /// `tab`'s main frame started a new document, or its renderer is gone: whatever the old
    /// documents' frames reported goes with them. Their own `pagehide` reports may never arrive
    /// (a frame host swapped on a cross-site navigation drops its last messages), and the new
    /// document's frames report afresh.
    fn forget(&mut self, tab: TabId) {
        self.0.remove(&tab);
    }

    fn probed(&self, tab: TabId) -> Busy {
        self.0.get(&tab).map_or(Busy::NONE, Frames::fold)
    }
}

/// The OS pressure source and the gauge that smooths it.
pub(super) struct PressureGauge {
    source: Box<dyn PressureSource>,
    gauge: Gauge,
}

impl PressureGauge {
    pub(super) fn system() -> Self {
        Self {
            source: wubble_sysmem::system_source(),
            gauge: Gauge::default(),
        }
    }
}

impl Host {
    /// Adds or removes one reason, dispatching only a change.
    pub(super) fn set_busy(&mut self, tab: TabId, reason: Reason, on: bool) {
        let held = self
            .model
            .tabs
            .get(&tab)
            .is_some_and(|t| t.busy.contains(reason));
        if self.model.tabs.contains_key(&tab) && held != on {
            self.dispatch(Command::Busy { tab, reason, on });
        }
    }

    /// A `wubble.probe` report from one of the tab's frames (`keys` after the prefix).
    pub(super) fn probe_report(&mut self, tab: TabId, frame: &FrameInfo, keys: &str) {
        self.busy.report(tab, &frame.id, keys);
        self.sync_probed(tab);
    }

    /// The tab's main frame committed a new document. Its frames' first reports (the empty
    /// hello at document start) may arrive before or after this; any reason needs the user or
    /// a permission prompt first, well after the commit.
    pub(super) fn busy_page_started(&mut self, tab: TabId) {
        self.busy.forget(tab);
        self.sync_probed(tab);
    }

    /// The tab's renderer is gone (crash, discard, close): its frames hold nothing, its pid is
    /// stale, its sound and DevTools are gone with it.
    pub(super) fn busy_view_gone(&mut self, tab: TabId) {
        self.busy_page_started(tab);
        self.tasks.forget(tab);
        for reason in [Reason::Audible, Reason::DevTools, Reason::Loading] {
            self.set_busy(tab, reason, false);
        }
    }

    /// Brings the model's probed reasons in line with the tab's frames.
    fn sync_probed(&mut self, tab: TabId) {
        let now = self.busy.probed(tab);
        for reason in PROBED {
            self.set_busy(tab, reason, now.contains(reason));
        }
    }

    /// DevTools open on a page keeps its tab awake (read here, not from CEF's agent callbacks:
    /// our own CDP freeze attaches an agent too).
    pub(super) fn devtools_tick(&mut self) {
        let tabs: Vec<TabId> = self.views.keys().copied().collect();
        for tab in tabs {
            let docked = self.devtools.dock == Dock::Open { tab };
            let open = docked || self.tab_view(tab).is_some_and(|v| v.has_devtools());
            self.set_busy(tab, Reason::DevTools, open);
        }
    }

    /// The OS's memory pressure through the gauge. Critical also drops every snapshot but those
    /// of the tabs on screen.
    pub(super) fn read_pressure(&mut self) -> Pressure {
        let reading = self.pressure.source.read();
        let (gauge, level) = self.pressure.gauge.step(reading);
        self.pressure.gauge = gauge;
        if level == Pressure::Critical {
            let shown = self.model.visible_tabs();
            self.snapshots.release_except(&shown);
        }
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_document_forgets_what_the_old_ones_reported() {
        let tab = TabId(1);
        let mut busy = BusyFrames::default();
        busy.report(tab, "old", "input");
        busy.report(tab, "old-child", "unload");
        assert_eq!(
            busy.probed(tab),
            Busy::of(&[Reason::UnsavedInput, Reason::BeforeUnload])
        );
        // A cross-site navigation: the old frames' `pagehide` reports never arrive.
        busy.forget(tab);
        assert_eq!(busy.probed(tab), Busy::NONE);
        // The new document's hello and its own reports count from scratch.
        busy.report(tab, "new", "");
        busy.report(tab, "new", "capture");
        assert_eq!(busy.probed(tab), Busy::of(&[Reason::Capture]));
    }
}
