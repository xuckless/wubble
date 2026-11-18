use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use crate::lists::RESOURCES_FILE;

/// A filter list to keep fresh. `interval_secs` follows the list's own `Expires`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListSpec {
    pub id: &'static str,
    pub url: &'static str,
    pub interval_secs: u64,
}

const DAY: u64 = 86_400;

/// Default lists plus the redirect/scriptlet resources JSON (Brave adblock-resources).
/// Fetched at runtime only, never bundled; the fetcher lives behind an off-by-default flag.
pub const DEFAULT_LISTS: &[ListSpec] = &[
    ListSpec {
        id: "easylist",
        url: "https://easylist.to/easylist/easylist.txt",
        interval_secs: 4 * DAY,
    },
    ListSpec {
        id: "easyprivacy",
        url: "https://easylist.to/easylist/easyprivacy.txt",
        interval_secs: 4 * DAY,
    },
    ListSpec {
        id: "ublock-filters",
        url: "https://ublockorigin.github.io/uAssets/filters/filters.txt",
        interval_secs: 5 * DAY,
    },
    ListSpec {
        id: "ublock-unbreak",
        url: "https://ublockorigin.github.io/uAssets/filters/unbreak.txt",
        interval_secs: 5 * DAY,
    },
    ListSpec {
        id: "resources",
        url: "https://raw.githubusercontent.com/brave/adblock-resources/master/dist/resources.json",
        interval_secs: 7 * DAY,
    },
];

/// Lists due for a fetch, in input order. `last_fetch` maps list id to unix seconds.
/// Never fetched, or a last fetch in the future (clock moved back), counts as due.
pub fn due<'a>(
    lists: &'a [ListSpec],
    last_fetch: &BTreeMap<String, u64>,
    now: u64,
) -> Vec<&'a ListSpec> {
    lists
        .iter()
        .filter(|l| match last_fetch.get(l.id) {
            None => true,
            Some(&last) if last > now => true,
            Some(&last) => now - last >= l.interval_secs,
        })
        .collect()
}

impl ListSpec {
    /// The list's file in a list directory: `resources.json` for the
    /// resources, `<id>.txt` for filter lists (what `Blocker::from_dir` reads).
    pub fn file_name(&self) -> String {
        if self.id == RESOURCES_ID {
            RESOURCES_FILE.to_owned()
        } else {
            format!("{}.txt", self.id)
        }
    }
}

/// The id of the resources entry in [`DEFAULT_LISTS`].
const RESOURCES_ID: &str = "resources";

/// When each list in `dir` was last fetched: its file's modification time
/// (unix seconds). Lists without a file are missing, so [`due`].
pub fn last_fetch(dir: &Path, lists: &[ListSpec]) -> BTreeMap<String, u64> {
    lists
        .iter()
        .filter_map(|l| {
            let modified = std::fs::metadata(dir.join(l.file_name()))
                .ok()?
                .modified()
                .ok()?;
            let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((l.id.to_owned(), secs))
        })
        .collect()
}

/// Fetches the lists in `lists` that are [`due`] at `now` into `dir`, each
/// written atomically (temporary file, then rename). A failed fetch keeps
/// the old file. Returns the ids written, in order. Only `fetch` touches
/// the network; the host calls this only with `blocker.lists_auto_update` on.
pub fn refresh(
    dir: &Path,
    lists: &[ListSpec],
    now: u64,
    fetch: &dyn Fn(&str) -> Result<String, String>,
) -> Vec<&'static str> {
    let due = due(lists, &last_fetch(dir, lists), now);
    if due.is_empty() {
        return Vec::new();
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("wubble-blocker: {}: {e}", dir.display());
        return Vec::new();
    }
    let mut written = Vec::new();
    for list in due {
        let text = match fetch(list.url) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("wubble-blocker: {} not fetched: {e}", list.id);
                continue;
            }
        };
        let path = dir.join(list.file_name());
        let tmp = dir.join(format!("{}.tmp", list.file_name()));
        match std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)) {
            Ok(()) => written.push(list.id),
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

    const A: ListSpec = ListSpec {
        id: "a",
        url: "u",
        interval_secs: 100,
    };
    const B: ListSpec = ListSpec {
        id: "b",
        url: "u",
        interval_secs: 1000,
    };
    fn last(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn ids(v: Vec<&ListSpec>) -> Vec<&str> {
        v.into_iter().map(|l| l.id).collect()
    }

    #[test]
    fn never_fetched_is_due() {
        assert_eq!(ids(due(&[A, B], &last(&[]), 5)), ["a", "b"]);
    }

    #[test]
    fn fresh_not_due_and_boundary_due() {
        let l = last(&[("a", 1000), ("b", 1000)]);
        assert!(due(&[A], &l, 1099).is_empty());
        assert_eq!(ids(due(&[A], &l, 1100)), ["a"]);
    }

    #[test]
    fn per_list_interval() {
        let l = last(&[("a", 1000), ("b", 1000)]);
        assert_eq!(ids(due(&[A, B], &l, 1500)), ["a"]);
    }

    #[test]
    fn clock_backwards_is_due() {
        assert_eq!(ids(due(&[A], &last(&[("a", 2000)]), 1000)), ["a"]);
    }

    #[test]
    fn file_names() {
        assert_eq!(A.file_name(), "a.txt");
        let res = DEFAULT_LISTS.iter().find(|l| l.id == "resources").unwrap();
        assert_eq!(res.file_name(), "resources.json");
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
        let lists = [A, ListSpec { url: "ub", ..B }];
        assert_eq!(refresh(&d.0, &lists, now(), &fetch), ["a", "b"]);
        assert_eq!(std::fs::read_to_string(d.0.join("a.txt")).unwrap(), "! u\n");
        assert!(!d.0.join("a.txt.tmp").exists());
        // Just written, so nothing is due an interval later than A's minus one.
        fetched.borrow_mut().clear();
        assert!(refresh(&d.0, &lists, now() + 50, &fetch).is_empty());
        assert!(fetched.borrow().is_empty());
        // A is due after its interval, B is not.
        assert_eq!(refresh(&d.0, &lists, now() + 200, &fetch), ["a"]);
        assert_eq!(*fetched.borrow(), ["u"]);
    }

    #[test]
    fn a_failed_fetch_keeps_the_old_file() {
        let d = dir("fail");
        let ok = |_: &str| Ok("||old.test^\n".to_owned());
        refresh(&d.0, &[A], now(), &ok);
        let fail = |_: &str| Err("offline".to_owned());
        assert!(refresh(&d.0, &[A], now() + 1000, &fail).is_empty());
        assert_eq!(
            std::fs::read_to_string(d.0.join("a.txt")).unwrap(),
            "||old.test^\n"
        );
    }

    #[test]
    fn defaults_have_unique_ids() {
        let mut ids: Vec<_> = DEFAULT_LISTS.iter().map(|l| l.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), DEFAULT_LISTS.len());
    }
}
