//! `--smoke popup`: pages that open windows through `window.open`, as sign-in pop-ups do, on two
//! loopback origins (`127.0.0.1` opens, `localhost` is opened). Each window is a real CEF pop-up
//! browser the host adopted, so the opener relation holds both ways. Clicks go to the opener's
//! view through CEF's input (a user gesture); every wait is "until the host sees X", bounded by a
//! count of CEF work runs, never by time.
//!
//! 1. The opener's `window.open` while loading, with no gesture: blocked (`null`), nothing opens.
//! 2. "Sign in": `window.open('', 'signin', 'width=500,height=600')`, then `w.location` set to the
//!    other origin's page. A Popup window shows that page (its title, set by its own script, says
//!    `window.opener` was set), with a frame; it posts a token the opener counts. Told to by the
//!    opener, it calls `window.close()`: the window goes.
//! 3. `window.open(url, '_blank')`: a tab after the opener (same Space and container) whose page
//!    sees its opener; a `rel=noopener` link: another tab whose page sees none.
//! 4. The same named target opened twice: one Popup window, showing the second page.
//! 5. "Sign in" again, then the opener's tab is closed: the pop-up lives on and runs script
//!    (it sees its opener closed).

use crate::nav_smoke::MAX_WORK_RUNS;
use wubble_engine::input::Point;

/// The opener page, served at `/popup` on `127.0.0.1`; `{PORT}` is the server's port. Its title
/// reports what it saw: whether the load-time `window.open` was blocked, and the tokens posted
/// back to it.
pub const OPENER_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8"><title>popup opener</title>
<style>
body { margin: 0; font: 16px sans-serif; background: #f4f0e8; }
button, a { position: absolute; left: 20px; width: 300px; height: 60px; display: block; }
</style>
<button style="top: 20px" onclick="signin()">Sign in</button>
<button style="top: 100px" onclick="tab()">New tab</button>
<a style="top: 180px" href="http://localhost:{PORT}/popup/child" target="_blank" rel="noopener">No opener</a>
<button style="top: 260px" onclick="named()">Named</button>
<script>
const other = 'http://localhost:{PORT}';
let tokens = 0, n = 0;
const blocked = window.open(other + '/popup/child', '_blank') === null;
function show() {
  document.title = 'popup opener ' + (blocked ? 'blocked' : 'opened') + ' tokens=' + tokens;
}
show();
function signin() {
  const w = window.open('', 'signin', 'width=500,height=600');
  window.signin_window = w;
  if (w) w.location = other + '/popup/signin';
}
function tab() { window.open(other + '/popup/child', '_blank'); }
function named() { window.open(other + '/popup/named?n=' + (++n), 'reuse', 'width=400,height=300'); }
addEventListener('message', (e) => {
  if (e.data === 'tok42') { tokens++; show(); }
});
</script>
"#;

/// The sign-in page, `/popup/signin` on `localhost`: tells its opener the token and closes
/// itself when the opener says so.
pub const SIGNIN_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8"><title>signin</title>
<body style="margin: 0; background: #3a7d5c; color: white; font: 24px sans-serif">Signed in
<script>
const has = window.opener !== null;
document.title = 'signin opener=' + has;
if (has) window.opener.postMessage('tok42', '*');
addEventListener('message', (e) => { if (e.data === 'close') window.close(); });
</script>
"#;

/// `/popup/child`: says whether it has an opener.
pub const CHILD_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8"><title>child</title>
<body style="background: #d8e4f0">child
<script>document.title = 'child opener=' + (window.opener !== null);</script>
"#;

/// `/popup/named?n=N`: says which time it was opened.
pub const NAMED_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8"><title>named</title>
<body style="background: #f0d8e4">named
<script>document.title = 'named ' + new URLSearchParams(location.search).get('n');</script>
"#;

pub fn opener_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/popup")
}

/// Where the opener page puts its controls, view-relative logical px.
pub const SIGNIN_AT: Point = Point { x: 170.0, y: 50.0 };
pub const TAB_AT: Point = Point { x: 170.0, y: 130.0 };
pub const NOOPENER_AT: Point = Point { x: 170.0, y: 210.0 };
pub const NAMED_AT: Point = Point { x: 170.0, y: 290.0 };

const SIGNED_IN: &str = "signin opener=true";
const WITH_OPENER: &str = "child opener=true";
const WITHOUT_OPENER: &str = "child opener=false";
/// The pop-up's title once the smoke's script ran in it after its opener closed (Chromium then
/// answers `window.opener` with null).
pub const ALIVE: &str = "alive opener-closed=true";
/// Run in the pop-up after its opener's tab closed.
pub const ALIVE_SCRIPT: &str =
    "document.title = 'alive opener-closed=' + (window.opener === null || window.opener.closed)";
/// Runs of CEF work between two runs of [`ALIVE_SCRIPT`].
const ASK_EVERY: u32 = 50;
/// Run in the opener: the sign-in pop-up closes itself.
pub const CLOSE_SIGNIN_SCRIPT: &str = "window.signin_window.postMessage('close', '*')";

/// The newest Popup window's tab, as the host sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopupSeen {
    pub title: String,
    /// What its view loaded (the model's URL).
    pub url: String,
    /// Its window has a frame of it.
    pub framed: bool,
}

/// What the host sees after a run of CEF work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The opener tab's title; `None` once it is gone.
    pub opener: Option<String>,
    /// The opener tab is shown and has a frame (it can be clicked).
    pub opener_framed: bool,
    pub popup_windows: usize,
    pub popup: Option<PopupSeen>,
    /// Titles of the tabs after the opener in its Space (Popup windows' tabs left out), in order,
    /// each in the opener's container.
    pub after_opener: Vec<String>,
    /// Some tab after the opener is in another container or Space.
    pub misplaced: bool,
    /// Requests the opener's pop-up pill holds (the badge).
    pub held: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for the opener page's title.
    Loaded,
    /// "Sign in" clicked, waiting for the pop-up's page and the token.
    SignIn,
    /// The pop-up told to close itself, waiting for its window to go.
    SignedIn,
    /// The `_blank` button clicked, waiting for its tab.
    Tab,
    /// The `noopener` link clicked, waiting for its tab.
    NoOpener,
    /// The named target opened once, waiting for its window.
    Named1,
    /// Opened again, waiting for the same window to show the second page.
    Named2,
    /// That window closed by the user, waiting for it to go.
    NamedClosed,
    /// "Sign in" once more, waiting for the pop-up.
    SignIn2,
    /// The opener's tab closed, waiting for the pop-up to answer the script.
    OpenerClosed,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupSmoke {
    pub stage: Stage,
    runs: u32,
}

impl Default for PopupSmoke {
    fn default() -> Self {
        Self {
            stage: Stage::Loaded,
            runs: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Show the opener's tab and click its page there.
    Click(Point),
    /// Run a script in the opener's page.
    InOpener(&'static str),
    /// Close the newest Popup window, as its close button does.
    ClosePopupWindow,
    CloseOpener,
    /// Run a script in the newest Popup window's page.
    InPopup(&'static str),
    Pass,
    Fail(String),
}

fn popup_titled(seen: &Seen, title: &str) -> bool {
    seen.popup_windows == 1
        && seen
            .popup
            .as_ref()
            .is_some_and(|p| p.title == title && p.framed)
}

fn opener_title(seen: &Seen) -> &str {
    seen.opener.as_deref().unwrap_or("")
}

pub fn next(s: PopupSmoke, seen: &Seen) -> (PopupSmoke, Option<Action>) {
    use Stage::*;
    let to = |stage| PopupSmoke { stage, runs: 0 };
    let ready = seen.opener_framed;
    if seen.misplaced {
        return (
            to(Done),
            Some(Action::Fail(format!(
                "a tab the opener opened is outside its Space or container: {seen:?}"
            ))),
        );
    }
    let step = match s.stage {
        Loaded if ready && opener_title(seen).starts_with("popup opener") => {
            if opener_title(seen) != "popup opener blocked tokens=0"
                || seen.popup_windows != 0
                || !seen.after_opener.is_empty()
            {
                return (
                    to(Done),
                    Some(Action::Fail(format!(
                        "a window.open without a gesture was not blocked: {seen:?}"
                    ))),
                );
            }
            Some((SignIn, Action::Click(SIGNIN_AT)))
        }
        SignIn if popup_titled(seen, SIGNED_IN) && opener_title(seen).ends_with("tokens=1") => {
            Some((SignedIn, Action::InOpener(CLOSE_SIGNIN_SCRIPT)))
        }
        SignedIn if seen.popup_windows == 0 && ready => Some((Tab, Action::Click(TAB_AT))),
        Tab if ready && seen.after_opener == [WITH_OPENER] => {
            Some((NoOpener, Action::Click(NOOPENER_AT)))
        }
        // the model puts a foreground tab right after its opener, so the newest comes first
        NoOpener if ready && seen.after_opener == [WITHOUT_OPENER, WITH_OPENER] => {
            Some((Named1, Action::Click(NAMED_AT)))
        }
        Named1 if ready && popup_titled(seen, "named 1") => Some((Named2, Action::Click(NAMED_AT))),
        Named2 if popup_titled(seen, "named 2") => Some((NamedClosed, Action::ClosePopupWindow)),
        NamedClosed if seen.popup_windows == 0 && ready => {
            Some((SignIn2, Action::Click(SIGNIN_AT)))
        }
        SignIn2 if popup_titled(seen, SIGNED_IN) && opener_title(seen).ends_with("tokens=2") => {
            Some((OpenerClosed, Action::CloseOpener))
        }
        OpenerClosed if seen.opener.is_none() && popup_titled(seen, ALIVE) => {
            Some((Done, Action::Pass))
        }
        // the opener's tab is gone and the pop-up is still there: ask it to show it is alive,
        // again now and then until CEF has closed the opener's browser too
        OpenerClosed
            if seen.opener.is_none()
                && seen.popup_windows == 1
                && s.runs.is_multiple_of(ASK_EVERY) =>
        {
            return (
                PopupSmoke {
                    runs: s.runs + 1,
                    ..s
                },
                Some(Action::InPopup(ALIVE_SCRIPT)),
            );
        }
        _ => None,
    };
    match step {
        Some((stage, action)) => (to(stage), Some(action)),
        None if s.stage == Done => (s, None),
        None if s.runs >= MAX_WORK_RUNS => (
            to(Done),
            Some(Action::Fail(format!(
                "stuck at {:?} after {MAX_WORK_RUNS} runs of CEF work: {seen:?}",
                s.stage
            ))),
        ),
        None => (
            PopupSmoke {
                runs: s.runs + 1,
                ..s
            },
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opener(title: &str) -> Seen {
        Seen {
            opener: Some(title.into()),
            opener_framed: true,
            ..Seen::default()
        }
    }

    fn with_popup(mut seen: Seen, title: &str) -> Seen {
        seen.popup_windows = 1;
        seen.popup = Some(PopupSeen {
            title: title.into(),
            url: String::new(),
            framed: true,
        });
        seen
    }

    #[test]
    fn the_steps_in_order() {
        let s = PopupSmoke::default();
        let (s, a) = next(s, &Seen::default());
        assert_eq!(a, None, "the page has not loaded");
        let (s, a) = next(s, &opener("popup opener blocked tokens=0"));
        assert_eq!(a, Some(Action::Click(SIGNIN_AT)));
        let blank = with_popup(opener("popup opener blocked tokens=0"), "");
        let (s, a) = next(s, &blank);
        assert_eq!(a, None, "a blank pop-up is not the sign-in page");
        let signed = with_popup(opener("popup opener blocked tokens=1"), SIGNED_IN);
        let (s, a) = next(s, &signed);
        assert_eq!(a, Some(Action::InOpener(CLOSE_SIGNIN_SCRIPT)));
        let (s, a) = next(s, &signed);
        assert_eq!(a, None, "the window is still there");
        let (s, a) = next(s, &opener("popup opener blocked tokens=1"));
        assert_eq!(a, Some(Action::Click(TAB_AT)));
        let mut tabbed = opener("popup opener blocked tokens=1");
        tabbed.after_opener = vec![WITH_OPENER.into()];
        let (s, a) = next(s, &tabbed);
        assert_eq!(a, Some(Action::Click(NOOPENER_AT)));
        tabbed.after_opener.insert(0, WITHOUT_OPENER.into());
        let (s, a) = next(s, &tabbed);
        assert_eq!(a, Some(Action::Click(NAMED_AT)));
        let (s, a) = next(s, &with_popup(tabbed.clone(), "named 1"));
        assert_eq!(a, Some(Action::Click(NAMED_AT)));
        let mut two = with_popup(tabbed.clone(), "named 2");
        two.popup_windows = 2;
        let (s, a) = next(s, &two);
        assert_eq!(a, None, "a second window instead of the named one");
        let (s, a) = next(s, &with_popup(tabbed.clone(), "named 2"));
        assert_eq!(a, Some(Action::ClosePopupWindow));
        let (s, a) = next(s, &tabbed);
        assert_eq!(a, Some(Action::Click(SIGNIN_AT)));
        let mut again = with_popup(tabbed, SIGNED_IN);
        again.opener = Some("popup opener blocked tokens=2".into());
        let (s, a) = next(s, &again);
        assert_eq!(a, Some(Action::CloseOpener));
        let alone = Seen {
            opener: None,
            opener_framed: false,
            ..again
        };
        let (s, a) = next(s, &alone);
        assert_eq!(a, Some(Action::InPopup(ALIVE_SCRIPT)));
        let (mut s, a) = next(s, &alone);
        assert_eq!(a, None, "not asked again at once");
        let early = with_popup(alone.clone(), "alive opener-closed=false");
        let mut asked = 0;
        for _ in 0..ASK_EVERY {
            let (n, a) = next(s, &early);
            s = n;
            asked += usize::from(a == Some(Action::InPopup(ALIVE_SCRIPT)));
        }
        assert_eq!(asked, 1, "asked again while the opener is still closing");
        let (s, a) = next(s, &with_popup(alone.clone(), ALIVE));
        assert_eq!((s.stage, a), (Stage::Done, Some(Action::Pass)));
    }

    #[test]
    fn an_unblocked_open_fails() {
        let (_, a) = next(
            PopupSmoke::default(),
            &opener("popup opener opened tokens=0"),
        );
        assert!(matches!(a, Some(Action::Fail(_))));
    }

    #[test]
    fn a_tab_elsewhere_fails() {
        let seen = Seen {
            misplaced: true,
            ..opener("popup opener blocked tokens=0")
        };
        assert!(matches!(
            next(PopupSmoke::default(), &seen).1,
            Some(Action::Fail(_))
        ));
    }

    #[test]
    fn a_blank_popup_gets_stuck() {
        let s = PopupSmoke {
            stage: Stage::SignIn,
            runs: MAX_WORK_RUNS,
        };
        let blank = with_popup(opener("popup opener blocked tokens=0"), "");
        assert!(matches!(next(s, &blank).1, Some(Action::Fail(_))));
    }
}
