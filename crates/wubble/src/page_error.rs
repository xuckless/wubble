//! The error panel over a tab's page: a failed load, a certificate error,
//! a crashed renderer. One enum with a pure transition; the host executes
//! the effects (reload, back, the certificate answer). A certificate panel
//! holds the engine's request open: Proceed answers it (the same request,
//! so a POST keeps its body), every other way out refuses it, exactly once.
//!
//! Drawn as a Slint panel over the page rather than a bundled HTML error
//! page (TODO.md Decided): welding has no custom scheme or load-HTML call,
//! and a `data:` page would replace the tab's address, history entry and
//! saved URL, and repeat the design tokens in CSS. Behaviour follows
//! cefclient `client_handler.cc` `OnLoadError` (an error for the failed URL,
//! ERR_ABORTED skipped) and Chromium's SSL interstitial
//! (`components/security_interstitials`: "Back to safety" is the default,
//! proceeding is explicit and remembered for the session), BSD-3-Clause;
//! behaviour and wording only.
//!
//! A load that failed with a network error reloads on its own (Chromium's
//! `NetErrorAutoReloader`, `components/error_page/content/browser/
//! net_error_auto_reloader.cc`, BSD-3-Clause; behaviour only): on a backoff
//! (`neterror::retry_delay_ms`), at once when the system comes back online,
//! only while the tab is shown, never after the user stopped it.

use wubble_core::neterror;
use wubble_engine::page::RequestId;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PageError {
    #[default]
    None,
    Load {
        url: String,
        code: i32,
        retry: Retry,
    },
    /// An automatic reload of the failed `url` is under way; the panel stays until it ends
    /// (`LoadEnded`: the page came) or fails again (`LoadFailed`: the next attempt).
    Reloading {
        url: String,
        code: i32,
        attempt: u32,
    },
    /// The engine waits on `request` for the user's choice.
    Cert {
        url: String,
        code: i32,
        request: RequestId,
    },
    Crashed,
}

/// A failed load's own reload (`PageError::Load`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    /// Not reloaded: not a network error, or the user stopped it.
    Never,
    /// Reload number `attempt` (from 0) is due at `due_ms` (wall clock).
    Due { attempt: u32, due_ms: u64 },
    /// The tab is hidden: reload number `attempt` waits for it to show.
    Paused { attempt: u32 },
}

impl Retry {
    /// Reload number `attempt` of a load that failed with `code`, at `now_ms`, `shown` or not.
    fn after(code: i32, attempt: u32, now_ms: u64, shown: bool) -> Self {
        match (neterror::retryable(code), shown) {
            (false, _) => Self::Never,
            (true, true) => Self::Due {
                attempt,
                due_ms: now_ms.saturating_add(neterror::retry_delay_ms(attempt)),
            },
            (true, false) => Self::Paused { attempt },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorEvent {
    /// The load of `url` failed at `now_ms` (wall clock), its tab `shown` or not.
    LoadFailed {
        url: String,
        code: i32,
        now_ms: u64,
        shown: bool,
    },
    /// A certificate error not in the session's exceptions; the engine
    /// waits on `request`.
    CertError {
        url: String,
        code: i32,
        request: RequestId,
    },
    /// The renderer process ended.
    Crashed,
    /// The tab committed `url`.
    Committed(String),
    /// The tab went from not loading to loading: a reload or a new
    /// navigation (the engine does not reload a crashed page by itself for
    /// this host, so a crash panel stays until the user acts).
    LoadStarted,
    /// The tab stopped loading.
    LoadEnded,
    /// The host's tick, at `now_ms` (wall clock): a due reload runs.
    Tick(u64),
    /// The system came back online (`os_online`): a due reload runs at once.
    Online,
    /// The tab went off screen: its reload waits.
    Hidden,
    /// The tab came on screen at `now_ms`: a waiting reload is due again.
    Shown(u64),
    /// The user stopped the page (Stop, Escape): no reload on its own.
    UserStopped,
    // The panel's buttons.
    Reload,
    GoBack {
        can_go_back: bool,
    },
    Proceed,
    ViewGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorEffect {
    /// Reload the page; `auto`: the failed load's own reload, not the user's.
    Reload {
        auto: bool,
    },
    Back,
    /// Nowhere to go back to: leave for an empty page.
    Blank,
    /// Add (host, code) to the session's exceptions.
    Allow {
        host: String,
        code: i32,
    },
    /// The engine's held certificate request, answered.
    AnswerCert {
        request: RequestId,
        proceed: bool,
    },
}

pub fn transition(state: PageError, event: ErrorEvent) -> (PageError, Vec<ErrorEffect>) {
    // A held request leaving the panel any way but Proceed is refused; a
    // gone view's requests were answered by welding.
    let held = match (&state, &event) {
        (PageError::Cert { request, .. }, e) if *e != ErrorEvent::ViewGone => Some(*request),
        _ => None,
    };
    let (next, mut effects) = step(state, event);
    if let Some(request) = held {
        let kept = matches!(&next, PageError::Cert { request: r, .. } if *r == request);
        let answered = effects
            .iter()
            .any(|e| matches!(e, ErrorEffect::AnswerCert { request: r, .. } if *r == request));
        if !kept && !answered {
            effects.insert(
                0,
                ErrorEffect::AnswerCert {
                    request,
                    proceed: false,
                },
            );
        }
    }
    (next, effects)
}

fn step(state: PageError, event: ErrorEvent) -> (PageError, Vec<ErrorEffect>) {
    use ErrorEvent as E;
    use PageError as S;
    let open = state != S::None;
    match (state, event) {
        // A refused certificate fails its load too: the panel already says why.
        (
            s @ S::Cert { .. },
            E::LoadFailed {
                url: failed,
                code: c,
                ..
            },
        ) if s.url() == Some(&failed) && neterror::is_cert(c) => (s, vec![]),
        // the own reload failed again: the next attempt, later
        (
            S::Reloading {
                url: was, attempt, ..
            },
            E::LoadFailed {
                url,
                code,
                now_ms,
                shown,
            },
        ) if was == url => {
            let retry = Retry::after(code, attempt.saturating_add(1), now_ms, shown);
            (S::Load { url, code, retry }, vec![])
        }
        (
            _,
            E::LoadFailed {
                url,
                code,
                now_ms,
                shown,
            },
        ) => {
            let retry = Retry::after(code, 0, now_ms, shown);
            (S::Load { url, code, retry }, vec![])
        }
        (_, E::CertError { url, code, request }) => (S::Cert { url, code, request }, vec![]),
        (_, E::Crashed) => (S::Crashed, vec![]),
        // Chromium commits the error document at the failed URL; anything
        // else committing means the page moved on.
        (s @ (S::Load { .. } | S::Cert { .. }), E::Committed(at)) if s.url() == Some(&at) => {
            (s, vec![])
        }
        // the own reload: its start and its commit change nothing, its end without a failure
        // is the page back
        (s @ S::Reloading { .. }, E::LoadStarted) => (s, vec![]),
        (s @ S::Reloading { .. }, E::Committed(at)) if s.url() == Some(&at) => (s, vec![]),
        (S::Reloading { .. }, E::LoadEnded) => (S::None, vec![]),
        (S::Reloading { url, code, .. }, E::UserStopped) => (
            S::Load {
                url,
                code,
                retry: Retry::Never,
            },
            vec![],
        ),
        // A new load: whatever it brings replaces the panel (a failure
        // raises it again).
        (_, E::Committed(_) | E::LoadStarted) => (S::None, vec![]),
        (_, E::Reload) if open => (S::None, vec![ErrorEffect::Reload { auto: false }]),
        // the schedule: due and shown, it reloads; online, a due one at once
        (
            S::Load {
                url,
                code,
                retry: Retry::Due { attempt, due_ms },
            },
            event @ (E::Tick(_) | E::Online),
        ) if matches!(event, E::Online) || matches!(event, E::Tick(now) if now >= due_ms) => (
            S::Reloading { url, code, attempt },
            vec![ErrorEffect::Reload { auto: true }],
        ),
        (
            S::Load {
                url,
                code,
                retry: Retry::Due { attempt, .. },
            },
            E::Hidden,
        ) => (
            S::Load {
                url,
                code,
                retry: Retry::Paused { attempt },
            },
            vec![],
        ),
        (
            S::Load {
                url,
                code,
                retry: Retry::Paused { attempt },
            },
            E::Shown(now_ms),
        ) => (
            S::Load {
                url,
                code,
                retry: Retry::after(code, attempt, now_ms, true),
            },
            vec![],
        ),
        (S::Load { url, code, .. }, E::UserStopped) => (
            S::Load {
                url,
                code,
                retry: Retry::Never,
            },
            vec![],
        ),
        (_, E::GoBack { can_go_back }) if open => (
            S::None,
            vec![if can_go_back {
                ErrorEffect::Back
            } else {
                ErrorEffect::Blank
            }],
        ),
        (S::Cert { url, code, request }, E::Proceed) if neterror::overridable(code) => {
            match wubble_core::zoom::host_of(&url) {
                Some(host) => (
                    S::None,
                    vec![
                        ErrorEffect::Allow { host, code },
                        ErrorEffect::AnswerCert {
                            request,
                            proceed: true,
                        },
                    ],
                ),
                None => (S::Cert { url, code, request }, vec![]),
            }
        }
        (_, E::ViewGone) => (S::None, vec![]),
        // every other pair leaves the state: a button with nothing to do, a tick before the
        // reload is due, the schedule's events where nothing is scheduled
        (
            s,
            E::Reload
            | E::GoBack { .. }
            | E::Proceed
            | E::LoadEnded
            | E::Tick(_)
            | E::Online
            | E::Hidden
            | E::Shown(_)
            | E::UserStopped,
        ) => (s, vec![]),
    }
}

impl PageError {
    fn url(&self) -> Option<&String> {
        match self {
            Self::Load { url, .. } | Self::Cert { url, .. } | Self::Reloading { url, .. } => {
                Some(url)
            }
            Self::None | Self::Crashed => None,
        }
    }

    /// The panel says it is trying again: a reload is due or under way.
    fn retrying(&self) -> bool {
        matches!(
            self,
            Self::Reloading { .. }
                | Self::Load {
                    retry: Retry::Due { .. },
                    ..
                }
        )
    }
}

/// What the panel shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorText {
    pub title: String,
    pub summary: String,
    /// The failed URL and the error: "https://a.test/ · ERR_X (-105)".
    pub detail: String,
    /// The default button: "Reload" or "Go back".
    pub primary: &'static str,
    /// The quiet button ("Proceed to a.test (unsafe)"); empty for none.
    pub secondary: String,
    /// A certificate panel: Go back / Proceed instead of Reload.
    pub cert: bool,
}

pub fn error_text(error: &PageError) -> Option<ErrorText> {
    let host = |url: &str| wubble_core::zoom::host_of(url).unwrap_or_else(|| url.to_string());
    let detail = |url: &str, code: i32| format!("{url} \u{b7} {} ({code})", neterror::name(code));
    Some(match error {
        PageError::None => return None,
        PageError::Load { url, code, .. } | PageError::Reloading { url, code, .. } => {
            let (title, summary) = neterror::describe(*code, &host(url));
            // a reload of its own is due or under way
            let trying = if error.retrying() {
                " \u{b7} Trying again\u{2026}"
            } else {
                ""
            };
            ErrorText {
                title: title.into(),
                summary,
                detail: format!("{}{trying}", detail(url, *code)),
                primary: "Reload",
                secondary: String::new(),
                cert: false,
            }
        }
        PageError::Cert { url, code, .. } => {
            let (title, summary) = neterror::describe(*code, &host(url));
            ErrorText {
                title: title.into(),
                summary,
                detail: detail(url, *code),
                primary: "Go back",
                secondary: if neterror::overridable(*code) {
                    format!("Proceed to {} (unsafe)", host(url))
                } else {
                    String::new()
                },
                cert: true,
            }
        }
        PageError::Crashed => ErrorText {
            title: "This page crashed".into(),
            summary: "Something went wrong while showing this page.".into(),
            detail: String::new(),
            primary: "Reload",
            secondary: String::new(),
            cert: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ErrorEvent as E;

    const URL: &str = "https://a.test/x";
    const REQ: RequestId = RequestId(9);

    fn cert(code: i32) -> PageError {
        transition(
            PageError::None,
            E::CertError {
                url: URL.into(),
                code,
                request: REQ,
            },
        )
        .0
    }

    /// The wall clock the schedule tests start at.
    const T0: u64 = 1_000_000;

    fn failed(url: &str, code: i32, shown: bool) -> ErrorEvent {
        E::LoadFailed {
            url: url.into(),
            code,
            now_ms: T0,
            shown,
        }
    }

    fn load(code: i32, retry: Retry) -> PageError {
        PageError::Load {
            url: URL.into(),
            code,
            retry,
        }
    }

    fn reloading(attempt: u32) -> PageError {
        PageError::Reloading {
            url: URL.into(),
            code: -106,
            attempt,
        }
    }

    fn due(attempt: u32, due_ms: u64) -> Retry {
        Retry::Due { attempt, due_ms }
    }

    /// The auto-reload schedule, one row per (state, event): what it becomes and does.
    #[test]
    #[rustfmt::skip]
    fn a_network_failure_reloads_on_its_own() {
        let auto = || vec![ErrorEffect::Reload { auto: true }];
        let cases: Vec<(&str, PageError, ErrorEvent, PageError, Vec<ErrorEffect>)> = vec![
            ("shown: the first reload after 1 s", PageError::None, failed(URL, -106, true),
                load(-106, due(0, T0 + 1_000)), vec![]),
            ("hidden: it waits for the tab", PageError::None, failed(URL, -106, false),
                load(-106, Retry::Paused { attempt: 0 }), vec![]),
            ("a certificate error never", PageError::None, failed(URL, -202, true),
                load(-202, Retry::Never), vec![]),
            ("a blocked address never", PageError::None, failed(URL, -20, true),
                load(-20, Retry::Never), vec![]),
            ("a tick before it is due", load(-106, due(0, T0 + 1_000)), E::Tick(T0 + 999),
                load(-106, due(0, T0 + 1_000)), vec![]),
            ("due: it reloads", load(-106, due(0, T0 + 1_000)), E::Tick(T0 + 1_000),
                reloading(0), auto()),
            ("late: it reloads", load(-106, due(2, T0)), E::Tick(T0 + 90_000), reloading(2), auto()),
            ("online: at once", load(-106, due(3, T0 + 60_000)), E::Online, reloading(3), auto()),
            ("online while hidden: it waits", load(-106, Retry::Paused { attempt: 1 }), E::Online,
                load(-106, Retry::Paused { attempt: 1 }), vec![]),
            ("hidden: the due one waits", load(-106, due(1, T0 + 5_000)), E::Hidden,
                load(-106, Retry::Paused { attempt: 1 }), vec![]),
            ("shown again: due after its delay", load(-106, Retry::Paused { attempt: 1 }), E::Shown(T0),
                load(-106, due(1, T0 + 5_000)), vec![]),
            ("a tick while hidden", load(-106, Retry::Paused { attempt: 1 }), E::Tick(T0 + 900_000),
                load(-106, Retry::Paused { attempt: 1 }), vec![]),
            ("stopped: never", load(-106, due(0, T0)), E::UserStopped, load(-106, Retry::Never), vec![]),
            ("stopped stays stopped", load(-106, Retry::Never), E::Tick(T0 + 900_000),
                load(-106, Retry::Never), vec![]),
            ("never: online changes nothing", load(-106, Retry::Never), E::Online,
                load(-106, Retry::Never), vec![]),
            ("shown: never stays never", load(-202, Retry::Never), E::Shown(T0),
                load(-202, Retry::Never), vec![]),
            ("the reload starts: the panel stays", reloading(0), E::LoadStarted, reloading(0), vec![]),
            ("its commit at the URL: still deciding", reloading(0), E::Committed(URL.into()),
                reloading(0), vec![]),
            ("it fails again: the next delay", reloading(0), failed(URL, -106, true),
                load(-106, due(1, T0 + 5_000)), vec![]),
            ("it fails hidden: it waits", reloading(4), failed(URL, -105, false),
                load(-105, Retry::Paused { attempt: 5 }), vec![]),
            ("it fails with another error: never", reloading(2), failed(URL, -202, true),
                load(-202, Retry::Never), vec![]),
            ("another page fails: a fresh schedule", reloading(4), failed("https://b.test/", -106, true),
                PageError::Load { url: "https://b.test/".into(), code: -106, retry: due(0, T0 + 1_000) }, vec![]),
            ("it ends without failing: the page is back", reloading(1), E::LoadEnded, PageError::None, vec![]),
            ("the user stops it: no more", reloading(1), E::UserStopped, load(-106, Retry::Never), vec![]),
            ("another page commits", reloading(1), E::Committed("https://b.test/".into()), PageError::None, vec![]),
            ("the user's Reload", reloading(1), E::Reload, PageError::None, vec![ErrorEffect::Reload { auto: false }]),
            ("a hidden tab's reload goes on", reloading(1), E::Hidden, reloading(1), vec![]),
            ("the error page's end changes nothing", load(-106, due(0, T0)), E::LoadEnded,
                load(-106, due(0, T0)), vec![]),
            ("no panel: the schedule's events do nothing", PageError::None, E::Online, PageError::None, vec![]),
            ("a crash panel is no network failure", PageError::Crashed, E::Tick(T0 + 900_000),
                PageError::Crashed, vec![]),
        ];
        for (name, state, event, want, effects) in cases {
            assert_eq!(transition(state, event), (want, effects), "{name}");
        }
    }

    #[test]
    fn the_panel_says_it_is_trying_again() {
        for (state, trying) in [
            (load(-106, due(0, T0)), true),
            (reloading(2), true),
            (load(-106, Retry::Paused { attempt: 0 }), false),
            (load(-106, Retry::Never), false),
        ] {
            let detail = error_text(&state).unwrap().detail;
            assert_eq!(
                detail.ends_with("Trying again\u{2026}"),
                trying,
                "{state:?}"
            );
        }
    }

    fn refuse() -> ErrorEffect {
        ErrorEffect::AnswerCert {
            request: REQ,
            proceed: false,
        }
    }

    #[test]
    fn a_refused_certificate_shows_and_its_load_error_changes_nothing() {
        let s = cert(-202);
        assert_eq!(
            s,
            PageError::Cert {
                url: URL.into(),
                code: -202,
                request: REQ,
            }
        );
        let later = failed(URL, -202, false);
        assert_eq!(transition(s.clone(), later), (s.clone(), vec![]));
        // The error page commits at the same URL: still shown.
        assert_eq!(transition(s.clone(), E::Committed(URL.into())), (s, vec![]));
    }

    #[test]
    fn proceed_allows_and_answers_the_held_request() {
        assert_eq!(
            transition(cert(-202), E::Proceed),
            (
                PageError::None,
                vec![
                    ErrorEffect::Allow {
                        host: "a.test".into(),
                        code: -202,
                    },
                    ErrorEffect::AnswerCert {
                        request: REQ,
                        proceed: true
                    }
                ]
            )
        );
        // Revoked: no way through.
        assert_eq!(transition(cert(-206), E::Proceed), (cert(-206), vec![]));
        assert_eq!(error_text(&cert(-206)).unwrap().secondary, "");
        assert_eq!(
            error_text(&cert(-202)).unwrap().secondary,
            "Proceed to a.test (unsafe)"
        );
        // Not a certificate panel: nothing to proceed past.
        let load = load(-105, Retry::Never);
        assert_eq!(transition(load.clone(), E::Proceed), (load, vec![]));
    }

    #[test]
    fn go_back_or_leave() {
        assert_eq!(
            transition(cert(-202), E::GoBack { can_go_back: true }),
            (PageError::None, vec![refuse(), ErrorEffect::Back])
        );
        assert_eq!(
            transition(cert(-202), E::GoBack { can_go_back: false }),
            (PageError::None, vec![refuse(), ErrorEffect::Blank])
        );
        assert_eq!(
            transition(PageError::None, E::GoBack { can_go_back: true }),
            (PageError::None, vec![])
        );
    }

    #[test]
    fn every_other_way_out_refuses_the_held_request_once() {
        let other = E::CertError {
            url: "https://b.test/".into(),
            code: -202,
            request: RequestId(10),
        };
        for event in [
            E::LoadStarted,
            E::Committed("https://b.test/".into()),
            E::Crashed,
            E::Reload,
            failed("https://b.test/", -105, false),
            other,
        ] {
            let (_, effects) = transition(cert(-202), event.clone());
            assert_eq!(
                effects.iter().filter(|e| **e == refuse()).count(),
                1,
                "{event:?}"
            );
        }
        // Staying on the panel answers nothing; a gone view's requests
        // were answered by welding.
        for event in [E::Committed(URL.into()), E::Proceed] {
            assert!(transition(cert(-206), event).1.is_empty());
        }
        assert_eq!(
            transition(cert(-202), E::ViewGone),
            (PageError::None, vec![])
        );
    }

    #[test]
    fn load_errors_show_until_the_page_moves_on() {
        let (s, _) = transition(PageError::None, failed(URL, -105, false));
        let text = error_text(&s).unwrap();
        assert_eq!(text.title, "This site can't be reached");
        assert_eq!(
            text.detail,
            "https://a.test/x \u{b7} ERR_NAME_NOT_RESOLVED (-105)"
        );
        assert_eq!((text.primary, text.cert), ("Reload", false));
        assert_eq!(
            transition(s.clone(), E::LoadStarted),
            (PageError::None, vec![])
        );
        assert_eq!(
            transition(s.clone(), E::Committed(URL.into())),
            (s.clone(), vec![])
        );
        assert_eq!(
            transition(s.clone(), E::Committed("https://b.test/".into())),
            (PageError::None, vec![])
        );
        assert_eq!(
            transition(s, E::Reload),
            (PageError::None, vec![ErrorEffect::Reload { auto: false }])
        );
        assert_eq!(
            transition(PageError::None, E::Reload),
            (PageError::None, vec![])
        );
    }

    /// The engine does not reload a crashed page for this host
    /// (`nav::transition`, `RendererTerminated { reload: false }`): the
    /// panel stays until the user reloads or navigates.
    #[test]
    fn a_crash_shows_until_the_page_loads_again() {
        let (s, _) = transition(PageError::None, E::Crashed);
        assert_eq!(s, PageError::Crashed);
        assert_eq!(error_text(&s).unwrap().title, "This page crashed");
        assert_eq!(
            transition(s.clone(), E::LoadStarted),
            (PageError::None, vec![])
        );
        assert_eq!(
            transition(s.clone(), E::Reload),
            (PageError::None, vec![ErrorEffect::Reload { auto: false }])
        );
        assert_eq!(
            transition(s, E::Committed(URL.into())),
            (PageError::None, vec![])
        );
    }

    #[test]
    fn a_gone_view_clears_every_panel() {
        for s in [
            cert(-202),
            PageError::Crashed,
            PageError::Load {
                url: URL.into(),
                code: -7,
                retry: Retry::Never,
            },
        ] {
            assert_eq!(transition(s, E::ViewGone), (PageError::None, vec![]));
        }
        assert_eq!(error_text(&PageError::None), None);
    }
}
