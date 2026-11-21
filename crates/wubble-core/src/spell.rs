//! The spelling dictionary Chromium finds in the profile's `Dictionaries` directory
//! (`chrome::DIR_APP_DICTIONARIES`, which CEF puts at `<root_cache_path>/Dictionaries`).
//!
//! Chromium names a dictionary `<language>-<major>-<minor>.bdic`: the version in the name is how
//! it replaces an old file, and a file under the wrong name is ignored and downloaded again (a
//! request wubble refuses). wubble ships the English one inside the app and puts it there
//! before CEF starts; this decides what to do about what is already in the directory. Pure:
//! the engine lists the directory and runs the answer.

/// A `.bdic` file as the directory holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub len: u64,
}

/// What to do to the directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Files of an older (or newer) version of the same language, to delete.
    pub remove: Vec<String>,
    /// The bundled file is missing or not the bundled one: write it.
    pub write: bool,
}

/// The language of a versioned name: `en-US-10-2.bdic` is `en-US`.
fn language(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".bdic")?;
    let (rest, minor) = stem.rsplit_once('-')?;
    let (language, major) = rest.rsplit_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(major) && digits(minor)).then_some(language)
}

/// The plan for the bundled dictionary `bundled` (its versioned name and size) given the
/// `.bdic` files `found`. A same-named file of another size is a truncated or foreign one:
/// rewritten. Other languages' files, and anything that is not a versioned `.bdic`, are left.
pub fn plan(bundled: &Found, found: &[Found]) -> Plan {
    let lang = language(&bundled.name);
    let remove = found
        .iter()
        .filter(|f| f.name != bundled.name && language(&f.name) == lang)
        .map(|f| f.name.clone())
        .collect();
    let write = !found.contains(bundled);
    Plan { remove, write }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, len: u64) -> Found {
        Found {
            name: name.into(),
            len,
        }
    }

    #[test]
    fn languages_come_from_versioned_names() {
        assert_eq!(language("en-US-10-2.bdic"), Some("en-US"));
        assert_eq!(language("en-GB-oxendict-10-2.bdic"), Some("en-GB-oxendict"));
        assert_eq!(language("en-US.bdic"), None);
        assert_eq!(language("en-US-10-2.bdic.tmp"), None);
        assert_eq!(language("notes.txt"), None);
    }

    #[test]
    fn a_missing_dictionary_is_written() {
        let p = plan(&file("en-US-10-2.bdic", 100), &[]);
        assert_eq!(
            p,
            Plan {
                remove: vec![],
                write: true
            }
        );
    }

    #[test]
    fn a_present_dictionary_is_left_alone() {
        let bundled = file("en-US-10-2.bdic", 100);
        assert_eq!(
            plan(&bundled, std::slice::from_ref(&bundled)),
            Plan::default()
        );
    }

    #[test]
    fn a_stale_version_is_replaced() {
        let p = plan(
            &file("en-US-10-2.bdic", 100),
            &[file("en-US-9-0.bdic", 90), file("fr-FR-3-0.bdic", 50)],
        );
        assert_eq!(p.remove, vec!["en-US-9-0.bdic".to_string()]);
        assert!(p.write);
    }

    #[test]
    fn a_truncated_file_is_rewritten() {
        let p = plan(
            &file("en-US-10-2.bdic", 100),
            &[file("en-US-10-2.bdic", 40)],
        );
        assert_eq!(
            p,
            Plan {
                remove: vec![],
                write: true
            }
        );
    }

    #[test]
    fn other_languages_stay() {
        let bundled = file("en-US-10-2.bdic", 100);
        let p = plan(&bundled, &[bundled.clone(), file("en-GB-10-2.bdic", 80)]);
        assert_eq!(p, Plan::default());
    }
}
