//! DevTools for a tab: docked beside the page as an ordinary view, or in its own window.
//!
//! CEF cannot put DevTools in a windowless browser (`ChromeBrowserDelegate::CreateDevToolsBrowser`
//! logs "Windowless rendering is not supported for this DevTools window" and makes a native
//! window), so the docked panel is Chromium's DevTools frontend loaded as a page from the
//! remote debugging endpoint (`remote-debugging-port`, which listens on 127.0.0.1 only):
//! `http://127.0.0.1:P/devtools/inspector.html?ws=127.0.0.1:P/devtools/page/<targetId>`. The
//! target id comes from the CDP call `Target.getTargetInfo` on the inspected page. Chromium's
//! `devtools_http_handler.cc` is BSD-3-Clause; behaviour only, nothing copied.
//!
//! [`transition`] is the one lifecycle: one dock per window, for one tab. The host executes the
//! [`DockEffect`]s and feeds the answers back as [`DockEvent`]s.

use crate::tab::TabId;

/// The pane key of the docked panel's view: no tab has it (tab ids count up from 1, and this
/// one still fits Slint's `int` pane ids).
pub const PANEL_TAB: TabId = TabId(i32::MAX as u64);

/// A point in the page, CSS pixels (CEF's `Point`, for inspect-at-point).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// Where DevTools may open, fixed for the run (the endpoint is a start switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The endpoint listens on this port: DevTools docks beside the page.
    Docked { port: u16 },
    /// No endpoint: DevTools opens in its own native window.
    DetachedOnly,
}

impl Mode {
    pub fn of(port: Option<u16>) -> Self {
        port.map_or(Self::DetachedOnly, |port| Self::Docked { port })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Dock {
    #[default]
    Closed,
    /// Waiting for the inspected page's target id.
    Resolving { tab: TabId },
    /// The frontend view is the docked panel.
    Open { tab: TabId },
    /// DevTools is in its own window.
    Detached { tab: TabId },
}

impl Dock {
    /// The tab being inspected, whichever way.
    pub fn tab(&self) -> Option<TabId> {
        match self {
            Self::Closed => None,
            Self::Resolving { tab } | Self::Open { tab } | Self::Detached { tab } => Some(*tab),
        }
    }

    pub fn is_panel_open(&self) -> bool {
        matches!(self, Self::Open { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockEvent {
    /// F12 or the chord, on the visible tab.
    Toggle(TabId),
    /// The context menu's Inspect at this point of the tab.
    InspectAt(TabId, Point),
    /// The answer to [`DockEffect::AskTarget`].
    TargetId(TabId, String),
    TargetFailed(TabId),
    /// The frontend view could not be opened (no window, nothing listening on the endpoint).
    OpenFailed(TabId),
    /// The frontend view closed or the seam was pulled shut.
    PanelClosed,
    TabGone(TabId),
    /// The native window closed on its own.
    WindowClosed(TabId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockEffect {
    /// Ask the tab's page for its target id (CDP `Target.getTargetInfo`).
    AskTarget(TabId),
    /// Open the panel view on this URL.
    OpenFrontend {
        tab: TabId,
        url: String,
    },
    CloseFrontend,
    OpenDetached {
        tab: TabId,
        inspect: Option<Point>,
    },
    CloseDetached(TabId),
}

/// The frontend page for `target` on the endpoint at `port`. `None` unless `target` is a hex
/// id, so nothing can add a query parameter or a path.
pub fn frontend_url(port: u16, target: &str) -> Option<String> {
    let valid = (16..=64).contains(&target.len()) && target.bytes().all(|b| b.is_ascii_hexdigit());
    valid.then(|| {
        format!(
            "http://127.0.0.1:{port}/devtools/inspector.html?ws=127.0.0.1:{port}/devtools/page/{target}"
        )
    })
}

/// What `dock` becomes on `event`, and what the host must do.
pub fn transition(dock: Dock, mode: Mode, event: DockEvent) -> (Dock, Vec<DockEffect>) {
    use DockEffect::*;
    use DockEvent::*;
    // closing whatever is open now
    let close = |dock: &Dock| match dock {
        Dock::Open { .. } => vec![CloseFrontend],
        Dock::Detached { tab } => vec![CloseDetached(*tab)],
        Dock::Closed | Dock::Resolving { .. } => vec![],
    };
    // opening for `tab` the way `mode` allows
    let open = |tab: TabId| match mode {
        Mode::Docked { .. } => (Dock::Resolving { tab }, vec![AskTarget(tab)]),
        Mode::DetachedOnly => (
            Dock::Detached { tab },
            vec![OpenDetached { tab, inspect: None }],
        ),
    };
    match event {
        Toggle(tab) if dock.tab() == Some(tab) => {
            let effects = close(&dock);
            (Dock::Closed, effects)
        }
        Toggle(tab) => {
            let (next, opening) = open(tab);
            let mut effects = close(&dock);
            effects.extend(opening);
            (next, effects)
        }
        // CEF's own inspect-at-point is the only way to reveal a node, and it is native-window
        // only: the docked panel gives way to it.
        InspectAt(tab, at) => {
            let mut effects = close(&dock);
            effects.push(OpenDetached {
                tab,
                inspect: Some(at),
            });
            (Dock::Detached { tab }, effects)
        }
        TargetId(tab, id) if dock == (Dock::Resolving { tab }) => {
            let url = match mode {
                Mode::Docked { port } => frontend_url(port, &id),
                Mode::DetachedOnly => None,
            };
            match url {
                Some(url) => (Dock::Open { tab }, vec![OpenFrontend { tab, url }]),
                // an id the frontend cannot use: the window instead
                None => (
                    Dock::Detached { tab },
                    vec![OpenDetached { tab, inspect: None }],
                ),
            }
        }
        TargetFailed(tab) if dock == (Dock::Resolving { tab }) => (
            Dock::Detached { tab },
            vec![OpenDetached { tab, inspect: None }],
        ),
        // the panel never came up: nothing to close, the window instead
        OpenFailed(tab) if dock == (Dock::Open { tab }) => (
            Dock::Detached { tab },
            vec![OpenDetached { tab, inspect: None }],
        ),
        PanelClosed if dock.is_panel_open() => (Dock::Closed, vec![CloseFrontend]),
        TabGone(tab) if dock.tab() == Some(tab) => {
            // the tab's view takes its DevTools window with it
            let effects = if dock.is_panel_open() {
                vec![CloseFrontend]
            } else {
                vec![]
            };
            (Dock::Closed, effects)
        }
        WindowClosed(tab) if dock == (Dock::Detached { tab }) => (Dock::Closed, vec![]),
        // stale answers and events for what is not open
        TargetId(..) | TargetFailed(_) | OpenFailed(_) | PanelClosed | TabGone(_)
        | WindowClosed(_) => (dock, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DockEffect::*;
    use DockEvent::*;

    const DOCKED: Mode = Mode::Docked { port: 9333 };
    const ID: &str = "0123456789ABCDEF0123456789abcdef";
    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    fn run(mode: Mode, events: &[DockEvent]) -> (Dock, Vec<Vec<DockEffect>>) {
        let mut dock = Dock::Closed;
        let mut all = vec![];
        for e in events {
            let (next, effects) = transition(dock, mode, e.clone());
            dock = next;
            all.push(effects);
        }
        (dock, all)
    }

    #[test]
    fn toggle_asks_for_the_target_then_opens_the_frontend_and_closes_again() {
        let (dock, fx) = run(DOCKED, &[Toggle(A), TargetId(A, ID.into()), Toggle(A)]);
        assert_eq!(dock, Dock::Closed);
        assert_eq!(fx[0], [AskTarget(A)]);
        assert_eq!(
            fx[1],
            [OpenFrontend {
                tab: A,
                url: frontend_url(9333, ID).unwrap()
            }]
        );
        assert_eq!(fx[2], [CloseFrontend]);
    }

    #[test]
    fn toggle_on_another_tab_moves_the_dock() {
        let (dock, fx) = run(DOCKED, &[Toggle(A), TargetId(A, ID.into()), Toggle(B)]);
        assert_eq!(dock, Dock::Resolving { tab: B });
        assert_eq!(fx[2], [CloseFrontend, AskTarget(B)]);
        // toggling while resolving cancels; the late answer is ignored
        let (dock, fx) = run(DOCKED, &[Toggle(A), Toggle(A), TargetId(A, ID.into())]);
        assert_eq!(dock, Dock::Closed);
        assert!(fx[1].is_empty() && fx[2].is_empty());
    }

    #[test]
    fn a_failed_or_unusable_target_falls_back_to_the_window() {
        let window = OpenDetached {
            tab: A,
            inspect: None,
        };
        let (dock, fx) = run(DOCKED, &[Toggle(A), TargetFailed(A)]);
        assert_eq!(dock, Dock::Detached { tab: A });
        assert_eq!(fx[1], std::slice::from_ref(&window));
        let (dock, fx) = run(DOCKED, &[Toggle(A), TargetId(A, "../x".into())]);
        assert_eq!(dock, Dock::Detached { tab: A });
        assert_eq!(fx[1], [window]);
    }

    #[test]
    fn a_panel_that_cannot_open_falls_back_to_the_window() {
        let (dock, fx) = run(DOCKED, &[Toggle(A), TargetId(A, ID.into()), OpenFailed(A)]);
        assert_eq!(dock, Dock::Detached { tab: A });
        assert_eq!(
            fx[2],
            [OpenDetached {
                tab: A,
                inspect: None
            }]
        );
        // a late failure for a closed or other dock changes nothing
        assert_eq!(run(DOCKED, &[OpenFailed(A)]).0, Dock::Closed);
        let (dock, fx) = run(DOCKED, &[Toggle(A), TargetId(A, ID.into()), OpenFailed(B)]);
        assert_eq!(dock, Dock::Open { tab: A });
        assert!(fx[2].is_empty());
    }

    #[test]
    fn tab_gone_and_panel_closed_close_the_dock() {
        let open = [Toggle(A), TargetId(A, ID.into())];
        let with = |last: DockEvent| run(DOCKED, &[open[0].clone(), open[1].clone(), last]);
        assert_eq!(with(PanelClosed), (Dock::Closed, with(PanelClosed).1));
        assert_eq!(with(PanelClosed).1[2], [CloseFrontend]);
        assert_eq!(with(TabGone(A)).0, Dock::Closed);
        assert_eq!(with(TabGone(A)).1[2], [CloseFrontend]);
        // another tab going changes nothing
        assert_eq!(with(TabGone(B)).0, Dock::Open { tab: A });
        assert!(with(TabGone(B)).1[2].is_empty());
        assert_eq!(run(DOCKED, &[Toggle(A), TabGone(A)]).0, Dock::Closed);
    }

    #[test]
    fn without_an_endpoint_the_window_is_the_only_way_and_no_target_is_asked() {
        let mode = Mode::DetachedOnly;
        let (dock, fx) = run(mode, &[Toggle(A), Toggle(A)]);
        assert_eq!(dock, Dock::Closed);
        assert_eq!(
            fx[0],
            [OpenDetached {
                tab: A,
                inspect: None
            }]
        );
        assert_eq!(fx[1], [CloseDetached(A)]);
        let (dock, fx) = run(mode, &[Toggle(A), Toggle(B)]);
        assert_eq!(dock, Dock::Detached { tab: B });
        assert_eq!(
            fx[1],
            [
                CloseDetached(A),
                OpenDetached {
                    tab: B,
                    inspect: None
                }
            ]
        );
        assert!(
            fx.iter()
                .flatten()
                .all(|e| !matches!(e, AskTarget(_) | OpenFrontend { .. }))
        );
        // the user closes the window
        assert_eq!(run(mode, &[Toggle(A), WindowClosed(A)]).0, Dock::Closed);
        assert_eq!(
            run(mode, &[Toggle(A), WindowClosed(B)]).0,
            Dock::Detached { tab: A }
        );
    }

    #[test]
    fn inspect_at_a_point_opens_the_window_in_both_modes() {
        let at = Point { x: 12, y: 34 };
        let want = OpenDetached {
            tab: A,
            inspect: Some(at),
        };
        for mode in [DOCKED, Mode::DetachedOnly] {
            let (dock, fx) = run(mode, &[InspectAt(A, at)]);
            assert_eq!(dock, Dock::Detached { tab: A });
            assert_eq!(fx[0], std::slice::from_ref(&want));
        }
        // an open panel gives way to it
        let (dock, fx) = run(
            DOCKED,
            &[Toggle(A), TargetId(A, ID.into()), InspectAt(A, at)],
        );
        assert_eq!(dock, Dock::Detached { tab: A });
        assert_eq!(fx[2], [CloseFrontend, want]);
    }

    #[test]
    fn a_stale_target_id_is_ignored() {
        let (dock, fx) = run(DOCKED, &[Toggle(A), Toggle(B), TargetId(A, ID.into())]);
        assert_eq!(dock, Dock::Resolving { tab: B });
        assert!(fx[2].is_empty());
        assert_eq!(run(DOCKED, &[TargetId(A, ID.into())]).0, Dock::Closed);
    }

    #[test]
    fn the_frontend_url_takes_only_hex_ids() {
        assert_eq!(
            frontend_url(9333, ID).as_deref(),
            Some(
                "http://127.0.0.1:9333/devtools/inspector.html?ws=127.0.0.1:9333/devtools/page/0123456789ABCDEF0123456789abcdef"
            )
        );
        for bad in [
            "",
            "../..",
            "0123456789abcdef&x=1",
            "0123456789abcde",
            "0123456789abcdeg",
            &"a".repeat(200),
        ] {
            assert_eq!(frontend_url(9333, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn mode_follows_the_port() {
        assert_eq!(Mode::of(None), Mode::DetachedOnly);
        assert_eq!(Mode::of(Some(9333)), DOCKED);
    }
}
