//! The OS keyring behind a trait, with a fake for tests.
//!
//! Design: Chromium `components/os_crypt` keeps its key in the OS store
//! (libsecret/KWallet, Keychain, DPAPI). There is deliberately no fallback key:
//! when the keyring fails the vault is Unavailable (not Chromium's legacy
//! `peanuts` password).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

pub const SERVICE: &str = "wubble";

/// Keyring account for a profile.
pub fn account(profile: &str) -> String {
    format!("vault-{profile}")
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct KeyRingError(pub String);

pub trait KeyRing {
    /// `Ok(None)` when no entry exists; any other failure is an error.
    fn get(&self, service: &str, account: &str)
    -> Result<Option<Zeroizing<Vec<u8>>>, KeyRingError>;
    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), KeyRingError>;
}

/// A keyring chosen at run time: the OS one, or the fake for a smoke.
impl KeyRing for Box<dyn KeyRing + Send> {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, KeyRingError> {
        (**self).get(service, account)
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), KeyRingError> {
        (**self).set(service, account, secret)
    }
}

/// The real keyring (Keychain, Credential Manager, Secret Service).
///
/// If the platform store cannot be initialised (for example Secret Service is not
/// up yet) the failure is cached for the life of the process: `get` and `set` keep
/// failing and the vault stays Unavailable until wubble restarts.
pub struct OsKeyRing;

impl OsKeyRing {
    fn entry(service: &str, account: &str) -> Result<keyring::Entry, KeyRingError> {
        // keyring 4 initialises the credential store once per process and caches
        // a failure, so a retry cannot succeed until the process restarts.
        if let Err(e) = keyring::Entry::store_status() {
            return Err(KeyRingError(format!(
                "credential store failed to initialise ({e}); restart wubble to retry"
            )));
        }
        keyring::Entry::new(service, account).map_err(|e| KeyRingError(e.to_string()))
    }
}

impl KeyRing for OsKeyRing {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, KeyRingError> {
        match Self::entry(service, account)?.inner.get_secret() {
            Ok(secret) => Ok(Some(Zeroizing::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeyRingError(e.to_string())),
        }
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), KeyRingError> {
        Self::entry(service, account)?
            .inner
            .set_secret(secret)
            .map_err(|e| KeyRingError(e.to_string()))
    }
}

type Entries = HashMap<(String, String), Vec<u8>>;

/// In-memory keyring for tests. Clones share state.
#[derive(Clone, Default)]
pub struct FakeKeyRing {
    entries: Arc<Mutex<Entries>>,
    failing: Arc<Mutex<bool>>,
}

impl FakeKeyRing {
    pub fn set_failing(&self, failing: bool) {
        *self.failing.lock().unwrap() = failing;
    }

    fn check(&self) -> Result<(), KeyRingError> {
        if *self.failing.lock().unwrap() {
            Err(KeyRingError("keyring unavailable".into()))
        } else {
            Ok(())
        }
    }
}

impl KeyRing for FakeKeyRing {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, KeyRingError> {
        self.check()?;
        let map = self.entries.lock().unwrap();
        Ok(map
            .get(&(service.to_owned(), account.to_owned()))
            .map(|v| Zeroizing::new(v.clone())))
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), KeyRingError> {
        self.check()?;
        let mut map = self.entries.lock().unwrap();
        map.insert((service.to_owned(), account.to_owned()), secret.to_vec());
        Ok(())
    }
}
