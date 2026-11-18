//! Per-value AES-256-GCM: `nonce(12) || ciphertext || tag(16)`.
//!
//! Layout copied from Chromium `components/os_crypt/async/common/encryptor.cc`
//! (fresh random 96-bit nonce per value, prepended). Unlike Chromium we bind
//! associated data: a version tag, the record id, the record origin and the
//! field name, so blobs cannot be swapped between rows or fields.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroizing;

pub const NONCE_LEN: usize = 12;
const VERSION_TAG: &[u8] = b"wubble-vault/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Username,
    Password,
}

impl Field {
    fn tag(self) -> &'static [u8] {
        match self {
            Field::Username => b"username",
            Field::Password => b"password",
        }
    }
}

/// One failure for every way decryption can go wrong (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("could not decrypt")]
    Decrypt,
    #[error("could not encrypt")]
    Encrypt,
}

fn aad(id: &str, origin: &str, field: Field) -> Vec<u8> {
    let mut out = VERSION_TAG.to_vec();
    for part in [id.as_bytes(), origin.as_bytes()] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part);
    }
    out.extend_from_slice(field.tag());
    out
}

pub fn seal(
    key: &[u8; 32],
    id: &str,
    origin: &str,
    field: Field,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let aad = aad(id, origin, field);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::Encrypt)?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open(
    key: &[u8; 32],
    id: &str,
    origin: &str,
    field: Field,
    blob: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if blob.len() < NONCE_LEN {
        return Err(CryptoError::Decrypt);
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let aad = aad(id, origin, field);
    Aes256Gcm::new(key.into())
        .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: &aad })
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Decrypt)
}

/// A fresh random 256-bit key.
pub fn generate_key() -> Zeroizing<[u8; 32]> {
    Zeroizing::new(Aes256Gcm::generate_key(OsRng).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const KEY: [u8; 32] = [9; 32];

    fn sealed() -> Vec<u8> {
        seal(&KEY, "id1", "https://a.test", Field::Password, b"hunter2").unwrap()
    }

    #[test]
    fn round_trip() {
        let blob = sealed();
        let pt = open(&KEY, "id1", "https://a.test", Field::Password, &blob).unwrap();
        assert_eq!(&**pt, b"hunter2");
        assert!(!blob.windows(7).any(|w| w == b"hunter2"));
    }

    #[test]
    fn wrong_key_fails() {
        assert_eq!(
            open(
                &[1; 32],
                "id1",
                "https://a.test",
                Field::Password,
                &sealed()
            )
            .unwrap_err(),
            CryptoError::Decrypt
        );
    }

    #[test]
    fn every_flipped_bit_region_fails() {
        let blob = sealed();
        for pos in [0, NONCE_LEN, blob.len() - 1] {
            let mut bad = blob.clone();
            bad[pos] ^= 1;
            assert!(open(&KEY, "id1", "https://a.test", Field::Password, &bad).is_err());
        }
    }

    #[test]
    fn wrong_associated_data_fails() {
        let blob = sealed();
        assert!(open(&KEY, "id2", "https://a.test", Field::Password, &blob).is_err());
        assert!(open(&KEY, "id1", "https://b.test", Field::Password, &blob).is_err());
        assert!(open(&KEY, "id1", "https://a.test", Field::Username, &blob).is_err());
    }

    #[test]
    fn id_and_origin_are_length_prefixed() {
        let blob = seal(&KEY, "ab", "c", Field::Password, b"x").unwrap();
        assert!(open(&KEY, "a", "bc", Field::Password, &blob).is_err());
    }

    #[test]
    fn short_blob_fails() {
        assert!(open(&KEY, "id1", "o", Field::Password, &[0; 11]).is_err());
        assert!(open(&KEY, "id1", "o", Field::Password, &[]).is_err());
    }

    #[test]
    fn nonces_are_unique() {
        let nonces: HashSet<Vec<u8>> = (0..1000).map(|_| sealed()[..NONCE_LEN].to_vec()).collect();
        assert_eq!(nonces.len(), 1000);
    }
}
