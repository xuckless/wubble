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
/// popup setting only judges requests without one), so a blocked site is held as an undecided
/// one is: for the address pill (Chromium shows its blocked pop-up icon for both). With
/// `glance` off (the `glance.enabled` setting), a peek is a foreground tab.
///
/// Chromium's own blocker is off (wubble-engine's `disable-popup-blocking` switch), so requests
/// without a gesture reach this table; were it on, they would never arrive.
///
/// `abuse`: the click's gesture does not count, since it landed on an invisible overlay or on
/// a link to another site (`click_guard::opened`), or the filter lists name the window
/// (`$popup`); the request is held as one without a gesture is, unless the site is allowed.
pub fn decide(
    opened: Opened,
    gesture: bool,
    site: Option<Decision>,
    glance: bool,
    abuse: bool,
) -> Outcome {
    let row = |g: bool| {
        ROWS.iter()
            .find(|(o, rg, _)| *o == opened && *rg == g)
            .map_or(Outcome::Blocked, |(_, _, out)| *out)
    };
    let out = match site {
        Some(Decision::Allow) => row(true),
        Some(Decision::Block) | None => row(gesture && !abuse),
    };
    match out {
        Outcome::Glance if !glance => Outcome::Tab { foreground: true },
        out => out,
    }
}

/// Whether the request becomes a browser Chromium creates beside its opener (a real pop-up:
/// `window.opener`, the handle `window.open` returns, named targets, `postMessage` and
/// `window.close()` all work), which the host then places: what the table opens at once, a tab
/// or a Popup window. A peek, and anything held back, is opened by the host itself later, as a
/// fresh page with no opener (Chromium's blocked pop-ups too reopen without one).
pub fn creates_browser(opened: Opened, gesture: bool) -> bool {
    matches!(
        decide(opened, gesture, None, true, false),
        Outcome::Tab { .. } | Outcome::PopupWindow
    )
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
            assert_eq!(decide(o, g, None, true, false), out, "{o:?} gesture={g}");
        }
        assert_eq!(
            decide(Opened::PopupWindow, true, None, true, false),
            Outcome::PopupWindow
        );
        assert_eq!(decide(Opened::Window, true, None, true, false), Outcome::Glance);
        assert_eq!(
            decide(Opened::Window, true, None, false, false),
            Outcome::Tab { foreground: true },
            "peeks off: a tab"
        );
        for o in ALL {
            assert_eq!(decide(o, false, None, true, false), Outcome::Blocked, "{o:?}");
        }
    }

    #[test]
    fn only_what_opens_at_once_is_a_browser() {
        assert!(creates_browser(Opened::PopupWindow, true));
        assert!(creates_browser(Opened::ForegroundTab, true));
        assert!(creates_browser(Opened::BackgroundTab, true));
        assert!(!creates_browser(Opened::Window, true), "a peek");
        for o in ALL {
            assert!(
                !creates_browser(o, false),
                "{o:?} without a gesture is held"
            );
        }
    }

    #[test]
    fn an_abused_click_counts_as_none() {
        for o in ALL {
            assert_eq!(
                decide(o, true, None, true, true),
                Outcome::Blocked,
                "{o:?}: held for the pill"
            );
            assert_eq!(
                decide(o, true, Some(Decision::Block), true, true),
                Outcome::Blocked,
                "{o:?}"
            );
            assert_eq!(
                decide(o, true, Some(Decision::Allow), true, true),
                decide(o, true, None, true, false),
                "{o:?}: an allowed site opens"
            );
            assert_eq!(decide(o, false, None, true, true), Outcome::Blocked, "{o:?}");
        }
    }

    #[test]
    fn the_site_choice_wins() {
        for o in ALL {
            assert_eq!(
                decide(o, false, Some(Decision::Allow), true, false),
                decide(o, true, None, true, false),
                "allowed: as with a gesture"
            );
            assert_eq!(
                decide(o, false, Some(Decision::Block), true, false),
                Outcome::Blocked
            );
            assert_eq!(
                decide(o, true, Some(Decision::Block), true, false),
                decide(o, true, None, true, false),
                "a click still opens"
            );
        }
    }
}
