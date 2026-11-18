//! History: `urls` + `visits` (Chromium url_database.cc / visit_database.cc shape) with a
//! Firefox Places style frecency computed at query time from a bound `now_ms`, so nothing
//! stored needs recalculating.

use turso::Connection;
use wubble_core::omnibox::HistoryEntry;
use wubble_core::pages::HistoryRow;

use crate::{HistoryRetentionDays, Result, Store, ids, narrow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Link = 0,
    Typed = 1,
}

/// What a delete removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryDeleted {
    pub visits: u64,
    /// Urls dropped because no visit was left.
    pub urls: u64,
}

/// Only web pages are history; internal and file URLs never are.
pub fn is_recordable(url: &str) -> bool {
    ["http://", "https://"]
        .iter()
        .any(|p| url.len() > p.len() && url[..p.len()].eq_ignore_ascii_case(p))
}

/// Match key: lowercase, scheme and a leading `www.` removed (Places autofill idea), so
/// typing `exa` finds `https://www.example.com/`.
pub fn url_key(url: &str) -> String {
    let lower = url.trim().to_lowercase();
    let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
    rest.strip_prefix("www.").unwrap_or(rest).to_string()
}

/// Exclusive upper bound of the keys starting with `key` (`char::MAX` sorts after any char a
/// URL key realistically holds). Turns a prefix match into an index range scan, no LIKE escaping.
fn prefix_upper(key: &str) -> String {
    format!("{key}{}", char::MAX)
}

const DAY_MS: i64 = 86_400_000;

/// Prefix-of-URL hits outrank title-only hits; then score = (typed*3 + visits) * recency
/// bucket, with Places' weights 100/70/50/30/10 for age <= 4/14/31/90/older days.
const QUERY: &str = "\
SELECT url, title, visit_count, typed_count, last_visit_time FROM urls \
WHERE hidden = 0 AND ((url_key >= ?1 AND url_key < ?2) OR instr(lower(title), ?3) > 0) \
ORDER BY \
  (CASE WHEN url_key >= ?1 AND url_key < ?2 THEN 1000000000 ELSE 0 END \
   + (typed_count * 3 + visit_count) * CASE \
       WHEN (?4 - last_visit_time) <= 4 * ?5 THEN 100 \
       WHEN (?4 - last_visit_time) <= 14 * ?5 THEN 70 \
       WHEN (?4 - last_visit_time) <= 31 * ?5 THEN 50 \
       WHEN (?4 - last_visit_time) <= 90 * ?5 THEN 30 \
       ELSE 10 END) DESC, \
  last_visit_time DESC, id \
LIMIT ?6";

impl Store {
    /// Record a visit at `at_ms`. Returns the visit id, or `None` for non-web URLs.
    pub async fn record_visit(
        &mut self,
        url: &str,
        title: &str,
        at_ms: i64,
        transition: Transition,
    ) -> Result<Option<i64>> {
        if !is_recordable(url) {
            return Ok(None);
        }
        let typed = (transition == Transition::Typed) as i64;
        let tx = self.conn.transaction().await?;
        tx.execute(
            "INSERT INTO urls(url, url_key, title, visit_count, typed_count, last_visit_time) \
             VALUES (?1, ?2, ?3, 1, ?4, ?5) \
             ON CONFLICT(url) DO UPDATE SET \
               visit_count = visit_count + 1, \
               typed_count = typed_count + excluded.typed_count, \
               last_visit_time = MAX(last_visit_time, excluded.last_visit_time), \
               title = CASE WHEN excluded.title <> '' THEN excluded.title ELSE title END",
            (url, url_key(url), title, typed, at_ms),
        )
        .await?;
        let mut rows = tx
            .query("SELECT id FROM urls WHERE url = ?1", [url])
            .await?;
        let url_id: i64 = match rows.next().await? {
            Some(r) => r.get(0)?,
            None => return Ok(None),
        };
        drop(rows);
        tx.execute(
            "INSERT INTO visits(url_id, visit_time, transition) VALUES (?1, ?2, ?3)",
            (url_id, at_ms, transition as i64),
        )
        .await?;
        let visit_id = tx.last_insert_rowid();
        tx.commit().await?;
        Ok(Some(visit_id))
    }

    /// Set the stored title of an already recorded `url`, without counting a visit. An empty
    /// title (pages report one while loading) and a URL never recorded change nothing.
    pub async fn update_title(&mut self, url: &str, title: &str) -> Result<()> {
        if title.is_empty() {
            return Ok(());
        }
        self.conn
            .execute("UPDATE urls SET title = ?2 WHERE url = ?1", (url, title))
            .await?;
        Ok(())
    }

    /// One page of the history page's rows: visits strictly before the `(visit_ms, visit_id)`
    /// cursor of the last row already shown (`None`: from the newest), newest first, at most
    /// `limit`. The id breaks ties between visits with the same millisecond. Searching and day grouping are
    /// `wubble_core::pages::history_page`.
    pub async fn list_visits(
        &self,
        limit: usize,
        before: Option<(i64, i64)>,
    ) -> Result<Vec<HistoryRow>> {
        let mut rows = self
            .conn
            .query(
                "SELECT u.url, u.title, v.visit_time, v.id FROM visits v JOIN urls u ON u.id = v.url_id \
                 WHERE v.visit_time < ?1 OR (v.visit_time = ?1 AND v.id < ?2) \
                 ORDER BY v.visit_time DESC, v.id DESC LIMIT ?3",
                {
                    let (ms, id) = before.unwrap_or((i64::MAX, i64::MAX));
                    (ms, id, limit as i64)
                },
            )
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(HistoryRow {
                url: r.get(0)?,
                title: r.get(1)?,
                visit_ms: r.get(2)?,
                visit_id: r.get(3)?,
            });
        }
        Ok(out)
    }

    /// Command-bar history suggestions for `prefix`, best first. Each entry carries the frecency
    /// inputs `wubble_core::omnibox::rank` needs (visit count, typed count, last visit). An empty prefix returns nothing.
    pub async fn query_history(
        &self,
        prefix: &str,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<HistoryEntry>> {
        let key = url_key(prefix);
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let title_needle = prefix.trim().to_lowercase();
        let mut rows = self
            .conn
            .query(
                QUERY,
                (
                    key.as_str(),
                    prefix_upper(&key),
                    title_needle,
                    now_ms,
                    DAY_MS,
                    limit as i64,
                ),
            )
            .await?;
        let mut hits = Vec::new();
        while let Some(r) = rows.next().await? {
            hits.push(HistoryEntry {
                url: r.get(0)?,
                title: r.get(1)?,
                visit_count: narrow(r.get(2)?, "visit_count")?,
                typed_count: narrow(r.get(3)?, "typed_count")?,
                last_visit_ms: r.get(4)?,
            });
        }
        Ok(hits)
    }
}

/// Fix the denormalised counters of `url_ids` from their visits and drop the urls that have no
/// visit left. Returns how many urls were dropped. Cost scales with the affected urls only.
pub(crate) async fn recompute_url_counters(c: &Connection, url_ids: &[i64]) -> Result<u64> {
    let mut update = c
        .prepare(
            "UPDATE urls SET \
               visit_count = (SELECT COUNT(*) FROM visits WHERE url_id = ?1), \
               typed_count = (SELECT COUNT(*) FROM visits WHERE url_id = ?1 AND transition = ?2), \
               last_visit_time = COALESCE((SELECT MAX(visit_time) FROM visits WHERE url_id = ?1), last_visit_time) \
             WHERE id = ?1",
        )
        .await?;
    let mut drop_empty = c
        .prepare("DELETE FROM urls WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM visits WHERE url_id = ?1)")
        .await?;
    let mut dropped = 0;
    for &id in url_ids {
        update.execute((id, Transition::Typed as i64)).await?;
        dropped += drop_empty.execute([id]).await?;
    }
    Ok(dropped)
}

/// Delete every visit with `from_ms <= visit_time < to_ms`, then recompute the counters of the
/// urls they belonged to (Chromium `ExpireHistoryBackend` / Places `removeVisitsByFilter` idea:
/// delete visits, fix the url row, drop it when no visit is left).
async fn delete_visits_between(c: &Connection, from_ms: i64, to_ms: i64) -> Result<HistoryDeleted> {
    let affected = ids(
        c,
        "SELECT DISTINCT url_id FROM visits WHERE visit_time >= ?1 AND visit_time < ?2",
        (from_ms, to_ms),
    )
    .await?;
    let visits = c
        .execute(
            "DELETE FROM visits WHERE visit_time >= ?1 AND visit_time < ?2",
            (from_ms, to_ms),
        )
        .await?;
    let urls = recompute_url_counters(c, &affected).await?;
    Ok(HistoryDeleted { visits, urls })
}

/// Delete the given urls with all their visits.
async fn delete_urls(c: &Connection, url_ids: &[i64]) -> Result<HistoryDeleted> {
    let mut out = HistoryDeleted::default();
    for &id in url_ids {
        out.visits += c
            .execute("DELETE FROM visits WHERE url_id = ?1", [id])
            .await?;
        out.urls += c.execute("DELETE FROM urls WHERE id = ?1", [id]).await?;
    }
    Ok(out)
}

/// Whether `key` (a `url_key`) belongs to the site `host_key`: the host, then end or a
/// separator, so `example.com` does not match `example.community`.
fn on_site(key: &str, host_key: &str) -> bool {
    key.strip_prefix(host_key)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', ':', '?', '#']))
}

impl Store {
    /// Delete visits before `cutoff_ms` (exclusive).
    pub async fn expire_history_older_than(&mut self, cutoff_ms: i64) -> Result<HistoryDeleted> {
        self.delete_history_range(i64::MIN, cutoff_ms).await
    }

    /// "Clear recent history": delete visits with `from_ms <= time < to_ms`.
    pub async fn delete_history_range(
        &mut self,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<HistoryDeleted> {
        let tx = self.conn.transaction().await?;
        let out = delete_visits_between(&tx, from_ms, to_ms).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// Forget one page: the url and all its visits.
    pub async fn delete_history_url(&mut self, url: &str) -> Result<HistoryDeleted> {
        let tx = self.conn.transaction().await?;
        let found = ids(&tx, "SELECT id FROM urls WHERE url = ?1", [url]).await?;
        let out = delete_urls(&tx, &found).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// "Forget this site": every url on `host` (scheme, `www.` and path ignored). Subdomains
    /// are other sites: there is no public-suffix list to tell them apart.
    pub async fn delete_history_site(&mut self, host: &str) -> Result<HistoryDeleted> {
        let host_key = url_key(host);
        let host_key = host_key.split(['/', '?', '#']).next().unwrap_or_default();
        if host_key.is_empty() {
            return Ok(HistoryDeleted::default());
        }
        let tx = self.conn.transaction().await?;
        let mut rows = tx
            .query(
                "SELECT id, url_key FROM urls WHERE url_key >= ?1 AND url_key < ?2",
                (host_key, prefix_upper(host_key)),
            )
            .await?;
        let mut found = Vec::new();
        while let Some(r) = rows.next().await? {
            if on_site(&r.get::<String>(1)?, host_key) {
                found.push(r.get::<i64>(0)?);
            }
        }
        drop(rows);
        let out = delete_urls(&tx, &found).await?;
        tx.commit().await?;
        Ok(out)
    }

    /// Apply the retention setting: drop visits older than `HistoryRetentionDays` before
    /// `now_ms`; nothing when it is 0 (keep everything). The app decides when to call it.
    pub async fn expire_history(&mut self, now_ms: i64) -> Result<HistoryDeleted> {
        match self.setting::<HistoryRetentionDays>().await? {
            0 => Ok(HistoryDeleted::default()),
            days => {
                let age = i64::from(days).saturating_mul(DAY_MS);
                self.expire_history_older_than(now_ms.saturating_sub(age))
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HistoryRetentionDays;

    fn run<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    const NOW: i64 = 1_000 * DAY_MS;

    fn ago(days: i64) -> i64 {
        NOW - days * DAY_MS
    }

    #[test]
    fn list_visits_is_newest_first_one_row_per_visit_and_pages() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for (url, t) in [
                ("https://a.test/", 10),
                ("https://b.test/", 30),
                ("https://a.test/", 20),
            ] {
                s.record_visit(url, "T", t, Transition::Link).await.unwrap();
            }
            let all = s.list_visits(10, None).await.unwrap();
            let got: Vec<(&str, i64)> = all.iter().map(|r| (r.url.as_str(), r.visit_ms)).collect();
            assert_eq!(
                got,
                [
                    ("https://b.test/", 30),
                    ("https://a.test/", 20),
                    ("https://a.test/", 10)
                ]
            );
            assert_eq!(all[0].title, "T");
            assert_eq!(s.list_visits(2, None).await.unwrap().len(), 2);
            let cur = |r: &HistoryRow| Some((r.visit_ms, r.visit_id));
            let older = s.list_visits(10, cur(&all[1])).await.unwrap();
            assert_eq!(older.len(), 1);
            assert_eq!(older[0].visit_ms, 10);
            assert!(s.list_visits(10, cur(&all[2])).await.unwrap().is_empty());
        });
    }

    #[test]
    fn list_visits_pages_across_equal_timestamps() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for i in 0..5 {
                let url = format!("https://a.test/{i}");
                s.record_visit(&url, "T", 100, Transition::Link)
                    .await
                    .unwrap();
            }
            let mut seen = Vec::new();
            let mut cursor = None;
            loop {
                let page = s.list_visits(2, cursor).await.unwrap();
                let Some(last) = page.last() else { break };
                cursor = Some((last.visit_ms, last.visit_id));
                seen.extend(page.into_iter().map(|r| r.url));
            }
            assert_eq!(seen.len(), 5);
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), 5);
        });
    }

    #[test]
    fn url_key_strips_scheme_and_www() {
        assert_eq!(url_key("https://www.Example.com/A"), "example.com/a");
        assert_eq!(url_key("http://example.com"), "example.com");
        assert_eq!(url_key("exa"), "exa");
        assert_eq!(url_key("www.go"), "go");
    }

    #[test]
    fn prefix_upper_bounds_the_prefix_range() {
        assert!("abc" < prefix_upper("abc").as_str());
        assert!("abc~zzz" < prefix_upper("abc").as_str());
        assert!("abd" > prefix_upper("abc").as_str());
    }

    #[test]
    fn only_web_urls_are_recordable() {
        assert!(is_recordable("https://a.com"));
        assert!(is_recordable("HTTP://a.com"));
        assert!(!is_recordable("https://"));
        assert!(!is_recordable("file:///etc/passwd"));
        assert!(!is_recordable("about:blank"));
    }

    #[test]
    fn older_visit_does_not_rewind_last_visit() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.record_visit("https://a.com/", "A", 50, Transition::Link)
                .await
                .unwrap();
            s.record_visit("https://a.com/", "A", 20, Transition::Link)
                .await
                .unwrap();
            let hits = s.query_history("a.com", 60, 10).await.unwrap();
            assert_eq!(hits[0].last_visit_ms, 50);
            assert_eq!(hits[0].visit_count, 2);
        });
    }

    #[test]
    fn visits_accumulate_and_keep_title() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = s
                .record_visit("https://a.com/", "A", 10, Transition::Typed)
                .await
                .unwrap();
            let b = s
                .record_visit("https://a.com/", "", 20, Transition::Link)
                .await
                .unwrap();
            assert!(a.is_some() && b.is_some() && a != b);
            assert_eq!(
                s.record_visit("file:///x", "x", 1, Transition::Link)
                    .await
                    .unwrap(),
                None
            );
            let hits = s.query_history("a.com", 30, 10).await.unwrap();
            assert_eq!(
                hits,
                vec![HistoryEntry {
                    url: "https://a.com/".into(),
                    title: "A".into(),
                    visit_count: 2,
                    typed_count: 1,
                    last_visit_ms: 20,
                }]
            );
            let mut rows = s
                .conn
                .query("SELECT count(*) FROM visits", ())
                .await
                .unwrap();
            assert_eq!(
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
                2
            );
        });
    }

    async fn urls(s: &Store, prefix: &str, limit: usize) -> Vec<String> {
        s.query_history(prefix, NOW, limit)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.url)
            .collect()
    }

    #[test]
    fn ranking_typed_recency_and_prefix() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let l = Transition::Link;
            s.record_visit("https://news.old.com/", "", ago(200), l)
                .await
                .unwrap();
            s.record_visit("https://news.new.com/", "", ago(1), l)
                .await
                .unwrap();
            s.record_visit("https://news.typed.com/", "", ago(200), Transition::Typed)
                .await
                .unwrap();
            // Title-only match never beats a URL prefix match.
            s.record_visit(
                "https://other.com/",
                "Latest news",
                ago(0),
                Transition::Typed,
            )
            .await
            .unwrap();
            assert_eq!(
                urls(&s, "news", 10).await,
                [
                    "https://news.new.com/",   // 1 visit * 100
                    "https://news.typed.com/", // typed 3 * 10 = 30
                    "https://news.old.com/",   // 1 * 10
                    "https://other.com/",      // title only
                ]
            );
            assert_eq!(urls(&s, "news", 2).await.len(), 2);
            assert_eq!(
                urls(&s, "https://www.news.n", 10).await,
                ["https://news.new.com/"]
            );
            assert!(urls(&s, "", 10).await.is_empty());
            assert!(urls(&s, "  ", 10).await.is_empty());
        });
    }

    #[test]
    fn scheme_and_www_are_ignored_when_matching() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.record_visit("https://www.example.com/x", "", 1, Transition::Link)
                .await
                .unwrap();
            assert_eq!(urls(&s, "exa", 10).await, ["https://www.example.com/x"]);
            assert_eq!(urls(&s, "www.exa", 10).await, ["https://www.example.com/x"]);
        });
    }

    #[test]
    fn wildcards_are_literal() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.record_visit("https://a.com/50%25", "", 1, Transition::Link)
                .await
                .unwrap();
            s.record_visit("https://a.com/a_b", "", 1, Transition::Link)
                .await
                .unwrap();
            s.record_visit("https://a.com/axb", "", 1, Transition::Link)
                .await
                .unwrap();
            assert_eq!(urls(&s, "a.com/a_", 10).await, ["https://a.com/a_b"]);
            assert_eq!(urls(&s, "a.com/50%", 10).await, ["https://a.com/50%25"]);
        });
    }

    #[test]
    fn update_title_changes_only_the_title() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.record_visit("https://a.com/", "", 10, Transition::Typed)
                .await
                .unwrap();
            s.update_title("https://a.com/", "A").await.unwrap();
            s.update_title("https://a.com/", "").await.unwrap();
            s.update_title("https://never.com/", "N").await.unwrap();
            let hits = s.query_history("a.com", 20, 10).await.unwrap();
            assert_eq!(
                hits,
                vec![HistoryEntry {
                    url: "https://a.com/".into(),
                    title: "A".into(),
                    visit_count: 1,
                    typed_count: 1,
                    last_visit_ms: 10,
                }]
            );
            assert!(s.query_history("never", 20, 10).await.unwrap().is_empty());
        });
    }

    async fn visit(s: &mut Store, url: &str, at: i64, t: Transition) {
        s.record_visit(url, "", at, t).await.unwrap();
    }

    async fn consistent(s: &Store) {
        let p = s.check_consistency().await.unwrap();
        assert!(p.is_empty(), "{p:?}");
    }

    async fn entry(s: &Store, prefix: &str) -> Option<HistoryEntry> {
        s.query_history(prefix, NOW, 10)
            .await
            .unwrap()
            .into_iter()
            .next()
    }

    #[test]
    fn expiry_removes_old_visits_and_fixes_counters() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            visit(&mut s, "https://a.test/", 10, Transition::Typed).await;
            visit(&mut s, "https://a.test/", 20, Transition::Link).await;
            visit(&mut s, "https://a.test/", 30, Transition::Link).await;
            visit(&mut s, "https://b.test/", 5, Transition::Link).await;
            consistent(&s).await;
            let d = s.expire_history_older_than(20).await.unwrap(); // 20 itself stays
            assert_eq!(d, HistoryDeleted { visits: 2, urls: 1 });
            let a = entry(&s, "a.test").await.unwrap();
            assert_eq!((a.visit_count, a.typed_count, a.last_visit_ms), (2, 0, 30));
            assert!(entry(&s, "b.test").await.is_none());
            consistent(&s).await;
            // Everything older than the newest: last visit stays right; nothing more to do twice.
            assert_eq!(
                s.expire_history_older_than(20).await.unwrap(),
                HistoryDeleted::default()
            );
        });
    }

    #[test]
    fn range_is_from_inclusive_to_exclusive() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for at in [10, 20, 30, 40] {
                visit(&mut s, "https://a.test/", at, Transition::Link).await;
            }
            let d = s.delete_history_range(20, 40).await.unwrap();
            assert_eq!(d, HistoryDeleted { visits: 2, urls: 0 });
            let a = entry(&s, "a.test").await.unwrap();
            assert_eq!((a.visit_count, a.last_visit_ms), (2, 40));
            let d = s.delete_history_range(0, 100).await.unwrap();
            assert_eq!(d, HistoryDeleted { visits: 2, urls: 1 });
            assert!(entry(&s, "a.test").await.is_none());
            consistent(&s).await;
        });
    }

    #[test]
    fn range_delete_recomputes_typed_count_and_last_visit() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            visit(&mut s, "https://a.test/", 10, Transition::Typed).await;
            visit(&mut s, "https://a.test/", 20, Transition::Typed).await;
            visit(&mut s, "https://a.test/", 30, Transition::Link).await;
            s.delete_history_range(25, 100).await.unwrap(); // the newest, a plain link
            let a = entry(&s, "a.test").await.unwrap();
            assert_eq!((a.visit_count, a.typed_count, a.last_visit_ms), (2, 2, 20));
            s.delete_history_range(0, 15).await.unwrap(); // a typed one
            let a = entry(&s, "a.test").await.unwrap();
            assert_eq!((a.visit_count, a.typed_count, a.last_visit_ms), (1, 1, 20));
            consistent(&s).await;
        });
    }

    #[test]
    fn delete_url_removes_only_that_url() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            visit(&mut s, "https://a.test/x", 1, Transition::Link).await;
            visit(&mut s, "https://a.test/x", 2, Transition::Link).await;
            visit(&mut s, "https://a.test/y", 3, Transition::Link).await;
            let d = s.delete_history_url("https://a.test/x").await.unwrap();
            assert_eq!(d, HistoryDeleted { visits: 2, urls: 1 });
            assert_eq!(urls(&s, "a.test", 10).await, ["https://a.test/y"]);
            assert_eq!(
                s.delete_history_url("https://a.test/x").await.unwrap(),
                HistoryDeleted::default()
            );
            consistent(&s).await;
        });
    }

    #[test]
    fn delete_site_matches_the_host_only() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for u in [
                "https://example.com/",
                "https://www.example.com/a?q=1",
                "http://example.com:8080/b",
                "https://example.community/",
                "https://example.com.evil.test/",
                "https://sub.example.com/",
                "https://other.test/",
            ] {
                visit(&mut s, u, 1, Transition::Link).await;
            }
            let d = s
                .delete_history_site("https://www.Example.com/whatever")
                .await
                .unwrap();
            assert_eq!(d, HistoryDeleted { visits: 3, urls: 3 });
            let mut left = vec![];
            let mut rows = s
                .conn
                .query("SELECT url FROM urls ORDER BY url", ())
                .await
                .unwrap();
            while let Some(r) = rows.next().await.unwrap() {
                left.push(r.get::<String>(0).unwrap());
            }
            assert_eq!(
                left,
                [
                    "https://example.com.evil.test/",
                    "https://example.community/",
                    "https://other.test/",
                    "https://sub.example.com/",
                ]
            );
            assert_eq!(
                s.delete_history_site("").await.unwrap(),
                HistoryDeleted::default()
            );
            consistent(&s).await;
        });
    }

    #[test]
    fn retention_setting_zero_keeps_everything_and_n_days_expires() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            visit(&mut s, "https://old.test/", ago(100), Transition::Link).await;
            visit(&mut s, "https://mid.test/", ago(40), Transition::Link).await;
            visit(&mut s, "https://new.test/", ago(1), Transition::Link).await;
            assert_eq!(s.setting::<HistoryRetentionDays>().await.unwrap(), 0);
            assert_eq!(
                s.expire_history(NOW).await.unwrap(),
                HistoryDeleted::default()
            );
            assert!(entry(&s, "old.test").await.is_some());
            s.set_setting::<HistoryRetentionDays>(90).await.unwrap();
            assert_eq!(
                s.expire_history(NOW).await.unwrap(),
                HistoryDeleted { visits: 1, urls: 1 }
            );
            s.set_setting::<HistoryRetentionDays>(30).await.unwrap();
            assert_eq!(
                s.expire_history(NOW).await.unwrap(),
                HistoryDeleted { visits: 1, urls: 1 }
            );
            assert!(entry(&s, "mid.test").await.is_none());
            assert!(entry(&s, "new.test").await.is_some());
            consistent(&s).await;
        });
    }
}
