//! Ordered migrations. The applied version lives in the database itself (`PRAGMA user_version`).

use crate::{Error, Result};

/// `(version, SQL)`, strictly ascending from 1. Append only; never edit a shipped entry.
pub const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../sql/001_init.sql")),
    (2, include_str!("../sql/002_core_model.sql")),
    (3, include_str!("../sql/003_logins_imports.sql")),
    (4, include_str!("../sql/004_sidebar_mode.sql")),
    (5, include_str!("../sql/005_site_permissions.sql")),
    (6, include_str!("../sql/006_downloads.sql")),
    (7, include_str!("../sql/007_splits.sql")),
    (8, include_str!("../sql/008_favicons.sql")),
    (9, include_str!("../sql/009_logins_vault_shape.sql")),
    (10, include_str!("../sql/010_space_icons.sql")),
    (11, include_str!("../sql/011_windows.sql")),
    (12, include_str!("../sql/012_archive.sql")),
    (13, include_str!("../sql/013_boosts.sql")),
    (14, include_str!("../sql/014_agent_log.sql")),
    (15, include_str!("../sql/015_agent_spaces.sql")),
];

pub fn latest(migrations: &[(i64, &str)]) -> i64 {
    migrations.last().map_or(0, |(v, _)| *v)
}

pub async fn schema_version(conn: &turso::Connection) -> Result<i64> {
    let mut rows = conn.query("PRAGMA user_version", ()).await?;
    match rows.next().await? {
        Some(row) => Ok(row.get::<i64>(0)?),
        None => Ok(0),
    }
}

/// Apply every migration newer than the stored version, each in its own transaction.
/// A newer database is refused, never downgraded.
pub async fn migrate(conn: &mut turso::Connection, migrations: &[(i64, &str)]) -> Result<()> {
    let found = schema_version(conn).await?;
    let supported = latest(migrations);
    if found > supported {
        return Err(Error::DatabaseTooNew { found, supported });
    }
    for (version, sql) in migrations.iter().filter(|(v, _)| *v > found) {
        let tx = conn.transaction().await?;
        tx.execute_batch(sql).await?;
        // `version` is a constant integer from the table above, never user input.
        tx.execute_batch(format!("PRAGMA user_version = {version}"))
            .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem() -> turso::Connection {
        let db = turso::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        // Leak the database handle for the test's lifetime; the connection keeps it usable.
        std::mem::forget(db);
        conn
    }

    #[test]
    fn migrations_are_ascending() {
        for (i, (v, _)) in MIGRATIONS.iter().enumerate() {
            assert_eq!(*v, i as i64 + 1);
        }
    }

    #[test]
    fn from_empty_reaches_latest_with_tables() {
        pollster::block_on(async {
            let mut conn = mem().await;
            assert_eq!(schema_version(&conn).await.unwrap(), 0);
            migrate(&mut conn, MIGRATIONS).await.unwrap();
            assert_eq!(schema_version(&conn).await.unwrap(), latest(MIGRATIONS));
            for table in [
                "spaces",
                "containers",
                "tabs",
                "tab_folders",
                "session",
                "urls",
                "visits",
                "bookmarks",
                "settings",
                "recently_closed",
                "logins",
                "import_marks",
                "site_permissions",
                "downloads",
                "favicons",
                "favicon_origins",
                "windows",
                "archive",
                "boosts",
                "agent_log",
            ] {
                let mut rows = conn
                    .query(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name=?1",
                        [table],
                    )
                    .await
                    .unwrap();
                assert!(
                    rows.next().await.unwrap().is_some(),
                    "missing table {table}"
                );
            }
        });
    }

    #[test]
    fn is_idempotent_and_keeps_data() {
        pollster::block_on(async {
            let mut conn = mem().await;
            migrate(&mut conn, MIGRATIONS).await.unwrap();
            conn.execute("INSERT INTO settings(key, value) VALUES ('a', 'b')", ())
                .await
                .unwrap();
            migrate(&mut conn, MIGRATIONS).await.unwrap();
            let mut rows = conn
                .query("SELECT value FROM settings WHERE key='a'", ())
                .await
                .unwrap();
            let row = rows.next().await.unwrap().unwrap();
            assert_eq!(row.get::<String>(0).unwrap(), "b");
        });
    }

    async fn count(conn: &turso::Connection, sql: &str) -> i64 {
        let mut rows = conn.query(sql, ()).await.unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    }

    /// A database of the previous schema, with data, reaches the latest
    /// with its rows intact and the new tables empty.
    #[test]
    fn a_v4_database_with_data_migrates_to_the_latest() {
        pollster::block_on(async {
            let mut conn = mem().await;
            migrate(&mut conn, &MIGRATIONS[..4]).await.unwrap();
            assert_eq!(schema_version(&conn).await.unwrap(), 4);
            conn.execute_batch(
                "INSERT INTO settings(key, value) VALUES ('ui.sidebar_mode', 'rail');
                 INSERT INTO containers(id, name, position) VALUES (0, 'Default', 0);
                 INSERT INTO spaces(id, name, position, container_id) VALUES (0, 'Home', 0, 0);
                 INSERT INTO tabs(id, space_id, container_id, position, url)
                     VALUES (1, 0, 0, 0, 'https://a.test/');
                 INSERT INTO bookmarks(id, kind, position, title, url, date_added, date_modified)
                     VALUES (1, 0, 0, 'A', 'https://a.test/', 1, 1);",
            )
            .await
            .unwrap();
            migrate(&mut conn, MIGRATIONS).await.unwrap();
            assert_eq!(schema_version(&conn).await.unwrap(), latest(MIGRATIONS));
            for (sql, want) in [
                ("SELECT count(*) FROM settings WHERE value = 'rail'", 1),
                ("SELECT count(*) FROM tabs WHERE url = 'https://a.test/'", 1),
                ("SELECT count(*) FROM bookmarks WHERE title = 'A'", 1),
                ("SELECT count(*) FROM site_permissions", 0),
                ("SELECT count(*) FROM downloads", 0),
                ("SELECT count(*) FROM split_panes", 0),
                ("SELECT count(*) FROM archive", 0),
                ("SELECT count(*) FROM tabs WHERE last_active = 0", 1),
                ("SELECT count(*) FROM spaces WHERE never_sleep = 0", 1),
            ] {
                assert_eq!(count(&conn, sql).await, want, "{sql}");
            }
        });
    }

    #[test]
    fn a_v11_database_with_data_gets_the_archive_columns() {
        pollster::block_on(async {
            let mut conn = mem().await;
            migrate(&mut conn, &MIGRATIONS[..11]).await.unwrap();
            conn.execute_batch(
                "INSERT INTO containers(id, name, position) VALUES (0, 'Default', 0);
                 INSERT INTO spaces(id, name, position, container_id) VALUES (0, 'Home', 0, 0);
                 INSERT INTO tabs(id, space_id, container_id, position, url)
                     VALUES (1, 0, 0, 0, 'https://a.test/');",
            )
            .await
            .unwrap();
            migrate(&mut conn, MIGRATIONS).await.unwrap();
            assert_eq!(schema_version(&conn).await.unwrap(), latest(MIGRATIONS));
            for (sql, want) in [
                (
                    "SELECT count(*) FROM tabs WHERE url = 'https://a.test/' AND last_active = 0",
                    1,
                ),
                (
                    "SELECT count(*) FROM spaces WHERE name = 'Home' AND never_sleep = 0",
                    1,
                ),
                ("SELECT count(*) FROM archive", 0),
                ("SELECT count(*) FROM boosts", 0),
            ] {
                assert_eq!(count(&conn, sql).await, want, "{sql}");
            }
        });
    }

    #[test]
    fn failed_step_rolls_back() {
        pollster::block_on(async {
            let mut conn = mem().await;
            let list = [
                (1, "CREATE TABLE t1(x INTEGER)"),
                (2, "CREATE TABLE a(x INTEGER); THIS IS NOT SQL"),
            ];
            assert!(migrate(&mut conn, &list).await.is_err());
            assert_eq!(schema_version(&conn).await.unwrap(), 1);
            let mut rows = conn
                .query("SELECT name FROM sqlite_master WHERE name='a'", ())
                .await
                .unwrap();
            assert!(rows.next().await.unwrap().is_none());
        });
    }

    #[test]
    fn rejects_newer_database() {
        pollster::block_on(async {
            let mut conn = mem().await;
            conn.execute_batch(format!("PRAGMA user_version = {}", latest(MIGRATIONS) + 1))
                .await
                .unwrap();
            let err = migrate(&mut conn, MIGRATIONS).await.unwrap_err();
            assert!(matches!(err, Error::DatabaseTooNew { .. }));
        });
    }
}
