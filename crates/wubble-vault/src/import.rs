//! Pure parser for the CSV that Firefox and Chromium export.
//!
//! Header handling follows Firefox `LoginCSVImport.sys.mjs`: names are matched
//! case-insensitively with aliases (`url` | `login_uri`, `username` |
//! `login_username`, `password` | `login_password`); two columns for one field
//! is an error; an empty username is allowed. Chromium's export header is
//! `name,url,username,password,note` (older versions lack `note`); `name`,
//! `note` and the other Firefox columns are ignored. Rows that cannot be
//! imported (non-http(s) URL such as `android://`, empty password) are skipped
//! with their line number rather than failing the file. [`apply`] saves the rows and dedups.
//!
//! Sources: export a CSV from Firefox, Zen or Chromium, then `parse_csv` and `apply`.
//! `wubble-import` has no logins reader (Firefox's `logins.json` is NSS-encrypted).

use crate::keyring::KeyRing;
use crate::origin::Origin;
use crate::record::NewLogin;
use crate::state::VaultState;
use crate::store::LoginStore;
use crate::vault::{Vault, VaultError};
use zeroize::Zeroizing;

pub struct ImportRow {
    pub origin: Origin,
    pub form_action_origin: Option<Origin>,
    pub username: String,
    pub password: Zeroizing<String>,
}

impl std::fmt::Debug for ImportRow {
    /// Redacts username and password, like `NewLogin`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportRow")
            .field("origin", &self.origin)
            .field("form_action_origin", &self.form_action_origin)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Skipped {
    pub line: u64,
    pub reason: &'static str,
}

#[derive(Debug, Default)]
pub struct Import {
    pub rows: Vec<ImportRow>,
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("missing column: {0}")]
    MissingColumn(&'static str),
    #[error("two columns map to {0}")]
    ConflictingColumns(&'static str),
    #[error("malformed CSV: {0}")]
    Csv(String),
}

#[derive(Clone, Copy)]
enum Col {
    Url,
    Username,
    Password,
    FormAction,
    HttpRealm,
}

fn column(header: &str) -> Option<Col> {
    match header.trim().to_ascii_lowercase().as_str() {
        "url" | "login_uri" => Some(Col::Url),
        "username" | "login_username" => Some(Col::Username),
        "password" | "login_password" => Some(Col::Password),
        "formactionorigin" => Some(Col::FormAction),
        "httprealm" => Some(Col::HttpRealm),
        _ => None,
    }
}

pub fn parse_csv(input: &str) -> Result<Import, ImportError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut reader = csv::ReaderBuilder::new().from_reader(input.as_bytes());
    let csv_err = |e: csv::Error| ImportError::Csv(e.to_string());

    let mut index: [Option<usize>; 5] = [None; 5];
    for (i, h) in reader.headers().map_err(csv_err)?.iter().enumerate() {
        if let Some(col) = column(h) {
            let slot = &mut index[col as usize];
            if slot.is_some() {
                return Err(ImportError::ConflictingColumns(
                    [
                        "url",
                        "username",
                        "password",
                        "formactionorigin",
                        "httprealm",
                    ][col as usize],
                ));
            }
            *slot = Some(i);
        }
    }

    let mut import = Import::default();
    let mut records = reader.records().peekable();
    if records.peek().is_none() {
        return Ok(import);
    }
    let need = |slot: Option<usize>, name| slot.ok_or(ImportError::MissingColumn(name));
    let (url, user, pass) = (
        need(index[Col::Url as usize], "url")?,
        need(index[Col::Username as usize], "username")?,
        need(index[Col::Password as usize], "password")?,
    );
    let form = index[Col::FormAction as usize];
    let realm = index[Col::HttpRealm as usize];

    for record in records {
        let record = record.map_err(csv_err)?;
        let line = record.position().map_or(0, |p| p.line());
        let get = |i: usize| record.get(i).unwrap_or("");
        let password = get(pass);
        if realm.is_some_and(|i| !get(i).is_empty()) {
            // HTTP auth (Basic/Digest) is not a form login; Firefox never fills it in forms.
            import.skipped.push(Skipped {
                line,
                reason: "HTTP auth login",
            });
            continue;
        }
        if password.is_empty() {
            import.skipped.push(Skipped {
                line,
                reason: "empty password",
            });
            continue;
        }
        let Ok(origin) = Origin::parse(get(url)) else {
            import.skipped.push(Skipped {
                line,
                reason: "not an http(s) URL",
            });
            continue;
        };
        import.rows.push(ImportRow {
            origin,
            form_action_origin: form.and_then(|i| Origin::parse(get(i)).ok()),
            username: get(user).to_owned(),
            password: Zeroizing::new(password.to_owned()),
        });
    }
    Ok(import)
}

/// What [`apply`] did with each row.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub saved: usize,
    /// Same origin, username and password already saved.
    pub duplicates: usize,
    /// Same origin and username, different password: the saved one is kept, never overwritten
    /// (an export carries no times to say which is newer).
    pub conflicts: usize,
    /// The vault refused the row (store failure, ...).
    pub failed: usize,
    /// Lines the parser skipped, forwarded.
    pub skipped: Vec<Skipped>,
}

/// Save parsed rows through the vault. Plaintext only ever goes into `Vault::save`, which
/// encrypts it; the store never sees it. A row is a duplicate or conflict by origin + username
/// (the vault's exact-origin lookup); logins that no longer decrypt are not seen and so cannot
/// block a save. Errors up front when the vault is not unlocked, before touching any row.
pub fn apply<K: KeyRing, S: LoginStore>(
    vault: &mut Vault<K, S>,
    import: Import,
    now_ms: i64,
) -> Result<ApplyReport, VaultError> {
    match vault.state() {
        VaultState::Unlocked { .. } => {}
        VaultState::Locked => return Err(VaultError::NotUnlocked),
        VaultState::Unavailable { reason } => return Err(VaultError::Unavailable(reason.clone())),
        VaultState::KeyLost { rows } => return Err(VaultError::KeyLost { rows: *rows }),
    }
    let mut report = ApplyReport {
        skipped: import.skipped,
        ..ApplyReport::default()
    };
    for row in import.rows {
        match apply_row(vault, row, now_ms) {
            Ok(Outcome::Saved) => report.saved += 1,
            Ok(Outcome::Duplicate) => report.duplicates += 1,
            Ok(Outcome::Conflict) => report.conflicts += 1,
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

enum Outcome {
    Saved,
    Duplicate,
    Conflict,
}

fn apply_row<K: KeyRing, S: LoginStore>(
    vault: &mut Vault<K, S>,
    row: ImportRow,
    now_ms: i64,
) -> Result<Outcome, VaultError> {
    let mut same_user = false;
    for c in vault.find_for_origin(&row.origin)? {
        if c.username == row.username {
            same_user = true;
            if *vault.reveal(&c.id)? == *row.password {
                return Ok(Outcome::Duplicate);
            }
        }
    }
    if same_user {
        return Ok(Outcome::Conflict);
    }
    vault.save(
        NewLogin {
            origin: row.origin,
            form_action_origin: row.form_action_origin,
            username: row.username,
            password: row.password,
        },
        now_ms,
    )?;
    Ok(Outcome::Saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyring::FakeKeyRing;
    use crate::store::MemoryStore;
    use crate::store_adapter::StoreLogins;
    use wubble_store::Store;

    #[test]
    fn firefox_sample() {
        let csv = "\u{feff}\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\"\n\
\"https://a.test\",\"alice\",\"p,w\"\"1\",\"\",\"https://a.test\",\"{g1}\",\"1\"\n\
\"https://b.test\",\"\",\"multi\nline\",\"\",\"\",\"{g2}\",\"2\"\n";
        let imp = parse_csv(csv).unwrap();
        assert!(imp.skipped.is_empty());
        assert_eq!(imp.rows.len(), 2);
        assert_eq!(imp.rows[0].username, "alice");
        assert_eq!(&*imp.rows[0].password, "p,w\"1");
        assert_eq!(
            imp.rows[0].form_action_origin.as_ref().unwrap().to_string(),
            "https://a.test"
        );
        assert_eq!(imp.rows[1].username, "");
        assert_eq!(&*imp.rows[1].password, "multi\nline");
        assert!(imp.rows[1].form_action_origin.is_none());
    }

    #[test]
    fn http_auth_rows_are_skipped_and_debug_is_redacted() {
        let csv = "url,username,password,httpRealm,formActionOrigin\n\
https://a.test,admin,pw-secret,Admin,\n\
https://b.test,bob-user,pw2,,https://b.test\n";
        let imp = parse_csv(csv).unwrap();
        assert_eq!(
            imp.skipped,
            vec![Skipped {
                line: 2,
                reason: "HTTP auth login"
            }]
        );
        assert_eq!(imp.rows.len(), 1);
        let shown = format!("{imp:?}");
        assert!(!shown.contains("bob-user") && !shown.contains("pw2"));
    }

    #[test]
    fn chromium_sample_with_and_without_note() {
        let imp = parse_csv("name,url,username,password,note\nA,https://a.test/login,u,p,hello\n")
            .unwrap();
        assert_eq!(imp.rows[0].origin.to_string(), "https://a.test");
        let imp = parse_csv("name,url,username,password\nA,https://a.test,u,p\n").unwrap();
        assert_eq!(imp.rows.len(), 1);
    }

    #[test]
    fn aliases_and_case_insensitive_headers() {
        let imp =
            parse_csv("Login_URI,LOGIN_USERNAME,Login_Password\nhttps://a.test,u,p\n").unwrap();
        assert_eq!(imp.rows.len(), 1);
        assert_eq!(imp.rows[0].username, "u");
    }

    #[test]
    fn column_errors() {
        assert_eq!(
            parse_csv("url,username\nhttps://a.test,u\n").unwrap_err(),
            ImportError::MissingColumn("password")
        );
        assert_eq!(
            parse_csv("url,login_uri,username,password\nhttps://a.test,https://a.test,u,p\n")
                .unwrap_err(),
            ImportError::ConflictingColumns("url")
        );
    }

    #[test]
    fn skipped_rows_carry_line_numbers() {
        let csv = "url,username,password\nhttps://a.test,u,p\nandroid://hash@com.app/,u,p\nhttps://b.test,u,\n";
        let imp = parse_csv(csv).unwrap();
        assert_eq!(imp.rows.len(), 1);
        assert_eq!(
            imp.skipped,
            vec![
                Skipped {
                    line: 3,
                    reason: "not an http(s) URL"
                },
                Skipped {
                    line: 4,
                    reason: "empty password"
                }
            ]
        );
    }

    #[test]
    fn header_only_and_empty_files() {
        let imp = parse_csv("url,username,password\n").unwrap();
        assert!(imp.rows.is_empty() && imp.skipped.is_empty());
        assert!(parse_csv("").unwrap().rows.is_empty());
    }

    const CSV: &str = "url,username,password\n\
https://a.test,alice,pw-one-xyz\n\
https://a.test,bob,pw-two-xyz\n\
https://b.test,,pw-three-xyz\n\
android://x@com.app/,u,p\n";

    fn memory_vault() -> Vault<FakeKeyRing, MemoryStore> {
        let mut v = Vault::new(FakeKeyRing::default(), MemoryStore::default(), "default");
        v.unlock();
        v
    }

    #[test]
    fn apply_saves_rows_that_decrypt_and_forwards_skips() {
        let mut v = memory_vault();
        let report = apply(&mut v, parse_csv(CSV).unwrap(), 5).unwrap();
        assert_eq!(
            (
                report.saved,
                report.duplicates,
                report.conflicts,
                report.failed
            ),
            (3, 0, 0, 0)
        );
        assert_eq!(report.skipped.len(), 1);
        let a = Origin::parse("https://a.test").unwrap();
        let found = v.find_for_origin(&a).unwrap();
        let mut users: Vec<_> = found.iter().map(|c| c.username.as_str()).collect();
        users.sort();
        assert_eq!(users, ["alice", "bob"]);
        let alice = found.iter().find(|c| c.username == "alice").unwrap();
        assert_eq!(&**v.reveal(&alice.id).unwrap(), "pw-one-xyz");
        assert!(v.list().unwrap().iter().all(|m| m.created_ms == 5));
    }

    #[test]
    fn apply_twice_is_all_duplicates() {
        let mut v = memory_vault();
        apply(&mut v, parse_csv(CSV).unwrap(), 5).unwrap();
        let again = apply(&mut v, parse_csv(CSV).unwrap(), 6).unwrap();
        assert_eq!((again.saved, again.duplicates, again.conflicts), (0, 3, 0));
        assert_eq!(v.list().unwrap().len(), 3);
    }

    #[test]
    fn same_user_with_another_password_is_a_conflict_and_keeps_the_original() {
        let mut v = memory_vault();
        apply(&mut v, parse_csv(CSV).unwrap(), 5).unwrap();
        let changed =
            "url,username,password\nhttps://a.test,alice,other-pw\nhttps://a.test,carol,pw-c\n";
        let report = apply(&mut v, parse_csv(changed).unwrap(), 6).unwrap();
        assert_eq!(
            (report.saved, report.conflicts, report.duplicates),
            (1, 1, 0)
        );
        let a = Origin::parse("https://a.test").unwrap();
        let alice = v
            .find_for_origin(&a)
            .unwrap()
            .into_iter()
            .find(|c| c.username == "alice")
            .unwrap();
        assert_eq!(&**v.reveal(&alice.id).unwrap(), "pw-one-xyz");
    }

    #[test]
    fn repeated_rows_inside_one_file_save_once() {
        let mut v = memory_vault();
        let csv = "url,username,password\nhttps://a.test,u,p\nhttps://a.test,u,p\n";
        let report = apply(&mut v, parse_csv(csv).unwrap(), 1).unwrap();
        assert_eq!((report.saved, report.duplicates), (1, 1));
    }

    #[test]
    fn a_locked_or_unavailable_vault_errors_and_saves_nothing() {
        let mut v = Vault::new(FakeKeyRing::default(), MemoryStore::default(), "default");
        assert_eq!(
            apply(&mut v, parse_csv(CSV).unwrap(), 1).unwrap_err(),
            VaultError::NotUnlocked
        );
        assert!(v.list().unwrap().is_empty());
        let kr = FakeKeyRing::default();
        kr.set_failing(true);
        let mut v = Vault::new(kr, MemoryStore::default(), "default");
        v.unlock();
        assert!(matches!(
            apply(&mut v, parse_csv(CSV).unwrap(), 1).unwrap_err(),
            VaultError::Unavailable(_)
        ));
        assert!(v.list().unwrap().is_empty());
    }

    #[test]
    fn the_database_file_never_holds_the_plaintext() {
        pollster::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("wubble.db");
            {
                let store = Store::open(&path).await.unwrap();
                let mut v = Vault::new(FakeKeyRing::default(), StoreLogins::new(store), "default");
                v.unlock();
                let report = apply(&mut v, parse_csv(CSV).unwrap(), 5).unwrap();
                assert_eq!(report.saved, 3);
            }
            // Every file the database wrote (main file and any journal). The plaintext origin
            // must show up somewhere, or the scan proves nothing.
            let mut origin_seen = false;
            for entry in std::fs::read_dir(dir.path()).unwrap() {
                let bytes = std::fs::read(entry.unwrap().path()).unwrap();
                let origin = b"https://b.test";
                origin_seen |= bytes.windows(origin.len()).any(|w| w == origin);
                for needle in ["pw-one-xyz", "pw-two-xyz", "pw-three-xyz", "alice", "bob"] {
                    assert!(
                        !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                        "{needle} found in the database"
                    );
                }
            }
            assert!(origin_seen, "the rows never reached the files");
        });
    }
}
