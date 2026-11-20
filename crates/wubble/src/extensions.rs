//! The edge of `wubble_core::extensions`: installs what the user picked (a directory or a
//! .crx file, through `wubble_engine::extensions`) and runs the list's effects, `Save` into the
//! store and `DeleteDir` on disk. Every change applies at the next start, when the list becomes
//! CEF's `--load-extension` (`persist::Settings::extensions`); the UI says "restart to apply"
//! from `wubble_core::extensions::restart_needed`.

use std::path::Path;

use wubble_core::extensions::{Effect, Event, ExtError, Installed, transition};
use wubble_core::settings::EXTENSIONS_INSTALLED;
use wubble_store::{InstalledExtensions, text_of};

use crate::persist::StoreClient;

/// The list after installing `picked`, a .crx file (unpacked under `root`, wubble's copies) or
/// an extension's directory (loaded in place), stored through `store`.
pub fn install(
    store: &StoreClient,
    list: Installed,
    picked: &Path,
    root: &Path,
) -> Result<Installed, ExtError> {
    let ext = if picked.is_file() {
        wubble_engine::extensions::unpack_crx(picked, root)?
    } else {
        wubble_engine::extensions::read_unpacked(picked)?
    };
    change(store, list, Event::Install(ext))
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
            Effect::DeleteDir(dir) => match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // The list no longer names it; a leftover directory is only disk space.
                Err(e) => eprintln!("wubble: cannot remove {}: {e}", dir.display()),
            },
            Effect::Refused(e) => return Err(e),
        }
    }
    Ok(list)
}
