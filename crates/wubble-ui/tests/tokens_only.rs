//! Every colour, size and duration in the UI comes from a token file. The
//! design repo's port token file (`tokens.slint`) and the kit (`kit/`: the
//! port's components and `derived.slint`, moved there unchanged) are exempt;
//! every other file is scanned. Also: no focus borders and no letter
//! tiles anywhere, port files included. Pure string scans; no timing.

use std::fs;
use std::path::{Path, PathBuf};

/// Files whose literals are the source of truth (or a verbatim copy of it).
const TOKEN_FILES: [&str; 1] = ["tokens.slint"];

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read ui dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") {
            out.push(path);
        }
    }
}

fn ui_files() -> Vec<PathBuf> {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    slint_files(&ui, &mut files);
    assert!(!files.is_empty(), "no slint files found");
    files
}

fn is_token_file(path: &Path) -> bool {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    path.starts_with(ui.join("kit"))
        || path.parent().is_some_and(|p| p.ends_with("ui"))
            && TOKEN_FILES.iter().any(|t| path.ends_with(t))
}

/// Returns the offending literals found in one line of Slint source.
fn literals(line: &str) -> Vec<String> {
    let code = line.split("//").next().unwrap_or("");
    // String contents (texts, SVG path data) are not lengths: blank them out.
    let mut quoted = false;
    let code: String = code
        .chars()
        .map(|c| {
            if c == '"' {
                quoted = !quoted;
            }
            if quoted { ' ' } else { c }
        })
        .collect();
    let code = code.as_str();
    let chars: Vec<char> = code.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '#' && chars.get(i + 1).is_some_and(|n| n.is_ascii_hexdigit()) {
            found.push(
                chars[i..]
                    .iter()
                    .copied()
                    .enumerate()
                    .take_while(|(k, h)| *k == 0 || h.is_ascii_hexdigit())
                    .map(|(_, h)| h)
                    .collect(),
            );
        }
        let starts_number = c.is_ascii_digit()
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '_' | '-')));
        if starts_number {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            let number: String = chars[i..j].iter().collect();
            let unit: String = chars[j..]
                .iter()
                .take_while(|u| u.is_alphabetic())
                .collect();
            let is_zero = number.chars().all(|d| d == '0' || d == '.');
            if matches!(unit.as_str(), "px" | "ms" | "s") && !is_zero {
                found.push(format!("{number}{unit}"));
            }
            i = j;
            continue;
        }
        i += 1;
    }
    for call in ["rgb(", "rgba(", "hsl(", "hsla("] {
        if code.contains(call) {
            found.push(call.to_string());
        }
    }
    found
}

#[test]
fn no_literals_outside_tokens() {
    let mut violations = Vec::new();
    for file in ui_files().into_iter().filter(|f| !is_token_file(f)) {
        let text = fs::read_to_string(&file).expect("read slint file");
        for (n, line) in text.lines().enumerate() {
            for lit in literals(line) {
                violations.push(format!("{}:{}: {lit}", file.display(), n + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "literals outside the token files:\n{}",
        violations.join("\n")
    );
}

#[test]
fn scanner_flags_literals() {
    assert_eq!(literals("background: #fff;"), ["#fff"]);
    assert_eq!(literals("width: 12px;"), ["12px"]);
    assert_eq!(literals("duration: 150ms;"), ["150ms"]);
    assert!(literals("color: rgba(0, 0, 0, 1);").contains(&"rgba(".to_string()));
    assert_eq!(literals("text: \"\u{d7}\"; background: #fff;"), ["#fff"]);
    assert!(literals("x: 0px; y: Spacing.s2; // 12px #fff").is_empty());
    assert!(literals("commands: \"M3 12h18s-3.8 8.5 1s\";").is_empty());
    assert_eq!(literals("text: \"a\"; width: 3px;"), ["3px"]);
}

/// Focus and selection show by fill and the touch sheen, never a border:
/// no `border-color` within three lines of a focus test.
#[test]
fn no_focus_borders() {
    let mut violations = Vec::new();
    for file in ui_files() {
        let text = fs::read_to_string(&file).expect("read slint file");
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            if !line.contains("border-color") {
                continue;
            }
            let near = &lines[n.saturating_sub(3)..(n + 4).min(lines.len())];
            if near
                .iter()
                .any(|l| l.contains("has-focus") || l.contains("focused when"))
            {
                violations.push(format!("{}:{}", file.display(), n + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "focus borders:\n{}",
        violations.join("\n")
    );
}

/// No squircle with a letter anywhere: no glyph tiles or glyph fields.
#[test]
fn no_letter_tiles() {
    let mut violations = Vec::new();
    for file in ui_files() {
        let text = fs::read_to_string(&file).expect("read slint file");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let words = code.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'));
            for word in words {
                if matches!(word, "glyph" | "GlyphTile" | "PinnedTile" | "SpaceDot") {
                    violations.push(format!("{}:{}: {word}", file.display(), n + 1));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "letter tiles:\n{}",
        violations.join("\n")
    );
}
