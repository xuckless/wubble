//! Links to other apps (`mailto:`, `tel:`, an app's scheme), host side: the engine hands them
//! over instead of loading them (`PageEvent::ExternalProtocol`); `wubble_core::external` says
//! whether one opens, is asked about (the permission prompt, with "Always for <site> and
//! <scheme> links" kept in the site table) or is dropped; opening is the OS's (`opener`, which
//! runs `xdg-open`, `open` or `ShellExecuteW`).

use wubble_core::external::{self, Verdict};
use wubble_core::tab::TabId;

use super::Host;
use crate::prompt::{Ask, PermissionEvent, PermissionRequest};

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

impl Host {
    /// The page in `tab` went to a link another app handles. `gesture` is CEF's word on a user
    /// gesture, when it gave one; without it the link is treated as made without one.
    pub(super) fn external_link(&mut self, tab: TabId, url: String, gesture: Option<bool>) {
        if self.agent_drives(tab) {
            if let Some((session, _)) = self.agent_grant_of(tab) {
                self.agent_log(
                    &session,
                    "external_link",
                    Some(url.clone()),
                    wubble_core::agent::log::Outcome::Refused,
                );
            }
            eprintln!("wubble: agent: {url}: external link refused");
            return;
        }
        let page = self.committed.get(&tab).cloned().unwrap_or_default();
        // a private window's tab reads its own choices over the stored ones
        let verdict =
            external::decide(&url, &page, gesture == Some(true), &self.site_view(tab).permissions());
        match verdict {
            Verdict::Open => self.open_in_app(&url),
            Verdict::Block => eprintln!("wubble: {url}: not opened (no click, or blocked)"),
            Verdict::Ask => {
                let request = PermissionRequest {
                    origin: page,
                    ask: Ask::ExternalLink { url },
                };
                self.apply_permission(tab, PermissionEvent::Requested(request));
            }
        }
    }

    pub(super) fn open_in_app(&mut self, url: &str) {
        (self.external_opener)(url);
    }
}
