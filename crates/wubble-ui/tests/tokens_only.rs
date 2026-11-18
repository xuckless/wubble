//! Every colour, size and duration in the UI comes from `tokens.slint`.
//! Pure string scan of the other `.slint` files; deterministic, no timing.

use std::fs;
use std::path::{Path, PathBuf};

fn slint_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read ui dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            slint_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "slint") && !path.ends_with("tokens.slint")
        {
            out.push(path);
        }
    }
}

/// Returns the offending literals found in one line of Slint source.
fn literals(line: &str) -> Vec<String> {
    let code = line.split("//").next().unwrap_or("");
    let chars: Vec<char> = code.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '#' && chars.get(i + 1).is_some_and(|n| n.is_ascii_hexdigit()) {
            found.push(
                code[i..]
                    .chars()
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
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    slint_files(&ui, &mut files);
    assert!(!files.is_empty(), "no slint files found");
    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("read slint file");
        for (n, line) in text.lines().enumerate() {
            for lit in literals(line) {
                violations.push(format!("{}:{}: {lit}", file.display(), n + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "literals outside tokens.slint:\n{}",
        violations.join("\n")
    );
}

#[test]
fn scanner_flags_literals() {
    assert_eq!(literals("background: #fff;"), ["#fff"]);
    assert_eq!(literals("width: 12px;"), ["12px"]);
    assert_eq!(literals("duration: 150ms;"), ["150ms"]);
    assert!(literals("color: rgba(0, 0, 0, 1);").contains(&"rgba(".to_string()));
    assert!(literals("x: 0px; y: Spacing.s2; // 12px #fff").is_empty());
}
