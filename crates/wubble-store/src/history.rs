//! History: `urls` + `visits` (Chromium url_database.cc / visit_database.cc shape) with a
//! Firefox Places style frecency computed at query time from a bound `now_ms`, so nothing
//! stored needs recalculating.

use crate::{Result, Store, narrow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Link = 0,
    Typed = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryHit {
    pub url: String,
    pub title: String,
    pub visit_count: u32,
    pub typed_count: u32,
    pub last_visit_ms: i64,
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

    /// Command-bar history suggestions for `prefix`, best first. An empty prefix returns nothing.
    pub async fn query_history(
        &self,
        prefix: &str,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<HistoryHit>> {
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
            hits.push(HistoryHit {
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

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000 * DAY_MS;

    fn ago(days: i64) -> i64 {
        NOW - days * DAY_MS
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
                vec![HistoryHit {
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
}
