//! The recently-closed list: tabs, Main windows, folders and Spaces, for reopen-closed. A
//! bounded list, newest first (`wubble_core::closed::Entry`).
//!
//! Design reference, no code copied: Chromium `components/sessions/core/tab_restore_service_helper.cc`
//! (entries kept newest first, oldest pruned past a maximum; a window or group entry holds its
//! tabs), BSD-3-Clause.
//!
//! No foreign keys to the session: the Space may be gone by the time an entry is reopened. A
//! grouped entry's tabs are `recently_closed_tabs` rows, which go with their entry.

use wubble_core::closed::{
    ClosedFolder, ClosedSpace, ClosedWindow, Entry, Listed, Slot, SpaceSnap, TabSnap,
};
use wubble_core::space::{ContainerId, Rgb, SpaceIcon, SpaceId};
use wubble_core::window::Geometry;

use crate::session::{parse_tint, tint_text};
use crate::*;

pub use wubble_core::closed::ClosedTab;
/// Most entries kept; older ones are dropped on push.
pub use wubble_core::closed::RECENTLY_CLOSED_MAX;

const KIND_TAB: i64 = 0;
const KIND_WINDOW: i64 = 1;
const KIND_FOLDER: i64 = 2;
const KIND_SPACE: i64 = 3;

const COLUMNS: &str = "id, kind, closed_at, space_id, container_id, url, title, name, tint, icon, width, height, x, y";

/// An entry's own row, before its tabs are read.
struct Head {
    id: i64,
    kind: i64,
    closed_at_ms: i64,
    space: Option<SpaceId>,
    container: ContainerId,
    url: String,
    title: String,
    name: String,
    tint: Rgb,
    icon: SpaceIcon,
    geometry: Option<Geometry>,
}

fn read_head(r: &turso::Row) -> Result<Head> {
    let space: Option<i64> = r.get(3)?;
    let int = |v: Option<i64>| v.and_then(|v| i32::try_from(v).ok());
    let size = |v: Option<i64>| v.and_then(|v| u32::try_from(v).ok());
    let (w, h, x, y) = (
        size(r.get(10)?),
        size(r.get(11)?),
        int(r.get(12)?),
        int(r.get(13)?),
    );
    let tint: Option<String> = r.get(8)?;
    let icon: Option<String> = r.get(9)?;
    let name: Option<String> = r.get(7)?;
    Ok(Head {
        id: r.get(0)?,
        kind: r.get(1)?,
        closed_at_ms: r.get(2)?,
        space: space
            .map(|v| narrow(v, "space id").map(SpaceId))
            .transpose()?,
        container: ContainerId(narrow(r.get(4)?, "container id")?),
        url: r.get(5)?,
        title: r.get(6)?,
        name: name.unwrap_or_default(),
        tint: tint
            .as_deref()
            .and_then(parse_tint)
            .unwrap_or(Rgb(0x6a, 0x8f, 0xff)),
        icon: icon
            .as_deref()
            .and_then(SpaceIcon::from_id)
            .unwrap_or(SpaceIcon::Personal),
        geometry: w.zip(h).map(|(width, height)| Geometry {
            width,
            height,
            pos: x.zip(y),
            // a closed window reopens in its normal size, on whatever monitor holds it
            ..Geometry::default()
        }),
    })
}

/// The entry `head` is, with `tabs` (its `recently_closed_tabs` rows); None for a grouped kind
/// left without a tab, or an unknown kind (a newer build's).
fn entry(head: Head, tabs: Vec<TabSnap>) -> Option<Entry> {
    let snap = |head: &Head, tabs| SpaceSnap {
        name: head.name.clone(),
        tint: head.tint,
        icon: head.icon,
        container: head.container,
        tabs,
    };
    if head.kind != KIND_TAB && tabs.is_empty() {
        return None;
    }
    Some(match head.kind {
        KIND_TAB => Entry::Tab(ClosedTab {
            url: head.url,
            title: head.title,
            container: head.container,
            space: head.space,
            closed_at_ms: head.closed_at_ms,
        }),
        KIND_WINDOW => Entry::Window(ClosedWindow {
            space: head.space?,
            snap: snap(&head, tabs),
            geometry: head.geometry,
            closed_at_ms: head.closed_at_ms,
        }),
        KIND_FOLDER => Entry::Folder(ClosedFolder {
            name: head.name,
            space: head.space,
            tabs,
            closed_at_ms: head.closed_at_ms,
        }),
        KIND_SPACE => Entry::Space(ClosedSpace {
            snap: snap(&head, tabs),
            closed_at_ms: head.closed_at_ms,
        }),
        _ => return None,
    })
}

fn kind_of(entry: &Entry) -> i64 {
    match entry {
        Entry::Tab(_) => KIND_TAB,
        Entry::Window(_) => KIND_WINDOW,
        Entry::Folder(_) => KIND_FOLDER,
        Entry::Space(_) => KIND_SPACE,
    }
}

impl Store {
    /// Record an entry as the newest and drop entries beyond [`RECENTLY_CLOSED_MAX`] (their tabs
    /// go with them).
    pub async fn push_closed(&mut self, entry: &Entry) -> Result<()> {
        let tx = self.conn.transaction().await?;
        let none = || None::<String>;
        let (space, container, url, title, name, tint, icon, geometry) = match entry {
            Entry::Tab(t) => (
                t.space,
                t.container,
                t.url.as_str(),
                t.title.as_str(),
                none(),
                none(),
                none(),
                None,
            ),
            Entry::Window(w) => (
                Some(w.space),
                w.snap.container,
                "",
                "",
                Some(w.snap.name.clone()),
                Some(tint_text(w.snap.tint)),
                Some(w.snap.icon.id().to_owned()),
                w.geometry.as_ref(),
            ),
            Entry::Folder(f) => (
                f.space,
                ContainerId::DEFAULT,
                "",
                "",
                Some(f.name.clone()),
                none(),
                none(),
                None,
            ),
            Entry::Space(s) => (
                None,
                s.snap.container,
                "",
                "",
                Some(s.snap.name.clone()),
                Some(tint_text(s.snap.tint)),
                Some(s.snap.icon.id().to_owned()),
                None,
            ),
        };
        tx.execute(
            "INSERT INTO recently_closed(kind, closed_at, space_id, container_id, url, title, \
             name, tint, icon, width, height, x, y) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            (
                kind_of(entry),
                entry.closed_at_ms(),
                space.map(|s| s.0),
                container.0,
                url,
                title,
                name,
                tint,
                icon,
                geometry.map(|g| i64::from(g.width)),
                geometry.map(|g| i64::from(g.height)),
                geometry.and_then(|g| g.pos).map(|p| i64::from(p.0)),
                geometry.and_then(|g| g.pos).map(|p| i64::from(p.1)),
            ),
        )
        .await?;
        let tabs = entry.tabs();
        if !tabs.is_empty() {
            let id = {
                let mut rows = tx.query("SELECT max(id) FROM recently_closed", ()).await?;
                let row = rows
                    .next()
                    .await?
                    .ok_or_else(|| Error::Corrupt("no row id for a closed entry".into()))?;
                row.get::<i64>(0)?
            };
            for (position, t) in tabs.iter().enumerate() {
                let (pinned, folder, folder_name) = match &t.slot {
                    Slot::Pinned => (1i64, None, None),
                    Slot::Normal => (0, None, None),
                    Slot::Folder { folder, name } => {
                        (0, Some(i64::from(*folder)), Some(name.as_str()))
                    }
                };
                tx.execute(
                    "INSERT INTO recently_closed_tabs(entry_id, position, url, title, \
                     container_id, pinned, folder, folder_name) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    (
                        id,
                        i64::try_from(position).unwrap_or(i64::MAX),
                        t.url.as_str(),
                        t.title.as_str(),
                        t.container.0,
                        pinned,
                        folder,
                        folder_name,
                    ),
                )
                .await?;
            }
        }
        tx.execute(
            format!(
                "DELETE FROM recently_closed WHERE id NOT IN \
                 (SELECT id FROM recently_closed ORDER BY id DESC LIMIT {RECENTLY_CLOSED_MAX})"
            ),
            (),
        )
        .await?;
        tx.execute(
            "DELETE FROM recently_closed_tabs WHERE entry_id NOT IN (SELECT id FROM recently_closed)",
            (),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The tabs of entry `id`, in order.
    async fn closed_tabs(conn: &turso::Connection, id: i64) -> Result<Vec<TabSnap>> {
        let mut rows = conn
            .query(
                "SELECT url, title, container_id, pinned, folder, folder_name \
                 FROM recently_closed_tabs WHERE entry_id = ?1 ORDER BY position",
                (id,),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            let pinned: i64 = r.get(3)?;
            let folder: Option<i64> = r.get(4)?;
            let folder_name: Option<String> = r.get(5)?;
            let slot = match folder.and_then(|f| u32::try_from(f).ok()) {
                Some(folder) => Slot::Folder {
                    folder,
                    name: folder_name.unwrap_or_default(),
                },
                None if pinned != 0 => Slot::Pinned,
                None => Slot::Normal,
            };
            out.push(TabSnap {
                url: r.get(0)?,
                title: r.get(1)?,
                container: ContainerId(narrow(r.get(2)?, "container id")?),
                slot,
            });
        }
        Ok(out)
    }

    async fn closed_heads(conn: &turso::Connection, limit: Option<usize>) -> Result<Vec<Head>> {
        let limit = limit.map_or(String::new(), |n| format!(" LIMIT {n}"));
        let mut rows = conn
            .query(
                format!("SELECT {COLUMNS} FROM recently_closed ORDER BY id DESC{limit}"),
                (),
            )
            .await?;
        let mut heads = vec![];
        while let Some(r) = rows.next().await? {
            heads.push(read_head(&r)?);
        }
        Ok(heads)
    }

    /// Newest first, each with its row id.
    pub async fn recently_closed(&self) -> Result<Vec<Listed>> {
        let mut out = vec![];
        for head in Self::closed_heads(&self.conn, None).await? {
            let id = head.id;
            let tabs = Self::closed_tabs(&self.conn, id).await?;
            if let Some(entry) = entry(head, tabs) {
                out.push(Listed { id, entry });
            }
        }
        Ok(out)
    }

    /// Remove and return the newest entry (reopen closed). An entry this build cannot read is
    /// dropped on the way.
    pub async fn pop_closed(&mut self) -> Result<Option<Entry>> {
        loop {
            let Some(head) = Self::closed_heads(&self.conn, Some(1)).await?.pop() else {
                return Ok(None);
            };
            let id = head.id;
            if let Some(entry) = self.take_closed(id).await? {
                return Ok(Some(entry));
            }
        }
    }

    /// Remove and return entry `id` (a row of the list reopened); None when it is gone or
    /// unreadable (it is removed all the same).
    pub async fn take_closed(&mut self, id: i64) -> Result<Option<Entry>> {
        let tx = self.conn.transaction().await?;
        let head = {
            let mut rows = tx
                .query(
                    format!("SELECT {COLUMNS} FROM recently_closed WHERE id = ?1"),
                    (id,),
                )
                .await?;
            match rows.next().await? {
                Some(r) => Some(read_head(&r)?),
                None => None,
            }
        };
        let Some(head) = head else {
            return Ok(None);
        };
        let tabs = Self::closed_tabs(&tx, id).await?;
        tx.execute(
            "DELETE FROM recently_closed_tabs WHERE entry_id = ?1",
            (id,),
        )
        .await?;
        tx.execute("DELETE FROM recently_closed WHERE id = ?1", (id,))
            .await?;
        tx.commit().await?;
        Ok(entry(head, tabs))
    }

    /// Forget entry `id` (a row's delete).
    pub async fn delete_closed(&mut self, id: i64) -> Result<()> {
        let tx = self.conn.transaction().await?;
        tx.execute(
            "DELETE FROM recently_closed_tabs WHERE entry_id = ?1",
            (id,),
        )
        .await?;
        tx.execute("DELETE FROM recently_closed WHERE id = ?1", (id,))
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// After history rows of the list were deleted: tabs whose entry is gone go, and grouped
/// entries left without a tab go too.
pub(crate) async fn prune_closed(conn: &turso::Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM recently_closed_tabs WHERE entry_id NOT IN (SELECT id FROM recently_closed)",
        (),
    )
    .await?;
    conn.execute(
        format!(
            "DELETE FROM recently_closed WHERE kind <> {KIND_TAB} AND id NOT IN \
             (SELECT entry_id FROM recently_closed_tabs)"
        ),
        (),
    )
    .await?;
    Ok(())
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

    fn entries(listed: Vec<Listed>) -> Vec<Entry> {
        listed.into_iter().map(|l| l.entry).collect()
    }

    #[test]
    fn bound_keeps_the_newest_in_order() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for i in 0..30 {
                s.push_closed(&closed(i).into()).await.unwrap();
            }
            let got = entries(s.recently_closed().await.unwrap());
            assert_eq!(got.len(), RECENTLY_CLOSED_MAX);
            let want: Vec<Entry> = (5..30).rev().map(|i| closed(i).into()).collect();
            assert_eq!(got, want);
        });
    }

    #[test]
    fn pop_returns_newest_and_empty_pop_is_none() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            assert_eq!(s.pop_closed().await.unwrap(), None);
            for i in 0..3 {
                s.push_closed(&closed(i).into()).await.unwrap();
            }
            assert_eq!(s.pop_closed().await.unwrap(), Some(closed(2).into()));
            assert_eq!(
                entries(s.recently_closed().await.unwrap()),
                vec![Entry::from(closed(1)), closed(0).into()]
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
            s.push_closed(&closed(0).into()).await.unwrap(); // space 0
            s.conn.execute("DELETE FROM tabs", ()).await.unwrap();
            s.conn.execute("DELETE FROM spaces", ()).await.unwrap();
            assert_eq!(
                entries(s.recently_closed().await.unwrap()),
                vec![Entry::from(closed(0))]
            );
        });
    }
}
