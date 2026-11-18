use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::catalog::{ListDef, Selection};

/// A filter list to keep fresh: a catalog list or a custom one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSpec {
    pub id: String,
    pub url: String,
    pub interval_secs: u64,
}

const DAY: u64 = 86_400;

/// When lists count as due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// On their schedule at this unix time.
    At(u64),
    /// All of them (the user asked for an update).
    All,
}

/// Lists due for a fetch, in input order. `last_fetch` maps list id to unix seconds.
/// Never fetched, or a last fetch in the future (clock moved back), counts as due.
pub fn due<'a>(
    lists: &'a [ListSpec],
    last_fetch: &BTreeMap<String, u64>,
    now: u64,
) -> Vec<&'a ListSpec> {
    lists
        .iter()
        .filter(|l| match last_fetch.get(&l.id) {
            None => true,
            Some(&last) if last > now => true,
            Some(&last) => now - last >= l.interval_secs,
        })
        .collect()
}

impl ListSpec {
    /// File-name prefix of the user's custom lists.
    pub const CUSTOM_PREFIX: &'static str = "custom-";

    /// A custom list: its id is a hash of its URL, so the same URL keeps
    /// its file. Fetched every four days.
    pub fn custom(url: &str) -> Self {
        // FNV-1a: stable across builds, unlike `DefaultHasher`.
        let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        Self {
            id: format!("{}{hash:016x}", Self::CUSTOM_PREFIX),
            url: url.to_owned(),
            interval_secs: 4 * DAY,
        }
    }

    /// The list's file in a list directory (what [`crate::sources`] reads).
    pub fn file_name(&self) -> String {
        format!("{}.txt", self.id)
    }

    /// The file beside it naming the URL it was fetched from.
    fn source_name(&self) -> String {
        format!("{}.src", self.id)
    }

    /// The list's downloaded copy in `dir`, if it was fetched from this
    /// list's URL. A copy from another URL (a catalog list whose source
    /// moved, such as uBO's filters before their CDN copy) is not this list.
    pub fn downloaded(&self, dir: &Path) -> Option<PathBuf> {
        let source = std::fs::read_to_string(dir.join(self.source_name())).ok()?;
        let file = dir.join(self.file_name());
        (source == self.url && file.is_file()).then_some(file)
    }
}

impl From<&ListDef> for ListSpec {
    fn from(def: &ListDef) -> Self {
        Self {
            id: def.id.to_owned(),
            url: def.url.to_owned(),
            interval_secs: def.interval_secs,
        }
    }
}

/// The lists `selection` keeps fresh: its enabled catalog lists, then its
/// custom lists that are HTTPS URLs.
pub fn specs(selection: &Selection) -> Vec<ListSpec> {
    selection
        .enabled()
        .map(ListSpec::from)
        .chain(
            selection
                .custom
                .iter()
                .filter(|u| is_list_url(u))
                .map(|u| ListSpec::custom(u)),
        )
        .collect()
}

/// Whether `url` may be a custom list: HTTPS with a host.
pub fn is_list_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
}

/// When each list in `dir` was last fetched: its file's modification time
/// (unix seconds). Lists without a [`ListSpec::downloaded`] copy are
/// missing, so [`due`].
pub fn last_fetch(dir: &Path, lists: &[ListSpec]) -> BTreeMap<String, u64> {
    lists
        .iter()
        .filter_map(|l| {
            let modified = std::fs::metadata(l.downloaded(dir)?)
                .ok()?
                .modified()
                .ok()?;
            let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((l.id.clone(), secs))
        })
        .collect()
}

/// Fetches the lists in `lists` that are due into `dir`, each written
/// atomically (temporary file, then rename). A failed fetch keeps the old
/// file. Returns the ids written, in order. Only `fetch` touches the
/// network; the host calls this only with `blocker.lists_auto_update` on or
/// when the user asks for an update.
pub fn refresh(
    dir: &Path,
    lists: &[ListSpec],
    when: Due,
    fetch: &dyn Fn(&str) -> Result<String, String>,
) -> Vec<String> {
    let due = match when {
        Due::At(now) => due(lists, &last_fetch(dir, lists), now),
        Due::All => lists.iter().collect(),
    };
    if due.is_empty() {
        return Vec::new();
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("wubble-blocker: {}: {e}", dir.display());
        return Vec::new();
    }
    let mut written = Vec::new();
    for list in due {
        let text = match fetch(&list.url) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("wubble-blocker: {} not fetched: {e}", list.id);
                continue;
            }
        };
        let path = dir.join(list.file_name());
        let tmp = dir.join(format!("{}.tmp", list.file_name()));
        // The source is named after the list is in place: a crash between
        // the two leaves a list that is not trusted yet, never a list
        // trusted under another URL.
        let written_ok = std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .and_then(|()| std::fs::write(dir.join(list.source_name()), &list.url));
        match written_ok {
            Ok(()) => written.push(list.id.clone()),
            Err(e) => eprintln!("wubble-blocker: {} not written: {e}", path.display()),
        }
    }
    written
}

/// The largest list accepted (uBlock's filters are a few MB).
const MAX_LIST_BYTES: u64 = 32 * 1024 * 1024;

/// Fetches `url` over HTTPS: the fetcher [`refresh`] gets in the browser.
pub fn fetch_http(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .https_only(true)
        .build()
        .into();
    agent
        .get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(MAX_LIST_BYTES)
        .read_to_string()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, url: &str, interval_secs: u64) -> ListSpec {
        ListSpec {
            id: id.into(),
            url: url.into(),
            interval_secs,
        }
    }

    fn a() -> ListSpec {
        spec("a", "u", 100)
    }

    fn b() -> ListSpec {
        spec("b", "u", 1000)
    }

    fn last(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn ids(v: Vec<&ListSpec>) -> Vec<&str> {
        v.into_iter().map(|l| l.id.as_str()).collect()
    }

    #[test]
    fn never_fetched_is_due() {
        assert_eq!(ids(due(&[a(), b()], &last(&[]), 5)), ["a", "b"]);
    }

    #[test]
    fn fresh_not_due_and_boundary_due() {
        let l = last(&[("a", 1000), ("b", 1000)]);
        assert!(due(&[a()], &l, 1099).is_empty());
        assert_eq!(ids(due(&[a()], &l, 1100)), ["a"]);
    }

    #[test]
    fn per_list_interval() {
        let l = last(&[("a", 1000), ("b", 1000)]);
        assert_eq!(ids(due(&[a(), b()], &l, 1500)), ["a"]);
    }

    #[test]
    fn clock_backwards_is_due() {
        assert_eq!(ids(due(&[a()], &last(&[("a", 2000)]), 1000)), ["a"]);
    }

    #[test]
    fn custom_lists_have_stable_ids_and_need_https() {
        let c = ListSpec::custom("https://lists.example/a.txt");
        assert_eq!(c, ListSpec::custom("https://lists.example/a.txt"));
        assert_ne!(c.id, ListSpec::custom("https://lists.example/b.txt").id);
        assert!(c.file_name().starts_with(ListSpec::CUSTOM_PREFIX));
        assert!(c.file_name().ends_with(".txt"));
        assert!(is_list_url("https://lists.example/a.txt"));
        assert!(!is_list_url("http://lists.example/a.txt"));
        assert!(!is_list_url("file:///etc/hosts"));
        assert!(!is_list_url("lists.example/a.txt"));
    }

    #[test]
    fn specs_follow_the_selection() {
        let sel = Selection {
            custom: vec!["https://l.example/x.txt".into(), "http://insecure/".into()],
            ..Selection::default()
        };
        let s = specs(&sel);
        assert_eq!(s.first().unwrap().id, "ublock-filters");
        assert_eq!(s.last().unwrap().url, "https://l.example/x.txt");
        assert!(!s.iter().any(|l| l.url.starts_with("http:")));
        assert!(s.iter().any(|l| l.id == "plowe-0"));
    }

    struct Dir(std::path::PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("wubble-refresh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Dir(d)
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn fetches_only_due_lists_and_writes_atomically() {
        let d = dir("due");
        let fetched = std::cell::RefCell::new(Vec::new());
        let fetch = |url: &str| {
            fetched.borrow_mut().push(url.to_owned());
            Ok(format!("! {url}\n"))
        };
        let lists = [a(), spec("b", "ub", 1000)];
        assert_eq!(refresh(&d.0, &lists, Due::At(now()), &fetch), ["a", "b"]);
        assert_eq!(std::fs::read_to_string(d.0.join("a.txt")).unwrap(), "! u\n");
        assert!(!d.0.join("a.txt.tmp").exists());
        // Just written, so nothing is due an interval later than A's minus one.
        fetched.borrow_mut().clear();
        assert!(refresh(&d.0, &lists, Due::At(now() + 50), &fetch).is_empty());
        assert!(fetched.borrow().is_empty());
        // A is due after its interval, B is not.
        assert_eq!(refresh(&d.0, &lists, Due::At(now() + 200), &fetch), ["a"]);
        assert_eq!(*fetched.borrow(), ["u"]);
        // An update the user asked for fetches everything.
        assert_eq!(refresh(&d.0, &lists, Due::All, &fetch), ["a", "b"]);
    }

    /// A copy fetched from a list's old URL (uBO's filters moved to their
    /// CDN copy) is neither used nor counted as fresh.
    #[test]
    fn a_copy_from_another_url_is_not_the_list() {
        let d = dir("moved");
        let old = spec("a", "https://old.example/a.txt", 100);
        let ok = |_: &str| Ok("||old.test^\n".to_owned());
        refresh(&d.0, std::slice::from_ref(&old), Due::At(now()), &ok);
        assert_eq!(old.downloaded(&d.0), Some(d.0.join("a.txt")));
        let moved = spec("a", "https://new.example/a.txt", 100);
        assert_eq!(moved.downloaded(&d.0), None);
        assert!(last_fetch(&d.0, std::slice::from_ref(&moved)).is_empty());
        // A copy without a source file (written before sources were named).
        std::fs::remove_file(d.0.join("a.src")).unwrap();
        assert_eq!(old.downloaded(&d.0), None);
        // The next refresh fetches it from the new URL.
        assert_eq!(
            refresh(&d.0, std::slice::from_ref(&moved), Due::At(now()), &ok),
            ["a"]
        );
        assert_eq!(moved.downloaded(&d.0), Some(d.0.join("a.txt")));
    }

    #[test]
    fn a_failed_fetch_keeps_the_old_file() {
        let d = dir("fail");
        let ok = |_: &str| Ok("||old.test^\n".to_owned());
        refresh(&d.0, &[a()], Due::At(now()), &ok);
        let fail = |_: &str| Err("offline".to_owned());
        assert!(refresh(&d.0, &[a()], Due::At(now() + 1000), &fail).is_empty());
        assert_eq!(
            std::fs::read_to_string(d.0.join("a.txt")).unwrap(),
            "||old.test^\n"
        );
    }
}
