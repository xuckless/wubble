//! Bookmarks and history from a Firefox/Zen `places.sqlite`, given as bytes.
//!
//! Schema: mozilla-firefox/firefox `toolkit/components/places/nsPlacesTables.h` (`moz_places`,
//! `moz_historyvisits`, `moz_bookmarks`), bookmark types and root guids from
//! `Bookmarks.sys.mjs`, visit types from `nsINavHistoryService.idl` (MPL-2.0). The two queries
//! and the root handling are ported from zephium-browser/Zephium
//! `crates/zephium-import/src/firefox.rs` (`bookmarks`, `history`; MPL-2.0).
//!
//! Firefox databases are in WAL mode (header bytes 18 and 19 are 2) and SQLite refuses to
//! deserialize such an image, so the parser patches those two bytes to 1 (rollback journal) on
//! its own copy. The image must already contain the WAL content: `discover::snapshot_places`
//! does that for a live profile.

use crate::{BookmarkNode, ImportError, ImportedVisit, is_web_url};
use rusqlite::{Connection, MAIN_DB, params};
use std::collections::HashMap;
use std::io::Cursor;

const MAX_BOOKMARKS: i64 = 200_000;
const MAX_VISITS: i64 = 1_000_000;
const MAX_DEPTH: usize = 32;
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// Root guids in display order (`Bookmarks.sys.mjs` `rootGuid`, `menuGuid`, ...).
const ROOTS: &[(&str, &str)] = &[
    ("toolbar_____", "Bookmarks Toolbar"),
    ("menu________", "Bookmarks Menu"),
    ("unfiled_____", "Other Bookmarks"),
    ("mobile______", "Mobile Bookmarks"),
];

const TYPE_BOOKMARK: i64 = 1;
const TYPE_FOLDER: i64 = 2;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    /// One folder per non-empty root.
    pub bookmarks: Vec<BookmarkNode>,
    /// Newest first.
    pub history: Vec<ImportedVisit>,
}

struct Row {
    id: i64,
    kind: i64,
    title: String,
    added_ms: i64,
    guid: String,
    url: Option<String>,
}

pub fn parse(bytes: &[u8]) -> Result<Places, ImportError> {
    if bytes.len() < 100 || !bytes.starts_with(SQLITE_MAGIC) {
        return Err(ImportError::Unreadable("not a SQLite database"));
    }
    let mut image = bytes.to_vec();
    if image[18] == 2 && image[19] == 2 {
        image[18] = 1;
        image[19] = 1;
    }
    let mut db = Connection::open_in_memory()?;
    db.deserialize_read_exact(MAIN_DB, Cursor::new(&image), image.len(), true)?;
    Ok(Places {
        bookmarks: bookmarks(&db)?,
        history: history(&db)?,
    })
}

fn bookmarks(db: &Connection) -> Result<Vec<BookmarkNode>, ImportError> {
    let mut stmt = db.prepare(
        "SELECT b.id, b.parent, b.type, b.title, b.dateAdded, b.guid, p.url
         FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk
         ORDER BY b.parent, b.position LIMIT ?1",
    )?;
    let mut rows = Vec::new();
    let mut children: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut q = stmt.query(params![MAX_BOOKMARKS])?;
    while let Some(r) = q.next()? {
        children.entry(r.get(1)?).or_default().push(rows.len());
        rows.push(Row {
            id: r.get(0)?,
            kind: r.get(2)?,
            title: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            added_ms: r.get::<_, Option<i64>>(4)?.unwrap_or(0) / 1000,
            guid: r.get(5)?,
            url: r.get(6)?,
        });
    }

    fn nodes(
        parent: i64,
        depth: usize,
        rows: &[Row],
        children: &HashMap<i64, Vec<usize>>,
    ) -> Vec<BookmarkNode> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        let kids = children.get(&parent).map(Vec::as_slice).unwrap_or(&[]);
        kids.iter()
            .filter_map(|&i| {
                let r = &rows[i];
                match (r.kind, &r.url) {
                    (TYPE_BOOKMARK, Some(url)) if !url.starts_with("place:") => {
                        Some(BookmarkNode::Link {
                            title: r.title.clone(),
                            url: url.clone(),
                            added_ms: r.added_ms,
                        })
                    }
                    (TYPE_FOLDER, _) => Some(BookmarkNode::Folder {
                        title: r.title.clone(),
                        children: nodes(r.id, depth + 1, rows, children),
                    }),
                    // separators (type 3), smart queries, links without a place
                    _ => None,
                }
            })
            .collect()
    }

    Ok(ROOTS
        .iter()
        .filter_map(|(guid, title)| {
            let root = rows.iter().find(|r| r.guid == *guid)?;
            let kids = nodes(root.id, 1, &rows, &children);
            (!kids.is_empty()).then(|| BookmarkNode::Folder {
                title: (*title).to_owned(),
                children: kids,
            })
        })
        .collect())
}

fn history(db: &Connection) -> Result<Vec<ImportedVisit>, ImportError> {
    // visit_type: TYPED = 2; skip EMBED 4, DOWNLOAD 7, FRAMED_LINK 8, RELOAD 9.
    let mut stmt = db.prepare(
        "SELECT p.url, p.title, v.visit_date, v.visit_type
         FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id
         WHERE p.hidden = 0 AND v.visit_type NOT IN (4, 7, 8, 9)
         ORDER BY v.visit_date DESC LIMIT ?1",
    )?;
    let mut out = Vec::new();
    let mut q = stmt.query(params![MAX_VISITS])?;
    while let Some(r) = q.next()? {
        let url: String = r.get(0)?;
        if !is_web_url(&url) {
            continue;
        }
        out.push(ImportedVisit {
            url,
            title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            at_ms: r.get::<_, i64>(2)? / 1000,
            typed: r.get::<_, i64>(3)? == 2,
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Columns from `nsPlacesTables.h`, reduced to what we read plus a few neighbours.
    const SCHEMA: &str = "
        CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR,
            visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL,
            typed INTEGER DEFAULT 0 NOT NULL, last_visit_date INTEGER, guid TEXT);
        CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, from_visit INTEGER,
            place_id INTEGER, visit_date INTEGER, visit_type INTEGER, session INTEGER);
        CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER DEFAULT NULL,
            parent INTEGER, position INTEGER, title LONGVARCHAR, dateAdded INTEGER,
            lastModified INTEGER, guid TEXT);
        INSERT INTO moz_places (id, url, title, hidden) VALUES
            (1, 'https://a.example/', 'A', 0),
            (2, 'https://b.example/', 'B', 0),
            (3, 'https://hidden.example/', 'H', 1),
            (4, 'place:type=6&sort=14', 'Smart', 0),
            (5, 'https://embed.example/', 'E', 0),
            (6, 'about:preferences', 'Prefs', 0);
        INSERT INTO moz_historyvisits (place_id, visit_date, visit_type) VALUES
            (1, 1000000, 1), (2, 3000000, 2), (3, 4000000, 1), (5, 5000000, 4),
            (1, 2000000, 9), (6, 6000000, 1);
        INSERT INTO moz_bookmarks (id, type, fk, parent, position, title, dateAdded, guid) VALUES
            (1, 2, NULL, 0, 0, 'root', 0, 'root________'),
            (2, 2, NULL, 1, 0, 'menu', 0, 'menu________'),
            (3, 2, NULL, 1, 1, 'toolbar', 0, 'toolbar_____'),
            (4, 2, NULL, 1, 2, 'unfiled', 0, 'unfiled_____'),
            (5, 2, NULL, 1, 3, 'mobile', 0, 'mobile______'),
            (6, 1, 1, 3, 0, 'A bookmark', 7000000, 'g6'),
            (7, 2, NULL, 3, 1, 'Dev', 0, 'g7'),
            (8, 1, 2, 7, 0, 'B bookmark', 8000000, 'g8'),
            (9, 3, NULL, 7, 1, NULL, 0, 'g9'),
            (10, 1, 4, 7, 2, 'Smart query', 0, 'g10'),
            (11, 1, NULL, 3, 2, 'No place', 0, 'g11'),
            (12, 1, 2, 2, 0, 'Menu link', 0, 'g12');
    ";

    /// An in-memory database image with the test data (journal mode: rollback).
    pub(crate) fn image() -> Vec<u8> {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db.serialize(MAIN_DB).unwrap().to_vec()
    }

    #[test]
    fn parses_tree_and_visits() {
        let p = parse(&image()).unwrap();
        let link = |t: &str, u: &str, ms| BookmarkNode::Link {
            title: t.into(),
            url: u.into(),
            added_ms: ms,
        };
        assert_eq!(
            p.bookmarks,
            [
                BookmarkNode::Folder {
                    title: "Bookmarks Toolbar".into(),
                    children: vec![
                        link("A bookmark", "https://a.example/", 7000),
                        BookmarkNode::Folder {
                            title: "Dev".into(),
                            children: vec![link("B bookmark", "https://b.example/", 8000)],
                        },
                    ],
                },
                BookmarkNode::Folder {
                    title: "Bookmarks Menu".into(),
                    children: vec![link("Menu link", "https://b.example/", 0)],
                },
            ]
        );
        // newest first; hidden place, EMBED, RELOAD and about: dropped
        let v: Vec<_> = p
            .history
            .iter()
            .map(|v| (v.url.as_str(), v.at_ms, v.typed))
            .collect();
        assert_eq!(
            v,
            [
                ("https://b.example/", 3000, true),
                ("https://a.example/", 1000, false)
            ]
        );
    }

    #[test]
    fn wal_mode_file_with_header_2_still_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("places.sqlite");
        {
            let db = Connection::open(&path).unwrap();
            db.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
                .unwrap();
            db.execute_batch(SCHEMA).unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!((bytes[18], bytes[19]), (2, 2));
        assert_eq!(parse(&bytes).unwrap().history.len(), 2);
    }

    #[test]
    fn rejects_non_sqlite_and_truncated() {
        assert!(matches!(parse(b"hello"), Err(ImportError::Unreadable(_))));
        let img = image();
        assert!(parse(&img[..img.len() / 2]).is_err());
    }

    #[test]
    fn bookmark_depth_is_capped() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db.execute_batch("DELETE FROM moz_bookmarks WHERE id > 5")
            .unwrap();
        for i in 0..40 {
            db.execute(
                "INSERT INTO moz_bookmarks (id, type, parent, position, title, guid)
                 VALUES (?1, 2, ?2, 0, 'f', ?3)",
                params![100 + i, if i == 0 { 3 } else { 99 + i }, format!("d{i}")],
            )
            .unwrap();
        }
        let p = parse(&db.serialize(MAIN_DB).unwrap()).unwrap();
        fn depth(n: &BookmarkNode) -> usize {
            match n {
                BookmarkNode::Folder { children, .. } => {
                    1 + children.iter().map(depth).max().unwrap_or(0)
                }
                _ => 0,
            }
        }
        assert_eq!(depth(&p.bookmarks[0]), MAX_DEPTH + 1);
    }
}
