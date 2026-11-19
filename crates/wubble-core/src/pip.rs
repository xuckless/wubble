//! Picture-in-picture, wubble's own: CEF's windowless browsers have no picture-in-picture
//! (Chromium's lives in `//chrome`'s `VideoOverlayWindowViews`), so the tab's video is put in
//! element fullscreen (the page shows only the video, letterboxed) and the tab's own frames are
//! shown in a small window of their own (`Shown::Pip`, owned by the model). [`Pip`] is which
//! tab's fullscreen belongs to picture-in-picture, its one owner, and [`transition`] the only way
//! it changes. The window opens with the request, before the page is asked, so the tab is on
//! screen (and its view visible) when it is asked: a tab switch has hidden it by then. The page
//! is asked only once the model has opened the window (`WindowOpened`): it refuses some tabs.
//!
//! Design reference for behaviour only (no code): Firefox `PictureInPictureChild.sys.mjs`
//! (MPL-2.0, the largest playing video wins) and Chromium's auto picture-in-picture on tab
//! switch.

use crate::tab::TabId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pip {
    #[default]
    Off,
    /// The window was asked for (`WindowOpened` says whether the model opened it), then the
    /// tab's video was asked to go fullscreen; waiting for the answer.
    Entering { tab: TabId },
    /// The tab's frames show in the picture-in-picture window.
    On { tab: TabId },
}

impl Pip {
    pub fn tab(self) -> Option<TabId> {
        match self {
            Pip::Off => None,
            Pip::Entering { tab } | Pip::On { tab } => Some(tab),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipInput {
    /// The overlay's button, or the action: pop `tab`'s video out (again: back in).
    Request(TabId),
    /// Whether the model opened the window `OpenWindow` asked for (it refuses a Little or Popup
    /// window's tab, a peek, a split pane).
    WindowOpened(bool),
    /// The page's answer to going fullscreen (the error's text if it refused).
    Fullscreen(Result<(), String>),
    /// The page left fullscreen (Escape, its own button, a navigation).
    FullscreenLeft(TabId),
    /// The window closed (its close button, return to tab, the window manager).
    WindowClosed,
    TabClosed(TabId),
    /// The user left `from` for another tab. `auto`: the "picture-in-picture on tab switch"
    /// setting; `playing`: `from`'s video plays with sound.
    TabSwitched {
        from: TabId,
        auto: bool,
        playing: bool,
    },
    /// The user came back to `tab`.
    TabActivated(TabId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipEffect {
    /// `kit.js`: the main video's `requestFullscreen()` as a user gesture.
    EnterFullscreen(TabId),
    ExitFullscreen(TabId),
    /// `Command::OpenPip`.
    OpenWindow(TabId),
    /// `Command::ClosePip`.
    CloseWindow(TabId),
    Toast(&'static str),
}

pub fn transition(state: Pip, input: PipInput) -> (Pip, Vec<PipEffect>) {
    use PipEffect::*;
    use PipInput::*;
    match (state, input) {
        (Pip::Off, Request(tab))
        | (
            Pip::Off,
            TabSwitched {
                from: tab,
                auto: true,
                playing: true,
            },
        ) => (Pip::Entering { tab }, vec![OpenWindow(tab)]),
        (Pip::Entering { tab }, WindowOpened(true)) => (state, vec![EnterFullscreen(tab)]),
        (Pip::Entering { .. }, WindowOpened(false)) => (Pip::Off, vec![]),
        // Asked again for the tab that is out, or back on it: back in.
        (Pip::On { tab }, Request(t) | TabActivated(t))
        | (Pip::Entering { tab }, TabActivated(t))
            if t == tab =>
        {
            (Pip::Off, vec![ExitFullscreen(tab), CloseWindow(tab)])
        }
        // One window at a time; a second request waits for the first to end.
        (state @ (Pip::Entering { .. } | Pip::On { .. }), Request(_)) => (state, vec![]),
        (Pip::Entering { tab }, Fullscreen(Ok(()))) => (Pip::On { tab }, vec![]),
        (Pip::Entering { tab }, Fullscreen(Err(_))) => (
            Pip::Off,
            vec![
                CloseWindow(tab),
                Toast("This video won't play in picture-in-picture"),
            ],
        ),
        (Pip::Entering { tab } | Pip::On { tab }, FullscreenLeft(t) | TabClosed(t)) if t == tab => {
            (Pip::Off, vec![CloseWindow(tab)])
        }
        (Pip::Entering { tab } | Pip::On { tab }, WindowClosed) => {
            (Pip::Off, vec![ExitFullscreen(tab)])
        }
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PipEffect::*;
    use PipInput::*;

    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    fn on() -> Pip {
        Pip::On { tab: A }
    }

    #[test]
    fn a_request_opens_the_window_and_goes_fullscreen() {
        let (s, e) = transition(Pip::Off, Request(A));
        assert_eq!((s, e), (Pip::Entering { tab: A }, vec![OpenWindow(A)]));
        let (s, e) = transition(s, WindowOpened(true));
        assert_eq!((s, e), (Pip::Entering { tab: A }, vec![EnterFullscreen(A)]));
        let (s, e) = transition(s, Fullscreen(Ok(())));
        assert_eq!((s, e), (on(), vec![]));
        assert_eq!(s.tab(), Some(A));
        // Asked again: back in.
        assert_eq!(
            transition(s, Request(A)),
            (Pip::Off, vec![ExitFullscreen(A), CloseWindow(A)])
        );
    }

    #[test]
    fn a_refusal_closes_the_window() {
        let entering = Pip::Entering { tab: A };
        let (s, e) = transition(entering, Fullscreen(Err("no".into())));
        assert_eq!(s, Pip::Off);
        assert!(matches!(&e[..], [CloseWindow(A), Toast(_)]));
        // A late answer after it ended changes nothing.
        assert_eq!(transition(Pip::Off, Fullscreen(Ok(()))), (Pip::Off, vec![]));
    }

    #[test]
    fn a_window_the_model_refuses_ends_it_before_the_page_is_asked() {
        let (s, _) = transition(Pip::Off, Request(A));
        let (s, e) = transition(s, WindowOpened(false));
        assert_eq!(
            (s, e),
            (Pip::Off, vec![]),
            "no fullscreen, nothing to close"
        );
        // The next request is taken.
        assert_eq!(
            transition(s, Request(B)),
            (Pip::Entering { tab: B }, vec![OpenWindow(B)])
        );
        // Only while entering.
        assert_eq!(transition(on(), WindowOpened(false)), (on(), vec![]));
        assert_eq!(transition(Pip::Off, WindowOpened(true)), (Pip::Off, vec![]));
    }

    #[test]
    fn leaving_fullscreen_closing_the_window_or_the_tab_ends_it() {
        let entering = Pip::Entering { tab: A };
        for s in [on(), entering] {
            assert_eq!(
                transition(s, FullscreenLeft(A)),
                (Pip::Off, vec![CloseWindow(A)])
            );
            assert_eq!(transition(s, FullscreenLeft(B)), (s, vec![]));
            // The window went (its close button, return to tab): the page leaves fullscreen.
            assert_eq!(
                transition(s, WindowClosed),
                (Pip::Off, vec![ExitFullscreen(A)])
            );
            assert_eq!(
                transition(s, TabClosed(A)),
                (Pip::Off, vec![CloseWindow(A)])
            );
            assert_eq!(transition(s, TabClosed(B)), (s, vec![]));
        }
        // The window closed because the machine asked: it is already off.
        assert_eq!(transition(Pip::Off, WindowClosed), (Pip::Off, vec![]));
    }

    #[test]
    fn tab_switches_pop_out_only_with_the_setting_and_a_playing_video() {
        for (auto, playing) in [(false, true), (true, false), (false, false)] {
            let input = TabSwitched {
                from: A,
                auto,
                playing,
            };
            assert_eq!(transition(Pip::Off, input), (Pip::Off, vec![]));
        }
        let input = TabSwitched {
            from: A,
            auto: true,
            playing: true,
        };
        assert_eq!(
            transition(Pip::Off, input),
            (Pip::Entering { tab: A }, vec![OpenWindow(A)])
        );
        // Coming back closes it.
        assert_eq!(
            transition(on(), TabActivated(A)),
            (Pip::Off, vec![ExitFullscreen(A), CloseWindow(A)])
        );
        assert_eq!(
            transition(Pip::Entering { tab: A }, TabActivated(A)),
            (Pip::Off, vec![ExitFullscreen(A), CloseWindow(A)])
        );
        assert_eq!(transition(on(), TabActivated(B)), (on(), vec![]));
    }

    #[test]
    fn never_two_windows() {
        assert_eq!(transition(on(), Request(B)), (on(), vec![]));
        let entering = Pip::Entering { tab: A };
        assert_eq!(transition(entering, Request(A)), (entering, vec![]));
        let switched = TabSwitched {
            from: B,
            auto: true,
            playing: true,
        };
        assert_eq!(transition(on(), switched), (on(), vec![]));
        assert_eq!(transition(on(), Fullscreen(Ok(()))), (on(), vec![]));
    }
}
