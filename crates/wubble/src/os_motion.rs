//! The OS's reduced-motion preference ("Calmer motion" on auto follows it). One trait, one
//! implementation per platform. Read off the UI thread: a hung portal never stalls a frame.
//!
//! Linux: the XDG Settings portal, `org.freedesktop.appearance reduced-motion` (1 = reduced;
//! xdg-desktop-portal 1.21 and later), else GNOME's `org.gnome.desktop.interface
//! enable-animations` through the same portal. macOS: `NSWorkspace`'s
//! `accessibilityDisplayShouldReduceMotion` (System Settings > Accessibility > Display > Reduce
//! motion), read on each call (an in-process property; no thread). Windows: the "Animation
//! effects" switch (`SPI_GETCLIENTAREAANIMATION`), read on each call as Slint's own winit backend
//! does (there is no change notification a winit window receives). Other targets: never reduced.

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
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::Workspace)
    }
    #[cfg(windows)]
    {
        Box::new(win::AnimationEffects)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Box::new(NoSource)
    }
}

/// No OS preference known: never reduced.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
struct NoSource;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
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

#[cfg(target_os = "macos")]
mod macos {
    use objc2_app_kit::NSWorkspace;

    /// The shared workspace's accessibility display options.
    pub struct Workspace;

    impl super::MotionSource for Workspace {
        fn reduced(&self) -> bool {
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
        }
    }
}

// Ported from Slint's winit backend (`i-slint-backend-winit-1.19.0/windows_settings.rs`,
// `motion_preference`; GPL-3.0-only OR the Slint licences, taken under GPL-3.0-only).
#[cfg(windows)]
mod win {
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };

    /// The "Animation effects" switch under Settings > Accessibility > Visual effects.
    pub struct AnimationEffects;

    impl super::MotionSource for AnimationEffects {
        /// Off = reduced; animations on when the setting cannot be read.
        fn reduced(&self) -> bool {
            let mut enabled = windows::core::BOOL::from(true);
            // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through `pvparam`, which points
            // at `enabled` for the call's duration.
            let read = unsafe {
                SystemParametersInfoW(
                    SPI_GETCLIENTAREAANIMATION,
                    0,
                    Some(&mut enabled as *mut _ as *mut core::ffi::c_void),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
            };
            read.is_ok() && !enabled.as_bool()
        }
    }
}
