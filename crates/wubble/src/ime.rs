//! IME for the web view: winit's composition events mapped to the engine's
//! composition calls. Pure; the host forwards winit `WindowEvent::Ime` here
//! while the page has focus and runs the [`ImeCall`]s.
//!
//! Slint 1.19 routes IME only to a focused `TextInput`
//! (i-slint-backend-winit 1.19 `winitwindowadapter.rs`: `Ime::Preedit` /
//! `Ime::Commit` become composition key events for that item, and
//! `input_method_request` turns its properties into `set_ime_allowed` /
//! `set_ime_cursor_area`); nothing reaches a non-text item. So the host
//! sees winit's events before Slint (`on_winit_window_event`), enables IME
//! on the raw window while the page has focus and places the candidate
//! window from the engine's `ImeBounds` itself. The preedit
//! goes to CEF with one underline over all of it (welding, change 58), as Chromium's
//! own input method does on Linux.
//!
//! winit reports preedit cursors as UTF-8 byte offsets; CEF takes UTF-16
//! code units ([`utf16_range`]). Pattern: cefclient's OSR IME handler
//! (`tests/cefclient/browser/osr_ime_handler_win.cc`, BSD-3-Clause), whose
//! calls welding already wraps.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ime {
    #[default]
    Idle,
    Composing,
}

/// winit's `Ime` events, as plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeInput {
    /// The composition text and its cursor (byte offsets), `None` to hide
    /// the cursor.
    Preedit(String, Option<(usize, usize)>),
    Commit(String),
    /// The input method was switched off (focus left, say).
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeCall {
    /// `View::ime_set_composition`; the selection in UTF-16 units.
    SetComposition { text: String, selection: (u32, u32) },
    /// `View::ime_commit`: commits, or inserts the text if nothing composes.
    Commit(String),
    /// `View::ime_cancel`.
    Cancel,
}

pub fn transition(state: Ime, input: ImeInput) -> (Ime, Option<ImeCall>) {
    use {Ime::*, ImeInput::*};
    match (state, input) {
        (_, Preedit(text, cursor)) if !text.is_empty() => {
            let at = cursor.unwrap_or((text.len(), text.len()));
            let selection = utf16_range(&text, at);
            (Composing, Some(ImeCall::SetComposition { text, selection }))
        }
        // An emptied preedit ends the composition (winit sends one before a
        // commit, and when the user deletes everything composed).
        (Composing, Preedit(..) | Disabled) => (Idle, Some(ImeCall::Cancel)),
        (Idle, Preedit(..) | Disabled) => (Idle, None),
        (_, Commit(text)) => (Idle, Some(ImeCall::Commit(text))),
    }
}

/// A byte range of `text` in UTF-16 code units. Offsets inside a character
/// or past the end count up to the character boundary before them.
pub fn utf16_range(text: &str, (start, end): (usize, usize)) -> (u32, u32) {
    let units = |byte: usize| -> u32 {
        let mut b = byte.min(text.len());
        while !text.is_char_boundary(b) {
            b -= 1;
        }
        text[..b].encode_utf16().count() as u32
    };
    (units(start), units(end))
}

#[cfg(test)]
mod tests {
    use super::{Ime::*, ImeInput::*, *};

    #[test]
    fn utf16_counts_units_not_bytes() {
        // "日本" is 6 bytes, 2 units; "😀" is 4 bytes, 2 units.
        let t = "a日本😀b";
        assert_eq!(utf16_range(t, (0, 1)), (0, 1));
        assert_eq!(utf16_range(t, (1, 7)), (1, 3));
        assert_eq!(utf16_range(t, (7, 11)), (3, 5));
        assert_eq!(utf16_range(t, (12, 12)), (6, 6));
        // Inside a character, or past the end.
        assert_eq!(utf16_range(t, (2, 99)), (1, 6));
    }

    #[test]
    fn preedit_composes_and_commit_inserts() {
        let (s, call) = transition(Idle, Preedit("にほ".into(), Some((3, 3))));
        assert_eq!(
            (s, call),
            (
                Composing,
                Some(ImeCall::SetComposition {
                    text: "にほ".into(),
                    selection: (1, 1)
                })
            )
        );
        // No cursor: at the end.
        let (_, call) = transition(s, Preedit("にほん".into(), None));
        assert_eq!(
            call,
            Some(ImeCall::SetComposition {
                text: "にほん".into(),
                selection: (3, 3)
            })
        );
        let (s, call) = transition(s, Preedit(String::new(), None));
        assert_eq!((s, call), (Idle, Some(ImeCall::Cancel)));
        let (s, call) = transition(s, Commit("日本".into()));
        assert_eq!((s, call), (Idle, Some(ImeCall::Commit("日本".into()))));
    }

    #[test]
    fn disabled_cancels_only_a_composition() {
        assert_eq!(transition(Idle, Disabled), (Idle, None));
        assert_eq!(
            transition(Composing, Disabled),
            (Idle, Some(ImeCall::Cancel))
        );
        assert_eq!(transition(Idle, Preedit(String::new(), None)), (Idle, None));
    }
}
