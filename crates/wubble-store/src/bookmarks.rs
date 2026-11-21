//! Bookmarks: a tree of folders and links over the `bookmarks` table (Firefox Places
//! `Bookmarks.sys.mjs` shape: parent + dense sibling position, no move into the own subtree;
//! design only, no code copied, see THIRD_PARTY_NOTICES.md).
//!
//! Invariants, kept by every writer inside its transaction and asserted by
//! `Store::check_consistency`: a parent is a folder; a folder has no url and a link has one;
//! sibling positions are 0..n with no gaps (roots are the rows with a NULL parent); no cycles.

use std::collections::HashSet;

use turso::Connection;
use wubble_core::omnibox::Bookmark;
use wubble_core::pages::{BookmarkEdit, BookmarkRow};

use crate::{Error, Result, Store, narrow, scalar, url_key};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BookmarkId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookmarkKind {
    Link = 0,
    Folder = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookmarkNode {
    pub id: BookmarkId,
    pub parent: Option<BookmarkId>,
    pub kind: BookmarkKind,
    pub position: u32,
    pub title: String,
    /// `Some` for links only.
    pub url: Option<String>,
    pub added_ms: i64,
    pub modified_ms: i64,
}

impl From<&BookmarkNode> for BookmarkRow {
    fn from(n: &BookmarkNode) -> BookmarkRow {
        BookmarkRow {
            id: n.id.0,
            parent: n.parent.map(|p| p.0),
            is_folder: n.kind == BookmarkKind::Folder,
            position: n.position,
            title: n.title.clone(),
            url: n.url.clone(),
        }
    }
}

const COLUMNS: &str =
    "id, parent_id, kind, position, title, url, date_added, date_modified FROM bookmarks";

fn node(r: &turso::Row) -> Result<BookmarkNode> {
    let kind = match r.get::<i64>(2)? {
        0 => BookmarkKind::Link,
        1 => BookmarkKind::Folder,
        k => return Err(Error::Corrupt(format!("bookmark kind {k}"))),
    };
    Ok(BookmarkNode {
        id: BookmarkId(r.get(0)?),
        parent: r.get::<Option<i64>>(1)?.map(BookmarkId),
        kind,
        position: narrow(r.get(3)?, "bookmark position")?,
        title: r.get(4)?,
        url: r.get(5)?,
        added_ms: r.get(6)?,
        modified_ms: r.get(7)?,
    })
}

/// Links must carry something navigable; scripts are never stored.
pub(crate) fn valid_link_url(url: &str) -> bool {
    let u = url.trim();
    !u.is_empty() && !u.to_ascii_lowercase().starts_with("javascript:")
}

/// The kind of bookmark `id`, `None` when absent.
async fn kind_of(c: &Connection, id: i64) -> Result<Option<i64>> {
    scalar(c, "SELECT kind FROM bookmarks WHERE id = ?1", [id]).await
}

/// Refuse a parent that is missing or a link.
async fn require_folder(c: &Connection, parent: Option<i64>) -> Result<()> {
    if let Some(p) = parent {
        match kind_of(c, p).await? {
            Some(1) => {}
            Some(_) => return Err(Error::Invalid(format!("bookmark {p} is not a folder"))),
            None => return Err(Error::Invalid(format!("no bookmark {p}"))),
        }
    }
    Ok(())
}

/// Append a node as the last child of `parent`. The one insert path: the public API and the
/// import share it.
pub(crate) async fn insert_node(
    c: &Connection,
    parent: Option<i64>,
    kind: BookmarkKind,
    title: &str,
    url: Option<&str>,
    added_ms: i64,
    modified_ms: i64,
) -> Result<i64> {
    require_folder(c, parent).await?;
    let position = scalar(
        c,
        "SELECT COUNT(*) FROM bookmarks WHERE parent_id IS ?1",
        [parent],
    )
    .await?
    .unwrap_or(0);
    c.execute(
        "INSERT INTO bookmarks(parent_id, kind, position, title, url, date_added, date_modified) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        (
            parent,
            kind as i64,
            position,
            title,
            url,
            added_ms,
            modified_ms,
        ),
    )
    .await?;
    Ok(c.last_insert_rowid())
}

/// A folder under `parent` with exactly this title, if any.
pub(crate) async fn find_folder(
    c: &Connection,
    parent: Option<i64>,
    title: &str,
) -> Result<Option<i64>> {
    scalar(
        c,
        "SELECT id FROM bookmarks WHERE parent_id IS ?1 AND kind = 1 AND title = ?2 ORDER BY position LIMIT 1",
        (parent, title),
    )
    .await
}

/// A link under `parent` with exactly this url, if any.
pub(crate) async fn find_link(
    c: &Connection,
    parent: Option<i64>,
    url: &str,
) -> Result<Option<i64>> {
    scalar(
        c,
        "SELECT id FROM bookmarks WHERE parent_id IS ?1 AND kind = 0 AND url = ?2 ORDER BY position LIMIT 1",
        (parent, url),
    )
    .await
}

/// Close the hole a node left at `position` under `parent`.
async fn close_gap(c: &Connection, parent: Option<i64>, position: i64) -> Result<()> {
    c.execute(
        "UPDATE bookmarks SET position = position - 1 WHERE parent_id IS ?1 AND position > ?2",
        (parent, position),
    )
    .await?;
    Ok(())
}

impl Store {
    /// A folder as the last child of `parent` (`None` = top level).
    pub async fn add_folder(
        &mut self,
        parent: Option<BookmarkId>,
        title: &str,
        now_ms: i64,
    ) -> Result<BookmarkId> {
        let tx = self.conn.transaction().await?;
        let id = insert_node(
            &tx,
            parent.map(|p| p.0),
            BookmarkKind::Folder,
            title,
            None,
            now_ms,
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(BookmarkId(id))
    }

    /// A link as the last child of `parent`.
    pub async fn add_bookmark(
        &mut self,
        parent: Option<BookmarkId>,
        title: &str,
        url: &str,
        now_ms: i64,
    ) -> Result<BookmarkId> {
        if !valid_link_url(url) {
            return Err(Error::Invalid(format!("not a bookmarkable url: {url:?}")));
        }
        let tx = self.conn.transaction().await?;
        let id = insert_node(
            &tx,
            parent.map(|p| p.0),
            BookmarkKind::Link,
            title,
            Some(url),
            now_ms,
            now_ms,
        )
        .await?;
        tx.commit().await?;
        Ok(BookmarkId(id))
    }

    /// Move `id` under `to` at `index` (clamped to the end). A folder cannot move into itself
    /// or one of its descendants.
    pub async fn move_bookmark(
        &mut self,
        id: BookmarkId,
        to: Option<BookmarkId>,
        index: u32,
        now_ms: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction().await?;
        let mut rows = tx
            .query(
                "SELECT parent_id, position FROM bookmarks WHERE id = ?1",
                [id.0],
            )
            .await?;
        let (old_parent, old_pos): (Option<i64>, i64) = match rows.next().await? {
            Some(r) => (r.get(0)?, r.get(1)?),
            None => return Err(Error::Invalid(format!("no bookmark {}", id.0))),
        };
        drop(rows);
        let to = to.map(|t| t.0);
        require_folder(&tx, to).await?;
        // Walk the new parent's ancestors; meeting `id` means a move into the own subtree.
        let mut at = to;
        let mut steps = 0;
        while let Some(a) = at {
            if a == id.0 {
                return Err(Error::Invalid(format!(
                    "bookmark {} cannot move into itself",
                    id.0
                )));
            }
            steps += 1;
            if steps > 100_000 {
                return Err(Error::Corrupt("bookmark tree too deep or cyclic".into()));
            }
            let mut up = tx
                .query("SELECT parent_id FROM bookmarks WHERE id = ?1", [a])
                .await?;
            at = match up.next().await? {
                Some(r) => r.get(0)?,
                None => None,
            };
        }
        close_gap(&tx, old_parent, old_pos).await?;
        let siblings = scalar(
            &tx,
            "SELECT COUNT(*) FROM bookmarks WHERE parent_id IS ?1 AND id <> ?2",
            (to, id.0),
        )
        .await?
        .unwrap_or(0);
        let index = i64::from(index).min(siblings);
        tx.execute(
            "UPDATE bookmarks SET position = position + 1 WHERE parent_id IS ?1 AND position >= ?2 AND id <> ?3",
            (to, index, id.0),
        )
        .await?;
        tx.execute(
            "UPDATE bookmarks SET parent_id = ?1, position = ?2, date_modified = ?3 WHERE id = ?4",
            (to, index, now_ms, id.0),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn rename_bookmark(
        &mut self,
        id: BookmarkId,
        title: &str,
        now_ms: i64,
    ) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE bookmarks SET title = ?1, date_modified = ?2 WHERE id = ?3",
                (title, now_ms, id.0),
            )
            .await?;
        if n == 0 {
            return Err(Error::Invalid(format!("no bookmark {}", id.0)));
        }
        Ok(())
    }

    /// Delete a node and everything below it, closing the gap among its siblings. Returns the
    /// number of rows removed (0 when `id` does not exist).
    pub async fn delete_bookmark(&mut self, id: BookmarkId) -> Result<u64> {
        let tx = self.conn.transaction().await?;
        let mut rows = tx
            .query(
                "SELECT parent_id, position FROM bookmarks WHERE id = ?1",
                [id.0],
            )
            .await?;
        let (parent, pos): (Option<i64>, i64) = match rows.next().await? {
            Some(r) => (r.get(0)?, r.get(1)?),
            None => return Ok(0),
        };
        drop(rows);
        let before = scalar(&tx, "SELECT COUNT(*) FROM bookmarks", ())
            .await?
            .unwrap_or(0);
        tx.execute("DELETE FROM bookmarks WHERE id = ?1", [id.0])
            .await?; // cascades
        let after = scalar(&tx, "SELECT COUNT(*) FROM bookmarks", ())
            .await?
            .unwrap_or(0);
        close_gap(&tx, parent, pos).await?;
        tx.commit().await?;
        Ok(u64::try_from(before - after).unwrap_or(0))
    }

    /// The children of `parent` (`None` = top level) in order.
    pub async fn bookmark_children(&self, parent: Option<BookmarkId>) -> Result<Vec<BookmarkNode>> {
        let mut rows = self
            .conn
            .query(
                &format!("SELECT {COLUMNS} WHERE parent_id IS ?1 ORDER BY position"),
                [parent.map(|p| p.0)],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(node(&r)?);
        }
        Ok(out)
    }

    /// Applies one page edit, then returns every row, so the page is redrawn from what is stored.
    pub async fn apply_bookmark_edit(
        &mut self,
        edit: &BookmarkEdit,
        now_ms: i64,
    ) -> Result<Vec<BookmarkRow>> {
        let id = |i: &i64| BookmarkId(*i);
        match edit {
            BookmarkEdit::Add { parent, title, url } => {
                self.add_bookmark(parent.as_ref().map(id), title, url, now_ms)
                    .await?;
            }
            BookmarkEdit::AddFolder { parent, title } => {
                self.add_folder(parent.as_ref().map(id), title, now_ms)
                    .await?;
            }
            BookmarkEdit::Rename { id: i, title } => {
                self.rename_bookmark(id(i), title, now_ms).await?;
            }
            BookmarkEdit::Move { id: i, to, index } => {
                self.move_bookmark(id(i), to.as_ref().map(id), *index, now_ms)
                    .await?;
            }
            BookmarkEdit::Delete { id: i } => {
                self.delete_bookmark(id(i)).await?;
            }
        }
        self.all_bookmarks().await
    }

    /// Every bookmark and folder in one query, for the bookmarks page
    /// (`wubble_core::pages::bookmark_tree` builds the tree).
    pub async fn all_bookmarks(&self) -> Result<Vec<BookmarkRow>> {
        let mut rows = self
            .conn
            .query(&format!("SELECT {COLUMNS} ORDER BY id"), ())
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(BookmarkRow::from(&node(&r)?));
        }
        Ok(out)
    }

    /// Command-bar bookmark candidates: links whose title or url contains `query`
    /// (case-insensitive, `%` and `_` literal). Url-prefix matches (scheme and `www.` ignored)
    /// come first, then title-prefix matches, then the rest; newer first within a tier. One
    /// entry per url. An empty query returns nothing.
    pub async fn search_bookmarks(&self, query: &str, limit: usize) -> Result<Vec<Bookmark>> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let key = url_key(&needle);
        // SQLite's `lower()` folds ASCII only: for a query with other letters every link is
        // read and matched below.
        let sql = if needle.is_ascii() {
            "SELECT title, url, date_added, id FROM bookmarks WHERE kind = 0 \
             AND (instr(lower(title), ?1) > 0 OR instr(lower(url), ?1) > 0 \
                  OR (?2 <> '' AND instr(lower(url), ?2) > 0))"
        } else {
            "SELECT title, url, date_added, id FROM bookmarks WHERE kind = 0 \
             AND (?1 <> '' OR ?2 <> '')"
        };
        let mut rows = self
            .conn
            .query(sql, (needle.as_str(), key.as_str()))
            .await?;
        let mut hits: Vec<(u8, i64, i64, Bookmark)> = Vec::new();
        while let Some(r) = rows.next().await? {
            let title: String = r.get(0)?;
            let url: String = r.get(1)?;
            let url_start = url_key(&url).starts_with(&key);
            // The SQL filter is a superset: a query that carried a scheme or `www.` is the
            // start of a url, so it must prefix the url key rather than occur anywhere.
            let found = title.to_lowercase().contains(&needle)
                || url.to_lowercase().contains(&needle)
                || (key != needle && url_start);
            if !found {
                continue;
            }
            let tier = if url_start {
                0
            } else if title.to_lowercase().starts_with(&needle) {
                1
            } else {
                2
            };
            hits.push((tier, -r.get::<i64>(2)?, r.get(3)?, Bookmark { url, title }));
        }
        hits.sort_by_key(|(tier, newest, id, _)| (*tier, *newest, *id));
        let mut seen = HashSet::new();
        Ok(hits
            .into_iter()
            .map(|(.., b)| b)
            .filter(|b| seen.insert(b.url.clone()))
            .take(limit)
            .collect())
    }

    /// Total rows, for tests.
    #[cfg(test)]
    pub(crate) async fn bookmark_count(&self) -> Result<i64> {
        Ok(scalar(&self.conn, "SELECT COUNT(*) FROM bookmarks", ())
            .await?
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    async fn titles(s: &Store, parent: Option<BookmarkId>) -> Vec<String> {
        s.bookmark_children(parent)
            .await
            .unwrap()
            .into_iter()
            .map(|n| n.title)
            .collect()
    }

    async fn consistent(s: &Store) {
        let p = s.check_consistency().await.unwrap();
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn all_bookmarks_feeds_the_core_tree() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let f = s.add_folder(None, "Work", 10).await.unwrap();
            s.add_bookmark(Some(f), "Docs", "https://docs.test", 11)
                .await
                .unwrap();
            s.add_bookmark(None, "Home", "https://home.test", 12)
                .await
                .unwrap();
            let rows = s.all_bookmarks().await.unwrap();
            assert_eq!(rows.len(), 3);
            let tree = wubble_core::pages::bookmark_tree(&rows);
            let top: Vec<_> = tree
                .iter()
                .map(|i| (i.title.as_str(), i.is_folder))
                .collect();
            assert_eq!(top, [("Work", true), ("Home", false)]);
            assert_eq!(
                tree[0].children[0].url.as_deref(),
                Some("https://docs.test")
            );
            assert!(
                Store::open_in_memory()
                    .await
                    .unwrap()
                    .all_bookmarks()
                    .await
                    .unwrap()
                    .is_empty()
            );
        });
    }

    #[test]
    fn add_and_list_in_order_with_nesting() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let f = s.add_folder(None, "Work", 10).await.unwrap();
            let a = s
                .add_bookmark(None, "A", "https://a.test/", 11)
                .await
                .unwrap();
            let g = s.add_folder(Some(f), "Sub", 12).await.unwrap();
            s.add_bookmark(Some(f), "B", "https://b.test/", 13)
                .await
                .unwrap();
            s.add_bookmark(Some(g), "C", "https://c.test/", 14)
                .await
                .unwrap();
            assert_eq!(titles(&s, None).await, ["Work", "A"]);
            assert_eq!(titles(&s, Some(f)).await, ["Sub", "B"]);
            assert_eq!(titles(&s, Some(g)).await, ["C"]);
            let root = s.bookmark_children(None).await.unwrap();
            assert_eq!(root[0].kind, BookmarkKind::Folder);
            assert_eq!(root[0].url, None);
            assert_eq!((root[0].position, root[1].position), (0, 1));
            assert_eq!(root[1].id, a);
            assert_eq!(root[1].url.as_deref(), Some("https://a.test/"));
            assert_eq!((root[1].added_ms, root[1].modified_ms), (11, 11));
            consistent(&s).await;
        });
    }

    #[test]
    fn parent_must_be_a_folder_and_urls_are_checked() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = s
                .add_bookmark(None, "A", "https://a.test/", 1)
                .await
                .unwrap();
            assert!(matches!(
                s.add_folder(Some(a), "x", 1).await,
                Err(Error::Invalid(_))
            ));
            assert!(matches!(
                s.add_bookmark(Some(BookmarkId(999)), "x", "https://x.test", 1)
                    .await,
                Err(Error::Invalid(_))
            ));
            for bad in ["", "  ", "JavaScript:alert(1)"] {
                assert!(matches!(
                    s.add_bookmark(None, "x", bad, 1).await,
                    Err(Error::Invalid(_))
                ));
            }
            assert_eq!(s.bookmark_count().await.unwrap(), 1);
            consistent(&s).await;
        });
    }

    #[test]
    fn apply_bookmark_edit_returns_the_fresh_rows() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let rows = s
                .apply_bookmark_edit(
                    &BookmarkEdit::AddFolder {
                        parent: None,
                        title: "F".into(),
                    },
                    1,
                )
                .await
                .unwrap();
            let f = rows[0].id;
            let rows = s
                .apply_bookmark_edit(
                    &BookmarkEdit::Add {
                        parent: Some(f),
                        title: "A".into(),
                        url: "https://a.test".into(),
                    },
                    2,
                )
                .await
                .unwrap();
            assert_eq!(rows.len(), 2);
            let a = rows.iter().find(|r| !r.is_folder).unwrap().id;
            let rows = s
                .apply_bookmark_edit(
                    &BookmarkEdit::Rename {
                        id: a,
                        title: "B".into(),
                    },
                    3,
                )
                .await
                .unwrap();
            assert!(rows.iter().any(|r| r.title == "B"));
            let rows = s
                .apply_bookmark_edit(
                    &BookmarkEdit::Move {
                        id: a,
                        to: None,
                        index: 0,
                    },
                    4,
                )
                .await
                .unwrap();
            assert_eq!(rows.iter().find(|r| r.id == a).unwrap().parent, None);
            // Refused edits are errors, not changes.
            assert!(
                s.apply_bookmark_edit(
                    &BookmarkEdit::Move {
                        id: f,
                        to: Some(f),
                        index: 0
                    },
                    5
                )
                .await
                .is_err()
            );
            assert!(
                s.apply_bookmark_edit(
                    &BookmarkEdit::Add {
                        parent: None,
                        title: "x".into(),
                        url: "javascript:1".into()
                    },
                    5,
                )
                .await
                .is_err()
            );
            let rows = s
                .apply_bookmark_edit(&BookmarkEdit::Delete { id: f }, 6)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
        });
    }

    #[test]
    fn move_reorders_clamps_and_keeps_positions_dense() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let mut ids = vec![];
            for t in ["a", "b", "c", "d"] {
                ids.push(
                    s.add_bookmark(None, t, &format!("https://{t}.test"), 1)
                        .await
                        .unwrap(),
                );
            }
            s.move_bookmark(ids[3], None, 0, 5).await.unwrap(); // d to the front
            assert_eq!(titles(&s, None).await, ["d", "a", "b", "c"]);
            s.move_bookmark(ids[3], None, 2, 6).await.unwrap(); // later within the same parent
            assert_eq!(titles(&s, None).await, ["a", "b", "d", "c"]);
            s.move_bookmark(ids[0], None, 99, 7).await.unwrap(); // clamped to the end
            assert_eq!(titles(&s, None).await, ["b", "d", "c", "a"]);
            let f = s.add_folder(None, "F", 8).await.unwrap();
            s.move_bookmark(ids[1], Some(f), 5, 9).await.unwrap(); // into a folder, clamped
            s.move_bookmark(ids[2], Some(f), 0, 9).await.unwrap();
            assert_eq!(titles(&s, Some(f)).await, ["c", "b"]);
            assert_eq!(titles(&s, None).await, ["d", "a", "F"]);
            let moved = s.bookmark_children(Some(f)).await.unwrap();
            assert_eq!(moved[0].modified_ms, 9);
            assert_eq!(moved[0].added_ms, 1);
            s.move_bookmark(ids[1], None, 0, 10).await.unwrap(); // back to the top level
            assert_eq!(titles(&s, None).await, ["b", "d", "a", "F"]);
            consistent(&s).await;
            assert!(matches!(
                s.move_bookmark(BookmarkId(999), None, 0, 1).await,
                Err(Error::Invalid(_))
            ));
        });
    }

    #[test]
    fn a_folder_cannot_move_into_itself_or_a_descendant() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = s.add_folder(None, "A", 1).await.unwrap();
            let b = s.add_folder(Some(a), "B", 1).await.unwrap();
            let c = s.add_folder(Some(b), "C", 1).await.unwrap();
            for to in [a, b, c] {
                assert!(matches!(
                    s.move_bookmark(a, Some(to), 0, 2).await,
                    Err(Error::Invalid(_))
                ));
            }
            // A refused move changes nothing.
            assert_eq!(titles(&s, Some(a)).await, ["B"]);
            let link = s
                .add_bookmark(None, "L", "https://l.test", 1)
                .await
                .unwrap();
            assert!(matches!(
                s.move_bookmark(a, Some(link), 0, 2).await,
                Err(Error::Invalid(_))
            ));
            s.move_bookmark(c, None, 0, 3).await.unwrap(); // up and out is fine
            consistent(&s).await;
        });
    }

    #[test]
    fn rename_updates_title_and_modified() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = s
                .add_bookmark(None, "A", "https://a.test", 1)
                .await
                .unwrap();
            s.rename_bookmark(a, "Renamed", 9).await.unwrap();
            let n = &s.bookmark_children(None).await.unwrap()[0];
            assert_eq!(
                (n.title.as_str(), n.added_ms, n.modified_ms),
                ("Renamed", 1, 9)
            );
            assert!(matches!(
                s.rename_bookmark(BookmarkId(999), "x", 1).await,
                Err(Error::Invalid(_))
            ));
        });
    }

    #[test]
    fn delete_cascades_and_closes_the_gap() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = s
                .add_bookmark(None, "a", "https://a.test", 1)
                .await
                .unwrap();
            let f = s.add_folder(None, "f", 1).await.unwrap();
            let c = s
                .add_bookmark(None, "c", "https://c.test", 1)
                .await
                .unwrap();
            s.add_bookmark(Some(f), "x", "https://x.test", 1)
                .await
                .unwrap();
            let g = s.add_folder(Some(f), "g", 1).await.unwrap();
            s.add_bookmark(Some(g), "y", "https://y.test", 1)
                .await
                .unwrap();
            assert_eq!(s.delete_bookmark(f).await.unwrap(), 4);
            assert_eq!(titles(&s, None).await, ["a", "c"]);
            assert_eq!(s.bookmark_children(None).await.unwrap()[1].position, 1);
            assert_eq!(s.bookmark_count().await.unwrap(), 2);
            assert_eq!(s.delete_bookmark(f).await.unwrap(), 0); // already gone
            assert_eq!(s.delete_bookmark(a).await.unwrap(), 1);
            assert_eq!(s.bookmark_children(None).await.unwrap()[0].id, c);
            consistent(&s).await;
        });
    }

    #[test]
    fn search_ignores_case_in_any_script() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.add_bookmark(None, "Привет мир", "https://a.test/", 1)
                .await
                .unwrap();
            s.add_bookmark(None, "Plain", "https://b.test/", 2)
                .await
                .unwrap();
            for query in ["привет", "ПРИВЕТ", "Привет"] {
                let hits = s.search_bookmarks(query, 10).await.unwrap();
                assert_eq!(hits.len(), 1, "query {query:?}");
                assert_eq!(hits[0].url, "https://a.test/");
            }
        });
    }

    #[test]
    fn search_matches_title_and_url_and_ranks_prefixes_first() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let f = s.add_folder(None, "F", 1).await.unwrap();
            s.add_bookmark(Some(f), "Rust docs", "https://docs.test/rust", 1)
                .await
                .unwrap();
            s.add_bookmark(None, "Rust", "https://www.rust-lang.org/", 3)
                .await
                .unwrap();
            s.add_bookmark(None, "Other", "https://example.com/rustic", 2)
                .await
                .unwrap();
            s.add_bookmark(None, "Rust again", "https://www.rust-lang.org/", 4)
                .await
                .unwrap(); // same url
            let urls = |v: Vec<Bookmark>| v.into_iter().map(|b| b.url).collect::<Vec<_>>();
            // url prefix (www. ignored) first, then title prefix, then contains; one per url.
            assert_eq!(
                urls(s.search_bookmarks("rust", 10).await.unwrap()),
                [
                    "https://www.rust-lang.org/",
                    "https://docs.test/rust",
                    "https://example.com/rustic",
                ]
            );
            assert_eq!(
                urls(s.search_bookmarks("https://www.rust", 10).await.unwrap()),
                ["https://www.rust-lang.org/"]
            );
            // Scheme and www. are ignored on both sides.
            assert_eq!(
                urls(s.search_bookmarks("www.rust-l", 10).await.unwrap()),
                ["https://www.rust-lang.org/"]
            );
            assert_eq!(
                urls(s.search_bookmarks("http://docs.te", 10).await.unwrap()),
                ["https://docs.test/rust"]
            );
            assert_eq!(
                urls(s.search_bookmarks("RUST-LANG", 10).await.unwrap()).len(),
                1
            );
            assert_eq!(s.search_bookmarks("rust", 1).await.unwrap().len(), 1);
            assert!(s.search_bookmarks("", 10).await.unwrap().is_empty());
            assert!(s.search_bookmarks("  ", 10).await.unwrap().is_empty());
            // Folders are never candidates.
            assert!(s.search_bookmarks("F", 10).await.unwrap().is_empty());
        });
    }

    #[test]
    fn search_wildcards_are_literal() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.add_bookmark(None, "100% sure", "https://a.test/a_b", 1)
                .await
                .unwrap();
            s.add_bookmark(None, "1000 sure", "https://a.test/axb", 1)
                .await
                .unwrap();
            assert_eq!(s.search_bookmarks("100%", 10).await.unwrap().len(), 1);
            assert_eq!(s.search_bookmarks("a_b", 10).await.unwrap().len(), 1);
        });
    }
}
