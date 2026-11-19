//! The GPU process, watched on the lifecycle tick: CEF reports no GPU-process crash, so the
//! host looks for the process by id (`wubble_sysmem::Meter::gpu_pid`) and reads a changed or
//! missing id through `wubble_core::gpu::watch`. Chromium restarts the process by itself and
//! the views keep painting (checked by `cargo run -p wubble-engine --example gpu_crash_smoke`);
//! the host logs and counts it, and asks every view for a fresh frame so a still page does not
//! wait for its next change.

use wubble_core::gpu::{self, Change};
use wubble_sysmem::Meter;

use super::Host;

#[derive(Default)]
pub(super) struct GpuWatch {
    meter: Meter,
    last: Option<u32>,
    seen: bool,
    /// GPU process restarts this run.
    restarts: u32,
}

impl Host {
    pub(super) fn gpu_check(&mut self) {
        let now = self.gpu.meter.gpu_pid();
        let change = gpu::watch(self.gpu.last, now, self.gpu.seen);
        self.gpu.seen |= now.is_some();
        self.gpu.last = now;
        match change {
            Change::Same => {}
            Change::Gone { was } => eprintln!("wubble: GPU process {was} gone"),
            Change::Restarted { was, now } => {
                self.gpu.restarts += 1;
                eprintln!(
                    "wubble: GPU process restarted ({} -> {now}), {} this run",
                    was.map_or_else(|| "gone".to_string(), |p| p.to_string()),
                    self.gpu.restarts
                );
                for id in self.views.values().copied().collect::<Vec<_>>() {
                    if let Some(view) = self.engine.view(id) {
                        view.repaint();
                    }
                }
            }
        }
    }
}
