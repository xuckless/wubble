//! Site sound rules: the `sound.muted_sites` list (a host covers its subdomains), the tab menu's
//! "Mute Site" switch, and a tab's mute with its reason as one lifecycle ([`transition`]).
//!
//! Chromium's policy, ported as rules, not code (BSD-3-Clause): `chrome/browser/ui/tabs/
//! tab_strip_model.cc` `ExecuteContextMenuCommand(CommandToggleSiteMuted)` writes the
//! `ContentSettingsType::SOUND` exception for the tab's site, and
//! `chrome/browser/content_settings/sound_content_setting_observer.cc`
//! `MuteOrUnmuteIfNecessary` applies it on each committed main-frame navigation and on each
//! change of the setting: a blocked site mutes the tab (reason "content setting"); an allowed
//! one unmutes it only when the content setting was what muted it, never a user's own mute.

use crate::routing::{host_in, host_key};

/// Why a tab's page is muted, or that it is not. The host keeps one per tab with a view; the
/// view's mute follows from it (each [`transition`] says when to change it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mute {
    #[default]
    Unmuted,
    /// The user muted the tab (its menu row, the badge, the key): a site rule leaves it alone.
    ByUser,
    /// Its site is on the muted list.
    BySite,
}

impl Mute {
    pub fn muted(self) -> bool {
        self != Mute::Unmuted
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteEvent {
    /// The user flipped the tab's mute.
    Toggled,
    /// The tab committed a page, or the list changed: whether the page's site is muted now.
    Site(bool),
}

/// The next mute and, when the view's mute must change, its new value.
pub fn transition(state: Mute, event: MuteEvent) -> (Mute, Option<bool>) {
    match (state, event) {
        (Mute::Unmuted, MuteEvent::Toggled) => (Mute::ByUser, Some(true)),
        (Mute::ByUser | Mute::BySite, MuteEvent::Toggled) => (Mute::Unmuted, Some(false)),
        (Mute::Unmuted, MuteEvent::Site(true)) => (Mute::BySite, Some(true)),
        (Mute::BySite, MuteEvent::Site(false)) => (Mute::Unmuted, Some(false)),
        (s @ (Mute::Unmuted | Mute::BySite), MuteEvent::Site(_)) => (s, None),
        (Mute::ByUser, MuteEvent::Site(_)) => (Mute::ByUser, None),
    }
}

/// Whether `url`'s site is muted by `sites`.
pub fn is_muted<S: AsRef<str>>(sites: &[S], url: &str) -> bool {
    !sites.is_empty() && host_in(sites, url)
}

/// What the menu's switch does to the list for a page at `url`: take off the entry that covers
/// it (its own host, or a parent's: `example.com` for `docs.example.com`), or add its host.
/// `None` for a page that is not on the web (no http or https host).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Toggle {
    Remove(String),
    Add(String),
}

pub fn toggle<S: AsRef<str>>(sites: &[S], url: &str) -> Option<Toggle> {
    let host = web_host(url)?;
    Some(
        match sites
            .iter()
            .find(|s| host_in(&[s.as_ref()], &host))
            .map(|s| s.as_ref().to_owned())
        {
            Some(entry) => Toggle::Remove(entry),
            None => Toggle::Add(host),
        },
    )
}

/// `url`'s host when it is a web page (http or https): the only pages a site rule names.
pub fn web_host(url: &str) -> Option<String> {
    let parsed = ::url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    host_key(url)
}

/// The entries of `new` not in `old` (to block) and of `old` not in `new` (back to the
/// default): what changes in each container's sound content setting.
pub fn diff(old: &[String], new: &[String]) -> (Vec<String>, Vec<String>) {
    let added = new.iter().filter(|s| !old.contains(s)).cloned().collect();
    let removed = old.iter().filter(|s| !new.contains(s)).cloned().collect();
    (added, removed)
}

/// The URLs a site's content-setting exception is written for: CEF's `SetContentSetting` takes
/// a URL, and Chromium scopes the sound exception to its origin, so both schemes are named.
pub fn origins(site: &str) -> [String; 2] {
    [format!("https://{site}/"), format!("http://{site}/")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_rule_mutes_and_unmutes_only_what_it_muted() {
        let (s, set) = transition(Mute::Unmuted, MuteEvent::Site(true));
        assert_eq!((s, set), (Mute::BySite, Some(true)));
        assert_eq!(transition(s, MuteEvent::Site(true)), (Mute::BySite, None));
        assert_eq!(
            transition(s, MuteEvent::Site(false)),
            (Mute::Unmuted, Some(false))
        );
        // the user's own mute stays whatever the site says
        assert_eq!(
            transition(Mute::ByUser, MuteEvent::Site(false)),
            (Mute::ByUser, None)
        );
        assert_eq!(
            transition(Mute::ByUser, MuteEvent::Site(true)),
            (Mute::ByUser, None)
        );
        assert_eq!(
            transition(Mute::Unmuted, MuteEvent::Site(false)),
            (Mute::Unmuted, None)
        );
    }

    #[test]
    fn the_user_flips_either_mute() {
        assert_eq!(
            transition(Mute::Unmuted, MuteEvent::Toggled),
            (Mute::ByUser, Some(true))
        );
        assert_eq!(
            transition(Mute::ByUser, MuteEvent::Toggled),
            (Mute::Unmuted, Some(false))
        );
        // unmuting a muted site's tab holds until its next page of the site
        let (s, set) = transition(Mute::BySite, MuteEvent::Toggled);
        assert_eq!((s, set), (Mute::Unmuted, Some(false)));
        assert_eq!(
            transition(s, MuteEvent::Site(true)),
            (Mute::BySite, Some(true))
        );
        assert!(Mute::BySite.muted() && Mute::ByUser.muted() && !Mute::Unmuted.muted());
    }

    #[test]
    fn a_host_covers_its_subdomains() {
        let sites = vec!["example.com".to_string()];
        assert!(is_muted(&sites, "https://docs.example.com/a"));
        assert!(is_muted(&sites, "http://example.com/"));
        assert!(!is_muted(&sites, "https://example.org/"));
        assert!(!is_muted::<String>(&[], "https://example.com/"));
    }

    #[test]
    fn the_switch_takes_off_the_covering_entry_or_adds_the_host() {
        let sites = vec!["example.com".to_string()];
        assert_eq!(
            toggle(&sites, "https://docs.example.com/x"),
            Some(Toggle::Remove("example.com".into()))
        );
        assert_eq!(
            toggle(&sites, "https://Music.Test/x"),
            Some(Toggle::Add("music.test".into()))
        );
        assert_eq!(toggle(&sites, "about:blank"), None);
        assert_eq!(toggle(&sites, "wubble://settings"), None);
    }

    #[test]
    fn diff_names_what_to_block_and_what_to_clear() {
        let old = vec!["a.test".to_string(), "b.test".to_string()];
        let new = vec!["b.test".to_string(), "c.test".to_string()];
        assert_eq!(
            diff(&old, &new),
            (vec!["c.test".to_string()], vec!["a.test".to_string()])
        );
        assert_eq!(
            origins("a.test"),
            ["https://a.test/".to_string(), "http://a.test/".to_string()]
        );
    }
}
