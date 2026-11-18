//! Remembered permission decisions (`wubble_core::site`): one row per origin
//! and kind. The host loads them all once into `SitePermissions` and writes
//! each decision the user asks to remember.
//!
//! Design reference, no code copied: Chromium `components/content_settings`
//! (BSD-3-Clause) and Firefox `nsIPermissionManager` (MPL-2.0).

use wubble_core::site::{Decision, Permission};

use crate::*;

impl Store {
    /// Remembers `decision` for `kind` on `origin`, replacing an earlier one.
    pub async fn set_site_permission(
        &mut self,
        origin: &str,
        kind: Permission,
        decision: Decision,
        at_ms: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction().await?;
        tx.execute(
            "DELETE FROM site_permissions WHERE origin = ?1 AND kind = ?2",
            (origin, kind.as_str()),
        )
        .await?;
        tx.execute(
            "INSERT INTO site_permissions(origin, kind, decision, decided_at) \
             VALUES (?1, ?2, ?3, ?4)",
            (
                origin,
                kind.as_str(),
                i64::from(decision == Decision::Allow),
                at_ms,
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Forgets the decision for `kind` on `origin`: the site is asked again.
    pub async fn forget_site_permission(&mut self, origin: &str, kind: Permission) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM site_permissions WHERE origin = ?1 AND kind = ?2",
                (origin, kind.as_str()),
            )
            .await?;
        Ok(())
    }

    /// Forgets every decision for `origin`.
    pub async fn forget_site(&mut self, origin: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM site_permissions WHERE origin = ?1", (origin,))
            .await?;
        Ok(())
    }

    /// Every remembered decision. A kind this build does not know is
    /// skipped (a newer build wrote it), not an error.
    pub async fn site_permissions(&self) -> Result<Vec<(String, Permission, Decision)>> {
        let mut rows = self
            .conn
            .query(
                "SELECT origin, kind, decision FROM site_permissions ORDER BY origin, kind",
                (),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            let kind: String = r.get(1)?;
            let Some(kind) = Permission::parse(&kind) else {
                continue;
            };
            let decision = match r.get::<i64>(2)? {
                1 => Decision::Allow,
                0 => Decision::Block,
                v => return Err(Error::Corrupt(format!("permission decision {v}"))),
            };
            out.push((r.get(0)?, kind, decision));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permission::*;

    #[test]
    fn set_replace_and_forget() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            assert!(s.site_permissions().await.unwrap().is_empty());
            let a = "https://a.test";
            let b = "https://b.test:8443";
            s.set_site_permission(a, Camera, Decision::Allow, 1)
                .await
                .unwrap();
            s.set_site_permission(a, Geolocation, Decision::Block, 2)
                .await
                .unwrap();
            s.set_site_permission(b, Camera, Decision::Block, 3)
                .await
                .unwrap();
            // Replacing keeps one row.
            s.set_site_permission(a, Camera, Decision::Block, 4)
                .await
                .unwrap();
            assert_eq!(
                s.site_permissions().await.unwrap(),
                vec![
                    (a.to_string(), Camera, Decision::Block),
                    (a.to_string(), Geolocation, Decision::Block),
                    (b.to_string(), Camera, Decision::Block),
                ]
            );
            s.forget_site_permission(a, Camera).await.unwrap();
            s.forget_site(b).await.unwrap();
            assert_eq!(
                s.site_permissions().await.unwrap(),
                vec![(a.to_string(), Geolocation, Decision::Block)]
            );
        });
    }

    #[test]
    fn unknown_kinds_are_skipped_and_bad_decisions_are_corrupt() {
        pollster::block_on(async {
            let s = Store::open_in_memory().await.unwrap();
            s.conn
                .execute(
                    "INSERT INTO site_permissions VALUES ('https://a.test', 'teleport', 1, 0)",
                    (),
                )
                .await
                .unwrap();
            assert!(s.site_permissions().await.unwrap().is_empty());
            s.conn
                .execute(
                    "INSERT INTO site_permissions VALUES ('https://a.test', 'camera', 7, 0)",
                    (),
                )
                .await
                .unwrap();
            assert!(matches!(s.site_permissions().await, Err(Error::Corrupt(_))));
        });
    }
}
