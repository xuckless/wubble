//! Session persistence: the store loads and saves `wubble_core::model::Model` directly.
//!
//! Not stored, because the core model has no counterpart: scroll, busy, folder collapse, openers
//! and the id counters. Tabs come back `Discarded` (no renderer survives a restart; the shown tab is `Active`), idle, with
//! pin state taken from placement. Counters are derived by `Model::from_parts`.
//!
//! `Tab::container` is the effective container fixed at creation (Space default or per-tab
//! override), so persisting it covers the override.
//!
//! Position scheme: `spaces.position`, `containers.position` index the model vectors;
//! `tabs.position` indexes the tab's list (pinned, top-level items, or folder tabs);
//! top-level tabs and folders share one position sequence per Space (their index in `items`).

use std::collections::HashMap;
use std::time::Instant;

use wubble_core::lifecycle::{Activity, Keep, Lifecycle, Pin};
use wubble_core::model::Model;
use wubble_core::space::{
    Container, ContainerId, Folder, FolderId, Item, MAX_PANES, Rgb, Space, SpaceIcon, SpaceId,
    SpaceKind, SplitGroup, Storage,
};
use wubble_core::tab::{Liveness, Tab, TabId};
use wubble_core::window::{Geometry, Window, WindowId};

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
    /// The saved session; `Model::new(now)` if none was ever saved. `now` starts
    /// the model's clock.
    pub async fn load_session(&self, now: Instant) -> Result<Model> {
        let c = &self.conn;
        let mut containers = vec![];
        let mut rows = c
            .query("SELECT id, name FROM containers ORDER BY position, id", ())
            .await?;
        while let Some(r) = rows.next().await? {
            containers.push(Container {
                id: ContainerId(narrow(r.get(0)?, "container id")?),
                name: r.get(1)?,
                storage: Storage::Disk,
            });
        }

        let default_tint = Model::new(now).spaces[0].tint;
        let mut spaces: Vec<Space> = vec![];
        let mut space_at: HashMap<SpaceId, usize> = HashMap::new();
        let mut rows = c
            .query(
                "SELECT id, name, tint, container_id, active_tab_id, icon, never_sleep, agent FROM spaces \
                 ORDER BY position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            let id = SpaceId(narrow(r.get(0)?, "space id")?);
            let tint: Option<String> = r.get(2)?;
            let active: Option<i64> = r.get(4)?;
            let icon: String = r.get(5)?;
            let icon = SpaceIcon::from_id(&icon).unwrap_or(SpaceIcon::nth(spaces.len()));
            space_at.insert(id, spaces.len());
            spaces.push(Space {
                id,
                name: r.get(1)?,
                tint: tint.as_deref().and_then(parse_tint).unwrap_or(default_tint),
                icon,
                container: ContainerId(narrow(r.get(3)?, "container id")?),
                pinned: vec![],
                items: vec![],
                active: active
                    .map(|v| narrow(v, "tab id").map(|v: u64| TabId(v)))
                    .transpose()?,
                split: None,
                never_sleep: r.get::<i64>(6)? != 0,
                // the `agent` column is the kind (a private Space is never written)
                kind: if r.get::<i64>(7)? != 0 {
                    SpaceKind::Agent
                } else {
                    SpaceKind::Normal
                },
            });
        }
        if spaces.is_empty() {
            return Ok(Model::new(now));
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
                    collapsed: false,
                }),
            ));
        }

        let mut tabs = vec![];
        let mut rows = c
            .query(
                "SELECT id, space_id, container_id, folder_id, pinned, position, url, title, last_active \
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
            // v1 did not forbid a pinned tab with a folder; pinned wins, the folder is dropped.
            match (pinned, folder) {
                (true, _) => spaces[si].pinned.push(id),
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
            }
            tabs.push(Tab {
                id,
                url: r.get(6)?,
                title: r.get(7)?,
                lifecycle: Lifecycle {
                    liveness: Liveness::Discarded,
                    pin: if pinned { Pin::Pinned } else { Pin::Unpinned },
                    activity: Activity::Idle,
                    keep: Keep::Normal,
                    hidden_since: None,
                },
                container: ContainerId(narrow(r.get(2)?, "container id")?),
                opener: None,
                last_active_ms: r.get(8)?,
                busy: Default::default(),
            });
        }
        for (space, mut items) in spaces.iter_mut().zip(top) {
            items.sort_by_key(|(p, _)| *p); // stable: ties keep read order
            space.items = items.into_iter().map(|(_, i)| i).collect();
        }

        // Splits: a group that is not a valid split of its Space's tabs is dropped, not an error,
        // so a damaged layout never costs the session.
        let mut groups: Vec<(Vec<TabId>, Vec<Option<f64>>)> = vec![(vec![], vec![]); spaces.len()];
        let mut rows = c
            .query(
                "SELECT space_id, tab_id, stop FROM split_panes ORDER BY space_id, position",
                (),
            )
            .await?;
        while let Some(r) = rows.next().await? {
            let space = SpaceId(narrow(r.get(0)?, "space id")?);
            if let Some(&si) = space_at.get(&space) {
                groups[si].0.push(TabId(narrow(r.get(1)?, "tab id")?));
                groups[si].1.push(r.get(2)?);
            }
        }
        for (space, (panes, stops)) in spaces.iter_mut().zip(groups) {
            space.split = split_group(space, panes, stops);
        }

        let mut rows = c
            .query("SELECT active_space_id FROM session WHERE id = 1", ())
            .await?;
        let active: Option<i64> = match rows.next().await? {
            Some(r) => r.get(0)?,
            None => None,
        };
        // v1 allowed no session row or a NULL active Space: fall back to the first Space.
        let active_space = match active {
            Some(v) => {
                let id = SpaceId(narrow(v, "space id")?);
                if space_at.contains_key(&id) {
                    id
                } else {
                    spaces[0].id
                }
            }
            None => spaces[0].id,
        };
        let windows = self.load_windows(&space_at, active_space).await?;
        // The model keeps the shown tabs (each pane of a shown split, in every window) Active;
        // every other tab has no renderer yet.
        let shown: Vec<TabId> = windows
            .iter()
            .filter_map(|w| spaces.iter().find(|s| Some(s.id) == w.space()))
            .flat_map(|s| {
                let a = s.active;
                match &s.split {
                    Some(g) if a.is_some_and(|a| g.tabs.contains(&a)) => g.tabs.clone(),
                    _ => a.into_iter().collect(),
                }
            })
            .collect();
        for t in tabs.iter_mut().filter(|t| shown.contains(&t.id)) {
            t.lifecycle.liveness = Liveness::Active;
        }
        Model::from_parts(spaces, containers, tabs, windows, now).map_err(Error::Corrupt)
    }

    /// The stored Main windows, focused last. A row on an unknown Space or on a Space an
    /// earlier row shows is dropped, not an error; with none left (every session before v11)
    /// one window shows `active_space`.
    async fn load_windows(
        &self,
        space_at: &HashMap<SpaceId, usize>,
        active_space: SpaceId,
    ) -> Result<Vec<Window>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, space_id, width, height, x, y FROM windows ORDER BY position",
                (),
            )
            .await?;
        let mut windows: Vec<Window> = vec![];
        while let Some(r) = rows.next().await? {
            let id = WindowId(narrow(r.get(0)?, "window id")?);
            let space = SpaceId(narrow(r.get(1)?, "space id")?);
            if !space_at.contains_key(&space)
                || windows
                    .iter()
                    .any(|w| w.id == id || w.space() == Some(space))
            {
                continue;
            }
            let int = |v: Option<i64>| v.and_then(|v| i32::try_from(v).ok());
            let size = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
            let (w, h, x, y) = (
                size(r.get(2)?),
                size(r.get(3)?),
                int(r.get(4)?),
                int(r.get(5)?),
            );
            let geometry = w.zip(h).map(|(width, height)| Geometry {
                width,
                height,
                pos: x.zip(y),
            });
            windows.push(Window {
                geometry,
                ..Window::main(id, space)
            });
        }
        if windows.is_empty() {
            windows.push(Window::main(WindowId(0), active_space));
        }
        Ok(windows)
    }

    /// Replace the stored session with `model`, in one transaction. A model that fails
    /// `Model::check` is refused and the stored session stays as it was.
    ///
    /// Written as delete-all plus the row writers below; a later diff-based save replaces the
    /// deletes with per-table diffs by id and reuses the same writers.
    pub async fn save_session(&mut self, model: &Model) -> Result<()> {
        model.check().map_err(Error::Corrupt)?;
        let tx = self.conn.transaction().await?;
        write_model(&tx, model).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The split stored for `space`: its panes left to right and the stop after each (`None` on the
/// last). `None` unless it is a valid split of the Space's tabs.
fn split_group(space: &Space, tabs: Vec<TabId>, stops: Vec<Option<f64>>) -> Option<SplitGroup> {
    let (last, seams) = stops.split_last()?;
    let stops: Vec<f64> = seams.iter().copied().collect::<Option<_>>()?;
    let in_space = |t: &TabId| {
        space.pinned.contains(t)
            || space.items.iter().any(|i| match i {
                Item::Tab(x) => x == t,
                Item::Folder(f) => f.tabs.contains(t),
            })
    };
    let fits = last.is_none()
        && (2..=MAX_PANES).contains(&tabs.len())
        && SplitGroup::stops_fit(&stops, tabs.len())
        && tabs
            .iter()
            .enumerate()
            .all(|(i, t)| in_space(t) && !tabs[..i].contains(t));
    fits.then_some(SplitGroup { tabs, stops })
}

/// Delete the whole stored session and write `model` in its place, inside `tx`. The caller has
/// run `Model::check`. Shared by `save_session` and `apply_import`. Nothing of a private window
/// is written (its Space, tabs, folders, split, window and in-memory container: Chrome never
/// writes an off-the-record profile's session), whatever model the caller passes.
pub(crate) async fn write_model(
    tx: &turso::transaction::Transaction<'_>,
    model: &Model,
) -> Result<()> {
    // Children first; foreign keys are enforced per statement.
    for table in [
        "windows",
        "split_panes",
        "tabs",
        "tab_folders",
        "spaces",
        "containers",
        "session",
    ] {
        tx.execute(format!("DELETE FROM {table}"), ()).await?;
    }
    write_containers(tx, model).await?;
    write_spaces(tx, model).await?;
    write_folders(tx, model).await?;
    write_tabs(tx, model).await?;
    write_splits(tx, model).await?;
    write_windows(tx, model).await?;
    let active = if model.is_private_space(model.active_space()) {
        model.normal_space()
    } else {
        model.active_space()
    };
    tx.execute(
        "INSERT INTO session(id, active_space_id) VALUES (1, ?1)",
        (active.0,),
    )
    .await?;
    Ok(())
}

/// The Main windows, in focus order (Little and Popup windows are not kept).
async fn write_windows(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare(
            "INSERT INTO windows(id, position, space_id, width, height, x, y) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .await?;
    let mains = m
        .windows
        .iter()
        .filter_map(|w| Some((w, w.space()?)))
        .filter(|(_, space)| !m.is_private_space(*space));
    for (i, (w, space)) in mains.enumerate() {
        let g = w.geometry;
        st.execute((
            i64::from(w.id.0),
            i as i64,
            space.0,
            g.map(|g| i64::from(g.width)),
            g.map(|g| i64::from(g.height)),
            g.and_then(|g| g.pos).map(|p| i64::from(p.0)),
            g.and_then(|g| g.pos).map(|p| i64::from(p.1)),
        ))
        .await?;
    }
    Ok(())
}

async fn write_containers(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare("INSERT INTO containers(id, name, position) VALUES (?1, ?2, ?3)")
        .await?;
    for (i, c) in m.containers.iter().filter(|c| c.storage != Storage::Memory).enumerate() {
        st.execute((c.id.0, c.name.as_str(), i as i64)).await?;
    }
    Ok(())
}

async fn write_spaces(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare(
            "INSERT INTO spaces(id, name, position, tint, container_id, active_tab_id, icon, never_sleep, \
             agent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .await?;
    for (i, s) in m.spaces.iter().filter(|s| s.kind != SpaceKind::Private).enumerate() {
        // A Space whose only tabs are in Little or Popup windows has no active tab; those
        // windows are not kept, so its first tab is shown after a restart.
        let active = s.active.or_else(|| m.order(s.id).first().copied());
        let active = active.map(tab_id).transpose()?;
        st.execute((
            s.id.0,
            s.name.as_str(),
            i as i64,
            tint_text(s.tint),
            s.container.0,
            active,
            s.icon.id(),
            i64::from(s.never_sleep),
            i64::from(s.kind == SpaceKind::Agent),
        ))
        .await?;
    }
    Ok(())
}

async fn write_folders(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare("INSERT INTO tab_folders(id, space_id, name, position) VALUES (?1, ?2, ?3, ?4)")
        .await?;
    for s in m.spaces.iter().filter(|s| s.kind != SpaceKind::Private) {
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
            "INSERT INTO tabs(id, space_id, container_id, folder_id, pinned, position, url, title, last_active) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
            tab.last_active_ms,
        ))
        .await?;
        Ok::<(), Error>(())
    };
    for s in m.spaces.iter().filter(|s| s.kind != SpaceKind::Private) {
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

async fn write_splits(tx: &turso::transaction::Transaction<'_>, m: &Model) -> Result<()> {
    let mut st = tx
        .prepare(
            "INSERT INTO split_panes(space_id, position, tab_id, stop) VALUES (?1, ?2, ?3, ?4)",
        )
        .await?;
    for s in m.spaces.iter().filter(|s| s.kind != SpaceKind::Private) {
        let Some(g) = &s.split else { continue };
        for (i, &t) in g.tabs.iter().enumerate() {
            st.execute((s.id.0, i as i64, tab_id(t)?, g.stops.get(i).copied()))
                .await?;
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

    /// The time every model here is built or loaded at.
    fn now() -> Instant {
        static NOW: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *NOW.get_or_init(Instant::now)
    }

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
        let mut m = Model::new(now());
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
        m = ap(
            m,
            Command::SetSpaceIcon {
                space: SpaceId(1),
                icon: SpaceIcon::Study,
            },
        );
        m = ap(
            m,
            Command::SetSpaceTint {
                space: SpaceId(1),
                tint: Rgb(4, 5, 6),
            },
        );
        // hidden tabs of unknown last visit are stamped by the first tick
        m = ap(
            m,
            Command::Tick {
                now: now(),
                wall_ms: 5_000,
                pressure: wubble_core::sleep::Pressure::None,
            },
        );
        m = ap(
            m,
            Command::SetSpaceNeverSleep {
                space: SpaceId(2),
                on: true,
            },
        );
        m = ap(m, Command::MarkAgentSpace(SpaceId(1)));
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
                    keep: Keep::Normal,
                    hidden_since: None,
                },
                opener: None,
                busy: Default::default(),
                ..t.clone()
            })
            .collect();
        let mut tabs: Vec<Tab> = tabs;
        let shown = m.visible_tabs();
        for t in tabs.iter_mut().filter(|t| shown.contains(&t.id)) {
            t.lifecycle.liveness = Liveness::Active;
        }
        Model::from_parts(
            m.spaces.clone(),
            m.containers.clone(),
            tabs,
            m.windows.clone(),
            now(),
        )
        .unwrap()
    }

    /// Model equality without the clock and last tick, which the store does not keep.
    #[track_caller]
    fn assert_same(a: &Model, b: &Model) {
        assert_eq!(a.spaces, b.spaces);
        assert_eq!(a.containers, b.containers);
        assert_eq!(a.tabs, b.tabs);
        assert_eq!(a.active_space(), b.active_space());
        assert_eq!(a.windows, b.windows);
    }

    #[test]
    fn windows_round_trip_and_only_main_ones_are_kept() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let ap = |m, c| apply(m, c).0;
            let mut m = ap(
                rich(),
                Command::NewWindow {
                    space: Some(SpaceId(0)),
                },
            );
            let size = |pos| Geometry {
                width: 800,
                height: 600,
                pos,
            };
            for (w, pos) in [(0, Some((10, -20))), (1, None)] {
                m = ap(
                    m,
                    Command::ResizeWindow {
                        window: WindowId(w),
                        geometry: size(pos),
                    },
                );
            }
            let with_little = ap(
                m.clone(),
                Command::OpenLittle {
                    url: "https://l.test".into(),
                    // the empty Space: its only tab is the Little window's
                    space: Some(SpaceId(2)),
                    container: None,
                },
            );
            assert_eq!(with_little.windows.len(), 3);
            store.save_session(&m).await.unwrap();
            let loaded = store.load_session(now()).await.unwrap();
            assert_same(&loaded, &restored(&m));
            assert_eq!(loaded.focused(), WindowId(1));
            assert_eq!(
                loaded.window(WindowId(0)).unwrap().geometry,
                Some(size(Some((10, -20))))
            );
            // both windows' pages come back Active
            assert_eq!(loaded.visible_tabs().len(), 2);
            // a Little window's tab is kept as a tab of its Space, without its window
            store.save_session(&with_little).await.unwrap();
            let loaded = store.load_session(now()).await.unwrap();
            assert_eq!(loaded.windows, m.windows);
            assert_eq!(loaded.tabs.len(), with_little.tabs.len());
            loaded.check().unwrap();
        });
    }

    #[test]
    fn a_session_without_window_rows_has_one_window_on_its_active_space() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&rich()).await.unwrap();
            store.conn.execute("DELETE FROM windows", ()).await.unwrap();
            let loaded = store.load_session(now()).await.unwrap();
            assert_eq!(loaded.windows, vec![Window::main(WindowId(0), SpaceId(1))]);
            // a row on an unknown Space is dropped
            store
                .conn
                .execute(
                    "INSERT INTO windows(id, position, space_id) VALUES (5, 0, 2), (6, 1, 2)",
                    (),
                )
                .await
                .unwrap();
            let loaded = store.load_session(now()).await.unwrap();
            assert_eq!(
                loaded.windows,
                vec![Window::main(WindowId(5), SpaceId(2))],
                "one Space twice"
            );
        });
    }

    #[test]
    fn empty_store_loads_a_new_model() {
        run(async {
            let store = Store::open_in_memory().await.unwrap();
            assert_same(
                &store.load_session(now()).await.unwrap(),
                &Model::new(now()),
            );
        });
    }

    #[test]
    fn rich_model_round_trips() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let m = rich();
            // The fixture covers what the test claims.
            assert_eq!(m.active_space(), SpaceId(1));
            assert_eq!(m.spaces[0].active, Some(TabId(2)));
            assert_eq!(m.spaces[0].pinned, vec![TabId(1)]);
            assert!(m.spaces[2].items.is_empty() && m.spaces[2].active.is_none());
            assert_eq!(m.spaces[1].container, m.spaces[2].container);
            assert_eq!(m.tabs[&TabId(4)].container, ContainerId(1));
            assert!(matches!(m.spaces[0].items[1], Item::Folder(_)));
            assert!(matches!(&m.spaces[0].items[3], Item::Folder(f) if f.tabs.is_empty()));
            assert!(m.spaces[2].never_sleep && !m.spaces[0].never_sleep);
            assert!(m.spaces[1].kind == SpaceKind::Agent && m.spaces[0].kind != SpaceKind::Agent);
            assert!(m.tabs.values().any(|t| t.last_active_ms == 5_000));
            store.save_session(&m).await.unwrap();
            assert_same(&store.load_session(now()).await.unwrap(), &restored(&m));
        });
    }

    #[test]
    fn ids_continue_after_load() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            store.save_session(&rich()).await.unwrap();
            let loaded = store.load_session(now()).await.unwrap();
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
            let small = new_tab(Model::new(now()), "https://x.test", None, None);
            store.save_session(&small).await.unwrap();
            assert_same(&store.load_session(now()).await.unwrap(), &restored(&small));
        });
    }

    #[test]
    fn invalid_model_is_refused_and_previous_kept() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let good = rich();
            store.save_session(&good).await.unwrap();
            let mut bad = good.clone();
            bad.windows[0].shown = wubble_core::window::Shown::Space(SpaceId(99));
            assert!(matches!(
                store.save_session(&bad).await,
                Err(Error::Corrupt(_))
            ));
            assert_same(&store.load_session(now()).await.unwrap(), &restored(&good));
        });
    }

    #[test]
    fn oversized_tab_id_is_rejected_at_save() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let m = new_tab(Model::new(now()), "https://x.test", None, None);
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
            assert_same(&store.load_session(now()).await.unwrap(), &restored(&m));
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
            let m = store.load_session(now()).await.unwrap();
            assert_eq!(m.spaces[1].tint, Model::new(now()).spaces[0].tint);
            assert_eq!(m.spaces[2].tint, Model::new(now()).spaces[0].tint);
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
            assert_same(&store.load_session(now()).await.unwrap(), &restored(&m));
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
            assert_eq!(
                migrate::schema_version(&store.conn).await.unwrap(),
                migrate::latest(MIGRATIONS)
            );

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
                    keep: Keep::Normal,
                    hidden_since: None,
                },
                container: ContainerId(container),
                opener: None,
                last_active_ms: 0,
                busy: Default::default(),
            };
            let spaces = vec![
                Space {
                    id: SpaceId(0),
                    name: "Home".into(),
                    tint: Rgb(0xaa, 0xbb, 0xcc),
                    icon: SpaceIcon::Personal,
                    container: D,
                    pinned: vec![TabId(10)],
                    // v1 never recorded the interleaving: folders first, then tabs.
                    items: vec![
                        Item::Folder(Folder {
                            id: FolderId(0),
                            name: "Reading".into(),
                            tabs: vec![TabId(12)],
                            collapsed: false,
                        }),
                        Item::Tab(TabId(11)),
                    ],
                    active: Some(TabId(10)), // backfilled: first tab, pinned first
                    split: None,
                    never_sleep: false,
                    kind: SpaceKind::Normal,
                },
                Space {
                    id: SpaceId(1),
                    name: "Jobs".into(),
                    tint: Model::new(now()).spaces[0].tint,
                    icon: SpaceIcon::Work,
                    container: ContainerId(1),
                    pinned: vec![],
                    items: vec![Item::Tab(TabId(20)), Item::Tab(TabId(21))],
                    active: Some(TabId(21)), // the saved active tab
                    split: None,
                    never_sleep: false,
                    kind: SpaceKind::Normal,
                },
            ];
            let containers = vec![
                Container {
                    id: D,
                    name: "Default".into(),
                    storage: Storage::Disk,
                },
                Container {
                    id: ContainerId(1),
                    name: "Work".into(),
                    storage: Storage::Disk,
                },
            ];
            let mut tabs = vec![
                tab(10, "https://p.test", "Pinned", 0, true),
                tab(11, "https://n.test", "Normal", 0, false),
                tab(12, "https://f.test", "Foldered", 0, false),
                tab(20, "https://j.test/2", "Two", 1, false),
                tab(21, "https://j.test/1", "Override", 0, false),
            ];
            // Space 1 is shown, so its active tab is the one live tab.
            tabs[4].lifecycle.liveness = Liveness::Active;
            let expected = Model::from_parts(
                spaces,
                containers,
                tabs,
                vec![Window::main(WindowId(0), SpaceId(1))],
                now(),
            )
            .unwrap();
            assert_same(&store.load_session(now()).await.unwrap(), &expected);
            // The migrated database also accepts a normal save.
            store.save_session(&expected).await.unwrap();
            assert_same(&store.load_session(now()).await.unwrap(), &expected);
        });
    }

    #[test]
    fn v1_without_active_space_or_with_pinned_in_folder_still_loads() {
        run(async {
            let mut store = Store::connect(":memory:").await.unwrap();
            migrate::migrate(&mut store.conn, &MIGRATIONS[..1])
                .await
                .unwrap();
            store
                .conn
                .execute_batch(
                    "INSERT INTO containers(id, name) VALUES (0, 'Default');
                     INSERT INTO spaces(id, name, position, tint, container_id) VALUES
                        (3, 'First', 0, NULL, 0), (4, 'Second', 1, NULL, 0);
                     INSERT INTO tab_folders(id, space_id, name, position) VALUES (0, 3, 'F', 0);
                     INSERT INTO tabs(id, space_id, container_id, folder_id, pinned, position, url, title, liveness)
                        VALUES (1, 3, 0, 0, 1, 0, 'https://a.test', 'A', 0);",
                )
                .await
                .unwrap();
            migrate::migrate(&mut store.conn, MIGRATIONS).await.unwrap();
            // No session row.
            let m = store.load_session(now()).await.unwrap();
            assert_eq!(m.active_space(), SpaceId(3));
            assert_eq!(m.spaces[0].pinned, vec![TabId(1)]);
            m.check().unwrap();
            // NULL, then a dangling active Space.
            for v in ["NULL", "99"] {
                store.conn.execute("DELETE FROM session", ()).await.unwrap();
                store
                    .conn
                    .execute(
                        format!("INSERT INTO session(id, active_space_id) VALUES (1, {v})"),
                        (),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    store.load_session(now()).await.unwrap().active_space(),
                    SpaceId(3)
                );
            }
        });
    }

    /// `rich()` with a three-pane split shown in Space B and a hidden two-pane split in Space A.
    fn with_splits() -> Model {
        let ap = |m, c| apply(m, c).0;
        let mut m = rich();
        m = new_tab(m, "https://b.test/3", None, None);
        // Space B: tabs 6, 7, 8; focus 6, split with 7 then 8
        m = ap(m, Command::ActivateTab(TabId(6)));
        m = ap(m, Command::SplitWith(TabId(7)));
        m = ap(m, Command::SplitWith(TabId(8)));
        m = ap(
            m,
            Command::ResizeSplit {
                space: SpaceId(1),
                stops: vec![0.25, 0.625],
            },
        );
        // Space A: 2 beside 5, then shown, then left for Space B again
        m = ap(m, Command::ActivateTab(TabId(5)));
        m = ap(m, Command::SplitWith(TabId(2)));
        m = ap(m, Command::SwitchSpace(SpaceId(1)));
        m.check().unwrap();
        m
    }

    #[test]
    fn splits_round_trip() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let m = with_splits();
            let b = m.spaces[1].split.clone().unwrap();
            assert_eq!(b.tabs, vec![TabId(6), TabId(7), TabId(8)]);
            assert_eq!(m.visible_tabs().len(), 3);
            assert_eq!(m.spaces[0].split.as_ref().unwrap().tabs.len(), 2);
            assert!(m.spaces[2].split.is_none());
            store.save_session(&m).await.unwrap();
            let back = store.load_session(now()).await.unwrap();
            assert_same(&back, &restored(&m));
            for t in [6, 7, 8] {
                assert_eq!(back.tabs[&TabId(t)].lifecycle.liveness, Liveness::Active);
            }
            assert_eq!(
                back.spaces[1].split.as_ref().unwrap().stops,
                vec![0.25, 0.625]
            );
        });
    }

    #[test]
    fn a_bad_stored_split_is_dropped_and_the_session_kept() {
        run(async {
            let mut store = Store::open_in_memory().await.unwrap();
            let m = with_splits();
            store.save_session(&m).await.unwrap();
            for damage in [
                // a pane from another Space
                "UPDATE split_panes SET tab_id = 2 WHERE space_id = 1 AND position = 1",
                // seams out of order
                "UPDATE split_panes SET stop = 0.9 WHERE space_id = 1 AND position = 0",
                // a missing seam
                "UPDATE split_panes SET stop = NULL WHERE space_id = 1 AND position = 1",
                // a pane short
                "DELETE FROM split_panes WHERE space_id = 0 AND position = 1",
            ] {
                store.save_session(&m).await.unwrap();
                store.conn.execute(damage, ()).await.unwrap();
                let back = store.load_session(now()).await.unwrap();
                back.check().unwrap();
                assert_eq!(back.tabs.len(), m.tabs.len(), "{damage}");
                let space = if damage.contains("space_id = 0") {
                    0
                } else {
                    1
                };
                assert_eq!(back.spaces[space].split, None, "{damage}");
                assert_eq!(
                    back.spaces[1 - space].split,
                    m.spaces[1 - space].split,
                    "{damage}"
                );
            }
        });
    }

    #[test]
    fn migrating_v6_to_v7_keeps_the_session_and_adds_no_split() {
        run(async {
            let mut store = Store::connect(":memory:").await.unwrap();
            migrate::migrate(&mut store.conn, &MIGRATIONS[..6])
                .await
                .unwrap();
            let mut m = rich();
            // v6 has no icon column: migration 10 gives each Space its icon by position.
            for (i, s) in m.spaces.iter_mut().enumerate() {
                s.icon = SpaceIcon::nth(i);
                s.never_sleep = false; // no such column before v12
                s.kind = SpaceKind::Normal; // nor this one before v15
            }
            m.tabs.values_mut().for_each(|t| t.last_active_ms = 0);
            let tx = store.conn.transaction().await.unwrap();
            // the v6 writers, without the split table or the icon
            write_containers(&tx, &m).await.unwrap();
            for (i, s) in m.spaces.iter().enumerate() {
                tx.execute(
                    "INSERT INTO spaces(id, name, position, tint, container_id, active_tab_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        s.id.0,
                        s.name.as_str(),
                        i as i64,
                        tint_text(s.tint),
                        s.container.0,
                        s.active.map(tab_id).transpose().unwrap(),
                    ),
                )
                .await
                .unwrap();
            }
            write_folders(&tx, &m).await.unwrap();
            // v6 has no last_active column: the v12 writer, with the column added for it
            tx.execute_batch("ALTER TABLE tabs ADD COLUMN last_active INTEGER NOT NULL DEFAULT 0")
                .await
                .unwrap();
            write_tabs(&tx, &m).await.unwrap();
            tx.execute_batch("ALTER TABLE tabs DROP COLUMN last_active")
                .await
                .unwrap();
            tx.execute("INSERT INTO session(id, active_space_id) VALUES (1, 1)", ())
                .await
                .unwrap();
            tx.commit().await.unwrap();
            assert!(
                store
                    .conn
                    .query("SELECT * FROM split_panes", ())
                    .await
                    .is_err()
            );
            migrate::migrate(&mut store.conn, MIGRATIONS).await.unwrap();
            assert_eq!(
                migrate::schema_version(&store.conn).await.unwrap(),
                migrate::latest(MIGRATIONS)
            );
            let back = store.load_session(now()).await.unwrap();
            assert_same(&back, &restored(&m));
            store.save_session(&with_splits()).await.unwrap();
        });
    }
}
