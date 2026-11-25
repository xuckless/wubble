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
    /// window's tab, a peek, a pane of a split on screen).
    WindowOpened(bool),
    /// The page's answer to going fullscreen (the error's text if it refused).
    Fullscreen(Result<(), String>),
    /// The page left fullscreen (Escape, its own button, a navigation).
    FullscreenLeft(TabId),
    /// The window closed (its close button, return to tab, the window manager).
    WindowClosed,
    /// The user closed the window for good (its close button, the window manager): the video
    /// stops, as in Chrome. Back to the tab ([`PipInput::TabActivated`], the model's return)
    /// is not this: the video plays on.
    Close,
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
    /// `kit.js`: pause the tab's main video.
    Pause(TabId),
    /// `Command::OpenPip`.
    OpenWindow(TabId),
    /// `Command::ClosePip`.
    CloseWindow(TabId),
    Toast(&'static str),
}

/// A Main window's panes going from `before` to `after` (one tab, or a split's): each pane left
/// is a switch away from it, in pane order (the machine takes the first whose video plays
/// aloud: one window at a time), then each pane come to is a return. `playing`: the tab's video
/// plays with sound.
pub fn switched(
    before: &[TabId],
    after: &[TabId],
    auto: bool,
    playing: impl Fn(TabId) -> bool,
) -> Vec<PipInput> {
    let left = before
        .iter()
        .filter(|t| !after.contains(t))
        .map(|&from| PipInput::TabSwitched {
            from,
            auto,
            playing: playing(from),
        });
    let come = after
        .iter()
        .filter(|t| !before.contains(t))
        .map(|&t| PipInput::TabActivated(t));
    left.chain(come).collect()
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
        (Pip::Entering { tab } | Pip::On { tab }, Close) => (
            Pip::Off,
            vec![Pause(tab), ExitFullscreen(tab), CloseWindow(tab)],
        ),
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
    fn closing_for_good_pauses_the_video_and_returning_does_not() {
        let entering = Pip::Entering { tab: A };
        for s in [on(), entering] {
            assert_eq!(
                transition(s, Close),
                (Pip::Off, vec![Pause(A), ExitFullscreen(A), CloseWindow(A)])
            );
            // back to the tab (the model's return closes the window; coming back to the tab):
            // the video keeps playing
            for e in [
                transition(s, WindowClosed).1,
                transition(s, TabActivated(A)).1,
                transition(s, Request(A)).1,
                transition(s, FullscreenLeft(A)).1,
                transition(s, TabClosed(A)).1,
            ] {
                assert!(!e.contains(&Pause(A)), "{e:?}");
            }
        }
        // Nothing is out: nothing to stop. The window the machine closed reports in after.
        assert_eq!(transition(Pip::Off, Close), (Pip::Off, vec![]));
        let (s, _) = transition(on(), Close);
        assert_eq!(transition(s, WindowClosed), (Pip::Off, vec![]));
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

    /// The owner's case: a video playing in one pane of a three-pane split pops out when the
    /// split is left for another tab, and goes back when the split comes back.
    #[test]
    fn leaving_a_split_pops_out_its_playing_pane() {
        const C: TabId = TabId(3);
        const D: TabId = TabId(4);
        let run = |state: Pip, inputs: Vec<PipInput>| {
            inputs.into_iter().fold((state, vec![]), |(s, mut fx), i| {
                let (s, more) = transition(s, i);
                fx.extend(more);
                (s, fx)
            })
        };
        let plays = |tab: TabId| move |t: TabId| t == tab;
        /// (name, panes before, panes after, the tab that plays aloud, the state then, effects)
        type Row = (
            &'static str,
            Vec<TabId>,
            Vec<TabId>,
            TabId,
            Pip,
            Vec<PipEffect>,
        );
        let rows: Vec<Row> = vec![
            (
                "one page left",
                vec![A],
                vec![B],
                A,
                Pip::Entering { tab: A },
                vec![OpenWindow(A)],
            ),
            (
                "the split's middle pane plays",
                vec![A, B, C],
                vec![D],
                B,
                Pip::Entering { tab: B },
                vec![OpenWindow(B)],
            ),
            (
                "its last pane plays",
                vec![A, B, C],
                vec![D],
                C,
                Pip::Entering { tab: C },
                vec![OpenWindow(C)],
            ),
            ("nothing plays", vec![A, B, C], vec![D], D, Pip::Off, vec![]),
            (
                "a pane focused in the split is no switch",
                vec![A, B, C],
                vec![A, B, C],
                B,
                Pip::Off,
                vec![],
            ),
            (
                "a pane closed out of the split, another stays",
                vec![A, B, C],
                vec![A, C],
                C,
                Pip::Off,
                vec![],
            ),
        ];
        for (name, before, after, playing, state, fx) in rows {
            let inputs = switched(&before, &after, true, plays(playing));
            assert_eq!(run(Pip::Off, inputs), (state, fx), "{name}");
        }
        // the setting off: nothing pops out
        let inputs = switched(&[A, B, C], &[D], false, plays(B));
        assert_eq!(run(Pip::Off, inputs), (Pip::Off, vec![]));
        // the split back with its pane out: the pane returns
        let inputs = switched(&[D], &[A, B, C], true, plays(B));
        assert_eq!(
            run(Pip::On { tab: B }, inputs),
            (Pip::Off, vec![ExitFullscreen(B), CloseWindow(B)])
        );
        // two playing panes: the first goes, one window at a time
        let inputs = switched(&[A, B, C], &[D], true, |t| t != D);
        assert_eq!(
            run(Pip::Off, inputs),
            (Pip::Entering { tab: A }, vec![OpenWindow(A)])
        );
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
