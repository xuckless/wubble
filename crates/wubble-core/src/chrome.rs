//! The chrome's pure rules (the design's `browser/chrome.tsx` and `command.tsx`): what the
//! address pill shows, when the sidebar is compact, the hidden sidebar's peek and the command
//! bar's life. Pure, no timers: the host runs the effects.

use crate::seam::SidebarMode;
use crate::url;

/// Below this live width the sidebar draws as the rail (chrome.tsx `narrow`: `w < 140`).
pub const NARROW: f64 = 140.0;

/// Whether the sidebar draws compact (the rail look), chrome.tsx:
/// `compact = (narrow || rail) && !peek && !(hidden && !narrow)`: narrow while its live width is
/// under [`NARROW`] (mid-drag too), never while it peeks, and not while it closes from wide.
pub fn compact(live_width: f64, mode: SidebarMode, peeking: bool) -> bool {
    let narrow = live_width < NARROW;
    let rail = mode == SidebarMode::Rail;
    let hidden = mode == SidebarMode::Hidden;
    (narrow || rail) && !peeking && !(hidden && !narrow)
}

/// How long the pointer rests in the edge strip before the hidden sidebar peeks, ms
/// (chrome.tsx `onMove`: `setTimeout(() => setPeek(true), 60)`). The peek is the store's peek
/// slot (`overlay::Kind::Peek`): the edge shows it at once, these are its delays.
pub const PEEK_IN_MS: u32 = 60;
/// How long after the pointer left the peeking sidebar it goes, ms (chrome.tsx `leaveSide`).
pub const PEEK_OUT_MS: u32 = 260;

/// What the command bar opens for (command.tsx `cmd.mode`): `Here` replaces the shown page (the
/// address pill), `New` opens a tab (New tab), `Split` opens beside the focused page. `Tabs` is
/// wubble's: tab search (Mod+Shift+A, Chrome's Tab Search), the bar listing every open and
/// recently closed tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandMode {
    Here,
    New,
    Split,
    Tabs,
}

/// Where it was opened from, window px: the panel grows from there (`cmd.from`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Where the bar's field is (owner, 2026-11-19: "for start page the center address bar should be
/// the only driver"): the overlay panel at the top of the page card (command.tsx), or the start
/// page's own search capsule, its results dropping beneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Placement {
    #[default]
    Overlay,
    InPage,
}

impl Placement {
    /// Where a bar opened over the shown page goes: the start page's capsule on the start page,
    /// the overlay everywhere else.
    pub fn for_page(start_page: bool) -> Self {
        if start_page {
            Placement::InPage
        } else {
            Placement::Overlay
        }
    }

    /// Where a bar opened in `mode` goes: tab search is always the overlay (Chrome's popover;
    /// the start page's capsule is for addresses), any other as [`Placement::for_page`].
    pub fn for_mode(mode: CommandMode, start_page: bool) -> Self {
        match mode {
            CommandMode::Tabs => Placement::Overlay,
            _ => Placement::for_page(start_page),
        }
    }
}

/// The command bar's life (command.tsx): open, then on its way out (`leaving`, the `wb-pop-out`
/// exit), then gone. `at` is where its field is while it lives.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CommandBar {
    #[default]
    Closed,
    Open {
        mode: CommandMode,
        from: Point,
        at: Placement,
    },
    Closing {
        mode: CommandMode,
        from: Point,
        at: Placement,
    },
}

impl CommandBar {
    pub fn placement(self) -> Option<Placement> {
        match self {
            CommandBar::Closed => None,
            CommandBar::Open { at, .. } | CommandBar::Closing { at, .. } => Some(at),
        }
    }

    pub fn mode(self) -> Option<CommandMode> {
        match self {
            CommandBar::Closed => None,
            CommandBar::Open { mode, .. } | CommandBar::Closing { mode, .. } => Some(mode),
        }
    }

    pub fn from(self) -> Option<Point> {
        match self {
            CommandBar::Closed => None,
            CommandBar::Open { from, .. } | CommandBar::Closing { from, .. } => Some(from),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CommandEvent {
    /// The address pill, New tab, the start page's search or an action opened it, its field
    /// `at` the overlay or the start page's capsule ([`Placement::for_page`]).
    Open {
        mode: CommandMode,
        from: Point,
        at: Placement,
    },
    /// Escape, a click on the scrim, the keys going elsewhere (`close`).
    Close,
    /// The exit ran its course (`setTimeout(..., CMD.out)`).
    Settled,
    /// A row was chosen (`go` / an action's `run`): the bar goes at once (`d.command = null`).
    Pick,
    /// "Open beside…": the open bar turns to picking a page beside (`d.command.mode = 'split'`).
    SetMode(CommandMode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandEffect {
    /// Stop an exit under way.
    Disarm,
    /// The input takes the keys, its text selected (`input.focus()`).
    FocusInput,
    /// Run the exit (`wb-pop-out`, the dropdown's out duration), then `Settled`.
    StartExit,
    /// The keys go back where they were, if the bar held them.
    RestoreFocus,
}

/// One step of the command bar.
pub fn command(state: CommandBar, event: CommandEvent) -> (CommandBar, Vec<CommandEffect>) {
    use CommandBar::*;
    use CommandEffect::*;
    use CommandEvent as E;
    match (state, event) {
        (_, E::Open { mode, from, at }) => (Open { mode, from, at }, vec![Disarm, FocusInput]),
        (Open { mode, from, at }, E::Close) => (Closing { mode, from, at }, vec![StartExit]),
        (Closing { .. }, E::Settled) | (Open { .. }, E::Pick) => (Closed, vec![RestoreFocus]),
        (Open { from, at, .. }, E::SetMode(mode)) => (Open { mode, from, at }, vec![FocusInput]),
        (s, _) => (s, vec![]),
    }
}

/// What the address pill shows for `url`: the host in ink and the rest in haze (chrome.tsx
/// `addressOf`). wubble's own pages read `wubble` + `://settings`; no page (a new tab, the start
/// page) reads `wubble` + `://start`, as the design's; a `data:` page (bundled pages) is named
/// by its scheme alone, never its raw body. `full` (Chrome's "Always show full URLs") keeps a
/// site's scheme in front of its host.
pub fn address_parts(url: &str, full: bool) -> (String, String) {
    let u = url.trim();
    if u.is_empty() || u.eq_ignore_ascii_case("about:blank") {
        return ("wubble".into(), "://start".into());
    }
    if u.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("data:")) {
        return ("data".into(), String::new());
    }
    let (host, path) = url::address_parts(u);
    // the host and the rest are the end of `u`: whole, the host reaches back to its start
    let host = if full {
        &u[..u.len() - path.len()]
    } else {
        host
    };
    (host.to_owned(), path.to_owned())
}

/// The pill's lock: a site over https (the design shows the lock for every site, the bubble for
/// its own pages); everything else gets the bubble.
pub fn is_secure(url: &str) -> bool {
    url.trim()
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [SidebarMode; 3] = [
        SidebarMode::Extended,
        SidebarMode::Rail,
        SidebarMode::Hidden,
    ];

    const AT: Point = Point { x: 48.0, y: 74.0 };

    fn open(mode: CommandMode) -> CommandEvent {
        CommandEvent::Open {
            mode,
            from: AT,
            at: Placement::Overlay,
        }
    }

    #[test]
    fn the_bar_opens_takes_the_keys_and_grows_from_its_point() {
        use CommandEffect::*;
        let (s, fx) = command(CommandBar::Closed, open(CommandMode::New));
        assert_eq!(
            s,
            CommandBar::Open {
                mode: CommandMode::New,
                from: AT,
                at: Placement::Overlay,
            }
        );
        assert_eq!(fx, [Disarm, FocusInput]);
        assert_eq!((s.mode(), s.from()), (Some(CommandMode::New), Some(AT)));
        assert_eq!(
            (CommandBar::Closed.mode(), CommandBar::Closed.from()),
            (None, None)
        );
    }

    #[test]
    fn close_runs_the_exit_then_settles_closed() {
        use CommandEffect::*;
        let s = command(CommandBar::Closed, open(CommandMode::Here)).0;
        let (s, fx) = command(s, CommandEvent::Close);
        assert!(matches!(
            s,
            CommandBar::Closing {
                mode: CommandMode::Here,
                ..
            }
        ));
        assert_eq!(fx, [StartExit]);
        // `if (leaving) return`: a second close does nothing
        assert_eq!(command(s, CommandEvent::Close), (s, vec![]));
        assert_eq!(command(s, CommandEvent::Pick), (s, vec![]));
        let (s, fx) = command(s, CommandEvent::Settled);
        assert_eq!(s, CommandBar::Closed);
        assert_eq!(fx, [RestoreFocus]);
    }

    #[test]
    fn a_pick_closes_at_once() {
        use CommandEffect::*;
        let s = command(CommandBar::Closed, open(CommandMode::New)).0;
        assert_eq!(
            command(s, CommandEvent::Pick),
            (CommandBar::Closed, vec![RestoreFocus])
        );
    }

    #[test]
    fn opening_again_while_it_closes_brings_it_back() {
        use CommandEffect::*;
        let s = command(CommandBar::Closed, open(CommandMode::Here)).0;
        let s = command(s, CommandEvent::Close).0;
        let (s, fx) = command(s, open(CommandMode::Split));
        assert_eq!(
            s,
            CommandBar::Open {
                mode: CommandMode::Split,
                from: AT,
                at: Placement::Overlay,
            }
        );
        assert_eq!(fx, [Disarm, FocusInput]);
    }

    #[test]
    fn settled_only_ends_an_exit() {
        let open_bar = command(CommandBar::Closed, open(CommandMode::Here)).0;
        for s in [CommandBar::Closed, open_bar] {
            assert_eq!(command(s, CommandEvent::Settled), (s, vec![]));
        }
    }

    #[test]
    fn closed_ignores_all_but_open() {
        for e in [
            CommandEvent::Close,
            CommandEvent::Settled,
            CommandEvent::Pick,
            CommandEvent::SetMode(CommandMode::Split),
        ] {
            assert_eq!(command(CommandBar::Closed, e), (CommandBar::Closed, vec![]));
        }
    }

    #[test]
    fn open_beside_turns_the_open_bar_to_split() {
        use CommandEffect::*;
        let s = command(CommandBar::Closed, open(CommandMode::Here)).0;
        let (s, fx) = command(s, CommandEvent::SetMode(CommandMode::Split));
        assert_eq!(
            s,
            CommandBar::Open {
                mode: CommandMode::Split,
                from: AT,
                at: Placement::Overlay,
            }
        );
        assert_eq!(fx, [FocusInput]);
        // not while it leaves
        let closing = command(s, CommandEvent::Close).0;
        assert_eq!(
            command(closing, CommandEvent::SetMode(CommandMode::New)),
            (closing, vec![])
        );
    }

    #[test]
    fn the_start_page_keeps_the_field_in_its_capsule() {
        use CommandEffect::*;
        assert_eq!(Placement::for_page(true), Placement::InPage);
        assert_eq!(Placement::for_page(false), Placement::Overlay);
        let in_page = CommandEvent::Open {
            mode: CommandMode::Here,
            from: AT,
            at: Placement::InPage,
        };
        let (s, fx) = command(CommandBar::Closed, in_page);
        assert_eq!(s.placement(), Some(Placement::InPage));
        assert_eq!(fx, [Disarm, FocusInput]);
        // Ctrl+L again on the start page: still in the page, the field takes the keys again
        let (s, fx) = command(s, in_page);
        assert_eq!(s.placement(), Some(Placement::InPage));
        assert_eq!(fx, [Disarm, FocusInput]);
        // it keeps its place while it leaves, and leaves as the overlay does
        let (s, fx) = command(s, CommandEvent::Close);
        assert_eq!(
            (s.placement(), fx),
            (Some(Placement::InPage), vec![StartExit])
        );
        let (s, fx) = command(s, CommandEvent::Settled);
        assert_eq!((s, fx), (CommandBar::Closed, vec![RestoreFocus]));
        assert_eq!(CommandBar::Closed.placement(), None);
        // an overlay bar opened elsewhere moves into the page when the page asks
        let s = command(CommandBar::Closed, open(CommandMode::Here)).0;
        let s = command(s, in_page).0;
        assert_eq!(s.placement(), Some(Placement::InPage));
        // a pick closes it at once
        assert_eq!(
            command(s, CommandEvent::Pick),
            (CommandBar::Closed, vec![RestoreFocus])
        );
    }

    #[test]
    fn compact_is_the_designs_expression() {
        use SidebarMode::*;
        // (live width, mode, peeking) -> compact
        let table = [
            (139.0, Extended, false, true),
            (140.0, Extended, false, false),
            (139.0, Rail, false, true),
            (140.0, Rail, false, true),
            // closing from wide: not compact until it is narrow
            (139.0, Hidden, false, true),
            (140.0, Hidden, false, false),
        ];
        for (w, mode, peeking, want) in table {
            assert_eq!(compact(w, mode, peeking), want, "{w} {mode:?} {peeking}");
        }
        for mode in MODES {
            for w in [0.0, 139.0, 140.0, 340.0] {
                assert!(!compact(w, mode, true), "a peek is never compact");
            }
        }
    }

    fn parts(url: &str) -> (String, String) {
        address_parts(url, false)
    }

    #[test]
    fn full_urls_keep_a_sites_scheme() {
        for (url, want) in [
            (
                "https://example.com/a?q=1",
                ("https://example.com", "/a?q=1"),
            ),
            ("http://localhost:8080/", ("http://localhost:8080", "/")),
            ("https://example.com", ("https://example.com", "")),
            ("file:///home/a.txt", ("file://", "/home/a.txt")),
            // wubble's own pages, no page and data pages read as without
            ("wubble://settings", ("wubble", "://settings")),
            ("", ("wubble", "://start")),
            ("data:,x", ("data", "")),
            ("about:version", ("about:", "version")),
        ] {
            assert_eq!(
                address_parts(url, true),
                (want.0.to_owned(), want.1.to_owned()),
                "{url}"
            );
        }
    }

    #[test]
    fn a_site_shows_its_host_and_the_rest() {
        assert_eq!(
            parts("https://example.com/a/b?q=1"),
            ("example.com".into(), "/a/b?q=1".into())
        );
        assert_eq!(
            parts("http://localhost:8080/"),
            ("localhost:8080".into(), "/".into())
        );
    }

    #[test]
    fn own_pages_read_wubble_and_their_name() {
        assert_eq!(
            parts("wubble://settings"),
            ("wubble".into(), "://settings".into())
        );
        assert_eq!(
            parts("wubble://history"),
            ("wubble".into(), "://history".into())
        );
    }

    #[test]
    fn no_page_is_the_start_page() {
        for url in ["", "  ", "about:blank", "ABOUT:BLANK"] {
            assert_eq!(parts(url), ("wubble".into(), "://start".into()), "{url:?}");
        }
    }

    #[test]
    fn a_data_page_never_shows_its_body() {
        assert_eq!(
            parts("data:text/html;charset=utf-8,%3Ch1%3Ehi"),
            ("data".into(), String::new())
        );
        assert_eq!(parts("DATA:,x"), ("data".into(), String::new()));
    }

    #[test]
    fn only_https_sites_get_the_lock() {
        assert!(is_secure("https://example.com/"));
        assert!(is_secure("HTTPS://example.com"));
        for url in [
            "http://example.com",
            "wubble://settings",
            "",
            "data:,x",
            "file:///x",
        ] {
            assert!(!is_secure(url), "{url}");
        }
    }
}
