//! Navigation state of one view and the events the host receives about it.
//!
//! Pure: no CEF types. [`crate::view::View`] translates welding's callbacks
//! and the host's commands into [`NavInput`]s, runs [`transition`] and
//! executes the [`NavEffect`]s. The host only ever sees [`WebEvent`]s.

use welding::CursorShape;

/// The pointer shape a page asks for. Exactly the shapes Slint can show
/// (i-slint-common 1.19 `BuiltInMouseCursor`). `cursor as i32` indexes the
/// `cursors` table in `wubble-ui/ui/app.slint`, which lists them in this order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum Cursor {
    #[default]
    Default,
    /// Hidden (CSS `cursor: none`).
    None,
    /// The link hand.
    Pointer,
    Text,
    Wait,
    Progress,
    Crosshair,
    Move,
    NotAllowed,
    Help,
    Grab,
    Grabbing,
    ResizeNs,
    ResizeEw,
    ResizeNeSw,
    ResizeNwSe,
}

impl From<CursorShape> for Cursor {
    /// Shapes Slint has no cursor for fall back to the nearest one it has.
    fn from(shape: CursorShape) -> Self {
        match shape {
            CursorShape::Default => Self::Default,
            CursorShape::Pointer => Self::Pointer,
            CursorShape::Text => Self::Text,
            CursorShape::Wait => Self::Wait,
            CursorShape::Crosshair => Self::Crosshair,
            CursorShape::Move | CursorShape::ResizeAll => Self::Move,
            CursorShape::NotAllowed => Self::NotAllowed,
            CursorShape::Help => Self::Help,
            CursorShape::Progress => Self::Progress,
            CursorShape::ResizeNs => Self::ResizeNs,
            CursorShape::ResizeEw => Self::ResizeEw,
            CursorShape::ResizeNeSw => Self::ResizeNeSw,
            CursorShape::ResizeNwSe => Self::ResizeNwSe,
            CursorShape::Grab => Self::Grab,
            CursorShape::Grabbing => Self::Grabbing,
            // welding's name for CEF's `CT_NONE`.
            CursorShape::Custom(name) if name == "none" => Self::None,
            // Zoom, other custom names, and shapes a newer welding adds.
            _ => Self::Default,
        }
    }
}

/// What the host learns about a view, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebEvent {
    UrlChanged(String),
    TitleChanged(String),
    LoadingChanged {
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    },
    /// The page's first icon URL, empty when it has none.
    FaviconChanged(String),
    CursorChanged(Cursor),
    /// `window.open` or a `target=_blank` link. CEF opened nothing itself.
    PopupRequested {
        url: String,
    },
    /// The renderer process died. The view reloads the page once by itself.
    RendererGone,
}

/// What the view shows, as far as the host cares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub favicon: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub cursor: Cursor,
}

/// Where a view's navigation is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// CEF has not started loading anything yet.
    #[default]
    Blank,
    /// `recovering`: this load follows a crash, so a second crash before it
    /// finishes is not reloaded again.
    Loading {
        recovering: bool,
    },
    Loaded,
    /// The renderer died. CEF's next `Loading { loading: true }` ends it.
    Crashed,
}

/// A view's navigation: where it is and what it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavState {
    pub phase: Phase,
    pub page: Page,
}

/// CEF callbacks (translated in `view.rs`) and host commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavInput {
    // From CEF.
    Address(String),
    Title(String),
    Favicon(String),
    Loading {
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    },
    RendererTerminated,
    Popup(String),
    Cursor(Cursor),
    // From the host.
    Load(String),
    Back,
    Forward,
    Reload,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavEffect {
    Emit(WebEvent),
    Load(String),
    Back,
    Forward,
    Reload,
    Stop,
    /// Nudge CEF to paint: a replacement renderer paints nothing until asked
    /// (welding `request_repaint`).
    Repaint,
}

/// Every (state, input) pair not matched leaves the state alone and does
/// nothing. A dead renderer is reloaded once; if it dies again before that
/// reload has finished loading, the view stays crashed.
pub fn transition(state: NavState, input: NavInput) -> (NavState, Vec<NavEffect>) {
    use NavEffect::Emit;
    let NavState { phase, page } = state;
    let loading = matches!(phase, Phase::Loading { .. });
    // A command that loads something; after a crash it also needs a repaint.
    let command = |effect: NavEffect| {
        let mut effects = vec![effect];
        if phase == Phase::Crashed {
            effects.push(NavEffect::Repaint);
        }
        effects
    };
    let (page, effects) = match input {
        NavInput::Address(url) if url != page.url => (
            Page {
                url: url.clone(),
                ..page
            },
            vec![Emit(WebEvent::UrlChanged(url))],
        ),
        NavInput::Title(title) if title != page.title => (
            Page {
                title: title.clone(),
                ..page
            },
            vec![Emit(WebEvent::TitleChanged(title))],
        ),
        NavInput::Favicon(url) if url != page.favicon => (
            Page {
                favicon: url.clone(),
                ..page
            },
            vec![Emit(WebEvent::FaviconChanged(url))],
        ),
        NavInput::Cursor(cursor) if cursor != page.cursor => (
            Page { cursor, ..page },
            vec![Emit(WebEvent::CursorChanged(cursor))],
        ),
        NavInput::Loading {
            loading: now_loading,
            can_go_back,
            can_go_forward,
        } => {
            let next = match (phase, now_loading) {
                (Phase::Crashed | Phase::Loading { recovering: true }, true) => {
                    Phase::Loading { recovering: true }
                }
                (_, true) => Phase::Loading { recovering: false },
                // A crashed view stays crashed until something loads again.
                (Phase::Crashed, false) => Phase::Crashed,
                (_, false) => Phase::Loaded,
            };
            let changed = (loading, page.can_go_back, page.can_go_forward)
                != (now_loading, can_go_back, can_go_forward);
            let effects = if changed {
                vec![Emit(WebEvent::LoadingChanged {
                    loading: now_loading,
                    can_go_back,
                    can_go_forward,
                })]
            } else {
                vec![]
            };
            let page = Page {
                can_go_back,
                can_go_forward,
                ..page
            };
            return (NavState { phase: next, page }, effects);
        }
        NavInput::RendererTerminated if phase != Phase::Crashed => {
            let mut effects = vec![Emit(WebEvent::RendererGone)];
            if phase != (Phase::Loading { recovering: true }) {
                effects.extend([NavEffect::Reload, NavEffect::Repaint]);
            }
            let state = NavState {
                phase: Phase::Crashed,
                page,
            };
            return (state, effects);
        }
        NavInput::Popup(url) => (page, vec![Emit(WebEvent::PopupRequested { url })]),
        NavInput::Load(url) => (page, command(NavEffect::Load(url))),
        NavInput::Reload => (page, command(NavEffect::Reload)),
        NavInput::Back if page.can_go_back => (page, command(NavEffect::Back)),
        NavInput::Forward if page.can_go_forward => (page, command(NavEffect::Forward)),
        NavInput::Stop if loading => (page, vec![NavEffect::Stop]),
        _ => (page, vec![]),
    };
    (NavState { phase, page }, effects)
}

#[cfg(test)]
mod tests {
    use super::{NavEffect as E, NavInput as I, *};

    fn page() -> Page {
        Page {
            url: "https://a/".into(),
            title: "A".into(),
            favicon: "https://a/i.ico".into(),
            can_go_back: true,
            can_go_forward: false,
            cursor: Cursor::Text,
        }
    }

    fn at(phase: Phase) -> NavState {
        NavState {
            phase,
            page: page(),
        }
    }

    fn run(start: NavState, inputs: Vec<NavInput>) -> (NavState, Vec<NavEffect>) {
        inputs
            .into_iter()
            .fold((start, vec![]), |(state, mut all), input| {
                let (next, effects) = transition(state, input);
                all.extend(effects);
                (next, all)
            })
    }

    fn loading(loading: bool, can_go_back: bool, can_go_forward: bool) -> NavInput {
        I::Loading {
            loading,
            can_go_back,
            can_go_forward,
        }
    }

    #[test]
    fn a_first_load_reports_each_change_once() {
        let (state, effects) = run(
            NavState::default(),
            vec![
                loading(true, false, false),
                I::Address("data:a".into()),
                I::Address("data:a".into()),
                I::Title("a".into()),
                I::Title("a".into()),
                loading(false, false, false),
                loading(false, false, false),
            ],
        );
        assert_eq!(state.phase, Phase::Loaded);
        assert_eq!(state.page.url, "data:a");
        assert_eq!(state.page.title, "a");
        assert_eq!(
            effects,
            vec![
                E::Emit(WebEvent::LoadingChanged {
                    loading: true,
                    can_go_back: false,
                    can_go_forward: false
                }),
                E::Emit(WebEvent::UrlChanged("data:a".into())),
                E::Emit(WebEvent::TitleChanged("a".into())),
                E::Emit(WebEvent::LoadingChanged {
                    loading: false,
                    can_go_back: false,
                    can_go_forward: false
                }),
            ]
        );
    }

    #[test]
    fn history_flags_gate_back_and_forward() {
        let start = NavState {
            phase: Phase::Loaded,
            page: Page::default(),
        };
        assert_eq!(transition(start.clone(), I::Back).1, vec![]);
        assert_eq!(transition(start.clone(), I::Forward).1, vec![]);
        let (state, effects) = run(start, vec![loading(false, true, true)]);
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::LoadingChanged {
                loading: false,
                can_go_back: true,
                can_go_forward: true
            })]
        );
        assert_eq!(transition(state.clone(), I::Back).1, vec![E::Back]);
        assert_eq!(transition(state, I::Forward).1, vec![E::Forward]);
    }

    #[test]
    fn a_crash_is_reported_once_and_reloaded_once() {
        let (state, effects) = run(
            at(Phase::Loaded),
            vec![I::RendererTerminated, I::RendererTerminated],
        );
        assert_eq!(state, at(Phase::Crashed));
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::RendererGone), E::Reload, E::Repaint]
        );
        // Commands while crashed repaint too.
        assert_eq!(transition(state, I::Reload).1, vec![E::Reload, E::Repaint]);
    }

    #[test]
    fn a_crash_during_the_recovery_load_is_not_reloaded() {
        let (state, effects) = run(
            at(Phase::Loaded),
            vec![
                I::RendererTerminated,
                loading(true, true, false),
                I::RendererTerminated,
            ],
        );
        assert_eq!(state.phase, Phase::Crashed);
        assert_eq!(
            effects,
            vec![
                E::Emit(WebEvent::RendererGone),
                E::Reload,
                E::Repaint,
                E::Emit(WebEvent::LoadingChanged {
                    loading: true,
                    can_go_back: true,
                    can_go_forward: false
                }),
                E::Emit(WebEvent::RendererGone),
            ]
        );
    }

    #[test]
    fn a_finished_recovery_rearms_the_reload() {
        let (state, _) = run(
            at(Phase::Loaded),
            vec![
                I::RendererTerminated,
                loading(true, true, false),
                loading(false, true, false),
            ],
        );
        assert_eq!(state.phase, Phase::Loaded);
        let (state, effects) = transition(state, I::RendererTerminated);
        assert_eq!(state.phase, Phase::Crashed);
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::RendererGone), E::Reload, E::Repaint]
        );
    }

    #[test]
    fn a_crashed_view_stays_crashed_until_loading_starts() {
        let (state, _) = transition(at(Phase::Crashed), loading(false, true, false));
        assert_eq!(state.phase, Phase::Crashed);
    }

    #[test]
    fn popups_are_reported_without_a_state_change() {
        let state = at(Phase::Loaded);
        let (next, effects) = transition(state.clone(), I::Popup("https://p/".into()));
        assert_eq!(next, state);
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::PopupRequested {
                url: "https://p/".into()
            })]
        );
    }

    #[test]
    fn favicon_and_cursor_are_deduplicated() {
        let (_, effects) = run(
            NavState::default(),
            vec![
                I::Favicon("https://a/i.ico".into()),
                I::Favicon("https://a/i.ico".into()),
                I::Cursor(Cursor::Pointer),
                I::Cursor(Cursor::Pointer),
                I::Cursor(Cursor::Default),
            ],
        );
        assert_eq!(
            effects,
            vec![
                E::Emit(WebEvent::FaviconChanged("https://a/i.ico".into())),
                E::Emit(WebEvent::CursorChanged(Cursor::Pointer)),
                E::Emit(WebEvent::CursorChanged(Cursor::Default)),
            ]
        );
    }

    #[test]
    fn stop_only_while_loading() {
        for phase in [
            Phase::Blank,
            Phase::Loading { recovering: false },
            Phase::Loading { recovering: true },
            Phase::Loaded,
            Phase::Crashed,
        ] {
            let stops = transition(at(phase), I::Stop).1 == vec![E::Stop];
            assert_eq!(stops, matches!(phase, Phase::Loading { .. }), "{phase:?}");
        }
    }

    #[test]
    fn cursor_from_every_welding_shape() {
        let cases = [
            (CursorShape::Default, Cursor::Default),
            (CursorShape::Pointer, Cursor::Pointer),
            (CursorShape::Text, Cursor::Text),
            (CursorShape::Wait, Cursor::Wait),
            (CursorShape::Crosshair, Cursor::Crosshair),
            (CursorShape::Move, Cursor::Move),
            (CursorShape::NotAllowed, Cursor::NotAllowed),
            (CursorShape::Help, Cursor::Help),
            (CursorShape::Progress, Cursor::Progress),
            (CursorShape::ResizeNs, Cursor::ResizeNs),
            (CursorShape::ResizeEw, Cursor::ResizeEw),
            (CursorShape::ResizeNeSw, Cursor::ResizeNeSw),
            (CursorShape::ResizeNwSe, Cursor::ResizeNwSe),
            (CursorShape::ResizeAll, Cursor::Move),
            (CursorShape::Grab, Cursor::Grab),
            (CursorShape::Grabbing, Cursor::Grabbing),
            (CursorShape::ZoomIn, Cursor::Default),
            (CursorShape::ZoomOut, Cursor::Default),
            (CursorShape::Custom("none".into()), Cursor::None),
            (CursorShape::Custom("alias".into()), Cursor::Default),
        ];
        for (shape, cursor) in cases {
            assert_eq!(Cursor::from(shape.clone()), cursor, "{shape:?}");
        }
    }
}
