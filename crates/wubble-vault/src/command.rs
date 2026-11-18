//! The vault as values: a [`VaultCommand`] in, a [`VaultAnswer`] out, so the store thread (which
//! owns the one vault) has a single request for all of it and the page's every action is a pure
//! mapping to a command. Secrets in either never reach `Debug`, so they never reach a log.

use std::fmt;
use std::path::PathBuf;

use wubble_core::pages::{LoginView, VaultStatus};
use zeroize::Zeroizing;

use crate::import::{self, ApplyReport};
use crate::keyring::KeyRing;
use crate::origin::Origin;
use crate::record::{NewLogin, RecordId};
use crate::state::VaultState;
use crate::store::LoginStore;
use crate::vault::{UnlockOutcome, Vault, VaultError};

/// The biggest CSV export read: a password export is a few hundred kB; this stops a wrong file
/// (a video, say) from filling memory.
const MAX_IMPORT_BYTES: u64 = 16 << 20;

pub enum VaultCommand {
    /// Fetch or create the key in the OS keyring (may prompt).
    Unlock,
    Lock,
    /// The saved logins with their usernames (nothing while locked).
    List,
    Reveal(RecordId),
    Delete(RecordId),
    Add {
        /// A site as typed: `example.com` means `https://example.com`.
        site: String,
        username: String,
        password: Zeroizing<String>,
    },
    /// A browser's password export, read from this path on the store thread.
    ImportCsv(PathBuf),
}

impl fmt::Debug for VaultCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlock => f.write_str("Unlock"),
            Self::Lock => f.write_str("Lock"),
            Self::List => f.write_str("List"),
            Self::Reveal(id) => f.debug_tuple("Reveal").field(id).finish(),
            Self::Delete(id) => f.debug_tuple("Delete").field(id).finish(),
            Self::Add { site, .. } => f
                .debug_struct("Add")
                .field("site", site)
                .finish_non_exhaustive(),
            Self::ImportCsv(path) => f.debug_tuple("ImportCsv").field(path).finish(),
        }
    }
}

pub enum VaultAnswer {
    Status(VaultStatus),
    Entries(Vec<LoginView>),
    Revealed(Zeroizing<String>),
    Done,
    /// A report line for the page ("Imported 3, ...").
    Imported(String),
    /// Why a command failed: the error's own text, which never holds a secret.
    Failed(String),
}

impl fmt::Debug for VaultAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(s) => f.debug_tuple("Status").field(s).finish(),
            Self::Entries(rows) => write!(f, "Entries({} rows)", rows.len()),
            Self::Revealed(_) => f.write_str("Revealed(<redacted>)"),
            Self::Done => f.write_str("Done"),
            Self::Imported(s) => f.debug_tuple("Imported").field(s).finish(),
            Self::Failed(s) => f.debug_tuple("Failed").field(s).finish(),
        }
    }
}

/// What an unlock means for the page.
fn status_of(outcome: UnlockOutcome) -> VaultStatus {
    match outcome {
        UnlockOutcome::Ready | UnlockOutcome::NewKey { unreadable: 0 } => VaultStatus::Unlocked,
        UnlockOutcome::NewKey { unreadable: rows } | UnlockOutcome::Unreadable { rows } => {
            VaultStatus::Unreadable { rows }
        }
        UnlockOutcome::Unavailable(why) => VaultStatus::Unavailable(why),
    }
}

/// The page's status for a vault that is not unlocked by this command.
fn locked_status(state: &VaultState) -> VaultStatus {
    match state {
        VaultState::Unavailable { reason } => VaultStatus::Unavailable(reason.clone()),
        _ => VaultStatus::Locked,
    }
}

/// The line the page shows after an import.
pub fn import_summary(r: &ApplyReport) -> String {
    let mut s = format!("Imported {}", r.saved);
    for (n, what) in [
        (r.duplicates, "already saved"),
        (r.conflicts, "differ from a saved login"),
        (r.failed, "failed"),
        (r.skipped.len(), "skipped"),
    ] {
        if n > 0 {
            s.push_str(&format!(", {n} {what}"));
        }
    }
    s
}

fn read_export(path: &PathBuf) -> Result<Zeroizing<String>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| "cannot open the file".to_owned())?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_IMPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read the file".to_owned())?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err("the file is too big to be a password export".into());
    }
    String::from_utf8(std::mem::take(&mut *bytes))
        .map(Zeroizing::new)
        .map_err(|_| "the file is not text".to_owned())
}

impl<K: KeyRing, S: LoginStore> Vault<K, S> {
    /// Runs one command. Nothing is listed or revealed unless the vault is unlocked.
    pub fn run(&mut self, command: VaultCommand, now_ms: i64) -> VaultAnswer {
        let failed = |e: VaultError| VaultAnswer::Failed(e.to_string());
        match command {
            VaultCommand::Unlock => VaultAnswer::Status(status_of(self.unlock())),
            VaultCommand::Lock => {
                self.lock();
                VaultAnswer::Status(locked_status(self.state()))
            }
            VaultCommand::List => match self.entries() {
                Ok(rows) => VaultAnswer::Entries(
                    rows.iter()
                        .map(|(meta, username)| {
                            let mut view = LoginView::from(meta);
                            view.username = username.clone();
                            view
                        })
                        .collect(),
                ),
                Err(VaultError::NotUnlocked) => VaultAnswer::Status(VaultStatus::Locked),
                Err(VaultError::Unavailable(why)) => {
                    VaultAnswer::Status(VaultStatus::Unavailable(why))
                }
                Err(e) => failed(e),
            },
            VaultCommand::Reveal(id) => self.reveal(&id).map_or_else(failed, VaultAnswer::Revealed),
            VaultCommand::Delete(id) => self.delete(&id).map_or_else(failed, |_| VaultAnswer::Done),
            VaultCommand::Add {
                site,
                username,
                password,
            } => {
                let site = site.trim();
                let site = if site.contains("://") {
                    site.to_owned()
                } else {
                    format!("https://{site}")
                };
                match Origin::parse(&site) {
                    Err(e) => VaultAnswer::Failed(format!("not a site: {e}")),
                    Ok(origin) => self
                        .save(
                            NewLogin {
                                origin,
                                form_action_origin: None,
                                username,
                                password,
                            },
                            now_ms,
                        )
                        .map_or_else(failed, |_| VaultAnswer::Done),
                }
            }
            VaultCommand::ImportCsv(path) => {
                let text = match read_export(&path) {
                    Ok(text) => text,
                    Err(why) => return VaultAnswer::Failed(why),
                };
                match import::parse_csv(&text) {
                    Err(e) => VaultAnswer::Failed(e.to_string()),
                    Ok(parsed) => match import::apply(self, parsed, now_ms) {
                        Ok(report) => VaultAnswer::Imported(import_summary(&report)),
                        Err(e) => failed(e),
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyring::FakeKeyRing;
    use crate::store::MemoryStore;

    type V = Vault<FakeKeyRing, MemoryStore>;

    fn vault(kr: &FakeKeyRing) -> V {
        Vault::new(kr.clone(), MemoryStore::default(), "default")
    }

    fn add(v: &mut V, site: &str, user: &str, pw: &str) -> VaultAnswer {
        v.run(
            VaultCommand::Add {
                site: site.into(),
                username: user.into(),
                password: Zeroizing::new(pw.into()),
            },
            5,
        )
    }

    fn entries(v: &mut V) -> Vec<LoginView> {
        match v.run(VaultCommand::List, 6) {
            VaultAnswer::Entries(rows) => rows,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unlock_outcomes_map_to_a_status() {
        let kr = FakeKeyRing::default();
        let mut v = vault(&kr);
        assert!(matches!(
            v.run(VaultCommand::Unlock, 0),
            VaultAnswer::Status(VaultStatus::Unlocked)
        ));
        add(&mut v, "a.test", "me", "pw");
        // A second profile on a fresh key cannot read the first one's rows.
        assert_eq!(
            status_of(UnlockOutcome::NewKey { unreadable: 2 }),
            VaultStatus::Unreadable { rows: 2 }
        );
        assert_eq!(
            status_of(UnlockOutcome::Unreadable { rows: 1 }),
            VaultStatus::Unreadable { rows: 1 }
        );
        assert_eq!(
            status_of(UnlockOutcome::Unavailable("x".into())),
            VaultStatus::Unavailable("x".into())
        );
        kr.set_failing(true);
        let mut v = vault(&kr);
        assert!(matches!(
            v.run(VaultCommand::Unlock, 0),
            VaultAnswer::Status(VaultStatus::Unavailable(_))
        ));
        // Unavailable lists nothing.
        assert!(matches!(
            v.run(VaultCommand::List, 0),
            VaultAnswer::Status(VaultStatus::Unavailable(_))
        ));
    }

    #[test]
    fn nothing_is_listed_or_revealed_while_locked() {
        let mut v = vault(&FakeKeyRing::default());
        assert!(matches!(
            v.run(VaultCommand::List, 0),
            VaultAnswer::Status(VaultStatus::Locked)
        ));
        assert!(matches!(
            v.run(VaultCommand::Reveal(RecordId("x".into())), 0),
            VaultAnswer::Failed(_)
        ));
        assert!(matches!(
            add(&mut v, "a.test", "me", "pw"),
            VaultAnswer::Failed(_)
        ));
    }

    #[test]
    fn add_list_reveal_delete_and_lock() {
        let mut v = vault(&FakeKeyRing::default());
        v.run(VaultCommand::Unlock, 0);
        assert!(matches!(
            add(&mut v, "a.test", "me", "hunter2"),
            VaultAnswer::Done
        ));
        assert!(matches!(
            add(&mut v, "https://b.test", "you", "pw"),
            VaultAnswer::Done
        ));
        let rows = entries(&mut v);
        assert_eq!(rows.len(), 2);
        let a = rows.iter().find(|r| r.site() == "a.test").unwrap();
        assert_eq!(a.username.as_deref(), Some("me"));
        let VaultAnswer::Revealed(pw) = v.run(VaultCommand::Reveal(RecordId(a.id.clone())), 7)
        else {
            panic!("no password")
        };
        assert_eq!(&*pw, "hunter2");
        assert!(matches!(
            v.run(VaultCommand::Delete(RecordId(a.id.clone())), 8),
            VaultAnswer::Done
        ));
        assert_eq!(entries(&mut v).len(), 1);
        assert!(matches!(
            v.run(VaultCommand::Lock, 9),
            VaultAnswer::Status(VaultStatus::Locked)
        ));
        assert!(matches!(
            v.run(VaultCommand::List, 9),
            VaultAnswer::Status(VaultStatus::Locked)
        ));
    }

    #[test]
    fn a_bad_site_is_refused_with_a_reason() {
        let mut v = vault(&FakeKeyRing::default());
        v.run(VaultCommand::Unlock, 0);
        let VaultAnswer::Failed(why) = add(&mut v, "not a site://", "me", "pw") else {
            panic!("accepted")
        };
        assert!(why.starts_with("not a site"));
        assert!(entries(&mut v).is_empty());
    }

    #[test]
    fn an_undecryptable_row_lists_without_a_username() {
        let kr = FakeKeyRing::default();
        let mut v = vault(&kr);
        v.run(VaultCommand::Unlock, 0);
        add(&mut v, "a.test", "me", "pw");
        // Another key over the same rows: they list, but nothing opens.
        let rows = v.store().all().unwrap();
        let mut other = Vault::new(FakeKeyRing::default(), MemoryStore::default(), "default");
        for r in rows {
            other.store_mut().insert(r).unwrap();
        }
        assert!(matches!(
            other.run(VaultCommand::Unlock, 0),
            VaultAnswer::Status(VaultStatus::Unreadable { rows: 1 })
        ));
        let rows = entries(&mut other);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].username, None);
    }

    #[test]
    fn importing_a_file_reports_what_it_did() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.csv");
        std::fs::write(
            &path,
            "url,username,password\nhttps://a.test,me,pw\nhttps://a.test,me,pw\nnot a url,x,y\n",
        )
        .unwrap();
        let mut v = vault(&FakeKeyRing::default());
        v.run(VaultCommand::Unlock, 0);
        let VaultAnswer::Imported(line) = v.run(VaultCommand::ImportCsv(path.clone()), 1) else {
            panic!("no report")
        };
        assert!(line.starts_with("Imported 1"), "{line}");
        assert!(line.contains("already saved"), "{line}");
        assert_eq!(entries(&mut v).len(), 1);
        // A missing file is a refusal, not a panic.
        assert!(matches!(
            v.run(VaultCommand::ImportCsv(dir.path().join("none.csv")), 1),
            VaultAnswer::Failed(_)
        ));
        // Locked: refused up front.
        v.run(VaultCommand::Lock, 2);
        assert!(matches!(
            v.run(VaultCommand::ImportCsv(path), 2),
            VaultAnswer::Failed(_)
        ));
    }

    #[test]
    fn commands_and_answers_never_print_a_secret() {
        let add = VaultCommand::Add {
            site: "a.test".into(),
            username: "alice-the-user".into(),
            password: Zeroizing::new("s3cret-pw".into()),
        };
        let shown = format!(
            "{add:?} {:?}",
            VaultAnswer::Revealed(Zeroizing::new("s3cret-pw".into()))
        );
        assert!(
            !shown.contains("s3cret-pw") && !shown.contains("alice-the-user"),
            "{shown}"
        );
        let rows = VaultAnswer::Entries(vec![]);
        assert_eq!(format!("{rows:?}"), "Entries(0 rows)");
    }

    #[test]
    fn the_summary_names_each_nonzero_count() {
        let r = ApplyReport {
            saved: 3,
            conflicts: 1,
            ..ApplyReport::default()
        };
        assert_eq!(
            import_summary(&r),
            "Imported 3, 1 differ from a saved login"
        );
    }
}
