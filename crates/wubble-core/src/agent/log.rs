//! The activity log: every call a session makes, tagged with its session id. A bounded ring in
//! memory ([`ActivityLog`]); the store keeps a longer tail (`STORE_MAX`). An entry names the
//! tool and the page it touched, never an argument's value, so a filled secret cannot land here.

use std::collections::VecDeque;

use super::SessionId;

/// Entries the in-memory ring keeps.
pub const RING_MAX: usize = 500;
/// Rows the store keeps (`agent_log`); older ones are deleted on append.
pub const STORE_MAX: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// A gate said no: a secret off its origin, a site off the list, a bad token.
    Refused,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub session: SessionId,
    /// Unix ms, supplied by the caller.
    pub at_ms: i64,
    /// The MCP tool name, or a lifecycle word (`session_started`, `session_stopped`).
    pub tool: String,
    /// The origin or URL the call touched, if any.
    pub target: Option<String>,
    pub outcome: Outcome,
}

#[derive(Clone, Debug)]
pub struct ActivityLog {
    entries: VecDeque<Entry>,
    cap: usize,
}

impl Default for ActivityLog {
    fn default() -> Self {
        Self::with_capacity(RING_MAX)
    }
}

impl ActivityLog {
    pub fn with_capacity(cap: usize) -> Self {
        ActivityLog {
            entries: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    /// Append; the oldest entry goes when the ring is full.
    pub fn push(&mut self, entry: Entry) {
        if self.entries.len() == self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    /// Oldest first, only `session`'s.
    pub fn of<'a>(&'a self, session: &'a SessionId) -> impl Iterator<Item = &'a Entry> {
        self.entries.iter().filter(move |e| &e.session == session)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One line of the activity log page, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The tool, and the page it touched.
    pub title: String,
    /// The session, how long ago, and how it ended.
    pub note: String,
    /// A gate said no or the call failed.
    pub failed: bool,
}

/// `entry` as a page line; `now_ms` is the clock the age is read against.
pub fn row(entry: &Entry, now_ms: i64) -> Row {
    let title = match &entry.target {
        Some(target) => format!("{}  {target}", entry.tool),
        None => entry.tool.clone(),
    };
    let outcome = match entry.outcome {
        Outcome::Done => "done",
        Outcome::Refused => "refused",
        Outcome::Failed => "failed",
    };
    let age = crate::pages::relative_age(
        u64::try_from(now_ms / 1000).unwrap_or(0),
        u64::try_from(entry.at_ms / 1000).unwrap_or(0),
    );
    Row {
        title,
        note: format!("{} · {age} · {outcome}", entry.session.0),
        failed: entry.outcome != Outcome::Done,
    }
}

/// The newest-first page lines of `entries` (the store keeps them oldest first).
pub fn rows(entries: &[Entry], now_ms: i64) -> Vec<Row> {
    entries.iter().rev().map(|e| row(e, now_ms)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(session: &str, n: i64) -> Entry {
        Entry {
            session: SessionId(session.into()),
            at_ms: n,
            tool: format!("tool{n}"),
            target: None,
            outcome: Outcome::Done,
        }
    }

    #[test]
    fn the_ring_drops_the_oldest() {
        let mut log = ActivityLog::with_capacity(3);
        for n in 0..5 {
            log.push(entry("a", n));
        }
        assert_eq!(log.len(), 3);
        assert_eq!(log.iter().map(|e| e.at_ms).collect::<Vec<_>>(), [2, 3, 4]);
    }

    #[test]
    fn the_default_ring_is_bounded_by_ring_max() {
        let mut log = ActivityLog::default();
        for n in 0..(RING_MAX as i64 + 10) {
            log.push(entry("a", n));
        }
        assert_eq!(log.len(), RING_MAX);
        assert_eq!(log.iter().next().unwrap().at_ms, 10);
    }

    #[test]
    fn entries_filter_by_session() {
        let mut log = ActivityLog::default();
        log.push(entry("a", 1));
        log.push(entry("b", 2));
        log.push(entry("a", 3));
        let a = SessionId("a".into());
        assert_eq!(log.of(&a).map(|e| e.at_ms).collect::<Vec<_>>(), [1, 3]);
        assert!(!log.is_empty());
    }

    #[test]
    fn a_page_line_names_the_tool_the_page_the_session_and_the_outcome() {
        let mut e = entry("s1", 0);
        e.tool = "navigate".into();
        e.target = Some("https://example.com/".into());
        e.at_ms = 1_000_000;
        let r = row(&e, 1_000_000 + 120_000);
        assert_eq!(r.title, "navigate  https://example.com/");
        assert_eq!(r.note, "s1 · 2 minutes ago · done");
        assert!(!r.failed);
        e.target = None;
        e.outcome = Outcome::Refused;
        let r = row(&e, 1_000_000);
        assert_eq!(r.title, "navigate");
        assert_eq!(r.note, "s1 · just now · refused");
        assert!(r.failed);
    }

    #[test]
    fn the_page_lists_the_newest_first() {
        let entries = [entry("a", 1), entry("a", 2), entry("a", 3)];
        let titles: Vec<_> = rows(&entries, 0).into_iter().map(|r| r.title).collect();
        assert_eq!(titles, ["tool3", "tool2", "tool1"]);
    }

    #[test]
    fn a_zero_capacity_still_keeps_one() {
        let mut log = ActivityLog::with_capacity(0);
        log.push(entry("a", 1));
        log.push(entry("a", 2));
        assert_eq!(log.len(), 1);
    }
}
