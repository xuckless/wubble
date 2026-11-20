//! The Task manager page's rows: tabs and wubble's other processes with their memory, CPU,
//! state and busy reasons. Pure: the host hands in which renderer each tab runs in (the pid its
//! frames report) and the process samples; nothing here reads a process.
//!
//! Several tabs can share one renderer (same site, or past the renderer limit). Each such row
//! shows the renderer's numbers marked shared, and the totals count every process once.

use std::collections::{BTreeMap, BTreeSet};

use crate::busy::Reason;
use crate::model::Model;
use crate::pages::InternalPage;
use crate::tab::{Liveness, TabId};

/// One process's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sample {
    /// Resident set size.
    pub rss_bytes: u64,
    /// Since the previous sample, 100 = one core.
    pub cpu_percent: f32,
}

/// What a row stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Tab(TabId),
    /// A process that is not a tab's renderer: the browser, GPU or a utility process.
    Process(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub kind: Kind,
    pub title: String,
    /// A tab's liveness; `None` for a process row.
    pub state: Option<Liveness>,
    pub pid: Option<u32>,
    pub sample: Option<Sample>,
    /// Labels of the tab's busy reasons, in display order.
    pub busy: Vec<&'static str>,
    /// How many tabs run in this row's renderer (1 = its own).
    pub shared_with: usize,
    pub never_sleep: bool,
    /// The Space a tab row's tab is in, when there is more than one (tabs of the same title in
    /// two Spaces are told apart, QA MED-06); `None` for a process row.
    pub space: Option<String>,
}

impl TaskRow {
    /// The row's second line: its Space, then why it stays awake.
    pub fn note(&self) -> String {
        let busy = self.busy.join(", ");
        match &self.space {
            Some(space) if busy.is_empty() => space.clone(),
            Some(space) => format!("{space} \u{b7} {busy}"),
            None => busy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tasks {
    pub rows: Vec<TaskRow>,
    /// Over every listed process, each once.
    pub total_rss_bytes: u64,
    pub total_cpu_percent: f32,
}

pub fn state_label(state: Liveness) -> &'static str {
    match state {
        Liveness::Active => "Active",
        Liveness::Background => "Background",
        Liveness::Frozen => "Frozen",
        Liveness::Discarded => "Discarded",
    }
}

/// The page's rows: the tabs in sidebar order (Space by Space), then `processes` (pid, label).
/// `pids` maps a tab to its renderer; `samples` holds each process's numbers.
pub fn rows(
    model: &Model,
    pids: &BTreeMap<TabId, u32>,
    processes: &[(u32, String)],
    samples: &BTreeMap<u32, Sample>,
) -> Tasks {
    let live_pid = |tab: &TabId| {
        let t = model.tabs.get(tab)?;
        let live =
            t.lifecycle.liveness != Liveness::Discarded && InternalPage::from_url(&t.url).is_none();
        live.then(|| pids.get(tab).copied()).flatten()
    };
    let mut sharing: BTreeMap<u32, usize> = BTreeMap::new();
    for pid in model.tabs.keys().filter_map(live_pid) {
        *sharing.entry(pid).or_default() += 1;
    }
    let mut rows = vec![];
    for space in &model.spaces {
        for id in model.order(space.id) {
            let t = &model.tabs[&id];
            let pid = live_pid(&id);
            rows.push(TaskRow {
                kind: Kind::Tab(id),
                title: if t.title.is_empty() {
                    t.url.clone()
                } else {
                    t.title.clone()
                },
                state: Some(t.lifecycle.liveness),
                pid,
                sample: pid.and_then(|p| samples.get(&p).copied()),
                busy: t.busy.labels(),
                shared_with: pid.map_or(0, |p| sharing[&p]),
                never_sleep: t.busy.contains(Reason::NeverSleep),
                space: (model.spaces.len() > 1).then(|| space.name.clone()),
            });
        }
    }
    for (pid, label) in processes.iter().filter(|(p, _)| !sharing.contains_key(p)) {
        rows.push(TaskRow {
            kind: Kind::Process(label.clone()),
            title: label.clone(),
            state: None,
            pid: Some(*pid),
            sample: samples.get(pid).copied(),
            busy: vec![],
            shared_with: 0,
            never_sleep: false,
            space: None,
        });
    }
    let counted: BTreeSet<u32> = rows.iter().filter_map(|r| r.pid).collect();
    let listed = || counted.iter().filter_map(|p| samples.get(p));
    Tasks {
        total_rss_bytes: listed().map(|s| s.rss_bytes).sum(),
        total_cpu_percent: listed().map(|s| s.cpu_percent).sum(),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, apply};
    use std::time::Instant;

    fn model(n: usize) -> (Model, Vec<TabId>) {
        let mut m = Model::new(Instant::now());
        let mut ids = vec![];
        for i in 0..n {
            let (m2, _) = apply(
                m,
                Command::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            );
            m = m2;
            ids.push(*m.tabs.keys().last().unwrap());
        }
        (m, ids)
    }

    fn sample(mb: u64, cpu: f32) -> Sample {
        Sample {
            rss_bytes: mb << 20,
            cpu_percent: cpu,
        }
    }

    /// With two Spaces each tab row names its Space (two tabs of one title stay apart), before
    /// why it stays awake; with one Space nothing is added.
    #[test]
    fn tab_rows_name_their_space_when_there_are_several() {
        let (m, ids) = model(1);
        let none = rows(&m, &BTreeMap::new(), &[], &BTreeMap::new());
        assert_eq!(none.rows[0].space, None);
        let (m, _) = apply(
            m,
            Command::Busy {
                tab: ids[0],
                reason: Reason::NeverSleep,
                on: true,
            },
        );
        let command = m.new_space();
        let (m, _) = apply(m, command);
        let t = rows(
            &m,
            &BTreeMap::new(),
            &[(9, "Browser".into())],
            &BTreeMap::new(),
        );
        let first = m.spaces[0].name.clone();
        assert_eq!(t.rows[0].space.as_deref(), Some(first.as_str()));
        assert_eq!(
            t.rows[0].note(),
            format!("{first} \u{b7} {}", t.rows[0].busy.join(", "))
        );
        let browser = t.rows.iter().find(|r| r.title == "Browser").unwrap();
        assert_eq!(
            (browser.space.clone(), browser.note()),
            (None, String::new())
        );
    }

    #[test]
    fn rows_join_tabs_pids_and_samples() {
        let (m, ids) = model(3);
        let (m, _) = apply(
            m,
            Command::Busy {
                tab: ids[0],
                reason: Reason::NeverSleep,
                on: true,
            },
        );
        let (m, _) = apply(
            m,
            Command::Busy {
                tab: ids[0],
                reason: Reason::Audible,
                on: true,
            },
        );
        let (m, _) = apply(m, Command::DiscardNow(ids[1]));
        // ids[0] and ids[2] share renderer 10; ids[1] is discarded (its stale pid ignored).
        let pids = BTreeMap::from([(ids[0], 10), (ids[1], 11), (ids[2], 10)]);
        let processes = vec![(1, "Browser".to_string()), (2, "GPU".to_string())];
        let samples = BTreeMap::from([
            (10, sample(100, 5.0)),
            (11, sample(999, 99.0)),
            (1, sample(50, 1.0)),
            (2, sample(30, 2.0)),
        ]);
        let t = rows(&m, &pids, &processes, &samples);
        assert_eq!(t.rows.len(), 5);
        let r0 = &t.rows[0];
        assert_eq!(r0.kind, Kind::Tab(ids[0]));
        assert_eq!(r0.title, "https://0.test/");
        assert_eq!(r0.state, Some(Liveness::Background));
        assert_eq!((r0.pid, r0.shared_with), (Some(10), 2));
        assert_eq!(r0.busy, vec!["Playing audio", "Never sleep"]);
        assert!(r0.never_sleep);
        let r1 = &t.rows[1];
        assert_eq!(
            (r1.state, r1.pid, r1.sample),
            (Some(Liveness::Discarded), None, None)
        );
        let r2 = &t.rows[2];
        assert_eq!(r2.state, Some(Liveness::Active));
        assert_eq!((r2.pid, r2.sample), (Some(10), Some(sample(100, 5.0))));
        assert_eq!(t.rows[3].kind, Kind::Process("Browser".into()));
        // The shared renderer counts once; the discarded tab's old renderer not at all.
        assert_eq!(t.total_rss_bytes, (100 + 50 + 30) << 20);
        assert_eq!(t.total_cpu_percent, 8.0);
    }

    #[test]
    fn labels_cover_every_state() {
        for s in [
            Liveness::Active,
            Liveness::Background,
            Liveness::Frozen,
            Liveness::Discarded,
        ] {
            assert!(!state_label(s).is_empty());
        }
    }
}
