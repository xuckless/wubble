//! The one public key wubble trusts for updates (docs/release.md, "One-time key setup").
//!
//! The second line of `wubble-update.pub` (minisign's base64 public key) goes into
//! [`PUBLIC_KEY`], and the same text into the `PUBLIC_KEY='...'` line of
//! `scripts/release/install.sh` (`cargo test -p wubble-release` checks they agree). Nothing
//! else names a key: a release build cannot be told to trust another one, by flag,
//! environment or file.

/// The all-zero key: no key has been set up, so the updater does nothing at all.
pub const PLACEHOLDER: &str = "RWQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// The trusted key: minisign's base64 public key line.
pub const PUBLIC_KEY: &str = "RWTnFx9Fhg43QcjvADtCopb89Q392ewf5ZLKBmzS8072IWc0OJVlL0lK";

/// The compiled-in key, or `None` while it is the placeholder (or not a key at all).
pub fn trusted() -> Option<minisign_verify::PublicKey> {
    parse(PUBLIC_KEY)
}

/// `text` as a key, `None` for the placeholder or anything that is not a key.
pub fn parse(text: &str) -> Option<minisign_verify::PublicKey> {
    if text.trim() == PLACEHOLDER {
        return None;
    }
    minisign_verify::PublicKey::from_base64(text.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_is_no_key() {
        assert_eq!(parse(PLACEHOLDER), None);
        assert_eq!(parse("not a key"), None);
        // the placeholder is a well-formed key of zeros, so a decode alone would accept it
        assert!(minisign_verify::PublicKey::from_base64(PLACEHOLDER).is_ok());
    }

    #[test]
    fn a_real_key_parses() {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        assert!(parse(&pair.pk.to_base64()).is_some());
    }
}
