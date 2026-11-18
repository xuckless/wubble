//! Page icons, so restored or discarded tabs and command-bar rows show an icon
//! without a live view. Raw RGBA at most 64x64 (16 KiB), so there is no codec
//! and the byte cap of [`Store::prune_favicons`] is exact.
//!
//! Design reference, no code copied: Chromium `components/favicon/core/favicon_backend.cc`
//! (page URL -> icon mapping, size-bounded, BSD-3-Clause) and Firefox Places
//! `moz_pages_w_icons` / `moz_icons` (MPL-2.0).
//!
//! Icons are filed by origin (`wubble_core::url::origin_key`), not by page URL.
//! No foreign keys: rows are deleted explicitly.

use std::collections::HashMap;

use wubble_core::favicon::valid_rgba;
use wubble_core::url::origin_key;

use crate::*;

/// Most origins looked up per statement.
const CHUNK: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favicon {
    pub icon_url: String,
    pub width: u32,
    pub height: u32,
    /// Straight alpha, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Store {
    /// Remember `icon` for the origin of `page_url`, replacing what the origin
    /// had, and refresh the icon's `last_used`. Pages without an origin
    /// (`about:`, `file:`, ...) store nothing. A blob whose length is not
    /// `width * height * 4`, or a side outside `1..=64`, is [`Error::Invalid`].
    pub async fn put_favicon(&mut self, page_url: &str, icon: &Favicon, now_ms: i64) -> Result<()> {
        if !valid_rgba(icon.width, icon.height, icon.rgba.len()) {
            return Err(Error::Invalid(format!(
                "favicon {}: {}x{} with {} bytes",
                icon.icon_url,
                icon.width,
                icon.height,
                icon.rgba.len()
            )));
        }
        let Some(origin) = origin_key(page_url) else {
            return Ok(());
        };
        let tx = self.conn.transaction().await?;
        tx.execute(
            "INSERT INTO favicons(icon_url, width, height, rgba, last_used) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(icon_url) DO UPDATE SET width = excluded.width, \
             height = excluded.height, rgba = excluded.rgba, last_used = excluded.last_used",
            (
                icon.icon_url.as_str(),
                icon.width,
                icon.height,
                icon.rgba.clone(),
                now_ms,
            ),
        )
        .await?;
        tx.execute(
            "INSERT INTO favicon_origins(origin, icon_url, last_used) VALUES (?1, ?2, ?3) \
             ON CONFLICT(origin) DO UPDATE SET icon_url = excluded.icon_url, \
             last_used = excluded.last_used",
            (origin, icon.icon_url.as_str(), now_ms),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The icon of every page URL that has one, keyed by the page URL given.
    /// Pages without an origin or without a stored icon are absent. Reads
    /// only; `last_used` is refreshed by [`Self::put_favicon`].
    pub async fn favicons_for(&self, page_urls: &[&str]) -> Result<HashMap<String, Favicon>> {
        let mut origins: Vec<String> = page_urls.iter().filter_map(|u| origin_key(u)).collect();
        origins.sort();
        origins.dedup();
        let mut by_origin: HashMap<String, Favicon> = HashMap::new();
        for chunk in origins.chunks(CHUNK) {
            let marks = (1..=chunk.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut rows = self
                .conn
                .query(
                    format!(
                        "SELECT o.origin, f.icon_url, f.width, f.height, f.rgba \
                         FROM favicon_origins o JOIN favicons f ON f.icon_url = o.icon_url \
                         WHERE o.origin IN ({marks})"
                    ),
                    chunk.to_vec(),
                )
                .await?;
            while let Some(r) = rows.next().await? {
                by_origin.insert(
                    r.get(0)?,
                    Favicon {
                        icon_url: r.get(1)?,
                        width: narrow(r.get(2)?, "favicon width")?,
                        height: narrow(r.get(3)?, "favicon height")?,
                        rgba: blob(&r, 4)?,
                    },
                );
            }
        }
        Ok(page_urls
            .iter()
            .filter_map(|url| {
                let icon = by_origin.get(&origin_key(url)?)?;
                Some((url.to_string(), icon.clone()))
            })
            .collect())
    }

    /// Drop the least recently used icons until the stored bytes fit in
    /// `max_bytes`, then the origin rows left pointing at no icon. Returns the
    /// icons removed.
    pub async fn prune_favicons(&mut self, max_bytes: u64) -> Result<usize> {
        let mut rows = self
            .conn
            .query(
                "SELECT icon_url, length(rgba) FROM favicons ORDER BY last_used DESC, icon_url",
                (),
            )
            .await?;
        let (mut kept, mut doomed) = (0u64, Vec::new());
        while let Some(r) = rows.next().await? {
            let size: u64 = narrow(r.get(1)?, "favicon size")?;
            if kept + size <= max_bytes && doomed.is_empty() {
                kept += size;
            } else {
                doomed.push(r.get::<String>(0)?);
            }
        }
        drop(rows);
        let tx = self.conn.transaction().await?;
        for url in &doomed {
            tx.execute("DELETE FROM favicons WHERE icon_url = ?1", [url.as_str()])
                .await?;
        }
        tx.execute(
            "DELETE FROM favicon_origins WHERE icon_url NOT IN (SELECT icon_url FROM favicons)",
            (),
        )
        .await?;
        tx.commit().await?;
        Ok(doomed.len())
    }

    /// Forget every icon (clear browsing data).
    pub async fn clear_favicons(&mut self) -> Result<()> {
        self.conn
            .execute_batch("DELETE FROM favicon_origins; DELETE FROM favicons;")
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }

    fn icon(url: &str, fill: u8) -> Favicon {
        Favicon {
            icon_url: url.into(),
            width: 2,
            height: 2,
            rgba: vec![fill; 16],
        }
    }

    #[test]
    fn round_trip_and_shared_by_origin() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let a = icon("https://a.test/i.png", 7);
            s.put_favicon("https://a.test/x?y#z", &a, 10).await.unwrap();
            let got = s
                .favicons_for(&[
                    "https://a.test/other",
                    "https://a.test/x",
                    "https://b.test/",
                ])
                .await
                .unwrap();
            assert_eq!(got.len(), 2);
            assert_eq!(got["https://a.test/other"], a);
            assert_eq!(got["https://a.test/x"], a);
        });
    }

    #[test]
    fn overwrite_replaces_bytes_and_mapping() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.put_favicon("https://a.test/", &icon("https://a.test/1.png", 1), 1)
                .await
                .unwrap();
            let newer = icon("https://a.test/2.png", 2);
            s.put_favicon("https://a.test/", &newer, 2).await.unwrap();
            let same_url = icon("https://a.test/2.png", 3);
            s.put_favicon("https://a.test/p", &same_url, 3)
                .await
                .unwrap();
            let got = s.favicons_for(&["https://a.test/"]).await.unwrap();
            assert_eq!(got["https://a.test/"], same_url);
        });
    }

    #[test]
    fn bad_blobs_are_invalid_and_odd_pages_store_nothing() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let mut bad = icon("https://a.test/i.png", 0);
            bad.rgba.pop();
            assert!(matches!(
                s.put_favicon("https://a.test/", &bad, 1).await,
                Err(Error::Invalid(_))
            ));
            let mut big = icon("https://a.test/i.png", 0);
            (big.width, big.height, big.rgba) = (65, 1, vec![0; 65 * 4]);
            assert!(matches!(
                s.put_favicon("https://a.test/", &big, 1).await,
                Err(Error::Invalid(_))
            ));
            for page in [
                "about:blank",
                "file:///x.html",
                "data:text/html,x",
                "nonsense",
            ] {
                s.put_favicon(page, &icon("https://a.test/i.png", 0), 1)
                    .await
                    .unwrap();
            }
            assert!(
                s.favicons_for(&["https://a.test/"])
                    .await
                    .unwrap()
                    .is_empty()
            );
        });
    }

    #[test]
    fn lookup_crosses_chunk_boundaries_in_one_call() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let pages: Vec<String> = (0..250).map(|i| format!("https://h{i}.test/p")).collect();
            for (i, page) in pages.iter().enumerate().filter(|(i, _)| i % 2 == 0) {
                s.put_favicon(page, &icon(&format!("https://h{i}.test/i.png"), i as u8), 1)
                    .await
                    .unwrap();
            }
            let mut urls: Vec<&str> = pages.iter().map(String::as_str).collect();
            urls.extend(["about:blank", "https://never.test/"]);
            let got = s.favicons_for(&urls).await.unwrap();
            assert_eq!(got.len(), 125);
            assert_eq!(got["https://h248.test/p"].rgba, vec![248u8; 16]);
            assert!(!got.contains_key("https://h1.test/p"));
            assert!(!got.contains_key("about:blank"));
        });
    }

    #[test]
    fn prune_drops_oldest_first_and_orphans() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            for (host, at) in [("old", 1), ("mid", 2), ("new", 3)] {
                let page = format!("https://{host}.test/");
                s.put_favicon(&page, &icon(&format!("https://{host}.test/i"), 1), at)
                    .await
                    .unwrap();
            }
            // Each icon is 16 bytes: a 40-byte cap keeps the two newest.
            assert_eq!(s.prune_favicons(40).await.unwrap(), 1);
            let urls = [
                "https://old.test/",
                "https://mid.test/",
                "https://new.test/",
            ];
            let got = s.favicons_for(&urls).await.unwrap();
            assert_eq!(got.len(), 2);
            assert!(!got.contains_key("https://old.test/"));
            let orphans = scalar(
                &s.conn,
                "SELECT count(*) FROM favicon_origins WHERE origin = 'https://old.test'",
                (),
            )
            .await
            .unwrap();
            assert_eq!(orphans, Some(0));
            assert_eq!(s.prune_favicons(u64::MAX).await.unwrap(), 0);
            assert_eq!(s.prune_favicons(0).await.unwrap(), 2);
            assert!(s.favicons_for(&urls).await.unwrap().is_empty());
        });
    }

    #[test]
    fn clear_forgets_everything() {
        run(async {
            let mut s = Store::open_in_memory().await.unwrap();
            s.put_favicon("https://a.test/", &icon("https://a.test/i", 1), 1)
                .await
                .unwrap();
            s.clear_favicons().await.unwrap();
            assert!(
                s.favicons_for(&["https://a.test/"])
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(s.prune_favicons(0).await.unwrap(), 0);
        });
    }
}
