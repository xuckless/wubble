//! Secure DNS (DNS over HTTPS), Chrome's Settings > Security > "Use secure DNS": Chromium's
//! default (automatic: DoH to the system's resolver when it is a known DoH provider), off, a
//! provider from [`PROVIDERS`], or a template of the user's own. The setting
//! (`settings::PRIVACY_SECURE_DNS`) stores one word ([`SecureDns::parse`]); [`SecureDns::prefs`]
//! is what Chromium's local state gets (`dns_over_https.mode`, `dns_over_https.templates`).
//! Nothing is sent to a provider unless the user picks it.
//!
//! The provider rows are Chromium's `net/dns/public/doh_provider_entry.cc` (BSD-3-Clause,
//! THIRD_PARTY_NOTICES.md), the entries Chrome shows in its menu (those with a `ui_name`):
//! name, template and privacy policy as there. The template check follows
//! `net/dns/public/dns_over_https_server_config.cc` (an https URI template; with a `dns`
//! variable it is a GET server, without one a POST server), no code copied.

/// One DoH provider the menu offers.
#[derive(Debug, PartialEq, Eq)]
pub struct Provider {
    /// The stored word, `provider:` and the provider's id.
    pub word: &'static str,
    /// Chrome's `ui_name`.
    pub name: &'static str,
    pub template: &'static str,
    pub privacy_policy: &'static str,
}

/// Ported from Chromium's `DohProviderEntry::GetList()`: the entries with a `ui_name`, in its
/// order.
pub const PROVIDERS: &[Provider] = &[
    Provider {
        word: "provider:cleanbrowsing-family",
        name: "CleanBrowsing (Family Filter)",
        template: "https://doh.cleanbrowsing.org/doh/family-filter{?dns}",
        privacy_policy: "https://cleanbrowsing.org/privacy",
    },
    Provider {
        word: "provider:cloudflare",
        name: "Cloudflare (1.1.1.1)",
        template: "https://chrome.cloudflare-dns.com/dns-query",
        privacy_policy: "https://developers.cloudflare.com/1.1.1.1/privacy/public-dns-resolver/",
    },
    Provider {
        word: "provider:cznic",
        name: "CZ.NIC ODVR",
        template: "https://odvr.nic.cz/doh",
        privacy_policy: "https://www.nic.cz/odvr/",
    },
    Provider {
        word: "provider:dns4eu",
        name: "DNS4EU Public Service (Protective)",
        template: "https://protective.joindns4.eu/dns-query{?dns}",
        privacy_policy: "https://app-eu1.hubspotdocuments.com/documents/142290803/view/1331719984?accessId=b505d9",
    },
    Provider {
        word: "provider:dnssb",
        name: "DNS.SB",
        template: "https://doh.dns.sb/dns-query{?dns}",
        privacy_policy: "https://dns.sb/privacy/",
    },
    Provider {
        word: "provider:google",
        name: "Google (Public DNS)",
        template: "https://dns.google/dns-query{?dns}",
        privacy_policy: "https://developers.google.com/speed/public-dns/privacy",
    },
    Provider {
        word: "provider:iij",
        name: "IIJ (Public DNS)",
        template: "https://public.dns.iij.jp/dns-query",
        privacy_policy: "https://policy.public.dns.iij.jp/",
    },
    Provider {
        word: "provider:nextdns",
        name: "NextDNS",
        template: "https://chromium.dns.nextdns.io",
        privacy_policy: "https://nextdns.io/privacy",
    },
    Provider {
        word: "provider:opendns",
        name: "OpenDNS",
        template: "https://doh.opendns.com/dns-query{?dns}",
        privacy_policy: "https://www.cisco.com/c/en/us/about/legal/privacy-full.html",
    },
    Provider {
        word: "provider:quad9",
        name: "Quad9 (9.9.9.9)",
        template: "https://dns.quad9.net/dns-query",
        privacy_policy: "https://www.quad9.net/home/privacy/",
    },
];

/// The stored words before a custom template's text.
const AUTOMATIC: &str = "automatic";
const OFF: &str = "off";
pub const CUSTOM: &str = "custom:";

const CHOICES: usize = PROVIDERS.len() + 3;

/// The menu's words and what it shows for each, in its order: automatic, off, the providers,
/// custom (whose stored text is `custom:` and the template). Built from [`PROVIDERS`], so a
/// provider is one row there; the Settings Select reads it (`settings::SELECT_ROWS`).
pub const LABELS: [(&str, &str); CHOICES] = {
    let mut labels = [("", ""); CHOICES];
    labels[0] = (AUTOMATIC, "With your current service provider");
    labels[1] = (OFF, "Off");
    let mut i = 0;
    while i < PROVIDERS.len() {
        labels[i + 2] = (PROVIDERS[i].word, PROVIDERS[i].name);
        i += 1;
    }
    labels[CHOICES - 1] = (CUSTOM, "Custom");
    labels
};

/// [`LABELS`]' words: the row's choices.
pub const WORDS: [&str; CHOICES] = crate::settings::select_words(&LABELS);

/// What the setting asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecureDns {
    /// Chromium's default: DoH to the system's resolver when it offers it, else plain DNS.
    Automatic,
    Off,
    Provider(&'static Provider),
    /// The user's own template; empty until one is typed (applied as automatic meanwhile).
    Custom(String),
}

/// Why a custom template was refused: the note under the row.
pub const NOT_HTTPS: &str = "Needs an address that starts with https://";
pub const BAD_VARIABLE: &str = "The only variable a template may have is {?dns}";
pub const NOT_A_URL: &str = "Needs an address such as https://dns.example/dns-query{?dns}";

/// Checks a DoH URI template as Chromium's `DnsOverHttpsServerConfig::FromString` does: an
/// `https://` URL with a host once its `{?dns}` (or `{dns}`) variable is expanded away; any
/// other variable, or none at all, is refused (a template without one is a POST server).
pub fn validate_template(template: &str) -> Result<(), &'static str> {
    if !template.starts_with("https://") {
        return Err(NOT_HTTPS);
    }
    let expanded = template.replace("{?dns}", "").replace("{dns}", "");
    if expanded.contains(['{', '}']) {
        return Err(BAD_VARIABLE);
    }
    match url::Url::parse(&expanded) {
        Ok(u) if u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty()) => Ok(()),
        _ => Err(NOT_A_URL),
    }
}

impl SecureDns {
    /// The stored text's meaning; `Err` is why it is refused (the row's note).
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        match text {
            AUTOMATIC => return Ok(Self::Automatic),
            OFF => return Ok(Self::Off),
            _ => {}
        }
        if let Some(p) = PROVIDERS.iter().find(|p| p.word == text) {
            return Ok(Self::Provider(p));
        }
        let template = text.strip_prefix(CUSTOM).ok_or(NOT_A_URL)?.trim();
        if !template.is_empty() {
            validate_template(template)?;
        }
        Ok(Self::Custom(template.to_owned()))
    }

    /// The menu word this choice is under (`custom:` for any custom template).
    pub fn word(&self) -> &'static str {
        match self {
            Self::Automatic => AUTOMATIC,
            Self::Off => OFF,
            Self::Provider(p) => p.word,
            Self::Custom(_) => CUSTOM,
        }
    }

    /// Chromium's `dns_over_https.mode` and `dns_over_https.templates` for this choice.
    pub fn prefs(&self) -> (&'static str, String) {
        match self {
            Self::Automatic => ("automatic", String::new()),
            Self::Off => ("off", String::new()),
            Self::Provider(p) => ("secure", p.template.to_owned()),
            Self::Custom(t) if t.is_empty() => ("automatic", String::new()),
            Self::Custom(t) => ("secure", t.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_template_is_valid_and_words_are_unique() {
        for p in PROVIDERS {
            assert_eq!(validate_template(p.template), Ok(()), "{}", p.word);
            assert!(p.word.starts_with("provider:") && p.privacy_policy.starts_with("https://"));
            assert_eq!(SecureDns::parse(p.word), Ok(SecureDns::Provider(p)));
        }
        for (i, w) in WORDS.iter().enumerate() {
            assert!(!WORDS[..i].contains(w), "{w} twice");
            assert_eq!(SecureDns::parse(w).map(|d| d.word()), Ok(*w), "{w}");
            assert_ne!(LABELS[i].1, "", "{w}");
        }
        assert_eq!(WORDS[0], "automatic");
        assert_eq!(WORDS[WORDS.len() - 1], "custom:");
    }

    #[test]
    fn templates_are_checked_like_chromiums() {
        let cases: &[(&str, Result<(), &str>)] = &[
            ("https://dns.example/dns-query{?dns}", Ok(())),
            ("https://dns.example/dns-query", Ok(())),
            ("https://dns.example/q{dns}", Ok(())),
            ("https://dns.example:8443/dns-query{?dns}", Ok(())),
            ("http://dns.example/dns-query{?dns}", Err(NOT_HTTPS)),
            ("dns.example/dns-query", Err(NOT_HTTPS)),
            ("", Err(NOT_HTTPS)),
            ("https://dns.example/{?name}", Err(BAD_VARIABLE)),
            ("https://dns.example/{?dns", Err(BAD_VARIABLE)),
            ("https://", Err(NOT_A_URL)),
            ("https:// spaced.example/", Err(NOT_A_URL)),
        ];
        for (template, want) in cases {
            assert_eq!(validate_template(template), *want, "{template:?}");
        }
    }

    #[test]
    fn stored_text_parses_or_says_why_not() {
        let cases: &[(&str, Result<SecureDns, &str>)] = &[
            ("automatic", Ok(SecureDns::Automatic)),
            ("off", Ok(SecureDns::Off)),
            ("custom:", Ok(SecureDns::Custom(String::new()))),
            (
                "custom: https://d.example/q{?dns} ",
                Ok(SecureDns::Custom("https://d.example/q{?dns}".into())),
            ),
            ("custom:http://d.example/q", Err(NOT_HTTPS)),
            ("provider:nope", Err(NOT_A_URL)),
            ("secure", Err(NOT_A_URL)),
            ("", Err(NOT_A_URL)),
        ];
        for (text, want) in cases {
            assert_eq!(&SecureDns::parse(text), want, "{text:?}");
        }
    }

    #[test]
    fn each_choice_becomes_chromiums_mode_and_templates() {
        let cloudflare = SecureDns::parse("provider:cloudflare").unwrap();
        let cases: &[(SecureDns, (&str, &str))] = &[
            (SecureDns::Automatic, ("automatic", "")),
            (SecureDns::Off, ("off", "")),
            (
                cloudflare,
                ("secure", "https://chrome.cloudflare-dns.com/dns-query"),
            ),
            (SecureDns::Custom(String::new()), ("automatic", "")),
            (
                SecureDns::Custom("https://d.example/q{?dns}".into()),
                ("secure", "https://d.example/q{?dns}"),
            ),
        ];
        for (choice, (mode, templates)) in cases {
            let (m, t) = choice.prefs();
            assert_eq!((m, t.as_str()), (*mode, *templates), "{choice:?}");
        }
    }
}
