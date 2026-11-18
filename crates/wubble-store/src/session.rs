//! Session persistence: the store loads and saves `wubble_core::model::Model` directly.
//!
//! Not stored, because the core model has no counterpart: scroll, busy, folder collapse, openers
//! and the id counters. Tabs come back `Discarded` (no renderer survives a restart), idle, with
//! pin state taken from placement. Counters are derived by `Model::from_parts`.
//!
//! `Tab::container` is the effective container fixed at creation (Space default or per-tab
//! override), so persisting it covers the override.
//!
//! Position scheme: `spaces.position`, `containers.position` index the model vectors;
//! `tabs.position` indexes the tab's list (pinned, top-level items, or folder tabs);
//! top-level tabs and folders share one position sequence per Space (their index in `items`).

use std::collections::HashMap;

use wubble_core::lifecycle::{Activity, Lifecycle, Pin};
use wubble_core::model::Model;
use wubble_core::space::{Container, ContainerId, Folder, FolderId, Item, Rgb, Space, SpaceId};
use wubble_core::tab::{Liveness, Tab, TabId};

use crate::*;

fn tint_text(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// `#rrggbb`; anything else is `None`.
fn parse_tint(s: &str) -> Option<Rgb> {
    let hex = s
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.is_ascii())?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
}

fn tab_id(t: TabId) -> Result<i64> {
    i64::try_from(t.0).map_err(|_| Error::Corrupt(format!("tab id too large: {}", t.0)))
}

impl Store {
    /// The saved session; `Model::new()` if none was ever saved.
    pub async fn load_session(&self) -> Result<Model> {
        let c = &self.conn;
        let mut containers = vec![];
        let mut rows = c
            .query("SELECT id, name FROM containers ORDER BY position, id", ())
            .await?;
        while let Some(r) = rows.next().await? {
            containers.push(Container {
                id: ContainerId(narrow(r.get(0)?, "container id")?),
                name: r.get(1)?,
            });
        }

        let default_tint = Model::new().spaces[0].tint;
        let mut spaces: Vec<Space> = vec![];
        let mut space_at: HashMap<SpaceId, usize> = HashMap::new();
        let mut rows = c
            .query(
                "SELECT id, name, tint, container_id, active_tab_id FROM spaces ORDER BY position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            let id = SpaceId(narrow(r.get(0)?, "space id")?);
            let tint: Option<String> = r.get(2)?;
            let active: Option<i64> = r.get(4)?;
            space_at.insert(id, spaces.len());
            spaces.push(Space {
                id,
                name: r.get(1)?,
                tint: tint.as_deref().and_then(parse_tint).unwrap_or(default_tint),
                container: ContainerId(narrow(r.get(3)?, "container id")?),
                pinned: vec![],
                items: vec![],
                active: active
                    .map(|v| narrow(v, "tab id").map(|v: u64| TabId(v)))
                    .transpose()?,
            });
        }
        if spaces.is_empty() {
            return Ok(Model::new());
        }

        // Top-level items per Space with their shared position, merged after both are read.
        let mut top: Vec<Vec<(i64, Item)>> = vec![vec![]; spaces.len()];
        let mut folder_at: HashMap<FolderId, (usize, usize)> = HashMap::new();
        let mut rows = c
            .query(
                "SELECT id, space_id, name, position FROM tab_folders ORDER BY space_id, position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            let id = FolderId(narrow(r.get(0)?, "folder id")?);
            let space = SpaceId(narrow(r.get(1)?, "space id")?);
            let si = *space_at
                .get(&space)
                .ok_or_else(|| Error::Corrupt(format!("folder in unknown space {space:?}")))?;
            folder_at.insert(id, (si, top[si].len()));
            top[si].push((
                r.get(3)?,
                Item::Folder(Folder {
                    id,
                    name: r.get(2)?,
                    tabs: vec![],
                }),
            ));
        }

        let mut tabs = vec![];
        let mut rows = c
            .query(
                "SELECT id, space_id, container_id, folder_id, pinned, position, url, title \
                 FROM tabs ORDER BY space_id, pinned DESC, position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            let id = TabId(narrow(r.get(0)?, "tab id")?);
            let space = SpaceId(narrow(r.get(1)?, "space id")?);
            let si = *space_at
                .get(&space)
                .ok_or_else(|| Error::Corrupt(format!("tab in unknown space {space:?}")))?;
            let folder: Option<i64> = r.get(3)?;
            let pinned = r.get::<i64>(4)? != 0;
            match (pinned, folder) {
                (true, None) => spaces[si].pinned.push(id),
                (false, None) => top[si].push((r.get(5)?, Item::Tab(id))),
                (false, Some(f)) => {
                    let f = FolderId(narrow(f, "folder id")?);
                    let (fsi, at) = *folder_at
                        .get(&f)
                        .filter(|(fsi, _)| *fsi == si)
                        .ok_or_else(|| Error::Corrupt(format!("tab {id:?} in bad folder {f:?}")))?;
                    let Item::Folder(folder) = &mut top[fsi][at].1 else {
                        unreachable!("folder_at points at folders");
                    };
                    folder.tabs.push(id);
                }
                (true, Some(_)) => {
                    return Err(Error::Corrupt(format!("pinned tab {id:?} in folder")));
                }
            }
            tabs.push(Tab {
                id,
                url: r.get(6)?,
                title: r.get(7)?,
                lifecycle: Lifecycle {
                    liveness: Liveness::Discarded,
                    pin: if pinned { Pin::Pinned } else { Pin::Unpinned },
                    activity: Activity::Idle,
                    hidden_since: None,
                },
                container: ContainerId(narrow(r.get(2)?, "container id")?),
                opener: None,
            });
        }
        for (space, mut items) in spaces.iter_mut().zip(top) {
            items.sort_by_key(|(p, _)| *p); // stable: ties keep read order
            space.items = items.into_iter().map(|(_, i)| i).collect();
        }

        let mut rows = c
            .query("SELECT active_space_id FROM session WHERE id = 1", ())
            .await?;
        let active: Option<i64> = match rows.next().await? {
            Some(r) => r.get(0)?,
            None => None,
        };
        let active_space = SpaceId(narrow(
            active.ok_or_else(|| Error::Corrupt("no active space".into()))?,
            "space id",
        )?);
        Model::from_parts(spaces, containers, tabs, active_space).map_err(Error::Corrupt)
    }

    /// Replace the stored session with `model`, in one transaction. A model that fails
    /// `Model::check` is refused and the stored session stays as it was.
    ///
    /// Written as delete-all plus the row writers below; a later diff-based save replaces the
    /// deletes with per-table diffs by id and reuses the same writers.
    pub async fn save_session(&mut self, model: &Model) -> Result<()> {
        model.check().map_err(Error::Corrupt)?;
        for id in model.tabs.keys() {
            tab_id(*id)?;
        }
        let tx = self.conn.transaction().await?;
        // Children first; foreign keys are enforced per statement.
        for table in ["tabs", "tab_folders", "spaces", "containers", "session"] {
            tx.execute(format!("DELETE FROM {table}"), ()).await?;
        }
        write_containers(&tx, model).await?;
        write_spaces(&tx, model).await?;
        write_folders(&tx, model).await?;
        write_tabs(&tx, model).await?;
        tx.execute(
            "INSERT INTO session(id, active_space_id) VALUES (1, ?1)",
            (model.active_space.0,),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn write_containers(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare("INSERT INTO containers(id, name, position) VALUES (?1, ?2, ?3)")
        .await?;
    for (i, c) in m.containers.iter().enumerate() {
        st.execute((c.id.0, c.name.as_str(), i as i64)).await?;
    }
    Ok(())
}

async fn write_spaces(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare(
            "INSERT INTO spaces(id, name, position, tint, container_id, active_tab_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .await?;
    for (i, s) in m.spaces.iter().enumerate() {
        let active = s.active.map(tab_id).transpose()?;
        st.execute((
            s.id.0,
            s.name.as_str(),
            i as i64,
            tint_text(s.tint),
            s.container.0,
            active,
        ))
        .await?;
    }
    Ok(())
}

async fn write_folders(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare("INSERT INTO tab_folders(id, space_id, name, position) VALUES (?1, ?2, ?3, ?4)")
        .await?;
    for s in &m.spaces {
        for (i, item) in s.items.iter().enumerate() {
            if let Item::Folder(f) = item {
                st.execute((f.id.0, s.id.0, f.name.as_str(), i as i64))
                    .await?;
            }
        }
    }
    Ok(())
}

async fn write_tabs(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare(
            "INSERT INTO tabs(id, space_id, container_id, folder_id, pinned, position, url, title) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .await?;
    let mut put = async |t: TabId, space: SpaceId, folder: Option<FolderId>, pinned, pos: usize| {
        let tab = &m.tabs[&t]; // present: `check` ran
        st.execute((
            tab_id(t)?,
            space.0,
            tab.container.0,
            folder.map(|f| f.0),
            i64::from(pinned),
            pos as i64,
            tab.url.as_str(),
            tab.title.as_str(),
        ))
        .await?;
        Ok::<(), Error>(())
    };
    for s in &m.spaces {
        for (i, &t) in s.pinned.iter().enumerate() {
            put(t, s.id, None, true, i).await?;
        }
        for (i, item) in s.items.iter().enumerate() {
            match item {
                Item::Tab(t) => put(*t, s.id, None, false, i).await?,
                Item::Folder(f) => {
                    for (j, &t) in f.tabs.iter().enumerate() {
                        put(t, s.id, Some(f.id), false, j).await?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use wubble_core::model::{Command, Dest, apply};

    use super::*;
    use crate::migrate::{self, MIGRATIONS};

    const D: ContainerId = ContainerId::DEFAULT;

    fn run<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    fn new_tab(
        m: Model,
        url: &str,
        container: Option<ContainerId>,
        opener: Option<TabId>,
    ) -> Model {
        let cmd = Command::NewTab {
            url: url.into(),
            container,
            opener,
        };
        apply(m, cmd).0
    }

    /// Two Spaces sharing a container, an empty third, pinned and foldered tabs, a per-tab
    /// container override, per-Space active tabs, and the second Space shown.
    fn rich() -> Model {
        let ap = |m, c| apply(m, c).0;
        let mut m = Model::new();
        m = ap(
            m,
            Command::NewContainer {
                name: "Work".into(),
            },
        );
        let work = ContainerId(1);
        for i in 1..=4u64 {
            let opener = (i == 2).then_some(TabId(1));
            let container = (i == 4).then_some(work); // override of the Space default
            m = new_tab(
                m,
                &format!("https://a.test/{i}?q=\"x\",ü"),
                container,
                opener,
            );
        }
        m = ap(m, Command::PinTab(TabId(1)));
        m = ap(
            m,
            Command::NewFolder {
                space: SpaceId(0),
                name: "Reading".into(),
            },
        );
        m = new_tab(m, "https://a.test/5", None, None); // after the folder
        m = ap(
            m,
            Command::NewFolder {
                space: SpaceId(0),
                name: "Empty".into(),
            },
        );
        for (i, t) in [3u64, 4].into_iter().enumerate() {
            m = ap(
                m,
                Command::MoveTab {
                    tab: TabId(t),
                    to: Dest::Folder {
                        folder: FolderId(0),
                        index: i,
                    },
                },
            );
        }
        for name in ["B", "C"] {
            m = ap(
                m,
                Command::NewSpace {
                    name: name.into(),
                    tint: Rgb(1, 2, 3),
                    container: work,
                },
            );
        }
        m = ap(m, Command::SwitchSpace(SpaceId(1)));
        m = new_tab(m, "https://b.test/1", Some(D), None);
        m = new_tab(m, "https://b.test/2", None, None);
        m = ap(m, Command::ActivateTab(TabId(2))); // Space A remembers tab 2
        m = ap(m, Command::SwitchSpace(SpaceId(1)));
        m.check().unwrap();
        m
    }

    /// What loading `m` back must give: tabs Discarded, idle, pin from placement, no opener.
    fn restored(m: &Model) -> Model {
        let tabs = m
            .tabs
            .values()
            .map(|t| Tab {
                lifecycle: Lifecycle {
                    liveness: Liveness::Discarded,
                    pin: if m.is_pinned(t.id) {
                        Pin::Pinned
                    } else {
                        Pin::Unpinned
                    },
                    activity: Activity::Idle,
                    hidden_since: None,
                },
                opener: None,
                ..t.clone()
            })
            .collect();
        Model::from_parts(m.spaces.clone(), m.containers.clone(), tabs, m.active_space).unwrap()
    }

    #[test]
    fn empty_store_loads_a_new_model() {
        run(async {
            let store = Store::open_in_memory().await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), Model::new());
        });
    }

    #[test]
    fn rich_model_round_trips() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let m = rich();
            // The fixture covers what the test claims.
            assert_eq!(m.active_space, SpaceId(1));
            assert_eq!(m.spaces[0].active, Some(TabId(2)));
            assert_eq!(m.spaces[0].pinned, vec![TabId(1)]);
            assert!(m.spaces[2].items.is_empty() && m.spaces[2].active.is_none());
            assert_eq!(m.spaces[1].container, m.spaces[2].container);
            assert_eq!(m.tabs[&TabId(4)].container, ContainerId(1));
            assert!(matches!(m.spaces[0].items[1], Item::Folder(_)));
            assert!(matches!(&m.spaces[0].items[3], Item::Folder(f) if f.tabs.is_empty()));
            store.save_session(&m).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), restored(&m));
        });
    }

    #[test]
    fn ids_continue_after_load() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&rich()).await.unwrap();
            let loaded = store.load_session().await.unwrap();
            let (m, fx) = apply(
                loaded,
                Command::NewTab {
                    url: "u".into(),
                    container: None,
                    opener: None,
                },
            );
            assert!(
                matches!(fx[0], wubble_core::model::Effect::CreateTab { tab, .. } if tab == TabId(8))
            );
            m.check().unwrap();
        });
    }

    #[test]
    fn save_replaces_previous_session() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&rich()).await.unwrap();
            let small = new_tab(Model::new(), "https://x.test", None, None);
            store.save_session(&small).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), restored(&small));
        });
    }

    #[test]
    fn invalid_model_is_refused_and_previous_kept() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let good = rich();
            store.save_session(&good).await.unwrap();
            let mut bad = good.clone();
            bad.active_space = SpaceId(99);
            assert!(matches!(
                store.save_session(&bad).await,
                Err(Error::Corrupt(_))
            ));
            assert_eq!(store.load_session().await.unwrap(), restored(&good));
        });
    }

    #[test]
    fn oversized_tab_id_is_rejected_at_save() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let m = new_tab(Model::new(), "https://x.test", None, None);
            let mut bad = m.clone();
            let mut tab = bad.tabs.remove(&TabId(1)).unwrap();
            tab.id = TabId(u64::MAX);
            bad.tabs.insert(tab.id, tab);
            bad.spaces[0].items = vec![Item::Tab(TabId(u64::MAX))];
            bad.spaces[0].active = Some(TabId(u64::MAX));
            bad.check().unwrap();
            store.save_session(&m).await.unwrap();
            assert!(matches!(
                store.save_session(&bad).await,
                Err(Error::Corrupt(_))
            ));
            assert_eq!(store.load_session().await.unwrap(), restored(&m));
        });
    }

    #[test]
    fn invalid_tint_loads_as_default() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&rich()).await.unwrap();
            store
                .conn
                .execute("UPDATE spaces SET tint = 'red' WHERE id = 1", ())
                .await
                .unwrap();
            store
                .conn
                .execute("UPDATE spaces SET tint = NULL WHERE id = 2", ())
                .await
                .unwrap();
            let m = store.load_session().await.unwrap();
            assert_eq!(m.spaces[1].tint, Model::new().spaces[0].tint);
            assert_eq!(m.spaces[2].tint, Model::new().spaces[0].tint);
        });
    }

    #[test]
    fn tint_text_round_trips() {
        assert_eq!(
            parse_tint(&tint_text(Rgb(0, 15, 255))),
            Some(Rgb(0, 15, 255))
        );
        for bad in ["", "#abc", "abcdef", "#gggggg", "#12345678", "#ééé"] {
            assert_eq!(parse_tint(bad), None, "{bad}");
        }
    }

    #[test]
    fn deleting_a_space_row_cascades_to_its_tabs() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&rich()).await.unwrap();
            store
                .conn
                .execute("DELETE FROM spaces WHERE id = 1", ())
                .await
                .unwrap();
            let mut rows = store
                .conn
                .query("SELECT COUNT(*) FROM tabs WHERE space_id = 1", ())
                .await
                .unwrap();
            assert_eq!(
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
                0
            );
        });
    }

    #[test]
    fn session_persists_across_reopen() {
        run(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested/wubble.db");
            let m = rich();
            {
                let mut store = Store::open(&path).await.unwrap();
                store.save_session(&m).await.unwrap();
            }
            let store = Store::open(&path).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), restored(&m));
        });
    }

    #[test]
    fn migrating_a_v1_database_keeps_its_data() {
        run(async {
            let mut store = Store::connect(":memory:").await.unwrap();
            migrate::migrate(&mut store.conn, &MIGRATIONS[..1])
                .await
                .unwrap();
            store
                .conn
                .execute_batch(
                    "INSERT INTO containers(id, name) VALUES (0, 'Default'), (1, 'Work');
                     INSERT INTO spaces(id, name, position, tint, container_id) VALUES
                        (0, 'Home', 0, '#aabbcc', 0), (1, 'Jobs', 1, NULL, 1);
                     INSERT INTO tab_folders(id, space_id, name, position, collapsed) VALUES
                        (0, 0, 'Reading', 1, 1);
                     INSERT INTO tabs(id, space_id, container_id, folder_id, pinned, position, url, title,
                                      liveness, busy, scroll_x, scroll_y, last_visible) VALUES
                        (10, 0, 0, NULL, 1, 0, 'https://p.test', 'Pinned', 0, 1, 1.5, 2.5, 99),
                        (11, 0, 0, NULL, 0, 0, 'https://n.test', 'Normal', 1, 0, 0, 0, NULL),
                        (12, 0, 0, 0,    0, 0, 'https://f.test', 'Foldered', 3, 0, 0, 0, NULL),
                        (20, 1, 1, NULL, 0, 1, 'https://j.test/2', 'Two', 2, 0, 0, 0, NULL),
                        (21, 1, 0, NULL, 0, 2, 'https://j.test/1', 'Override', 0, 0, 0, 0, NULL);
                     INSERT INTO session(id, active_space_id, active_tab_id, saved_at) VALUES (1, 1, 21, 5);",
                )
                .await
                .unwrap();
            migrate::migrate(&mut store.conn, MIGRATIONS).await.unwrap();
            assert_eq!(migrate::schema_version(&store.conn).await.unwrap(), 2);

            for dropped in [
                "SELECT liveness FROM tabs",
                "SELECT scroll_x FROM tabs",
                "SELECT collapsed FROM tab_folders",
                "SELECT saved_at FROM session",
                "SELECT active_tab_id FROM session",
            ] {
                assert!(store.conn.query(dropped, ()).await.is_err(), "{dropped}");
            }

            let tab = |id, url: &str, title: &str, container, pinned| Tab {
                id: TabId(id),
                url: url.into(),
                title: title.into(),
                lifecycle: Lifecycle {
                    liveness: Liveness::Discarded,
                    pin: if pinned { Pin::Pinned } else { Pin::Unpinned },
                    activity: Activity::Idle,
                    hidden_since: None,
                },
                container: ContainerId(container),
                opener: None,
            };
            let spaces = vec![
                Space {
                    id: SpaceId(0),
                    name: "Home".into(),
                    tint: Rgb(0xaa, 0xbb, 0xcc),
                    container: D,
                    pinned: vec![TabId(10)],
                    items: vec![
                        Item::Tab(TabId(11)),
                        Item::Folder(Folder {
                            id: FolderId(0),
                            name: "Reading".into(),
                            tabs: vec![TabId(12)],
                        }),
                    ],
                    active: Some(TabId(10)), // backfilled: first tab, pinned first
                },
                Space {
                    id: SpaceId(1),
                    name: "Jobs".into(),
                    tint: Model::new().spaces[0].tint,
                    container: ContainerId(1),
                    pinned: vec![],
                    items: vec![Item::Tab(TabId(20)), Item::Tab(TabId(21))],
                    active: Some(TabId(21)), // the saved active tab
                },
            ];
            let containers = vec![
                Container {
                    id: D,
                    name: "Default".into(),
                },
                Container {
                    id: ContainerId(1),
                    name: "Work".into(),
                },
            ];
            let tabs = vec![
                tab(10, "https://p.test", "Pinned", 0, true),
                tab(11, "https://n.test", "Normal", 0, false),
                tab(12, "https://f.test", "Foldered", 0, false),
                tab(20, "https://j.test/2", "Two", 1, false),
                tab(21, "https://j.test/1", "Override", 0, false),
            ];
            let expected = Model::from_parts(spaces, containers, tabs, SpaceId(1)).unwrap();
            assert_eq!(store.load_session().await.unwrap(), expected);
            // The migrated database also accepts a normal save.
            store.save_session(&expected).await.unwrap();
            assert_eq!(store.load_session().await.unwrap(), expected);
        });
    }
}
