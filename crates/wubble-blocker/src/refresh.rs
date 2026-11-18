use std::collections::BTreeMap;

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
    fn defaults_have_unique_ids() {
        let mut ids: Vec<_> = DEFAULT_LISTS.iter().map(|l| l.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), DEFAULT_LISTS.len());
    }
}
