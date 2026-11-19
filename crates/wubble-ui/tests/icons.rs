//! One icon table: every glyph path lives in `ui/kit/gen/icons.slint` (the design's) or
//! `ui/kit/derived.slint` (`Derived`, the ones the design lacks), drawn by the kit `Icon`.
//! Pure string scans; no timing.

use std::fs;
use std::path::{Path, PathBuf};

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

fn ui() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// Path data is a string starting with a move command and a number.
fn has_path_literal(line: &str) -> bool {
    let line = line.split("//").next().unwrap_or("");
    line.contains("\"M") && line.chars().any(|c| c.is_ascii_digit()) && {
        let after = line.split("\"M").nth(1).unwrap_or("");
        after
            .trim_start()
            .starts_with(|c: char| c.is_ascii_digit() || c == '-')
    }
}

#[test]
fn no_private_icon_paths() {
    let mut files = Vec::new();
    slint_files(&ui(), &mut files);
    let mut bad = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(ui())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // the autoscroll pointer's arrowheads are cursor geometry, not an icon
        if rel.starts_with("kit/gen/")
            || rel == "kit/derived.slint"
            || rel == "components/autoscroll-cursor.slint"
        {
            continue;
        }
        for (i, line) in fs::read_to_string(&f).unwrap().lines().enumerate() {
            if has_path_literal(line) || line.contains("commands: \"M") {
                bad.push(format!("{rel}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(bad.is_empty(), "private path data:\n{}", bad.join("\n"));
}

#[test]
fn icon_defaults_are_the_designs() {
    let icon = fs::read_to_string(ui().join("kit/controls/icon.slint")).unwrap();
    for needle in [
        "size: IconStroke.size;",
        "weight: IconStroke.width;",
        "viewbox-width: IconStroke.grid;",
        "stroke-width: root.size * root.weight / IconStroke.grid;",
        "stroke-line-cap: round;",
        "stroke-line-join: round;",
    ] {
        assert!(icon.contains(needle), "Icon lost `{needle}`");
    }
    let shell = fs::read_to_string(ui().join("kit/derived.slint")).unwrap();
    for needle in [
        "icon: IconStroke.size;",
        "icon-stroke: IconStroke.width;",
        "icon-grid: IconStroke.grid;",
    ] {
        assert!(shell.contains(needle), "Shell lost `{needle}`");
    }
}

/// Every derived glyph stays on the design's grid: numbers within the 24 box.
#[test]
fn derived_glyphs_fit_the_grid() {
    let text = fs::read_to_string(ui().join("kit/derived.slint")).unwrap();
    let derived = text
        .split("export global Derived")
        .nth(1)
        .expect("Derived global");
    for line in derived.lines().filter(|l| l.contains("<string>")) {
        let d = line.split('"').nth(1).unwrap();
        let mut num = String::new();
        let mut nums = Vec::new();
        for c in d.chars().chain([' ']) {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
            } else {
                if let Ok(n) = num.parse::<f32>() {
                    nums.push(n);
                }
                num.clear();
            }
        }
        assert!(
            nums.iter().all(|n| *n <= 24.0),
            "outside the 24 grid: {line}"
        );
    }
}
