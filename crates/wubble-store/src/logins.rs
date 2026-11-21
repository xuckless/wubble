//! Password-vault rows: opaque encrypted blobs plus plaintext metadata. No crypto here and no
//! vault types: the vault crate adapts its storage trait to these plain functions
//! (design: Chromium `login_database.cc`, Firefox `LoginManagerStorage`; no code copied).
//!
//! The id is a text key the vault generates before the row exists, because it is part of the
//! AEAD associated data of both blobs: the store never makes one.

use crate::{Result, Store, blob, narrow};

/// A stored login. The blobs are whatever the vault wrote (`nonce || ciphertext || tag`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRow {
    pub id: String,
    /// `scheme://host[:port]`, plaintext so the vault can match without decrypting.
    pub origin: String,
    /// `None`: HTTP auth or no form.
    pub form_action_origin: Option<String>,
    pub username_enc: Vec<u8>,
    pub password_enc: Vec<u8>,
    pub created_ms: i64,
    /// `None`: never used.
    pub last_used_ms: Option<i64>,
    pub times_used: u32,
}

const COLUMNS: &str = "id, origin, form_action_origin, username_enc, password_enc, \
    created_ms, last_used_ms, times_used FROM logins";

fn row(r: &turso::Row) -> Result<LoginRow> {
    Ok(LoginRow {
        id: r.get(0)?,
        origin: r.get(1)?,
        form_action_origin: r.get(2)?,
        username_enc: blob(r, 3)?,
        password_enc: blob(r, 4)?,
        created_ms: r.get(5)?,
        last_used_ms: r.get(6)?,
        times_used: narrow(r.get(7)?, "times_used")?,
    })
}

impl Store {
    /// Insert a full row. An id that exists is a constraint error (`Error::Db`).
    pub async fn insert_login(&mut self, login: LoginRow) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO logins(id, origin, form_action_origin, username_enc, password_enc, \
                 created_ms, last_used_ms, times_used) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    login.id,
                    login.origin,
                    login.form_action_origin,
                    login.username_enc,
                    login.password_enc,
                    login.created_ms,
                    login.last_used_ms,
                    i64::from(login.times_used),
                ),
            )
            .await?;
        Ok(())
    }

    pub async fn login(&self, id: &str) -> Result<Option<LoginRow>> {
        let mut rows = self
            .conn
            .query(&format!("SELECT {COLUMNS} WHERE id = ?1"), [id])
            .await?;
        rows.next().await?.map(|r| row(&r)).transpose()
    }

    /// Logins of exactly this origin, oldest first.
    pub async fn logins_for_origin(&self, origin: &str) -> Result<Vec<LoginRow>> {
        self.logins_where("WHERE origin = ?1 ORDER BY created_ms, id", [origin])
            .await
    }

    pub async fn all_logins(&self) -> Result<Vec<LoginRow>> {
        self.logins_where("ORDER BY created_ms, id", ()).await
    }

    async fn logins_where(
        &self,
        tail: &str,
        params: impl turso::IntoParams,
    ) -> Result<Vec<LoginRow>> {
        let mut rows = self
            .conn
            .query(&format!("SELECT {COLUMNS} {tail}"), params)
            .await?;
        let mut out = Vec::new();
        while let Some(r) = rows.next().await? {
            out.push(row(&r)?);
        }
        Ok(out)
    }

    /// Replace the encrypted username and password (password change, key rotation). `false`
    /// when the login does not exist.
    pub async fn replace_login_secrets(
        &mut self,
        id: &str,
        username_enc: Vec<u8>,
        password_enc: Vec<u8>,
    ) -> Result<bool> {
        let n = self
            .conn
            .execute(
                "UPDATE logins SET username_enc = ?1, password_enc = ?2 WHERE id = ?3",
                (username_enc, password_enc, id),
            )
            .await?;
        Ok(n > 0)
    }

    /// Count a use at `now_ms`; the last-used time never moves backwards.
    pub async fn record_login_use(&mut self, id: &str, now_ms: i64) -> Result<bool> {
        let n = self
            .conn
            .execute(
                "UPDATE logins SET times_used = times_used + 1, \
                 last_used_ms = MAX(COALESCE(last_used_ms, ?1), ?1) WHERE id = ?2",
                (now_ms, id),
            )
            .await?;
        Ok(n > 0)
    }

    pub async fn delete_login(&mut self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM logins WHERE id = ?1", [id])
            .await?
            > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    fn login(id: &str, origin: &str, seed: u8) -> LoginRow {
        LoginRow {
            id: id.into(),
            origin: origin.into(),
            form_action_origin: Some(format!("{origin}/login")),
            // Not UTF-8, with zero bytes: blobs must survive untouched.
            username_enc: vec![0xff, 0x00, seed, 0xfe],
            password_enc: vec![0x00, 0x80, 0xc3, 0x28, seed],
            created_ms: 1000 + i64::from(seed),
            last_used_ms: None,
            times_used: 0,
        }
    }

    #[test]
    fn bytes_round_trip_exactly() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let want = login("id-a", "https://a.test", 7);
            s.insert_login(want.clone()).await.unwrap();
            assert_eq!(s.login("id-a").await.unwrap(), Some(want));
            assert_eq!(s.login("id-b").await.unwrap(), None);
            let mut empty = login("id-e", "https://e.test", 1);
            empty.form_action_origin = None;
            empty.username_enc = vec![];
            empty.last_used_ms = Some(5);
            empty.times_used = 3;
            s.insert_login(empty.clone()).await.unwrap();
            assert_eq!(s.login("id-e").await.unwrap(), Some(empty));
        });
    }

    #[test]
    fn duplicate_id_is_a_constraint_error() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.insert_login(login("same", "https://a.test", 1))
                .await
                .unwrap();
            let err = s
                .insert_login(login("same", "https://b.test", 2))
                .await
                .unwrap_err();
            assert!(err.is_constraint(), "{err:?}");
            assert_eq!(s.all_logins().await.unwrap().len(), 1);
        });
    }

    #[test]
    fn origin_lookup_is_exact() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.insert_login(login("a1", "https://a.test", 1))
                .await
                .unwrap();
            s.insert_login(login("x", "https://a.test:8443", 2))
                .await
                .unwrap();
            s.insert_login(login("y", "http://a.test", 3))
                .await
                .unwrap();
            s.insert_login(login("a2", "https://a.test", 4))
                .await
                .unwrap();
            let ids: Vec<String> = s
                .logins_for_origin("https://a.test")
                .await
                .unwrap()
                .into_iter()
                .map(|l| l.id)
                .collect();
            assert_eq!(ids, ["a1", "a2"]);
            assert!(
                s.logins_for_origin("https://b.test")
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(s.all_logins().await.unwrap().len(), 4);
        });
    }

    #[test]
    fn replace_secrets_leaves_metadata() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.insert_login(login("a", "https://a.test", 1))
                .await
                .unwrap();
            assert!(s.record_login_use("a", 50).await.unwrap());
            assert!(
                s.replace_login_secrets("a", vec![9], vec![7, 7])
                    .await
                    .unwrap()
            );
            let l = s.login("a").await.unwrap().unwrap();
            assert_eq!((l.username_enc, l.password_enc), (vec![9], vec![7, 7]));
            assert_eq!(
                (l.origin.as_str(), l.times_used, l.last_used_ms),
                ("https://a.test", 1, Some(50))
            );
            assert!(
                !s.replace_login_secrets("nope", vec![], vec![])
                    .await
                    .unwrap()
            );
        });
    }

    #[test]
    fn use_counts_and_last_used_never_goes_back() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.insert_login(login("a", "https://a.test", 1))
                .await
                .unwrap();
            s.record_login_use("a", 500).await.unwrap();
            s.record_login_use("a", 300).await.unwrap(); // clock went back
            s.record_login_use("a", 700).await.unwrap();
            let l = s.login("a").await.unwrap().unwrap();
            assert_eq!((l.times_used, l.last_used_ms), (3, Some(700)));
            assert!(!s.record_login_use("nope", 1).await.unwrap());
        });
    }

    #[test]
    fn delete_reports_whether_a_row_went() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.insert_login(login("a", "https://a.test", 1))
                .await
                .unwrap();
            assert!(s.delete_login("a").await.unwrap());
            assert!(!s.delete_login("a").await.unwrap());
            assert!(s.all_logins().await.unwrap().is_empty());
        });
    }

    #[test]
    fn logins_persist_across_reopen() {
        run(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("wubble.db");
            {
                let mut s = Store::open(&path).await.unwrap();
                s.insert_login(login("a", "https://a.test", 5))
                    .await
                    .unwrap();
            }
            let s = Store::open(&path).await.unwrap();
            assert_eq!(
                s.login("a").await.unwrap().unwrap(),
                login("a", "https://a.test", 5)
            );
            assert_eq!(s.check_consistency().await.unwrap(), Vec::<String>::new());
        });
    }

    #[test]
    fn a_v3_database_gets_the_vault_shape() {
        run(async {
            let mut s = Store::connect(":memory:").await.unwrap();
            crate::migrate::migrate(&mut s.conn, &crate::MIGRATIONS[..3])
                .await
                .unwrap();
            // The old integer-id table is there and empty.
            assert!(
                s.conn
                    .query("SELECT id, username_ct FROM logins", ())
                    .await
                    .is_ok()
            );
            crate::migrate::migrate(&mut s.conn, crate::MIGRATIONS)
                .await
                .unwrap();
            assert!(
                s.conn
                    .query("SELECT username_ct FROM logins", ())
                    .await
                    .is_err()
            );
            s.insert_login(login("a", "https://a.test", 1))
                .await
                .unwrap();
            assert_eq!(s.all_logins().await.unwrap().len(), 1);
        });
    }

    /// A login written just before the process ends without closing the database (a quit
    /// that skips the shutdown, a crash) is there when the database opens again: the child
    /// inserts and exits with no destructor run; the parent reads.
    #[test]
    fn a_login_survives_an_exit_that_skips_the_shutdown() {
        const DB: &str = "WUBBLE_STORE_ABRUPT_DB";
        if let Ok(path) = std::env::var(DB) {
            run(async {
                let mut s = Store::open(std::path::Path::new(&path)).await.unwrap();
                s.insert_login(login("id-a", "https://a.test", 7))
                    .await
                    .unwrap();
                std::process::exit(0);
            });
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wubble.db");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "logins::tests::a_login_survives_an_exit_that_skips_the_shutdown",
                "--nocapture",
            ])
            .env(DB, &path)
            .status()
            .unwrap();
        assert!(status.success());
        run(async {
            let s = Store::open(&path).await.unwrap();
            assert_eq!(
                s.login("id-a").await.unwrap(),
                Some(login("id-a", "https://a.test", 7))
            );
        });
    }
}
