//! The settings that are one Chromium profile preference each, as data: [`PREFS`] maps a
//! setting to its preference and how the stored text becomes the preference's value. The engine
//! sets every row on each container's request context as it comes up and on each change
//! (`Engine::set_profile_pref`); Chromium does the rest.
//!
//! Preference names and values from Chromium (`chrome/common/pref_names.h`,
//! `components/privacy_sandbox/privacy_sandbox_prefs.h`, `components/payments/core/payment_prefs.h`,
//! `chrome/browser/preloading/preloading_prefs.h`,
//! `third_party/blink/common/peerconnection/webrtc_ip_handling_policy.cc`), BSD-3-Clause; no code
//! copied.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::settings::{self as s, SettingDef, SettingKind};

/// A Chromium preference's value, as wubble sets them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefValue {
    Bool(bool),
    Int(i32),
    Str(Cow<'static, str>),
}

/// How a setting's stored text becomes its preference's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mapping {
    /// A `Bool` setting: the same bool.
    Bool,
    /// A `Count` setting: the same number.
    Int,
    /// A `Choice` setting: the value at its word's place among the choice's words.
    Choice(&'static [PrefValue]),
}

/// One setting that is one profile preference.
#[derive(Debug)]
pub struct PrefRow {
    pub setting: &'static SettingDef,
    /// Chromium's preference name.
    pub pref: &'static str,
    pub mapping: Mapping,
}

/// Every setting that is a profile preference. Add a preference as a row here (and its
/// `SettingDef` in `settings`): the engine, the page and the store need nothing else.
pub const PREFS: &[PrefRow] = &[
    PrefRow {
        setting: &s::PAGES_MIN_FONT_SIZE,
        pref: "webkit.webprefs.minimum_font_size",
        mapping: Mapping::Int,
    },
    PrefRow {
        setting: &s::PRIVACY_DO_NOT_TRACK,
        pref: "enable_do_not_track",
        mapping: Mapping::Bool,
    },
    PrefRow {
        setting: &s::PRIVACY_PRELOAD,
        pref: "net.network_prediction_options",
        // `NetworkPredictionOptions`: kStandard 0, kDisabled 2
        mapping: Mapping::Choice(&[PrefValue::Int(0), PrefValue::Int(2)]),
    },
    PrefRow {
        setting: &s::PRIVACY_CAN_MAKE_PAYMENT,
        pref: "payments.can_make_payment_enabled",
        mapping: Mapping::Bool,
    },
    PrefRow {
        setting: &s::PRIVACY_AD_TOPICS,
        pref: "privacy_sandbox.m1.topics_enabled",
        mapping: Mapping::Bool,
    },
    PrefRow {
        setting: &s::PRIVACY_SITE_SUGGESTED_ADS,
        pref: "privacy_sandbox.m1.fledge_enabled",
        mapping: Mapping::Bool,
    },
    PrefRow {
        setting: &s::PRIVACY_AD_MEASUREMENT,
        pref: "privacy_sandbox.m1.ad_measurement_enabled",
        mapping: Mapping::Bool,
    },
    PrefRow {
        setting: &s::PRIVACY_WEBRTC_IP,
        pref: "webrtc.ip_handling_policy",
        mapping: Mapping::Choice(&[
            PrefValue::Str(Cow::Borrowed("default")),
            PrefValue::Str(Cow::Borrowed("default_public_and_private_interfaces")),
            PrefValue::Str(Cow::Borrowed("default_public_interface_only")),
            PrefValue::Str(Cow::Borrowed("disable_non_proxied_udp")),
        ]),
    },
];

impl PrefRow {
    /// The preference's value for the setting's stored text (its default when absent or bad).
    pub fn value(&self, stored: &HashMap<String, String>) -> PrefValue {
        let text = crate::apply::value_of(self.setting, stored);
        match (self.mapping, self.setting.kind) {
            (Mapping::Bool, _) => PrefValue::Bool(text == "1"),
            (Mapping::Int, _) => PrefValue::Int(text.parse().unwrap_or(0)),
            (Mapping::Choice(values), SettingKind::Choice(words)) => {
                let at = words.iter().position(|w| *w == text).unwrap_or(0);
                values[at].clone()
            }
            // a row whose mapping does not fit its kind; `rows_fit_their_settings` refuses it
            (Mapping::Choice(values), _) => values[0].clone(),
        }
    }
}

/// The preference `key`'s setting is, and its value: `None` for a setting that is none.
pub fn pref_of(key: &str, stored: &HashMap<String, String>) -> Option<(&'static str, PrefValue)> {
    PREFS
        .iter()
        .find(|r| r.setting.key == key)
        .map(|r| (r.pref, r.value(stored)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn rows_fit_their_settings() {
        for (i, row) in PREFS.iter().enumerate() {
            let key = row.setting.key;
            assert_eq!(s::find(key), Some(row.setting), "{key} is in settings::ALL");
            assert!(
                PREFS[..i]
                    .iter()
                    .all(|o| o.setting.key != key && o.pref != row.pref),
                "{key} twice"
            );
            assert!(!row.setting.restart, "{key}: a preference applies live");
            match (row.mapping, row.setting.kind) {
                (Mapping::Bool, SettingKind::Bool) => {}
                (Mapping::Int, SettingKind::Count { max, .. }) => {
                    assert!(i32::try_from(max).is_ok(), "{key}")
                }
                (Mapping::Choice(values), SettingKind::Choice(words)) => {
                    assert_eq!(values.len(), words.len(), "{key}: one value per word")
                }
                other => panic!("{key}: mapping {other:?} does not fit"),
            }
        }
    }

    #[test]
    fn defaults_are_chromes_except_the_sandbox_apis() {
        let empty = HashMap::new();
        let want: &[(&str, PrefValue)] = &[
            ("webkit.webprefs.minimum_font_size", PrefValue::Int(0)),
            ("enable_do_not_track", PrefValue::Bool(false)),
            ("net.network_prediction_options", PrefValue::Int(0)),
            ("payments.can_make_payment_enabled", PrefValue::Bool(true)),
            // Chrome asks for consent first; wubble never shows that consent, so off
            ("privacy_sandbox.m1.topics_enabled", PrefValue::Bool(false)),
            ("privacy_sandbox.m1.fledge_enabled", PrefValue::Bool(false)),
            (
                "privacy_sandbox.m1.ad_measurement_enabled",
                PrefValue::Bool(false),
            ),
            (
                "webrtc.ip_handling_policy",
                PrefValue::Str("default".into()),
            ),
        ];
        for (pref, value) in want {
            let row = PREFS.iter().find(|r| r.pref == *pref).expect(pref);
            assert_eq!(&row.value(&empty), value, "{pref}");
        }
    }

    #[test]
    fn stored_text_becomes_the_value() {
        let cases: &[(&str, &str, Option<(&str, PrefValue)>)] = &[
            (
                "privacy.do_not_track",
                "1",
                Some(("enable_do_not_track", PrefValue::Bool(true))),
            ),
            (
                "privacy.preload",
                "off",
                Some(("net.network_prediction_options", PrefValue::Int(2))),
            ),
            (
                "privacy.preload",
                "nonsense",
                Some(("net.network_prediction_options", PrefValue::Int(0))),
            ),
            (
                "privacy.webrtc_ip",
                "proxy-only",
                Some((
                    "webrtc.ip_handling_policy",
                    PrefValue::Str("disable_non_proxied_udp".into()),
                )),
            ),
            (
                "pages.min_font_size",
                "12",
                Some(("webkit.webprefs.minimum_font_size", PrefValue::Int(12))),
            ),
            (
                "pages.min_font_size",
                "99",
                Some(("webkit.webprefs.minimum_font_size", PrefValue::Int(0))),
            ),
            ("search.engine", "google", None),
        ];
        for (key, text, want) in cases {
            assert_eq!(
                &pref_of(key, &stored(&[(*key, *text)])),
                want,
                "{key} = {text}"
            );
        }
    }
}
