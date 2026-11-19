//! The OS's reduced-motion preference ("Calmer motion" on auto follows it). One trait, one
//! implementation per platform. Read off the UI thread: a hung portal never stalls a frame.
//!
//! Linux: the XDG Settings portal, `org.freedesktop.appearance reduced-motion` (1 = reduced;
//! xdg-desktop-portal 1.21 and later), else GNOME's `org.gnome.desktop.interface
//! enable-animations` through the same portal. macOS and Windows have no reader yet (Slint 1.19
//! keeps its own reading internal, winit 0.30 has none): the setting alone decides there.

/// Where the OS's reduced-motion preference comes from.
pub trait MotionSource {
    /// Whether the OS asks for reduced motion, as last read.
    fn reduced(&self) -> bool;
}

/// The platform's source.
pub fn source() -> Box<dyn MotionSource> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::Portal::start())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(NoSource)
    }
}

/// No OS preference known: never reduced.
#[cfg(not(target_os = "linux"))]
struct NoSource;

#[cfg(not(target_os = "linux"))]
impl MotionSource for NoSource {
    fn reduced(&self) -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    /// How often a worker reads its flag again (an OS change shows within this).
    const POLL: Duration = Duration::from_secs(5);

    /// A flag of the session bus read on a worker thread every [`POLL`], the latest answer in
    /// the atomic (false until the first). The worker stops when there is no session bus or
    /// `read` finds nothing to follow. Shared by the OS preferences the host follows.
    pub(crate) fn poll_session_flag(
        name: &str,
        read: fn(&zbus::blocking::Connection) -> Option<bool>,
    ) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let out = flag.clone();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let Ok(bus) = zbus::blocking::Connection::session() else {
                    return;
                };
                while let Some(on) = read(&bus) {
                    out.store(on, Ordering::Relaxed);
                    std::thread::sleep(POLL);
                }
            });
        if let Err(e) = spawned {
            eprintln!("wubble: {name}: {e}");
        }
        flag
    }

    /// The portal read on a worker thread; the latest answer in an atomic.
    pub struct Portal {
        reduced: Arc<AtomicBool>,
    }

    impl Portal {
        pub fn start() -> Self {
            Self {
                reduced: poll_session_flag("wubble-os-motion", read),
            }
        }
    }

    impl super::MotionSource for Portal {
        fn reduced(&self) -> bool {
            self.reduced.load(Ordering::Relaxed)
        }
    }

    fn read_one(
        bus: &zbus::blocking::Connection,
        namespace: &str,
        key: &str,
    ) -> Option<zbus::zvariant::OwnedValue> {
        let reply = bus
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                "ReadOne",
                &(namespace, key),
            )
            .ok()?;
        reply
            .body()
            .deserialize::<zbus::zvariant::OwnedValue>()
            .ok()
    }

    /// The portal's answer: `org.freedesktop.appearance reduced-motion`, else GNOME's
    /// `enable-animations` (off = reduced). `None` when neither is there.
    fn read(bus: &zbus::blocking::Connection) -> Option<bool> {
        if let Some(v) = read_one(bus, "org.freedesktop.appearance", "reduced-motion")
            && let Ok(n) = u32::try_from(&v)
        {
            return Some(n == 1);
        }
        let v = read_one(bus, "org.gnome.desktop.interface", "enable-animations")?;
        bool::try_from(&v).ok().map(|on| !on)
    }
}
