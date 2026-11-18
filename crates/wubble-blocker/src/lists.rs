//! Filter lists on disk: every `*.txt` in a directory plus an optional
//! `resources.json`, compiled once and cached.
//!
//! The compiled engine is cached in one file with a key file beside it
//! (`<cache>.key`) naming each list's file name, length and modification
//! time. A start whose lists did not change loads the cache instead of
//! parsing the lists again; any mismatch, unreadable cache or deserialization
//! error rebuilds from the texts. The cache never holds resources
//! (`Engine::serialize` leaves them out), so `resources.json` is applied
//! after every load.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::blocker::Blocker;

/// Redirect and scriptlet resources inside a list directory.
pub const RESOURCES_FILE: &str = "resources.json";

impl Blocker {
    /// The blocker for the lists in `dir`, from `cache` when it matches
    /// them, else compiled (and the cache rewritten). A missing or empty
    /// directory gives a blocker that allows everything and writes no cache.
    pub fn from_dir(dir: &Path, cache: &Path) -> Blocker {
        Self::from_cache(dir, cache).unwrap_or_else(|| Self::compile_dir(dir, cache))
    }

    /// The cached blocker, if the cache matches the lists in `dir`. Cheap
    /// enough for the UI thread: no list is parsed.
    pub fn from_cache(dir: &Path, cache: &Path) -> Option<Blocker> {
        let lists = list_files(dir);
        if lists.is_empty() {
            return Some(with_resources(Blocker::from_lists(&[]), dir));
        }
        let stored = std::fs::read_to_string(key_path(cache)).ok()?;
        if stored != cache_key(&lists) {
            return None;
        }
        Blocker::load(cache).ok().map(|b| with_resources(b, dir))
    }

    /// Compiles the lists in `dir` and rewrites the cache. Slow for real
    /// lists (seconds): call it off the UI thread.
    pub fn compile_dir(dir: &Path, cache: &Path) -> Blocker {
        let lists = list_files(dir);
        let texts: Vec<String> = lists
            .iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let blocker = Blocker::from_lists(&refs);
        if !lists.is_empty() {
            // The key is written after the cache, so a crash between the two
            // leaves a stale key that no longer matches, never a wrong hit.
            let saved = blocker
                .save(cache)
                .and_then(|()| Ok(std::fs::write(key_path(cache), cache_key(&lists))?));
            if let Err(e) = saved {
                eprintln!("wubble-blocker: cache not written: {e}");
            }
        }
        with_resources(blocker, dir)
    }
}

/// `dir/*.txt`, sorted by name.
fn list_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "txt") && p.is_file())
        .collect();
    files.sort();
    files
}

/// One line per list: file name, length, modification time in ns.
fn cache_key(lists: &[PathBuf]) -> String {
    lists
        .iter()
        .map(|p| {
            let meta = std::fs::metadata(p).ok();
            let len = meta.as_ref().map_or(0, |m| m.len());
            let mtime = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            format!("{name} {len} {mtime}\n")
        })
        .collect()
}

fn key_path(cache: &Path) -> PathBuf {
    let mut key = cache.as_os_str().to_owned();
    key.push(".key");
    key.into()
}

/// Applies `dir/resources.json` if present; a bad file is skipped (the
/// lists still block, `$redirect` rules then block instead of redirecting).
fn with_resources(mut blocker: Blocker, dir: &Path) -> Blocker {
    if let Ok(json) = std::fs::read_to_string(dir.join(RESOURCES_FILE))
        && let Err(e) = blocker.use_resources_json(&json)
    {
        eprintln!("wubble-blocker: {RESOURCES_FILE} ignored: {e}");
    }
    blocker
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::Decision;
    use crate::site::Shields;
    use crate::types::{NetRequest, ResourceType};

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "wubble-blocker-lists-{name}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("lists")).unwrap();
            Self(dir)
        }
        fn lists(&self) -> PathBuf {
            self.0.join("lists")
        }
        fn cache(&self) -> PathBuf {
            self.0.join("engine.bin")
        }
        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.lists().join(name), text).unwrap();
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn blocks(b: &Blocker, url: &str, ty: ResourceType) -> bool {
        let req = NetRequest {
            url,
            source_url: "https://site.org/",
            resource_type: ty,
        };
        b.decide(req, Shields::On) == Decision::Block
    }

    #[test]
    fn reads_only_txt_files() {
        let d = Dir::new("txt");
        d.write("a.txt", "||a.test^\n");
        d.write("b.txt", "||b.test^\n");
        d.write("c.md", "||c.test^\n");
        let b = Blocker::from_dir(&d.lists(), &d.cache());
        assert!(blocks(&b, "https://a.test/x", ResourceType::Script));
        assert!(blocks(&b, "https://b.test/x", ResourceType::Script));
        assert!(!blocks(&b, "https://c.test/x", ResourceType::Script));
        let names: Vec<_> = list_files(&d.lists())
            .iter()
            .map(|p| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
    }

    #[test]
    fn missing_or_empty_dir_allows_all_and_writes_no_cache() {
        let d = Dir::new("empty");
        let b = Blocker::from_dir(&d.lists(), &d.cache());
        assert!(!blocks(&b, "https://a.test/x", ResourceType::Script));
        let b = Blocker::from_dir(&d.0.join("missing"), &d.cache());
        assert!(!blocks(&b, "https://a.test/x", ResourceType::Script));
        assert!(!d.cache().exists());
    }

    #[test]
    fn a_matching_cache_is_loaded_instead_of_the_lists() {
        let d = Dir::new("hit");
        d.write("a.txt", "||a.test^\n");
        assert!(Blocker::from_cache(&d.lists(), &d.cache()).is_none());
        Blocker::from_dir(&d.lists(), &d.cache());
        assert!(key_path(&d.cache()).exists());
        // Swap the cache for another engine, key untouched: a hit loads it.
        Blocker::from_lists(&["||other.test^\n"])
            .save(&d.cache())
            .unwrap();
        let b = Blocker::from_cache(&d.lists(), &d.cache()).expect("cache hit");
        assert!(blocks(&b, "https://other.test/x", ResourceType::Script));
        assert!(!blocks(&b, "https://a.test/x", ResourceType::Script));
    }

    #[test]
    fn a_changed_list_rebuilds() {
        let d = Dir::new("miss");
        d.write("a.txt", "||a.test^\n");
        Blocker::from_dir(&d.lists(), &d.cache());
        d.write("a.txt", "||a.test^\n||more.test^\n");
        assert!(Blocker::from_cache(&d.lists(), &d.cache()).is_none());
        let b = Blocker::from_dir(&d.lists(), &d.cache());
        assert!(blocks(&b, "https://more.test/x", ResourceType::Script));
        // The rebuilt cache matches again.
        assert!(Blocker::from_cache(&d.lists(), &d.cache()).is_some());
    }

    #[test]
    fn a_corrupt_cache_rebuilds() {
        let d = Dir::new("corrupt");
        d.write("a.txt", "||a.test^\n");
        Blocker::from_dir(&d.lists(), &d.cache());
        std::fs::write(d.cache(), b"garbage").unwrap();
        assert!(Blocker::from_cache(&d.lists(), &d.cache()).is_none());
        let b = Blocker::from_dir(&d.lists(), &d.cache());
        assert!(blocks(&b, "https://a.test/x", ResourceType::Script));
    }

    const RES: &str = r#"[{"name":"1x1.gif","aliases":[],"kind":{"mime":"image/gif"},"content":"R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7"}]"#;

    #[test]
    fn resources_are_applied_after_a_cache_load() {
        let d = Dir::new("res");
        d.write("a.txt", "||img.test^$image,redirect=1x1.gif\n");
        d.write(RESOURCES_FILE, RES);
        let redirects = |b: &Blocker| {
            let req = NetRequest {
                url: "https://img.test/a.png",
                source_url: "https://site.org/",
                resource_type: ResourceType::Image,
            };
            matches!(b.decide(req, Shields::On), Decision::Redirect { .. })
        };
        assert!(redirects(&Blocker::from_dir(&d.lists(), &d.cache())));
        let cached = Blocker::from_cache(&d.lists(), &d.cache()).expect("cache hit");
        assert!(redirects(&cached));
        // A bad resources file keeps the lists.
        d.write(RESOURCES_FILE, "nope");
        let b = Blocker::from_dir(&d.lists(), &d.cache());
        assert!(blocks(&b, "https://img.test/a.png", ResourceType::Image));
    }
}
