//! Reading a translation's numbers and diffing them with its source's (the kit plan's
//! "Shaders are translations"): shared by the scene's shader test (wubble-gfx
//! `tests/shader_constants.rs`) and the thinking orb's (wubble-core `tests/orb_constants.rs`, which
//! includes this file by path). A translated file opens with `// translates: <file>:<a>-<b>[; ...]`;
//! its numbers are its source lines' numbers, in the same order, unless a line within a difference
//! says why (`// wubble change:`).

pub const CHANGE: &str = "// wubble change:";

/// A number as a set key: its value, to 1e-9 (so `2`, `2.0` and `2.` are one).
pub fn key(v: f64) -> i64 {
    (v * 1e9).round() as i64
}

/// `text` with its block comments (`/* ... */`, across lines) blanked out, its lines kept, so
/// a line number still names the same line. A continuation line that starts with an operator
/// (`* rs,`) is code, not a doc comment's line.
pub fn uncommented(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(a) = rest.find("/*") {
        // a `/*` after `//` on its line is part of the line comment
        let line_start = rest[..a].rfind('\n').map_or(0, |i| i + 1);
        if rest[line_start..a].contains("//") {
            let end = rest[a..].find('\n').map_or(rest.len(), |i| a + i);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        out.push_str(&rest[..a]);
        let close = rest[a + 2..]
            .find("*/")
            .map_or(rest.len(), |i| a + 2 + i + 2);
        out.extend(
            rest[a..close]
                .chars()
                .map(|c| if c == '\n' { '\n' } else { ' ' }),
        );
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

/// A line without its `//` comment (block comments: [`uncommented`]).
pub fn code(line: &str) -> String {
    line.split("//").next().unwrap_or("").to_owned()
}

/// The number literals of a line of shader code, unsigned, with comments, attributes
/// (`@binding(3)`) and identifiers (`vec3`, `f32`, `uB2`) left out.
pub fn numbers(line: &str) -> Vec<f64> {
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
pub fn header(name: &str, wgsl: &str) -> Vec<(String, usize, usize)> {
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

/// One number of the translation: its value and the (0-based) line it is on, after the header.
pub struct WgslNumber {
    pub value: f64,
    pub line: usize,
}

/// The translation's numbers in order, and which of its lines are marked `// wubble change:`.
pub fn wgsl_numbers(wgsl: &str) -> (Vec<WgslNumber>, Vec<bool>) {
    let (mut out, mut changed) = (vec![], vec![]);
    let plain = uncommented(wgsl);
    for (line, (text, code)) in wgsl.lines().zip(plain.lines()).skip(1).enumerate() {
        changed.push(text.contains(CHANGE));
        out.extend(
            numbers(code)
                .into_iter()
                .map(|value| WgslNumber { value, line }),
        );
    }
    (out, changed)
}

/// The differences between the GLSL's numbers and the WGSL's, in order: a longest common
/// subsequence of the two, then each stretch between two matched numbers (a hunk) that has a
/// number of one side only (the source is the GLSL or TS, the translation the WGSL or Rust). A hunk is excused when a WGSL line it spans is marked
/// `// wubble change:`; the lines of the matched numbers around it count, so a change that drops
/// a number in the middle of a line is excused by that line's mark.
pub fn differences(glsl: &[f64], wgsl: &[WgslNumber], changed: &[bool]) -> Vec<String> {
    let (n, m) = (glsl.len(), wgsl.len());
    let same = |i: usize, j: usize| key(glsl[i]) == key(wgsl[j].value);
    // lcs[i][j]: the common length of glsl[i..] and wgsl[j..]
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if same(i, j) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut matched = vec![];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(i, j) {
            matched.push((i, j));
            (i, j) = (i + 1, j + 1);
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let last_line = changed.len().saturating_sub(1);
    let mut errors = vec![];
    let mut from = (0, 0);
    for &(gi, wj) in matched.iter().chain([(n, m)].iter()) {
        let (g, w) = (&glsl[from.0..gi], &wgsl[from.1..wj]);
        if !g.is_empty() || !w.is_empty() {
            // the WGSL lines from the matched number before the hunk to the one after it
            let first = from.1.checked_sub(1).map_or(0, |k| wgsl[k].line);
            let last = wgsl.get(wj).map_or(last_line, |x| x.line);
            if !changed[first..=last.max(first)].iter().any(|&c| c) {
                let show = |v: &mut dyn Iterator<Item = f64>| {
                    v.map(|x| x.to_string()).collect::<Vec<_>>().join(", ")
                };
                errors.push(format!(
                    "WGSL lines {}-{}: GLSL has [{}], WGSL has [{}] (mark the line `{CHANGE}` \
                     if it differs on purpose)",
                    first + 2,
                    last + 2,
                    show(&mut g.iter().copied()),
                    show(&mut w.iter().map(|x| x.value)),
                ));
            }
        }
        from = (gi + 1, wj + 1);
    }
    errors
}
