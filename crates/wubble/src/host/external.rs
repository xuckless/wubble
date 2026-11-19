//! Links to other apps (`mailto:`, `tel:`, an app's scheme), host side: the engine hands them
//! over instead of loading them (`PageEvent::ExternalProtocol`); `wubble_core::external` says
//! whether one opens, is asked about (the permission prompt, with "Always for <site> and
//! <scheme> links" kept in the site table) or is dropped; opening is the OS's (`opener`, which
//! runs `xdg-open`, `open` or `ShellExecuteW`).

use wubble_core::external::{self, Verdict};
use wubble_core::tab::TabId;
use wubble_engine::page::RequestId;

use super::Host;
use crate::prompt::{PermissionEvent, PermissionRequest};

/// How a link reaches its app: the OS by default; `--smoke parity` records it instead.
pub(crate) type Opener = fn(&str);

/// Hands `url` to the app the OS has for its scheme, off the UI thread (the OS may take a
/// while to start it).
pub(crate) fn open_with_os(url: &str) {
    let url = url.to_owned();
    let spawned = std::thread::Builder::new()
        .name("wubble-open-link".into())
        .spawn(move || {
            if let Err(e) = opener::open(&url) {
                eprintln!("wubble: no app opened {url}: {e}");
            }
        });
    if let Err(e) = spawned {
        eprintln!("wubble: cannot open a link in another app: {e}");
    }
}

/// The ids of the host's own external-link prompts, apart from the engine's request ids (which
/// count up from 1 per view) and small enough for the prompt's serial.
const FIRST_ASK: u32 = 1_000_000_000;

impl Host {
    /// The page in `tab` went to a link another app handles. `gesture` is CEF's word on a user
    /// gesture, when it gave one; without it the link is treated as made without one.
    pub(super) fn external_link(&mut self, tab: TabId, url: String, gesture: Option<bool>) {
        let page = self.committed.get(&tab).cloned().unwrap_or_default();
        match external::decide(&url, &page, gesture == Some(true), &self.site_permissions) {
            Verdict::Open => self.open_in_app(&url),
            Verdict::Block => eprintln!("wubble: {url}: not opened (no click, or blocked)"),
            Verdict::Ask => {
                self.external_asks += 1;
                let request = PermissionRequest {
                    id: RequestId(FIRST_ASK + self.external_asks),
                    origin: page,
                    kinds: vec![],
                    external: Some(url),
                };
                self.apply_permission(tab, PermissionEvent::Requested(request));
            }
        }
    }

    pub(super) fn open_in_app(&mut self, url: &str) {
        (self.external_opener)(url);
    }
}
