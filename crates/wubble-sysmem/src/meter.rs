//! wubble's own process tree with each process's memory and CPU, through `sysinfo` (MIT, a
//! dependency; nothing copied). Linux reports RSS, which counts shared pages in every process
//! that maps them: the numbers are shown, never asserted.

use std::collections::BTreeMap;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use wubble_core::tasks::Sample;

/// One process of wubble's tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    /// What it is (`label`): Browser, Renderer, GPU, ...
    pub label: String,
    pub sample: Sample,
    /// CPU time used since it started, milliseconds (`sysinfo`'s accumulated CPU time; Linux
    /// counts it in clock ticks, 10 ms).
    pub cpu_ms: u64,
}

/// Samples wubble's process tree. Keeps its `System` between samples: CPU use is measured
/// between two refreshes, so the first sample shows 0 %.
pub struct Meter {
    system: System,
    root: u32,
}

impl Default for Meter {
    fn default() -> Self {
        Self::new(std::process::id())
    }
}

impl Meter {
    /// A meter of `root` and its descendants.
    pub fn new(root: u32) -> Self {
        Self {
            system: System::new(),
            root,
        }
    }

    /// Refreshes and returns the tree, `root` first. Reads every process on the system.
    pub fn sample(&mut self) -> Vec<Proc> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu()
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        self.tree()
    }

    /// Refreshes the CPU time of only the tree [`Meter::sample`] last found and of the root's
    /// threads, root first: a few files, not the whole system, so cheap enough for the UI
    /// thread and (nearly) not counted in the root's own time. A process started since that
    /// sample is missed; memory is that sample's.
    pub fn resample(&mut self) -> Vec<Proc> {
        let mut pids: Vec<Pid> = self.tree().iter().map(|p| Pid::from_u32(p.pid)).collect();
        if let Some(tasks) = self
            .system
            .process(Pid::from_u32(self.root))
            .and_then(|p| p.tasks())
        {
            pids.extend(tasks.iter().copied());
        }
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            false,
            ProcessRefreshKind::nothing().with_cpu(),
        );
        self.tree()
    }

    /// The tree as of the last refresh, `root` first.
    fn tree(&self) -> Vec<Proc> {
        let processes = self.system.processes();
        // Linux lists each thread as a task of its process, with the process as its parent and
        // the process's memory: counted, every thread would add its process's RSS again.
        let table: Vec<(u32, Option<u32>)> = processes
            .iter()
            .filter(|(_, p)| p.thread_kind().is_none())
            .map(|(pid, p)| (pid.as_u32(), p.parent().map(Pid::as_u32)))
            .collect();
        descendants(self.root, &table)
            .into_iter()
            .filter_map(|pid| {
                let p = processes.get(&Pid::from_u32(pid))?;
                let args: Vec<String> = p
                    .cmd()
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                Some(Proc {
                    pid,
                    label: label(&args, pid == self.root),
                    sample: Sample {
                        rss_bytes: p.memory(),
                        cpu_percent: p.cpu_usage(),
                    },
                    cpu_ms: p.accumulated_cpu_time(),
                })
            })
            .collect()
    }

    /// The threads of `root` as of the last refresh: id, name, CPU time in ms, the main thread
    /// first ([`with_main`]). Linux only (sysinfo lists threads there); empty elsewhere.
    pub fn threads(&self) -> Vec<(u32, String, u64)> {
        let Some(root) = self.system.process(Pid::from_u32(self.root)) else {
            return Vec::new();
        };
        let Some(tasks) = root.tasks() else {
            return Vec::new();
        };
        let threads = tasks
            .iter()
            .filter_map(|tid| {
                let t = self.system.process(*tid)?;
                Some((
                    tid.as_u32(),
                    t.name().to_string_lossy().into_owned(),
                    t.accumulated_cpu_time(),
                ))
            })
            .collect();
        with_main(self.root, root.accumulated_cpu_time(), threads)
    }

    /// Whether `pid` still runs (not gone, not a zombie), refreshing that one process only: the
    /// cheap check between full looks ([`Meter::gpu_pid`] reads every process on the system).
    pub fn still_running(&mut self, pid: u32) -> bool {
        let pid = Pid::from_u32(pid);
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        self.system
            .process(pid)
            .is_some_and(|p| p.status() != sysinfo::ProcessStatus::Zombie)
    }

    /// The id of Chromium's GPU process among wubble's children, refreshing only the process
    /// list and command lines (no memory or CPU). It still reads every process on the system
    /// (about 10 ms on a desktop's 800), so it belongs off the UI thread. `None` when there is
    /// none now. On macOS and Windows it is the child launched with
    /// `--type=gpu-process`; on Linux Chromium forks it from its unsandboxed zygote, so its
    /// command line is the zygote's, and it is the process that runs Viz's compositor thread
    /// (`VizCompositorThread`, named `VizCompositorTh` by the kernel's 15-character limit).
    pub fn gpu_pid(&mut self) -> Option<u32> {
        let kind = ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet);
        // Threads are needed only to find Viz's on Linux.
        #[cfg(not(target_os = "linux"))]
        let kind = kind.without_tasks();
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let processes = self.system.processes();
        let table: Vec<(u32, Option<u32>)> = processes
            .iter()
            .map(|(pid, p)| (pid.as_u32(), p.parent().map(Pid::as_u32)))
            .collect();
        let tree = descendants(self.root, &table);
        let get = |pid: u32| processes.get(&Pid::from_u32(pid));
        let labelled = tree.iter().copied().find(|&pid| {
            get(pid).is_some_and(|p| {
                let args: Vec<String> = p
                    .cmd()
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                label(&args, pid == self.root) == "GPU"
            })
        });
        labelled.or_else(|| {
            tree.iter().find_map(|&pid| {
                let p = get(pid)?;
                (p.name() == VIZ_THREAD).then(|| p.parent().map(Pid::as_u32))?
            })
        })
    }

    /// Ends `pid` (SIGKILL / TerminateProcess) if it is one of wubble's child processes as of the
    /// last sample, never wubble itself or anything outside its tree. Whether it was signalled.
    pub fn end(&self, pid: u32) -> bool {
        let table: Vec<(u32, Option<u32>)> = self
            .system
            .processes()
            .iter()
            .map(|(p, proc)| (p.as_u32(), proc.parent().map(Pid::as_u32)))
            .collect();
        pid != self.root
            && descendants(self.root, &table).contains(&pid)
            && self
                .system
                .process(Pid::from_u32(pid))
                .is_some_and(|p| p.kill())
    }
}

/// The id a renderer reports for itself (`std::process::id()` in the renderer) as one of `pids`
/// (wubble's processes): a sandboxed Linux renderer lives in its own PID namespace and reports
/// the id it has there, which is not the system's. `ns` gives a process's innermost-namespace id
/// (Linux `NSpid`, [`crate::namespace_pid`]; `None` where there is none, and then the reported id
/// is the system's). `None` when no process or more than one has that id and none is the
/// reported id itself.
pub fn host_pid(reported: u32, pids: &[u32], ns: impl Fn(u32) -> Option<u32>) -> Option<u32> {
    let hits: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|&p| ns(p).unwrap_or(p) == reported)
        .collect();
    match hits.as_slice() {
        [one] => Some(*one),
        [] => None,
        many => many.contains(&reported).then_some(reported),
    }
}

/// `threads` with the main thread first. sysinfo leaves out the thread whose id is the
/// process's (Linux `get_proc_tasks`), and the process's time holds every thread's: the main
/// thread's is the rest, together with threads that have exited. No row without threads.
pub fn with_main(
    root: u32,
    root_ms: u64,
    threads: Vec<(u32, String, u64)>,
) -> Vec<(u32, String, u64)> {
    if threads.is_empty() {
        return threads;
    }
    let others: u64 = threads.iter().map(|t| t.2).sum();
    let mut out = vec![(root, "main".to_string(), root_ms.saturating_sub(others))];
    out.extend(threads);
    out
}

/// The kernel's name of Viz's compositor thread, which runs in the GPU process.
const VIZ_THREAD: &str = "VizCompositorTh";

/// `root` and every process descending from it in `table` (pid, parent), root first, then in
/// breadth-first order by pid. Empty when `root` is not in the table.
pub fn descendants(root: u32, table: &[(u32, Option<u32>)]) -> Vec<u32> {
    if !table.iter().any(|(p, _)| *p == root) {
        return vec![];
    }
    let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(pid, parent) in table {
        if let Some(parent) = parent.filter(|&p| p != pid) {
            children.entry(parent).or_default().push(pid);
        }
    }
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let mut kids = children.get(&out[i]).cloned().unwrap_or_default();
        kids.sort_unstable();
        kids.retain(|k| !out.contains(k));
        out.extend(kids);
        i += 1;
    }
    out
}

/// A Chromium process's role from its command line (`--type=`, and `--utility-sub-type=` for
/// the network and storage services). The root process is the browser. A renderer hosting an
/// extension (`--extension-process`, Chromium `extensions/common/switches.cc`) is an Extension.
pub fn label(args: &[String], root: bool) -> String {
    let value = |key: &str| {
        args.iter()
            .find_map(|a| a.strip_prefix(key)?.strip_prefix('='))
    };
    if root {
        return "Browser".into();
    }
    match value("--type") {
        Some("renderer") if args.iter().any(|a| a == "--extension-process") => "Extension".into(),
        Some("renderer") => "Renderer".into(),
        Some("gpu-process") => "GPU".into(),
        Some("zygote") => "Zygote".into(),
        Some("utility") => match value("--utility-sub-type") {
            Some(s) if s.starts_with("network.") => "Network service".into(),
            Some(s) if s.starts_with("storage.") => "Storage service".into(),
            Some(s) if s.starts_with("audio.") => "Audio service".into(),
            _ => "Utility".into(),
        },
        Some(other) => other.to_string(),
        None => "Helper".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descendants_walk_the_tree_from_the_root_only() {
        // 1 is init; 10 is wubble with 11 (zygote) -> 12, 13 (renderers) and 14 (gpu);
        // 20 is someone else's process; 30 is its own parent (a bad entry).
        let table = [
            (1, None),
            (10, Some(1)),
            (12, Some(11)),
            (11, Some(10)),
            (14, Some(10)),
            (13, Some(11)),
            (20, Some(1)),
            (30, Some(30)),
        ];
        assert_eq!(descendants(10, &table), vec![10, 11, 14, 12, 13]);
        assert_eq!(descendants(99, &table), Vec::<u32>::new());
        assert_eq!(descendants(30, &table), vec![30]);
    }

    #[test]
    fn a_namespaced_renderers_reported_id_is_found_among_the_processes() {
        // 100 browser, 101 zygote, 120 and 121 sandboxed renderers that see themselves as 5
        // and 6, 130 the GPU process (no namespace)
        let ns = |p| match p {
            120 => Some(5),
            121 => Some(6),
            101 => Some(1),
            100 | 130 => Some(p),
            _ => None,
        };
        let all = [100, 101, 120, 121, 130];
        assert_eq!(host_pid(5, &all, ns), Some(120));
        assert_eq!(host_pid(6, &all, ns), Some(121));
        // no namespace (no sandbox, or another OS): the reported id is the system's
        assert_eq!(host_pid(130, &all, ns), Some(130));
        assert_eq!(host_pid(130, &all, |_| None), Some(130));
        // unknown, or two renderers claiming one id and neither the id itself
        assert_eq!(host_pid(77, &all, ns), None);
        assert_eq!(
            host_pid(5, &[120, 122], |p| Some(if p == 120 || p == 122 {
                5
            } else {
                p
            })),
            None
        );
    }

    #[test]
    fn labels_from_the_command_line() {
        let args = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(label(&args("wubble --x"), true), "Browser");
        assert_eq!(
            label(&args("h --type=renderer --lang=en"), false),
            "Renderer"
        );
        assert_eq!(label(&args("h --type=gpu-process"), false), "GPU");
        assert_eq!(
            label(&args("h --type=renderer --extension-process"), false),
            "Extension"
        );
        assert_eq!(
            label(
                &args("h --type=utility --utility-sub-type=network.mojom.NetworkService"),
                false
            ),
            "Network service"
        );
        assert_eq!(label(&args("h --type=utility"), false), "Utility");
        assert_eq!(label(&args("h --type=broker"), false), "broker");
        assert_eq!(label(&args("h"), false), "Helper");
    }

    #[test]
    fn the_main_thread_is_the_process_less_its_other_threads() {
        let threads = vec![(11, "a".to_string(), 30), (12, "b".to_string(), 20)];
        assert_eq!(
            with_main(10, 140, threads),
            vec![
                (10, "main".to_string(), 90),
                (11, "a".to_string(), 30),
                (12, "b".to_string(), 20)
            ]
        );
        // Clock ticks may put the threads' sum past the process's.
        assert_eq!(with_main(10, 5, vec![(11, "a".into(), 10)])[0].2, 0);
        assert!(with_main(10, 140, vec![]).is_empty());
    }

    #[test]
    fn the_meter_sees_this_process() {
        let mut m = Meter::default();
        let tree = m.sample();
        assert_eq!(tree.first().map(|p| p.pid), Some(std::process::id()));
        assert_eq!(tree[0].label, "Browser");
        assert_eq!(
            m.resample().first().map(|p| p.pid),
            Some(std::process::id())
        );
        #[cfg(target_os = "linux")]
        assert_eq!(m.threads().first().map(|t| t.0), Some(std::process::id()));
        // Threads are not processes of the tree: with one more running, it is still this
        // process alone (no test here starts a child process).
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::spawn(move || wait.recv());
        assert_eq!(m.sample().len(), 1);
        drop(stop);
        let _ = thread.join();
        // Never ends itself.
        assert!(!m.end(std::process::id()));
    }
}
