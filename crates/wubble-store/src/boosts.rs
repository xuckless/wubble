//! Boosts (`wubble_core::boost`): one row each, in the user's order. The host loads them all
//! once at start and writes each edit; matching them to pages is `wubble_core::boost`'s.

use wubble_core::boost::{At, Boost, Tint};

use crate::*;

impl Store {
    /// Every Boost, in the user's order (`position`, then id).
    pub async fn boosts(&self) -> Result<Vec<Boost>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name, site, css, js, js_at, tint, tint_strength, font, enabled, \
                 position FROM boosts ORDER BY position, id",
                (),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            let tint = match r.get_value(6)? {
                turso::Value::Integer(rgb) => Some(Tint {
                    rgb: narrow::<u32>(rgb, "boost tint")? & 0x00ff_ffff,
                    strength: narrow::<u8>(r.get::<i64>(7)?.clamp(0, 100), "tint strength")?,
                }),
                _ => None,
            };
            let font = match r.get_value(8)? {
                turso::Value::Text(f) => Some(f),
                _ => None,
            };
            out.push(Boost {
                id: r.get(0)?,
                name: r.get(1)?,
                site: r.get(2)?,
                css: r.get(3)?,
                js: r.get(4)?,
                js_at: At::from_index(r.get(5)?),
                tint,
                font,
                enabled: r.get::<i64>(9)? != 0,
                order: r.get(10)?,
            });
        }
        Ok(out)
    }

    /// Stores `boost`: a new one (id 0) gets the next id and goes last; an existing one is
    /// replaced in place. Returns its id.
    pub async fn put_boost(&mut self, boost: &Boost, at_ms: i64) -> Result<i64> {
        let tint = boost.tint.map(|t| i64::from(t.rgb & 0x00ff_ffff));
        let strength = boost.tint.map_or(0, |t| i64::from(t.strength.min(100)));
        let fields = (
            boost.name.as_str(),
            boost.site.as_str(),
            boost.css.as_str(),
            boost.js.as_str(),
            boost.js_at.index(),
            tint,
            strength,
            boost.font.as_deref(),
            i64::from(boost.enabled),
            at_ms,
        );
        if boost.id != 0 {
            let changed = self
                .conn
                .execute(
                    "UPDATE boosts SET name = ?1, site = ?2, css = ?3, js = ?4, js_at = ?5, \
                     tint = ?6, tint_strength = ?7, font = ?8, enabled = ?9, updated_at = ?10, \
                     position = ?11 WHERE id = ?12",
                    (
                        fields.0,
                        fields.1,
                        fields.2,
                        fields.3,
                        fields.4,
                        fields.5,
                        fields.6,
                        fields.7,
                        fields.8,
                        fields.9,
                        boost.order,
                        boost.id,
                    ),
                )
                .await?;
            if changed == 0 {
                return Err(Error::Invalid(format!("no boost {}", boost.id)));
            }
            return Ok(boost.id);
        }
        let tx = self.conn.transaction().await?;
        let position = scalar(&tx, "SELECT coalesce(max(position) + 1, 0) FROM boosts", ())
            .await?
            .unwrap_or(0);
        tx.execute(
            "INSERT INTO boosts(name, site, css, js, js_at, tint, tint_strength, font, \
             enabled, updated_at, position) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                fields.0, fields.1, fields.2, fields.3, fields.4, fields.5, fields.6, fields.7,
                fields.8, fields.9, position,
            ),
        )
        .await?;
        let id = tx.last_insert_rowid();
        tx.commit().await?;
        Ok(id)
    }

    pub async fn delete_boost(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM boosts WHERE id = ?1", (id,))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Boost> {
        let text = include_str!("../tests/fixtures/boosts.json");
        let rows: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
        rows.iter()
            .map(|v| Boost {
                id: 0,
                name: v["name"].as_str().unwrap().into(),
                site: v["site"].as_str().unwrap().into(),
                css: v["css"].as_str().unwrap().into(),
                js: v["js"].as_str().unwrap().into(),
                js_at: At::from_index(v["js_at"].as_i64().unwrap()),
                tint: v["tint"].as_u64().map(|rgb| Tint {
                    rgb: rgb as u32,
                    strength: v["tint_strength"].as_u64().unwrap() as u8,
                }),
                font: v["font"].as_str().map(Into::into),
                enabled: v["enabled"].as_bool().unwrap(),
                order: 0,
            })
            .collect()
    }

    #[test]
    fn boosts_round_trip_in_order() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            assert!(s.boosts().await.unwrap().is_empty());
            let mut ids = vec![];
            for b in fixture() {
                ids.push(s.put_boost(&b, 1).await.unwrap());
            }
            let stored = s.boosts().await.unwrap();
            assert_eq!(stored.len(), 3);
            for (i, (got, want)) in stored.iter().zip(fixture()).enumerate() {
                assert_eq!(got.id, ids[i]);
                assert_eq!(got.order, i as i64, "new Boosts go last");
                assert_eq!(
                    Boost {
                        id: 0,
                        order: 0,
                        ..got.clone()
                    },
                    want
                );
            }
            // An edit replaces the row; moving it first reorders the list.
            let mut first = stored[2].clone();
            first.css = "a{color:red}".into();
            first.tint = None;
            first.order = -1;
            assert_eq!(s.put_boost(&first, 2).await.unwrap(), first.id);
            let stored = s.boosts().await.unwrap();
            assert_eq!(stored[0], first);
            assert_eq!(stored.len(), 3);
            s.delete_boost(first.id).await.unwrap();
            assert_eq!(s.boosts().await.unwrap().len(), 2);
            // An edit of a deleted Boost is refused, not re-created.
            assert!(s.put_boost(&first, 3).await.is_err());
        });
    }
}
