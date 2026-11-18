//! Where a blocker's lists come from, compiled once and cached.
//!
//! A [`Source`] is one list: a catalog list ([`crate::catalog::LISTS`]) from
//! its downloaded copy in the profile's list directory (`<id>.txt`) when
//! there is one, else from the snapshot bundled in the binary; or a list of
//! the user's own (any other `*.txt` there, and the downloaded custom lists).
//! Bundled and downloaded lists take the same path.
//!
//! The compiled engine is cached in one file with a key file beside it
//! (`<cache>.key`) naming each source: a file's name, length and
//! modification time, a bundled list's length and a hash of its header. A
//! start whose sources did not change loads the cache instead of parsing the
//! lists again; any mismatch, unreadable cache or deserialization error
//! rebuilds from the texts. The cache never holds resources
//! (`Engine::serialize` leaves them out), so the bundled resources are
//! applied after every load.

use std::borrow::Cow;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::blocker::Blocker;
use crate::catalog::{self, ListDef, Selection};
use crate::refresh::ListSpec;

/// Changes whenever the way sources are turned into filters changes, so an
/// older cache is rebuilt.
const KEY_SCHEME: &str = "wubble-blocker sources 2\n";

/// Where a list's text is.
#[derive(Clone, Debug)]
pub enum Origin {
    /// Compiled into the binary.
    Bundled(&'static str),
    /// A file in the list directory.
    File(PathBuf),
    /// Given as text (`--smoke block`, tests).
    Text(String),
}

/// One list to compile.
#[derive(Clone, Debug)]
pub struct Source {
    /// The catalog row, or `None` for a list of the user's own (standard
    /// format, untrusted, every rule type).
    pub def: Option<&'static ListDef>,
    pub origin: Origin,
}

impl Source {
    /// A list of the user's own given as text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            def: None,
            origin: Origin::Text(text.into()),
        }
    }

    pub(crate) fn read(&self) -> Option<Cow<'_, str>> {
        match &self.origin {
            Origin::Bundled(text) => Some(Cow::Borrowed(text)),
            Origin::Text(text) => Some(Cow::Borrowed(text)),
            Origin::File(path) => match std::fs::read_to_string(path) {
                Ok(text) => Some(Cow::Owned(text)),
                Err(e) => {
                    eprintln!("wubble-blocker: {} not read: {e}", path.display());
                    None
                }
            },
        }
    }

    /// This source's line in the cache key.
    fn key(&self) -> String {
        let id = self.def.map_or("-", |d| d.id);
        match &self.origin {
            Origin::Bundled(text) => format!("{id} bundled {} {:016x}\n", text.len(), head(text)),
            Origin::Text(text) => format!("{id} text {} {:016x}\n", text.len(), head(text)),
            Origin::File(path) => {
                let meta = std::fs::metadata(path).ok();
                let len = meta.as_ref().map_or(0, |m| m.len());
                let mtime = meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                format!("{id} file {name} {len} {mtime}\n")
            }
        }
    }
}

/// A hash of a text's first KiB: a list's header names its version
/// (`Last modified`), so this and the length tell snapshots apart without
/// hashing megabytes on the UI thread.
fn head(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.as_bytes()[..text.len().min(1024)].hash(&mut h);
    h.finish()
}

/// The sources for `selection` from the list directory `dir`: each enabled
/// catalog list (downloaded copy, else bundled; a list with neither is
/// skipped), then every other `*.txt` in `dir` sorted by name (the user's
/// own, and the selected custom lists that were downloaded).
pub fn sources(dir: &Path, selection: &Selection) -> Vec<Source> {
    let mut out: Vec<Source> = selection
        .enabled()
        .filter_map(|def| {
            let file = dir.join(format!("{}.txt", def.id));
            let origin = if file.is_file() {
                Origin::File(file)
            } else {
                Origin::Bundled(def.bundled?)
            };
            Some(Source {
                def: Some(def),
                origin,
            })
        })
        .collect();
    let custom: Vec<String> = selection
        .custom
        .iter()
        .map(|url| ListSpec::custom(url).file_name())
        .collect();
    out.extend(
        list_files(dir)
            .into_iter()
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                let stem = name.trim_end_matches(".txt");
                if catalog::find(stem).is_some() {
                    return false;
                }
                !stem.starts_with(ListSpec::CUSTOM_PREFIX) || custom.iter().any(|c| *c == name)
            })
            .map(|p| Source {
                def: None,
                origin: Origin::File(p),
            }),
    );
    out
}

/// The bundled lists of `selection` alone.
pub fn bundled_sources(selection: &Selection) -> Vec<Source> {
    sources(Path::new(""), selection)
}

impl Blocker {
    /// The blocker for `sources`, from `cache` when it matches them, else
    /// compiled (and the cache rewritten).
    pub fn from_sources_cached(sources: &[Source], cache: &Path) -> Blocker {
        Self::from_cache(sources, cache).unwrap_or_else(|| Self::compile(sources, Some(cache)))
    }

    /// The cached blocker, if the cache matches `sources`. Cheap enough for
    /// the UI thread: no list is read.
    pub fn from_cache(sources: &[Source], cache: &Path) -> Option<Blocker> {
        if sources.is_empty() {
            return Some(Blocker::from_lists(&[]).with_bundled_resources());
        }
        let stored = std::fs::read_to_string(key_path(cache)).ok()?;
        if stored != cache_key(sources) {
            return None;
        }
        Blocker::load(cache)
            .ok()
            .map(Blocker::with_bundled_resources)
    }

    /// Compiles `sources` (with the bundled resources), uncached.
    pub fn from_sources(sources: &[Source]) -> Blocker {
        Self::compile(sources, None)
    }

    /// Compiles `sources` and rewrites `cache`, if given. Slow for real
    /// lists (seconds): call it off the UI thread.
    pub fn compile(sources: &[Source], cache: Option<&Path>) -> Blocker {
        let blocker = Blocker::from_parsed(sources);
        if let Some(cache) = cache.filter(|_| !sources.is_empty()) {
            // The key is written after the cache, so a crash between the two
            // leaves a stale key that no longer matches, never a wrong hit.
            let saved = blocker
                .save(cache)
                .and_then(|()| Ok(std::fs::write(key_path(cache), cache_key(sources))?));
            if let Err(e) = saved {
                eprintln!("wubble-blocker: cache not written: {e}");
            }
        }
        blocker.with_bundled_resources()
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

fn cache_key(sources: &[Source]) -> String {
    std::iter::once(KEY_SCHEME.to_owned())
        .chain(sources.iter().map(Source::key))
        .collect()
}

fn key_path(cache: &Path) -> PathBuf {
    let mut key = cache.as_os_str().to_owned();
    key.push(".key");
    key.into()
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

    /// No catalog list: only the directory's own files.
    fn none() -> Selection {
        Selection {
            lists: catalog::LISTS
                .iter()
                .map(|l| (l.id.to_owned(), 0))
                .collect(),
            ..Selection::default()
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

    fn names(sources: &[Source]) -> Vec<String> {
        sources
            .iter()
            .map(|s| match &s.origin {
                Origin::File(p) => p.file_name().unwrap().to_string_lossy().into_owned(),
                Origin::Bundled(_) => format!("bundled {}", s.def.unwrap().id),
                Origin::Text(_) => "text".into(),
            })
            .collect()
    }

    #[test]
    fn reads_only_txt_files() {
        let d = Dir::new("txt");
        d.write("a.txt", "||a.test^\n");
        d.write("b.txt", "||b.test^\n");
        d.write("c.md", "||c.test^\n");
        let s = sources(&d.lists(), &none());
        assert_eq!(names(&s), ["a.txt", "b.txt"]);
        let b = Blocker::from_sources_cached(&s, &d.cache());
        assert!(blocks(&b, "https://a.test/x", ResourceType::Script));
        assert!(blocks(&b, "https://b.test/x", ResourceType::Script));
        assert!(!blocks(&b, "https://c.test/x", ResourceType::Script));
    }

    #[test]
    fn a_downloaded_copy_overrides_the_bundled_list_and_disabled_ones_are_skipped() {
        let d = Dir::new("override");
        d.write("easylist.txt", "||e.test^\n");
        d.write("ublock-filters.txt", "||off.test^\n");
        let sel = Selection {
            lists: [("ublock-filters".to_owned(), 0)].into(),
            ..Selection::default()
        };
        let n = names(&sources(&d.lists(), &sel));
        assert!(n.contains(&"easylist.txt".to_owned()), "{n:?}");
        assert!(n.contains(&"bundled easyprivacy".to_owned()), "{n:?}");
        // Neither the disabled list's file nor its bundled copy.
        assert!(!n.iter().any(|n| n.contains("ublock-filters")), "{n:?}");
        // Not bundled, not downloaded: absent.
        assert!(!n.iter().any(|n| n.contains("plowe-0")), "{n:?}");
    }

    #[test]
    fn custom_lists_load_only_while_selected() {
        let d = Dir::new("custom");
        let url = "https://lists.example/mine.txt";
        let file = ListSpec::custom(url).file_name();
        d.write(&file, "||c.test^\n");
        d.write("user.txt", "||u.test^\n");
        assert_eq!(names(&sources(&d.lists(), &none())), ["user.txt"]);
        let sel = Selection {
            custom: vec![url.to_owned()],
            ..none()
        };
        assert_eq!(
            names(&sources(&d.lists(), &sel)),
            [file.as_str(), "user.txt"]
        );
    }

    #[test]
    fn no_sources_allow_all_and_write_no_cache() {
        let d = Dir::new("empty");
        let b = Blocker::from_sources_cached(&[], &d.cache());
        assert!(!blocks(&b, "https://a.test/x", ResourceType::Script));
        assert!(sources(&d.0.join("missing"), &none()).is_empty());
        assert!(!d.cache().exists());
    }

    #[test]
    fn a_matching_cache_is_loaded_instead_of_the_lists() {
        let d = Dir::new("hit");
        d.write("a.txt", "||a.test^\n");
        let s = sources(&d.lists(), &none());
        assert!(Blocker::from_cache(&s, &d.cache()).is_none());
        Blocker::from_sources_cached(&s, &d.cache());
        assert!(key_path(&d.cache()).exists());
        // Swap the cache for another engine, key untouched: a hit loads it.
        Blocker::from_lists(&["||other.test^\n"])
            .save(&d.cache())
            .unwrap();
        let b = Blocker::from_cache(&s, &d.cache()).expect("cache hit");
        assert!(blocks(&b, "https://other.test/x", ResourceType::Script));
        assert!(!blocks(&b, "https://a.test/x", ResourceType::Script));
    }

    #[test]
    fn a_changed_list_or_selection_rebuilds() {
        let d = Dir::new("miss");
        d.write("a.txt", "||a.test^\n");
        let s = sources(&d.lists(), &none());
        Blocker::from_sources_cached(&s, &d.cache());
        d.write("a.txt", "||a.test^\n||more.test^\n");
        assert!(Blocker::from_cache(&s, &d.cache()).is_none());
        let b = Blocker::from_sources_cached(&s, &d.cache());
        assert!(blocks(&b, "https://more.test/x", ResourceType::Script));
        // The rebuilt cache matches again; another selection does not.
        assert!(Blocker::from_cache(&s, &d.cache()).is_some());
        let more = sources(&d.lists(), &Selection::default());
        assert!(Blocker::from_cache(&more, &d.cache()).is_none());
    }

    #[test]
    fn a_corrupt_cache_rebuilds() {
        let d = Dir::new("corrupt");
        d.write("a.txt", "||a.test^\n");
        let s = sources(&d.lists(), &none());
        Blocker::from_sources_cached(&s, &d.cache());
        std::fs::write(d.cache(), b"garbage").unwrap();
        assert!(Blocker::from_cache(&s, &d.cache()).is_none());
        let b = Blocker::from_sources_cached(&s, &d.cache());
        assert!(blocks(&b, "https://a.test/x", ResourceType::Script));
    }

    #[test]
    fn bundled_keys_name_the_snapshot() {
        let key = cache_key(&bundled_sources(&Selection::default()));
        assert!(key.starts_with(KEY_SCHEME));
        assert!(key.contains("\neasylist bundled "), "{key}");
        assert_eq!(key, cache_key(&bundled_sources(&Selection::default())));
    }
}
