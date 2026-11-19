//! The GPU process, watched on the lifecycle tick: CEF reports no GPU-process crash, so the
//! host looks for the process by id (`wubble_sysmem::Meter::gpu_pid`) and reads a changed or
//! missing id through `wubble_core::gpu::watch`. Chromium restarts the process by itself and
//! the views keep painting (checked by `cargo run -p wubble-engine --example gpu_crash_smoke`);
//! the host logs and counts it, and asks every view for a fresh frame so a still page does not
//! wait for its next change. The look runs on a worker thread: walking the system's process
//! list takes longer than a 120 Hz frame, so each tick reads the answer to the last tick's ask.

use std::sync::mpsc::{self, Receiver, Sender};

use wubble_core::gpu::{self, Change};
use wubble_sysmem::Meter;

use super::Host;

pub(super) struct GpuWatch {
    /// `None` if the worker could not start: the GPU process is then not watched.
    looker: Option<Looker>,
    last: Option<u32>,
    seen: bool,
    /// GPU process restarts this run.
    restarts: u32,
}

impl Default for GpuWatch {
    fn default() -> Self {
        Self {
            looker: Looker::spawn(),
            last: None,
            seen: false,
            restarts: 0,
        }
    }
}

/// The worker that looks the GPU process up when asked; it ends when the host drops `ask`.
struct Looker {
    ask: Sender<()>,
    answers: Receiver<Option<u32>>,
}

impl Looker {
    fn spawn() -> Option<Self> {
        let (ask, asks) = mpsc::channel::<()>();
        let (answer, answers) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("wubble-gpu-watch".into())
            .spawn(move || {
                let mut meter = Meter::default();
                let mut known = None;
                for () in asks {
                    // A found process is only checked to still run (one process read); the
                    // whole system's list is walked again only when it is gone.
                    known = known
                        .filter(|&pid| meter.still_running(pid))
                        .or_else(|| meter.gpu_pid());
                    if answer.send(known).is_err() {
                        break;
                    }
                }
            });
        match spawned {
            Ok(_) => Some(Self { ask, answers }),
            Err(e) => {
                eprintln!("wubble: the GPU process is not watched: {e}");
                None
            }
        }
    }
}

impl Host {
    pub(super) fn gpu_check(&mut self) {
        let Some(looker) = &self.gpu.looker else {
            return;
        };
        let looked: Vec<Option<u32>> = looker.answers.try_iter().collect();
        // A worker that died stops being asked.
        if looker.ask.send(()).is_err() {
            self.gpu.looker = None;
        }
        for now in looked {
            self.gpu_looked(now);
        }
    }

    fn gpu_looked(&mut self, now: Option<u32>) {
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
