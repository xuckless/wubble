//! The error panel over a tab's page: a failed load, a refused certificate,
//! a crashed renderer. One enum with a pure transition; the host executes
//! the effects (reload, back, proceed).
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

use wubble_core::neterror;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PageError {
    #[default]
    None,
    Load {
        url: String,
        code: i32,
    },
    /// The engine was told to refuse the certificate; the user may proceed.
    Cert {
        url: String,
        code: i32,
    },
    Crashed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorEvent {
    LoadFailed {
        url: String,
        code: i32,
    },
    /// A certificate error the host refused (not in the session's
    /// exceptions).
    CertRefused {
        url: String,
        code: i32,
    },
    /// The renderer process ended.
    Crashed,
    /// The tab committed `url`.
    Committed(String),
    /// The tab started loading (a reload, a new navigation, the engine's
    /// own reload after a crash).
    LoadStarted,
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
    Reload,
    Back,
    /// Nowhere to go back to: leave for an empty page.
    Blank,
    /// Add (host, code) to the session's exceptions, then load `url` again.
    AllowAndLoad {
        host: String,
        code: i32,
        url: String,
    },
}

pub fn transition(state: PageError, event: ErrorEvent) -> (PageError, Vec<ErrorEffect>) {
    use ErrorEvent as E;
    use PageError as S;
    let open = state != S::None;
    match (state, event) {
        // A refused certificate fails its load too: the panel already says why.
        (
            S::Cert { url, code },
            E::LoadFailed {
                url: failed,
                code: c,
            },
        ) if failed == url && neterror::is_cert(c) => (S::Cert { url, code }, vec![]),
        (_, E::LoadFailed { url, code }) => (S::Load { url, code }, vec![]),
        (_, E::CertRefused { url, code }) => (S::Cert { url, code }, vec![]),
        (_, E::Crashed) => (S::Crashed, vec![]),
        // Chromium commits the error document at the failed URL; anything
        // else committing means the page moved on.
        (s @ (S::Load { .. } | S::Cert { .. }), E::Committed(at)) if s.url() == Some(&at) => {
            (s, vec![])
        }
        // A new load: whatever it brings replaces the panel (a failure
        // raises it again).
        (_, E::Committed(_) | E::LoadStarted) => (S::None, vec![]),
        (_, E::Reload) if open => (S::None, vec![ErrorEffect::Reload]),
        (_, E::GoBack { can_go_back }) if open => (
            S::None,
            vec![if can_go_back {
                ErrorEffect::Back
            } else {
                ErrorEffect::Blank
            }],
        ),
        (S::Cert { url, code }, E::Proceed) if neterror::overridable(code) => {
            match wubble_core::zoom::host_of(&url) {
                Some(host) => (S::None, vec![ErrorEffect::AllowAndLoad { host, code, url }]),
                None => (S::Cert { url, code }, vec![]),
            }
        }
        (_, E::ViewGone) => (S::None, vec![]),
        (s, E::Reload | E::GoBack { .. } | E::Proceed) => (s, vec![]),
    }
}

impl PageError {
    fn url(&self) -> Option<&String> {
        match self {
            Self::Load { url, .. } | Self::Cert { url, .. } => Some(url),
            Self::None | Self::Crashed => None,
        }
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
        PageError::Load { url, code } => {
            let (title, summary) = neterror::describe(*code, &host(url));
            ErrorText {
                title: title.into(),
                summary,
                detail: detail(url, *code),
                primary: "Reload",
                secondary: String::new(),
                cert: false,
            }
        }
        PageError::Cert { url, code } => {
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

    fn cert(code: i32) -> PageError {
        transition(
            PageError::None,
            E::CertRefused {
                url: URL.into(),
                code,
            },
        )
        .0
    }

    #[test]
    fn a_refused_certificate_shows_and_its_load_error_changes_nothing() {
        let s = cert(-202);
        assert_eq!(
            s,
            PageError::Cert {
                url: URL.into(),
                code: -202
            }
        );
        let later = E::LoadFailed {
            url: URL.into(),
            code: -202,
        };
        assert_eq!(transition(s.clone(), later), (s.clone(), vec![]));
        // The error page commits at the same URL: still shown.
        assert_eq!(transition(s.clone(), E::Committed(URL.into())), (s, vec![]));
    }

    #[test]
    fn proceed_allows_and_loads_again() {
        assert_eq!(
            transition(cert(-202), E::Proceed),
            (
                PageError::None,
                vec![ErrorEffect::AllowAndLoad {
                    host: "a.test".into(),
                    code: -202,
                    url: URL.into()
                }]
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
        let load = PageError::Load {
            url: URL.into(),
            code: -105,
        };
        assert_eq!(transition(load.clone(), E::Proceed), (load, vec![]));
    }

    #[test]
    fn go_back_or_leave() {
        assert_eq!(
            transition(cert(-202), E::GoBack { can_go_back: true }),
            (PageError::None, vec![ErrorEffect::Back])
        );
        assert_eq!(
            transition(cert(-202), E::GoBack { can_go_back: false }),
            (PageError::None, vec![ErrorEffect::Blank])
        );
        assert_eq!(
            transition(PageError::None, E::GoBack { can_go_back: true }),
            (PageError::None, vec![])
        );
    }

    #[test]
    fn load_errors_show_until_the_page_moves_on() {
        let (s, _) = transition(
            PageError::None,
            E::LoadFailed {
                url: URL.into(),
                code: -105,
            },
        );
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
            (PageError::None, vec![ErrorEffect::Reload])
        );
        assert_eq!(
            transition(PageError::None, E::Reload),
            (PageError::None, vec![])
        );
    }

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
            (PageError::None, vec![ErrorEffect::Reload])
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
            },
        ] {
            assert_eq!(transition(s, E::ViewGone), (PageError::None, vec![]));
        }
        assert_eq!(error_text(&PageError::None), None);
    }
}
