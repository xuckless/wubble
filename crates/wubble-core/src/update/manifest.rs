//! The update manifest, the checks on it and on what it points to: pure functions over bytes.
//!
//! The format is Tauri's updater manifest (tauri-apps/plugins-workspace `plugins/updater`, MIT OR
//! Apache-2.0: a version, a date, notes, and per platform a URL and its minisign signature),
//! with the archive's size and SHA-256 added and a minisign signature over the manifest itself
//! (`<manifest>.minisig`), so a manifest is trusted as a whole before anything in it is read.
//! No code copied. The signed manifest's hash and the archive's own signature are both checked.
//!
//! ```json
//! {"schema":1,"version":"0.1.0-preview.1","date":"2026-11-19T12:00:00Z",
//!  "notes":"https://github.com/xuckless/wubble/releases/tag/v0.1.0-preview.1",
//!  "platforms":{"linux-x86_64":{"url":"https://.../wubble-0.1.0-preview.1-linux-x86_64.tar.xz",
//!    "size":123,"sha256":"<hex>","signature":"<the archive's .minisig text>"}}}
//! ```

use std::collections::BTreeMap;
use std::io::Read;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Channel;

/// The schema this build reads; a manifest of another is refused.
pub const SCHEMA: u64 = 1;
/// The largest manifest read, bytes.
pub const MAX_MANIFEST: u64 = 64 * 1024;
/// The largest archive accepted, bytes, whatever a manifest says.
pub const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
/// The bundle's own description of itself, at the top of a Linux bundle and in a macOS
/// bundle's `Contents/Resources`. An install is self-managed only when it has one.
pub const RELEASE_FILE: &str = "release.json";

/// Why an update was not taken. Shown in Settings as "Update failed: ...".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The signature is not one of the trusted key's.
    BadSignature,
    /// Signed, but by another key.
    WrongKey,
    /// Not a manifest, a signature or a release file this build reads.
    Malformed(String),
    /// A newer release exists, but none for this platform.
    NoBuild(String),
    /// The archive's size or SHA-256 is not the manifest's.
    Tampered,
    /// The bundle says it is another version or for another platform.
    Mismatch(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::BadSignature => f.write_str("the signature does not match"),
            Refusal::WrongKey => f.write_str("signed with another key"),
            Refusal::Malformed(what) => write!(f, "{what}"),
            Refusal::NoBuild(platform) => write!(f, "no build for {platform}"),
            Refusal::Tampered => f.write_str("the download is not the release's"),
            Refusal::Mismatch(what) => write!(f, "{what}"),
        }
    }
}

/// One platform's archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub url: String,
    pub size: u64,
    pub sha256: [u8; 32],
    /// The archive's minisign signature (the `.minisig` file's text).
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    /// RFC 3339, as written; shown, never compared.
    pub date: String,
    /// The release page.
    pub notes: String,
    /// By platform id (`super::PLATFORMS`).
    pub platforms: BTreeMap<String, Artifact>,
}

/// What a check found to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: Version,
    pub artifact: Artifact,
}

fn malformed(what: impl Into<String>) -> Refusal {
    Refusal::Malformed(what.into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let text = text.as_bytes();
    if text.len() != 64 {
        return None;
    }
    let mut out = [0; 32];
    for (i, pair) in text.chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The SHA-256 as lowercase hex (the `.sha256` files and the manifest).
pub fn sha256_hex(digest: &[u8; 32]) -> String {
    hex(digest)
}

impl Manifest {
    /// Reads a manifest. Unknown fields are ignored; a missing or wrong one is refused.
    pub fn parse(bytes: &[u8]) -> Result<Manifest, Refusal> {
        let v: Value =
            serde_json::from_slice(bytes).map_err(|e| malformed(format!("manifest: {e}")))?;
        if v.get("schema").and_then(Value::as_u64) != Some(SCHEMA) {
            return Err(malformed("manifest: unknown schema"));
        }
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let version = text(&v, "version")
            .and_then(|s| Version::parse(&s).ok())
            .ok_or_else(|| malformed("manifest: no version"))?;
        let mut platforms = BTreeMap::new();
        for (id, entry) in v
            .get("platforms")
            .and_then(Value::as_object)
            .ok_or_else(|| malformed("manifest: no platforms"))?
        {
            let bad = || malformed(format!("manifest: bad entry for {id}"));
            platforms.insert(
                id.clone(),
                Artifact {
                    url: text(entry, "url").ok_or_else(bad)?,
                    size: entry.get("size").and_then(Value::as_u64).ok_or_else(bad)?,
                    sha256: text(entry, "sha256")
                        .and_then(|h| unhex(&h))
                        .ok_or_else(bad)?,
                    signature: text(entry, "signature").ok_or_else(bad)?,
                },
            );
        }
        Ok(Manifest {
            version,
            date: text(&v, "date").unwrap_or_default(),
            notes: text(&v, "notes").unwrap_or_default(),
            platforms,
        })
    }

    /// The manifest's text (`wubble-release make-manifest`); [`Manifest::parse`] reads it back.
    pub fn to_json(&self) -> String {
        let platforms: serde_json::Map<String, Value> = self
            .platforms
            .iter()
            .map(|(id, a)| {
                (
                    id.clone(),
                    json!({
                        "url": a.url,
                        "size": a.size,
                        "sha256": hex(&a.sha256),
                        "signature": a.signature,
                    }),
                )
            })
            .collect();
        let v = json!({
            "schema": SCHEMA,
            "version": self.version.to_string(),
            "date": self.date,
            "notes": self.notes,
            "platforms": platforms,
        });
        serde_json::to_string_pretty(&v).expect("a manifest serialises") + "\n"
    }
}

fn signature(text: &str) -> Result<Signature, Refusal> {
    Signature::decode(text).map_err(|_| malformed("not a minisign signature"))
}

fn refusal(e: minisign_verify::Error) -> Refusal {
    match e {
        minisign_verify::Error::UnexpectedKeyId => Refusal::WrongKey,
        minisign_verify::Error::InvalidSignature => Refusal::BadSignature,
        // a legacy (not prehashed) signature, or another algorithm
        other => malformed(format!("signature: {other}")),
    }
}

/// The manifest, once its signature (`.minisig` text) is the key's: nothing of an unsigned
/// manifest is read.
pub fn verify_manifest(bytes: &[u8], sig: &str, key: &PublicKey) -> Result<Manifest, Refusal> {
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(malformed("manifest too large"));
    }
    key.verify(bytes, &signature(sig)?, false)
        .map_err(refusal)?;
    Manifest::parse(bytes)
}

/// What a verified manifest offers this build: `None` when it is not newer than `current`,
/// is the version that already failed here (`skip`), or is a preview on the stable channel.
/// A newer release without this platform's build is refused (said in Settings).
pub fn choose(
    manifest: &Manifest,
    current: &Version,
    platform: &str,
    channel: Channel,
    skip: Option<&Version>,
) -> Result<Option<Offer>, Refusal> {
    let v = &manifest.version;
    if v <= current || !channel.accepts(v) || skip == Some(v) {
        return Ok(None);
    }
    let artifact = manifest
        .platforms
        .get(platform)
        .ok_or_else(|| Refusal::NoBuild(platform.to_owned()))?;
    if artifact.size > MAX_ARCHIVE {
        return Err(malformed("archive too large"));
    }
    Ok(Some(Offer {
        version: v.clone(),
        artifact: artifact.clone(),
    }))
}

/// Whether `archive` is the manifest's: its size and SHA-256, and its own signature by `key`
/// (minisign's prehashed form, read as a stream). Runs before anything is unpacked.
pub fn verify_archive(
    mut archive: impl Read,
    artifact: &Artifact,
    key: &PublicKey,
) -> Result<(), Refusal> {
    let sig = signature(&artifact.signature)?;
    let mut stream = key.verify_stream(&sig).map_err(refusal)?;
    let mut sha = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0; 1 << 16];
    loop {
        let n = archive
            .read(&mut buf)
            .map_err(|e| malformed(format!("cannot read the download: {e}")))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > artifact.size {
            return Err(Refusal::Tampered);
        }
        sha.update(&buf[..n]);
        stream.update(&buf[..n]);
    }
    let digest: [u8; 32] = sha.finalize().into();
    if size != artifact.size || digest != artifact.sha256 {
        return Err(Refusal::Tampered);
    }
    stream.finalize().map_err(refusal)
}

/// A bundle's `release.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: Version,
    pub platform: String,
    pub commit: String,
}

impl ReleaseInfo {
    pub fn parse(text: &str) -> Result<ReleaseInfo, Refusal> {
        let v: Value =
            serde_json::from_str(text).map_err(|e| malformed(format!("{RELEASE_FILE}: {e}")))?;
        let field = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        Ok(ReleaseInfo {
            version: field("version")
                .and_then(|s| Version::parse(&s).ok())
                .ok_or_else(|| malformed(format!("{RELEASE_FILE}: no version")))?,
            platform: field("platform")
                .ok_or_else(|| malformed(format!("{RELEASE_FILE}: no platform")))?,
            commit: field("commit").unwrap_or_default(),
        })
    }

    pub fn to_json(&self) -> String {
        json!({
            "version": self.version.to_string(),
            "platform": self.platform,
            "commit": self.commit,
        })
        .to_string()
    }

    /// An unpacked bundle is the version and platform it was downloaded as.
    pub fn matches(&self, version: &Version, platform: &str) -> Result<(), Refusal> {
        if &self.version != version {
            return Err(Refusal::Mismatch(format!(
                "the bundle is {}, not {version}",
                self.version
            )));
        }
        if self.platform != platform {
            return Err(Refusal::Mismatch(format!(
                "the bundle is for {}, not {platform}",
                self.platform
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A key pair made for the test, and signing with it.
    pub(crate) struct Signer(minisign::KeyPair);

    impl Signer {
        pub(crate) fn new() -> Self {
            Signer(minisign::KeyPair::generate_unencrypted_keypair().unwrap())
        }
        pub(crate) fn key(&self) -> PublicKey {
            PublicKey::from_base64(&self.0.pk.to_base64()).unwrap()
        }
        pub(crate) fn sign(&self, bytes: &[u8]) -> String {
            minisign::sign(Some(&self.0.pk), &self.0.sk, bytes, None, None)
                .unwrap()
                .into_string()
        }
    }

    pub(crate) fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    pub(crate) fn manifest_for(signer: &Signer, version: &str, archive: &[u8]) -> Manifest {
        let mut platforms = BTreeMap::new();
        platforms.insert(
            "linux-x86_64".to_owned(),
            Artifact {
                url: "https://example.test/a.tar.xz".into(),
                size: archive.len() as u64,
                sha256: Sha256::digest(archive).into(),
                signature: signer.sign(archive),
            },
        );
        Manifest {
            version: v(version),
            date: "2026-11-19T00:00:00Z".into(),
            notes: String::new(),
            platforms,
        }
    }

    const ARCHIVE: &[u8] = b"a tiny bundle";

    #[test]
    fn a_manifest_round_trips() {
        let m = manifest_for(&Signer::new(), "0.2.0-preview.1", ARCHIVE);
        assert_eq!(Manifest::parse(m.to_json().as_bytes()), Ok(m));
    }

    #[test]
    fn a_good_signature_is_read() {
        let s = Signer::new();
        let text = manifest_for(&s, "0.2.0", ARCHIVE).to_json();
        let m = verify_manifest(text.as_bytes(), &s.sign(text.as_bytes()), &s.key()).unwrap();
        assert_eq!(m.version, v("0.2.0"));
    }

    #[test]
    fn another_key_is_refused() {
        let (s, other) = (Signer::new(), Signer::new());
        let text = manifest_for(&s, "0.2.0", ARCHIVE).to_json();
        let sig = s.sign(text.as_bytes());
        assert_eq!(
            verify_manifest(text.as_bytes(), &sig, &other.key()),
            Err(Refusal::WrongKey)
        );
    }

    #[test]
    fn a_tampered_manifest_is_refused() {
        let s = Signer::new();
        let text = manifest_for(&s, "0.2.0", ARCHIVE).to_json();
        let sig = s.sign(text.as_bytes());
        let forged = text.replace("0.2.0", "9.0.0");
        assert_eq!(
            verify_manifest(forged.as_bytes(), &sig, &s.key()),
            Err(Refusal::BadSignature)
        );
        assert!(matches!(
            verify_manifest(text.as_bytes(), "junk", &s.key()),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn unknown_schemas_and_bad_entries_are_refused() {
        assert!(Manifest::parse(br#"{"schema":2,"version":"1.0.0","platforms":{}}"#).is_err());
        assert!(
            Manifest::parse(
                br#"{"schema":1,"version":"1.0.0","platforms":{"linux-x86_64":{"url":"u","size":1,"sha256":"zz","signature":"s"}}}"#
            )
            .is_err()
        );
        assert!(Manifest::parse(br#"{"schema":1,"version":"one","platforms":{}}"#).is_err());
    }

    #[test]
    fn only_newer_versions_for_this_platform_and_channel_are_offered() {
        let s = Signer::new();
        let m = manifest_for(&s, "0.2.0-preview.2", ARCHIVE);
        let linux = "linux-x86_64";
        let offer = |current: &str, channel, skip: Option<&Version>| {
            choose(&m, &v(current), linux, channel, skip).map(|o| o.map(|o| o.version))
        };
        assert_eq!(
            offer("0.2.0-preview.1", Channel::Preview, None),
            Ok(Some(v("0.2.0-preview.2")))
        );
        // preview.10 is newer than preview.2: numeric identifiers compare as numbers
        assert_eq!(offer("0.2.0-preview.10", Channel::Preview, None), Ok(None));
        // downgrade refused, the same version is up to date
        assert_eq!(offer("0.2.0", Channel::Preview, None), Ok(None));
        assert_eq!(offer("0.2.0-preview.2", Channel::Preview, None), Ok(None));
        // a preview is not for the stable channel
        assert_eq!(offer("0.1.0", Channel::Stable, None), Ok(None));
        // the version that failed to start here is not offered again
        assert_eq!(
            offer("0.1.0", Channel::Preview, Some(&v("0.2.0-preview.2"))),
            Ok(None)
        );
        // wrong platform refused
        assert_eq!(
            choose(&m, &v("0.1.0"), "macos-aarch64", Channel::Preview, None),
            Err(Refusal::NoBuild("macos-aarch64".into()))
        );
    }

    #[test]
    fn the_archive_must_be_the_manifests() {
        let s = Signer::new();
        let m = manifest_for(&s, "0.2.0", ARCHIVE);
        let a = &m.platforms["linux-x86_64"];
        assert_eq!(verify_archive(ARCHIVE, a, &s.key()), Ok(()));
        // tampered payload, same length
        assert_eq!(
            verify_archive(&b"a tinY bundle"[..], a, &s.key()),
            Err(Refusal::Tampered)
        );
        // longer or shorter
        assert_eq!(
            verify_archive(&b"a tiny bundle!"[..], a, &s.key()),
            Err(Refusal::Tampered)
        );
        assert_eq!(
            verify_archive(&ARCHIVE[1..], a, &s.key()),
            Err(Refusal::Tampered)
        );
        // the hash is the manifest's but the archive's signature is another key's
        assert_eq!(
            verify_archive(ARCHIVE, a, &Signer::new().key()),
            Err(Refusal::WrongKey)
        );
        // the hash matches but the signature is over something else
        let forged = Artifact {
            signature: s.sign(b"something else"),
            ..a.clone()
        };
        assert_eq!(
            verify_archive(ARCHIVE, &forged, &s.key()),
            Err(Refusal::BadSignature)
        );
    }

    #[test]
    fn release_files_round_trip_and_must_match() {
        let info = ReleaseInfo {
            version: v("0.2.0"),
            platform: "linux-x86_64".into(),
            commit: "abc".into(),
        };
        assert_eq!(ReleaseInfo::parse(&info.to_json()), Ok(info.clone()));
        assert_eq!(info.matches(&v("0.2.0"), "linux-x86_64"), Ok(()));
        assert!(info.matches(&v("0.2.1"), "linux-x86_64").is_err());
        assert!(info.matches(&v("0.2.0"), "macos-aarch64").is_err());
        assert!(ReleaseInfo::parse("{}").is_err());
    }
}
