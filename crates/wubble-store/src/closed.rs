//! Recently closed tabs, for reopen-closed-tab. A bounded list, newest first.
//!
//! Design reference, no code copied: Chromium `components/sessions/core/tab_restore_service_helper.cc`
//! (entries kept newest first, oldest pruned past a maximum), BSD-3-Clause.
//!
//! No foreign keys: the Space may be gone by the time the tab is reopened.

use wubble_core::space::{ContainerId, SpaceId};

use crate::*;

/// Most entries kept; older ones are dropped on push.
pub use wubble_core::closed::RECENTLY_CLOSED_MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub container: ContainerId,
    pub space: Option<SpaceId>,
    /// Unix ms, supplied by the caller.
    pub closed_at_ms: i64,
}

const COLUMNS: &str = "closed_at, space_id, container_id, url, title";

/// Reads the `COLUMNS` starting at column `at`.
fn read(r: &turso::Row, at: usize) -> Result<ClosedTab> {
    let space: Option<i64> = r.get(at + 1)?;
    Ok(ClosedTab {
        closed_at_ms: r.get(at)?,
        space: space
            .map(|v| narrow(v, "space id").map(SpaceId))
            .transpose()?,
        container: ContainerId(narrow(r.get(at + 2)?, "container id")?),
        url: r.get(at + 3)?,
        title: r.get(at + 4)?,
    })
}

impl Store {
    /// Record a closed tab as the newest entry and drop entries beyond [`RECENTLY_CLOSED_MAX`].
    pub async fn push_closed(&mut self, tab: &ClosedTab) -> Result<()> {
        let tx = self.conn.transaction().await?;
        tx.execute(
            format!("INSERT INTO recently_closed({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
            (
                tab.closed_at_ms,
                tab.space.map(|s| s.0),
                tab.container.0,
                tab.url.as_str(),
                tab.title.as_str(),
            ),
        )
        .await?;
        tx.execute(
            format!(
                "DELETE FROM recently_closed WHERE id NOT IN \
                 (SELECT id FROM recently_closed ORDER BY id DESC LIMIT {RECENTLY_CLOSED_MAX})"
            ),
            (),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Newest first.
    pub async fn recently_closed(&self) -> Result<Vec<ClosedTab>> {
        let mut rows = self
            .conn
            .query(
                format!("SELECT {COLUMNS} FROM recently_closed ORDER BY id DESC"),
                (),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            out.push(read(&r, 0)?);
        }
        Ok(out)
    }

    /// Remove and return the newest entry (reopen closed tab).
    pub async fn pop_closed(&mut self) -> Result<Option<ClosedTab>> {
        let tx = self.conn.transaction().await?;
        let mut rows = tx
            .query(
                format!("SELECT id, {COLUMNS} FROM recently_closed ORDER BY id DESC LIMIT 1"),
                (),
            )
            .await?;
        let Some(r) = rows.next().await? else {
            return Ok(None);
        };
        let id: i64 = r.get(0)?;
        let tab = read(&r, 1)?;
        drop(rows);
        tx.execute("DELETE FROM recently_closed WHERE id = ?1", (id,))
            .await?;
        tx.commit().await?;
        Ok(Some(tab))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(i: i64) -> ClosedTab {
        ClosedTab {
            url: format!("https://x.test/{i}"),
            title: format!("T{i}"),
            container: ContainerId(i as u32 % 3),
            space: (i % 2 == 0).then_some(SpaceId(i as u32)),
            closed_at_ms: 1_000 + i,
        }
    }

    #[test]
    fn bound_keeps_the_newest_in_order() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for i in 0..30 {
                s.push_closed(&closed(i)).await.unwrap();
            }
            let got = s.recently_closed().await.unwrap();
            assert_eq!(got.len(), RECENTLY_CLOSED_MAX);
            let want: Vec<_> = (5..30).rev().map(closed).collect();
            assert_eq!(got, want);
        });
    }

    #[test]
    fn pop_returns_newest_and_empty_pop_is_none() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            assert_eq!(s.pop_closed().await.unwrap(), None);
            for i in 0..3 {
                s.push_closed(&closed(i)).await.unwrap();
            }
            assert_eq!(s.pop_closed().await.unwrap(), Some(closed(2)));
            assert_eq!(
                s.recently_closed().await.unwrap(),
                vec![closed(1), closed(0)]
            );
            s.pop_closed().await.unwrap();
            s.pop_closed().await.unwrap();
            assert_eq!(s.pop_closed().await.unwrap(), None);
        });
    }

    #[test]
    fn entries_survive_their_space_being_deleted() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let mut m = wubble_core::model::Model::new(std::time::Instant::now());
            m.spaces[0].id = SpaceId(0);
            s.save_session(&m).await.unwrap();
            s.push_closed(&closed(0)).await.unwrap(); // space 0
            s.conn.execute("DELETE FROM tabs", ()).await.unwrap();
            s.conn.execute("DELETE FROM spaces", ()).await.unwrap();
            assert_eq!(s.recently_closed().await.unwrap(), vec![closed(0)]);
        });
    }
}
