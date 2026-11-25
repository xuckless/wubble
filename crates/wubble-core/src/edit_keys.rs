//! macOS's standard text-editing key bindings as data: a chord and the editing commands
//! AppKit gives a text view for it.
//!
//! In a window Chromium gets these from AppKit (`interpretKeyEvents:` calls
//! `-[RenderWidgetHostViewCocoa doCommandBySelector:]`) and sends them to Blink with the
//! key-down (`ForwardKeyboardEventWithCommands`). CEF's windowless `SendKeyEvent` sends none,
//! and Blink's own Mac table (`editing_behavior.cc`) leaves the Cmd and Ctrl editing keys to
//! AppKit, so Cmd+Delete in a page's text field did nothing.
//!
//! A command is the selector without its colon, as Chromium names it for Blink
//! (`RenderWidgetHostViewMacEditCommandHelper::CommandNameForSelector`); Blink reads the
//! name case-insensitively. The rows are the defaults of AppKit's `StandardKeyBinding.dict`
//! (Apple, "Text System Defaults and Key Bindings") for the keys wubble forwards; facts
//! only, nothing copied.

use crate::keys::Chord;

/// Chords (as [`Chord::parse`] reads them) and their commands, run in order.
pub type EditBindings = &'static [(&'static str, &'static [&'static str])];

/// macOS: Option is `Alt`, Fn+Delete is `Delete` (forward delete). Every binding is a row;
/// a new one is a new row, not a branch in the engine.
#[rustfmt::skip]
pub const MAC_EDIT_BINDINGS: EditBindings = &[
    ("Cmd+Backspace", &["deleteToBeginningOfLine"]),
    ("Alt+Backspace", &["deleteWordBackward"]),
    ("Cmd+Delete", &["deleteToEndOfLine"]),
    ("Alt+Delete", &["deleteWordForward"]),
    ("Cmd+Left", &["moveToLeftEndOfLine"]),
    ("Cmd+Shift+Left", &["moveToLeftEndOfLineAndModifySelection"]),
    ("Cmd+Right", &["moveToRightEndOfLine"]),
    ("Cmd+Shift+Right", &["moveToRightEndOfLineAndModifySelection"]),
    ("Alt+Left", &["moveWordLeft"]),
    ("Alt+Shift+Left", &["moveWordLeftAndModifySelection"]),
    ("Alt+Right", &["moveWordRight"]),
    ("Alt+Shift+Right", &["moveWordRightAndModifySelection"]),
    ("Cmd+Up", &["moveToBeginningOfDocument"]),
    ("Cmd+Shift+Up", &["moveToBeginningOfDocumentAndModifySelection"]),
    ("Cmd+Down", &["moveToEndOfDocument"]),
    ("Cmd+Shift+Down", &["moveToEndOfDocumentAndModifySelection"]),
    ("Alt+Up", &["moveBackward", "moveToBeginningOfParagraph"]),
    ("Alt+Shift+Up", &["moveParagraphBackwardAndModifySelection"]),
    ("Alt+Down", &["moveForward", "moveToEndOfParagraph"]),
    ("Alt+Shift+Down", &["moveParagraphForwardAndModifySelection"]),
    // Emacs
    ("Ctrl+A", &["moveToBeginningOfParagraph"]),
    ("Ctrl+Shift+A", &["moveToBeginningOfParagraphAndModifySelection"]),
    ("Ctrl+E", &["moveToEndOfParagraph"]),
    ("Ctrl+Shift+E", &["moveToEndOfParagraphAndModifySelection"]),
    ("Ctrl+K", &["deleteToEndOfParagraph"]),
    ("Ctrl+D", &["deleteForward"]),
    ("Ctrl+H", &["deleteBackward"]),
    ("Ctrl+F", &["moveForward"]),
    ("Ctrl+Shift+F", &["moveForwardAndModifySelection"]),
    ("Ctrl+B", &["moveBackward"]),
    ("Ctrl+Shift+B", &["moveBackwardAndModifySelection"]),
    ("Ctrl+N", &["moveDown"]),
    ("Ctrl+Shift+N", &["moveDownAndModifySelection"]),
    ("Ctrl+P", &["moveUp"]),
    ("Ctrl+Shift+P", &["moveUpAndModifySelection"]),
];

/// The commands `bindings` gives `chord`; none for a chord without a row.
pub fn edit_commands(bindings: EditBindings, chord: Chord) -> &'static [&'static str] {
    bindings
        .iter()
        .find(|(c, _)| Chord::parse(c).ok() == Some(chord))
        .map(|&(_, commands)| commands)
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    #[test]
    fn every_row_parses_once() {
        let chords: Vec<Chord> = MAC_EDIT_BINDINGS.iter().map(|(c, _)| ch(c)).collect();
        for (i, c) in chords.iter().enumerate() {
            assert!(!chords[..i].contains(c), "{c} bound twice");
        }
        assert!(MAC_EDIT_BINDINGS.iter().all(|(_, cmds)| !cmds.is_empty()));
    }

    #[rustfmt::skip]
    #[test]
    fn chords_give_appkit_s_commands() {
        let cases: &[(&str, &str, &[&str])] = &[
            ("owner's bug", "Cmd+Backspace", &["deleteToBeginningOfLine"]),
            ("option delete", "Alt+Backspace", &["deleteWordBackward"]),
            ("cmd fn delete", "Cmd+Delete", &["deleteToEndOfLine"]),
            ("option fn delete", "Alt+Delete", &["deleteWordForward"]),
            ("cmd left", "Cmd+Left", &["moveToLeftEndOfLine"]),
            ("cmd shift right", "Cmd+Shift+Right", &["moveToRightEndOfLineAndModifySelection"]),
            ("option left", "Alt+Left", &["moveWordLeft"]),
            ("option shift right", "Alt+Shift+Right", &["moveWordRightAndModifySelection"]),
            ("cmd up", "Cmd+Up", &["moveToBeginningOfDocument"]),
            ("cmd shift down", "Cmd+Shift+Down", &["moveToEndOfDocumentAndModifySelection"]),
            ("option up, two commands", "Alt+Up", &["moveBackward", "moveToBeginningOfParagraph"]),
            ("ctrl a", "Ctrl+A", &["moveToBeginningOfParagraph"]),
            ("ctrl shift e", "Ctrl+Shift+E", &["moveToEndOfParagraphAndModifySelection"]),
            ("ctrl k", "Ctrl+K", &["deleteToEndOfParagraph"]),
            ("ctrl d", "Ctrl+D", &["deleteForward"]),
            ("ctrl h", "Ctrl+H", &["deleteBackward"]),
            ("ctrl f", "Ctrl+F", &["moveForward"]),
            ("ctrl b", "Ctrl+B", &["moveBackward"]),
            ("ctrl n", "Ctrl+N", &["moveDown"]),
            ("ctrl p", "Ctrl+P", &["moveUp"]),
            ("plain backspace is Blink's", "Backspace", &[]),
            ("plain left is Blink's", "Left", &[]),
            ("shift backspace unbound", "Shift+Backspace", &[]),
            ("cmd a is the shell's select all", "Cmd+A", &[]),
            ("ctrl alt a unbound", "Ctrl+Alt+A", &[]),
            ("ctrl shift k unbound", "Ctrl+Shift+K", &[]),
        ];
        for (name, chord, want) in cases {
            assert_eq!(edit_commands(MAC_EDIT_BINDINGS, ch(chord)), *want, "{name}");
        }
    }

    #[test]
    fn no_bindings_no_commands() {
        assert_eq!(edit_commands(&[], ch("Cmd+Backspace")), &[] as &[&str]);
    }
}
