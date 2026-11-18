//! Session snapshot: one transaction replaces everything, which is simple and always consistent.

use wubble_core::space::{ContainerId, SpaceId};
use wubble_core::tab::TabId;

use crate::model::{liveness_code, liveness_from_code};
use crate::*;

impl Store {
    /// The saved session; empty if none was ever saved.
    pub async fn load_session(&self) -> Result<StoredSession> {
        let c = &self.conn;
        let mut s = StoredSession::default();

        let mut rows = c
            .query("SELECT id, name FROM containers ORDER BY id", ())
            .await?;
        while let Some(r) = rows.next().await? {
            s.containers.push(StoredContainer {
                id: ContainerId(narrow(r.get(0)?, "container id")?),
                name: r.get(1)?,
            });
        }
        let mut rows = c
            .query(
                "SELECT id, name, tint, container_id FROM spaces ORDER BY position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            s.spaces.push(StoredSpace {
                id: SpaceId(narrow(r.get(0)?, "space id")?),
                name: r.get(1)?,
                tint: r.get(2)?,
                container: ContainerId(narrow(r.get(3)?, "container id")?),
            });
        }
        let mut rows = c
            .query(
                "SELECT id, space_id, name, collapsed FROM tab_folders ORDER BY space_id, position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            s.folders.push(StoredFolder {
                id: FolderId(narrow(r.get(0)?, "folder id")?),
                space: SpaceId(narrow(r.get(1)?, "space id")?),
                name: r.get(2)?,
                collapsed: r.get::<i64>(3)? != 0,
            });
        }
        let mut rows = c
            .query(
                "SELECT id, space_id, container_id, folder_id, pinned, url, title, liveness, busy, \
                 scroll_x, scroll_y, last_visible FROM tabs ORDER BY position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            let code: i64 = r.get(7)?;
            let folder: Option<i64> = r.get(3)?;
            s.tabs.push(StoredTab {
                id: TabId(narrow(r.get(0)?, "tab id")?),
                space: SpaceId(narrow(r.get(1)?, "space id")?),
                container: ContainerId(narrow(r.get(2)?, "container id")?),
                folder: folder
                    .map(|f| narrow(f, "folder id").map(FolderId))
                    .transpose()?,
                pinned: r.get::<i64>(4)? != 0,
                url: r.get(5)?,
                title: r.get(6)?,
                liveness: liveness_from_code(code)
                    .ok_or_else(|| Error::Corrupt(format!("liveness code {code}")))?,
                busy: r.get::<i64>(8)? != 0,
                scroll: (r.get(9)?, r.get(10)?),
                last_visible_ms: r.get(11)?,
            });
        }
        let mut rows = c
            .query(
                "SELECT active_space_id, active_tab_id, saved_at FROM session WHERE id = 1",
                (),
            )
            .await?;
        if let Some(r) = rows.next().await? {
            let space: Option<i64> = r.get(0)?;
            let tab: Option<i64> = r.get(1)?;
            s.active_space = space
                .map(|v| narrow(v, "space id").map(SpaceId))
                .transpose()?;
            s.active_tab = tab.map(|v| narrow(v, "tab id").map(TabId)).transpose()?;
            s.saved_at_ms = r.get(2)?;
        }
        Ok(s)
    }

    /// Replace the stored session with `s`. Parents before children is not required; ids must be
    /// unique and every referenced container/space must be present or the save fails and rolls back.
    pub async fn save_session(&mut self, s: &StoredSession) -> Result<()> {
        // Validate before touching the database: ids are stored as i64, scroll must be finite.
        let id = |v: u64, what| {
            i64::try_from(v).map_err(|_| Error::Corrupt(format!("{what} too large: {v}")))
        };
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        let tab_ids = s
            .tabs
            .iter()
            .map(|t| id(t.id.0, "tab id"))
            .collect::<Result<Vec<_>>>()?;
        let active_tab = s.active_tab.map(|t| id(t.0, "tab id")).transpose()?;
        let tx = self.conn.transaction().await?;
        // Children first; foreign keys are enforced per statement.
        for table in ["tabs", "tab_folders", "spaces", "containers", "session"] {
            tx.execute(format!("DELETE FROM {table}"), ()).await?;
        }
        let mut st = tx
            .prepare("INSERT INTO containers(id, name) VALUES (?1, ?2)")
            .await?;
        for c in &s.containers {
            st.execute((c.id.0, c.name.as_str())).await?;
        }
        let mut st = tx
            .prepare("INSERT INTO spaces(id, name, position, tint, container_id) VALUES (?1, ?2, ?3, ?4, ?5)")
            .await?;
        for (i, sp) in s.spaces.iter().enumerate() {
            st.execute((
                sp.id.0,
                sp.name.as_str(),
                i as i64,
                sp.tint.clone(),
                sp.container.0,
            ))
            .await?;
        }
        let mut st = tx
            .prepare("INSERT INTO tab_folders(id, space_id, name, position, collapsed) VALUES (?1, ?2, ?3, ?4, ?5)")
            .await?;
        for (i, f) in s.folders.iter().enumerate() {
            st.execute((
                f.id.0,
                f.space.0,
                f.name.as_str(),
                i as i64,
                f.collapsed as i64,
            ))
            .await?;
        }
        let mut st = tx
            .prepare(
                "INSERT INTO tabs(id, space_id, container_id, folder_id, pinned, position, url, title, \
                 liveness, busy, scroll_x, scroll_y, last_visible) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            )
            .await?;
        for (i, t) in s.tabs.iter().enumerate() {
            st.execute((
                tab_ids[i],
                t.space.0,
                t.container.0,
                t.folder.map(|f| f.0),
                t.pinned as i64,
                i as i64,
                t.url.as_str(),
                t.title.as_str(),
                liveness_code(t.liveness),
                t.busy as i64,
                finite(t.scroll.0),
                finite(t.scroll.1),
                t.last_visible_ms,
            ))
            .await?;
        }
        tx.execute(
            "INSERT INTO session(id, active_space_id, active_tab_id, saved_at) VALUES (1, ?1, ?2, ?3)",
            (s.active_space.map(|v| v.0), active_tab, s.saved_at_ms),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use wubble_core::tab::Liveness;

    use super::*;

    fn sample() -> StoredSession {
        let tab = |id, space, folder, pinned, liveness| StoredTab {
            id: TabId(id),
            space: SpaceId(space),
            container: ContainerId(1),
            folder,
            pinned,
            url: format!("https://example.com/{id}"),
            title: format!("Tab {id}"),
            liveness,
            busy: id == 2,
            scroll: (1.5, 480.25),
            last_visible_ms: if id == 4 {
                None
            } else {
                Some(1_000 + id as i64)
            },
        };
        StoredSession {
            containers: vec![
                StoredContainer {
                    id: ContainerId(1),
                    name: "Default".into(),
                },
                StoredContainer {
                    id: ContainerId(2),
                    name: "Work".into(),
                },
            ],
            spaces: vec![
                StoredSpace {
                    id: SpaceId(7),
                    name: "Home".into(),
                    tint: Some("#aabbcc".into()),
                    container: ContainerId(1),
                },
                StoredSpace {
                    id: SpaceId(3),
                    name: "Work".into(),
                    tint: None,
                    container: ContainerId(2),
                },
            ],
            folders: vec![StoredFolder {
                id: FolderId(1),
                space: SpaceId(7),
                name: "Reading".into(),
                collapsed: true,
            }],
            tabs: vec![
                tab(10, 7, None, true, Liveness::Active),
                tab(2, 7, Some(FolderId(1)), false, Liveness::Background),
                tab(5, 3, None, false, Liveness::Frozen),
                tab(4, 3, None, false, Liveness::Discarded),
            ],
            active_space: Some(SpaceId(7)),
            active_tab: Some(TabId(2)),
            saved_at_ms: 123_456,
        }
    }

    #[test]
    fn empty_store_loads_empty_session() {
        pollster::block_on(async {
            let store = Store::open_in_memory().await.unwrap();
            assert_eq!(
                store.load_session().await.unwrap(),
                StoredSession::default()
            );
        });
    }

    #[test]
    fn round_trip_keeps_order_and_fields() {
        pollster::block_on(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let s = sample();
            store.save_session(&s).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), s);
        });
    }

    #[test]
    fn save_replaces_previous_session() {
        pollster::block_on(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&sample()).await.unwrap();
            let mut smaller = sample();
            smaller.tabs.truncate(1);
            smaller.folders.clear();
            smaller.tabs[0].folder = None;
            smaller.active_tab = None;
            store.save_session(&smaller).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), smaller);
        });
    }

    #[test]
    fn failed_save_keeps_previous_session() {
        pollster::block_on(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let good = sample();
            store.save_session(&good).await.unwrap();
            let mut bad = sample();
            bad.tabs[0].space = SpaceId(999); // no such space
            assert!(store.save_session(&bad).await.is_err());
            assert_eq!(store.load_session().await.unwrap(), good);
        });
    }

    #[test]
    fn non_finite_scroll_is_saved_as_zero() {
        pollster::block_on(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let mut s = sample();
            s.tabs[0].scroll = (f64::NAN, f64::INFINITY);
            store.save_session(&s).await.unwrap();
            assert_eq!(
                store.load_session().await.unwrap().tabs[0].scroll,
                (0.0, 0.0)
            );
        });
    }

    #[test]
    fn oversized_tab_id_is_rejected_at_save() {
        pollster::block_on(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let good = sample();
            store.save_session(&good).await.unwrap();
            let mut bad = sample();
            bad.tabs[0].id = TabId(u64::MAX);
            assert!(matches!(
                store.save_session(&bad).await,
                Err(Error::Corrupt(_))
            ));
            assert_eq!(store.load_session().await.unwrap(), good);
        });
    }

    #[test]
    fn deleting_a_space_cascades_to_its_tabs() {
        pollster::block_on(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&sample()).await.unwrap();
            store
                .conn
                .execute("DELETE FROM spaces WHERE id = 3", ())
                .await
                .unwrap();
            let s = store.load_session().await.unwrap();
            assert_eq!(s.spaces.len(), 1);
            assert!(s.tabs.iter().all(|t| t.space == SpaceId(7)));
        });
    }

    #[test]
    fn session_persists_across_reopen() {
        pollster::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested/wubble.db");
            let s = sample();
            {
                let mut store = Store::open(&path).await.unwrap();
                store.save_session(&s).await.unwrap();
            }
            let store = Store::open(&path).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), s);
        });
    }
}
