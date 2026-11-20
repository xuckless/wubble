//! Chrome extensions the user installed: the list as data, the one owner of which extensions
//! Chromium loads. Chromium owns their running state; it does not keep extensions loaded from
//! the command line between runs (`installed_loader.cc`, "Skip extensions that were loaded from
//! the command-line"), so this list is all there is. Every change applies at the next start:
//! the list becomes `--load-extension` (`wubble_engine::app`), read only when CEF starts. So
//! nothing is deleted while running: the running Chromium keeps reading a removed or replaced
//! copy's files (its popup, its service worker restarting); [`unused_copies`] are deleted at
//! the next start, before CEF loads.
//!
//! Ported:
//! - Extension ids ([`id_from_key`], [`id_from_path`]): Chromium
//!   `components/crx_file/id_util.cc` (`GenerateId`, `GenerateIdFromHash`,
//!   `ConvertHexadecimalToIDAlphabet`, `GenerateIdForPath`, `MaybeNormalizePath`), test vectors
//!   from `id_util_unittest.cc`; the key-or-path rule from `extensions/common/extension.cc`
//!   (`ComputeExtensionID`). BSD-3-Clause.
//! - CRX3 parsing ([`parse_crx`]) and the manifest key that keeps a .crx's id when it is loaded
//!   unpacked ([`manifest_with_key`]): samuelmaddock/electron-browser-shell @3817c42,
//!   `packages/electron-chrome-web-store/src/browser/installer.ts` (`parseCrx`, `unpackCrx`),
//!   MIT; CRX2 dropped, the `pbf` reader replaced by a small varint reader. Field numbers from
//!   Chromium `components/crx_file/crx3.proto` (BSD-3-Clause). As there, the signature is not
//!   verified: the declared id must match a key in the file, which stops a renamed or mixed-up
//!   file, not a forged one.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

mod projection;
pub use projection::{ExtTab, ExtWindow, ExtWindowKind, Projection, project};

/// A Chrome extension id: 32 letters `a` to `p`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtensionId(String);

impl ExtensionId {
    /// `s` if it is an id's shape (Chromium `IdIsValid`, lowercase only).
    pub fn parse(s: &str) -> Option<Self> {
        (s.len() == 2 * ID_BYTES && s.bytes().all(|b| (b'a'..=b'p').contains(&b)))
            .then(|| Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Bytes of the hash an id is made of (`kIdSize`).
const ID_BYTES: usize = 16;

/// The id of the first [`ID_BYTES`] of `hash`, each hex digit `0..f` written `a..p`.
fn id_from_hash(hash: &[u8]) -> ExtensionId {
    let mut id = String::with_capacity(2 * ID_BYTES);
    for byte in &hash[..ID_BYTES] {
        for nibble in [byte >> 4, byte & 0xf] {
            id.push(char::from(b'a' + nibble));
        }
    }
    ExtensionId(id)
}

/// The id of an extension with this public key (DER SubjectPublicKeyInfo): `GenerateId`.
pub fn id_from_key(der: &[u8]) -> ExtensionId {
    id_from_hash(&Sha256::digest(der))
}

/// The id of an unpacked extension without a `key`, from its directory as Chromium sees it
/// (absolute, symlinks resolved): `GenerateIdForPath`. The path's bytes; on Windows its UTF-16
/// code units with a lower-case drive letter made upper-case.
pub fn id_from_path(dir: &Path) -> ExtensionId {
    #[cfg(windows)]
    let bytes: Vec<u8> = {
        use std::os::windows::ffi::OsStrExt;
        let mut units: Vec<u16> = dir.as_os_str().encode_wide().collect();
        if units.len() >= 2 && units[1] == u16::from(b':') && (0x61..=0x7a).contains(&units[0]) {
            units[0] -= 0x20;
        }
        units.iter().flat_map(|u| u.to_le_bytes()).collect()
    };
    #[cfg(not(windows))]
    let bytes: Vec<u8> = {
        use std::os::unix::ffi::OsStrExt;
        dir.as_os_str().as_bytes().to_vec()
    };
    id_from_key(&bytes)
}

/// Why an extension could not be read or installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtError {
    Manifest(String),
    Crx(String),
    /// A file in the archive would land outside the extension's directory, or has an odd name.
    BadEntry(String),
    /// Chromium splits `--load-extension` on `,`, and the list is stored as UTF-8 text.
    UnusablePath(PathBuf),
    Io(String),
}

impl fmt::Display for ExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtError::Manifest(e) => write!(f, "manifest.json: {e}"),
            ExtError::Crx(e) => write!(f, "not a usable .crx file: {e}"),
            ExtError::BadEntry(name) => write!(f, "the archive has an unsafe entry {name:?}"),
            ExtError::UnusablePath(p) => write!(
                f,
                "{} cannot be loaded: the path has a comma or is not UTF-8",
                p.display()
            ),
            ExtError::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ExtError {}

/// What wubble reads from an extension's `manifest.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// As written: a `__MSG_name__` placeholder is shown raw for now.
    pub name: String,
    pub version: String,
    /// What it is granted at load (nothing asks first): `permissions`, `host_permissions` and
    /// every content script's `matches`, in that order, each once.
    pub permissions: Vec<String>,
    /// The base64 public key, when the manifest pins its id.
    pub key: Option<String>,
}

impl Manifest {
    /// The id Chromium gives this manifest's extension loaded from `dir` (`ComputeExtensionID`):
    /// from the key if there is one, else from the path.
    pub fn id(&self, dir: &Path) -> Result<ExtensionId, ExtError> {
        match &self.key {
            Some(key) => base64::engine::general_purpose::STANDARD
                .decode(key.trim())
                .map(|der| id_from_key(&der))
                .map_err(|e| ExtError::Manifest(format!("key: {e}"))),
            None => Ok(id_from_path(dir)),
        }
    }
}

/// Whether `v` is a version Chromium takes (`base::Version`: one to four dot-separated
/// numbers). Also what keeps it safe as a directory name.
fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() <= 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Reads a Manifest V3 `manifest.json`; older manifests are refused (Chromium 156 no longer
/// runs Manifest V2).
pub fn parse_manifest(json: &str) -> Result<Manifest, ExtError> {
    let bad = |e: &str| ExtError::Manifest(e.to_owned());
    // Chromium allows a byte order mark.
    let v: Value = serde_json::from_str(json.trim_start_matches('\u{feff}'))
        .map_err(|e| ExtError::Manifest(e.to_string()))?;
    let text = |k: &str| v.get(k).and_then(Value::as_str);
    if v.get("manifest_version").and_then(Value::as_u64) != Some(3) {
        return Err(bad("only manifest_version 3 is supported"));
    }
    let name = text("name")
        .filter(|n| !n.is_empty())
        .ok_or(bad("no name"))?;
    let version = text("version").ok_or(bad("no version"))?;
    if !valid_version(version) {
        return Err(bad("version is not one to four numbers"));
    }
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect()
    };
    let mut permissions = strings(v.get("permissions"));
    permissions.extend(strings(v.get("host_permissions")));
    for script in v
        .get("content_scripts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        permissions.extend(strings(script.get("matches")));
    }
    let mut seen = std::collections::HashSet::new();
    permissions.retain(|p| seen.insert(p.clone()));
    Ok(Manifest {
        name: name.to_owned(),
        version: version.to_owned(),
        permissions,
        key: text("key").map(str::to_owned),
    })
}

/// `json` with `"key"` set to `public_key` in base64, every other field kept: an unpacked
/// copy of a .crx then keeps the .crx's id.
pub fn manifest_with_key(json: &str, public_key: &[u8]) -> Result<String, ExtError> {
    let mut v: Map<String, Value> = serde_json::from_str(json.trim_start_matches('\u{feff}'))
        .map_err(|e| ExtError::Manifest(e.to_string()))?;
    v.insert(
        "key".into(),
        Value::String(base64::engine::general_purpose::STANDARD.encode(public_key)),
    );
    Ok(serde_json::to_string_pretty(&v).expect("a JSON object serialises"))
}

/// A parsed CRX3 file, borrowing from its bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Crx<'a> {
    pub id: ExtensionId,
    /// The `sha256_with_rsa` key whose id is the declared one.
    pub public_key: &'a [u8],
    /// The zip archive after the header.
    pub zip: &'a [u8],
}

const CRX_MAGIC: &[u8; 4] = b"Cr24";

/// The length-delimited fields of a protobuf message, `(field number, bytes)`, in order; other
/// wire types are skipped. `None` when the message is malformed.
fn proto_fields(mut buf: &[u8]) -> Option<Vec<(u64, &[u8])>> {
    fn varint(buf: &mut &[u8]) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = buf.split_first()?;
            *buf = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    let mut out = vec![];
    while !buf.is_empty() {
        let tag = varint(&mut buf)?;
        let skip = match tag & 7 {
            0 => {
                varint(&mut buf)?;
                0
            }
            1 => 8,
            2 => {
                let len = usize::try_from(varint(&mut buf)?).ok()?;
                let field = buf.get(..len)?;
                out.push((tag >> 3, field));
                len
            }
            5 => 4,
            _ => return None,
        };
        buf = buf.get(skip..)?;
    }
    Some(out)
}

/// The first length-delimited field `number` of `fields`.
fn field<'a>(fields: &[(u64, &'a [u8])], number: u64) -> Option<&'a [u8]> {
    fields.iter().find(|(f, _)| *f == number).map(|(_, b)| *b)
}

/// `CrxFileHeader` (crx3.proto) field numbers.
const HEADER_SHA256_WITH_RSA: u64 = 2;
const HEADER_SIGNED_HEADER_DATA: u64 = 10000;
/// `AsymmetricKeyProof.public_key`, `SignedData.crx_id`.
const PROOF_PUBLIC_KEY: u64 = 1;
const SIGNED_CRX_ID: u64 = 1;

/// Reads a CRX3 file's header: its declared id, the matching key and the zip after it.
pub fn parse_crx(bytes: &[u8]) -> Result<Crx<'_>, ExtError> {
    let bad = |e: &str| ExtError::Crx(e.to_owned());
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if bytes.get(..4) != Some(CRX_MAGIC) {
        return Err(bad("it does not start with Cr24"));
    }
    match u32_at(4) {
        Some(3) => {}
        Some(v) => return Err(ExtError::Crx(format!("format version {v}, not 3"))),
        None => return Err(bad("truncated")),
    }
    let header_len = u32_at(8).ok_or(bad("truncated"))? as usize;
    let header = bytes
        .get(12..12 + header_len)
        .ok_or(bad("the header is truncated"))?;
    let zip = &bytes[12 + header_len..];
    let fields = proto_fields(header).ok_or(bad("the header is malformed"))?;
    let signed = field(&fields, HEADER_SIGNED_HEADER_DATA).ok_or(bad("no signed data"))?;
    let crx_id = proto_fields(signed)
        .and_then(|s| field(&s, SIGNED_CRX_ID))
        .filter(|id| id.len() == ID_BYTES)
        .ok_or(bad("no crx id"))?;
    let id = id_from_hash(crx_id);
    let public_key = fields
        .iter()
        .filter(|(f, _)| *f == HEADER_SHA256_WITH_RSA)
        .filter_map(|(_, proof)| field(&proto_fields(proof)?, PROOF_PUBLIC_KEY))
        .find(|key| id_from_key(key) == id)
        .ok_or(bad("no key matches its id"))?;
    Ok(Crx {
        id,
        public_key,
        zip,
    })
}

/// A CRX3 file of `zip` with `public_key` as its one RSA key and the id it makes, signature
/// empty: what [`parse_crx`] reads back. For tests and the extensions smoke only; Chromium's own
/// installer would refuse it.
pub fn crx3_unsigned(public_key: &[u8], zip: &[u8]) -> Vec<u8> {
    fn field(out: &mut Vec<u8>, number: u64, bytes: &[u8]) {
        let mut varint = |mut v: u64| loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            out.push(if v == 0 { byte } else { byte | 0x80 });
            if v == 0 {
                break;
            }
        };
        varint(number << 3 | 2);
        varint(bytes.len() as u64);
        out.extend_from_slice(bytes);
    }
    let mut proof = vec![];
    field(&mut proof, PROOF_PUBLIC_KEY, public_key);
    let mut signed = vec![];
    field(
        &mut signed,
        SIGNED_CRX_ID,
        &Sha256::digest(public_key)[..ID_BYTES],
    );
    let mut header = vec![];
    field(&mut header, HEADER_SHA256_WITH_RSA, &proof);
    field(&mut header, HEADER_SIGNED_HEADER_DATA, &signed);
    let mut out = CRX_MAGIC.to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend((header.len() as u32).to_le_bytes());
    out.extend(header);
    out.extend_from_slice(zip);
    out
}

/// A zip of `files` (`(name, bytes)`), stored without compression: with [`crx3_unsigned`], a
/// .crx for tests and the extensions smoke. Format: PKWARE APPNOTE.TXT 4.3 (local headers, the
/// central directory, its end record); CRC-32 with the reflected polynomial 0xEDB88320.
pub fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
            }
        }
        !crc
    }
    let mut out = vec![];
    let mut central = vec![];
    for (name, data) in files {
        let (crc, size, name_len) = (crc32(data), data.len() as u32, name.len() as u16);
        let offset = out.len() as u32;
        // version 2.0, no flags, stored, no time
        let common = |v: &mut Vec<u8>| {
            v.extend(20u16.to_le_bytes());
            v.extend([0u8; 8]);
            v.extend(crc.to_le_bytes());
            v.extend(size.to_le_bytes());
            v.extend(size.to_le_bytes());
            v.extend(name_len.to_le_bytes());
            v.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend_from_slice(data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        common(&mut central);
        // comment length, disk, internal and external attributes, offset
        central.extend([0u8; 2 + 2 + 2 + 4]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let (at, len, count) = (out.len() as u32, central.len() as u32, files.len() as u16);
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(len.to_le_bytes());
    out.extend(at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// Where an archive entry goes inside the extension's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Dir(PathBuf),
    File(PathBuf),
}

/// The relative path of the zip entry `name`, or why it is refused: absolute names, `..`,
/// backslashes, drive letters and empty parts could write outside the directory ("zip slip").
pub fn zip_entry(name: &str) -> Result<Entry, ExtError> {
    let refuse = || ExtError::BadEntry(name.to_owned());
    let (path, dir) = match name.strip_suffix('/') {
        Some(dir) => (dir, true),
        None => (name, false),
    };
    if path.is_empty() || path.contains(['\\', ':', '\0']) {
        return Err(refuse());
    }
    let mut rel = PathBuf::new();
    for part in path.split('/') {
        match Path::new(part).components().next() {
            Some(Component::Normal(_)) if Path::new(part).components().count() == 1 => {
                rel.push(part)
            }
            _ => return Err(refuse()),
        }
    }
    Ok(if dir {
        Entry::Dir(rel)
    } else {
        Entry::File(rel)
    })
}

/// Where an installed extension's files are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The user's own directory, loaded in place; never deleted by wubble.
    Unpacked { dir: PathBuf },
    /// A .crx file the user picked (`file`, kept for the UI only), unpacked by wubble into
    /// `dir`, which wubble owns.
    Crx { file: PathBuf, dir: PathBuf },
}

/// One installed extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub id: ExtensionId,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub source: Source,
    /// What it was granted ([`Manifest::permissions`]).
    pub permissions: Vec<String>,
}

impl Extension {
    /// Enabled, from `manifest` as read in `source`'s directory.
    pub fn new(id: ExtensionId, manifest: Manifest, source: Source) -> Self {
        Self {
            id,
            name: manifest.name,
            version: manifest.version,
            enabled: true,
            source,
            permissions: manifest.permissions,
        }
    }

    /// The directory Chromium loads.
    pub fn dir(&self) -> &Path {
        match &self.source {
            Source::Unpacked { dir } | Source::Crx { dir, .. } => dir,
        }
    }

    /// The directory wubble owns: deleted at the next start once nothing names it.
    fn owned_dir(&self) -> Option<&Path> {
        match &self.source {
            Source::Unpacked { .. } => None,
            Source::Crx { dir, .. } => Some(dir),
        }
    }

    fn to_json(&self) -> Value {
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let source = match &self.source {
            Source::Unpacked { dir } => json!({ "kind": "unpacked", "dir": path(dir) }),
            Source::Crx { file, dir } => {
                json!({ "kind": "crx", "file": path(file), "dir": path(dir) })
            }
        };
        json!({
            "id": self.id.as_str(),
            "name": self.name,
            "version": self.version,
            "enabled": self.enabled,
            "source": source,
            "permissions": self.permissions,
        })
    }

    fn from_json(v: &Value) -> Option<Self> {
        let text = |v: &Value, k: &str| v.get(k)?.as_str().map(str::to_owned);
        let s = v.get("source")?;
        let path = |k: &str| text(s, k).map(PathBuf::from);
        let source = match s.get("kind")?.as_str()? {
            "unpacked" => Source::Unpacked { dir: path("dir")? },
            "crx" => Source::Crx {
                file: path("file")?,
                dir: path("dir")?,
            },
            _ => return None,
        };
        Some(Self {
            id: ExtensionId::parse(v.get("id")?.as_str()?)?,
            name: text(v, "name")?,
            version: text(v, "version")?,
            enabled: v.get("enabled")?.as_bool()?,
            source,
            permissions: v
                .get("permissions")?
                .as_array()?
                .iter()
                .map(|p| p.as_str().map(str::to_owned))
                .collect::<Option<_>>()?,
        })
    }
}

/// Every installed extension, in id order (the one order everything derives from).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installed(Vec<Extension>);

impl Installed {
    pub fn extensions(&self) -> &[Extension] {
        &self.0
    }

    pub fn get(&self, id: &ExtensionId) -> Option<&Extension> {
        self.0.iter().find(|e| &e.id == id)
    }

    /// The stored text: a JSON array, one line.
    pub fn to_json(&self) -> String {
        Value::Array(self.0.iter().map(Extension::to_json).collect()).to_string()
    }

    /// The list stored as `text`; `None` when it is not a JSON array. An entry that cannot be
    /// read is dropped, the rest kept.
    pub fn from_json(text: &str) -> Option<Self> {
        let Value::Array(items) = serde_json::from_str(text).ok()? else {
            return None;
        };
        let mut list: Vec<Extension> = items.iter().filter_map(Extension::from_json).collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list.dedup_by(|a, b| a.id == b.id);
        Some(Self(list))
    }
}

/// What the user does to the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A new extension, or a new copy of one installed (same id), which keeps its enabled flag.
    Install(Extension),
    Enable(ExtensionId),
    Disable(ExtensionId),
    Remove(ExtensionId),
}

/// What the edge does after a change. Every change takes effect at the next start, and a copy
/// the list stops naming is deleted then ([`unused_copies`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Store the list (`settings::EXTENSIONS_INSTALLED`).
    Save(Installed),
    /// The event was refused, the list is unchanged.
    Refused(ExtError),
}

/// Whether Chromium can be given `dir` in `--load-extension` and the list can store it.
fn loadable(dir: &Path) -> Result<(), ExtError> {
    match dir.to_str() {
        Some(s) if !s.contains(',') => Ok(()),
        _ => Err(ExtError::UnusablePath(dir.to_path_buf())),
    }
}

/// The list after `event`, and what to do about it: one `Save` per change; nothing for an
/// event that changes nothing (an unknown id, enabling an enabled extension), `Refused` for an
/// install Chromium could not load.
pub fn transition(list: Installed, event: Event) -> (Installed, Vec<Effect>) {
    let before = list.clone();
    let Installed(mut items) = list;
    let at = |items: &[Extension], id: &ExtensionId| items.iter().position(|e| &e.id == id);
    match event {
        Event::Install(mut ext) => {
            // Only the directory reaches Chromium; a .crx file's own path is never loaded.
            if let Err(e) = loadable(ext.dir()) {
                return (before, vec![Effect::Refused(e)]);
            }
            match at(&items, &ext.id) {
                Some(i) => {
                    ext.enabled = items[i].enabled;
                    items[i] = ext;
                }
                None => {
                    ext.enabled = true;
                    items.push(ext);
                    items.sort_by(|a, b| a.id.cmp(&b.id));
                }
            }
        }
        Event::Enable(id) => {
            if let Some(i) = at(&items, &id) {
                items[i].enabled = true;
            }
        }
        Event::Disable(id) => {
            if let Some(i) = at(&items, &id) {
                items[i].enabled = false;
            }
        }
        Event::Remove(id) => {
            if let Some(i) = at(&items, &id) {
                items.remove(i);
            }
        }
    }
    let after = Installed(items);
    if after == before {
        return (after, vec![]);
    }
    (after.clone(), vec![Effect::Save(after)])
}

/// The copies in wubble's extensions directory (`found`, every `<id>/<version>` there) that
/// `list` does not name: replaced, removed, refused or half-unpacked ones. The edge deletes them
/// at start, before CEF loads, when no running Chromium can still be reading them.
pub fn unused_copies(list: &Installed, found: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let named: std::collections::HashSet<&Path> =
        list.0.iter().filter_map(Extension::owned_dir).collect();
    found
        .into_iter()
        .filter(|d| !named.contains(d.as_path()))
        .collect()
}

/// The directories Chromium loads: the enabled extensions', in id order.
pub fn load_paths(list: &Installed) -> Vec<PathBuf> {
    list.0
        .iter()
        .filter(|e| e.enabled)
        .map(|e| e.dir().to_path_buf())
        .collect()
}

/// Whether the list differs from what this run loaded (`at_start`, the list when CEF started):
/// the UI's "restart to apply". Every enabled entry counts, not only its directory: a new copy
/// in the same directory (an unpacked extension edited and installed again) is only read at
/// start too.
pub fn restart_needed(at_start: &Installed, now: &Installed) -> bool {
    let loaded = |list: &Installed| -> Vec<Extension> {
        list.0.iter().filter(|e| e.enabled).cloned().collect()
    };
    loaded(at_start) != loaded(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
        "manifest_version": 3,
        "name": "Mark",
        "version": "1.2.0",
        "permissions": ["storage", "scripting"],
        "host_permissions": ["https://*.example.com/*", "storage"],
        "content_scripts": [{"matches": ["http://127.0.0.1/*"], "js": ["c.js"]}],
        "background": {"service_worker": "w.js"}
    }"#;

    #[test]
    fn ids_match_chromiums_vectors() {
        // id_util_unittest.cc: GenerateId("test"), GenerateId("_"), GenerateIdForPath.
        assert_eq!(
            id_from_key(b"test").as_str(),
            "jpignaibiiemhngfjkcpokkamffknabf"
        );
        assert_eq!(
            id_from_key(b"_").as_str(),
            "ncocknphbhhlhkikpnnlmbcnbgdempcd"
        );
        #[cfg(not(windows))]
        let want = "lnkgfdknojmdambfcanadbhmfjfljobb";
        #[cfg(windows)]
        let want = "jjlkojfgbeklddcpckipekckcmgcbfjn";
        assert_eq!(id_from_path(Path::new("/path/to/file.ext")).as_str(), want);
        assert!(ExtensionId::parse(want).is_some());
        assert!(ExtensionId::parse("zzlkojfgbeklddcpckipekckcmgcbfjn").is_none());
        assert!(ExtensionId::parse("abc").is_none());
    }

    #[test]
    fn manifests_merge_what_is_granted() {
        let m = parse_manifest(MANIFEST).unwrap();
        assert_eq!((m.name.as_str(), m.version.as_str()), ("Mark", "1.2.0"));
        assert_eq!(
            m.permissions,
            [
                "storage",
                "scripting",
                "https://*.example.com/*",
                "http://127.0.0.1/*"
            ]
        );
        assert_eq!(m.key, None);
        assert_eq!(
            m.id(Path::new("/x")).unwrap(),
            id_from_path(Path::new("/x"))
        );
        let v2 = MANIFEST.replace("\"manifest_version\": 3", "\"manifest_version\": 2");
        assert!(parse_manifest(&v2).is_err());
        for version in ["../../x", "1..2", "", "1.2.3.4.5", "a"] {
            let bad = MANIFEST.replace("1.2.0", version);
            assert!(parse_manifest(&bad).is_err(), "{version:?}");
        }
        assert!(parse_manifest("{").is_err());
        assert!(parse_manifest(r#"{"manifest_version": 3, "version": "1"}"#).is_err());
    }

    #[test]
    fn the_key_keeps_the_crx_id() {
        let with = manifest_with_key(MANIFEST, b"test").unwrap();
        let m = parse_manifest(&with).unwrap();
        assert_eq!(m.key.as_deref(), Some("dGVzdA=="));
        assert_eq!(m.id(Path::new("/anywhere")).unwrap(), id_from_key(b"test"));
        assert_eq!(m.name, "Mark");
        // A second key replaces the first.
        let again = parse_manifest(&manifest_with_key(&with, b"_").unwrap()).unwrap();
        assert_eq!(again.id(Path::new("/")).unwrap(), id_from_key(b"_"));
        let broken = MANIFEST.replace("\"name\"", "\"key\": \"%%%\", \"name\"");
        assert!(parse_manifest(&broken).unwrap().id(Path::new("/")).is_err());
    }

    #[test]
    fn crx3_files_parse_and_bad_ones_are_refused() {
        let crx = crx3_unsigned(b"public key", b"PK zip bytes");
        let parsed = parse_crx(&crx).unwrap();
        assert_eq!(parsed.id, id_from_key(b"public key"));
        assert_eq!(parsed.public_key, b"public key");
        assert_eq!(parsed.zip, b"PK zip bytes");

        let mut magic = crx.clone();
        magic[0] = b'X';
        assert!(parse_crx(&magic).is_err());
        let mut v2 = crx.clone();
        v2[4] = 2;
        assert_eq!(
            parse_crx(&v2),
            Err(ExtError::Crx("format version 2, not 3".into()))
        );
        assert!(parse_crx(&crx[..20]).is_err());
        assert!(parse_crx(&crx[..6]).is_err());
        // The declared id is another key's: no key matches it. The crx id is the header's last
        // bytes.
        let other = crx3_unsigned(b"other key!", b"PK zip bytes");
        assert_eq!(other.len(), crx.len());
        let id_at = crx.len() - b"PK zip bytes".len() - ID_BYTES;
        let mut mixed = crx.clone();
        mixed[id_at..id_at + ID_BYTES].copy_from_slice(&other[id_at..id_at + ID_BYTES]);
        assert_eq!(
            parse_crx(&mixed),
            Err(ExtError::Crx("no key matches its id".into()))
        );
    }

    #[test]
    fn stored_zips_have_the_known_crc() {
        // `printf 123456789 | gzip | tail -c8 | head -c4`: CRC-32 0xCBF43926.
        let zip = zip_stored(&[("a.txt", b"123456789")]);
        assert_eq!(&zip[..4], b"PK\x03\x04");
        assert_eq!(&zip[14..18], &0xcbf4_3926u32.to_le_bytes());
        assert_eq!(&zip[zip.len() - 22..zip.len() - 18], b"PK\x05\x06");
    }

    #[test]
    fn zip_entries_stay_inside() {
        assert_eq!(
            zip_entry("js/c.js"),
            Ok(Entry::File(PathBuf::from("js").join("c.js")))
        );
        assert_eq!(zip_entry("icons/"), Ok(Entry::Dir(PathBuf::from("icons"))));
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "a\\b",
            "C:/x",
            "a//b",
            "",
            "/",
            "./a",
            "a/./b",
        ] {
            assert!(zip_entry(bad).is_err(), "{bad:?}");
        }
    }

    fn ext(id: &str, source: Source) -> Extension {
        Extension {
            id: id_from_key(id.as_bytes()),
            name: id.into(),
            version: "1".into(),
            enabled: true,
            source,
            permissions: vec!["storage".into()],
        }
    }

    fn unpacked(name: &str) -> Extension {
        ext(
            name,
            Source::Unpacked {
                dir: PathBuf::from(format!("/src/{name}")),
            },
        )
    }

    fn crx(name: &str, version: &str) -> Extension {
        ext(
            name,
            Source::Crx {
                file: PathBuf::from(format!("/dl/{name}.crx")),
                dir: PathBuf::from(format!("/p/extensions/{name}/{version}")),
            },
        )
    }

    fn saved(list: &Installed) -> Effect {
        Effect::Save(list.clone())
    }

    #[test]
    fn install_enable_disable_remove() {
        let (list, fx) = transition(Installed::default(), Event::Install(unpacked("a")));
        assert_eq!(list.extensions().len(), 1);
        assert_eq!(fx, [saved(&list)]);
        let (list, fx) = transition(list, Event::Install(crx("b", "1")));
        assert_eq!(fx, [saved(&list)]);
        let a = id_from_key(b"a");
        let b = id_from_key(b"b");

        let (list, fx) = transition(list, Event::Disable(a.clone()));
        assert!(!list.get(&a).unwrap().enabled);
        assert_eq!(fx, [saved(&list)]);
        // Again: nothing changes, nothing is saved.
        let (list, fx) = transition(list, Event::Disable(a.clone()));
        assert!(fx.is_empty());
        // A new copy keeps the disabled flag; an unpacked dir is never deleted.
        let mut newer = unpacked("a");
        newer.version = "2".into();
        let (list, fx) = transition(list, Event::Install(newer));
        let got = list.get(&a).unwrap();
        assert!(!got.enabled && got.version == "2");
        assert_eq!(fx, [saved(&list)]);
        let (list, fx) = transition(list, Event::Enable(a.clone()));
        assert!(list.get(&a).unwrap().enabled);
        assert_eq!(fx, [saved(&list)]);

        // A new crx version replaces the old copy, which is left for the next start.
        let (list, fx) = transition(list, Event::Install(crx("b", "2")));
        assert_eq!(fx, [saved(&list)]);
        assert_eq!(list.get(&b).unwrap().dir(), Path::new("/p/extensions/b/2"));
        let (list, fx) = transition(list, Event::Remove(b.clone()));
        assert!(list.get(&b).is_none());
        assert_eq!(fx, [saved(&list)]);
        let (list, fx) = transition(list, Event::Remove(a));
        assert_eq!(fx, [saved(&list)]);
        assert!(list.extensions().is_empty());

        // Unknown ids change nothing.
        for event in [
            Event::Enable(b.clone()),
            Event::Disable(b.clone()),
            Event::Remove(b),
        ] {
            let (same, fx) = transition(list.clone(), event);
            assert!(fx.is_empty() && same == list);
        }
    }

    #[test]
    fn paths_chromium_cannot_take_are_refused() {
        let comma = ext("c", Source::Unpacked { dir: "/a,b".into() });
        let (list, fx) = transition(Installed::default(), Event::Install(comma));
        assert!(list.extensions().is_empty());
        assert_eq!(fx, [Effect::Refused(ExtError::UnusablePath("/a,b".into()))]);
        // A .crx file's own path never reaches Chromium: a comma there is fine.
        let file = ext(
            "c",
            Source::Crx {
                file: "/dl/uBlock, Lite.crx".into(),
                dir: "/p/c".into(),
            },
        );
        let (list, fx) = transition(list, Event::Install(file));
        assert_eq!(fx, [saved(&list)]);
        let dir = ext(
            "d",
            Source::Crx {
                file: "/dl/d.crx".into(),
                dir: "/p/d,1".into(),
            },
        );
        assert!(matches!(
            transition(list, Event::Install(dir)).1.as_slice(),
            [Effect::Refused(_)]
        ));
    }

    #[test]
    fn copies_the_list_stops_naming_are_unused() {
        let mut list = Installed::default();
        for e in [unpacked("a"), crx("b", "1"), crx("c", "1")] {
            list = transition(list, Event::Install(e)).0;
        }
        list = transition(list, Event::Install(crx("b", "2"))).0;
        list = transition(list, Event::Remove(id_from_key(b"c"))).0;
        // Disabled copies stay: enabling one needs its files.
        list = transition(list, Event::Disable(id_from_key(b"b"))).0;
        let found = ["b/1", "b/2", "c/1", "x/1.unpacking-7"]
            .map(|d| PathBuf::from("/p/extensions").join(d));
        assert_eq!(
            unused_copies(&list, found),
            ["b/1", "c/1", "x/1.unpacking-7"].map(|d| PathBuf::from("/p/extensions").join(d))
        );
        assert!(unused_copies(&list, []).is_empty());
    }

    #[test]
    fn load_paths_are_the_enabled_in_id_order_and_restart_is_derived() {
        let mut list = Installed::default();
        for e in [unpacked("x"), unpacked("y"), crx("z", "3")] {
            list = transition(list, Event::Install(e)).0;
        }
        let mut want: Vec<(ExtensionId, PathBuf)> = list
            .extensions()
            .iter()
            .map(|e| (e.id.clone(), e.dir().to_path_buf()))
            .collect();
        want.sort();
        let all: Vec<PathBuf> = want.iter().map(|(_, d)| d.clone()).collect();
        assert_eq!(load_paths(&list), all);
        let at_start = list.clone();
        assert!(!restart_needed(&at_start, &list));
        let (list, _) = transition(list, Event::Disable(id_from_key(b"y")));
        let without: Vec<PathBuf> = all
            .iter()
            .filter(|d| *d != Path::new("/src/y"))
            .cloned()
            .collect();
        assert_eq!(load_paths(&list), without);
        assert!(restart_needed(&at_start, &list));
        assert!(restart_needed(&Installed::default(), &list));
        assert!(!restart_needed(
            &Installed::default(),
            &Installed::default()
        ));
        // Disabling then enabling again is what was loaded.
        let (list, _) = transition(list, Event::Enable(id_from_key(b"y")));
        assert!(!restart_needed(&at_start, &list));
        // The same directory installed again with a new manifest: the paths are the same,
        // what Chromium read at start is not.
        let mut edited = unpacked("x");
        edited.version = "9".into();
        edited.permissions.push("tabs".into());
        let (list, _) = transition(list, Event::Install(edited));
        assert_eq!(load_paths(&list), all);
        assert!(restart_needed(&at_start, &list));
    }

    #[test]
    fn the_list_round_trips_through_json() {
        let mut list = Installed::default();
        for e in [unpacked("a"), crx("b", "1.0")] {
            list = transition(list, Event::Install(e)).0;
        }
        list = transition(list, Event::Disable(id_from_key(b"a"))).0;
        let text = list.to_json();
        assert!(!text.contains('\n'));
        assert_eq!(Installed::from_json(&text), Some(list.clone()));
        assert_eq!(Installed::from_json("[]"), Some(Installed::default()));
        assert_eq!(Installed::from_json("garbage"), None);
        assert_eq!(Installed::from_json("{}"), None);
        // A broken entry is dropped, the rest kept.
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v[0]["id"] = json!("not an id");
        let kept = Installed::from_json(&v.to_string()).unwrap();
        assert_eq!(kept.extensions().len(), 1);
    }
}
