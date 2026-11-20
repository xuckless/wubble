//! `press_key`'s key names: Playwright's spelling (`Enter`, `ArrowDown`, `Control+A`,
//! `Meta+Shift+T`; the names follow the DOM `KeyboardEvent.key` values Playwright documents for
//! `keyboard.press`) read into the shell's own [`Chord`], which the host turns into key events
//! as a US keyboard makes them. Design reference only, no code copied.

use crate::keys::{Chord, ChordError};

/// Playwright's modifier and key names to [`Chord::parse`]'s.
const ALIASES: &[(&str, &str)] = &[
    ("Control", "Ctrl"),
    ("Meta", "Cmd"),
    ("ArrowLeft", "Left"),
    ("ArrowRight", "Right"),
    ("ArrowUp", "Up"),
    ("ArrowDown", "Down"),
    (" ", "Space"),
];

/// The chord a Playwright key name stands for.
pub fn chord_of(name: &str) -> Result<Chord, ChordError> {
    // `Shift++` is the plus key with Shift; `+` alone is the plus key.
    let (head, plus) = match name.strip_suffix("++") {
        Some(head) => (head, true),
        None if name == "+" => ("", true),
        None => (name, false),
    };
    let mut parts: Vec<&str> = if head.is_empty() {
        vec![]
    } else {
        head.split('+')
            .map(|p| {
                ALIASES
                    .iter()
                    .find(|(from, _)| from.eq_ignore_ascii_case(p))
                    .map_or(p, |(_, to)| to)
            })
            .collect()
    };
    if plus {
        parts.push("Plus");
    }
    Chord::parse(&parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Key;

    #[test]
    fn playwright_names_read_as_chords() {
        let c = |s: &str| chord_of(s).unwrap();
        assert_eq!(c("Enter").key, Key::Enter);
        assert_eq!(c("ArrowDown").key, Key::Down);
        let a = c("Control+A");
        assert!(a.ctrl && !a.shift);
        assert_eq!(a.key, Key::Char('a'));
        let t = c("Meta+Shift+T");
        assert!(t.cmd && t.shift);
        assert_eq!(c(" ").key, Key::Space);
        assert_eq!(c("+").key, Key::Char('+'));
        assert!(c("Control++").ctrl);
        assert_eq!(c("F5").key, Key::F(5));
    }

    #[test]
    fn unknown_names_are_refused() {
        for bad in ["", "Hyper", "Control+", "A+B"] {
            assert!(chord_of(bad).is_err(), "{bad}");
        }
    }
}
