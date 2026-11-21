//! The English spelling dictionary, shipped inside the app so spell check works with no request.
//!
//! Chromium looks for `<language>-<major>-<minor>.bdic` in `<root_cache_path>/Dictionaries` (CEF's
//! `OverrideUserDataDir`, `chrome::DIR_APP_DICTIONARIES`) and downloads it from
//! `redirector.gvt1.com` when it is not there; wubble refuses that host, so [`install`] puts the
//! bundled file there before CEF starts. What to write and delete is `wubble_core::spell::plan`.
//! macOS needs none of this: Chromium uses the system spell checker there
//! (`use_browser_spellchecker`), so no file is written.
//!
//! The file is Chromium's `third_party/hunspell_dictionaries/en-US-10-2.bdic` (built from SCOWL
//! en_US; permissive licences, see `assets/dictionaries/README_en_US.txt` and
//! `THIRD_PARTY_NOTICES.md`). Its version is the one `spellcheck_common.cc` expects for en-US in
//! CEF 156: a bump of the pin means a new file here under its new name.

use std::path::Path;

use wubble_core::spell::{self, Found};

/// The versioned name Chromium looks for.
pub const NAME: &str = "en-US-10-2.bdic";

const BUNDLED: &[u8] = include_bytes!("../assets/dictionaries/en-US-10-2.bdic");

/// Writes the bundled dictionary into `<cache_dir>/Dictionaries` unless it is already there.
/// Best effort: spell check is a convenience, so a failure is logged and start goes on.
pub fn install(cache_dir: &Path) {
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = install_into(&cache_dir.join("Dictionaries"), BUNDLED) {
        eprintln!("wubble: spelling dictionary not installed: {e}");
    }
    #[cfg(target_os = "macos")]
    let _ = (cache_dir, BUNDLED);
}

#[cfg(any(not(target_os = "macos"), test))]
fn install_into(dir: &Path, bundled: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let found: Vec<Found> = std::fs::read_dir(dir)?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let len = entry.metadata().ok()?.len();
            name.ends_with(".bdic").then_some(Found { name, len })
        })
        .collect();
    let plan = spell::plan(
        &Found {
            name: NAME.into(),
            len: bundled.len() as u64,
        },
        &found,
    );
    for name in &plan.remove {
        std::fs::remove_file(dir.join(name))?;
    }
    if plan.write {
        // Renamed into place: Chromium opens the file by name and a half-written one would be read.
        let tmp = dir.join(format!("{NAME}.tmp"));
        std::fs::write(&tmp, bundled)?;
        std::fs::rename(&tmp, dir.join(NAME))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_file_is_a_bdic_of_the_expected_name() {
        assert!(BUNDLED.starts_with(b"BDic"));
    }

    #[test]
    fn install_writes_replaces_and_keeps() {
        let dir = std::env::temp_dir().join(format!("wubble-dict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("en-US-9-0.bdic"), b"old").unwrap();
        std::fs::write(dir.join("fr-FR-3-0.bdic"), b"fr").unwrap();
        install_into(&dir, b"bundled").unwrap();
        assert_eq!(std::fs::read(dir.join(NAME)).unwrap(), b"bundled");
        assert!(!dir.join("en-US-9-0.bdic").exists());
        assert!(dir.join("fr-FR-3-0.bdic").exists());
        // Present: untouched.
        std::fs::write(dir.join(NAME), b"BUNDLED").unwrap();
        install_into(&dir, b"bundled").unwrap();
        assert_eq!(std::fs::read(dir.join(NAME)).unwrap(), b"BUNDLED");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
