//! The host's half of HTTPS-Upgrades (`wubble_core::https`, `crate::https`): a failed upgrade's
//! effects, the HTTPS-Only panel's Continue, and the allowlist written to its setting (a private
//! container's only for this run).

use wubble_core::https::{self, UpgradeEffect};
use wubble_core::settings as defs;
use wubble_core::space::Storage;
use wubble_core::tab::TabId;
use wubble_store::{PrivacyHttpsAllowed, text_of};

use super::Host;
use crate::page_error::ErrorEvent;

impl Host {
    /// The load of `url` in `tab` failed: `true` when it was an upgrade's, whose effects then
    /// ran (no other panel shows it).
    pub(super) fn https_failed(&mut self, tab: TabId, url: &str) -> bool {
        let Some(browser) = self.tab_view(tab).map(|v| v.browser_id()) else {
            return false;
        };
        let Some(effects) = self.blocking.https.failed(browser, url) else {
            return false;
        };
        self.https_effects(tab, effects);
        true
    }

    /// The HTTPS-Only panel's Continue to site.
    pub(super) fn https_continue(&mut self, tab: TabId) {
        if let Some(browser) = self.tab_view(tab).map(|v| v.browser_id()) {
            let effects = self.blocking.https.continue_to_site(browser);
            self.https_effects(tab, effects);
        }
    }

    fn https_effects(&mut self, tab: TabId, effects: Vec<UpgradeEffect>) {
        for effect in effects {
            match effect {
                // the request filter's own
                UpgradeEffect::Redirect(_) | UpgradeEffect::Block => {}
                UpgradeEffect::LoadHttp(url) => {
                    if let Some(view) = self.tab_view(tab) {
                        view.load(&url);
                    }
                }
                UpgradeEffect::Warn { http, https } => self.apply_error(
                    tab,
                    ErrorEvent::HttpsOnly {
                        url: http,
                        failed: https,
                    },
                ),
                UpgradeEffect::Allow(host) => self.https_allow(tab, &host),
            }
        }
    }

    /// `host` stays on http in `tab`'s container for `https::ALLOW_SECS`; a private
    /// container's for this run only, never stored.
    fn https_allow(&mut self, tab: TabId, host: &str) {
        let Some(container) = self.model.container_of(tab) else {
            return;
        };
        let key = https::allow_key(container.0, host);
        if self.model.storage_of(container) == Storage::Memory {
            self.blocking.https.allow_for_session(key);
            return;
        }
        let now_s = super::surfaces::wall_ms() / 1000;
        let table = https::allow(&self.blocking.https.allowed(), &key, now_s);
        self.write_setting(
            defs::PRIVACY_HTTPS_ALLOWED.key,
            &text_of::<PrivacyHttpsAllowed>(&table),
        );
    }
}
