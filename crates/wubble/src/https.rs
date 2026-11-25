//! HTTPS-Upgrades shared with CEF's IO thread (`wubble_core::https`): the mode and allowlist as
//! the settings last made them, each live view's container, and each view's navigation machine.
//! The request filter (`blocking.rs`) asks [`Upgrades::document`] for every main-frame request;
//! the host feeds the failures and Continue on the UI thread. One lock, held for a lookup and
//! one transition: nothing waits on the UI thread, and nothing here may panic (poison-tolerant
//! locks, no indexing).

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, MutexGuard, PoisonError};

use wubble_core::https::{self, HttpsMode, Upgrade, UpgradeEffect, UpgradeEvent};
use wubble_core::zoom::ZoomTable;

struct Inner {
    mode: HttpsMode,
    /// `settings::PRIVACY_HTTPS_ALLOWED`: `container/host` to the Unix second it ends.
    allowed: ZoomTable,
    /// The same for private containers, never stored: for this run only.
    session: BTreeSet<String>,
    /// Each live view's container, by browser id: a view not here is never upgraded.
    containers: HashMap<i32, u32>,
    navs: HashMap<i32, Upgrade>,
}

pub struct Upgrades(Mutex<Inner>);

fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Default for Upgrades {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            mode: HttpsMode::Upgrade,
            allowed: ZoomTable::new(),
            session: BTreeSet::new(),
            containers: HashMap::new(),
            navs: HashMap::new(),
        }))
    }
}

impl Upgrades {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The settings' mode and allowlist (`apply::Applied::Https`).
    pub fn set(&self, mode: HttpsMode, allowed: ZoomTable) {
        let mut inner = self.inner();
        inner.mode = mode;
        inner.allowed = allowed;
    }

    /// The stored allowlist, for the host to extend.
    pub fn allowed(&self) -> ZoomTable {
        self.inner().allowed.clone()
    }

    /// Allows `key` (`https::allow_key`) for this run only: a private container's.
    pub fn allow_for_session(&self, key: String) {
        self.inner().session.insert(key);
    }

    /// The live views and their containers; the navigations of views gone are dropped.
    pub fn keep_views(&self, live: &[(i32, u32)]) {
        let mut inner = self.inner();
        inner.containers = live.iter().copied().collect();
        let Inner {
            containers, navs, ..
        } = &mut *inner;
        navs.retain(|browser, _| containers.contains_key(browser));
    }

    /// A main-frame request of view `browser_id` for `url` (CEF's IO thread): the `https://`
    /// URL to load instead, if it is upgraded.
    pub fn document(&self, browser_id: i32, url: &str) -> Option<String> {
        let mut inner = self.inner();
        let container = *inner.containers.get(&browser_id)?;
        let allowed = https::host_of(url).is_some_and(|host| {
            let key = https::allow_key(container, &host);
            inner.session.contains(&key) || https::allowed(&inner.allowed, &key, now_s())
        });
        let plan = https::plan(url, inner.mode, allowed);
        let event = UpgradeEvent::Document {
            url: url.to_owned(),
            plan,
        };
        Self::step(&mut inner, browser_id, event)
            .into_iter()
            .find_map(|e| match e {
                UpgradeEffect::Redirect(to) => Some(to),
                _ => None,
            })
    }

    /// Whether a failed load of `url` in view `browser_id` is the upgrade's
    /// ([`Self::failed`] takes it).
    pub fn owns(&self, browser_id: i32, url: &str) -> bool {
        self.inner()
            .navs
            .get(&browser_id)
            .is_some_and(|nav| nav.owns_failure(url))
    }

    /// The load of `url` in view `browser_id` failed: the upgrade's effects when the failure
    /// is its own, `None` when another panel shows it.
    pub fn failed(&self, browser_id: i32, url: &str) -> Option<Vec<UpgradeEffect>> {
        let mut inner = self.inner();
        if !inner.navs.get(&browser_id)?.owns_failure(url) {
            return None;
        }
        let event = UpgradeEvent::Failed {
            url: url.to_owned(),
        };
        Some(Self::step(&mut inner, browser_id, event))
    }

    /// The panel's Continue to site in view `browser_id`.
    pub fn continue_to_site(&self, browser_id: i32) -> Vec<UpgradeEffect> {
        let mut inner = self.inner();
        Self::step(&mut inner, browser_id, UpgradeEvent::Continue)
    }

    /// One transition of the view's machine, under the lock already held.
    fn step(inner: &mut Inner, browser_id: i32, event: UpgradeEvent) -> Vec<UpgradeEffect> {
        let state = inner.navs.remove(&browser_id).unwrap_or_default();
        let (next, effects) = https::transition(state, event, inner.mode);
        if next != Upgrade::Idle {
            inner.navs.insert(browser_id, next);
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_views_are_upgraded_and_allowed_hosts_are_not() {
        let u = Upgrades::default();
        assert_eq!(u.document(7, "http://a.test/"), None, "an unknown view");
        u.keep_views(&[(7, 2), (8, 3)]);
        assert_eq!(
            u.document(7, "http://a.test/").as_deref(),
            Some("https://a.test/")
        );
        // the redirected request passes
        assert_eq!(u.document(7, "https://a.test/"), None);
        u.allow_for_session(https::allow_key(3, "a.test"));
        assert_eq!(
            u.document(8, "http://a.test/"),
            None,
            "allowed in container 3"
        );
        u.set(HttpsMode::Off, ZoomTable::new());
        assert_eq!(u.document(7, "http://b.test/"), None, "off");
    }

    #[test]
    fn a_failed_upgrade_is_owned_until_the_view_goes() {
        let u = Upgrades::default();
        u.keep_views(&[(7, 0)]);
        u.document(7, "http://a.test/");
        assert_eq!(
            u.failed(7, "https://b.test/"),
            None,
            "another page's failure"
        );
        assert_eq!(
            u.failed(7, "https://a.test/"),
            Some(vec![UpgradeEffect::LoadHttp("http://a.test/".into())])
        );
        // the fallback's request stays on http
        assert_eq!(u.document(7, "http://a.test/"), None);
        u.keep_views(&[]);
        assert_eq!(u.failed(7, "https://a.test/"), None, "the view went");
    }
}
