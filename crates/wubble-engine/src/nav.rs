//! Navigation state of one view and the events the host receives about it.
//!
//! Pure: no CEF types. [`crate::view::View`] translates welding's callbacks
//! and the host's commands into [`NavInput`]s, runs [`transition`] and
//! executes the [`NavEffect`]s. The host only ever sees [`WebEvent`]s.

use welding::CursorShape;

use crate::input::KeyId;

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
    /// Middle-click autoscroll, the page still (all axes, vertical only,
    /// horizontal only). The shell hides the pointer for these and draws its
    /// own (`wubble_core::autoscroll`).
    PanStill,
    PanStillVertical,
    PanStillHorizontal,
    /// Autoscrolling toward a compass point.
    PanN,
    PanNE,
    PanE,
    PanSE,
    PanS,
    PanSW,
    PanW,
    PanNW,
}

impl Cursor {
    /// What this cursor says about autoscroll.
    pub fn pan(self) -> wubble_core::autoscroll::Pan {
        use wubble_core::autoscroll::{Direction as D, Pan};
        match self {
            Cursor::PanStill | Cursor::PanStillVertical | Cursor::PanStillHorizontal => Pan::Still,
            Cursor::PanN => Pan::Moving(D::N),
            Cursor::PanNE => Pan::Moving(D::NE),
            Cursor::PanE => Pan::Moving(D::E),
            Cursor::PanSE => Pan::Moving(D::SE),
            Cursor::PanS => Pan::Moving(D::S),
            Cursor::PanSW => Pan::Moving(D::SW),
            Cursor::PanW => Pan::Moving(D::W),
            Cursor::PanNW => Pan::Moving(D::NW),
            _ => Pan::None,
        }
    }
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
            CursorShape::Panning(p) => {
                use welding::Panning as P;
                match p {
                    P::Middle => Self::PanStill,
                    P::MiddleVertical => Self::PanStillVertical,
                    P::MiddleHorizontal => Self::PanStillHorizontal,
                    P::North => Self::PanN,
                    P::NorthEast => Self::PanNE,
                    P::East => Self::PanE,
                    P::SouthEast => Self::PanSE,
                    P::South => Self::PanS,
                    P::SouthWest => Self::PanSW,
                    P::West => Self::PanW,
                    P::NorthWest => Self::PanNW,
                }
            }
            // welding's name for CEF's `CT_NONE`.
            CursorShape::Custom(name) if name == "none" => Self::None,
            // Zoom, other custom names, and shapes a newer welding adds.
            _ => Self::Default,
        }
    }
}

/// A page's request to open a window or tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupRequest {
    pub url: String,
    /// Whether a click or key press led to it.
    pub gesture: bool,
    pub opened: wubble_core::popup::Opened,
    /// The `width`/`height` window features, when both were given.
    pub size: Option<(u32, u32)>,
}

/// What the host learns about a view, in the order it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebEvent {
    UrlChanged(String),
    TitleChanged(String),
    /// The view went from not loading to loading: a new navigation or a
    /// reload. Sent just before its `LoadingChanged`; a `LoadingChanged`
    /// that only moves the history flags mid-load comes without it.
    LoadStarted,
    LoadingChanged {
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
    },
    /// The page's first icon URL, empty when it has none.
    FaviconChanged(String),
    CursorChanged(Cursor),
    /// `window.open` or a `target=_blank` link. CEF opened nothing itself; the host's
    /// policy (`wubble_core::popup::decide`) says what opens.
    PopupRequested(PopupRequest),
    /// The renderer process died. Unless the host draws its own crash page
    /// ([`NavInput::RendererTerminated`]), the view reloads the page once by
    /// itself.
    RendererGone,
    /// The page left this key-down alone (CEF `OnKeyEvent`): a page-first
    /// shortcut may run now.
    KeyUnhandled(KeyId),
}

/// What the view shows, as far as the host cares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub favicon: String,
    /// The icon URL last fetched for this page, whatever came of it: one
    /// attempt per URL. Cleared when a load starts, so a reload or a new page
    /// declaring the same URL fetches it again.
    pub favicon_fetched: String,
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
    /// `reload`: reload the page once by itself; false when the host shows
    /// a crash page whose Reload recovers instead.
    RendererTerminated {
        reload: bool,
    },
    Popup(PopupRequest),
    Cursor(Cursor),
    // From the host.
    Load(String),
    Back,
    Forward,
    Reload,
    /// Reload, bypassing the cache.
    HardReload,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavEffect {
    Emit(WebEvent),
    Load(String),
    Back,
    Forward,
    Reload,
    HardReload,
    Stop,
    /// Download the icon image at this URL (`favicon.rs`).
    FetchFavicon(String),
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
        // CEF reports the icon URLs again and again on one page (every
        // `<link>` change): each URL is fetched once per load, failed or not.
        // A new page that declares the same URL starts a load, which clears
        // `favicon_fetched`, so its icon still reaches the host.
        NavInput::Favicon(url) => {
            let fetch = !url.is_empty() && url != page.favicon_fetched;
            let mut effects: Vec<_> = (url != page.favicon)
                .then(|| Emit(WebEvent::FaviconChanged(url.clone())))
                .into_iter()
                .collect();
            if fetch {
                effects.push(NavEffect::FetchFavicon(url.clone()));
            }
            let favicon_fetched = if fetch {
                url.clone()
            } else {
                page.favicon_fetched.clone()
            };
            let page = Page {
                favicon: url,
                favicon_fetched,
                ..page
            };
            (page, effects)
        }
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
            let mut effects = vec![];
            if now_loading && !loading {
                effects.push(Emit(WebEvent::LoadStarted));
            }
            if changed {
                effects.push(Emit(WebEvent::LoadingChanged {
                    loading: now_loading,
                    can_go_back,
                    can_go_forward,
                }));
            }
            let favicon_fetched = if now_loading && !loading {
                String::new()
            } else {
                page.favicon_fetched
            };
            let page = Page {
                can_go_back,
                can_go_forward,
                favicon_fetched,
                ..page
            };
            return (NavState { phase: next, page }, effects);
        }
        NavInput::RendererTerminated { reload } if phase != Phase::Crashed => {
            let mut effects = vec![Emit(WebEvent::RendererGone)];
            if reload && phase != (Phase::Loading { recovering: true }) {
                effects.extend([NavEffect::Reload, NavEffect::Repaint]);
            }
            let state = NavState {
                phase: Phase::Crashed,
                page,
            };
            return (state, effects);
        }
        NavInput::Popup(request) => (page, vec![Emit(WebEvent::PopupRequested(request))]),
        NavInput::Load(url) => (page, command(NavEffect::Load(url))),
        NavInput::Reload => (page, command(NavEffect::Reload)),
        NavInput::HardReload => (page, command(NavEffect::HardReload)),
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
            favicon_fetched: String::new(),
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
                E::Emit(WebEvent::LoadStarted),
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
            vec![
                I::RendererTerminated { reload: true },
                I::RendererTerminated { reload: true },
            ],
        );
        assert_eq!(state, at(Phase::Crashed));
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::RendererGone), E::Reload, E::Repaint]
        );
        // Commands while crashed repaint too.
        assert_eq!(
            transition(state.clone(), I::Reload).1,
            vec![E::Reload, E::Repaint]
        );
        assert_eq!(
            transition(state, I::HardReload).1,
            vec![E::HardReload, E::Repaint]
        );
    }

    #[test]
    fn a_crash_during_the_recovery_load_is_not_reloaded() {
        let (state, effects) = run(
            at(Phase::Loaded),
            vec![
                I::RendererTerminated { reload: true },
                loading(true, true, false),
                I::RendererTerminated { reload: true },
            ],
        );
        assert_eq!(state.phase, Phase::Crashed);
        assert_eq!(
            effects,
            vec![
                E::Emit(WebEvent::RendererGone),
                E::Reload,
                E::Repaint,
                E::Emit(WebEvent::LoadStarted),
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
                I::RendererTerminated { reload: true },
                loading(true, true, false),
                loading(false, true, false),
            ],
        );
        assert_eq!(state.phase, Phase::Loaded);
        let (state, effects) = transition(state, I::RendererTerminated { reload: true });
        assert_eq!(state.phase, Phase::Crashed);
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::RendererGone), E::Reload, E::Repaint]
        );
    }

    #[test]
    fn history_flags_mid_load_are_no_new_load() {
        let (state, effects) = run(
            NavState::default(),
            vec![loading(true, false, false), loading(true, true, false)],
        );
        assert_eq!(state.phase, Phase::Loading { recovering: false });
        assert_eq!(
            effects
                .iter()
                .filter(|e| **e == E::Emit(WebEvent::LoadStarted))
                .count(),
            1
        );
        assert_eq!(
            effects.last(),
            Some(&E::Emit(WebEvent::LoadingChanged {
                loading: true,
                can_go_back: true,
                can_go_forward: false
            }))
        );
    }

    #[test]
    fn a_host_crash_page_gets_no_reload() {
        let (state, effects) =
            transition(at(Phase::Loaded), I::RendererTerminated { reload: false });
        assert_eq!(state.phase, Phase::Crashed);
        assert_eq!(effects, vec![E::Emit(WebEvent::RendererGone)]);
        // The crash page's Reload starts a load that ends the crash.
        let (state, effects) = transition(state, loading(true, true, false));
        assert_eq!(state.phase, Phase::Loading { recovering: true });
        assert_eq!(effects[0], E::Emit(WebEvent::LoadStarted));
    }

    #[test]
    fn a_crashed_view_stays_crashed_until_loading_starts() {
        let (state, _) = transition(at(Phase::Crashed), loading(false, true, false));
        assert_eq!(state.phase, Phase::Crashed);
    }

    #[test]
    fn popups_are_reported_without_a_state_change() {
        let state = at(Phase::Loaded);
        let request = PopupRequest {
            url: "https://p/".into(),
            gesture: false,
            opened: wubble_core::popup::Opened::PopupWindow,
            size: Some((400, 300)),
        };
        let (next, effects) = transition(state.clone(), I::Popup(request.clone()));
        assert_eq!(next, state);
        assert_eq!(effects, vec![E::Emit(WebEvent::PopupRequested(request))]);
    }

    #[test]
    fn favicon_event_and_cursor_are_deduplicated() {
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
                E::FetchFavicon("https://a/i.ico".into()),
                E::Emit(WebEvent::CursorChanged(Cursor::Pointer)),
                E::Emit(WebEvent::CursorChanged(Cursor::Default)),
            ]
        );
    }

    fn fetches(effects: &[NavEffect]) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|e| match e {
                E::FetchFavicon(url) => Some(url.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_icon_is_fetched_once_per_url_and_load() {
        let a = || I::Favicon("https://a/i.png".into());
        let b = || I::Favicon("https://a/j.png".into());
        let (_, effects) = run(
            NavState::default(),
            vec![
                loading(true, false, false),
                // A page re-reporting its icons (YouTube does, dozens of
                // times): one fetch, even if that one failed.
                a(),
                a(),
                a(),
                loading(false, false, false),
                a(),
                // A new URL is fetched; going back to the old one is a change
                // of URL too.
                b(),
                b(),
                a(),
                // A reload (or a new page with the same icon) fetches again.
                loading(true, false, false),
                a(),
                a(),
            ],
        );
        assert_eq!(
            fetches(&effects),
            [
                "https://a/i.png",
                "https://a/j.png",
                "https://a/i.png",
                "https://a/i.png"
            ]
        );
    }

    #[test]
    fn a_history_flag_change_mid_load_does_not_refetch() {
        let (_, effects) = run(
            NavState::default(),
            vec![
                loading(true, false, false),
                I::Favicon("https://a/i.png".into()),
                loading(true, true, false),
                I::Favicon("https://a/i.png".into()),
            ],
        );
        assert_eq!(fetches(&effects), ["https://a/i.png"]);
    }

    #[test]
    fn an_empty_favicon_url_fetches_nothing() {
        let state = NavState {
            page: Page {
                favicon: "https://a/i.ico".into(),
                ..Page::default()
            },
            ..NavState::default()
        };
        let (_, effects) = transition(state, I::Favicon(String::new()));
        assert_eq!(
            effects,
            vec![E::Emit(WebEvent::FaviconChanged(String::new()))]
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
    fn panning_cursors_say_where_autoscroll_goes() {
        use wubble_core::autoscroll::{Direction, Pan};
        assert_eq!(Cursor::Default.pan(), Pan::None);
        assert_eq!(Cursor::PanStillVertical.pan(), Pan::Still);
        assert_eq!(Cursor::PanS.pan(), Pan::Moving(Direction::S));
        assert_eq!(Cursor::PanNE.pan(), Pan::Moving(Direction::NE));
        // The shell's cursor table ends with the panning cursors.
        assert_eq!(Cursor::PanNW as i32, 26);
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
            (
                CursorShape::Panning(welding::Panning::Middle),
                Cursor::PanStill,
            ),
            (CursorShape::Panning(welding::Panning::South), Cursor::PanS),
            (
                CursorShape::Panning(welding::Panning::NorthWest),
                Cursor::PanNW,
            ),
        ];
        for (shape, cursor) in cases {
            assert_eq!(Cursor::from(shape.clone()), cursor, "{shape:?}");
        }
    }
}
