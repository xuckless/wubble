//! Navigation state of one view and the events the host receives about it.
//!
//! Pure: no CEF types. [`crate::view::View`] translates welding's callbacks
//! and the host's commands into [`NavInput`]s, runs [`transition`] and
//! executes the [`NavEffect`]s. The host only ever sees [`WebEvent`]s.

use welding::CursorShape;

/// The pointer shape a page asks for. Exactly the shapes Slint can show
/// (i-slint-common 1.19 `BuiltInMouseCursor`), so the host maps 1:1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// The renderer process died; `View::reload` brings the page back.
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

/// Where a view's navigation is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavState {
    /// CEF has not started loading anything yet.
    Blank {
        page: Page,
    },
    Loading {
        page: Page,
    },
    Loaded {
        page: Page,
    },
    /// The renderer died. CEF's next `Loading { loading: true }` (after a
    /// host command) ends it.
    Crashed {
        page: Page,
    },
}

impl Default for NavState {
    fn default() -> Self {
        Self::Blank {
            page: Page::default(),
        }
    }
}

impl NavState {
    pub fn page(&self) -> &Page {
        match self {
            Self::Blank { page }
            | Self::Loading { page }
            | Self::Loaded { page }
            | Self::Crashed { page } => page,
        }
    }

    fn loading(&self) -> bool {
        matches!(self, Self::Loading { .. })
    }

    fn with_page(&self, page: Page) -> Self {
        match self {
            Self::Blank { .. } => Self::Blank { page },
            Self::Loading { .. } => Self::Loading { page },
            Self::Loaded { .. } => Self::Loaded { page },
            Self::Crashed { .. } => Self::Crashed { page },
        }
    }
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

/// Every (state, input) pair not matched leaves the state alone and does nothing.
pub fn transition(state: NavState, input: NavInput) -> (NavState, Vec<NavEffect>) {
    use NavEffect::Emit;
    let page = state.page().clone();
    let crashed = matches!(state, NavState::Crashed { .. });
    // A command that loads something; after a crash it also needs a repaint.
    let command = |effect: NavEffect| {
        let mut effects = vec![effect];
        if crashed {
            effects.push(NavEffect::Repaint);
        }
        (state.clone(), effects)
    };
    match input {
        NavInput::Address(url) if url != page.url => (
            state.with_page(Page {
                url: url.clone(),
                ..page
            }),
            vec![Emit(WebEvent::UrlChanged(url))],
        ),
        NavInput::Title(title) if title != page.title => (
            state.with_page(Page {
                title: title.clone(),
                ..page
            }),
            vec![Emit(WebEvent::TitleChanged(title))],
        ),
        NavInput::Favicon(url) if url != page.favicon => (
            state.with_page(Page {
                favicon: url.clone(),
                ..page
            }),
            vec![Emit(WebEvent::FaviconChanged(url))],
        ),
        NavInput::Cursor(cursor) if cursor != page.cursor => (
            state.with_page(Page { cursor, ..page }),
            vec![Emit(WebEvent::CursorChanged(cursor))],
        ),
        NavInput::Loading {
            loading,
            can_go_back,
            can_go_forward,
        } => {
            let page = Page {
                can_go_back,
                can_go_forward,
                ..page
            };
            let next = match (&state, loading) {
                (_, true) => NavState::Loading { page },
                // A crashed view stays crashed until something loads again.
                (NavState::Crashed { .. }, false) => NavState::Crashed { page },
                (_, false) => NavState::Loaded { page },
            };
            let old = (
                state.loading(),
                state.page().can_go_back,
                state.page().can_go_forward,
            );
            let changed = old != (loading, can_go_back, can_go_forward);
            let effects = if changed {
                vec![Emit(WebEvent::LoadingChanged {
                    loading,
                    can_go_back,
                    can_go_forward,
                })]
            } else {
                vec![]
            };
            (next, effects)
        }
        NavInput::RendererTerminated if !crashed => (
            NavState::Crashed { page },
            vec![Emit(WebEvent::RendererGone)],
        ),
        NavInput::Popup(url) => (state, vec![Emit(WebEvent::PopupRequested { url })]),
        NavInput::Load(url) => command(NavEffect::Load(url)),
        NavInput::Reload => command(NavEffect::Reload),
        NavInput::Back if page.can_go_back => command(NavEffect::Back),
        NavInput::Forward if page.can_go_forward => command(NavEffect::Forward),
        NavInput::Stop if state.loading() => (state, vec![NavEffect::Stop]),
        _ => (state, vec![]),
    }
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

    fn states() -> Vec<NavState> {
        vec![
            NavState::default(),
            NavState::Loading { page: page() },
            NavState::Loaded { page: page() },
            NavState::Crashed { page: page() },
        ]
    }

    fn inputs() -> Vec<NavInput> {
        vec![
            I::Address("https://a/".into()),
            I::Address("https://b/".into()),
            I::Title("A".into()),
            I::Title("B".into()),
            I::Favicon("https://a/i.ico".into()),
            I::Favicon("".into()),
            I::Cursor(Cursor::Text),
            I::Cursor(Cursor::Pointer),
            I::Loading {
                loading: true,
                can_go_back: true,
                can_go_forward: false,
            },
            I::Loading {
                loading: false,
                can_go_back: true,
                can_go_forward: false,
            },
            I::Loading {
                loading: false,
                can_go_back: false,
                can_go_forward: true,
            },
            I::RendererTerminated,
            I::Popup("https://p/".into()),
            I::Load("https://c/".into()),
            I::Back,
            I::Forward,
            I::Reload,
            I::Stop,
        ]
    }

    /// The table written out independently of `transition`.
    fn expected(state: &NavState, input: &NavInput) -> (NavState, Vec<NavEffect>) {
        use NavState::*;
        let blank = matches!(state, Blank { .. });
        let crashed = matches!(state, Crashed { .. });
        let p = state.page().clone();
        let same = (state.clone(), vec![]);
        let emit = |next: NavState, event| (next, vec![E::Emit(event)]);
        let cmd = |effect| {
            let mut effects = vec![effect];
            if crashed {
                effects.push(E::Repaint);
            }
            (state.clone(), effects)
        };
        let loading = |l: bool, back: bool, fwd: bool| LoadingChanged {
            loading: l,
            can_go_back: back,
            can_go_forward: fwd,
        };
        use WebEvent::*;
        match input {
            I::Address(u) if blank || u != "https://a/" => emit(
                state.with_page(Page {
                    url: u.clone(),
                    ..p
                }),
                UrlChanged(u.clone()),
            ),
            I::Title(t) if blank || t != "A" => emit(
                state.with_page(Page {
                    title: t.clone(),
                    ..p
                }),
                TitleChanged(t.clone()),
            ),
            // Blank's favicon is already "".
            I::Favicon(f) if (blank && !f.is_empty()) || (!blank && f.is_empty()) => emit(
                state.with_page(Page {
                    favicon: f.clone(),
                    ..p
                }),
                FaviconChanged(f.clone()),
            ),
            I::Cursor(c) if *c != p.cursor => {
                emit(state.with_page(Page { cursor: *c, ..p }), CursorChanged(*c))
            }
            I::Loading {
                loading: l,
                can_go_back: b,
                can_go_forward: f,
            } => {
                let page = Page {
                    can_go_back: *b,
                    can_go_forward: *f,
                    ..p
                };
                let next = if *l {
                    Loading { page }
                } else if crashed {
                    Crashed { page }
                } else {
                    Loaded { page }
                };
                let was = match state {
                    Blank { .. } => (false, false, false),
                    Loading { .. } => (true, true, false),
                    Loaded { .. } | Crashed { .. } => (false, true, false),
                };
                if was == (*l, *b, *f) {
                    (next, vec![])
                } else {
                    emit(next, loading(*l, *b, *f))
                }
            }
            I::RendererTerminated if !crashed => emit(Crashed { page: p }, RendererGone),
            I::Popup(u) => emit(state.clone(), PopupRequested { url: u.clone() }),
            I::Load(u) => cmd(E::Load(u.clone())),
            I::Reload => cmd(E::Reload),
            // Every page() here can go back but not forward; Blank can do neither.
            I::Back if !blank => cmd(E::Back),
            I::Stop if matches!(state, Loading { .. }) => (state.clone(), vec![E::Stop]),
            _ => same,
        }
    }

    #[test]
    fn table_covers_every_pair() {
        for state in states() {
            for input in inputs() {
                assert_eq!(
                    transition(state.clone(), input.clone()),
                    expected(&state, &input),
                    "{state:?} + {input:?}"
                );
            }
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
        assert!(matches!(state, NavState::Loaded { .. }));
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
        let start = NavState::Loaded {
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
    fn a_crash_is_reported_once_and_reload_repaints() {
        let (state, effects) = run(
            NavState::Loaded { page: page() },
            vec![I::RendererTerminated, I::RendererTerminated, I::Reload],
        );
        assert!(matches!(state, NavState::Crashed { .. }));
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::RendererGone), E::Reload, E::Repaint]
        );
        // CEF's loading callback ends the crash; commands then need no repaint.
        let (state, _) = run(state, vec![loading(true, true, false)]);
        assert!(matches!(state, NavState::Loading { .. }));
        assert_eq!(transition(state, I::Reload).1, vec![E::Reload]);
    }

    #[test]
    fn popups_are_reported_without_a_state_change() {
        let state = NavState::Loaded { page: page() };
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
        for state in states() {
            let stops = transition(state.clone(), I::Stop).1 == vec![E::Stop];
            assert_eq!(
                stops,
                matches!(state, NavState::Loading { .. }),
                "{state:?}"
            );
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
