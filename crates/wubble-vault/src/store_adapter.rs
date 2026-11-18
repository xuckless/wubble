//! `LoginStore` over wubble-store's `logins` table: the one production storage of the vault.
//!
//! Sync vs async, decided here: the vault trait stays synchronous and this adapter drives each
//! store future with `pollster::block_on` (the store needs no executor, see its crate docs).
//! The app already owns the `Store` on one dedicated thread that block_ons every request
//! (`crates/wubble/src/persist.rs`), so that thread is the one place a sync vault may block on
//! disk. Contract: build and use a `StoreLogins` (and so a `Vault` over it) only on the store
//! thread, never on the Slint thread. An async trait would push `async fn` through the vault,
//! its importer and every test for no gain.
//!
//! Ownership: the adapter owns the `Store` so the unlocked key stays in the `Vault` between
//! requests. The store thread reaches its other requests through
//! `vault.store_mut().store_mut()`.

use wubble_store::{LoginRow, Store};

use crate::origin::Origin;
use crate::record::{RecordId, StoredRecord};
use crate::store::{LoginStore, StoreError};

pub struct StoreLogins {
    store: Store,
}

impl StoreLogins {
    pub fn new(store: Store) -> Self {
        StoreLogins { store }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    pub fn into_inner(self) -> Store {
        self.store
    }
}

fn backend(e: wubble_store::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

fn to_row(r: StoredRecord) -> LoginRow {
    LoginRow {
        id: r.id.0,
        origin: r.origin.to_string(),
        form_action_origin: r.form_action_origin.map(|o| o.to_string()),
        username_enc: r.username_enc,
        password_enc: r.password_enc,
        created_ms: r.created_ms,
        last_used_ms: r.last_used_ms,
        times_used: r.times_used,
    }
}

/// A stored origin that does not parse means the row was written by something else or
/// corrupted: fail loudly rather than hide it.
fn from_row(r: LoginRow) -> Result<StoredRecord, StoreError> {
    let origin = |s: &str| {
        Origin::parse(s).map_err(|e| StoreError::Backend(format!("corrupt origin {s:?}: {e}")))
    };
    Ok(StoredRecord {
        origin: origin(&r.origin)?,
        form_action_origin: r.form_action_origin.as_deref().map(origin).transpose()?,
        id: RecordId(r.id),
        username_enc: r.username_enc,
        password_enc: r.password_enc,
        created_ms: r.created_ms,
        last_used_ms: r.last_used_ms,
        times_used: r.times_used,
    })
}

fn rows(rows: Vec<LoginRow>) -> Result<Vec<StoredRecord>, StoreError> {
    rows.into_iter().map(from_row).collect()
}

impl LoginStore for StoreLogins {
    fn insert(&mut self, record: StoredRecord) -> Result<(), StoreError> {
        pollster::block_on(self.store.insert_login(to_row(record))).map_err(|e| {
            if e.is_constraint() {
                StoreError::Duplicate
            } else {
                backend(e)
            }
        })
    }

    fn update(&mut self, record: StoredRecord) -> Result<(), StoreError> {
        let found = pollster::block_on(self.store.replace_login_secrets(
            &record.id.0,
            record.username_enc,
            record.password_enc,
        ))
        .map_err(backend)?;
        found.then_some(()).ok_or(StoreError::NotFound)
    }

    fn delete(&mut self, id: &RecordId) -> Result<bool, StoreError> {
        pollster::block_on(self.store.delete_login(&id.0)).map_err(backend)
    }

    fn get(&self, id: &RecordId) -> Result<Option<StoredRecord>, StoreError> {
        pollster::block_on(self.store.login(&id.0))
            .map_err(backend)?
            .map(from_row)
            .transpose()
    }

    fn by_origin(&self, origin: &Origin) -> Result<Vec<StoredRecord>, StoreError> {
        rows(
            pollster::block_on(self.store.logins_for_origin(&origin.to_string()))
                .map_err(backend)?,
        )
    }

    fn all(&self) -> Result<Vec<StoredRecord>, StoreError> {
        rows(pollster::block_on(self.store.all_logins()).map_err(backend)?)
    }

    fn touch(&mut self, id: &RecordId, now_ms: i64) -> Result<(), StoreError> {
        let found =
            pollster::block_on(self.store.record_login_use(&id.0, now_ms)).map_err(backend)?;
        found.then_some(()).ok_or(StoreError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use zeroize::Zeroizing;

    use super::*;
    use crate::keyring::FakeKeyRing;
    use crate::record::NewLogin;
    use crate::vault::{UnlockOutcome, Vault, VaultError};

    type V = Vault<FakeKeyRing, StoreLogins>;

    fn open(path: &Path) -> StoreLogins {
        StoreLogins::new(pollster::block_on(Store::open(path)).unwrap())
    }

    fn vault_at(path: &Path, kr: &FakeKeyRing) -> V {
        Vault::new(kr.clone(), open(path), "default")
    }

    fn login(origin: &str, user: &str, pw: &str) -> NewLogin {
        NewLogin {
            origin: Origin::parse(origin).unwrap(),
            form_action_origin: Some(Origin::parse(origin).unwrap()),
            username: user.into(),
            password: Zeroizing::new(pw.into()),
        }
    }

    fn page(s: &str) -> Origin {
        Origin::parse(s).unwrap()
    }

    #[test]
    fn save_find_update_delete_and_mark_used_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault_at(&dir.path().join("w.db"), &FakeKeyRing::default());
        assert_eq!(v.unlock(), UnlockOutcome::NewKey { unreadable: 0 });
        let id = v.save(login("https://a.test", "alice", "pw1"), 5).unwrap();
        v.save(login("https://b.test", "bob", "pw2"), 6).unwrap();
        let found = v.find_for_origin(&page("https://a.test")).unwrap();
        assert_eq!((found.len(), found[0].username.as_str()), (1, "alice"));
        assert_eq!(
            found[0]
                .form_action_origin
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("https://a.test")
        );
        assert_eq!(&**v.reveal(&id).unwrap(), "pw1");
        // Update re-encrypts with fresh nonces.
        let before = v.store().get(&id).unwrap().unwrap();
        v.update(&id, "alice", "pw1-new").unwrap();
        let after = v.store().get(&id).unwrap().unwrap();
        assert_ne!(before.password_enc, after.password_enc);
        assert_eq!(&**v.reveal(&id).unwrap(), "pw1-new");
        v.mark_used(&id, 9).unwrap();
        v.mark_used(&id, 7).unwrap();
        let meta = v.list().unwrap().into_iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            (meta.created_ms, meta.last_used_ms, meta.times_used),
            (5, Some(9), 2)
        );
        assert!(v.delete(&id).unwrap());
        assert!(!v.delete(&id).unwrap());
        assert_eq!(v.list().unwrap().len(), 1);
        assert_eq!(
            v.reveal(&id).unwrap_err(),
            VaultError::Store(StoreError::NotFound)
        );
        assert_eq!(
            v.mark_used(&id, 1).unwrap_err(),
            VaultError::Store(StoreError::NotFound)
        );
        assert_eq!(
            v.update(&id, "a", "b").unwrap_err(),
            VaultError::Store(StoreError::NotFound)
        );
    }

    #[test]
    fn a_reopened_database_still_decrypts_with_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let kr = FakeKeyRing::default();
        let id = {
            let mut v = vault_at(&path, &kr);
            v.unlock();
            v.save(login("https://a.test", "alice", "pw1"), 5).unwrap()
        };
        let mut v = vault_at(&path, &kr);
        assert_eq!(v.unlock(), UnlockOutcome::Ready);
        assert_eq!(&**v.reveal(&id).unwrap(), "pw1");
        assert_eq!(v.find_for_origin(&page("https://a.test")).unwrap().len(), 1);
    }

    #[test]
    fn a_wrong_key_fails_closed_and_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let id = {
            let mut v = vault_at(&path, &FakeKeyRing::default());
            v.unlock();
            v.save(login("https://a.test", "alice", "pw1"), 5).unwrap()
        };
        // Same database, a keyring that has never seen the first key.
        let mut v = vault_at(&path, &FakeKeyRing::default());
        assert_eq!(v.unlock(), UnlockOutcome::NewKey { unreadable: 1 });
        assert!(v.reveal(&id).is_err());
        assert!(
            v.find_for_origin(&page("https://a.test"))
                .unwrap()
                .is_empty()
        );
        // The row is still there, listed, and was not deleted.
        assert_eq!(v.list().unwrap().len(), 1);
    }

    #[test]
    fn a_tampered_blob_fails_to_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault_at(&dir.path().join("w.db"), &FakeKeyRing::default());
        v.unlock();
        let id = v.save(login("https://a.test", "alice", "pw1"), 5).unwrap();
        let mut row = v.store().get(&id).unwrap().unwrap();
        row.password_enc[14] ^= 1;
        v.store_mut().update(row).unwrap();
        assert!(v.reveal(&id).is_err());
    }

    #[test]
    fn a_duplicate_id_maps_to_duplicate_and_a_bad_origin_to_backend() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(&dir.path().join("w.db"));
        let rec = StoredRecord {
            id: RecordId("same".into()),
            origin: page("https://a.test"),
            form_action_origin: None,
            username_enc: vec![1],
            password_enc: vec![2],
            created_ms: 1,
            last_used_ms: None,
            times_used: 0,
        };
        s.insert(rec.clone()).unwrap();
        assert_eq!(s.insert(rec.clone()).unwrap_err(), StoreError::Duplicate);
        assert_eq!(s.get(&rec.id).unwrap(), Some(rec));
        // A row some other writer left with an origin we cannot parse is an error, not a skip.
        pollster::block_on(s.store_mut().insert_login(LoginRow {
            id: "bad".into(),
            origin: "not an origin".into(),
            form_action_origin: None,
            username_enc: vec![],
            password_enc: vec![],
            created_ms: 0,
            last_used_ms: None,
            times_used: 0,
        }))
        .unwrap();
        assert!(matches!(s.all().unwrap_err(), StoreError::Backend(_)));
    }

    #[test]
    fn list_converts_to_page_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut v = vault_at(&dir.path().join("w.db"), &FakeKeyRing::default());
        v.unlock();
        v.save(login("https://a.test", "alice", "pw1"), 5).unwrap();
        let entries: Vec<wubble_core::pages::LoginEntry> =
            v.list().unwrap().iter().map(Into::into).collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (
                entries[0].origin.as_str(),
                entries[0].created_ms,
                entries[0].times_used
            ),
            ("https://a.test", 5, 0)
        );
    }
}
