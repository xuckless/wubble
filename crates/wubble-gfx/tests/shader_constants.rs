//! The scene's shaders are translations of the design's web GLSL (kit plan, "Shaders are
//! translations"): every WGSL file opens with `// translates: <file>:<a>-<b>[; ...]`, and the
//! numbers in it are the numbers of the GLSL lines it cites, in the same order (a diff of the two
//! sequences, so a swapped pair or a value moved to another place fails). A number only one side
//! has is a failure, unless a WGSL line within that difference says why (`// wubble change:`). The
//! render constants the shaders depend on (blur σ, pyramid divisors, glass padding, the DPR cap)
//! are checked against the web's TypeScript the same way.
//!
//! The GLSL is read from `tools/fidelity/glsl-snapshot/` (the design's `core/gl/*.ts`, copied
//! verbatim by `tools/fidelity/extract.mjs`, which `--check`s it against the clone), so this runs
//! without the clone.

use std::fs;
use std::path::{Path, PathBuf};

use wubble_gfx::scene::{BLUR_SIGMA, DPR_MAX, GLASS_PAD, PYRAMID_DIV};

#[path = "support/translation.rs"]
#[allow(dead_code)]
mod translation;
use translation::*;

/// The header of a shader that is wubble's own, with no design counterpart.
const WUBBLE_ONLY: &str = "// translates: wubble-only";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn snapshot(file: &str) -> String {
    let path = repo().join("tools/fidelity/glsl-snapshot").join(file);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The numbers of the GLSL lines `ranges` cite, in order, without the `#version`/`precision`
/// lines and the `${...}` includes (each included chunk is its own WGSL file).
fn glsl_numbers(name: &str, ranges: &[(String, usize, usize)]) -> Vec<f64> {
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
            // `const X = `#version 300 es` opens a block; `${FILM}` includes another
            if ["#version", "precision ", "${"]
                .iter()
                .any(|s| line.contains(s))
            {
                continue;
            }
            out.extend(numbers(line));
        }
    }
    out
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

#[test]
fn every_shader_has_the_numbers_of_the_glsl_it_translates() {
    let all = shaders();
    assert_eq!(all.len(), 12, "the scene's twelve shaders");
    // wubble's own passes (the page in the scene) translate nothing: there is no GLSL to cite
    let own: Vec<&str> = all
        .iter()
        .filter(|(_, w)| w.starts_with(&format!("{WUBBLE_ONLY}\n")))
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(own, ["page.wgsl"], "wubble's own shaders");
    let mut errors = vec![];
    for (name, wgsl) in all.iter().filter(|(n, _)| !own.contains(&n.as_str())) {
        let glsl = glsl_numbers(name, &header(name, wgsl));
        let (numbers, changed) = wgsl_numbers(wgsl);
        errors.extend(
            differences(&glsl, &numbers, &changed)
                .into_iter()
                .map(|e| format!("{name}: {e}")),
        );
    }
    assert!(errors.is_empty(), "\n{}", errors.join("\n"));
}

#[test]
fn a_swapped_or_moved_number_is_a_difference() {
    let glsl: Vec<f64> = [
        "float s = mix(0.22, 0.38, k);",
        "float l = exp(-d * d / 0.45) * 0.03;",
        "vec2 f = vec2(0.85, 0.45);",
    ]
    .iter()
    .flat_map(|l| numbers(l))
    .collect();
    let diff = |wgsl: &str| {
        let wgsl = format!("// translates: x\n{wgsl}");
        let (n, c) = wgsl_numbers(&wgsl);
        differences(&glsl, &n, &c)
    };
    let same = "let s = mix(0.22, 0.38, k);\n\
                let l = exp(-d * d / 0.45) * 0.03;\n\
                let f = vec2(0.85, 0.45);";
    assert!(diff(same).is_empty(), "{:?}", diff(same));
    // a swapped pair: every number is still in the file
    let swapped = same.replace("mix(0.22, 0.38", "mix(0.38, 0.22");
    assert!(!diff(&swapped).is_empty());
    // a value changed to one found elsewhere in the file
    let moved = same.replace("/ 0.45)", "/ 0.85)");
    assert!(!diff(&moved).is_empty());
    // a dropped number
    assert!(!diff(&same.replace(" * 0.03", "")).is_empty());
    // the same changes, marked, are excused
    let marked = swapped.replace("k);", &format!("k); {CHANGE} why"));
    assert!(diff(&marked).is_empty(), "{:?}", diff(&marked));
    let dropped = same.replace(" * 0.03;", &format!("; {CHANGE} no light"));
    assert!(diff(&dropped).is_empty(), "{:?}", diff(&dropped));
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
