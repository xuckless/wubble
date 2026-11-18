//! Record types. Secrets exist in plaintext only in `NewLogin` and the values
//! returned by `reveal`; everything stored or listed is encrypted or metadata.

use crate::origin::Origin;
use rand::{RngCore, rngs::OsRng};
use std::fmt;
use zeroize::Zeroizing;

/// 128-bit random hex id, generated client side so it can be bound into the
/// AEAD associated data before the row exists.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordId(pub String);

impl RecordId {
    pub fn generate() -> RecordId {
        let mut bytes = [0u8; 16];
        OsRng.fill_bytes(&mut bytes);
        RecordId(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// A login to save.
pub struct NewLogin {
    pub origin: Origin,
    pub form_action_origin: Option<Origin>,
    pub username: String,
    pub password: Zeroizing<String>,
}

impl fmt::Debug for NewLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewLogin")
            .field("origin", &self.origin)
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .finish()
    }
}

/// The stored row: origins in plaintext (lookup needs them, as in Chromium and
/// Firefox), username and password as AEAD blobs.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredRecord {
    pub id: RecordId,
    pub origin: Origin,
    pub form_action_origin: Option<Origin>,
    pub username_enc: Vec<u8>,
    pub password_enc: Vec<u8>,
    pub created_ms: i64,
    pub last_used_ms: Option<i64>,
    pub times_used: u32,
}

impl fmt::Debug for StoredRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredRecord")
            .field("id", &self.id)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

/// Listing row: no username, no password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub id: RecordId,
    pub origin: Origin,
    pub form_action_origin: Option<Origin>,
    pub created_ms: i64,
    pub last_used_ms: Option<i64>,
    pub times_used: u32,
}

impl From<&StoredRecord> for Meta {
    fn from(r: &StoredRecord) -> Meta {
        Meta {
            id: r.id.clone(),
            origin: r.origin.clone(),
            form_action_origin: r.form_action_origin.clone(),
            created_ms: r.created_ms,
            last_used_ms: r.last_used_ms,
            times_used: r.times_used,
        }
    }
}

impl From<&Meta> for wubble_core::pages::LoginEntry {
    fn from(m: &Meta) -> Self {
        wubble_core::pages::LoginEntry {
            id: m.id.0.clone(),
            origin: m.origin.to_string(),
            created_ms: m.created_ms,
            last_used_ms: m.last_used_ms,
            times_used: m.times_used,
        }
    }
}

/// A fill candidate for a page: the username is shown, the password is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: RecordId,
    pub username: String,
    pub form_action_origin: Option<Origin>,
}
