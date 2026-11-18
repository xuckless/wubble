//! Consistency check for tests (and a debug aid): foreign keys valid, no orphan rows,
//! denormalised counters equal what the rows say.
//!
//! turso 0.9.0-pre.5 has no `PRAGMA foreign_key_check` (no `ForeignKeyCheck` in
//! `turso_parser`'s `PragmaName`), so every relation is an explicit LEFT JOIN or NOT EXISTS
//! query. Each entry returns the ids of the offending rows.

use std::collections::HashMap;

use crate::{Result, Store, ids};

/// (what is wrong, SQL returning the offending ids).
const CHECKS: &[(&str, &str)] = &[
    (
        "visit without url",
        "SELECT v.id FROM visits v LEFT JOIN urls u ON u.id = v.url_id WHERE u.id IS NULL",
    ),
    (
        "tab in missing space",
        "SELECT t.id FROM tabs t LEFT JOIN spaces s ON s.id = t.space_id WHERE s.id IS NULL",
    ),
    (
        "tab with missing container",
        "SELECT t.id FROM tabs t LEFT JOIN containers c ON c.id = t.container_id WHERE c.id IS NULL",
    ),
    (
        "tab in missing folder",
        "SELECT t.id FROM tabs t LEFT JOIN tab_folders f ON f.id = t.folder_id \
         WHERE t.folder_id IS NOT NULL AND f.id IS NULL",
    ),
    (
        "tab in a folder of another space",
        "SELECT t.id FROM tabs t JOIN tab_folders f ON f.id = t.folder_id WHERE f.space_id <> t.space_id",
    ),
    (
        "pinned tab in a folder",
        "SELECT id FROM tabs WHERE pinned <> 0 AND folder_id IS NOT NULL",
    ),
    (
        "folder in missing space",
        "SELECT f.id FROM tab_folders f LEFT JOIN spaces s ON s.id = f.space_id WHERE s.id IS NULL",
    ),
    (
        "space with missing container",
        "SELECT s.id FROM spaces s LEFT JOIN containers c ON c.id = s.container_id WHERE c.id IS NULL",
    ),
    (
        "space whose active tab is not one of its tabs",
        "SELECT s.id FROM spaces s WHERE s.active_tab_id IS NOT NULL AND NOT EXISTS \
         (SELECT 1 FROM tabs t WHERE t.id = s.active_tab_id AND t.space_id = s.id)",
    ),
    (
        "session whose active space is missing",
        "SELECT s.id FROM session s WHERE s.active_space_id IS NOT NULL AND NOT EXISTS \
         (SELECT 1 FROM spaces p WHERE p.id = s.active_space_id)",
    ),
    (
        "bookmark with missing parent",
        "SELECT b.id FROM bookmarks b LEFT JOIN bookmarks p ON p.id = b.parent_id \
         WHERE b.parent_id IS NOT NULL AND p.id IS NULL",
    ),
    (
        "bookmark whose parent is not a folder",
        "SELECT b.id FROM bookmarks b JOIN bookmarks p ON p.id = b.parent_id WHERE p.kind <> 1",
    ),
    (
        "bookmark link without url",
        "SELECT id FROM bookmarks WHERE kind = 0 AND url IS NULL",
    ),
    (
        "bookmark folder with a url",
        "SELECT id FROM bookmarks WHERE kind = 1 AND url IS NOT NULL",
    ),
    (
        "bookmark of unknown kind",
        "SELECT id FROM bookmarks WHERE kind NOT IN (0, 1)",
    ),
    (
        // The id reported is the parent's (-1 for the root level).
        "bookmark siblings with non-dense positions (parent id)",
        "SELECT COALESCE(parent_id, -1) FROM bookmarks GROUP BY parent_id \
         HAVING MIN(position) <> 0 OR MAX(position) <> COUNT(*) - 1 OR COUNT(DISTINCT position) <> COUNT(*)",
    ),
    (
        "url whose counters or last visit differ from its visits",
        "SELECT u.id FROM urls u WHERE \
           u.visit_count <> (SELECT COUNT(*) FROM visits v WHERE v.url_id = u.id) \
           OR u.typed_count <> (SELECT COUNT(*) FROM visits v WHERE v.url_id = u.id AND v.transition = 1) \
           OR u.last_visit_time <> COALESCE((SELECT MAX(v.visit_time) FROM visits v WHERE v.url_id = u.id), -1)",
    ),
    (
        "url without visits",
        "SELECT u.id FROM urls u WHERE NOT EXISTS (SELECT 1 FROM visits v WHERE v.url_id = u.id)",
    ),
    (
        "login with negative use count",
        "SELECT rowid FROM logins WHERE times_used < 0",
    ),
];

impl Store {
    /// One line per violation; empty means consistent. Read only.
    pub async fn check_consistency(&self) -> Result<Vec<String>> {
        let mut problems = Vec::new();
        for (what, sql) in CHECKS {
            for id in ids(&self.conn, sql, ()).await? {
                problems.push(format!("{what}: {id}"));
            }
        }
        // Cycles: walk each node's parent chain; a chain longer than the node count loops.
        let mut parent: HashMap<i64, Option<i64>> = HashMap::new();
        let mut rows = self
            .conn
            .query("SELECT id, parent_id FROM bookmarks", ())
            .await?;
        while let Some(r) = rows.next().await? {
            parent.insert(r.get(0)?, r.get(1)?);
        }
        let mut in_cycle: Vec<i64> = parent
            .keys()
            .copied()
            .filter(|&start| {
                let mut at = parent[&start];
                for _ in 0..parent.len() {
                    match at {
                        None => return false,
                        Some(p) if p == start => return true,
                        Some(p) => at = parent.get(&p).copied().flatten(),
                    }
                }
                at.is_some() // chain never ended but never returned to `start`: hangs off a cycle
            })
            .collect();
        in_cycle.sort_unstable();
        problems.extend(
            in_cycle
                .iter()
                .map(|id| format!("bookmark in a cycle: {id}")),
        );
        Ok(problems)
    }
}

#[cfg(test)]
mod tests {
    /// `CHECKS` is an independent oracle written with literals; pin what they assume.
    #[test]
    fn check_sql_literals_match_the_discriminants() {
        assert_eq!(crate::Transition::Typed as i64, 1);
        assert_eq!(crate::BookmarkKind::Link as i64, 0);
        assert_eq!(crate::BookmarkKind::Folder as i64, 1);
    }

    use wubble_core::model::{Command, Model, apply};

    use super::*;
    use crate::Transition;
    use crate::migrate::{self, MIGRATIONS};

    fn run<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    /// A consistent database touching every checked table.
    async fn seeded() -> Store {
        let mut s = Store::open_in_memory().await.unwrap();
        let m = apply(
            Model::new(std::time::Instant::now()),
            Command::NewTab {
                url: "https://mine.test".into(),
                container: None,
                opener: None,
            },
        )
        .0;
        s.save_session(&m).await.unwrap();
        s.record_visit("https://a.test/", "A", 10, Transition::Typed)
            .await
            .unwrap();
        s.record_visit("https://a.test/", "A", 20, Transition::Link)
            .await
            .unwrap();
        let f = s.add_folder(None, "F", 1).await.unwrap();
        let g = s.add_folder(Some(f), "G", 1).await.unwrap();
        s.add_bookmark(Some(g), "L", "https://l.test", 1)
            .await
            .unwrap();
        s.add_bookmark(None, "R", "https://r.test", 1)
            .await
            .unwrap();
        s
    }

    #[test]
    fn a_consistent_database_has_no_problems() {
        run(async {
            let s = seeded().await;
            assert_eq!(s.check_consistency().await.unwrap(), Vec::<String>::new());
        });
    }

    /// Plant `sql` (foreign keys off, so orphans can be made) and expect a problem mentioning `what`.
    async fn detects(sql: &str, what: &str) {
        let s = seeded().await;
        s.conn
            .execute_batch("PRAGMA foreign_keys = OFF")
            .await
            .unwrap();
        s.conn.execute_batch(sql).await.unwrap();
        let problems = s.check_consistency().await.unwrap();
        assert!(
            problems.iter().any(|p| p.contains(what)),
            "{what:?} not reported for {sql}: {problems:?}"
        );
    }

    #[test]
    fn planted_faults_are_reported() {
        run(async {
            detects(
                "INSERT INTO visits(url_id, visit_time, transition) VALUES (999, 1, 0)",
                "visit without url",
            )
            .await;
            detects("UPDATE urls SET visit_count = 7", "counters").await;
            detects("UPDATE urls SET typed_count = 0", "counters").await;
            detects("UPDATE urls SET last_visit_time = 1", "counters").await;
            detects(
                "INSERT INTO urls(url, url_key, last_visit_time) VALUES ('https://empty.test/', 'empty.test/', 1)",
                "url without visits",
            )
            .await;
            detects(
                "INSERT INTO bookmarks(parent_id, kind, position, title, url, date_added, date_modified) \
                 VALUES (NULL, 0, 2, 'bad', NULL, 1, 1)",
                "link without url",
            )
            .await;
            detects(
                "INSERT INTO bookmarks(parent_id, kind, position, title, url, date_added, date_modified) \
                 VALUES (NULL, 1, 2, 'bad', 'https://x.test', 1, 1)",
                "folder with a url",
            )
            .await;
            detects(
                "UPDATE bookmarks SET position = position + 5 WHERE title = 'R'",
                "non-dense",
            )
            .await;
            detects(
                "UPDATE bookmarks SET position = 0 WHERE title = 'R'",
                "non-dense",
            )
            .await;
            detects(
                "UPDATE bookmarks SET parent_id = 999 WHERE title = 'R'",
                "missing parent",
            )
            .await;
            detects(
                "UPDATE bookmarks SET parent_id = (SELECT id FROM bookmarks WHERE title = 'R') WHERE title = 'L'",
                "parent is not a folder",
            )
            .await;
            // F contains G; making G the parent of F closes a loop.
            detects(
                "UPDATE bookmarks SET parent_id = (SELECT id FROM bookmarks WHERE title = 'G') WHERE title = 'F'",
                "in a cycle",
            )
            .await;
            detects("UPDATE tabs SET space_id = 99", "tab in missing space").await;
            detects("UPDATE tabs SET container_id = 99", "missing container").await;
            detects("UPDATE tabs SET folder_id = 99", "missing folder").await;
            detects(
                "UPDATE spaces SET container_id = 99",
                "space with missing container",
            )
            .await;
            detects("UPDATE spaces SET active_tab_id = 99", "active tab").await;
            detects("UPDATE session SET active_space_id = 99", "active space").await;
            detects(
                "INSERT INTO tab_folders(id, space_id, name, position) VALUES (5, 99, 'x', 0)",
                "folder in missing space",
            )
            .await;
        });
    }

    #[test]
    fn a_v2_database_with_data_migrates_to_the_latest_schema() {
        run(async {
            let mut s = Store::connect(":memory:").await.unwrap();
            migrate::migrate(&mut s.conn, &MIGRATIONS[..2])
                .await
                .unwrap();
            assert!(s.conn.query("SELECT * FROM logins", ()).await.is_err());
            s.conn
                .execute_batch(
                    "INSERT INTO containers(id, name, position) VALUES (0, 'Default', 0);
                     INSERT INTO spaces(id, name, position, tint, container_id, active_tab_id) VALUES
                        (0, 'Home', 0, '#112233', 0, 5);
                     INSERT INTO tabs(id, space_id, container_id, folder_id, pinned, position, url, title) VALUES
                        (5, 0, 0, NULL, 0, 0, 'https://t.test', 'T');
                     INSERT INTO session(id, active_space_id) VALUES (1, 0);
                     INSERT INTO urls(id, url, url_key, title, visit_count, typed_count, last_visit_time) VALUES
                        (1, 'https://u.test/', 'u.test/', 'U', 2, 1, 30);
                     INSERT INTO visits(url_id, visit_time, transition) VALUES (1, 10, 1), (1, 30, 0);
                     INSERT INTO bookmarks(id, parent_id, kind, position, title, url, date_added, date_modified) VALUES
                        (1, NULL, 1, 0, 'Folder', NULL, 1, 1),
                        (2, 1, 0, 0, 'Link', 'https://b.test', 1, 1);
                     INSERT INTO settings(key, value) VALUES ('ui.compact', '1');
                     INSERT INTO recently_closed(closed_at, space_id, container_id, url, title)
                        VALUES (9, 0, 0, 'https://c.test', 'C');",
                )
                .await
                .unwrap();
            migrate::migrate(&mut s.conn, MIGRATIONS).await.unwrap();
            assert_eq!(migrate::schema_version(&s.conn).await.unwrap(), 4);

            // Old data intact and consistent.
            assert_eq!(s.check_consistency().await.unwrap(), Vec::<String>::new());
            let m = s.load_session(std::time::Instant::now()).await.unwrap();
            assert_eq!(m.tabs.len(), 1);
            assert_eq!(
                s.query_history("u.test", 100, 5).await.unwrap()[0].visit_count,
                2
            );
            assert_eq!(s.bookmark_children(None).await.unwrap()[0].title, "Folder");
            assert_eq!(s.search_bookmarks("b.test", 5).await.unwrap().len(), 1);
            assert!(s.setting::<crate::CompactMode>().await.unwrap());
            assert_eq!(s.recently_closed().await.unwrap().len(), 1);

            // New tables work, and old paths keep working on the migrated file.
            assert!(s.all_logins().await.unwrap().is_empty());
            s.add_bookmark(None, "New", "https://new.test", 5)
                .await
                .unwrap();
            s.record_visit("https://u.test/", "U", 40, Transition::Link)
                .await
                .unwrap();
            s.save_session(&m).await.unwrap();
            assert_eq!(s.check_consistency().await.unwrap(), Vec::<String>::new());
        });
    }
}
