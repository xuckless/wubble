//! The agent activity log (`wubble_core::agent::log`): appended per call, bounded to
//! `STORE_MAX` rows. Design reference, no code copied: `closed.rs`'s bounded list.

use wubble_core::agent::SessionId;
use wubble_core::agent::log::{Entry, Outcome, STORE_MAX};

use crate::*;

const COLUMNS: &str = "session, at_ms, tool, target, outcome";

fn outcome_code(o: Outcome) -> i64 {
    match o {
        Outcome::Done => 0,
        Outcome::Refused => 1,
        Outcome::Failed => 2,
    }
}

fn outcome_of(code: i64) -> Outcome {
    match code {
        0 => Outcome::Done,
        1 => Outcome::Refused,
        _ => Outcome::Failed,
    }
}

impl Store {
    /// Append an entry and drop rows beyond `STORE_MAX`.
    pub async fn push_agent_log(&mut self, e: &Entry) -> Result<()> {
        self.push_agent_log_keeping(e, STORE_MAX).await
    }

    async fn push_agent_log_keeping(&mut self, e: &Entry, keep: usize) -> Result<()> {
        let tx = self.conn.transaction().await?;
        tx.execute(
            format!("INSERT INTO agent_log({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
            (
                e.session.0.as_str(),
                e.at_ms,
                e.tool.as_str(),
                e.target.as_deref(),
                outcome_code(e.outcome),
            ),
        )
        .await?;
        tx.execute(
            format!(
                "DELETE FROM agent_log WHERE id NOT IN \
                 (SELECT id FROM agent_log ORDER BY id DESC LIMIT {keep})"
            ),
            (),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The newest `limit` entries, oldest first.
    pub async fn agent_log(&self, limit: usize) -> Result<Vec<Entry>> {
        let mut rows = self
            .conn
            .query(
                format!("SELECT {COLUMNS} FROM agent_log ORDER BY id DESC LIMIT {limit}"),
                (),
            )
            .await?;
        let mut out = vec![];
        while let Some(r) = rows.next().await? {
            out.push(Entry {
                session: SessionId(r.get(0)?),
                at_ms: r.get(1)?,
                tool: r.get(2)?,
                target: r.get(3)?,
                outcome: outcome_of(r.get(4)?),
            });
        }
        out.reverse();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: i64) -> Entry {
        Entry {
            session: SessionId("s".into()),
            at_ms: n,
            tool: format!("t{n}"),
            target: (n % 2 == 0).then(|| "https://a.test".to_string()),
            outcome: [Outcome::Done, Outcome::Refused, Outcome::Failed][n as usize % 3],
        }
    }

    #[test]
    fn the_log_is_bounded_and_reads_back_in_order() {
        pollster::block_on(async {
            let mut s = Store::open_in_memory().await.unwrap();
            let (keep, total) = (5, 12);
            for n in 0..total {
                s.push_agent_log_keeping(&entry(n), keep).await.unwrap();
            }
            let all = s.agent_log(usize::MAX >> 1).await.unwrap();
            assert_eq!(all.len(), keep);
            assert_eq!(all.first().unwrap().at_ms, total - keep as i64);
            assert_eq!(*all.last().unwrap(), entry(total - 1));
            let tail = s.agent_log(2).await.unwrap();
            assert_eq!(tail, vec![entry(total - 2), entry(total - 1)]);
        });
    }
}
