//! Release tooling (docs/release.md): the update key pair, minisign signatures, and the update
//! manifest written from a folder of release archives. The signing workflow
//! (`.github/workflows/release-sign.yml`) runs it with the secret key from the environment;
//! `--smoke update` signs its own fake release with a key pair made for the run.
//!
//! Keys and signatures are minisign's (jedisct1/rust-minisign, MIT, as a library), so the
//! `minisign` CLI makes and checks the same files. The manifest's schema is
//! `wubble_core::update::manifest`'s, its one definition.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};
use wubble_core::update::{Artifact, Manifest, PLATFORMS};

pub use wubble_core::update::Version;
pub use wubble_core::update::manifest::Refusal;

/// A secret key ready to sign, with its public half.
pub struct Key {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

/// A new key pair's files.
pub struct NewKey {
    /// `wubble-update.key`: the secret key box (encrypted when a password was given).
    pub secret: String,
    /// `wubble-update.pub`: the public key box.
    pub public: String,
    /// The public key's base64 line: what goes into `update::key::PUBLIC_KEY`.
    pub base64: String,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Key {
    /// A key pair for this run only (`--smoke update`, tests): never written anywhere.
    pub fn ephemeral() -> Result<Key, String> {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().map_err(err)?;
        Ok(Key {
            pk: pair.pk,
            sk: pair.sk,
        })
    }

    /// A secret key box's text (`wubble-update.key`), decrypted with `password` when it is
    /// encrypted.
    pub fn from_secret(text: &str, password: Option<String>) -> Result<Key, String> {
        let sk_box = minisign::SecretKeyBox::from_string(text).map_err(err)?;
        let sk = match password {
            Some(pw) => sk_box.into_secret_key(Some(pw)),
            None => sk_box.into_unencrypted_secret_key(),
        }
        .map_err(|e| format!("cannot open the secret key: {e}"))?;
        let pk = minisign::PublicKey::from_secret_key(&sk).map_err(err)?;
        Ok(Key { pk, sk })
    }

    pub fn public_base64(&self) -> String {
        self.pk.to_base64()
    }

    /// The `.minisig` text for `data` (minisign's prehashed form). The trusted comment names
    /// the file; nothing reads it back.
    pub fn sign(&self, data: impl Read, file_name: &str) -> Result<String, String> {
        let comment = format!("file:{file_name}");
        minisign::sign(Some(&self.pk), &self.sk, data, Some(&comment), None)
            .map(minisign::SignatureBox::into_string)
            .map_err(err)
    }
}

/// A new key pair, its secret encrypted with `password` when given.
pub fn keygen(password: Option<String>) -> Result<NewKey, String> {
    let encrypted = password.is_some();
    let pair = match password {
        Some(pw) => minisign::KeyPair::generate_encrypted_keypair(Some(pw)),
        None => minisign::KeyPair::generate_unencrypted_keypair(),
    }
    .map_err(err)?;
    let comment = if encrypted {
        "wubble update key (encrypted)"
    } else {
        "wubble update key"
    };
    Ok(NewKey {
        secret: pair.sk.to_box(Some(comment)).map_err(err)?.into_string(),
        public: pair.pk.to_box().map_err(err)?.into_string(),
        base64: pair.pk.to_base64(),
    })
}

/// The update archive's file name for `version` on `platform`.
pub fn archive_name(version: &Version, platform: &str) -> String {
    format!("wubble-{version}-{platform}.tar.xz")
}

/// The manifest for `version` from the archives in `dir` (each with its `.minisig`), their
/// URLs under `base_url`. At least one platform's archive must be there.
pub fn make_manifest(
    version: &Version,
    base_url: &str,
    notes: &str,
    dir: &Path,
    date: &str,
) -> Result<Manifest, String> {
    let mut platforms = std::collections::BTreeMap::new();
    for platform in PLATFORMS {
        let name = archive_name(version, platform);
        let path = dir.join(&name);
        if !path.is_file() {
            continue;
        }
        let sig_path = dir.join(format!("{name}.minisig"));
        let signature = std::fs::read_to_string(&sig_path)
            .map_err(|e| format!("{}: {e}", sig_path.display()))?;
        let mut file = std::fs::File::open(&path).map_err(|e| format!("{name}: {e}"))?;
        let (mut sha, mut size, mut buf) = (Sha256::new(), 0u64, vec![0; 1 << 16]);
        loop {
            let n = file.read(&mut buf).map_err(|e| format!("{name}: {e}"))?;
            if n == 0 {
                break;
            }
            sha.update(&buf[..n]);
            size += n as u64;
        }
        platforms.insert(
            platform.to_owned(),
            Artifact {
                url: format!("{}/{name}", base_url.trim_end_matches('/')),
                size,
                sha256: sha.finalize().into(),
                signature,
            },
        );
    }
    if platforms.is_empty() {
        return Err(format!(
            "no update archive (wubble-{version}-<platform>.tar.xz) in {}",
            dir.display()
        ));
    }
    Ok(Manifest {
        version: version.clone(),
        date: date.to_owned(),
        notes: notes.to_owned(),
        platforms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::update::{key, manifest};

    #[test]
    fn signatures_verify_with_the_core_checks() {
        let k = Key::ephemeral().unwrap();
        let public = key::parse(&k.public_base64()).unwrap();
        let dir = std::env::temp_dir().join(format!("wubble-release-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v = Version::parse("0.3.0-preview.1").unwrap();
        let name = archive_name(&v, "linux-x86_64");
        std::fs::write(dir.join(&name), b"archive").unwrap();
        let sig = k.sign(&b"archive"[..], &name).unwrap();
        std::fs::write(dir.join(format!("{name}.minisig")), &sig).unwrap();
        let m = make_manifest(&v, "https://h.test/d/", "notes", &dir, "date").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let text = m.to_json();
        let signed = manifest::verify_manifest(
            text.as_bytes(),
            &k.sign(text.as_bytes(), "latest.json").unwrap(),
            &public,
        )
        .unwrap();
        let a = &signed.platforms["linux-x86_64"];
        assert_eq!(a.url, format!("https://h.test/d/{name}"));
        assert_eq!(
            manifest::verify_archive(&b"archive"[..], a, &public),
            Ok(())
        );
    }

    /// (An encrypted key costs scrypt's 1 GiB per open, too much for a unit test; the
    /// workflow's first run opens one.)
    #[test]
    fn a_new_key_opens_and_signs() {
        let plain = keygen(None).unwrap();
        let k = Key::from_secret(&plain.secret, None).unwrap();
        assert_eq!(k.public_base64(), plain.base64);
        assert!(plain.public.contains(&plain.base64));
        assert!(Key::from_secret("not a key", None).is_err());
    }

    /// install.sh carries the same public key as the binary (docs/release.md).
    #[test]
    fn install_sh_trusts_the_binarys_key() {
        let script = include_str!("../../../scripts/release/install.sh");
        let line = script
            .lines()
            .find_map(|l| l.strip_prefix("PUBLIC_KEY='"))
            .and_then(|l| l.strip_suffix('\''))
            .expect("install.sh has a PUBLIC_KEY='...' line");
        assert_eq!(line, key::PUBLIC_KEY);
    }
}
