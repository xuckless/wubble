//! The Slint squash (`ui/components/squash.slint`) plays the keys of `motion::SQUASH_KEYS`, the
//! one table: a text check of its three arrays and its length token. Pure string reads.

use std::fs;
use std::path::Path;

use wubble_core::motion::{SQUASH_KEYS, SQUASH_MS};

fn array(text: &str, name: &str) -> Vec<f64> {
    let line = text
        .lines()
        .find(|l| l.contains(&format!("> {name}: [")))
        .unwrap_or_else(|| panic!("no {name} array"));
    let inner = line.split('[').nth(2).and_then(|s| s.split(']').next());
    inner
        .expect("an array literal")
        .split(',')
        .map(|n| n.trim().parse().expect("a number"))
        .collect()
}

#[test]
fn the_slint_squash_keys_are_the_core_ones() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let squash = fs::read_to_string(ui.join("components/squash.slint")).unwrap();
    let col = |f: fn(&(f64, f64, f64)) -> f64| SQUASH_KEYS.iter().map(f).collect::<Vec<_>>();
    assert_eq!(array(&squash, "kt"), col(|k| k.0));
    assert_eq!(array(&squash, "kx"), col(|k| k.1));
    assert_eq!(array(&squash, "ky"), col(|k| k.2));
    let tokens = fs::read_to_string(ui.join("app-tokens.slint")).unwrap();
    assert!(tokens.contains(&format!("squash-ms: {SQUASH_MS}ms;")));
}
