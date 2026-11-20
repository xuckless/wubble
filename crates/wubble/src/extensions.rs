//! The edge of `wubble_core::extensions`: installs what the user picked (a directory or a
//! .crx file, through `wubble_engine::extensions`), runs the list's `Save` into the store, and
//! at start deletes the copies the list no longer names. Every change applies at the next start,
//! when the list becomes CEF's `--load-extension` (`persist::Settings::extensions`); the UI says
//! "restart to apply" from `wubble_core::extensions::restart_needed`.

use std::path::Path;

use wubble_core::extensions::{
    Effect, Event, ExtError, Extension, Installed, transition, unused_copies,
};
use wubble_core::settings::EXTENSIONS_INSTALLED;
use wubble_store::{InstalledExtensions, text_of};

use crate::persist::StoreClient;

/// The extension `picked` holds: a .crx file (unpacked under `root`, wubble's copies) or an
/// extension's directory (loaded in place). Nothing is installed yet: the list is not touched
/// (an unpacked copy a refused install leaves is [`sweep`]'s).
pub fn read(picked: &Path, root: &Path) -> Result<Extension, ExtError> {
    if picked.is_file() {
        wubble_engine::extensions::unpack_crx(picked, root)
    } else {
        wubble_engine::extensions::read_unpacked(picked)
    }
}

/// The list after installing `picked` ([`read`]), stored through `store`.
pub fn install(
    store: &StoreClient,
    list: Installed,
    picked: &Path,
    root: &Path,
) -> Result<Installed, ExtError> {
    change(store, list, Event::Install(read(picked, root)?))
}

/// The list after `event`, its effects run.
pub fn change(store: &StoreClient, list: Installed, event: Event) -> Result<Installed, ExtError> {
    let (list, effects) = transition(list, event);
    for effect in effects {
        match effect {
            Effect::Save(list) => {
                let text = text_of::<InstalledExtensions>(&list);
                if !store.set_setting(EXTENSIONS_INSTALLED.key, &text) {
                    return Err(ExtError::Io("the store refused the extension list".into()));
                }
            }
            Effect::Refused(e) => return Err(e),
        }
    }
    Ok(list)
}

/// Deletes the copies under `root` that `list` does not name. Only before CEF starts: until
/// then no Chromium is reading them.
pub fn sweep(root: &Path, list: &Installed) {
    for dir in unused_copies(list, wubble_engine::extensions::copies(root)) {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            // The list no longer names it; a leftover directory is only disk space.
            eprintln!("wubble: cannot remove {}: {e}", dir.display());
        }
        // The id's directory too once it is empty; not empty is fine.
        if let Some(id_dir) = dir.parent() {
            let _ = std::fs::remove_dir(id_dir);
        }
    }
}
