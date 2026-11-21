//! The Archive: tabs that went unvisited long enough to leave the tab list
//! (`wubble_core::archive`). Newest first, searchable and paged like the history page, kept
//! until the user restores or deletes them.
//!
//! The shape is `closed.rs` (no foreign keys: the Space may be gone when a tab is restored)
//! with `history.rs::list_visits`'s search and keyset paging.

use wubble_core::space::{ContainerId, SpaceId};

use crate::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedTab {
    /// The row's id, the cursor tie-break and the key to restore or delete by.
    pub id: i64,
    pub url: String,
    pub title: String,
    pub container: ContainerId,
    pub space: Option<SpaceId>,
    /// Unix ms the tab was last on screen.
    pub last_active_ms: i64,
    /// Unix ms the tab was archived; the page groups by it.
    pub archived_at_ms: i64,
}

const COLUMNS: &str = "id, archived_at, last_active, space_id, container_id, url, title";

fn read(r: &turso::Row) -> Result<ArchivedTab> {
    let space: Option<i64> = r.get(3)?;
    Ok(ArchivedTab {
        id: r.get(0)?,
        archived_at_ms: r.get(1)?,
        last_active_ms: r.get(2)?,
        space: space
            .map(|v| narrow(v, "space id").map(SpaceId))
            .transpose()?,
        container: ContainerId(narrow(r.get(4)?, "container id")?),
        url: r.get(5)?,
        title: r.get(6)?,
    })
}

impl Store {
    /// Adds a tab to the Archive and returns its id. `tab.id` is ignored: the store assigns it.
    pub async fn push_archive(&mut self, tab: &ArchivedTab) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO archive(archived_at, last_active, space_id, container_id, url, title) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    tab.archived_at_ms,
                    tab.last_active_ms,
                    tab.space.map(|s| s.0),
                    tab.container.0,
                    tab.url.as_str(),
                    tab.title.as_str(),
                ),
            )
            .await?;
        Ok(self.conn.last_insert_rowid())
    }

    /// One page: entries strictly before the `(archived_at, id)` cursor of the last row shown
    /// (`None`: from the newest), newest first, at most `limit`. `query` keeps the entries
    /// whose title or url contains it, in any case (empty keeps all).
    pub async fn list_archive(
        &self,
        limit: usize,
        before: Option<(i64, i64)>,
        query: &str,
    ) -> Result<Vec<ArchivedTab>> {
        let needle = query.trim().to_lowercase();
        if needle.is_ascii() {
            return self.archive_matching(limit, before, &needle).await;
        }
        // SQLite's `lower()` folds ASCII only: a query with other letters is matched here.
        let mut out = vec![];
        let mut cursor = before;
        loop {
            let chunk = self.archive_matching(SCAN_CHUNK, cursor, "").await?;
            let last = chunk.last().map(|t| (t.archived_at_ms, t.id));
            let read = chunk.len();
            for t in chunk {
                if contains_folded(&t.title, &needle) || contains_folded(&t.url, &needle) {
                    out.push(t);
                    if out.len() == limit {
                        return Ok(out);
                    }
                }
            }
            if read < SCAN_CHUNK {
                return Ok(out);
            }
            cursor = last;
        }
    }

    /// [`Self::list_archive`] where the SQL does the matching: `needle` is lowercase ASCII.
    async fn archive_matching(
        &self,
        limit: usize,
        before: Option<(i64, i64)>,
        needle: &str,
    ) -> Result<Vec<ArchivedTab>> {
        let (at, id) = before.unwrap_or((i64::MAX, i64::MAX));
        let mut rows = self
            .conn
            .query(
                format!(
                    "SELECT {COLUMNS} FROM archive \
                     WHERE (archived_at < ?1 OR (archived_at = ?1 AND id < ?2)) \
                       AND (?4 = '' OR instr(lower(title), ?4) > 0 OR instr(lower(url), ?4) > 0) \
                     ORDER BY archived_at DESC, id DESC LIMIT ?3"
                ),
                (at, id, limit as i64, needle.to_owned()),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            out.push(read(&r)?);
        }
        Ok(out)
    }

    /// Removes and returns the entry (restore). `None` when it is already gone.
    pub async fn take_archive(&mut self, id: i64) -> Result<Option<ArchivedTab>> {
        let tx = self.conn.transaction().await?;
        let mut rows = tx
            .query(
                format!("SELECT {COLUMNS} FROM archive WHERE id = ?1"),
                (id,),
            )
            .await?;
        let Some(r) = rows.next().await? else {
            return Ok(None);
        };
        let tab = read(&r)?;
        drop(rows);
        tx.execute("DELETE FROM archive WHERE id = ?1", (id,))
            .await?;
        tx.commit().await?;
        Ok(Some(tab))
    }

    pub async fn delete_archive(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM archive WHERE id = ?1", (id,))
            .await?;
        Ok(())
    }

    /// Removes every entry.
    pub async fn clear_archive(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM archive", ()).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(i: i64, at: i64) -> ArchivedTab {
        ArchivedTab {
            id: 0,
            url: format!("https://x.test/{i}"),
            title: format!("Title {i}"),
            container: ContainerId(i as u32 % 3),
            space: (i % 2 == 0).then_some(SpaceId(i as u32)),
            last_active_ms: 100 + i,
            archived_at_ms: at,
        }
    }

    fn urls(v: &[ArchivedTab]) -> Vec<&str> {
        v.iter().map(|t| t.url.as_str()).collect()
    }

    #[test]
    fn push_lists_newest_first_and_ties_keep_insertion_order_reversed() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for (i, at) in [(0, 10), (1, 30), (2, 20), (3, 30)] {
                s.push_archive(&tab(i, at)).await.unwrap();
            }
            let all = s.list_archive(10, None, "").await.unwrap();
            assert_eq!(
                urls(&all),
                [
                    "https://x.test/3",
                    "https://x.test/1",
                    "https://x.test/2",
                    "https://x.test/0"
                ]
            );
            let mut want = tab(3, 30);
            want.id = all[0].id;
            assert_eq!(all[0], want);
        });
    }

    #[test]
    fn search_ignores_case_in_any_script() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for (i, title) in ["Привет мир", "Plain"].into_iter().enumerate() {
                let mut t = tab(i as i64, 10 + i as i64);
                t.title = title.into();
                s.push_archive(&t).await.unwrap();
            }
            for query in ["привет", "ПРИВЕТ", "Привет"] {
                let hits = s.list_archive(10, None, query).await.unwrap();
                assert_eq!(urls(&hits), ["https://x.test/0"], "query {query:?}");
            }
        });
    }

    #[test]
    fn search_and_keyset_paging() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for i in 0..5 {
                s.push_archive(&tab(i, 10 + i)).await.unwrap();
            }
            let first = s.list_archive(2, None, "").await.unwrap();
            assert_eq!(urls(&first), ["https://x.test/4", "https://x.test/3"]);
            let last = first.last().unwrap();
            let second = s
                .list_archive(2, Some((last.archived_at_ms, last.id)), "")
                .await
                .unwrap();
            assert_eq!(urls(&second), ["https://x.test/2", "https://x.test/1"]);
            let hits = s.list_archive(10, None, " TITLE 3 ").await.unwrap();
            assert_eq!(urls(&hits), ["https://x.test/3"]);
            let hits = s.list_archive(10, None, "X.TEST/1").await.unwrap();
            assert_eq!(urls(&hits), ["https://x.test/1"]);
            assert!(
                s.list_archive(10, None, "nothing")
                    .await
                    .unwrap()
                    .is_empty()
            );
        });
    }

    #[test]
    fn take_returns_and_removes_delete_removes() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for i in 0..3 {
                s.push_archive(&tab(i, 10 + i)).await.unwrap();
            }
            let all = s.list_archive(10, None, "").await.unwrap();
            let got = s.take_archive(all[1].id).await.unwrap().unwrap();
            assert_eq!(got, all[1]);
            assert_eq!(s.take_archive(all[1].id).await.unwrap(), None);
            s.delete_archive(all[0].id).await.unwrap();
            let left = s.list_archive(10, None, "").await.unwrap();
            assert_eq!(left, vec![all[2].clone()]);
            s.clear_archive().await.unwrap();
            assert!(s.list_archive(10, None, "").await.unwrap().is_empty());
        });
    }

    #[test]
    fn entries_survive_their_space_being_deleted() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.push_archive(&tab(0, 5)).await.unwrap(); // space 0
            s.conn.execute("DELETE FROM spaces", ()).await.unwrap();
            assert_eq!(s.list_archive(10, None, "").await.unwrap().len(), 1);
        });
    }
}
