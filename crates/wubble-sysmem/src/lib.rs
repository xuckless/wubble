//! What the OS says about memory: the pressure source for the lifecycle tick and the process
//! meter for the Task manager.
//!
//! Only reading happens here; the parsers, thresholds and the gauge are pure in
//! `wubble_core::pressure`, the page's rows in `wubble_core::tasks`.
//!
//! - [`PressureSource`]: one trait, one reader per OS, chosen by [`system_source`].
//!   Linux reads `/proc/pressure/memory` (PSI), else `/proc/meminfo` MemAvailable. macOS reads the
//!   sysctl `kern.memorystatus_vm_pressure_level` (the value Chromium's
//!   `base/memory/memory_pressure_monitor_mac.cc` reads next to its dispatch source; polled on
//!   our tick instead of a dispatch source). Windows reads the low-memory resource notification
//!   and `GlobalMemoryStatusEx`.
//! - [`Meter`]: RSS and CPU of wubble's own process tree, through `sysinfo` (MIT).

mod meter;

pub use meter::{Meter, Proc, descendants, label};
use wubble_core::pressure::Reading;

/// Where memory pressure readings come from. Read on the lifecycle tick (every few seconds).
pub trait PressureSource {
    fn read(&mut self) -> Reading;
}

/// This OS's source.
pub fn system_source() -> Box<dyn PressureSource> {
    #[cfg(target_os = "linux")]
    return Box::new(linux::ProcPressure);
    #[cfg(target_os = "macos")]
    return Box::new(macos::SysctlPressure);
    #[cfg(windows)]
    return Box::new(windows::WinPressure::new());
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Box::new(Unknown);
}

/// No source on this OS: never any pressure.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
struct Unknown;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
impl PressureSource for Unknown {
    fn read(&mut self) -> Reading {
        Reading::Unknown
    }
}

/// Linux. Pure part separate from the file reads so fixtures can drive it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod linux {
    use wubble_core::pressure::{Reading, parse_meminfo, parse_psi};

    /// `/proc/pressure/memory`, else `/proc/meminfo`.
    pub struct ProcPressure;

    impl super::PressureSource for ProcPressure {
        fn read(&mut self) -> Reading {
            let psi = std::fs::read_to_string("/proc/pressure/memory").ok();
            // Only read when PSI is missing (a kernel without it, or booted with `psi=0`, where
            // the read fails).
            let meminfo = || std::fs::read_to_string("/proc/meminfo").ok();
            read_from(psi.as_deref(), meminfo)
        }
    }

    /// The reading from the two files' texts (`None`: unreadable).
    pub fn read_from(psi: Option<&str>, meminfo: impl FnOnce() -> Option<String>) -> Reading {
        if let Some(p) = psi.and_then(parse_psi) {
            return Reading::Psi(p);
        }
        meminfo()
            .as_deref()
            .and_then(parse_meminfo)
            .unwrap_or(Reading::Unknown)
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use wubble_core::pressure::{Reading, mac_level};

    pub struct SysctlPressure;

    impl super::PressureSource for SysctlPressure {
        fn read(&mut self) -> Reading {
            let mut value: libc::c_int = 0;
            let mut size = std::mem::size_of::<libc::c_int>();
            // SAFETY: a NUL-terminated name, an int-sized out buffer and its size.
            let rc = unsafe {
                libc::sysctlbyname(
                    c"kern.memorystatus_vm_pressure_level".as_ptr(),
                    (&raw mut value).cast(),
                    &raw mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rc != 0 {
                return Reading::Unknown;
            }
            Reading::Level(mac_level(i64::from(value)))
        }
    }
}

#[cfg(windows)]
mod windows {
    use ::windows::Win32::Foundation::{CloseHandle, HANDLE};
    use ::windows::Win32::System::Memory::{
        CreateMemoryResourceNotification, LowMemoryResourceNotification,
        QueryMemoryResourceNotification,
    };
    use ::windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use wubble_core::pressure::{Reading, level_of};
    use wubble_core::sleep::Pressure;

    /// The system's low-memory notification (polled, not waited on) and the available
    /// physical memory.
    pub struct WinPressure {
        low: Option<HANDLE>,
    }

    impl WinPressure {
        pub fn new() -> Self {
            // SAFETY: plain Win32 call; the handle is closed on drop.
            let low = unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification) };
            Self { low: low.ok() }
        }
    }

    impl Drop for WinPressure {
        fn drop(&mut self) {
            if let Some(h) = self.low.take() {
                // SAFETY: our own handle, closed once.
                let _ = unsafe { CloseHandle(h) };
            }
        }
    }

    impl super::PressureSource for WinPressure {
        fn read(&mut self) -> Reading {
            let mut status = MEMORYSTATUSEX {
                dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };
            // SAFETY: an initialised struct with its length set.
            let available = unsafe { GlobalMemoryStatusEx(&mut status) }
                .ok()
                .map(|()| Reading::Available {
                    avail: status.ullAvailPhys,
                    total: status.ullTotalPhys,
                })
                .unwrap_or(Reading::Unknown);
            let mut low = ::windows::core::BOOL(0);
            let signalled = self.low.is_some_and(|h| {
                // SAFETY: our own notification handle and a BOOL out parameter.
                unsafe { QueryMemoryResourceNotification(h, &mut low) }.is_ok() && low.as_bool()
            });
            if signalled && level_of(&available) < Pressure::Moderate {
                return Reading::Level(Pressure::Moderate);
            }
            available
        }
    }
}

#[cfg(test)]
mod tests {
    use super::linux::read_from;
    use wubble_core::pressure::{Psi, Reading};

    const PSI: &str = "some avg10=12.50 avg60=1.00 avg300=0.10 total=9\n\
                       full avg10=0.00 avg60=0.00 avg300=0.00 total=1\n";
    const MEMINFO: &str = "MemTotal: 1000 kB\nMemFree: 10 kB\nMemAvailable: 40 kB\n";

    #[test]
    fn psi_first_meminfo_when_psi_is_missing_or_broken() {
        let psi = Reading::Psi(Psi {
            some_avg10: 12.5,
            full_avg10: 0.0,
        });
        let meminfo = Reading::Available {
            avail: 40 * 1024,
            total: 1000 * 1024,
        };
        assert_eq!(read_from(Some(PSI), || panic!("meminfo not needed")), psi);
        assert_eq!(read_from(None, || Some(MEMINFO.into())), meminfo);
        assert_eq!(read_from(Some("garbage"), || Some(MEMINFO.into())), meminfo);
        assert_eq!(read_from(None, || None), Reading::Unknown);
    }
}
