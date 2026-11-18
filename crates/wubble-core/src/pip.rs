//! Picture-in-picture, wubble's own: CEF's windowless browsers have no picture-in-picture
//! (Chromium's lives in `//chrome`'s `VideoOverlayWindowViews`), so the tab's video is put in
//! element fullscreen (the page shows only the video, letterboxed) and the tab's own frames are
//! shown in a small window of their own. [`Pip`] is which tab is popped out, its one owner, and
//! [`transition`] the only way it changes.
//!
//! Design reference for behaviour only (no code): Firefox `PictureInPictureChild.sys.mjs`
//! (MPL-2.0, the largest playing video wins) and Chromium's auto picture-in-picture on tab
//! switch.

use crate::tab::TabId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pip {
    #[default]
    Off,
    /// The tab's video was asked to go fullscreen; waiting for the answer.
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
    /// The page's answer to going fullscreen (the error's text if it refused).
    Fullscreen(Result<(), String>),
    /// The window could (`true`) or could not be opened.
    WindowOpened(bool),
    /// The page left fullscreen (Escape, its own button, a navigation).
    FullscreenLeft(TabId),
    /// The user closed the window.
    WindowClosed,
    TabClosed(TabId),
    /// The user left `from` for another tab. `auto`: the "picture-in-picture on tab switch"
    /// setting; `playing`: `from`'s video plays.
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
    OpenWindow(TabId),
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
        ) => (Pip::Entering { tab }, vec![EnterFullscreen(tab)]),
        // Asked again for the tab that is out: back in.
        (Pip::On { tab }, Request(t)) if t == tab => {
            (Pip::Off, vec![ExitFullscreen(tab), CloseWindow(tab)])
        }
        // One window at a time; a second request waits for the first to end.
        (state @ (Pip::Entering { .. } | Pip::On { .. }), Request(_)) => (state, vec![]),
        (Pip::Entering { tab }, Fullscreen(Ok(()))) => (Pip::On { tab }, vec![OpenWindow(tab)]),
        (Pip::Entering { .. }, Fullscreen(Err(_))) => (
            Pip::Off,
            vec![Toast("This video won't play in picture-in-picture")],
        ),
        (Pip::On { tab }, WindowOpened(false)) => (
            Pip::Off,
            vec![
                ExitFullscreen(tab),
                Toast("Picture-in-picture has no window to open"),
            ],
        ),
        (Pip::On { tab }, FullscreenLeft(t)) if t == tab => (Pip::Off, vec![CloseWindow(tab)]),
        (Pip::Entering { tab }, FullscreenLeft(t)) if t == tab => (Pip::Off, vec![]),
        (Pip::On { tab }, WindowClosed) => (Pip::Off, vec![ExitFullscreen(tab)]),
        (Pip::On { tab }, TabClosed(t)) if t == tab => (Pip::Off, vec![CloseWindow(tab)]),
        (Pip::Entering { tab }, TabClosed(t)) if t == tab => (Pip::Off, vec![]),
        (Pip::On { tab }, TabActivated(t)) if t == tab => {
            (Pip::Off, vec![ExitFullscreen(tab), CloseWindow(tab)])
        }
        (Pip::Entering { tab }, TabActivated(t)) if t == tab => {
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
    fn a_request_goes_fullscreen_then_opens_the_window() {
        let (s, e) = transition(Pip::Off, Request(A));
        assert_eq!((s, e), (Pip::Entering { tab: A }, vec![EnterFullscreen(A)]));
        let (s, e) = transition(s, Fullscreen(Ok(())));
        assert_eq!((s, e), (on(), vec![OpenWindow(A)]));
        assert_eq!(s.tab(), Some(A));
        // Asked again: back in.
        assert_eq!(
            transition(s, Request(A)),
            (Pip::Off, vec![ExitFullscreen(A), CloseWindow(A)])
        );
    }

    #[test]
    fn refusals_end_it() {
        let entering = Pip::Entering { tab: A };
        let (s, e) = transition(entering, Fullscreen(Err("no".into())));
        assert_eq!(s, Pip::Off);
        assert!(matches!(&e[..], [Toast(_)]));
        let (s, e) = transition(on(), WindowOpened(false));
        assert_eq!(s, Pip::Off);
        assert!(matches!(&e[..], [ExitFullscreen(A), Toast(_)]));
        assert_eq!(transition(on(), WindowOpened(true)), (on(), vec![]));
    }

    #[test]
    fn leaving_fullscreen_closing_the_window_or_the_tab_ends_it() {
        assert_eq!(
            transition(on(), FullscreenLeft(A)),
            (Pip::Off, vec![CloseWindow(A)])
        );
        assert_eq!(transition(on(), FullscreenLeft(B)), (on(), vec![]));
        assert_eq!(
            transition(on(), WindowClosed),
            (Pip::Off, vec![ExitFullscreen(A)])
        );
        assert_eq!(
            transition(on(), TabClosed(A)),
            (Pip::Off, vec![CloseWindow(A)])
        );
        assert_eq!(transition(on(), TabClosed(B)), (on(), vec![]));
        let entering = Pip::Entering { tab: A };
        assert_eq!(transition(entering, TabClosed(A)), (Pip::Off, vec![]));
        assert_eq!(transition(entering, FullscreenLeft(A)), (Pip::Off, vec![]));
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
            (Pip::Entering { tab: A }, vec![EnterFullscreen(A)])
        );
        // Coming back closes it.
        assert_eq!(
            transition(on(), TabActivated(A)),
            (Pip::Off, vec![ExitFullscreen(A), CloseWindow(A)])
        );
        assert_eq!(
            transition(Pip::Entering { tab: A }, TabActivated(A)),
            (Pip::Off, vec![ExitFullscreen(A)])
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
