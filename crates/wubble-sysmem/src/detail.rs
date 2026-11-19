//! Per-process memory beyond RSS for `--measure memory`: PSS and USS on Linux
//! (`/proc/<pid>/smaps_rollup`), `phys_footprint` on macOS (`proc_pid_rusage`, the number
//! Activity Monitor shows), and GPU memory per process on Linux with an NVIDIA driver
//! (`nvidia-smi -q -d PIDS`, when the binary is there). The parsers are pure in
//! `wubble_core::memreport`. Reading smaps_rollup walks every mapping of the process (about a
//! millisecond for a browser), so this is for the measurement, not the Task manager's tick.

use std::collections::BTreeMap;

use wubble_core::memreport::{ProcMem, Rollup};

use crate::Proc;

/// `procs` (one [`crate::Meter::sample`]) with each process's detail and GPU memory.
pub fn memory(procs: &[Proc]) -> Vec<ProcMem> {
    let gpu = gpu_by_pid();
    procs
        .iter()
        .map(|p| {
            let rollup = rollup(p.pid).unwrap_or_default();
            ProcMem {
                pid: p.pid,
                label: p.label.clone(),
                rss: p.sample.rss_bytes,
                pss: rollup.pss,
                uss: rollup.uss,
                footprint: footprint(p.pid),
                gpu: gpu.get(&p.pid).copied(),
            }
        })
        .collect()
}

/// `/proc/<pid>/smaps_rollup`, parsed. `None` off Linux or when the read is refused.
pub fn rollup(pid: u32) -> Option<Rollup> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    Some(wubble_core::memreport::parse_smaps_rollup(&text))
}

/// The process's id in its innermost PID namespace (Linux `NSpid`, last field): what a sandboxed
/// renderer sees as its own id and reports on its frame messages. `None` off Linux.
pub fn namespace_pid(pid: u32) -> Option<u32> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("NSpid:"))?
        .split_whitespace()
        .last()?
        .parse()
        .ok()
}

/// The process's `phys_footprint` (macOS); `None` elsewhere or when the call fails.
#[cfg(target_os = "macos")]
pub fn footprint(pid: u32) -> Option<u64> {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: a zeroed `rusage_info_v4` the kernel fills for flavor RUSAGE_INFO_V4.
    let rc = unsafe {
        libc::proc_pid_rusage(
            libc::c_int::try_from(pid).ok()?,
            libc::RUSAGE_INFO_V4,
            (&raw mut info).cast(),
        )
    };
    (rc == 0).then_some(info.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
pub fn footprint(_pid: u32) -> Option<u64> {
    None
}

/// GPU memory per process from `nvidia-smi` (Linux); empty without the binary or the driver.
/// On macOS GPU memory is unified and counted in each process's footprint.
pub fn gpu_by_pid() -> BTreeMap<u32, u64> {
    if !cfg!(target_os = "linux") {
        return BTreeMap::new();
    }
    std::process::Command::new("nvidia-smi")
        .args(["-q", "-d", "PIDS"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| wubble_core::memreport::parse_nvidia_pids(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    /// This process's own detail is readable on the OSes that have one.
    #[test]
    fn reads_this_process() {
        let pid = std::process::id();
        if cfg!(target_os = "linux") {
            assert!(super::rollup(pid).and_then(|r| r.pss).is_some());
        }
        if cfg!(target_os = "linux") {
            assert!(super::namespace_pid(pid).is_some());
        }
        if cfg!(target_os = "macos") {
            assert!(super::footprint(pid).is_some());
        }
    }
}
