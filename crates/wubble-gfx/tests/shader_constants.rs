//! The scene's shaders are translations of the design's web GLSL (kit plan, "Shaders are
//! translations"): every WGSL file opens with `// translates: <file>:<a>-<b>[; ...]`, and the
//! numbers in it are the numbers of the GLSL lines it cites, both ways. A number only one side
//! has is a failure, unless the WGSL line that holds it says why (`// wubble change:`). The
//! render constants the shaders depend on (blur σ, pyramid divisors, glass padding, the DPR cap)
//! are checked against the web's TypeScript the same way.
//!
//! The GLSL is read from `tools/fidelity/glsl-snapshot/` (the design's `core/gl/*.ts`, copied
//! verbatim by `tools/fidelity/extract.mjs`, which `--check`s it against the clone), so this runs
//! without the clone.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use wubble_gfx::scene::{BLUR_SIGMA, DPR_MAX, GLASS_PAD, PYRAMID_DIV};

const CHANGE: &str = "// wubble change:";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn snapshot(file: &str) -> String {
    let path = repo().join("tools/fidelity/glsl-snapshot").join(file);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A number as a set key: its value, to 1e-9 (so `2`, `2.0` and `2.` are one).
fn key(v: f64) -> i64 {
    (v * 1e9).round() as i64
}

/// A line without its comments (`//`, `/* */` within the line, a doc comment's own lines).
fn code(line: &str) -> String {
    let t = line.trim_start();
    if t.starts_with("/*") && t.ends_with("*/") || t.starts_with("* ") || t == "*" {
        return String::new();
    }
    let mut out = line.split("//").next().unwrap_or("").to_owned();
    while let (Some(a), Some(b)) = (out.find("/*"), out.find("*/")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + 2, " ");
    }
    out
}

/// The number literals of a line of shader code, unsigned, with comments, attributes
/// (`@binding(3)`) and identifiers (`vec3`, `f32`, `uB2`) left out.
fn numbers(line: &str) -> Vec<f64> {
    let code = code(line);
    let code = code.as_str();
    let b = code.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'@' || c.is_ascii_alphabetic() || c == b'_' {
            // an identifier (or an attribute's name): skip it, and an attribute's argument
            let at = c == b'@';
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if at && b.get(i) == Some(&b'(') {
                while i < b.len() && b[i] != b')' {
                    i += 1;
                }
            }
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            let mut dot = false;
            while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'.' && !dot)) {
                dot |= b[i] == b'.';
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                i += 1;
                if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                    i += 1;
                }
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            // a type suffix (`1u`, `2.0f`)
            let text = &code[start..i];
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            out.push(
                text.trim_end_matches('.')
                    .parse::<f64>()
                    .unwrap_or_else(|_| panic!("number {text:?} in {line:?}")),
            );
        } else {
            i += 1;
        }
    }
    out
}

/// The cited ranges of a WGSL file's header: `(file, first, last)`, 1-based and inclusive.
fn header(name: &str, wgsl: &str) -> Vec<(String, usize, usize)> {
    let first = wgsl.lines().next().unwrap_or("");
    let cites = first
        .strip_prefix("// translates: ")
        .unwrap_or_else(|| panic!("{name}: no `// translates:` header"));
    cites
        .split(';')
        .map(|c| {
            let c = c.trim();
            let (file, lines) = c
                .rsplit_once(':')
                .unwrap_or_else(|| panic!("{name}: cite {c:?}"));
            let (a, b) = lines.split_once('-').unwrap_or((lines, lines));
            let n = |s: &str| -> usize {
                s.parse()
                    .unwrap_or_else(|_| panic!("{name}: line {s:?} in {c:?}"))
            };
            (file.to_owned(), n(a), n(b))
        })
        .collect()
}

/// The numbers of the GLSL lines `ranges` cite, without the `#version`/`precision` lines and
/// the `${...}` includes (each included chunk is its own WGSL file).
fn glsl_numbers(name: &str, ranges: &[(String, usize, usize)]) -> BTreeSet<i64> {
    let mut set = BTreeSet::new();
    for (file, a, b) in ranges {
        let text = snapshot(file);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            *a >= 1 && a <= b && *b <= lines.len(),
            "{name}: {file}:{a}-{b} is outside the file ({} lines)",
            lines.len()
        );
        for line in &lines[a - 1..*b] {
            // `const X = `#version 300 es` opens a block; `${FILM}` includes another
            if ["#version", "precision ", "${"]
                .iter()
                .any(|s| line.contains(s))
            {
                continue;
            }
            set.extend(numbers(line).into_iter().map(key));
        }
    }
    set
}

/// The WGSL file's numbers, and the ones on lines marked `// wubble change:` (exempt).
fn wgsl_numbers(wgsl: &str) -> (BTreeSet<i64>, BTreeSet<i64>) {
    let (mut plain, mut changed) = (BTreeSet::new(), BTreeSet::new());
    for line in wgsl.lines().skip(1) {
        let set = if line.contains(CHANGE) {
            &mut changed
        } else {
            &mut plain
        };
        set.extend(numbers(line).into_iter().map(key));
    }
    (plain, changed)
}

fn shaders() -> Vec<(String, String)> {
    let dir = repo().join("crates/wubble-gfx/src/scene/shaders");
    let mut out: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("shaders dir")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()? == "wgsl").then(|| {
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    fs::read_to_string(&p).expect("read wgsl"),
                )
            })
        })
        .collect();
    out.sort();
    out
}

fn show(set: &BTreeSet<i64>) -> String {
    set.iter()
        .map(|&k| format!("{}", k as f64 / 1e9))
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn every_shader_has_the_numbers_of_the_glsl_it_translates() {
    let all = shaders();
    assert_eq!(all.len(), 9, "the scene's nine shaders");
    let mut errors = vec![];
    for (name, wgsl) in &all {
        let ranges = header(name, wgsl);
        let glsl = glsl_numbers(name, &ranges);
        let (plain, changed) = wgsl_numbers(wgsl);
        let only_wgsl: BTreeSet<i64> = plain.difference(&glsl).copied().collect();
        let only_glsl: BTreeSet<i64> = glsl
            .iter()
            .filter(|k| !plain.contains(k) && !changed.contains(k))
            .copied()
            .collect();
        if !only_wgsl.is_empty() {
            errors.push(format!(
                "{name}: numbers not in the GLSL (mark the line `{CHANGE}`): {}",
                show(&only_wgsl)
            ));
        }
        if !only_glsl.is_empty() {
            errors.push(format!(
                "{name}: GLSL numbers missing from the WGSL: {}",
                show(&only_glsl)
            ));
        }
    }
    assert!(errors.is_empty(), "\n{}", errors.join("\n"));
}

/// The numbers of the first snapshot line of `file` that contains `needle`.
fn ts_line(file: &str, needle: &str) -> Vec<f64> {
    let text = snapshot(file);
    let line = text
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{file}: no line with {needle:?}"));
    numbers(line)
}

#[test]
fn the_render_constants_are_the_web_s() {
    // scene.ts frame(): blur(this.bg, ..., 3.5); blur(this.b1, ..., 4.5); blur(this.b2, ..., 5.0)
    for (level, from) in ["this.bg,", "this.b1,", "this.b2,"].iter().enumerate() {
        let n = ts_line("core/gl/scene.ts", &format!("blur({from}"));
        assert_eq!(
            n.last().copied(),
            Some(f64::from(BLUR_SIGMA[level])),
            "σ of level {}",
            level + 1
        );
    }
    // scene.ts resize(): b1 = ceil(hw / 2), b2 = ceil(hw / 4), b3 = ceil(hw / 8)
    for (level, b) in ["this.b1 = ", "this.b2 = ", "this.b3 = "]
        .iter()
        .enumerate()
    {
        let n = ts_line("core/gl/scene.ts", b);
        assert_eq!(
            n.first().copied(),
            Some(f64::from(PYRAMID_DIV[level])),
            "{b}"
        );
    }
    // scene.ts resize(w, h, dpr = Math.min(devicePixelRatio, 2))
    assert_eq!(
        ts_line("core/gl/scene.ts", "Math.min(devicePixelRatio,"),
        [f64::from(DPR_MAX)]
    );
    // scene.ts drawGlass(): const pad = 70
    assert_eq!(ts_line("core/gl/scene.ts", "const pad = "), [GLASS_PAD]);
}

#[test]
fn the_snapshot_is_of_one_design_commit() {
    let commit = snapshot("COMMIT");
    let commit = commit.trim();
    assert!(
        commit.len() == 7 && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "{commit:?}"
    );
}

#[test]
fn numbers_are_read_like_a_compiler_reads_them() {
    assert_eq!(
        numbers("let x = vec3<f32>(0.115, 2., .5) * 1e-5 + uB2.x; // 70 px"),
        [0.115, 2.0, 0.5, 1e-5]
    );
    assert_eq!(
        numbers("@group(0) @binding(3) var uB2: texture_2d<f32>;"),
        Vec::<f64>::new()
    );
    assert_eq!(
        numbers("for (int i = 0; i < 7; i++) { float a = -12.0; }"),
        [0.0, 7.0, 12.0]
    );
    assert_eq!(numbers("uniform vec4 uRip[8]; uint n = 4u;"), [8.0, 4.0]);
}
