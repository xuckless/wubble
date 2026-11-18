//! wubble's built-in password store: per-value AES-256-GCM, master key in the OS keyring.
//!
//! Threat model:
//! - Protects against: theft of the database file or a backup of it (the key is
//!   only in the OS keyring, never on disk next to the data).
//! - Protects against: other OS users, since the keyring is per user.
//! - Protects against: swapping ciphertexts between rows or fields (id, origin
//!   and field are bound as AEAD associated data).
//! - Does not protect against malware running as the user: it can ask the same
//!   keyring or read this process's memory.
//! - Does not protect while the OS keyring is unlocked and the disk is stolen
//!   together with the logged-in session.
//! - Origins and times are stored in plaintext (needed for lookup).
//! - No master password and no key rotation; deleting the keyring entry makes
//!   records unreadable (they fail closed, nothing is auto-deleted).
//! - No keyring means Unavailable: nothing is saved and there is no fallback
//!   key (Chromium's legacy `peanuts` fallback is deliberately not copied).
//! - Zeroizing is best effort; strings handed to other layers are out of our control.
//!
//! Design copied from Chromium `components/os_crypt` and Firefox's logins store;
//! see `THIRD_PARTY_NOTICES.md`. No CEF, wgpu or Slint here.

pub mod crypto;
pub mod generate;
pub mod import;
pub mod keyring;
pub mod origin;
pub mod record;
pub mod state;
pub mod store;
pub mod store_adapter;
pub mod vault;

pub use generate::{GenError, GeneratorOptions, generate};
pub use import::{ApplyReport, Import, ImportError, ImportRow, apply, parse_csv};
pub use keyring::{FakeKeyRing, KeyRing, OsKeyRing};
pub use origin::Origin;
pub use record::{Candidate, Meta, NewLogin, RecordId};
pub use state::VaultState;
pub use store::{LoginStore, MemoryStore, StoreError};
pub use store_adapter::StoreLogins;
pub use vault::{UnlockOutcome, Vault, VaultError};
