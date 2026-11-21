//! The Task manager page (wubble://tasks): `wubble_core::tasks::rows` over the model, each
//! tab's renderer pid (from its main frame's messages) and the process meter (`wubble-sysmem`), drawn as
//! `TaskItem`s. The meter samples once a second, and only while the page is on screen.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use slint::VecModel;
use wubble_core::busy::Reason;
use wubble_core::download::format_bytes;
use wubble_core::model::Command;
use wubble_core::tab::{Liveness, TabId};
use wubble_core::tasks::{self, Kind, TaskRow};
use wubble_engine::page::FrameInfo;
use wubble_sysmem::Meter;
use wubble_ui::{TaskAction, TaskItem};

use super::Host;
use crate::with_host;

/// How often the page samples while it is shown.
const EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(super) struct TasksPage {
    /// Alive only while the page is shown: its `System` holds the previous CPU sample.
    meter: Option<Meter>,
    timer: slint::Timer,
    rows: Rc<VecModel<TaskItem>>,
    /// Each live tab's renderer pid, as its main frame's messages carry it (welding change 34;
    /// every frame sends the probes' hello at document start).
    pids: BTreeMap<TabId, u32>,
}

impl TasksPage {
    /// A message from one of `tab`'s frames.
    pub(super) fn frame_seen(&mut self, tab: TabId, frame: &FrameInfo) {
        if frame.is_main && frame.pid != 0 {
            self.pids.insert(tab, frame.pid);
        }
    }

    /// The live tabs' renderer pids (several tabs may share one).
    pub(super) fn renderer_pids(&self) -> std::collections::BTreeSet<u32> {
        self.pids.values().copied().collect()
    }

    /// `tab`'s renderer is gone: its pid is stale.
    pub(super) fn forget(&mut self, tab: TabId) {
        self.pids.remove(&tab);
    }
}

/// One row as the page draws it. A renderer several tabs share shows its memory on each,
/// marked shared.
fn item(row: &TaskRow, own_pid: u32) -> TaskItem {
    let tab = match row.kind {
        Kind::Tab(t) => i32::try_from(t.0).unwrap_or(-1),
        Kind::Process(_) => -1,
    };
    let idle = row.busy.is_empty();
    let memory = row.sample.map_or(String::new(), |s| {
        let bytes = format_bytes(s.rss_bytes);
        if row.shared_with > 1 {
            format!("{bytes} (shared by {})", row.shared_with)
        } else {
            bytes
        }
    });
    TaskItem {
        tab,
        pid: row.pid.and_then(|p| i32::try_from(p).ok()).unwrap_or(0),
        title: row.title.as_str().into(),
        state: row.state.map_or("", tasks::state_label).into(),
        memory: memory.into(),
        cpu: row
            .sample
            .map_or(String::new(), |s| format!("{:.1} %", s.cpu_percent))
            .into(),
        busy: row.note().into(),
        never_sleep: row.never_sleep,
        can_sleep: matches!(row.kind, Kind::Tab(_)) && !row.internal,
        can_discard: !row.internal
            && idle
            && matches!(row.state, Some(Liveness::Background | Liveness::Frozen)),
        can_end: row.pid.is_some_and(|p| p != own_pid),
    }
}

impl Host {
    /// The page came on screen or left it: the meter and its timer run only meanwhile.
    pub(super) fn tasks_shown(&mut self, shown: bool) {
        if !shown {
            self.tasks.timer.stop();
            self.tasks.meter = None;
            return;
        }
        if let Some(window) = self.w.window.upgrade() {
            window.set_tasks(self.tasks.rows.clone().into());
        }
        self.tasks.meter = Some(Meter::default());
        self.sample_tasks();
        self.tasks
            .timer
            .start(slint::TimerMode::Repeated, EVERY, || {
                with_host(Host::sample_tasks);
            });
    }

    /// Samples the processes and redraws the page.
    pub fn sample_tasks(&mut self) {
        let Some(meter) = self.tasks.meter.as_mut() else {
            return;
        };
        let procs = meter.sample();
        let samples: BTreeMap<u32, tasks::Sample> =
            procs.iter().map(|p| (p.pid, p.sample)).collect();
        let processes: Vec<(u32, String)> =
            procs.iter().map(|p| (p.pid, p.label.clone())).collect();
        // a renderer reports the id it has in its sandbox's PID namespace: the system's id is
        // what the meter knows
        let all: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        let pids: BTreeMap<TabId, u32> = self
            .tasks
            .pids
            .iter()
            .filter_map(|(tab, reported)| {
                let pid = wubble_sysmem::host_pid(*reported, &all, wubble_sysmem::namespace_pid)?;
                Some((*tab, pid))
            })
            .collect();
        let t = tasks::rows(&self.model, &pids, &processes, &samples);
        let own = std::process::id();
        wubble_ui::sync(
            &self.tasks.rows,
            t.rows.iter().map(|r| item(r, own)).collect(),
        );
        if let Some(window) = self.w.window.upgrade() {
            window.set_tasks_total(
                format!(
                    "{} in {} processes, {:.1} % CPU",
                    format_bytes(t.total_rss_bytes),
                    procs.len(),
                    t.total_cpu_percent
                )
                .into(),
            );
        }
    }

    /// A row's button.
    pub fn task_action(&mut self, tab: i32, pid: i32, action: TaskAction) {
        let tab = u64::try_from(tab).ok().map(TabId);
        match (action, tab) {
            (TaskAction::End, _) => {
                let ended = u32::try_from(pid)
                    .ok()
                    .zip(self.tasks.meter.as_ref())
                    .is_some_and(|(pid, meter)| meter.end(pid));
                if !ended {
                    eprintln!("wubble: process {pid} is not one of wubble's; not ended");
                }
            }
            (TaskAction::Discard, Some(tab)) => {
                let internal = self
                    .model
                    .tabs
                    .get(&tab)
                    .is_some_and(|t| wubble_core::pages::InternalPage::from_url(&t.url).is_some());
                if !internal {
                    self.dispatch(Command::DiscardNow(tab));
                }
            }
            (TaskAction::NeverSleep, Some(tab)) => {
                let internal = self
                    .model
                    .tabs
                    .get(&tab)
                    .is_some_and(|t| wubble_core::pages::InternalPage::from_url(&t.url).is_some());
                if !internal {
                    let on = self
                        .model
                        .tabs
                        .get(&tab)
                        .is_some_and(|t| !t.busy.contains(Reason::NeverSleep));
                    self.set_busy(tab, Reason::NeverSleep, on);
                }
            }
            (_, None) => {}
        }
        self.sample_tasks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::tasks::Sample;

    fn row(kind: Kind, state: Option<Liveness>, pid: Option<u32>) -> TaskRow {
        TaskRow {
            kind,
            title: "t".into(),
            state,
            pid,
            sample: Some(Sample {
                rss_bytes: 12_345_678,
                cpu_percent: 2.25,
            }),
            busy: vec![],
            shared_with: 1,
            never_sleep: false,
            internal: false,
            space: None,
        }
    }

    #[test]
    fn rows_offer_only_the_actions_that_apply() {
        let hidden = item(
            &row(Kind::Tab(TabId(3)), Some(Liveness::Frozen), Some(9)),
            1,
        );
        assert_eq!((hidden.tab, hidden.pid), (3, 9));
        assert_eq!(hidden.state, "Frozen");
        assert_eq!(
            (hidden.memory.as_str(), hidden.cpu.as_str()),
            ("12.3 MB", "2.2 %")
        );
        assert!(hidden.can_discard && hidden.can_sleep && hidden.can_end);
        let internal = item(
            &TaskRow {
                internal: true,
                ..row(Kind::Tab(TabId(5)), Some(Liveness::Background), None)
            },
            1,
        );
        assert!(!internal.can_discard && !internal.can_sleep && !internal.can_end);
        let busy = TaskRow {
            busy: vec!["Playing audio"],
            shared_with: 2,
            ..row(Kind::Tab(TabId(3)), Some(Liveness::Background), Some(9))
        };
        let busy = item(&busy, 1);
        assert!(!busy.can_discard && busy.can_sleep);
        assert_eq!(busy.busy, "Playing audio");
        assert_eq!(busy.memory, "12.3 MB (shared by 2)");
        let shown = item(
            &row(Kind::Tab(TabId(3)), Some(Liveness::Active), Some(9)),
            1,
        );
        assert!(!shown.can_discard && shown.can_sleep);
        let browser = item(&row(Kind::Process("Browser".into()), None, Some(1)), 1);
        assert_eq!((browser.tab, browser.state.as_str()), (-1, ""));
        assert!(!browser.can_end && !browser.can_discard && !browser.can_sleep);
        let gone = item(
            &TaskRow {
                sample: None,
                ..row(Kind::Tab(TabId(4)), Some(Liveness::Discarded), None)
            },
            1,
        );
        assert!(!gone.can_end && gone.memory.is_empty());
    }
}
