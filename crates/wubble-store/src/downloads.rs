//! The downloads list (`wubble_core::download::DownloadEntry`), so it
//! survives a restart. Ids are the host's; the newest entry has the highest.
//! What a restart does to unfinished entries is the core's rule
//! (`download::lost_all`), applied by the host when it loads them.

use std::path::PathBuf;

use wubble_core::download::{DownloadEntry, DownloadState};
use wubble_core::space::ContainerId;

use crate::*;

const COLUMNS: &str = "id, url, name, path, total, received, state, error, started_at";

fn read(r: &turso::Row) -> Result<DownloadEntry> {
    let state: String = r.get(6)?;
    let error: Option<String> = r.get(7)?;
    let total: Option<i64> = r.get(4)?;
    let path: String = r.get(3)?;
    Ok(DownloadEntry {
        id: r.get(0)?,
        url: r.get(1)?,
        name: r.get(2)?,
        path: PathBuf::from(path),
        total: total.map(|t| narrow(t, "download size")).transpose()?,
        received: narrow(r.get(5)?, "download bytes")?,
        state: DownloadState::parse(&state, error)
            .ok_or_else(|| Error::Corrupt(format!("download state {state:?}")))?,
        started_ms: r.get(8)?,
        container: ContainerId::DEFAULT,
    })
}

/// Bytes as SQLite's integer (no file is 2^63 bytes).
fn bytes(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Store {
    /// Adds `entry` under its own id (replacing a row with that id).
    pub async fn add_download(&mut self, entry: &DownloadEntry) -> Result<()> {
        let tx = self.conn.transaction().await?;
        tx.execute("DELETE FROM downloads WHERE id = ?1", (entry.id,))
            .await?;
        tx.execute(
            format!("INSERT INTO downloads({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
            (
                entry.id,
                entry.url.as_str(),
                entry.name.as_str(),
                entry.path.to_string_lossy().as_ref(),
                entry.total.map(bytes),
                bytes(entry.received),
                entry.state.as_str(),
                entry.state.error(),
                entry.started_ms,
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Writes `entry`'s progress and state.
    pub async fn update_download(&mut self, entry: &DownloadEntry) -> Result<()> {
        self.conn
            .execute(
                "UPDATE downloads SET total = ?2, received = ?3, state = ?4, error = ?5 \
                 WHERE id = ?1",
                (
                    entry.id,
                    entry.total.map(bytes),
                    bytes(entry.received),
                    entry.state.as_str(),
                    entry.state.error(),
                ),
            )
            .await?;
        Ok(())
    }

    /// The newest `limit` entries, newest first. A row this build cannot
    /// read (a state a newer build added, a size out of range) is skipped,
    /// like an unknown permission kind: one row must not cost the session.
    pub async fn downloads(&self, limit: usize) -> Result<Vec<DownloadEntry>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut rows = self
            .conn
            .query(
                format!("SELECT {COLUMNS} FROM downloads ORDER BY id DESC LIMIT ?1"),
                (limit,),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            match read(&r) {
                Ok(entry) => out.push(entry),
                Err(Error::Corrupt(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    pub async fn remove_download(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM downloads WHERE id = ?1", (id,))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::download::{LOST, lost_all};

    fn entry(id: i64, state: DownloadState) -> DownloadEntry {
        DownloadEntry {
            id,
            url: format!("https://a.test/{id}.zip"),
            name: format!("{id}.zip"),
            path: format!("/d/{id}.zip").into(),
            received: 10,
            total: (id % 2 == 0).then_some(100),
            state,
            started_ms: 1_000 + id,
            container: ContainerId::DEFAULT,
        }
    }

    #[test]
    fn add_update_list_remove() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = entry(1, DownloadState::Running);
            let b = entry(2, DownloadState::Interrupted("Network error".into()));
            s.add_download(&a).await.unwrap();
            s.add_download(&b).await.unwrap();
            assert_eq!(s.downloads(10).await.unwrap(), vec![b.clone(), a.clone()]);
            assert_eq!(s.downloads(1).await.unwrap(), vec![b.clone()]);
            let done = DownloadEntry {
                received: 5_000_000_000,
                total: Some(5_000_000_000),
                state: DownloadState::Complete,
                ..a.clone()
            };
            s.update_download(&done).await.unwrap();
            assert_eq!(s.downloads(10).await.unwrap(), vec![b.clone(), done]);
            s.remove_download(2).await.unwrap();
            assert_eq!(s.downloads(10).await.unwrap().len(), 1);
            // Adding under an existing id replaces the row.
            s.add_download(&a).await.unwrap();
            assert_eq!(s.downloads(10).await.unwrap(), vec![a]);
        });
    }

    #[test]
    fn a_restart_fails_what_was_unfinished() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let states = [
                DownloadState::Running,
                DownloadState::Paused,
                DownloadState::Interrupted("x".into()),
                DownloadState::Complete,
                DownloadState::Cancelled,
                DownloadState::Failed("y".into()),
            ];
            for (i, state) in states.iter().enumerate() {
                s.add_download(&entry(i as i64 + 1, state.clone()))
                    .await
                    .unwrap();
            }
            for e in lost_all(s.downloads(10).await.unwrap()) {
                s.update_download(&e).await.unwrap();
            }
            let got: Vec<DownloadState> = s
                .downloads(10)
                .await
                .unwrap()
                .into_iter()
                .rev()
                .map(|e| e.state)
                .collect();
            let lost = || DownloadState::Failed(LOST.into());
            assert_eq!(
                got,
                vec![
                    lost(),
                    lost(),
                    lost(),
                    DownloadState::Complete,
                    DownloadState::Cancelled,
                    DownloadState::Failed("y".into()),
                ]
            );
        });
    }

    #[test]
    fn unreadable_rows_are_skipped() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for id in 1..=3 {
                s.add_download(&entry(id, DownloadState::Complete))
                    .await
                    .unwrap();
            }
            s.conn
                .execute("UPDATE downloads SET state = 'teleported' WHERE id = 1", ())
                .await
                .unwrap();
            s.conn
                .execute("UPDATE downloads SET received = -1 WHERE id = 2", ())
                .await
                .unwrap();
            let ids: Vec<i64> = s
                .downloads(10)
                .await
                .unwrap()
                .iter()
                .map(|e| e.id)
                .collect();
            assert_eq!(ids, [3]);
        });
    }
}
