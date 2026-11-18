//! The snapshots of hidden tabs: a small picture of each tab's last frame, taken when it is
//! hidden, shown while a discarded tab restores (and later in hover previews).
//!
//! [`Book`] is the one owner of the snapshots and their byte budget. It is generic over the
//! picture so it stays free of GPU types; the host stores its textures in it. Taking order is a
//! counter, not a clock: the oldest snapshot is evicted first when the budget is exceeded.

use std::collections::BTreeMap;

use crate::tab::TabId;

#[derive(Debug, Clone, PartialEq)]
struct Entry<T> {
    picture: T,
    bytes: u64,
    /// When it was taken, in insert order.
    taken: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Book<T> {
    entries: BTreeMap<TabId, Entry<T>>,
    budget: u64,
    next: u64,
}

impl<T> Book<T> {
    /// A book holding at most `budget` bytes of pictures.
    pub fn new(budget: u64) -> Self {
        Self {
            entries: BTreeMap::new(),
            budget,
            next: 0,
        }
    }

    /// Files `tab`'s new snapshot, replacing its old one, and evicts the oldest others until the
    /// book fits its budget. Returns the evicted tabs, oldest first. A picture larger than the
    /// whole budget is not kept.
    pub fn insert(&mut self, tab: TabId, picture: T, bytes: u64) -> Vec<TabId> {
        self.entries.remove(&tab);
        if bytes > self.budget {
            return vec![];
        }
        let taken = self.next;
        self.next += 1;
        self.entries.insert(
            tab,
            Entry {
                picture,
                bytes,
                taken,
            },
        );
        let mut evicted = vec![];
        while self.total() > self.budget {
            let oldest = self
                .entries
                .iter()
                .filter(|(t, _)| **t != tab)
                .min_by_key(|(_, e)| e.taken)
                .map(|(t, _)| *t);
            let Some(oldest) = oldest else { break };
            self.entries.remove(&oldest);
            evicted.push(oldest);
        }
        evicted
    }

    pub fn get(&self, tab: TabId) -> Option<&T> {
        self.entries.get(&tab).map(|e| &e.picture)
    }

    /// Drops `tab`'s snapshot; whether it had one.
    pub fn release(&mut self, tab: TabId) -> bool {
        self.entries.remove(&tab).is_some()
    }

    /// Drops every snapshot but those of `keep` (under memory pressure: the tabs restoring on
    /// screen keep theirs).
    pub fn release_except(&mut self, keep: &[TabId]) {
        self.entries.retain(|t, _| keep.contains(t));
    }

    /// Bytes held.
    pub fn total(&self) -> u64 {
        self.entries.values().map(|e| e.bytes).sum()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: u64) -> TabId {
        TabId(n)
    }

    #[test]
    fn keeps_to_its_budget_evicting_the_oldest_first() {
        let mut b = Book::new(100);
        assert!(b.insert(t(1), "a", 40).is_empty());
        assert!(b.insert(t(2), "b", 40).is_empty());
        assert_eq!(b.total(), 80);
        // Needs 60 more than fits: both older ones go, oldest first.
        assert_eq!(b.insert(t(3), "c", 90), vec![t(1), t(2)]);
        assert_eq!((b.len(), b.total()), (1, 90));
        assert_eq!(b.get(t(3)), Some(&"c"));
        assert_eq!(b.get(t(1)), None);
    }

    #[test]
    fn a_new_snapshot_replaces_the_old_and_counts_as_newest() {
        let mut b = Book::new(100);
        b.insert(t(1), "a", 40);
        b.insert(t(2), "b", 40);
        assert!(b.insert(t(1), "a2", 40).is_empty());
        assert_eq!((b.get(t(1)), b.total()), (Some(&"a2"), 80));
        // t(2) is now the oldest.
        assert_eq!(b.insert(t(3), "c", 30), vec![t(2)]);
    }

    #[test]
    fn a_picture_larger_than_the_budget_is_not_kept() {
        let mut b = Book::new(10);
        b.insert(t(1), "a", 5);
        assert!(b.insert(t(1), "big", 11).is_empty());
        assert!(b.is_empty());
    }

    #[test]
    fn release_and_release_except() {
        let mut b = Book::new(100);
        for n in 1..=4 {
            b.insert(t(n), n, 10);
        }
        assert!(b.release(t(2)));
        assert!(!b.release(t(2)));
        assert_eq!(b.total(), 30);
        b.release_except(&[t(3)]);
        assert_eq!((b.len(), b.get(t(3))), (1, Some(&3)));
        b.release_except(&[]);
        assert_eq!(b.total(), 0);
    }
}
