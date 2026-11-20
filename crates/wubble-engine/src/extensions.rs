//! Chrome extensions from the user's files: an unpacked directory as it is, a .crx file unpacked
//! by wubble. Both end as a directory Chromium loads at the next start (`--load-extension`,
//! `app::extension_switches`); CEF 156 has no extension API of its own (CEF #3685), and
//! Chromium's own .crx installer is not reachable without a patch (docs/research/extensions.md).
//! The list and its rules are `wubble_core::extensions`; this file reads and writes files, and
//! unzips through CEF's own zip reader (`CefZipReader`, minizip), so no zip crate is needed.
//!
//! The .crx steps (parse the CRX3 header, unzip, write `"key"` into the manifest so the copy
//! keeps the .crx's id) are ported from samuelmaddock/electron-browser-shell @3817c42,
//! `packages/electron-chrome-web-store/src/browser/installer.ts` (`unpackCrx`), MIT.

use std::path::{Path, PathBuf};

use cef::ImplZipReader;
use wubble_core::extensions::{
    Entry, ExtError, Extension, Manifest, Source, manifest_with_key, parse_crx, parse_manifest,
    zip_entry,
};

const MANIFEST: &str = "manifest.json";

/// The most an archive may unpack to: a guard against zip bombs, far above any real extension.
const MAX_UNPACKED_BYTES: u64 = 512 << 20;

fn io(context: &Path) -> impl Fn(std::io::Error) -> ExtError + '_ {
    move |e| ExtError::Io(format!("{}: {e}", context.display()))
}

/// The path as Chromium sees it (`base::MakeAbsoluteFilePath` in `UnpackedInstaller`):
/// absolute with symlinks resolved, which is what a path-made id hashes.
fn as_chromium_sees(path: &Path) -> Result<PathBuf, ExtError> {
    #[cfg(windows)]
    let full = std::path::absolute(path);
    #[cfg(not(windows))]
    let full = path.canonicalize();
    full.map_err(io(path))
}

fn read_manifest(dir: &Path) -> Result<Manifest, ExtError> {
    let path = dir.join(MANIFEST);
    parse_manifest(&std::fs::read_to_string(&path).map_err(io(&path))?)
}

/// The extension in the directory `dir`, loaded in place.
pub fn read_unpacked(dir: &Path) -> Result<Extension, ExtError> {
    let dir = as_chromium_sees(dir)?;
    let manifest = read_manifest(&dir)?;
    let id = manifest.id(&dir)?;
    Ok(Extension::new(id, manifest, Source::Unpacked { dir }))
}

/// The entries of the zip `data` with their bytes, through CEF's zip reader. Works before
/// `cef::initialize`: the reader is plain minizip over a copy of the bytes.
fn unzip(data: &[u8]) -> Result<Vec<(Entry, Vec<u8>)>, ExtError> {
    let bad = |what: &str| ExtError::Crx(format!("the archive {what}"));
    // The API version pin welding sets before `initialize`; only the first call counts.
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    // CEF copies the bytes (`CefBytesReader::SetData`); the pointer is only read.
    let mut copy = data.to_vec();
    let mut stream = cef::stream_reader_create_for_data(copy.as_mut_ptr(), copy.len())
        .ok_or_else(|| bad("cannot be read"))?;
    let reader = cef::zip_reader_create(Some(&mut stream)).ok_or_else(|| bad("is not a zip"))?;
    let mut out = vec![];
    let mut total = 0u64;
    let mut buf = vec![0u8; 64 << 10];
    let mut more = reader.move_to_first_file() != 0;
    while more {
        let name = {
            let name = reader.file_name();
            let raw: Option<&cef::sys::_cef_string_utf16_t> = (&name).into();
            raw.map(|r| cef::CefStringUtf16::from(*r).to_string())
                .unwrap_or_default()
        };
        let entry = zip_entry(&name)?;
        let mut bytes = vec![];
        if matches!(entry, Entry::File(_)) {
            if reader.open_file(None) == 0 {
                return Err(bad(&format!("entry {name:?} cannot be opened")));
            }
            loop {
                let n = usize::try_from(reader.read_file(Some(&mut buf)))
                    .map_err(|_| bad(&format!("entry {name:?} is broken")))?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > MAX_UNPACKED_BYTES {
                    return Err(bad("is too large unpacked"));
                }
                bytes.extend_from_slice(&buf[..n]);
            }
            // minizip checks the entry's CRC here.
            if reader.close_file() == 0 {
                return Err(bad(&format!("entry {name:?} fails its checksum")));
            }
        }
        out.push((entry, bytes));
        more = reader.move_to_next_file() != 0;
    }
    reader.close();
    Ok(out)
}

/// Writes `files` into the new directory `to`, `manifest` last as `manifest.json`.
fn write_tree(to: &Path, files: &[(Entry, Vec<u8>)], manifest: &str) -> Result<(), ExtError> {
    std::fs::create_dir_all(to).map_err(io(to))?;
    for (entry, data) in files {
        match entry {
            Entry::Dir(rel) => {
                let dir = to.join(rel);
                std::fs::create_dir_all(&dir).map_err(io(&dir))?;
            }
            Entry::File(rel) => {
                let file = to.join(rel);
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).map_err(io(parent))?;
                }
                std::fs::write(&file, data).map_err(io(&file))?;
            }
        }
    }
    let file = to.join(MANIFEST);
    std::fs::write(&file, manifest).map_err(io(&file))
}

/// Unpacks the .crx `file` into `root/<id>/<version>` with the .crx's key in the manifest,
/// through a temporary sibling renamed into place, so a failed unpack leaves nothing
/// half-written. An existing copy of the same version is replaced.
pub fn unpack_crx(file: &Path, root: &Path) -> Result<Extension, ExtError> {
    let file = as_chromium_sees(file)?;
    let bytes = std::fs::read(&file).map_err(io(&file))?;
    let crx = parse_crx(&bytes)?;
    let files = unzip(crx.zip)?;
    let manifest_text = files
        .iter()
        .find(|(e, _)| *e == Entry::File(PathBuf::from(MANIFEST)))
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .ok_or_else(|| ExtError::Crx("no manifest.json".into()))?;
    let version = parse_manifest(&manifest_text)?.version;
    let keyed = manifest_with_key(&manifest_text, crx.public_key)?;
    let id_dir = root.join(crx.id.as_str());
    let dir = id_dir.join(&version);
    let tmp = id_dir.join(format!("{version}.unpacking-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let written = write_tree(&tmp, &files, &keyed).and_then(|()| {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(io(&dir))?;
        }
        std::fs::rename(&tmp, &dir).map_err(io(&dir))
    });
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let dir = as_chromium_sees(&dir)?;
    let manifest = read_manifest(&dir)?;
    // The key written above makes the id the .crx's wherever the copy lives.
    let id = manifest.id(&dir)?;
    if id != crx.id {
        return Err(ExtError::Crx("the unpacked copy has another id".into()));
    }
    Ok(Extension::new(id, manifest, Source::Crx { file, dir }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::extensions::{crx3_unsigned, id_from_key, id_from_path, zip_stored};

    const MANIFEST_V3: &str =
        r#"{"manifest_version": 3, "name": "T", "version": "2.1", "permissions": ["storage"]}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wubble-ext-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_crx_unpacks_with_its_key_and_keeps_its_id() {
        let dir = scratch("crx");
        let zip = zip_stored(&[
            ("manifest.json", MANIFEST_V3.as_bytes()),
            ("js/", b""),
            ("js/c.js", b"// c"),
        ]);
        let file = dir.join("t.crx");
        std::fs::write(&file, crx3_unsigned(b"key bytes", &zip)).unwrap();
        let root = dir.join("extensions");
        let first = unpack_crx(&file, &root);
        // Again: the same version is replaced in place.
        let again = unpack_crx(&file, &root);
        let ext = first.unwrap();
        let read_back = std::fs::read_to_string(ext.dir().join("js/c.js"));
        let leftovers = std::fs::read_dir(root.join(ext.id.as_str()))
            .unwrap()
            .count();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(ext.id, id_from_key(b"key bytes"));
        assert_eq!((ext.name.as_str(), ext.version.as_str()), ("T", "2.1"));
        assert!(ext.dir().ends_with(format!("{}/2.1", ext.id)));
        assert_eq!(read_back.unwrap(), "// c");
        assert_eq!(again.unwrap(), ext);
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn a_crx_with_an_escaping_entry_writes_nothing() {
        let dir = scratch("slip");
        let zip = zip_stored(&[
            ("manifest.json", MANIFEST_V3.as_bytes()),
            ("../evil.js", b"x"),
        ]);
        let file = dir.join("t.crx");
        std::fs::write(&file, crx3_unsigned(b"k", &zip)).unwrap();
        let got = unpack_crx(&file, &dir.join("extensions"));
        let wrote = dir.join("extensions").exists() || dir.join("evil.js").exists();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(got, Err(ExtError::BadEntry("../evil.js".into())));
        assert!(!wrote);
    }

    #[test]
    fn an_unpacked_directory_gets_its_path_id() {
        let dir = scratch("unpacked");
        std::fs::write(dir.join("manifest.json"), MANIFEST_V3).unwrap();
        let ext = read_unpacked(&dir);
        let canonical = as_chromium_sees(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let ext = ext.unwrap();
        assert_eq!(ext.id, id_from_path(&canonical));
        assert_eq!(ext.dir(), canonical);
        assert!(ext.enabled);
        assert_eq!(ext.permissions, ["storage"]);
    }
}
