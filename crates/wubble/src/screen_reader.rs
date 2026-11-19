//! Whether a screen reader runs, for pages' accessibility on auto (`a11y.web`,
//! `wubble_core::apply::web_accessibility`). One trait, one implementation per platform, read off
//! the UI thread as the reduced-motion preference is (`os_motion.rs`).
//!
//! Linux: AT-SPI's `org.a11y.Status ScreenReaderEnabled` on the session bus's `org.a11y.Bus`
//! (the property Orca sets, and GTK and Qt read to turn their own accessibility on). macOS
//! (`NSWorkspace.isVoiceOverEnabled`) and Windows (`SPI_GETSCREENREADER`) have no reader yet:
//! the setting alone decides there.

/// Where "a screen reader runs" comes from.
pub trait ScreenReader {
    fn running(&self) -> bool;
}

/// The platform's source.
pub fn source() -> Box<dyn ScreenReader> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::AtSpi::start())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(NoReader)
    }
}

/// No way to tell yet: none runs.
#[cfg(not(target_os = "linux"))]
struct NoReader;

#[cfg(not(target_os = "linux"))]
impl ScreenReader for NoReader {
    fn running(&self) -> bool {
        false
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub struct AtSpi {
        running: Arc<AtomicBool>,
    }

    impl AtSpi {
        pub fn start() -> Self {
            Self {
                running: crate::os_motion::linux::poll_session_flag("wubble-screen-reader", read),
            }
        }
    }

    impl super::ScreenReader for AtSpi {
        fn running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }
    }

    /// `org.a11y.Status ScreenReaderEnabled`; `None` without an AT-SPI bus.
    fn read(bus: &zbus::blocking::Connection) -> Option<bool> {
        let reply = bus
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.a11y.Status", "ScreenReaderEnabled"),
            )
            .ok()?;
        let v = reply
            .body()
            .deserialize::<zbus::zvariant::OwnedValue>()
            .ok()?;
        bool::try_from(&v).ok()
    }
}
