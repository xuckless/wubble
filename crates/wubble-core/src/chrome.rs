//! The chrome's pure rules (the design's `browser/chrome.tsx` and `command.tsx`): what the
//! address pill shows, when the sidebar is compact, and the hidden sidebar's peek. Pure, no
//! timers: the host runs the effects.

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
/// (chrome.tsx `onMove`: `setTimeout(() => setPeek(true), 60)`).
pub const PEEK_IN_MS: u32 = 60;
/// How long after the pointer left the peeking sidebar it goes, ms (chrome.tsx `leaveSide`).
pub const PEEK_OUT_MS: u32 = 260;

/// The hidden sidebar's peek (chrome.tsx:250-257): a presentation of [`SidebarMode::Hidden`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Peek {
    #[default]
    Idle,
    /// The pointer moved in the edge strip; it shows when the timer fires.
    Arming,
    Shown,
    /// The pointer left the panel; it goes when the timer fires.
    Leaving,
}

impl Peek {
    /// The panel is on screen (sliding in, resting or about to go).
    pub fn shown(self) -> bool {
        matches!(self, Peek::Shown | Peek::Leaving)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekEvent {
    /// The pointer entered the edge strip (`.wb-peek-edge` onPointerEnter): shows at once.
    EdgeEntered,
    /// The pointer moved in the strip a hidden sidebar peeks from without entering the edge
    /// element (wubble: the window's west resize strip over it): arms the 60 ms timer.
    StripMoved,
    /// The pointer is over the panel or the strip again.
    PanelEntered,
    /// The pointer left the panel and the strip (`leaveSide`).
    PanelLeft,
    TimerFired,
    /// The sidebar's mode changed (`useEffect(() => { if (!hidden) setPeek(false) })`).
    ModeChanged(SidebarMode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekEffect {
    /// (Re)start the one timer, ms.
    Arm(u32),
    Disarm,
    Show,
    Hide,
}

/// One step of the peek.
pub fn peek(state: Peek, event: PeekEvent) -> (Peek, Vec<PeekEffect>) {
    use Peek::*;
    use PeekEffect::*;
    use PeekEvent::*;
    match (state, event) {
        (s, ModeChanged(m)) if m != SidebarMode::Hidden => {
            let mut fx = vec![Disarm];
            if s.shown() {
                fx.push(Hide);
            }
            (Idle, fx)
        }
        (_, ModeChanged(_)) => (state, vec![]),
        (Idle, EdgeEntered) => (Shown, vec![Show]),
        (Arming, EdgeEntered) => (Shown, vec![Disarm, Show]),
        (Idle | Arming, StripMoved) => (Arming, vec![Arm(PEEK_IN_MS)]),
        (Arming, TimerFired) => (Shown, vec![Show]),
        (Arming, PanelLeft) => (Idle, vec![Disarm]),
        (Shown, PanelLeft) => (Leaving, vec![Arm(PEEK_OUT_MS)]),
        (Leaving, EdgeEntered | StripMoved | PanelEntered) => (Shown, vec![Disarm]),
        (Leaving, TimerFired) => (Idle, vec![Hide]),
        (s, _) => (s, vec![]),
    }
}

/// What the address pill shows for `url`: the host in ink and the rest in haze (chrome.tsx
/// `addressOf`). wubble's own pages read `wubble` + `://settings`; no page (a new tab, the start
/// page) reads `wubble` + `://start`, as the design's; a `data:` page (bundled pages) is named
/// by its scheme alone, never its raw body.
pub fn address_parts(url: &str) -> (String, String) {
    let u = url.trim();
    if u.is_empty() || u.eq_ignore_ascii_case("about:blank") {
        return ("wubble".into(), "://start".into());
    }
    if u.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("data:")) {
        return ("data".into(), String::new());
    }
    let (host, path) = url::address_parts(u);
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
    use PeekEffect::*;
    use PeekEvent::*;

    const MODES: [SidebarMode; 3] = [
        SidebarMode::Extended,
        SidebarMode::Rail,
        SidebarMode::Hidden,
    ];

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

    #[test]
    fn the_edge_shows_the_peek_at_once() {
        assert_eq!(peek(Peek::Idle, EdgeEntered), (Peek::Shown, vec![Show]));
        assert_eq!(
            peek(Peek::Arming, EdgeEntered),
            (Peek::Shown, vec![Disarm, Show])
        );
    }

    #[test]
    fn a_move_in_the_strip_arms_and_rearms() {
        assert_eq!(
            peek(Peek::Idle, StripMoved),
            (Peek::Arming, vec![Arm(PEEK_IN_MS)])
        );
        assert_eq!(
            peek(Peek::Arming, StripMoved),
            (Peek::Arming, vec![Arm(PEEK_IN_MS)])
        );
        assert_eq!(peek(Peek::Arming, TimerFired), (Peek::Shown, vec![Show]));
    }

    #[test]
    fn leaving_the_panel_cancels_arming() {
        assert_eq!(peek(Peek::Arming, PanelLeft), (Peek::Idle, vec![Disarm]));
    }

    #[test]
    fn the_peek_goes_a_while_after_the_pointer_left() {
        assert_eq!(
            peek(Peek::Shown, PanelLeft),
            (Peek::Leaving, vec![Arm(PEEK_OUT_MS)])
        );
        assert_eq!(peek(Peek::Leaving, TimerFired), (Peek::Idle, vec![Hide]));
        for back in [PanelEntered, EdgeEntered, StripMoved] {
            assert_eq!(
                peek(Peek::Leaving, back),
                (Peek::Shown, vec![Disarm]),
                "{back:?}"
            );
        }
    }

    #[test]
    fn a_stale_timer_or_a_repeat_does_nothing() {
        for (s, e) in [
            (Peek::Idle, TimerFired),
            (Peek::Shown, TimerFired),
            (Peek::Idle, PanelLeft),
            (Peek::Idle, PanelEntered),
            (Peek::Shown, EdgeEntered),
            (Peek::Shown, StripMoved),
            (Peek::Shown, PanelEntered),
            (Peek::Leaving, PanelLeft),
            (Peek::Arming, PanelEntered),
        ] {
            assert_eq!(peek(s, e), (s, vec![]), "{s:?} {e:?}");
        }
    }

    #[test]
    fn a_mode_other_than_hidden_ends_the_peek_from_any_state() {
        for s in [Peek::Idle, Peek::Arming, Peek::Shown, Peek::Leaving] {
            for m in [SidebarMode::Extended, SidebarMode::Rail] {
                let (to, fx) = peek(s, ModeChanged(m));
                assert_eq!(to, Peek::Idle);
                assert_eq!(fx.first(), Some(&Disarm), "{s:?}");
                assert_eq!(fx.contains(&Hide), s.shown(), "{s:?}");
            }
            assert_eq!(peek(s, ModeChanged(SidebarMode::Hidden)), (s, vec![]));
        }
    }

    fn parts(url: &str) -> (String, String) {
        address_parts(url)
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
