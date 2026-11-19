//! What wubble does with a page's request to open a window or tab (`window.open`, a
//! `target=_blank` link): one table and one pure function.
//!
//! The rule is the popup blocker's of Chromium (`components/blocked_content/
//! popup_blocker_tab_helper.cc`) and Firefox (`dom.disable_open_during_load`): without a user
//! gesture a page opens nothing unless the site is allowed to. Design reference only, no code
//! copied. A request with window features (a size) is Chromium's `NEW_POPUP` disposition and
//! gets a Popup window; everything else becomes a tab.

use crate::site::Decision;

/// What the page asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// A link or `window.open` into a new foreground tab.
    ForegroundTab,
    /// Middle click, Ctrl+click.
    BackgroundTab,
    /// `window.open` with window features (width, height, `popup`).
    PopupWindow,
    /// Shift+click or `window.open` asking for a new window without features.
    Window,
}

/// What wubble does with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A new tab beside the opener; `foreground` shows it.
    Tab { foreground: bool },
    /// A Popup window showing a new tab.
    PopupWindow,
    /// Nothing opens; the address pill offers to allow it.
    Blocked,
    /// A peek over the opener's page (`model::Glance`): Shift+click on a link. A tab instead when
    /// the setting is off.
    Glance,
}

/// Each request with and without a user gesture. Every (`Opened`, gesture) pair has one row.
pub const ROWS: &[(Opened, bool, Outcome)] = &[
    (
        Opened::ForegroundTab,
        true,
        Outcome::Tab { foreground: true },
    ),
    (Opened::ForegroundTab, false, Outcome::Blocked),
    (
        Opened::BackgroundTab,
        true,
        Outcome::Tab { foreground: false },
    ),
    (Opened::BackgroundTab, false, Outcome::Blocked),
    (Opened::PopupWindow, true, Outcome::PopupWindow),
    (Opened::PopupWindow, false, Outcome::Blocked),
    // a page asking for a whole new window gets a peek (a tab with peeks off): windows are
    // the user's. In practice this is Shift+click on a link.
    (Opened::Window, true, Outcome::Glance),
    (Opened::Window, false, Outcome::Blocked),
];

/// The table's answer, then the site's remembered choice: an allowed site opens without a
/// gesture what a gesture would have opened. A click always opens (Chromium's rule too: the
/// popup setting only judges requests without one), so a blocked site differs from an
/// undecided one only in that the pill does not offer to allow it again. With `glance` off (the
/// `glance.enabled` setting), a peek is a foreground tab.
pub fn decide(opened: Opened, gesture: bool, site: Option<Decision>, glance: bool) -> Outcome {
    let row = |g: bool| {
        ROWS.iter()
            .find(|(o, rg, _)| *o == opened && *rg == g)
            .map_or(Outcome::Blocked, |(_, _, out)| *out)
    };
    let out = match site {
        Some(Decision::Allow) => row(true),
        Some(Decision::Block) | None => row(gesture),
    };
    match out {
        Outcome::Glance if !glance => Outcome::Tab { foreground: true },
        out => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Opened; 4] = [
        Opened::ForegroundTab,
        Opened::BackgroundTab,
        Opened::PopupWindow,
        Opened::Window,
    ];

    #[test]
    fn every_request_has_exactly_one_row() {
        for o in ALL {
            for g in [true, false] {
                let n = ROWS
                    .iter()
                    .filter(|(ro, rg, _)| *ro == o && *rg == g)
                    .count();
                assert_eq!(n, 1, "{o:?} gesture={g}");
            }
        }
        assert_eq!(ROWS.len(), ALL.len() * 2);
    }

    #[test]
    fn the_table() {
        for &(o, g, out) in ROWS {
            assert_eq!(decide(o, g, None, true), out, "{o:?} gesture={g}");
        }
        assert_eq!(
            decide(Opened::PopupWindow, true, None, true),
            Outcome::PopupWindow
        );
        assert_eq!(decide(Opened::Window, true, None, true), Outcome::Glance);
        assert_eq!(
            decide(Opened::Window, true, None, false),
            Outcome::Tab { foreground: true },
            "peeks off: a tab"
        );
        for o in ALL {
            assert_eq!(decide(o, false, None, true), Outcome::Blocked, "{o:?}");
        }
    }

    #[test]
    fn the_site_choice_wins() {
        for o in ALL {
            assert_eq!(
                decide(o, false, Some(Decision::Allow), true),
                decide(o, true, None, true),
                "allowed: as with a gesture"
            );
            assert_eq!(
                decide(o, false, Some(Decision::Block), true),
                Outcome::Blocked
            );
            assert_eq!(
                decide(o, true, Some(Decision::Block), true),
                decide(o, true, None, true),
                "a click still opens"
            );
        }
    }
}
