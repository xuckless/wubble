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
}

impl Permission {
    pub const ALL: [Permission; 14] = [
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
