//! The vault: state machine plus encrypted records over a `LoginStore`.

use crate::crypto::{self, CryptoError, Field};
use crate::keyring::{self, KeyRing};
use crate::origin::Origin;
use crate::record::{Candidate, Meta, NewLogin, RecordId, StoredRecord};
use crate::state::{Effect, Event, Key, VaultState, transition};
use crate::store::{LoginStore, StoreError};
use zeroize::Zeroizing;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VaultError {
    #[error("vault is locked")]
    NotUnlocked,
    #[error("vault unavailable: {0}")]
    Unavailable(String),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What an unlock found. The UI says "saved passwords cannot be read" for `NewKey` with
/// `unreadable > 0` and for `Unreadable`; nothing is deleted either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// Unlocked, and every stored login opens (or there are none).
    Ready,
    /// The keyring had no key, so this unlock made a new one. `unreadable` is how many logins
    /// the store already held: all were sealed under a key that is gone (0 on a fresh profile).
    NewKey { unreadable: usize },
    /// Unlocked with the existing key, but `rows` stored logins do not open with it (a database
    /// restored from another machine or keyring). New logins still save normally.
    Unreadable { rows: usize },
    /// No usable keyring, or the store could not be read. The vault is not unlocked.
    Unavailable(String),
}

pub struct Vault<K: KeyRing, S: LoginStore> {
    state: VaultState,
    keyring: K,
    store: S,
    account: String,
}

impl<K: KeyRing, S: LoginStore> Vault<K, S> {
    pub fn new(keyring: K, store: S, profile: &str) -> Self {
        Vault {
            state: VaultState::Locked,
            keyring,
            store,
            account: keyring::account(profile),
        }
    }

    pub fn state(&self) -> &VaultState {
        &self.state
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// The storage, for requests that are not about logins (the store thread owns one store).
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Fetch or create the master key and report what that means for the stored logins.
    /// May block on an OS prompt and reads every login once: call off the UI thread.
    pub fn unlock(&mut self) -> UnlockOutcome {
        if matches!(self.state, VaultState::Unlocked { .. }) {
            return UnlockOutcome::Ready;
        }
        let created = self.dispatch(Event::UnlockRequested);
        let VaultState::Unlocked { key } = &self.state else {
            return UnlockOutcome::Unavailable(match &self.state {
                VaultState::Unavailable { reason } => reason.clone(),
                _ => "unlock did not complete".into(),
            });
        };
        let stored = match self.store.all() {
            Ok(rows) => rows,
            Err(e) => {
                self.lock();
                return UnlockOutcome::Unavailable(e.to_string());
            }
        };
        if created {
            return UnlockOutcome::NewKey {
                unreadable: stored.len(),
            };
        }
        let rows = stored
            .iter()
            .filter(|r| open_string(key, r, Field::Username, &r.username_enc).is_err())
            .count();
        if rows == 0 {
            UnlockOutcome::Ready
        } else {
            UnlockOutcome::Unreadable { rows }
        }
    }

    pub fn lock(&mut self) {
        self.dispatch(Event::LockRequested);
    }

    /// Run the transition and its effects. Returns whether a key was created on the way.
    fn dispatch(&mut self, event: Event) -> bool {
        let state = std::mem::replace(&mut self.state, VaultState::Locked);
        let (state, effects) = transition(state, event);
        self.state = state;
        let mut created = false;
        for effect in effects {
            created |= effect == Effect::CreateKey;
            let event = self.run(effect);
            created |= self.dispatch(event);
        }
        created
    }

    /// The only impure step: talk to the keyring.
    fn run(&self, effect: Effect) -> Event {
        let (service, account) = (keyring::SERVICE, self.account.as_str());
        let read = |keyring: &K| match keyring.get(service, account) {
            Ok(Some(bytes)) => match <[u8; 32]>::try_from(bytes.as_slice()) {
                // A key of the wrong length is never replaced: that would orphan every record.
                Ok(key) => Event::KeyLoaded(Zeroizing::new(key)),
                Err(_) => Event::KeyringFailed("stored vault key has the wrong length".into()),
            },
            Ok(None) => Event::KeyMissing,
            Err(e) => Event::KeyringFailed(e.to_string()),
        };
        match effect {
            Effect::LoadKey => read(&self.keyring),
            Effect::CreateKey => {
                let key = crypto::generate_key();
                if let Err(e) = self.keyring.set(service, account, &*key) {
                    return Event::KeyringFailed(e.to_string());
                }
                // Read back to confirm the store kept our key. This does not make
                // creation atomic: there must be one Vault per profile (the profile
                // lock enforces it), or two creators could each keep a different key.
                match read(&self.keyring) {
                    Event::KeyMissing => Event::KeyringFailed("key not stored".into()),
                    event => event,
                }
            }
        }
    }

    fn key(&self) -> Result<&Key, VaultError> {
        match &self.state {
            VaultState::Unlocked { key } => Ok(key),
            VaultState::Locked => Err(VaultError::NotUnlocked),
            VaultState::Unavailable { reason } => Err(VaultError::Unavailable(reason.clone())),
        }
    }

    pub fn save(&mut self, login: NewLogin, now_ms: i64) -> Result<RecordId, VaultError> {
        let id = RecordId::generate();
        let (username_enc, password_enc) =
            self.encrypt(&id, &login.origin, &login.username, &login.password)?;
        self.store.insert(StoredRecord {
            id: id.clone(),
            origin: login.origin,
            form_action_origin: login.form_action_origin,
            username_enc,
            password_enc,
            created_ms: now_ms,
            last_used_ms: None,
            times_used: 0,
        })?;
        Ok(id)
    }

    /// Replace username and password, re-encrypting with fresh nonces.
    pub fn update(
        &mut self,
        id: &RecordId,
        username: &str,
        password: &str,
    ) -> Result<(), VaultError> {
        let mut row = self.store.get(id)?.ok_or(StoreError::NotFound)?;
        (row.username_enc, row.password_enc) = self.encrypt(id, &row.origin, username, password)?;
        self.store.update(row)?;
        Ok(())
    }

    pub fn delete(&mut self, id: &RecordId) -> Result<bool, VaultError> {
        Ok(self.store.delete(id)?)
    }

    /// Logins for a page: exact origin match only (`LoginStore::by_origin` is the one
    /// place the rule lives). Passwords are not decrypted. A row that does not decrypt
    /// (for example saved under a key the keyring has since lost) is skipped so it
    /// cannot hide the readable ones; `list` still shows it.
    pub fn find_for_origin(&self, page: &Origin) -> Result<Vec<Candidate>, VaultError> {
        let key = self.key()?;
        Ok(self
            .store
            .by_origin(page)?
            .into_iter()
            .filter_map(|r| {
                let username = open_string(key, &r, Field::Username, &r.username_enc).ok()?;
                Some(Candidate {
                    id: r.id,
                    username: username.to_string(),
                    form_action_origin: r.form_action_origin,
                })
            })
            .collect())
    }

    /// Metadata only, no plaintext. Works while locked.
    pub fn list(&self) -> Result<Vec<Meta>, VaultError> {
        Ok(self.store.all()?.iter().map(Meta::from).collect())
    }

    pub fn reveal(&self, id: &RecordId) -> Result<Zeroizing<String>, VaultError> {
        let key = self.key()?;
        let row = self.store.get(id)?.ok_or(StoreError::NotFound)?;
        open_string(key, &row, Field::Password, &row.password_enc)
    }

    pub fn mark_used(&mut self, id: &RecordId, now_ms: i64) -> Result<(), VaultError> {
        Ok(self.store.touch(id, now_ms)?)
    }

    fn encrypt(
        &self,
        id: &RecordId,
        origin: &Origin,
        username: &str,
        password: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
        let key = self.key()?;
        let origin = origin.to_string();
        Ok((
            crypto::seal(key, &id.0, &origin, Field::Username, username.as_bytes())?,
            crypto::seal(key, &id.0, &origin, Field::Password, password.as_bytes())?,
        ))
    }
}

fn open_string(
    key: &Key,
    row: &StoredRecord,
    field: Field,
    blob: &[u8],
) -> Result<Zeroizing<String>, VaultError> {
    let bytes = crypto::open(key, &row.id.0, &row.origin.to_string(), field, blob)?;
    let s = std::str::from_utf8(&bytes).map_err(|_| CryptoError::Decrypt)?;
    Ok(Zeroizing::new(s.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyring::FakeKeyRing;
    use crate::store::MemoryStore;

    type V = Vault<FakeKeyRing, MemoryStore>;

    fn vault(kr: &FakeKeyRing) -> V {
        Vault::new(kr.clone(), MemoryStore::default(), "default")
    }

    fn unlocked() -> V {
        let mut v = vault(&FakeKeyRing::default());
        assert_eq!(v.unlock(), UnlockOutcome::NewKey { unreadable: 0 });
        v
    }

    fn login(origin: &str, user: &str, pw: &str) -> NewLogin {
        NewLogin {
            origin: Origin::parse(origin).unwrap(),
            form_action_origin: None,
            username: user.into(),
            password: Zeroizing::new(pw.into()),
        }
    }

    fn key_of(v: &V) -> [u8; 32] {
        **v.key().unwrap()
    }

    #[test]
    fn key_is_created_once_and_reused() {
        let kr = FakeKeyRing::default();
        let (mut a, mut b) = (vault(&kr), vault(&kr));
        a.unlock();
        b.unlock();
        assert_eq!(key_of(&a), key_of(&b));
        a.lock();
        assert_eq!(a.state(), &VaultState::Locked);
        a.unlock();
        assert_eq!(key_of(&a), key_of(&b));
    }

    #[test]
    fn keys_are_per_profile() {
        let kr = FakeKeyRing::default();
        let mut a = Vault::new(kr.clone(), MemoryStore::default(), "a");
        let mut b = Vault::new(kr.clone(), MemoryStore::default(), "b");
        a.unlock();
        b.unlock();
        assert_ne!(key_of(&a), key_of(&b));
        assert!(kr.get("wubble", "vault-a").unwrap().is_some());
        assert!(kr.get("wubble", "vault-b").unwrap().is_some());
    }

    #[test]
    fn wrong_length_key_is_unavailable_and_kept() {
        let kr = FakeKeyRing::default();
        kr.set("wubble", "vault-default", &[1; 5]).unwrap();
        let mut v = vault(&kr);
        assert!(matches!(v.unlock(), UnlockOutcome::Unavailable(_)));
        assert!(matches!(v.state(), VaultState::Unavailable { .. }));
        assert_eq!(
            &**kr.get("wubble", "vault-default").unwrap().unwrap(),
            &[1; 5]
        );
    }

    #[test]
    fn keyring_failure_saves_nothing() {
        let kr = FakeKeyRing::default();
        kr.set_failing(true);
        let mut v = vault(&kr);
        assert!(matches!(v.unlock(), UnlockOutcome::Unavailable(_)));
        let err = v.save(login("https://a.test", "u", "p"), 1).unwrap_err();
        assert!(matches!(err, VaultError::Unavailable(_)));
        assert!(v.store().is_empty());
        // Retrying after the keyring recovers is an explicit unlock.
        kr.set_failing(false);
        assert_eq!(v.unlock(), UnlockOutcome::NewKey { unreadable: 0 });
    }

    #[test]
    fn locked_rejects_secrets() {
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "u", "p"), 1).unwrap();
        v.lock();
        assert_eq!(v.reveal(&id).unwrap_err(), VaultError::NotUnlocked);
        assert_eq!(
            v.save(login("https://a.test", "u", "p"), 1).unwrap_err(),
            VaultError::NotUnlocked
        );
        assert_eq!(
            v.find_for_origin(&Origin::parse("https://a.test").unwrap())
                .unwrap_err(),
            VaultError::NotUnlocked
        );
        assert_eq!(v.list().unwrap().len(), 1);
    }

    #[test]
    fn save_reveal_find_round_trip() {
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "alice", "pw1"), 5).unwrap();
        v.save(login("https://b.test", "bob", "pw2"), 6).unwrap();
        assert_eq!(&**v.reveal(&id).unwrap(), "pw1");
        let found = v
            .find_for_origin(&Origin::parse("https://a.test/login").unwrap())
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].username, "alice");
        // https credential is never offered on http.
        assert!(
            v.find_for_origin(&Origin::parse("http://a.test").unwrap())
                .unwrap()
                .is_empty()
        );
        v.mark_used(&id, 9).unwrap();
        let meta = v.list().unwrap().into_iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            (meta.created_ms, meta.last_used_ms, meta.times_used),
            (5, Some(9), 1)
        );
    }

    #[test]
    fn update_uses_new_nonces() {
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "u", "old"), 1).unwrap();
        let before = v.store().get(&id).unwrap().unwrap();
        v.update(&id, "u", "new").unwrap();
        let after = v.store().get(&id).unwrap().unwrap();
        assert_ne!(before.password_enc[..12], after.password_enc[..12]);
        assert_ne!(before.username_enc[..12], after.username_enc[..12]);
        assert_eq!(&**v.reveal(&id).unwrap(), "new");
    }

    #[test]
    fn nonces_unique_across_saves() {
        let mut v = unlocked();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let id = v.save(login("https://a.test", "u", "p"), 1).unwrap();
            let row = v.store().get(&id).unwrap().unwrap();
            assert!(seen.insert(row.username_enc[..12].to_vec()));
            assert!(seen.insert(row.password_enc[..12].to_vec()));
        }
    }

    #[test]
    fn delete_removes() {
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "u", "p"), 1).unwrap();
        assert!(v.delete(&id).unwrap());
        assert!(!v.delete(&id).unwrap());
        assert_eq!(
            v.reveal(&id).unwrap_err(),
            VaultError::Store(StoreError::NotFound)
        );
    }

    #[test]
    fn swapped_or_tampered_blobs_fail_closed() {
        let mut v = unlocked();
        let a = v.save(login("https://a.test", "u", "pa"), 1).unwrap();
        let b = v.save(login("https://a.test", "u", "pb"), 1).unwrap();
        let blob_b = v.store().get(&b).unwrap().unwrap().password_enc;
        // Cross-row swap (same origin, different id).
        v.store_mut().row_mut(&a).unwrap().password_enc = blob_b;
        assert_eq!(
            v.reveal(&a).unwrap_err(),
            VaultError::Crypto(CryptoError::Decrypt)
        );
        // Username and password blobs swapped inside one row.
        let row = v.store_mut().row_mut(&b).unwrap();
        std::mem::swap(&mut row.username_enc, &mut row.password_enc);
        assert!(v.reveal(&b).is_err());
        // Bit flip.
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "u", "p"), 1).unwrap();
        v.store_mut().row_mut(&id).unwrap().password_enc[14] ^= 1;
        assert!(v.reveal(&id).is_err());
        // Origin column rewritten.
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "u", "p"), 1).unwrap();
        v.store_mut().row_mut(&id).unwrap().origin = Origin::parse("https://evil.test").unwrap();
        assert!(v.reveal(&id).is_err());
    }

    #[test]
    fn unreadable_row_does_not_hide_good_ones() {
        let mut v = unlocked();
        let page = Origin::parse("https://a.test").unwrap();
        let old = v.save(login("https://a.test", "old", "p"), 1).unwrap();
        // Rows saved under a key that is now gone: swap in a vault with a new key.
        let mut v2 = unlocked();
        v2.store = std::mem::take(&mut v.store);
        let new = v2.save(login("https://a.test", "new", "p"), 2).unwrap();
        let found = v2.find_for_origin(&page).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, new);
        assert!(v2.reveal(&old).is_err());
        assert_eq!(v2.list().unwrap().len(), 2);
    }

    #[test]
    fn other_key_cannot_reveal() {
        let mut v = unlocked();
        let id = v.save(login("https://a.test", "u", "p"), 1).unwrap();
        let mut other = unlocked();
        other.store = std::mem::take(&mut v.store);
        assert!(other.reveal(&id).is_err());
    }

    #[test]
    fn unlock_reports_what_the_key_means_for_stored_logins() {
        // Fresh profile: a new key, nothing lost. Unlocking again is Ready without a recount.
        let kr = FakeKeyRing::default();
        let mut v = vault(&kr);
        assert_eq!(v.unlock(), UnlockOutcome::NewKey { unreadable: 0 });
        assert_eq!(v.unlock(), UnlockOutcome::Ready);
        v.save(login("https://a.test", "u", "p"), 1).unwrap();
        v.save(login("https://b.test", "u", "p"), 1).unwrap();
        // Same key, same rows: Ready.
        v.lock();
        assert_eq!(v.unlock(), UnlockOutcome::Ready);
        // The keyring entry is gone (wiped, new machine): a new key over two unreadable rows.
        let mut wiped = Vault::new(
            FakeKeyRing::default(),
            std::mem::take(&mut v.store),
            "default",
        );
        assert_eq!(wiped.unlock(), UnlockOutcome::NewKey { unreadable: 2 });
        // A different key already in the keyring: loaded fine, rows do not open.
        let other = FakeKeyRing::default();
        other.set("wubble", "vault-default", &[9; 32]).unwrap();
        let mut other = Vault::new(other, std::mem::take(&mut wiped.store), "default");
        assert_eq!(other.unlock(), UnlockOutcome::Unreadable { rows: 2 });
        // New logins still save, and only the old ones are counted.
        other.save(login("https://c.test", "u", "p"), 2).unwrap();
        other.lock();
        assert_eq!(other.unlock(), UnlockOutcome::Unreadable { rows: 2 });
    }

    #[test]
    fn debug_leaks_nothing() {
        let mut v = unlocked();
        let l = login("https://a.test", "alice-user", "s3cret-pw");
        let shown = format!("{l:?}");
        assert!(!shown.contains("s3cret-pw") && !shown.contains("alice-user"));
        let id = v.save(l, 1).unwrap();
        let row = v.store().get(&id).unwrap().unwrap();
        let shown = format!("{row:?} {:?}", v.list().unwrap());
        assert!(!shown.contains("s3cret-pw") && !shown.contains("alice-user"));
    }
}
