//! Where a run keeps its data: the profile directory, holding wubble's
//! database and (in browse mode) CEF's cache; and where downloads go.

use std::path::{Path, PathBuf};

use wubble_engine::app::{self, CEF_CACHE};

/// The database file inside a profile.
const DB_FILE: &str = "wubble.db";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub dir: PathBuf,
}

impl Profile {
    /// `--profile` if given, else the per-user data directory
    /// (`wubble_engine::app::user_data_dir`). `None` if neither exists.
    pub fn resolve(cli: Option<PathBuf>) -> Option<Profile> {
        choose(cli, app::user_data_dir)
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.join(DB_FILE)
    }

    /// wubble's unpacked copies of .crx extensions (`wubble_engine::extensions::unpack_crx`).
    pub fn extensions_dir(&self) -> PathBuf {
        self.dir.join("extensions")
    }

    /// CEF's cache: the same place the engine uses without a profile.
    pub fn cef_cache(&self) -> PathBuf {
        self.dir.join(CEF_CACHE)
    }
}

/// `--downloads` if given, else the OS downloads directory (the XDG user
/// dir on Linux, `~/Downloads` on macOS, the known folder on Windows, via
/// the `dirs` crate). `None`: downloads are declined.
pub fn downloads_dir(cli: Option<PathBuf>) -> Option<PathBuf> {
    choose_dir(cli, dirs::download_dir)
}

/// `cli` wins over the per-user default, which is only looked up without it.
/// A relative `cli` is taken from the current directory.
fn choose_dir(cli: Option<PathBuf>, default: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    match cli {
        Some(dir) => Some(absolute(&dir)),
        None => default(),
    }
}

fn choose(cli: Option<PathBuf>, default: impl FnOnce() -> Option<PathBuf>) -> Option<Profile> {
    choose_dir(cli, default).map(|dir| Profile { dir })
}

fn absolute(dir: &Path) -> PathBuf {
    std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_profile_wins() {
        let default = || -> Option<PathBuf> { panic!("not looked up") };
        // absolute on every OS (`/p` is not on Windows, where it gains the current drive)
        let dir = std::env::temp_dir().join("p");
        assert_eq!(choose(Some(dir.clone()), default), Some(Profile { dir }));
        assert_eq!(
            choose(None, || Some("/data/wubble".into())),
            Some(Profile {
                dir: "/data/wubble".into()
            })
        );
        assert_eq!(choose(None, || None), None);
    }

    #[test]
    fn a_relative_profile_is_made_absolute() {
        let p = choose(Some("rel".into()), || None).unwrap();
        assert!(p.dir.is_absolute(), "{p:?}");
        assert!(p.dir.ends_with("rel"));
    }

    #[test]
    fn the_cli_downloads_folder_wins() {
        let default = || -> Option<PathBuf> { panic!("not looked up") };
        let dl = std::env::temp_dir().join("dl");
        assert_eq!(choose_dir(Some(dl.clone()), default), Some(dl));
        assert_eq!(
            choose_dir(None, || Some("/home/u/Downloads".into())),
            Some("/home/u/Downloads".into())
        );
        assert_eq!(choose_dir(None, || None), None);
        let rel = choose_dir(Some("dl".into()), || None).unwrap();
        assert!(rel.is_absolute() && rel.ends_with("dl"), "{rel:?}");
    }

    #[test]
    fn files_inside_the_profile() {
        let p = Profile { dir: "/p".into() };
        assert_eq!(p.db_path(), PathBuf::from("/p/wubble.db"));
        assert_eq!(p.cef_cache(), PathBuf::from("/p/cef"));
    }
}
