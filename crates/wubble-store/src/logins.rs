//! Password-vault rows: opaque encrypted blobs plus plaintext metadata. No crypto here and no
//! vault types: the vault crate adapts its storage trait to these plain functions
//! (design: Chromium `login_database.cc`, Firefox `LoginManagerStorage`; no code copied).

use crate::{Result, Store, blob, narrow};

/// A stored login. Ciphertexts and nonces are whatever the vault wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRow {
    pub id: i64,
    /// `scheme://host[:port]`, plaintext so the vault can match without decrypting.
    pub origin: String,
    /// `None`: HTTP auth or no form.
    pub form_action_origin: Option<String>,
    pub username_ct: Vec<u8>,
    pub username_nonce: Vec<u8>,
    pub password_ct: Vec<u8>,
    pub password_nonce: Vec<u8>,
    pub created_ms: i64,
    /// `None`: never used.
    pub last_used_ms: Option<i64>,
    pub times_used: u32,
}

/// A login to insert; the store assigns the id and starts the use counters at zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLogin {
    pub origin: String,
    pub form_action_origin: Option<String>,
    pub username_ct: Vec<u8>,
    pub username_nonce: Vec<u8>,
    pub password_ct: Vec<u8>,
    pub password_nonce: Vec<u8>,
    pub created_ms: i64,
}

const COLUMNS: &str = "id, origin, form_action_origin, username_ct, username_nonce, \
    password_ct, password_nonce, created, last_used, times_used FROM logins";

fn row(r: &turso::Row) -> Result<LoginRow> {
    Ok(LoginRow {
        id: r.get(0)?,
        origin: r.get(1)?,
        form_action_origin: r.get(2)?,
        username_ct: blob(r, 3)?,
        username_nonce: blob(r, 4)?,
        password_ct: blob(r, 5)?,
        password_nonce: blob(r, 6)?,
        created_ms: r.get(7)?,
        last_used_ms: r.get(8)?,
        times_used: narrow(r.get(9)?, "times_used")?,
    })
}

impl Store {
    pub async fn insert_login(&mut self, login: NewLogin) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO logins(origin, form_action_origin, username_ct, username_nonce, \
                 password_ct, password_nonce, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    login.origin,
                    login.form_action_origin,
                    login.username_ct,
                    login.username_nonce,
                    login.password_ct,
                    login.password_nonce,
                    login.created_ms,
                ),
            )
            .await?;
        Ok(self.conn.last_insert_rowid())
    }

    pub async fn login(&self, id: i64) -> Result<Option<LoginRow>> {
        let mut rows = self
            .conn
            .query(&format!("SELECT {COLUMNS} WHERE id = ?1"), [id])
            .await?;
        rows.next().await?.map(|r| row(&r)).transpose()
    }

    /// Logins of exactly this origin, in id order.
    pub async fn logins_for_origin(&self, origin: &str) -> Result<Vec<LoginRow>> {
        self.logins_where("WHERE origin = ?1 ORDER BY id", [origin])
            .await
    }

    pub async fn all_logins(&self) -> Result<Vec<LoginRow>> {
        self.logins_where("ORDER BY id", ()).await
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

    /// Replace the encrypted username and password (key rotation, password change). `false`
    /// when the login does not exist.
    pub async fn replace_login_secrets(
        &mut self,
        id: i64,
        username_ct: Vec<u8>,
        username_nonce: Vec<u8>,
        password_ct: Vec<u8>,
        password_nonce: Vec<u8>,
    ) -> Result<bool> {
        let n = self
            .conn
            .execute(
                "UPDATE logins SET username_ct = ?1, username_nonce = ?2, \
                 password_ct = ?3, password_nonce = ?4 WHERE id = ?5",
                (username_ct, username_nonce, password_ct, password_nonce, id),
            )
            .await?;
        Ok(n > 0)
    }

    /// Count a use at `now_ms`; the last-used time never moves backwards.
    pub async fn record_login_use(&mut self, id: i64, now_ms: i64) -> Result<bool> {
        let n = self
            .conn
            .execute(
                "UPDATE logins SET times_used = times_used + 1, \
                 last_used = MAX(COALESCE(last_used, ?1), ?1) WHERE id = ?2",
                (now_ms, id),
            )
            .await?;
        Ok(n > 0)
    }

    pub async fn delete_login(&mut self, id: i64) -> Result<bool> {
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

    fn new_login(origin: &str, seed: u8) -> NewLogin {
        NewLogin {
            origin: origin.into(),
            form_action_origin: Some(format!("{origin}/login")),
            // Not UTF-8, with zero bytes: blobs must survive untouched.
            username_ct: vec![0xff, 0x00, seed, 0xfe],
            username_nonce: vec![seed; 12],
            password_ct: vec![0x00, 0x80, 0xc3, 0x28, seed],
            password_nonce: vec![seed.wrapping_add(1); 12],
            created_ms: 1000 + i64::from(seed),
        }
    }

    #[test]
    fn bytes_round_trip_exactly() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let id = s
                .insert_login(new_login("https://a.test", 7))
                .await
                .unwrap();
            let got = s.login(id).await.unwrap().unwrap();
            let want = new_login("https://a.test", 7);
            assert_eq!(got.id, id);
            assert_eq!(got.username_ct, want.username_ct);
            assert_eq!(got.password_ct, want.password_ct);
            assert_eq!(got.username_nonce, want.username_nonce);
            assert_eq!(got.password_nonce, want.password_nonce);
            assert_eq!(got.form_action_origin, want.form_action_origin);
            assert_eq!(
                (got.created_ms, got.last_used_ms, got.times_used),
                (1007, None, 0)
            );
            assert_eq!(s.login(id + 1).await.unwrap(), None);
            let mut empty = new_login("https://e.test", 1);
            empty.form_action_origin = None;
            empty.username_ct = vec![];
            let id = s.insert_login(empty).await.unwrap();
            let got = s.login(id).await.unwrap().unwrap();
            assert_eq!((got.form_action_origin, got.username_ct), (None, vec![]));
        });
    }

    #[test]
    fn origin_lookup_is_exact() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a1 = s
                .insert_login(new_login("https://a.test", 1))
                .await
                .unwrap();
            s.insert_login(new_login("https://a.test:8443", 2))
                .await
                .unwrap();
            s.insert_login(new_login("http://a.test", 3)).await.unwrap();
            let a2 = s
                .insert_login(new_login("https://a.test", 4))
                .await
                .unwrap();
            let ids: Vec<i64> = s
                .logins_for_origin("https://a.test")
                .await
                .unwrap()
                .iter()
                .map(|l| l.id)
                .collect();
            assert_eq!(ids, [a1, a2]);
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
            let id = s
                .insert_login(new_login("https://a.test", 1))
                .await
                .unwrap();
            assert!(s.record_login_use(id, 50).await.unwrap());
            assert!(
                s.replace_login_secrets(id, vec![9], vec![8; 12], vec![7, 7], vec![6; 12])
                    .await
                    .unwrap()
            );
            let l = s.login(id).await.unwrap().unwrap();
            assert_eq!((l.username_ct, l.username_nonce), (vec![9], vec![8; 12]));
            assert_eq!((l.password_ct, l.password_nonce), (vec![7, 7], vec![6; 12]));
            assert_eq!(
                (l.origin.as_str(), l.times_used, l.last_used_ms),
                ("https://a.test", 1, Some(50))
            );
            assert!(
                !s.replace_login_secrets(id + 9, vec![], vec![], vec![], vec![])
                    .await
                    .unwrap()
            );
        });
    }

    #[test]
    fn use_counts_and_last_used_never_goes_back() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let id = s
                .insert_login(new_login("https://a.test", 1))
                .await
                .unwrap();
            s.record_login_use(id, 500).await.unwrap();
            s.record_login_use(id, 300).await.unwrap(); // clock went back
            s.record_login_use(id, 700).await.unwrap();
            let l = s.login(id).await.unwrap().unwrap();
            assert_eq!((l.times_used, l.last_used_ms), (3, Some(700)));
            assert!(!s.record_login_use(id + 1, 1).await.unwrap());
        });
    }

    #[test]
    fn delete_reports_whether_a_row_went() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let id = s
                .insert_login(new_login("https://a.test", 1))
                .await
                .unwrap();
            assert!(s.delete_login(id).await.unwrap());
            assert!(!s.delete_login(id).await.unwrap());
            assert!(s.all_logins().await.unwrap().is_empty());
        });
    }

    #[test]
    fn logins_persist_across_reopen() {
        run(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("wubble.db");
            let id = {
                let mut s = Store::open(&path).await.unwrap();
                s.insert_login(new_login("https://a.test", 5))
                    .await
                    .unwrap()
            };
            let s = Store::open(&path).await.unwrap();
            let l = s.login(id).await.unwrap().unwrap();
            assert_eq!(l.password_ct, new_login("https://a.test", 5).password_ct);
        });
    }
}
