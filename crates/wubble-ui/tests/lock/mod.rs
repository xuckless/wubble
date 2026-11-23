//! What the shell's locks share (`kit_lock.rs`, `surface_lock.rs`): the `.slint` files, a line's
//! code, its property statements, and the ratchet that holds violation counts to a baseline that
//! only shrinks. Pure string scans; no timing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Violations counted per (file, rule, token).
pub type Counts = BTreeMap<(String, String, String), usize>;

pub fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

pub fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// Every `.slint` file, as (path relative to `ui/`, text).
pub fn ui_files() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    files(&ui_dir(), "slint", &mut paths);
    assert!(!paths.is_empty(), "no slint files found");
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(ui_dir())
                .expect("under ui")
                .to_string_lossy();
            (
                rel.replace('\\', "/"),
                fs::read_to_string(&p).expect("read"),
            )
        })
        .collect()
}

/// One line with `//` comments and string contents blanked out.
pub fn code(line: &str) -> String {
    let mut quoted = false;
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if !quoted && c == '/' && chars.peek() == Some(&'/') {
            break;
        }
        if c == '"' {
            quoted = !quoted;
            out.push(c);
        } else {
            out.push(if quoted { ' ' } else { c });
        }
    }
    out
}

/// The property statements of a line of code, as (name, expression): `name: expression` at the
/// start of a statement (after `{`, `;` or `}`).
pub fn statements(code: &str) -> Vec<(&str, &str)> {
    code.split(['{', ';', '}'])
        .filter_map(|statement| {
            let (name, expression) = statement.split_once(':')?;
            let name = name.trim();
            let plain = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
            plain.then_some((name, expression))
        })
        .collect()
}

/// A lock's baseline and how its messages name it.
pub struct Lock {
    /// The test's name: `tests/<name>.rs`, its baseline `tests/<name>_baseline.txt`.
    pub name: &'static str,
    /// Set, it rewrites the baseline (only when no count went up).
    pub env: &'static str,
    /// The rewritten baseline's first lines.
    pub header: &'static str,
    /// What to do about a new violation.
    pub hint: &'static str,
}

impl Lock {
    fn baseline_path(&self) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/{}_baseline.txt", self.name))
    }

    fn read_baseline(&self) -> Counts {
        let text = fs::read_to_string(self.baseline_path()).unwrap_or_default();
        text.lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                assert_eq!(f.len(), 4, "baseline line: {l}");
                (
                    (f[0].to_string(), f[1].to_string(), f[2].to_string()),
                    f[3].parse().expect("count"),
                )
            })
            .collect()
    }

    fn write_baseline(&self, counts: &Counts) {
        let mut text = String::from(self.header);
        for ((file, rule, token), n) in counts {
            text.push_str(&format!("{file}\t{rule}\t{token}\t{n}\n"));
        }
        fs::write(self.baseline_path(), text).expect("write baseline");
    }

    /// Holds `counts` to the baseline: fails on any count above it (a new violation) and on any
    /// below it (the baseline must be lowered with the fix). With the lock's variable set it
    /// rewrites the baseline, but only when no count went up: it can only shrink. The first
    /// baseline is recorded once, when the file does not exist yet.
    pub fn ratchet(&self, counts: &Counts) {
        let write = std::env::var_os(self.env).is_some();
        if !self.baseline_path().exists() && write {
            self.write_baseline(counts);
        }
        let baseline = self.read_baseline();
        let mut grown = Vec::new();
        let mut shrunk = Vec::new();
        for (key, n) in counts {
            let was = baseline.get(key).copied().unwrap_or(0);
            if *n > was {
                grown.push(format!(
                    "{}: {} `{}`: {n} (baseline {was})",
                    key.0, key.1, key.2
                ));
            }
        }
        for (key, was) in &baseline {
            let n = counts.get(key).copied().unwrap_or(0);
            if n < *was {
                shrunk.push(format!(
                    "{}: {} `{}`: {n} (baseline {was})",
                    key.0, key.1, key.2
                ));
            }
        }
        assert!(
            grown.is_empty(),
            "new {} violations ({}):\n{}",
            self.name,
            self.hint,
            grown.join("\n")
        );
        if write {
            self.write_baseline(counts);
            return;
        }
        assert!(
            shrunk.is_empty(),
            "violations cleared: lower the baseline ({}=1 cargo test -p wubble-ui --test {}):\n{}",
            self.env,
            self.name,
            shrunk.join("\n")
        );
    }
}
