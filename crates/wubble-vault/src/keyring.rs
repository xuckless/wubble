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

/// A keyring in one file, for smokes that restart on a throwaway profile (`--smoke passwords`):
/// what the OS keyring keeps across runs, without touching it. One `service account hex` line
/// per entry. `broken`: every call fails, as a keyring that cannot be reached. Never for a real
/// profile: the key would sit next to the data it protects.
pub struct FileKeyRing {
    path: std::path::PathBuf,
    broken: bool,
}

impl FileKeyRing {
    pub fn new(path: impl Into<std::path::PathBuf>, broken: bool) -> Self {
        FileKeyRing {
            path: path.into(),
            broken,
        }
    }

    fn read(&self) -> Result<Vec<(String, String, Vec<u8>)>, KeyRingError> {
        if self.broken {
            return Err(KeyRingError(
                "the keyring cannot be reached (the smoke's broken keyring)".into(),
            ));
        }
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(KeyRingError(e.to_string())),
        };
        Ok(text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split(' ');
                let (s, a, hex) = (parts.next()?, parts.next()?, parts.next()?);
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
                    .collect::<Option<Vec<u8>>>()?;
                Some((s.to_owned(), a.to_owned(), bytes))
            })
            .collect())
    }
}

impl KeyRing for FileKeyRing {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, KeyRingError> {
        Ok(self
            .read()?
            .into_iter()
            .find(|(s, a, _)| s == service && a == account)
            .map(|(_, _, v)| Zeroizing::new(v)))
    }

    fn set(&self, service: &str, account: &str, secret: &[u8]) -> Result<(), KeyRingError> {
        let mut entries = self.read()?;
        entries.retain(|(s, a, _)| !(s == service && a == account));
        entries.push((service.to_owned(), account.to_owned(), secret.to_vec()));
        let text: String = entries
            .iter()
            .map(|(s, a, v)| {
                let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
                format!("{s} {a} {hex}\n")
            })
            .collect();
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|e| KeyRingError(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| KeyRingError(e.to_string()))?;
        }
        file.write_all(text.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_keyring_keeps_entries_across_instances_and_a_broken_one_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring");
        let a = FileKeyRing::new(&path, false);
        assert_eq!(a.get("wubble", "vault-p").unwrap(), None);
        a.set("wubble", "vault-p", &[1, 2, 255]).unwrap();
        a.set("wubble", "vault-q", &[9]).unwrap();
        a.set("wubble", "vault-p", &[3]).unwrap();
        let b = FileKeyRing::new(&path, false);
        assert_eq!(&**b.get("wubble", "vault-p").unwrap().unwrap(), &[3]);
        assert_eq!(&**b.get("wubble", "vault-q").unwrap().unwrap(), &[9]);
        let broken = FileKeyRing::new(&path, true);
        assert!(broken.get("wubble", "vault-p").is_err());
        assert!(broken.set("wubble", "vault-p", &[1]).is_err());
        assert_eq!(&**b.get("wubble", "vault-p").unwrap().unwrap(), &[3]);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_keyring_writes_with_0600_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring");
        let ring = FileKeyRing::new(&path, false);
        ring.set("wubble", "vault-p", &[1, 2, 3]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
