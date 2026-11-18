//! Where a run keeps its data: the profile directory, holding wubble's
//! database and (in browse mode) CEF's cache.

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

    /// CEF's cache: the same place the engine uses without a profile.
    pub fn cef_cache(&self) -> PathBuf {
        self.dir.join(CEF_CACHE)
    }
}

/// `cli` wins over the per-user default, which is only looked up without it.
/// A relative `cli` is taken from the current directory.
fn choose(cli: Option<PathBuf>, default: impl FnOnce() -> Option<PathBuf>) -> Option<Profile> {
    let dir = match cli {
        Some(dir) => absolute(&dir),
        None => default()?,
    };
    Some(Profile { dir })
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
        assert_eq!(
            choose(Some("/p".into()), default),
            Some(Profile { dir: "/p".into() })
        );
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
    fn files_inside_the_profile() {
        let p = Profile { dir: "/p".into() };
        assert_eq!(p.db_path(), PathBuf::from("/p/wubble.db"));
        assert_eq!(p.cef_cache(), PathBuf::from("/p/cef"));
    }
}
