//! Pure parser for the CSV that Firefox and Chromium export.
//!
//! Header handling follows Firefox `LoginCSVImport.sys.mjs`: names are matched
//! case-insensitively with aliases (`url` | `login_uri`, `username` |
//! `login_username`, `password` | `login_password`); two columns for one field
//! is an error; an empty username is allowed. Chromium's export header is
//! `name,url,username,password,note` (older versions lack `note`); `name`,
//! `note` and the other Firefox columns are ignored. Rows that cannot be
//! imported (non-http(s) URL such as `android://`, empty password) are skipped
//! with their line number rather than failing the file. Dedup is the caller's job.

use crate::origin::Origin;
use zeroize::Zeroizing;

#[derive(Debug)]
pub struct ImportRow {
    pub origin: Origin,
    pub form_action_origin: Option<Origin>,
    pub username: String,
    pub password: Zeroizing<String>,
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
}

fn column(header: &str) -> Option<Col> {
    match header.trim().to_ascii_lowercase().as_str() {
        "url" | "login_uri" => Some(Col::Url),
        "username" | "login_username" => Some(Col::Username),
        "password" | "login_password" => Some(Col::Password),
        "formactionorigin" => Some(Col::FormAction),
        _ => None,
    }
}

pub fn parse_csv(input: &str) -> Result<Import, ImportError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut reader = csv::ReaderBuilder::new().from_reader(input.as_bytes());
    let csv_err = |e: csv::Error| ImportError::Csv(e.to_string());

    let mut index: [Option<usize>; 4] = [None; 4];
    for (i, h) in reader.headers().map_err(csv_err)?.iter().enumerate() {
        if let Some(col) = column(h) {
            let slot = &mut index[col as usize];
            if slot.is_some() {
                return Err(ImportError::ConflictingColumns(
                    ["url", "username", "password", "formactionorigin"][col as usize],
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

    for record in records {
        let record = record.map_err(csv_err)?;
        let line = record.position().map_or(0, |p| p.line());
        let get = |i: usize| record.get(i).unwrap_or("");
        let password = get(pass);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
