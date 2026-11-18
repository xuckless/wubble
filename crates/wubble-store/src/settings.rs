//! Key/value settings. Keys are constants here; values are text.

use crate::{Result, Store};

pub const SEARCH_ENGINE: &str = "search.engine";
/// "1" when live engine suggestions are enabled. Off by default.
pub const SUGGEST_LIVE: &str = "suggest.live";

impl Store {
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut rows = self
            .conn
            .query("SELECT value FROM settings WHERE key = ?1", [key])
            .await?;
        Ok(match rows.next().await? {
            Some(r) => Some(r.get(0)?),
            None => None,
        })
    }

    pub async fn set_setting(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                (key, value),
            )
            .await?;
        Ok(())
    }

    /// Live suggestions flag; false until the user turns it on.
    pub async fn live_suggestions(&self) -> Result<bool> {
        Ok(self.get_setting(SUGGEST_LIVE).await?.as_deref() == Some("1"))
    }

    pub async fn set_live_suggestions(&mut self, on: bool) -> Result<()> {
        self.set_setting(SUGGEST_LIVE, if on { "1" } else { "0" })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_overwrite() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            assert_eq!(s.get_setting(SEARCH_ENGINE).await.unwrap(), None);
            s.set_setting(SEARCH_ENGINE, "google").await.unwrap();
            s.set_setting(SEARCH_ENGINE, "duckduckgo").await.unwrap();
            assert_eq!(
                s.get_setting(SEARCH_ENGINE).await.unwrap().as_deref(),
                Some("duckduckgo")
            );
        });
    }

    #[test]
    fn live_suggestions_default_off() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            assert!(!s.live_suggestions().await.unwrap());
            s.set_live_suggestions(true).await.unwrap();
            assert!(s.live_suggestions().await.unwrap());
            s.set_live_suggestions(false).await.unwrap();
            assert!(!s.live_suggestions().await.unwrap());
        });
    }
}
