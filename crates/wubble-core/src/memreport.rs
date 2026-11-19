//! `--measure memory`'s report: per process resident, proportional and unique memory, the macOS
//! footprint and GPU memory where readable, summed on one basis, as a table and as CSV. Pure:
//! the OS readers (`wubble-sysmem`) hand in file and command texts; nothing here reads a process.
//!
//! The totals never mix quantities: RSS counts a shared page in every process that maps it, so
//! a process tree's sum is the sum of PSS on Linux (`/proc/<pid>/smaps_rollup`), of
//! `phys_footprint` on macOS (what Activity Monitor shows as Memory), and RSS only when neither
//! is known for every process, labelled so. The choice of fields follows Chromium
//! `base/process/process_metrics_linux.cc` and `process_metrics_mac.cc` (BSD-3-Clause; field
//! names only, no code copied).

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One process's memory, in bytes. `None`: not readable on this OS or for this process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcMem {
    pub pid: u32,
    /// Browser, Renderer, GPU, Network service, ...
    pub label: String,
    pub rss: u64,
    /// Proportional set size (Linux): each shared page divided among the processes mapping it.
    pub pss: Option<u64>,
    /// Unique set size (Linux): private clean + private dirty pages.
    pub uss: Option<u64>,
    /// `phys_footprint` (macOS): dirty, compressed and IOSurface/GPU pages charged to it.
    pub footprint: Option<u64>,
    /// GPU memory the driver charges to this process (Linux, NVIDIA).
    pub gpu: Option<u64>,
}

/// The fields of `/proc/<pid>/smaps_rollup` the report uses, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rollup {
    pub rss: Option<u64>,
    pub pss: Option<u64>,
    /// `Private_Clean + Private_Dirty`, when both are there.
    pub uss: Option<u64>,
    /// Anonymous memory (heaps, GPU driver mappings) and file-backed memory (mapped code and
    /// data) of the PSS.
    pub pss_anon: Option<u64>,
    pub pss_file: Option<u64>,
}

/// Parses `/proc/<pid>/smaps_rollup` (kB values). Lines it does not know are ignored; a field
/// that is missing stays `None`.
pub fn parse_smaps_rollup(text: &str) -> Rollup {
    let mut fields: BTreeMap<&str, u64> = BTreeMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let mut words = rest.split_whitespace();
        if let (Some(n), Some("kB")) = (
            words.next().and_then(|n| n.parse::<u64>().ok()),
            words.next(),
        ) {
            fields.insert(key.trim(), n * 1024);
        }
    }
    let get = |k: &str| fields.get(k).copied();
    Rollup {
        rss: get("Rss"),
        pss: get("Pss"),
        uss: get("Private_Clean")
            .zip(get("Private_Dirty"))
            .map(|(c, d)| c + d),
        pss_anon: get("Pss_Anon"),
        pss_file: get("Pss_File"),
    }
}

/// `/proc/<pid>/smaps` summed by what is mapped: PSS per path (`[heap]`, `[stack]` and the like
/// as the kernel names them, `[anon]` for unnamed mappings), largest first, the first `n`.
pub fn smaps_by_mapping(text: &str, n: usize) -> Vec<(String, u64)> {
    let mut by: BTreeMap<String, u64> = BTreeMap::new();
    let mut name = String::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or("");
        if first.contains('-') && !first.ends_with(':') {
            // `start-end perms offset dev inode [path]`: the path may hold spaces.
            let path: Vec<&str> = words.skip(4).collect();
            name = if path.is_empty() {
                "[anon]".into()
            } else {
                path.join(" ")
            };
        } else if first == "Pss:"
            && let Some(kb) = words.next().and_then(|v| v.parse::<u64>().ok())
        {
            *by.entry(name.clone()).or_insert(0) += kb * 1024;
        }
    }
    let mut out: Vec<(String, u64)> = by.into_iter().filter(|(_, b)| *b > 0).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.truncate(n);
    out
}

/// GPU memory per process from `nvidia-smi -q -d PIDS`: each `Process ID` line followed by its
/// `Used GPU Memory : N MiB`. That query lists graphics clients (type G and C+G); the CSV
/// `--query-compute-apps` lists compute clients only and misses a Vulkan or GL process.
/// A process on two GPUs is summed; `N/A` amounts are skipped.
pub fn parse_nvidia_pids(text: &str) -> BTreeMap<u32, u64> {
    let mut out = BTreeMap::new();
    let mut pid = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Process ID" => pid = value.parse::<u32>().ok(),
            "Used GPU Memory" => {
                let mib = value
                    .strip_suffix("MiB")
                    .and_then(|n| n.trim().parse::<u64>().ok());
                if let (Some(p), Some(mib)) = (pid.take(), mib) {
                    *out.entry(p).or_insert(0) += mib * 1024 * 1024;
                }
            }
            _ => {}
        }
    }
    out
}

/// Which quantity a total sums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Pss,
    Footprint,
    /// Over-counts shared pages: only when neither of the others is known for every process.
    Rss,
}

impl Basis {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pss => "PSS",
            Self::Footprint => "footprint",
            Self::Rss => "RSS (over-counts shared pages)",
        }
    }

    fn of(self, p: &ProcMem) -> u64 {
        match self {
            Self::Pss => p.pss.unwrap_or(0),
            Self::Footprint => p.footprint.unwrap_or(0),
            Self::Rss => p.rss,
        }
    }
}

/// A process tree's sums, each process once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Totals {
    pub basis: Basis,
    pub bytes: u64,
    /// GPU memory over the processes that report it.
    pub gpu: u64,
    pub processes: usize,
    /// Per label: process count and bytes on `basis`, by label.
    pub by_label: BTreeMap<String, (usize, u64)>,
}

/// PSS when every process has it, else footprint when every process has it, else RSS.
pub fn totals(procs: &[ProcMem]) -> Totals {
    let all = |f: fn(&ProcMem) -> bool| !procs.is_empty() && procs.iter().all(f);
    let basis = if all(|p| p.pss.is_some()) {
        Basis::Pss
    } else if all(|p| p.footprint.is_some()) {
        Basis::Footprint
    } else {
        Basis::Rss
    };
    let mut by_label: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for p in procs {
        let e = by_label.entry(p.label.clone()).or_default();
        e.0 += 1;
        e.1 += basis.of(p);
    }
    Totals {
        basis,
        bytes: procs.iter().map(|p| basis.of(p)).sum(),
        gpu: procs.iter().filter_map(|p| p.gpu).sum(),
        processes: procs.len(),
        by_label,
    }
}

/// Bytes as MiB with one decimal.
pub fn mib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

fn opt_mib(v: Option<u64>) -> String {
    v.map_or_else(|| "-".into(), mib)
}

/// The processes of `state` as a table (MiB), then the total and the sum per kind.
pub fn table(state: &str, procs: &[ProcMem]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "memory: {state}");
    let _ = writeln!(
        out,
        "{:>8} {:<16} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "pid", "process", "rss", "pss", "uss", "footprint", "gpu"
    );
    for p in procs {
        let _ = writeln!(
            out,
            "{:>8} {:<16} {:>9} {:>9} {:>9} {:>9} {:>9}",
            p.pid,
            p.label,
            mib(p.rss),
            opt_mib(p.pss),
            opt_mib(p.uss),
            opt_mib(p.footprint),
            opt_mib(p.gpu)
        );
    }
    let t = totals(procs);
    let _ = writeln!(
        out,
        "total: {} MiB {} over {} processes, gpu {} MiB",
        mib(t.bytes),
        t.basis.name(),
        t.processes,
        mib(t.gpu)
    );
    for (label, (n, bytes)) in &t.by_label {
        let _ = writeln!(out, "  {label}: {n} x, {} MiB", mib(*bytes));
    }
    out
}

/// The CSV header of [`csv_rows`].
pub const CSV_HEADER: &str =
    "state,pid,process,rss_bytes,pss_bytes,uss_bytes,footprint_bytes,gpu_bytes\n";

/// One row per process, unknown values empty; a field holding a comma or a quote is quoted.
pub fn csv_rows(state: &str, procs: &[ProcMem]) -> String {
    let field = |s: &str| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_owned()
        }
    };
    let opt = |v: Option<u64>| v.map_or_else(String::new, |v| v.to_string());
    let mut out = String::new();
    for p in procs {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{}",
            field(state),
            p.pid,
            field(&p.label),
            p.rss,
            opt(p.pss),
            opt(p.uss),
            opt(p.footprint),
            opt(p.gpu)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLLUP: &str =
        "5605805b3000-7fff080a1000 ---p 00000000 00:00 0                          [rollup]
Rss:                2004 kB
Pss:                 160 kB
Pss_Dirty:           104 kB
Pss_Anon:            104 kB
Pss_File:             56 kB
Shared_Clean:       1864 kB
Private_Clean:        36 kB
Private_Dirty:       104 kB
Swap:                  0 kB
";

    #[test]
    fn smaps_rollup_fields_in_bytes() {
        let r = parse_smaps_rollup(ROLLUP);
        assert_eq!(r.rss, Some(2004 * 1024));
        assert_eq!(r.pss, Some(160 * 1024));
        assert_eq!(r.uss, Some(140 * 1024));
        assert_eq!(r.pss_anon, Some(104 * 1024));
        assert_eq!(r.pss_file, Some(56 * 1024));
        // A kernel without Pss (or a refused read): None, never 0.
        let r = parse_smaps_rollup("Rss: 8 kB\nPrivate_Dirty: 4 kB\ngarbage\n");
        assert_eq!((r.rss, r.pss, r.uss), (Some(8192), None, None));
        assert_eq!(parse_smaps_rollup(""), Rollup::default());
    }

    #[test]
    fn nvidia_pids_from_the_query_output() {
        let text = "GPU 00000000:01:00.0
    Processes
        Process ID                        : 5317
            Type                          : G
            Name                          : /usr/bin/gnome-shell
            Used GPU Memory               : 119 MiB
        Process ID                        : 42
            Type                          : C+G
            Used GPU Memory               : N/A
        Process ID                        : 7
            Type                          : G
            Used GPU Memory               : 6 MiB

GPU 00000000:02:00.0
        Process ID                        : 7
            Used GPU Memory               : 2 MiB
";
        let m = parse_nvidia_pids(text);
        assert_eq!(m.len(), 2);
        assert_eq!(m[&5317], 119 * 1024 * 1024);
        assert_eq!(m[&7], 8 * 1024 * 1024);
        assert!(!m.contains_key(&42));
        assert!(parse_nvidia_pids("").is_empty());
    }

    #[test]
    fn smaps_summed_by_mapping_largest_first() {
        let text = "00400000-00452000 r-xp 00000000 08:02 173521      /usr/lib/libcef.so
Rss:                 100 kB
Pss:                  40 kB
00600000-00700000 rw-p 00000000 00:00 0
Pss:                  30 kB
00800000-00900000 r--p 00052000 08:02 173521      /usr/lib/libcef.so
Pss:                  20 kB
00a00000-00b00000 rw-p 00000000 00:00 0           [heap]
Pss:                  25 kB
00c00000-00d00000 rw-p 00000000 00:00 0
Pss:                   0 kB
00e00000-00f00000 r--p 00000000 08:02 1           /opt/my dir/a.so
Pss:                   1 kB
";
        assert_eq!(
            smaps_by_mapping(text, 3),
            vec![
                ("/usr/lib/libcef.so".into(), 60 * 1024),
                ("[anon]".into(), 30 * 1024),
                ("[heap]".into(), 25 * 1024),
            ]
        );
        assert_eq!(
            smaps_by_mapping(text, 9)[3],
            ("/opt/my dir/a.so".into(), 1024)
        );
    }

    fn p(pid: u32, label: &str, rss: u64, pss: Option<u64>, footprint: Option<u64>) -> ProcMem {
        ProcMem {
            pid,
            label: label.into(),
            rss,
            pss,
            uss: None,
            footprint,
            gpu: None,
        }
    }

    #[test]
    fn totals_pick_one_basis_and_never_mix() {
        let linux = [
            p(1, "Browser", 100, Some(60), None),
            p(2, "Renderer", 50, Some(20), None),
        ];
        let t = totals(&linux);
        assert_eq!((t.basis, t.bytes, t.processes), (Basis::Pss, 80, 2));
        let mac = [
            p(1, "Browser", 100, None, Some(70)),
            p(2, "Renderer", 50, None, Some(30)),
        ];
        assert_eq!(
            (totals(&mac).basis, totals(&mac).bytes),
            (Basis::Footprint, 100)
        );
        // One process without PSS: RSS for all, not PSS plus RSS.
        let mixed = [
            p(1, "Browser", 100, Some(60), None),
            p(2, "Renderer", 50, None, Some(30)),
        ];
        let t = totals(&mixed);
        assert_eq!((t.basis, t.bytes), (Basis::Rss, 150));
        assert_eq!(t.basis.name(), "RSS (over-counts shared pages)");
        assert_eq!(totals(&[]).basis, Basis::Rss);
        let three = [
            p(1, "Browser", 0, Some(5), None),
            p(2, "Renderer", 0, Some(2), None),
            p(3, "Renderer", 0, Some(3), None),
        ];
        assert_eq!(totals(&three).by_label["Renderer"], (2, 5));
    }

    #[test]
    fn table_and_csv_from_fixed_input() {
        let procs = [
            ProcMem {
                pid: 10,
                label: "Browser".into(),
                rss: 3 * 1024 * 1024,
                pss: Some(2 * 1024 * 1024),
                uss: Some(1024 * 1024 + 51_200),
                footprint: None,
                gpu: Some(512 * 1024),
            },
            p(11, "Utility, odd", 1024 * 1024, Some(1024 * 1024), None),
        ];
        assert_eq!(
            table("one", &procs),
            "memory: one
     pid process                rss       pss       uss footprint       gpu
      10 Browser                3.0       2.0       1.0         -       0.5
      11 Utility, odd           1.0       1.0         -         -         -
total: 3.0 MiB PSS over 2 processes, gpu 0.5 MiB
  Browser: 1 x, 2.0 MiB
  Utility, odd: 1 x, 1.0 MiB
"
        );
        assert_eq!(
            CSV_HEADER.to_owned() + &csv_rows("one", &procs),
            "state,pid,process,rss_bytes,pss_bytes,uss_bytes,footprint_bytes,gpu_bytes
one,10,Browser,3145728,2097152,1099776,,524288
one,11,\"Utility, odd\",1048576,1048576,,,
"
        );
    }
}
