//! `containers.json` (Firefox contextual identities, also used by Zen).
//!
//! Field names and default names: mozilla-firefox/firefox
//! `toolkit/components/contextualidentity/ContextualIdentityService.sys.mjs`
//! (`_userIdentities`, `public`, `userContextId`, l10nIds `user-context-*2`), MPL-2.0.

use crate::ImportError;
use serde_json::Value;
use wubble_core::space::{Container, ContainerId, Storage};

/// The four built-in identities store only an l10n id, not a name.
const DEFAULT_NAMES: &[(&str, &str)] = &[
    ("user-context-personal", "Personal"),
    ("user-context-work", "Work"),
    ("user-context-banking", "Banking"),
    ("user-context-shopping", "Shopping"),
];

pub fn parse(bytes: &[u8]) -> Result<Vec<Container>, ImportError> {
    let json: Value =
        serde_json::from_slice(bytes).map_err(|_| ImportError::Unreadable("containers.json"))?;
    let identities = json["identities"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    Ok(identities
        .iter()
        .filter(|i| i["public"].as_bool() != Some(false))
        .filter_map(|i| {
            let id = u32::try_from(i["userContextId"].as_u64()?).ok()?;
            let name = match i["name"].as_str().filter(|n| !n.is_empty()) {
                Some(n) => n,
                None => {
                    let l10n = i["l10nId"].as_str()?;
                    DEFAULT_NAMES.iter().find(|(p, _)| l10n.starts_with(p))?.1
                }
            };
            Some(Container {
                id: ContainerId(id),
                name: name.to_owned(),
                storage: Storage::Disk,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture() {
        let got = parse(include_bytes!("../tests/fixtures/containers.json")).unwrap();
        let names: Vec<_> = got.iter().map(|c| (c.id.0, c.name.as_str())).collect();
        assert_eq!(
            names,
            [
                (1, "Personal"),
                (2, "Work"),
                (3, "Banking"),
                (4, "Shopping"),
                (6, "Gaming")
            ]
        );
    }

    #[test]
    fn malformed_json() {
        assert!(parse(b"{").is_err());
    }

    #[test]
    fn missing_identities_is_empty() {
        assert!(parse(b"{}").unwrap().is_empty());
    }
}
