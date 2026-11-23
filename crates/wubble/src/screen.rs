//! Full screen: the window's own (F11, the green light) and the page's
//! (`element.requestFullscreen()`), told apart by one enum. Pure; the host
//! executes [`ScreenEffect`]s.
//!
//! Page fullscreen hides the sidebar and the window frame, makes the window
//! full screen if it was not, and shows "Press Esc to exit full screen" for
//! a moment; Esc or F11 asks the page to leave (CEF `ExitFullscreen`), and
//! the page's own `FullscreenChanged(false)` ends it, restoring the window
//! as it was. Windowless CEF is Alloy style, so the host does all of this
//! itself (cefclient `OnFullscreenModeChange` for windowless browsers,
//! BSD-3-Clause; behaviour only).
//!
//! The window's mode is owned by the window: whenever the screen is not in
//! page fullscreen, [`observe`] rereads it, so the green light toggling the
//! window behind the shell's back is never stale here.
//!
//! Each window has its own [`Screen`] (as each Chromium `Browser` has its own
//! `FullscreenController`): a page fullscreen is its window's only, and a page
//! leaving goes to the window that holds it ([`page_owner`]).

use wubble_core::tab::TabId;
use wubble_core::window::WindowId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Windowed,
    /// The window is full screen; the shell shows as usual.
    Window,
    /// `tab`'s page is full screen. `restore_window`: the window was made
    /// full screen for it and goes back to a window when the page leaves.
    Page { tab: TabId, restore_window: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenEvent {
    /// F11 / the Fullscreen action.
    Toggle,
    /// Esc (only page fullscreen takes it).
    Escape,
    PageEntered(TabId),
    /// The tab's page left (CEF confirmed), or the tab stopped being the
    /// visible one, or its view closed. Another tab's leaving is ignored.
    PageLeft(TabId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenEffect {
    SetWindowFullscreen(bool),
    /// Hide (true) or show the sidebar and frame; the page fills the window.
    HideChrome(bool),
    /// Ask the tab's page to leave fullscreen (`View::exit_fullscreen`).
    ExitPage(TabId),
    /// "Press Esc to exit full screen".
    Hint,
}

/// The state with the window's mode reread, outside page fullscreen.
pub fn observe(state: Screen, window_fullscreen: bool) -> Screen {
    match state {
        Screen::Page { .. } => state,
        _ if window_fullscreen => Screen::Window,
        _ => Screen::Windowed,
    }
}

/// `(next, effects, consumed)`: `consumed` is false when the event is not
/// the screen's (Esc outside page fullscreen goes to the page).
pub fn transition(state: Screen, event: ScreenEvent) -> (Screen, Vec<ScreenEffect>, bool) {
    use {Screen::*, ScreenEffect::*, ScreenEvent::*};
    match (state, event) {
        (Windowed, Toggle) => (Window, vec![SetWindowFullscreen(true)], true),
        (Window, Toggle) => (Windowed, vec![SetWindowFullscreen(false)], true),
        (Page { tab, .. }, Toggle | Escape) => (state, vec![ExitPage(tab)], true),
        (Windowed | Window, Escape) => (state, vec![], false),
        (Windowed, PageEntered(tab)) => (
            Page {
                tab,
                restore_window: true,
            },
            vec![SetWindowFullscreen(true), HideChrome(true), Hint],
            true,
        ),
        (Window, PageEntered(tab)) => (
            Page {
                tab,
                restore_window: false,
            },
            vec![HideChrome(true), Hint],
            true,
        ),
        (Page { .. }, PageEntered(_)) => (state, vec![], true),
        (
            Page {
                tab,
                restore_window,
            },
            PageLeft(left),
        ) if tab == left => {
            let (next, mut effects) = if restore_window {
                (Windowed, vec![SetWindowFullscreen(false)])
            } else {
                (Window, vec![])
            };
            effects.push(HideChrome(false));
            (next, effects, true)
        }
        // A late confirmation of a page fullscreen already left, or another
        // tab's.
        (_, PageLeft(_)) => (state, vec![], true),
    }
}

/// The window whose page fullscreen is `tab`'s, of each window's screen.
pub fn page_owner(
    screens: impl IntoIterator<Item = (WindowId, Screen)>,
    tab: TabId,
) -> Option<WindowId> {
    screens
        .into_iter()
        .find(|&(_, s)| matches!(s, Screen::Page { tab: t, .. } if t == tab))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::{Screen::*, ScreenEffect::*, ScreenEvent::*, *};

    const T: TabId = TabId(1);
    const OTHER: TabId = TabId(2);

    fn page(restore_window: bool) -> Screen {
        Page {
            tab: T,
            restore_window,
        }
    }

    #[test]
    fn every_pair() {
        let events = [Toggle, Escape, PageEntered(T), PageLeft(T), PageLeft(OTHER)];
        let table = [
            (
                Windowed,
                Toggle,
                Window,
                vec![SetWindowFullscreen(true)],
                true,
            ),
            (
                Window,
                Toggle,
                Windowed,
                vec![SetWindowFullscreen(false)],
                true,
            ),
            (page(true), Toggle, page(true), vec![ExitPage(T)], true),
            (page(false), Toggle, page(false), vec![ExitPage(T)], true),
            (Windowed, Escape, Windowed, vec![], false),
            (Window, Escape, Window, vec![], false),
            (page(true), Escape, page(true), vec![ExitPage(T)], true),
            (page(false), Escape, page(false), vec![ExitPage(T)], true),
            (
                Windowed,
                PageEntered(T),
                page(true),
                vec![SetWindowFullscreen(true), HideChrome(true), Hint],
                true,
            ),
            (
                Window,
                PageEntered(T),
                page(false),
                vec![HideChrome(true), Hint],
                true,
            ),
            (page(true), PageEntered(T), page(true), vec![], true),
            (page(false), PageEntered(T), page(false), vec![], true),
            (Windowed, PageLeft(T), Windowed, vec![], true),
            (Window, PageLeft(T), Window, vec![], true),
            (
                page(true),
                PageLeft(T),
                Windowed,
                vec![SetWindowFullscreen(false), HideChrome(false)],
                true,
            ),
            (
                page(false),
                PageLeft(T),
                Window,
                vec![HideChrome(false)],
                true,
            ),
            (Windowed, PageLeft(OTHER), Windowed, vec![], true),
            (Window, PageLeft(OTHER), Window, vec![], true),
            (page(true), PageLeft(OTHER), page(true), vec![], true),
            (page(false), PageLeft(OTHER), page(false), vec![], true),
        ];
        assert_eq!(table.len(), 4 * events.len());
        for (state, event, next, effects, consumed) in table {
            assert_eq!(
                transition(state, event),
                (next, effects, consumed),
                "{state:?} {event:?}"
            );
        }
    }

    #[test]
    fn page_fullscreen_from_a_fullscreen_window_restores_the_window() {
        let (s, _, _) = transition(Window, PageEntered(T));
        let (s, fx, _) = transition(s, Escape);
        assert_eq!(fx, [ExitPage(T)]);
        let (s, fx, _) = transition(s, PageLeft(T));
        assert_eq!((s, fx), (Window, vec![HideChrome(false)]));
    }

    const WA: WindowId = WindowId(1);
    const WB: WindowId = WindowId(2);

    #[test]
    fn a_page_belongs_to_the_window_that_holds_it() {
        let other = Page {
            tab: OTHER,
            restore_window: false,
        };
        // (name, each window's screen, the page's tab, the window that holds it)
        type OwnerRow<'a> = (&'a str, &'a [(WindowId, Screen)], TabId, Option<WindowId>);
        let rows: &[OwnerRow] = &[
            (
                "one window holds it",
                &[(WA, page(true)), (WB, Windowed)],
                T,
                Some(WA),
            ),
            (
                "the second window's",
                &[(WA, Windowed), (WB, page(false))],
                T,
                Some(WB),
            ),
            (
                "both in page fullscreen: T",
                &[(WA, page(true)), (WB, other)],
                T,
                Some(WA),
            ),
            (
                "both in page fullscreen: OTHER",
                &[(WA, page(true)), (WB, other)],
                OTHER,
                Some(WB),
            ),
            (
                "a tab no window holds",
                &[(WA, page(true)), (WB, Windowed)],
                OTHER,
                None,
            ),
            (
                "windowed and full screen hold no tab",
                &[(WA, Windowed), (WB, Window)],
                T,
                None,
            ),
            ("no windows", &[], T, None),
        ];
        for &(name, screens, tab, want) in rows {
            assert_eq!(page_owner(screens.iter().copied(), tab), want, "{name}");
        }
    }

    /// Two windows, each its own screen: A's page and B's page go full screen and leave apart,
    /// and B's keys never reach A's page.
    #[test]
    fn independent_windows() {
        let (a, _, _) = transition(Windowed, PageEntered(T));
        let (b, _, _) = transition(Window, PageEntered(OTHER));
        assert!(
            matches!((a, b), (Page { tab: T, .. }, Page { tab: OTHER, .. })),
            "both full screen"
        );
        // B's Escape and F11 ask B's page, never A's
        assert_eq!(transition(b, Escape).1, [ExitPage(OTHER)]);
        assert_eq!(transition(b, Toggle).1, [ExitPage(OTHER)]);
        // A's page leaves in A (its owner): A restores its window, B stays
        let screens = [(WA, a), (WB, b)];
        assert_eq!(page_owner(screens, T), Some(WA));
        let (a, fx, _) = transition(a, PageLeft(T));
        assert_eq!(
            (a, fx),
            (
                Windowed,
                vec![SetWindowFullscreen(false), HideChrome(false)]
            )
        );
        assert_eq!(
            page_owner([(WA, a), (WB, b)], OTHER),
            Some(WB),
            "B still holds its page"
        );
        // F11 on a windowed B while A's page is full screen: B's window only
        assert_eq!(transition(Windowed, Toggle).1, [SetWindowFullscreen(true)]);
        let (b, fx, _) = transition(b, PageLeft(OTHER));
        assert_eq!((b, fx), (Window, vec![HideChrome(false)]));
    }

    #[test]
    fn observe_follows_the_window_outside_page_fullscreen() {
        assert_eq!(observe(Windowed, true), Window);
        assert_eq!(observe(Window, false), Windowed);
        assert_eq!(observe(page(true), false), page(true));
    }
}
