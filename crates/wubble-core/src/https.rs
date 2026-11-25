//! HTTPS-Upgrades and HTTPS-Only mode, Chrome's "Always use secure connections": an `http://`
//! navigation is sent to `https://` first. In upgrade mode (Chrome's automatic HTTPS-Upgrades,
//! the default) a failed upgrade loads the `http://` page silently; in only mode it shows
//! "Connection is not secure" with Go back and Continue to site, and Continue remembers the
//! site for its container ([`ALLOW_SECS`], as Chrome does). The upgrade adds no request to a
//! new party: the same host is asked over another scheme.
//!
//! [`plan`] decides one request; [`transition`] is one tab's navigation as a machine, run under
//! the host's lock from CEF's IO thread (the request) and the UI thread (the failure, Continue).
//! Behaviour from Chromium `chrome/browser/ssl/https_upgrades_interceptor.cc`,
//! `https_only_mode_tab_helper.cc` and `components/security_interstitials/content/
//! https_only_mode_blocking_page.cc` (exempt hosts, fallback, the interstitial's wording, the
//! allowlist's 15 days), BSD-3-Clause; no code copied.

use std::collections::BTreeMap;

/// `settings::PRIVACY_HTTPS_MODE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpsMode {
    /// Pages load over the scheme they ask for.
    Off,
    /// Try https first; fall back to http without asking (Chrome's default).
    Upgrade,
    /// Try https first; ask before loading http.
    Only,
}

impl HttpsMode {
    /// The stored words, in the variants' order.
    pub const WORDS: [&'static str; 3] = ["off", "upgrade", "only"];
    const ALL: [HttpsMode; 3] = [Self::Off, Self::Upgrade, Self::Only];

    pub fn word(self) -> &'static str {
        Self::WORDS[self as usize]
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.word() == word)
    }
}

/// How long Continue to site keeps a host on http for its container: Chrome's 15 days.
pub const ALLOW_SECS: u64 = 15 * 24 * 60 * 60;

/// The allowlist's key for `host` in `container` (`settings::PRIVACY_HTTPS_ALLOWED`'s keys).
pub fn allow_key(container: u32, host: &str) -> String {
    format!("{container}/{host}")
}

/// Whether `table` (key to expiry, Unix seconds) still allows `key` at `now_s`.
pub fn allowed(table: &BTreeMap<String, u32>, key: &str, now_s: u64) -> bool {
    table
        .get(key)
        .is_some_and(|&until| u64::from(until) > now_s)
}

/// `table` with `key` allowed from `now_s` for [`ALLOW_SECS`], and expired keys dropped.
pub fn allow(table: &BTreeMap<String, u32>, key: &str, now_s: u64) -> BTreeMap<String, u32> {
    let until = u32::try_from(now_s.saturating_add(ALLOW_SECS)).unwrap_or(u32::MAX);
    let mut next: BTreeMap<String, u32> = table
        .iter()
        .filter(|(_, until)| u64::from(**until) > now_s)
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    next.insert(key.to_owned(), until);
    next
}

/// What one main-frame request does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Load this `https://` URL instead.
    Upgrade(String),
    Keep,
}

/// Hosts Chrome never upgrades: IP literals, `localhost` and its subdomains, single labels
/// (intranet names), `.local` (mDNS), and any URL with a port of its own.
fn exempt(url: &url::Url) -> bool {
    let host = match url.host() {
        Some(url::Host::Domain(h)) => h.trim_end_matches('.').to_ascii_lowercase(),
        // an IP literal, or no host
        _ => return true,
    };
    url.port().is_some()
        || !host.contains('.')
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
}

/// The request to `url` in `mode`; `allowed`: the user chose http for this host (its
/// container's allowlist) or this navigation is a fallback.
pub fn plan(url: &str, mode: HttpsMode, allowed: bool) -> Plan {
    if mode == HttpsMode::Off || allowed {
        return Plan::Keep;
    }
    let Ok(mut parsed) = url::Url::parse(url) else {
        return Plan::Keep;
    };
    if parsed.scheme() != "http" || exempt(&parsed) || parsed.set_scheme("https").is_err() {
        return Plan::Keep;
    }
    Plan::Upgrade(parsed.into())
}

/// The host of `url`, as the allowlist keys it.
pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(|h| h.trim_end_matches('.').to_ascii_lowercase())
}

/// One tab's navigation, as far as upgrades go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Upgrade {
    /// The last navigation was not upgraded.
    #[default]
    Idle,
    /// The navigation to `http` went to `https` instead.
    Upgraded { http: String, https: String },
    /// The upgrade failed in upgrade mode: `http` loads, and is not upgraded again while this
    /// lasts (a reload of it too).
    Fallback { http: String, https: String },
    /// The upgrade failed in only mode: the panel asks.
    Warn { http: String, https: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeEvent {
    /// A main-frame request for `url`, which [`plan`] answered with `plan` (its allowlist read).
    Document { url: String, plan: Plan },
    /// The load of `url` failed: a network or certificate error.
    Failed { url: String },
    /// The panel's Continue to site.
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeEffect {
    /// The request goes to this URL instead.
    Redirect(String),
    /// Load this `http://` URL in the tab.
    LoadHttp(String),
    /// Show "Connection is not secure" for `http`, whose upgrade `https` failed.
    Warn { http: String, https: String },
    /// Remember `host` on http for the tab's container ([`allow`]).
    Allow(String),
}

/// One step. The event's failure is the machine's own (no other panel shows it) exactly when
/// the state names its URL as the upgrade's. Every transition is a row here: add behaviour as a
/// state or a row, not as a branch at a caller.
pub fn transition(
    state: Upgrade,
    event: UpgradeEvent,
    mode: HttpsMode,
) -> (Upgrade, Vec<UpgradeEffect>) {
    use Upgrade as S;
    use UpgradeEffect as Fx;
    use UpgradeEvent as E;
    match (state, event) {
        // the upgrade's own request, as the redirect sends it
        (s @ S::Upgraded { .. }, E::Document { url, .. }) if s.https() == Some(&url) => (s, vec![]),
        // the fallback (and a reload of it) stays on http
        (s @ S::Fallback { .. }, E::Document { url, .. }) if s.http() == Some(&url) => (s, vec![]),
        (_, E::Document { url, plan }) => match plan {
            Plan::Upgrade(https) => (
                S::Upgraded {
                    http: url,
                    https: https.clone(),
                },
                vec![Fx::Redirect(https)],
            ),
            Plan::Keep => (S::Idle, vec![]),
        },
        (S::Upgraded { http, https }, E::Failed { url }) if url == https => match mode {
            HttpsMode::Only => (
                S::Warn {
                    http: http.clone(),
                    https: https.clone(),
                },
                vec![Fx::Warn { http, https }],
            ),
            // the mode was turned off meanwhile: the page the user asked for
            HttpsMode::Upgrade | HttpsMode::Off => (
                S::Fallback {
                    http: http.clone(),
                    https,
                },
                vec![Fx::LoadHttp(http)],
            ),
        },
        // the https load's late failure (the refused certificate's own error): already handled
        (s @ (S::Fallback { .. } | S::Warn { .. }), E::Failed { url })
            if s.https() == Some(&url) =>
        {
            (s, vec![])
        }
        (S::Warn { http, https }, E::Continue) => {
            let allow = host_of(&http).map(Fx::Allow);
            let effects: Vec<_> = allow
                .into_iter()
                .chain([Fx::LoadHttp(http.clone())])
                .collect();
            (S::Fallback { http, https }, effects)
        }
        // a failure of another URL is another panel's; Continue without a warning does nothing
        (s, E::Failed { .. } | E::Continue) => (s, vec![]),
    }
}

impl Upgrade {
    fn https(&self) -> Option<&String> {
        match self {
            Self::Upgraded { https, .. }
            | Self::Fallback { https, .. }
            | Self::Warn { https, .. } => Some(https),
            Self::Idle => None,
        }
    }

    fn http(&self) -> Option<&String> {
        match self {
            Self::Upgraded { http, .. } | Self::Fallback { http, .. } | Self::Warn { http, .. } => {
                Some(http)
            }
            Self::Idle => None,
        }
    }

    /// Whether a failed load of `url` is this machine's to handle ([`UpgradeEvent::Failed`]
    /// changes or keeps the state without another panel).
    pub fn owns_failure(&self, url: &str) -> bool {
        self.https().is_some_and(|h| h == url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_follow_chromes_exemptions() {
        use HttpsMode::{Off, Only, Upgrade};
        let up = |u: &str| Plan::Upgrade(u.into());
        let cases: &[(&str, HttpsMode, bool, Plan)] = &[
            (
                "http://example.com/a?b=1",
                Upgrade,
                false,
                up("https://example.com/a?b=1"),
            ),
            (
                "http://example.com/",
                Only,
                false,
                up("https://example.com/"),
            ),
            ("http://example.com/", Off, false, Plan::Keep),
            ("http://example.com/", Upgrade, true, Plan::Keep),
            ("https://example.com/", Upgrade, false, Plan::Keep),
            ("ftp://example.com/", Upgrade, false, Plan::Keep),
            ("http://192.168.1.1/", Upgrade, false, Plan::Keep),
            ("http://[::1]/", Upgrade, false, Plan::Keep),
            ("http://localhost/", Upgrade, false, Plan::Keep),
            ("http://app.localhost/", Upgrade, false, Plan::Keep),
            ("http://intranet/", Upgrade, false, Plan::Keep),
            ("http://printer.local/", Upgrade, false, Plan::Keep),
            ("http://example.com:8080/", Upgrade, false, Plan::Keep),
            // the default port is no port of its own
            (
                "http://example.com:80/",
                Upgrade,
                false,
                up("https://example.com/"),
            ),
            ("not a url", Upgrade, false, Plan::Keep),
        ];
        for (url, mode, allowed, want) in cases {
            assert_eq!(
                &plan(url, *mode, *allowed),
                want,
                "{url} {mode:?} {allowed}"
            );
        }
    }

    #[test]
    fn modes_are_words() {
        for word in HttpsMode::WORDS {
            assert_eq!(HttpsMode::from_word(word).map(HttpsMode::word), Some(word));
        }
        assert_eq!(HttpsMode::from_word("always"), None);
    }

    #[test]
    fn the_allowlist_keeps_a_host_for_fifteen_days_per_container() {
        let now = 1_000_000;
        let key = allow_key(3, "a.test");
        assert_eq!(key, "3/a.test");
        let table = allow(&BTreeMap::new(), &key, now);
        assert!(allowed(&table, &key, now));
        assert!(allowed(&table, &key, now + ALLOW_SECS - 1));
        assert!(!allowed(&table, &key, now + ALLOW_SECS));
        assert!(
            !allowed(&table, &allow_key(4, "a.test"), now),
            "another container"
        );
        // an expired entry goes when another is added
        let later = allow(&table, "3/b.test", now + ALLOW_SECS + 1);
        assert_eq!(later.keys().collect::<Vec<_>>(), ["3/b.test"]);
    }

    const HTTP: &str = "http://a.test/";
    const HTTPS: &str = "https://a.test/";

    fn upgraded() -> Upgrade {
        Upgrade::Upgraded {
            http: HTTP.into(),
            https: HTTPS.into(),
        }
    }
    fn fallback() -> Upgrade {
        Upgrade::Fallback {
            http: HTTP.into(),
            https: HTTPS.into(),
        }
    }
    fn warn() -> Upgrade {
        Upgrade::Warn {
            http: HTTP.into(),
            https: HTTPS.into(),
        }
    }
    fn doc(url: &str, plan: Plan) -> UpgradeEvent {
        UpgradeEvent::Document {
            url: url.into(),
            plan,
        }
    }
    fn failed(url: &str) -> UpgradeEvent {
        UpgradeEvent::Failed { url: url.into() }
    }

    /// One row per (state, event): what it becomes and does.
    #[test]
    #[rustfmt::skip]
    fn a_navigation_upgrades_and_falls_back_or_warns() {
        use HttpsMode::{Off, Only, Upgrade as Up};
        use UpgradeEffect as Fx;
        let to_https = || Plan::Upgrade(HTTPS.into());
        let cases: Vec<(&str, Upgrade, UpgradeEvent, HttpsMode, Upgrade, Vec<UpgradeEffect>)> = vec![
            ("an http page is upgraded", Upgrade::Idle, doc(HTTP, to_https()), Up,
                upgraded(), vec![Fx::Redirect(HTTPS.into())]),
            ("its redirected request passes", upgraded(), doc(HTTPS, Plan::Keep), Up, upgraded(), vec![]),
            ("a page not upgraded", upgraded(), doc("https://b.test/", Plan::Keep), Up,
                Upgrade::Idle, vec![]),
            ("upgrade mode: the failure loads http", upgraded(), failed(HTTPS), Up,
                fallback(), vec![Fx::LoadHttp(HTTP.into())]),
            ("only mode: the failure warns", upgraded(), failed(HTTPS), Only,
                warn(), vec![Fx::Warn { http: HTTP.into(), https: HTTPS.into() }]),
            ("turned off meanwhile: http", upgraded(), failed(HTTPS), Off,
                fallback(), vec![Fx::LoadHttp(HTTP.into())]),
            ("another URL's failure is not ours", upgraded(), failed("https://b.test/"), Up,
                upgraded(), vec![]),
            ("the fallback's request stays on http", fallback(), doc(HTTP, to_https()), Up,
                fallback(), vec![]),
            ("the https load's late failure", fallback(), failed(HTTPS), Up, fallback(), vec![]),
            ("the http page's own failure is the error panel's", fallback(), failed(HTTP), Up,
                fallback(), vec![]),
            ("a new page after the fallback", fallback(), doc("http://b.test/",
                Plan::Upgrade("https://b.test/".into())), Up,
                Upgrade::Upgraded { http: "http://b.test/".into(), https: "https://b.test/".into() },
                vec![Fx::Redirect("https://b.test/".into())]),
            ("Continue: remembered, and http loads", warn(), UpgradeEvent::Continue, Only,
                fallback(), vec![Fx::Allow("a.test".into()), Fx::LoadHttp(HTTP.into())]),
            ("the warning's late failure", warn(), failed(HTTPS), Only, warn(), vec![]),
            ("Go back leaves for another page", warn(), doc("https://b.test/", Plan::Keep), Only,
                Upgrade::Idle, vec![]),
            ("Continue without a warning", upgraded(), UpgradeEvent::Continue, Only, upgraded(), vec![]),
            ("idle: a failure is not ours", Upgrade::Idle, failed(HTTPS), Up, Upgrade::Idle, vec![]),
            ("idle: Continue does nothing", Upgrade::Idle, UpgradeEvent::Continue, Only,
                Upgrade::Idle, vec![]),
        ];
        for (name, state, event, mode, want, effects) in cases {
            assert_eq!(transition(state, event, mode), (want, effects), "{name}");
        }
    }

    #[test]
    fn only_the_upgrade_owns_its_failure() {
        assert!(upgraded().owns_failure(HTTPS));
        assert!(fallback().owns_failure(HTTPS));
        assert!(warn().owns_failure(HTTPS));
        assert!(!upgraded().owns_failure(HTTP));
        assert!(!Upgrade::Idle.owns_failure(HTTPS));
    }
}
