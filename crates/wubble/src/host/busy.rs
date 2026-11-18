//! Why each tab must stay awake, gathered into the model's one busy set
//! (`wubble_core::busy`), and the memory pressure the lifecycle tick runs under.
//!
//! Sources, one writer each, all through `Command::Busy`: loading and downloads
//! (`Host::report_busy`), sound (the engine's audible probe), DevTools (read on the tick), the
//! renderer-side probes (`wubble-helper/src/probes.js`: capture, WebRTC, unsaved input,
//! beforeunload, picture-in-picture) and the Task manager's "never sleep".

use std::collections::BTreeMap;

use wubble_core::busy::{Busy, Frames, PROBED, Reason};
use wubble_core::model::Command;
use wubble_core::pressure::Gauge;
use wubble_core::sleep::Pressure;
use wubble_core::tab::TabId;
use wubble_engine::page::FrameInfo;
use wubble_sysmem::PressureSource;

use super::Host;

/// The prefix of a probe report among the context script's frame messages.
pub(super) const PROBE_PREFIX: &str = "wubble.probe ";

/// What each live tab's frames last reported, and the renderer each tab's main frame runs in.
#[derive(Default)]
pub(super) struct BusyFrames {
    frames: BTreeMap<TabId, Frames>,
    pids: BTreeMap<TabId, u32>,
}

impl BusyFrames {
    /// Each live tab's renderer pid, as its main frame reported it.
    pub(super) fn pids(&self) -> &BTreeMap<TabId, u32> {
        &self.pids
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
        if frame.is_main && frame.pid != 0 {
            self.busy.pids.insert(tab, frame.pid);
        }
        self.busy
            .frames
            .entry(tab)
            .or_default()
            .report(&frame.id, Busy::from_probe(keys));
        self.sync_probed(tab);
    }

    /// The tab's renderer is gone (crash, discard, close): its frames hold nothing, its pid is
    /// stale, its sound and DevTools are gone with it.
    pub(super) fn busy_view_gone(&mut self, tab: TabId) {
        self.busy.frames.remove(&tab);
        self.busy.pids.remove(&tab);
        self.frozen.remove(&tab);
        self.sync_probed(tab);
        for reason in [Reason::Audible, Reason::DevTools, Reason::Loading] {
            self.set_busy(tab, reason, false);
        }
    }

    /// Brings the model's probed reasons in line with the tab's frames.
    fn sync_probed(&mut self, tab: TabId) {
        let now = self.busy.frames.get(&tab).map_or(Busy::NONE, Frames::fold);
        for reason in PROBED {
            self.set_busy(tab, reason, now.contains(reason));
        }
    }

    /// DevTools open on a page keeps its tab awake (read here, not from CEF's agent callbacks:
    /// our own CDP freeze attaches an agent too).
    pub(super) fn devtools_tick(&mut self) {
        let tabs: Vec<TabId> = self.views.keys().copied().collect();
        for tab in tabs {
            let open = self.tab_view(tab).is_some_and(|v| v.has_devtools());
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
