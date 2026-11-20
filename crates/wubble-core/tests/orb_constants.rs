//! The thinking orb's geometry is a translation of the design's TypeScript (Ultra Reactivate's
//! `vendor/thinking-orbs/engine`): every translated file under `src/orb/` opens with
//! `// translates: <file>:<a>-<b>[; ...]`, and its numbers are the cited TS lines' numbers, in the
//! same order, unless a line within a difference is marked `// wubble change:` (the scene's
//! shaders follow the same rule; the reader and the diff are theirs, shared by path).
//!
//! The TS is read from `tools/fidelity/ts-snapshot/` (copied verbatim by
//! `tools/fidelity/extract.mjs`, which `--check`s it against the clone), so this runs without the
//! clone. That the port's output equals the TS's is `orb::tests` (frames the TS computed).

use std::fs;
use std::path::{Path, PathBuf};

#[path = "../../wubble-gfx/tests/support/translation.rs"]
#[allow(dead_code)]
mod translation;
use translation::*;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A cited TS file from the snapshot: the citation is relative to `src/wubble`, the snapshot
/// keeps the files under their path below `src/`.
fn snapshot(file: &str) -> String {
    let rel = file
        .strip_prefix("../")
        .unwrap_or_else(|| panic!("{file}: the orb's sources are beside src/wubble"));
    let path = repo().join("tools/fidelity/ts-snapshot").join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn ts_numbers(name: &str, ranges: &[(String, usize, usize)]) -> Vec<f64> {
    let mut out = vec![];
    for (file, a, b) in ranges {
        let text = uncommented(&snapshot(file));
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            *a >= 1 && a <= b && *b <= lines.len(),
            "{name}: {file}:{a}-{b} is outside the file ({} lines)",
            lines.len()
        );
        for line in &lines[a - 1..*b] {
            out.extend(numbers(line));
        }
    }
    out
}

fn translations() -> Vec<(String, String)> {
    let dir = repo().join("crates/wubble-core/src/orb");
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("orb dir")
        .filter_map(|e| {
            let p = e.ok()?.path();
            let text = fs::read_to_string(&p).ok()?;
            text.starts_with("// translates: ")
                .then(|| (p.file_name().unwrap().to_string_lossy().into_owned(), text))
        })
        .collect();
    out.sort();
    out
}

#[test]
fn every_orb_file_has_the_numbers_of_the_ts_it_translates() {
    let all = translations();
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "braid.rs",
            "lattice.rs",
            "morph.rs",
            "orbits.rs",
            "presets.rs",
            "prim.rs",
            "profiles.rs",
            "ribbon.rs",
            "web.rs"
        ],
        "the orb's translated files"
    );
    let mut errors = vec![];
    for (name, rust) in &all {
        let ts = ts_numbers(name, &header(name, rust));
        let (ours, changed) = wgsl_numbers(rust);
        errors.extend(
            differences(&ts, &ours, &changed)
                .into_iter()
                .map(|e| format!("{name}: {e}")),
        );
    }
    assert!(errors.is_empty(), "\n{}", errors.join("\n"));
}

#[test]
fn the_snapshot_is_of_one_design_commit() {
    let path = repo().join("tools/fidelity/ts-snapshot/COMMIT");
    let commit = fs::read_to_string(path).expect("COMMIT");
    let commit = commit.trim();
    assert!(
        commit.len() == 7 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{commit:?}"
    );
}
