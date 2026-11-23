//! The surface lock (notes int-0.6.1/surface-store-plan.md): every shell surface is a row of
//! `wubble_core::overlay::ROWS`, and whether it shows is its owner's, set from the host: no
//! surface owns its own visibility in Slint. The shell `.slint` files (everything outside
//! `ui/kit/`) are scanned for what breaks that:
//! - `binding` (app.slint): a surface's `open`, `shown`, `visible`, `active` or `if` reads a
//!   property Slint itself writes (an `in-out` one, or one assigned in the file) or another
//!   element's (`zone.has-hover`), not only what the host set;
//! - `hover-shown`: `has-hover` feeding `open`, `shown`, `visible` or `active` (a hover is an
//!   enter or leave the host hears, then answers);
//! - `timer`: a `Timer` in a shell file (a surface's time is the host's);
//! - `open-write`: a `root.*-open` written from Slint (two writers of one state);
//! - `escape`: Escape handled outside the kit (it is the overlays' rule, `overlay::Esc`).
//!
//! Today's violations are recorded in `surface_lock_baseline.txt` (file, rule, token, count),
//! held as the kit lock's are (`lock::Lock::ratchet`): the baseline only shrinks, as the steps
//! that move the surfaces into the store clear them. `SURFACE_LOCK_WRITE=1 cargo test -p
//! wubble-ui --test surface_lock` rewrites it, only when no count went up. Two `GlassKeys` sharing
//! a value, a row naming a key `GlassKeys` lacks, or a surface's key no row names, fail outright.
//!
//! Pure string scans; no timing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use wubble_core::overlay::ROWS;

mod lock;
use lock::{Counts, Lock, code, statements, ui_dir, ui_files};

const SURFACE_LOCK: Lock = Lock {
    name: "surface_lock",
    env: "SURFACE_LOCK_WRITE",
    header: "# The surface lock's recorded violations (tests/surface_lock.rs): file, rule, token, count.\n\
             # Counts only go down: a step that moves a surface into the store lowers them here.\n",
    hint: "a surface shows by what the host sets: a row of wubble_core::overlay::ROWS",
};

/// The statements a surface shows by.
const SHOWN_BY: [&str; 4] = ["open", "shown", "visible", "active"];

/// The `a.b` references in an expression: (element or `root`, property).
fn references(expression: &str) -> Vec<(&str, &str)> {
    expression
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .filter_map(|word| {
            let mut parts = word.split('.');
            let (element, property) = (parts.next()?, parts.next()?);
            let lower = element.chars().next()?.is_ascii_lowercase();
            (lower && !property.is_empty()).then_some((element, property))
        })
        .collect()
}

/// The root properties a line of code assigns (`root.name = ...`; not `==`, `=>` or `<=>`).
fn assigned(code: &str) -> Vec<String> {
    code.match_indices("root.")
        .filter_map(|(i, _)| {
            let name: String = code[i + 5..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            let after = code[i + 5 + name.len()..].trim_start();
            let compared = after.starts_with("==") || after.starts_with("=>");
            (after.starts_with('=') && !compared).then_some(name)
        })
        .collect()
}

/// app.slint's root properties Slint itself writes: the `in-out` ones and those assigned.
fn slint_written(text: &str) -> BTreeSet<String> {
    let mut written = BTreeSet::new();
    for line in text.lines() {
        let code = code(line);
        if let Some(rest) = code.trim().strip_prefix("in-out property <")
            && let Some((_, name)) = rest.split_once('>')
        {
            let name = name.trim().split([':', ';', ' ']).next().unwrap_or("");
            written.insert(name.to_string());
        }
        written.extend(assigned(&code));
    }
    written
}

/// The rules one line of a shell file breaks, as (rule, token). `written`: app.slint's
/// Slint-written root properties (`None` for any other file: `binding` is app.slint's rule).
fn surface_violations(
    line: &str,
    written: Option<&BTreeSet<String>>,
) -> Vec<(&'static str, String)> {
    let code = code(line);
    let mut out = Vec::new();
    let mut shown_by: Vec<&str> = statements(&code)
        .into_iter()
        .filter(|(name, _)| SHOWN_BY.contains(name))
        .map(|(_, expression)| expression)
        .collect();
    // an element under `if`: `if condition: Element {`
    if code.trim_start().starts_with("if ") && code.contains(':') {
        shown_by.push(&code);
    }
    for expression in shown_by {
        if expression.contains("has-hover") {
            out.push(("hover-shown", "has-hover".to_string()));
        }
        let Some(written) = written else {
            continue;
        };
        for (element, property) in references(expression) {
            if element != "root" || written.contains(property) {
                out.push(("binding", format!("{element}.{property}")));
            }
        }
    }
    if code.contains("Timer {") || code.contains("Timer{") {
        out.push(("timer", "Timer".to_string()));
    }
    for name in assigned(&code) {
        if name.ends_with("-open") {
            out.push(("open-write", format!("root.{name}")));
        }
    }
    for _ in code.matches("Key.Escape") {
        out.push(("escape", "Key.Escape".to_string()));
    }
    out
}

fn violations() -> Counts {
    let mut counts = BTreeMap::new();
    for (rel, text) in ui_files()
        .into_iter()
        .filter(|(rel, _)| !rel.starts_with("kit/"))
    {
        let written = (rel == "app.slint").then(|| slint_written(&text));
        for line in text.lines() {
            for (rule, token) in surface_violations(line, written.as_ref()) {
                *counts
                    .entry((rel.clone(), rule.to_string(), token))
                    .or_insert(0) += 1;
            }
        }
    }
    counts
}

#[test]
fn the_surface_lock_holds() {
    SURFACE_LOCK.ratchet(&violations());
}

#[test]
fn the_scanner_finds_each_rule() {
    let written: BTreeSet<String> = ["downloads-open".to_string()].into();
    let rules = |line: &str| {
        surface_violations(line, Some(&written))
            .into_iter()
            .map(|(rule, token)| format!("{rule}:{token}"))
            .collect::<Vec<_>>()
    };
    #[rustfmt::skip]
    let rows: &[(&str, &str, &[&str])] = &[
        ("a host-set property", "        open: root.menu.open;", &[]),
        ("an element under a host-set if", "    if root.drag-zone > 0: DropMark {", &[]),
        ("a global and an enum", "    visible: GlassLayer.scene-active || root.page == Kind.start;", &[]),
        ("a property Slint writes", "        open: root.downloads-open;", &["binding:root.downloads-open"]),
        ("another element's hover", "    shown: area.has-hover;", &["hover-shown:has-hover", "binding:area.has-hover"]),
        ("an element under a Slint-written if", "    if root.downloads-open: Thing {", &["binding:root.downloads-open"]),
        ("a statement if is no element", "        if root.downloads-open {", &[]),
        ("a timer", "    hide := Timer {", &["timer:Timer"]),
        ("an open written", "            root.downloads-open = false;", &["open-write:root.downloads-open"]),
        ("an open compared", "    if root.spaces-open == true {", &[]),
        ("Escape in a handler", "        if event.text == Key.Escape {", &["escape:Key.Escape"]),
        ("in a comment or a string", "    text: \"shown: a.has-hover Timer {\"; // root.x-open = 1", &[]),
    ];
    for (name, line, want) in rows {
        assert_eq!(rules(*line), *want, "{name}");
    }
    let text = "in-out property <bool> spaces-open: false;\n\
                in property <bool> menu;\n\
                root.peek-x = 2;\n\
                if root.menu == 1 {";
    let want: BTreeSet<String> = ["peek-x".to_string(), "spaces-open".to_string()].into();
    assert_eq!(slint_written(text), want);
}

/// `GlassKeys`' entries in a model.slint text, as (name, value).
fn glass_keys(text: &str) -> Vec<(String, i64)> {
    let start = text
        .find("export global GlassKeys {")
        .expect("GlassKeys in model.slint");
    let body = &text[start..];
    let end = body.find("\n}").expect("GlassKeys ends");
    body[..end]
        .lines()
        .filter_map(|line| {
            let code = code(line);
            let rest = code.trim().strip_prefix("out property <int> ")?;
            let (name, value) = rest.split_once(':')?;
            let value = value.trim().trim_end_matches(';').trim().parse().ok()?;
            Some((name.trim().to_string(), value))
        })
        .collect()
}

/// The glass of the chrome and the scene's base: no surface's.
const CHROME_GLASS: [&str; 4] = ["sidebar", "address", "favourites", "overlays"];

/// What is wrong between `GlassKeys` (`keys`) and the rows' glass (`glass`): two keys sharing a
/// value (they would take each other's glass away), a row naming a key `GlassKeys` lacks, a
/// surface's key no row names (a surface without its row).
fn glass_faults(keys: &[(String, i64)], glass: &[&str]) -> Vec<String> {
    let mut faults = vec![];
    let mut seen = BTreeMap::new();
    for (name, value) in keys {
        if let Some(other) = seen.insert(value, name) {
            faults.push(format!(
                "GlassKeys.{name} and GlassKeys.{other} are both {value}"
            ));
        }
    }
    let names: BTreeSet<&str> = keys.iter().map(|(name, _)| name.as_str()).collect();
    for name in glass.iter().filter(|name| !names.contains(*name)) {
        faults.push(format!("a row's glass {name} is no GlassKeys key"));
    }
    for name in names
        .iter()
        .filter(|n| !CHROME_GLASS.contains(n) && !glass.contains(n))
    {
        faults.push(format!("GlassKeys.{name} is no row's glass"));
    }
    faults
}

/// `GlassKeys` and `ROWS` agree: each surface's glass is a key of its own, each surface key a
/// row's.
#[test]
fn every_glass_key_is_a_row() {
    let text =
        fs::read_to_string(ui_dir().join("components/model.slint")).expect("read model.slint");
    let keys = glass_keys(&text);
    assert!(keys.len() > 10, "GlassKeys read as {keys:?}");
    let glass: Vec<&str> = ROWS.iter().filter_map(|row| row.glass).collect();
    assert_eq!(glass_faults(&keys, &glass), Vec::<String>::new());
}

#[test]
fn the_glass_check_finds_each_fault() {
    let text = "export global GlassKeys {\n    out property <int> sidebar: 1;\n    \
                out property <int> find: 2;\n    out property <int> menu: 3;\n}";
    let keys = glass_keys(text);
    let dup = [keys.clone(), vec![("print".to_string(), 2)]].concat();
    #[rustfmt::skip]
    let rows: &[(&str, &[(String, i64)], &[&str], &[&str])] = &[
        ("clean", &keys, &["find", "menu", "find"], &[]),
        ("two keys alike", &dup, &["find", "menu", "print"], &["GlassKeys.print and GlassKeys.find are both 2"]),
        ("a row names a missing key", &keys, &["find", "menu", "toast"], &["a row's glass toast is no GlassKeys key"]),
        ("a key no row names", &keys, &["find"], &["GlassKeys.menu is no row's glass"]),
    ];
    for (name, keys, glass, want) in rows {
        assert_eq!(glass_faults(keys, glass), *want, "{name}");
    }
}
