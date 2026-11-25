//! The OS edge of the error panel's auto-reload (`page_error`): news that the system is online
//! again, without a request of its own (Chromium's `NetworkChangeNotifier` does the same, from
//! the OS). One trait, one implementation per platform: Linux asks NetworkManager over D-Bus
//! (zbus, already in the tree), macOS reads SystemConfiguration's reachability of any address;
//! elsewhere, or without NetworkManager, the backoff alone retries.

use std::time::Duration;

/// Called on another thread each time the system goes from offline to online.
pub type CameOnline = Box<dyn Fn() + Send + 'static>;

/// Where the system says whether it is online.
trait OnlineWatch {
    /// Watches on a thread of its own for the run, calling `came_online` on each return.
    fn watch(_came_online: CameOnline) {}
}

/// Starts watching for the run; a platform with nothing to ask does nothing.
pub fn watch(came_online: CameOnline) {
    Current::watch(came_online);
}

/// Runs `body` on a named thread; a failure to start is one line.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn spawn(body: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new()
        .name("wubble-online".into())
        .spawn(body)
    {
        eprintln!("wubble: no network watch ({e}); failed pages retry on their backoff alone");
    }
}

/// How often macOS's reachability is read (it is a local read, no request).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const POLL: Duration = Duration::from_secs(2);

#[cfg(target_os = "linux")]
type Current = linux::NetworkManager;
#[cfg(target_os = "macos")]
type Current = macos::Reachability;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
type Current = NoWatch;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct NoWatch;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl OnlineWatch for NoWatch {}

#[cfg(target_os = "linux")]
mod linux {
    /// NetworkManager's `State` property (`org.freedesktop.NetworkManager`): 70 is
    /// `NM_STATE_CONNECTED_GLOBAL`, the internet reachable.
    const CONNECTED_GLOBAL: u32 = 70;

    pub struct NetworkManager;

    impl super::OnlineWatch for NetworkManager {
        fn watch(came_online: super::CameOnline) {
            super::spawn(move || {
                if let Err(e) = follow(&came_online) {
                    // no system bus or no NetworkManager (systemd-networkd alone): the backoff
                    eprintln!("wubble: no NetworkManager to watch ({e})");
                }
            });
        }
    }

    fn follow(came_online: &super::CameOnline) -> zbus::Result<()> {
        let bus = zbus::blocking::Connection::system()?;
        let manager = zbus::blocking::Proxy::new(
            &bus,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )?;
        let mut online = manager.get_property::<u32>("State")? >= CONNECTED_GLOBAL;
        for change in manager.receive_property_changed::<u32>("State") {
            let Ok(state) = change.get() else {
                continue;
            };
            let now = state >= CONNECTED_GLOBAL;
            if now && !online {
                came_online();
            }
            online = now;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCNetworkReachabilityCreateWithAddress(
            allocator: *const c_void,
            address: *const libc::sockaddr,
        ) -> *mut c_void;
        fn SCNetworkReachabilityGetFlags(target: *mut c_void, flags: *mut u32) -> u8;
    }

    /// `kSCNetworkReachabilityFlagsReachable` and `...ConnectionRequired`.
    const REACHABLE: u32 = 1 << 1;
    const CONNECTION_REQUIRED: u32 = 1 << 2;

    pub struct Reachability;

    impl super::OnlineWatch for Reachability {
        fn watch(came_online: super::CameOnline) {
            super::spawn(move || {
                // the zero address: whether any route out exists (Apple's Reachability sample)
                let mut zero: libc::sockaddr_in = unsafe { std::mem::zeroed() };
                zero.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
                zero.sin_family = libc::AF_INET as u8;
                // SAFETY: a valid sockaddr_in, read during the call only; the target is never
                // released: it lives for the run, as this thread does.
                let target = unsafe {
                    SCNetworkReachabilityCreateWithAddress(
                        std::ptr::null(),
                        (&raw const zero).cast::<libc::sockaddr>(),
                    )
                };
                if target.is_null() {
                    eprintln!("wubble: no reachability to watch");
                    return;
                }
                let online = || {
                    let mut flags = 0u32;
                    // SAFETY: `target` is a live SCNetworkReachability, `flags` a valid out.
                    let read = unsafe { SCNetworkReachabilityGetFlags(target, &mut flags) } != 0;
                    read && flags & REACHABLE != 0 && flags & CONNECTION_REQUIRED == 0
                };
                let mut was = online();
                loop {
                    std::thread::sleep(super::POLL);
                    let now = online();
                    if now && !was {
                        came_online();
                    }
                    was = now;
                }
            });
        }
    }
}
