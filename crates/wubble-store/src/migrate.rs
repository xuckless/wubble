//! Ordered migrations. The applied version lives in the database itself (`PRAGMA user_version`).

use crate::{Error, Result};

/// `(version, SQL)`, strictly ascending from 1. Append only; never edit a shipped entry.
pub const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../sql/001_init.sql")),
    (2, include_str!("../sql/002_core_model.sql")),
    (3, include_str!("../sql/003_logins_imports.sql")),
    (4, include_str!("../sql/004_logins_vault_shape.sql")),
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
