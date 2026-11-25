//! Site permissions: what a page may ask for, and what the user decided per
//! site. One table, origin x kind -> Allow or Block, the one owner of
//! remembered decisions (the store is its persistence). Nothing is granted
//! by default: no row means ask.
//!
//! Design reference, no code copied: Chromium `components/content_settings`
//! (a setting per primary origin and content type, ALLOW / BLOCK / ASK),
//! BSD-3-Clause, and Firefox `nsIPermissionManager` (origin x type -> allow /
//! deny), MPL-2.0. Prompt wording is Chromium's
//! `components/permissions/permission_util.cc` / `permission_request.cc`
//! strings (BSD-3-Clause).

use std::collections::BTreeMap;

use crate::pagetheme::{PageTheme, ThemeTable};
use crate::zoom::ZoomTable;

/// What a page can be granted through the prompt. The engine's kinds that
/// are not here (protected media, desktop capture, unknown bits) are never
/// prompted for: a request naming one is denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    Camera,
    Microphone,
    Geolocation,
    Notifications,
    Clipboard,
    MidiSysex,
    IdleDetection,
    LocalFonts,
    StorageAccess,
    KeyboardLock,
    PointerLock,
    /// Offering to save passwords; only `Block` is ever stored ("Never for
    /// this site" on the save prompt). No page asks for it.
    SavePasswords,
    /// Opening windows and tabs without a click (`popup::decide`). Asked through the address
    /// pill's notice, not the permission prompt.
    Popups,
    /// Handing links of one scheme (`mailto:`, `tel:`) to another app. Remembered per site and
    /// scheme, under the origin `external::key` makes (`mailto:https://a.test`).
    ExternalLinks,
    /// Downloading files: asked on a site's first download and on any download no click asked
    /// for (`click_guard::download`), Safari's "Allow downloads on <site>?".
    Downloads,
}

impl Permission {
    pub const ALL: [Permission; 15] = [
        Self::Camera,
        Self::Microphone,
        Self::Geolocation,
        Self::Notifications,
        Self::Clipboard,
        Self::MidiSysex,
        Self::IdleDetection,
        Self::LocalFonts,
        Self::StorageAccess,
        Self::KeyboardLock,
        Self::PointerLock,
        Self::SavePasswords,
        Self::Popups,
        Self::ExternalLinks,
        Self::Downloads,
    ];

    /// The stored name. Stable: never rename one.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Microphone => "microphone",
            Self::Geolocation => "geolocation",
            Self::Notifications => "notifications",
            Self::Clipboard => "clipboard",
            Self::MidiSysex => "midi-sysex",
            Self::IdleDetection => "idle-detection",
            Self::LocalFonts => "local-fonts",
            Self::StorageAccess => "storage-access",
            Self::KeyboardLock => "keyboard-lock",
            Self::PointerLock => "pointer-lock",
            Self::SavePasswords => "password-save",
            Self::Popups => "popups",
            Self::ExternalLinks => "external-links",
            Self::Downloads => "downloads",
        }
    }

    pub fn parse(name: &str) -> Option<Permission> {
        Self::ALL.into_iter().find(|p| p.as_str() == name)
    }

    /// What the prompt says the site wants to do (Chromium's strings).
    pub fn ask_text(self) -> &'static str {
        match self {
            Self::Camera => "Use your camera",
            Self::Microphone => "Use your microphone",
            Self::Geolocation => "Know your location",
            Self::Notifications => "Show notifications",
            Self::Clipboard => "See text and images copied to the clipboard",
            Self::MidiSysex => "Control and reprogram your MIDI devices",
            Self::IdleDetection => "Know when you're actively using this device",
            Self::LocalFonts => "Use the fonts on your computer",
            Self::StorageAccess => "Use info it has saved about you",
            Self::KeyboardLock => "See and control your keyboard",
            Self::PointerLock => "Use and move your mouse",
            Self::SavePasswords => "Save your passwords",
            Self::Popups => "Open pop-ups and new tabs",
            Self::ExternalLinks => "Open links in another app",
            Self::Downloads => "Download files",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
}

/// What a request gets before anyone is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Grant,
    Deny,
    Ask,
}

/// Remembered decisions, origin x kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SitePermissions(BTreeMap<(String, Permission), Decision>);

impl SitePermissions {
    pub fn set(&mut self, origin: &str, kind: Permission, decision: Decision) {
        self.0.insert((origin.to_string(), kind), decision);
    }

    pub fn forget(&mut self, origin: &str, kind: Permission) {
        self.0.remove(&(origin.to_string(), kind));
    }

    /// Every remembered decision, by origin then kind.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Permission, Decision)> {
        self.0.iter().map(|((o, k), d)| (o.as_str(), *k, *d))
    }

    pub fn get(&self, origin: &str, kind: Permission) -> Option<Decision> {
        self.0.get(&(origin.to_string(), kind)).copied()
    }

    /// A request from `origin` (`None`: an opaque origin, never remembered)
    /// for every kind in `asked`. Any kind blocked: deny; every kind
    /// allowed: grant; otherwise ask (a request is answered whole, so a
    /// camera allowed and a microphone not yet decided still asks). Asking
    /// for nothing grants nothing.
    pub fn decide(&self, origin: Option<&str>, asked: &[Permission]) -> Verdict {
        if asked.is_empty() {
            return Verdict::Deny;
        }
        let Some(origin) = origin else {
            return Verdict::Ask;
        };
        let decisions: Vec<_> = asked.iter().map(|&k| self.get(origin, k)).collect();
        if decisions.contains(&Some(Decision::Block)) {
            Verdict::Deny
        } else if decisions.iter().all(|d| *d == Some(Decision::Allow)) {
            Verdict::Grant
        } else {
            Verdict::Ask
        }
    }
}

/// Every per-site table as stored (the store and the settings): what a normal tab reads and
/// writes, and what a private tab reads under its own choices.
#[derive(Clone, Copy, Debug)]
pub struct Stored<'a> {
    pub permissions: &'a SitePermissions,
    pub zoom: &'a ZoomTable,
    pub zoom_default: u32,
    pub themes: &'a ThemeTable,
    pub theme_default: PageTheme,
    /// `settings::BLOCKER_EXCEPTIONS`'s hosts.
    pub blocker: &'a [String],
    /// `settings::SLEEP_NEVER_SITES`'s sites.
    pub never_sleep: &'a [String],
}

/// Hosts a private tab switched on (`true`) or off in a stored list, where that differs.
pub type Toggles = BTreeMap<String, bool>;

/// `stored` with `over`'s switches applied.
pub fn toggled(stored: &[String], over: &Toggles) -> Vec<String> {
    let mut out: Vec<String> = stored
        .iter()
        .filter(|h| over.get(*h) != Some(&false))
        .cloned()
        .collect();
    out.extend(
        over.iter()
            .filter(|&(h, on)| *on && !stored.contains(h))
            .map(|(h, _)| h.clone()),
    );
    out
}

/// One per-site change, as the one write (`Host::write_site`) takes it: to the stored tables
/// from a normal tab, to [`PrivateChoices`] from a private one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SiteChange {
    Permission(String, Permission, Decision),
    /// The site of `page` zoomed to `percent`.
    Zoom {
        page: String,
        percent: u32,
    },
    /// The site of `page` shown as `theme`.
    Theme {
        page: String,
        theme: PageTheme,
    },
    /// The host joins or leaves the blocker's exceptions.
    Blocker(String),
    /// The site joins or leaves the never-sleep list.
    NeverSleep(String),
}

/// The per-site choices made in private windows (permissions, Allow pop-ups, save-password
/// "Never", zoom, page theme, blocker exceptions, never-sleep sites): never stored, read over
/// the stored ones by private tabs only ([`SiteView`]), and dropped with the private windows'
/// in-memory container (Chrome's off-the-record `HostContentSettingsMap` and zoom map go with
/// the off-the-record profile).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivateChoices {
    pub permissions: SitePermissions,
    pub zoom: ZoomTable,
    pub themes: ThemeTable,
    pub blocker: Toggles,
    pub never_sleep: Toggles,
}

impl PrivateChoices {
    /// `change` made in a private tab, over `stored`: a row equal to the stored one goes.
    pub fn write(&mut self, stored: &Stored, change: SiteChange) {
        let flip = |over: &mut Toggles, list: &[String], host: String| {
            let on = !toggled(list, over).contains(&host);
            if on == list.contains(&host) {
                over.remove(&host);
            } else {
                over.insert(host, on);
            }
        };
        match change {
            SiteChange::Permission(site, kind, decision) => {
                self.permissions.set(&site, kind, decision);
            }
            SiteChange::Zoom { page, percent } => {
                if let Some(host) = crate::zoom::host_of(&page) {
                    let level = crate::zoom::wanted(stored.zoom, &page, stored.zoom_default);
                    let table = std::mem::take(&mut self.zoom);
                    self.zoom = crate::zoom::next_table(table, &host, percent, level);
                }
            }
            SiteChange::Theme { page, theme } => {
                if let Some(host) = crate::zoom::host_of(&page) {
                    let was = crate::pagetheme::wanted(stored.themes, &page, stored.theme_default);
                    let table = std::mem::take(&mut self.themes);
                    self.themes = crate::pagetheme::next_table(table, &host, theme, was);
                }
            }
            SiteChange::Blocker(host) => flip(&mut self.blocker, stored.blocker, host),
            SiteChange::NeverSleep(site) => flip(&mut self.never_sleep, stored.never_sleep, site),
        }
    }
}

/// One tab's reading of the per-site tables (`Host::site_view`): the stored ones, under a
/// private tab's own choices.
#[derive(Clone, Copy, Debug)]
pub struct SiteView<'a> {
    pub stored: Stored<'a>,
    /// `Some` for a private tab.
    pub private: Option<&'a PrivateChoices>,
}

impl SiteView<'_> {
    /// The remembered decisions. A private tab inherits only the stored Block decisions, and
    /// asks again where the stored answer is Allow (its answer stays private), as Chrome's
    /// off-the-record profile inherits only BLOCK content settings.
    pub fn permissions(&self) -> std::borrow::Cow<'_, SitePermissions> {
        let Some(private) = self.private else {
            return std::borrow::Cow::Borrowed(self.stored.permissions);
        };
        let mut out: SitePermissions = self
            .stored
            .permissions
            .iter()
            .filter(|&(.., d)| d == Decision::Block)
            .map(|(o, k, d)| (o.to_owned(), k, d))
            .collect();
        out.0
            .extend(private.permissions.0.iter().map(|(k, d)| (k.clone(), *d)));
        std::borrow::Cow::Owned(out)
    }

    pub fn zoom(&self, page: &str) -> u32 {
        let level = crate::zoom::wanted(self.stored.zoom, page, self.stored.zoom_default);
        self.private
            .map_or(level, |p| crate::zoom::wanted(&p.zoom, page, level))
    }

    /// The hosts the blocker skips: the stored list, with a private tab's switches.
    pub fn blocker(&self) -> Vec<String> {
        self.private.map_or_else(
            || self.stored.blocker.to_vec(),
            |p| toggled(self.stored.blocker, &p.blocker),
        )
    }

    /// The sites whose tabs never sleep: the stored list, with a private tab's switches.
    pub fn never_sleep(&self) -> Vec<String> {
        self.private.map_or_else(
            || self.stored.never_sleep.to_vec(),
            |p| toggled(self.stored.never_sleep, &p.never_sleep),
        )
    }

    pub fn theme(&self, page: &str) -> PageTheme {
        let theme = crate::pagetheme::wanted(self.stored.themes, page, self.stored.theme_default);
        self.private
            .map_or(theme, |p| crate::pagetheme::wanted(&p.themes, page, theme))
    }
}

impl FromIterator<(String, Permission, Decision)> for SitePermissions {
    fn from_iter<I: IntoIterator<Item = (String, Permission, Decision)>>(rows: I) -> Self {
        Self(rows.into_iter().map(|(o, k, d)| ((o, k), d)).collect())
    }
}

/// The origin decisions are remembered under: `scheme://host[:port]`,
/// lowercase, default ports dropped. `None` for opaque origins (`data:`,
/// `about:`, `file:`), whose requests are always asked and never stored.
pub fn origin_of(page: &str) -> Option<String> {
    let parsed = url::Url::parse(page).ok()?;
    let origin = parsed.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permission::*;

    #[test]
    fn names_round_trip_and_are_unique() {
        for p in Permission::ALL {
            assert_eq!(Permission::parse(p.as_str()), Some(p));
            assert!(!p.ask_text().is_empty());
        }
        let mut names: Vec<_> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Permission::ALL.len());
        assert_eq!(Permission::parse("Camera"), None);
        assert_eq!(Permission::parse(""), None);
    }

    #[test]
    fn decide_table() {
        let o = "https://a.test";
        let mut t = SitePermissions::default();
        assert_eq!(t.decide(Some(o), &[Camera]), Verdict::Ask, "no row asks");
        assert_eq!(t.decide(Some(o), &[]), Verdict::Deny, "nothing asked");
        t.set(o, Camera, Decision::Allow);
        assert_eq!(t.decide(Some(o), &[Camera]), Verdict::Grant);
        assert_eq!(
            t.decide(Some(o), &[Camera, Microphone]),
            Verdict::Ask,
            "partly decided asks"
        );
        t.set(o, Microphone, Decision::Allow);
        assert_eq!(t.decide(Some(o), &[Camera, Microphone]), Verdict::Grant);
        t.set(o, Microphone, Decision::Block);
        assert_eq!(t.decide(Some(o), &[Camera, Microphone]), Verdict::Deny);
        assert_eq!(t.decide(Some(o), &[Geolocation, Microphone]), Verdict::Deny);
        // Another origin and an opaque one are not affected.
        assert_eq!(t.decide(Some("https://b.test"), &[Camera]), Verdict::Ask);
        assert_eq!(t.decide(None, &[Camera]), Verdict::Ask);
        t.forget(o, Microphone);
        assert_eq!(t.get(o, Microphone), None);
        assert_eq!(t.decide(Some(o), &[Camera]), Verdict::Grant);
    }

    #[test]
    fn a_private_tab_reads_stored_blocks_and_its_own_choices_only() {
        let (a, b) = ("https://a.test", "https://b.test");
        let stored: SitePermissions = [
            (a.to_string(), Popups, Decision::Block),
            (b.to_string(), Camera, Decision::Allow),
        ]
        .into_iter()
        .collect();
        let (zoom, themes) = (ZoomTable::new(), ThemeTable::new());
        let tables = Stored {
            permissions: &stored,
            zoom: &zoom,
            zoom_default: 100,
            themes: &themes,
            theme_default: PageTheme::Auto,
            blocker: &[],
            never_sleep: &[],
        };
        let mut private = PrivateChoices::default();
        private.write(
            &tables,
            SiteChange::Permission(b.into(), SavePasswords, Decision::Block),
        );
        private.write(
            &tables,
            SiteChange::Permission(a.into(), Microphone, Decision::Allow),
        );
        let in_private = SiteView {
            stored: tables,
            private: Some(&private),
        };
        let in_normal = SiteView {
            stored: tables,
            private: None,
        };
        // (origin, kind, what a private tab reads, what a normal tab reads)
        let cases = [
            (a, Popups, Some(Decision::Block), Some(Decision::Block)),
            // a stored Allow is asked again in private
            (b, Camera, None, Some(Decision::Allow)),
            (b, SavePasswords, Some(Decision::Block), None),
            (a, Microphone, Some(Decision::Allow), None),
            (a, Camera, None, None),
        ];
        for (origin, kind, private_reads, normal_reads) in cases {
            assert_eq!(
                in_private.permissions().get(origin, kind),
                private_reads,
                "{origin} {kind:?}"
            );
            assert_eq!(
                in_normal.permissions().get(origin, kind),
                normal_reads,
                "{origin} {kind:?}"
            );
        }
        // nothing private reaches the stored table
        assert_eq!(stored.iter().count(), 2);
    }

    /// Every per-site change from a private tab lands in its own table only, a change back to
    /// the stored value drops the row, and a normal tab reads none of it.
    #[test]
    fn private_site_changes_stay_private_and_fold_back_to_the_stored_value() {
        let page = "https://a.test/x";
        let stored_perms = SitePermissions::default();
        let zoom: ZoomTable = [("a.test".to_string(), 150)].into_iter().collect();
        let themes = ThemeTable::new();
        let blocker = vec!["a.test".to_string()];
        let never = vec![];
        let stored = Stored {
            permissions: &stored_perms,
            zoom: &zoom,
            zoom_default: 100,
            themes: &themes,
            theme_default: PageTheme::Auto,
            blocker: &blocker,
            never_sleep: &never,
        };
        let dark = PageTheme::Dark;
        let mut p = PrivateChoices::default();
        p.write(
            &stored,
            SiteChange::Zoom {
                page: page.into(),
                percent: 80,
            },
        );
        p.write(
            &stored,
            SiteChange::Theme {
                page: page.into(),
                theme: dark,
            },
        );
        p.write(&stored, SiteChange::Blocker("a.test".into()));
        p.write(&stored, SiteChange::NeverSleep("a.test".into()));
        let view = SiteView {
            stored,
            private: Some(&p),
        };
        assert_eq!(view.zoom(page), 80);
        assert_eq!(view.theme(page), dark);
        assert_eq!(view.blocker(), Vec::<String>::new());
        assert_eq!(view.never_sleep(), vec!["a.test".to_string()]);
        let normal = SiteView {
            stored,
            private: None,
        };
        assert_eq!(
            (normal.zoom(page), normal.theme(page)),
            (150, PageTheme::Auto)
        );
        // a normal tab reads the stored lists only
        assert_eq!(
            (normal.blocker(), normal.never_sleep()),
            (blocker.clone(), never.clone())
        );
        // back to the stored values: nothing is left
        p.write(
            &stored,
            SiteChange::Zoom {
                page: page.into(),
                percent: 150,
            },
        );
        p.write(
            &stored,
            SiteChange::Theme {
                page: page.into(),
                theme: PageTheme::Auto,
            },
        );
        p.write(&stored, SiteChange::Blocker("a.test".into()));
        p.write(&stored, SiteChange::NeverSleep("a.test".into()));
        assert_eq!(p, PrivateChoices::default());
    }

    #[test]
    fn built_from_rows() {
        let t: SitePermissions = [
            ("https://a.test".to_string(), Camera, Decision::Allow),
            ("https://a.test".to_string(), Geolocation, Decision::Block),
        ]
        .into_iter()
        .collect();
        assert_eq!(t.get("https://a.test", Camera), Some(Decision::Allow));
        assert_eq!(t.get("https://a.test", Geolocation), Some(Decision::Block));
    }

    #[test]
    fn origins() {
        let cases = [
            ("https://a.test/x?y#z", Some("https://a.test")),
            ("https://A.Test:443/", Some("https://a.test")),
            ("https://a.test:8443/", Some("https://a.test:8443")),
            ("http://localhost:8080/p", Some("http://localhost:8080")),
            ("https://user:pw@a.test/", Some("https://a.test")),
            ("file:///tmp/x.html", None),
            ("data:text/html,x", None),
            ("about:blank", None),
            ("not a url", None),
        ];
        for (url, want) in cases {
            assert_eq!(origin_of(url).as_deref(), want, "{url}");
        }
    }
}
