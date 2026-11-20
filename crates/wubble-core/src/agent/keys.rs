//! `press_key`'s key names: Playwright's spelling (`Enter`, `ArrowDown`, `Control+A`,
//! `Meta+Shift+T`; the names follow the DOM `KeyboardEvent.key` values Playwright documents for
//! `keyboard.press`) read into the shell's own [`Chord`], which the host turns into key events
//! as a US keyboard makes them. Design reference only, no code copied.

use crate::action::Action;
use crate::keys::{Chord, ChordError, Route};

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

/// A history step or reload of the session's own tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionNav {
    Back,
    Forward,
    Reload,
}

/// What `press_key` does with a chord, by what the keymap binds it to (`binding`): the keys an
/// agent presses reach its page as the owner's would, except that wubble's own shortcuts never
/// act on the window, the owner's tabs or their history for an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPlan {
    /// Not a browser shortcut: to the page.
    Page,
    /// The page first; the browser's action for it (`Action`) does not run for an agent.
    PageNotBrowser(Action),
    /// A history step or reload: wubble does it on the session's tab, as `navigate` does.
    Navigate(SessionNav),
    /// A shortcut of the window (tabs, Spaces, the address bar, closed tabs): nothing is
    /// pressed.
    Refused(Action),
}

/// The plan for a chord the keymap binds as `binding`.
pub fn plan(binding: Option<(Action, Route)>) -> KeyPlan {
    let Some((action, route)) = binding else {
        return KeyPlan::Page;
    };
    match action {
        Action::Back => KeyPlan::Navigate(SessionNav::Back),
        Action::Forward => KeyPlan::Navigate(SessionNav::Forward),
        Action::Reload | Action::HardReload => KeyPlan::Navigate(SessionNav::Reload),
        // Blink runs the edit chords itself where the page takes them
        a if a.is_edit() => KeyPlan::Page,
        a => match route {
            Route::PageFirst => KeyPlan::PageNotBrowser(a),
            Route::Shell => KeyPlan::Refused(a),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{Key, Keymap, Platform};

    #[test]
    fn browser_chords_are_planned_by_their_binding() {
        let map = Keymap::defaults(Platform::Linux);
        let p = |name: &str| plan(map.action_for(chord_of(name).unwrap()));
        assert_eq!(p("Enter"), KeyPlan::Page);
        assert_eq!(p("Control+A"), KeyPlan::Page);
        assert_eq!(p("Alt+ArrowLeft"), KeyPlan::Navigate(SessionNav::Back));
        assert_eq!(p("Alt+ArrowRight"), KeyPlan::Navigate(SessionNav::Forward));
        assert_eq!(p("F5"), KeyPlan::Navigate(SessionNav::Reload));
        assert_eq!(
            p("Control+L"),
            KeyPlan::PageNotBrowser(Action::FocusAddress)
        );
        assert_eq!(
            p("Control+Shift+T"),
            KeyPlan::Refused(Action::ReopenClosedTab)
        );
        assert_eq!(p("Control+T"), KeyPlan::Refused(Action::NewTab));
    }

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
