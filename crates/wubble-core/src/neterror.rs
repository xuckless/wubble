//! Chromium's network errors as the error panel shows them, and the
//! session's certificate exceptions.
//!
//! The code table is ported from Chromium `net/base/net_error_list.h`
//! (BSD-3-Clause; every number checked against CEF 156's `cef_errorcode_t`).
//! Titles and summaries follow the register of
//! `components/error_page/common/localized_error.cc` and, for certificates,
//! `components/security_interstitials/core/ssl_error_ui.cc` (BSD-3-Clause);
//! the sentences are short rewrites, not copies. Session-only certificate
//! exceptions are the idea of Chromium's `SSLHostStateDelegate::AllowCert`
//! (in memory, per host and error, for the browsing session); no code.

use std::collections::BTreeSet;

/// `(code, name)`, the errors wubble names. Others show as `ERR_<code>`.
pub const NAMES: &[(i32, &str)] = &[
    (-2, "ERR_FAILED"),
    (-6, "ERR_FILE_NOT_FOUND"),
    (-7, "ERR_TIMED_OUT"),
    (-10, "ERR_ACCESS_DENIED"),
    (-20, "ERR_BLOCKED_BY_CLIENT"),
    (-21, "ERR_NETWORK_CHANGED"),
    (-100, "ERR_CONNECTION_CLOSED"),
    (-101, "ERR_CONNECTION_RESET"),
    (-102, "ERR_CONNECTION_REFUSED"),
    (-104, "ERR_CONNECTION_FAILED"),
    (-105, "ERR_NAME_NOT_RESOLVED"),
    (-106, "ERR_INTERNET_DISCONNECTED"),
    (-107, "ERR_SSL_PROTOCOL_ERROR"),
    (-109, "ERR_ADDRESS_UNREACHABLE"),
    (-113, "ERR_SSL_VERSION_OR_CIPHER_MISMATCH"),
    (-118, "ERR_CONNECTION_TIMED_OUT"),
    (-200, "ERR_CERT_COMMON_NAME_INVALID"),
    (-201, "ERR_CERT_DATE_INVALID"),
    (-202, "ERR_CERT_AUTHORITY_INVALID"),
    (-203, "ERR_CERT_CONTAINS_ERRORS"),
    (-204, "ERR_CERT_NO_REVOCATION_MECHANISM"),
    (-205, "ERR_CERT_UNABLE_TO_CHECK_REVOCATION"),
    (-206, "ERR_CERT_REVOKED"),
    (-207, "ERR_CERT_INVALID"),
    (-208, "ERR_CERT_WEAK_SIGNATURE_ALGORITHM"),
    (-210, "ERR_CERT_NON_UNIQUE_NAME"),
    (-211, "ERR_CERT_WEAK_KEY"),
    (-212, "ERR_CERT_NAME_CONSTRAINT_VIOLATION"),
    (-213, "ERR_CERT_VALIDITY_TOO_LONG"),
    (-217, "ERR_CERT_KNOWN_INTERCEPTION_BLOCKED"),
    (-219, "ERR_CERT_SELF_SIGNED_LOCAL_NETWORK"),
    (-300, "ERR_INVALID_URL"),
    (-301, "ERR_DISALLOWED_URL_SCHEME"),
    (-302, "ERR_UNKNOWN_URL_SCHEME"),
    (-310, "ERR_TOO_MANY_REDIRECTS"),
    (-312, "ERR_UNSAFE_PORT"),
    (-324, "ERR_EMPTY_RESPONSE"),
    (-354, "ERR_CONTENT_LENGTH_MISMATCH"),
    (-355, "ERR_INCOMPLETE_CHUNKED_ENCODING"),
];

/// "ERR_NAME_NOT_RESOLVED", or "ERR_-123" for a code the table lacks.
pub fn name(code: i32) -> String {
    NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| format!("ERR_{code}"), |(_, n)| (*n).to_string())
}

/// A transfer that broke off, in a few words: a media download's reason, where the error
/// panel's page-sized sentences do not fit.
pub fn transfer_failure(code: i32) -> String {
    let words = match code {
        -100 | -101 | -324 | -354 | -355 => "the connection closed before the end of the file",
        -102 | -104 => "the server refused the connection",
        -105 => "the server's address could not be found",
        -106 | -21 => "the network went away",
        -7 | -118 => "the connection timed out",
        c if is_cert(c) => "the server's certificate is not valid",
        _ => "the transfer failed",
    };
    format!("{words} ({})", name(code))
}

/// A certificate error: Chromium's -200 to -219 range (`-220` is the
/// range's end marker, not an error).
pub fn is_cert(code: i32) -> bool {
    (-219..=-200).contains(&code)
}

/// Whether the user may proceed past this certificate error. Never for a
/// revoked certificate or a known interception (Chromium shows those
/// without a way through); other certificate errors may be overridden.
pub fn overridable(code: i32) -> bool {
    is_cert(code) && !matches!(code, -206 | -217)
}

/// What the panel says about `code` on `host`: a title and one sentence.
pub fn describe(code: i32, host: &str) -> (&'static str, String) {
    const UNREACHABLE: &str = "This site can't be reached";
    match code {
        c if is_cert(c) => (
            "Your connection is not private",
            format!(
                "Attackers might be trying to steal your information from {host}. \
                 Its certificate cannot be trusted."
            ),
        ),
        -105 => (
            UNREACHABLE,
            format!("{host}'s server IP address could not be found."),
        ),
        -102 => (UNREACHABLE, format!("{host} refused to connect.")),
        -7 | -118 => (UNREACHABLE, format!("{host} took too long to respond.")),
        -101 => (UNREACHABLE, "The connection was reset.".into()),
        -106 => (
            "No internet",
            "This device is not connected to the internet.".into(),
        ),
        -21 => (
            "Your connection was interrupted",
            "The network changed.".into(),
        ),
        -107 | -113 => (
            "This site can't provide a secure connection",
            format!("{host} sent a response wubble cannot use securely."),
        ),
        -324 => (
            "This page isn't working",
            format!("{host} didn't send any data."),
        ),
        -310 => (
            "This page isn't working",
            format!("{host} redirected you too many times."),
        ),
        -20 => (
            "This page has been blocked",
            format!("wubble's content blocker blocked {host}."),
        ),
        -6 | -10 => (
            "Your file couldn't be accessed",
            "It may have been moved, edited or deleted.".into(),
        ),
        -300 | -301 | -302 | -312 => (
            UNREACHABLE,
            "The address is not one wubble can open.".into(),
        ),
        _ => (UNREACHABLE, format!("{host} could not be loaded.")),
    }
}

/// The network errors a failed page is reloaded after on its own: the ones that end when the
/// network comes back (Chromium's `NetErrorAutoReloader`,
/// `components/error_page/content/browser/net_error_auto_reloader.cc`, BSD-3-Clause; behaviour
/// only): no network, a name not resolved, the network changed, a connection that failed,
/// closed, reset, was refused or aborted, an unreachable address, a timeout. Never a certificate
/// error, a blocked or refused address, or a page the server answered.
const RETRYABLE: &[i32] = &[
    -7,   // ERR_TIMED_OUT
    -21,  // ERR_NETWORK_CHANGED
    -100, // ERR_CONNECTION_CLOSED
    -101, // ERR_CONNECTION_RESET
    -102, // ERR_CONNECTION_REFUSED
    -103, // ERR_CONNECTION_ABORTED
    -104, // ERR_CONNECTION_FAILED
    -105, // ERR_NAME_NOT_RESOLVED
    -106, // ERR_INTERNET_DISCONNECTED
    -109, // ERR_ADDRESS_UNREACHABLE
    -118, // ERR_CONNECTION_TIMED_OUT
    -137, // ERR_NAME_RESOLUTION_FAILED
];

/// Whether a load that failed with `code` is reloaded on its own ([`RETRYABLE`]).
pub fn retryable(code: i32) -> bool {
    RETRYABLE.contains(&code)
}

/// How long after each failure the page is reloaded again, in ms: the backoff of Chromium's
/// auto-reloader (1 s, 5 s, 30 s, 1 min, 5 min, 10 min); later attempts keep the last.
const RETRY_DELAYS_MS: &[u64] = &[1_000, 5_000, 30_000, 60_000, 300_000, 600_000];

/// The wait before reload number `attempt` (0: the first).
pub fn retry_delay_ms(attempt: u32) -> u64 {
    let last = RETRY_DELAYS_MS.len() - 1;
    RETRY_DELAYS_MS[usize::try_from(attempt).map_or(last, |a| a.min(last))]
}

/// Certificate errors the user chose to proceed past, per host and error,
/// for this run only (never stored).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CertExceptions(BTreeSet<(String, i32)>);

impl CertExceptions {
    pub fn allow(&mut self, host: &str, code: i32) {
        self.0.insert((host.to_ascii_lowercase(), code));
    }

    pub fn allowed(&self, host: &str, code: i32) -> bool {
        self.0.contains(&(host.to_ascii_lowercase(), code))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_errors_retry_others_never() {
        for (code, want) in [
            (-106, true),
            (-105, true),
            (-21, true),
            (-100, true),
            (-101, true),
            (-102, true),
            (-104, true),
            (-109, true),
            (-7, true),
            (-118, true),
            (-202, false),
            (-206, false),
            (-20, false),
            (-3, false),
            (-6, false),
            (-300, false),
            (-310, false),
            (-324, false),
            (0, false),
        ] {
            assert_eq!(retryable(code), want, "{code}");
        }
    }

    #[test]
    fn the_backoff_grows_then_holds() {
        for (attempt, want) in [
            (0, 1_000),
            (1, 5_000),
            (2, 30_000),
            (3, 60_000),
            (4, 300_000),
            (5, 600_000),
            (6, 600_000),
            (u32::MAX, 600_000),
        ] {
            assert_eq!(retry_delay_ms(attempt), want, "{attempt}");
        }
    }

    /// The numbers are Chromium's and never change; this pins the table.
    #[test]
    fn the_code_table_is_pinned() {
        let pinned = [
            (-2, "ERR_FAILED"),
            (-20, "ERR_BLOCKED_BY_CLIENT"),
            (-102, "ERR_CONNECTION_REFUSED"),
            (-105, "ERR_NAME_NOT_RESOLVED"),
            (-106, "ERR_INTERNET_DISCONNECTED"),
            (-118, "ERR_CONNECTION_TIMED_OUT"),
            (-200, "ERR_CERT_COMMON_NAME_INVALID"),
            (-201, "ERR_CERT_DATE_INVALID"),
            (-202, "ERR_CERT_AUTHORITY_INVALID"),
            (-206, "ERR_CERT_REVOKED"),
            (-217, "ERR_CERT_KNOWN_INTERCEPTION_BLOCKED"),
            (-219, "ERR_CERT_SELF_SIGNED_LOCAL_NETWORK"),
            (-310, "ERR_TOO_MANY_REDIRECTS"),
            (-324, "ERR_EMPTY_RESPONSE"),
        ];
        for (code, want) in pinned {
            assert_eq!(name(code), want);
        }
        assert_eq!(NAMES.len(), 39);
        let mut codes: Vec<i32> = NAMES.iter().map(|(c, _)| *c).collect();
        codes.dedup();
        assert_eq!(codes.len(), NAMES.len(), "duplicate code");
        assert_eq!(name(-999), "ERR_-999");
    }

    #[test]
    fn a_broken_transfer_is_said_in_words() {
        assert_eq!(
            transfer_failure(-354),
            "the connection closed before the end of the file (ERR_CONTENT_LENGTH_MISMATCH)"
        );
        assert!(transfer_failure(-105).starts_with("the server's address"));
        assert!(transfer_failure(-999).starts_with("the transfer failed (ERR_-999"));
    }

    #[test]
    fn cert_range() {
        assert!(is_cert(-200) && is_cert(-219));
        assert!(!is_cert(-199) && !is_cert(-220) && !is_cert(-105));
        for (code, n) in NAMES {
            assert_eq!(is_cert(*code), n.starts_with("ERR_CERT_"), "{n}");
        }
    }

    #[test]
    fn which_cert_errors_can_be_passed() {
        assert!(overridable(-202) && overridable(-201) && overridable(-219));
        assert!(!overridable(-206) && !overridable(-217));
        assert!(!overridable(-105), "not a certificate error");
    }

    #[test]
    fn descriptions_name_the_host() {
        let (title, text) = describe(-105, "a.test");
        assert_eq!(title, "This site can't be reached");
        assert_eq!(text, "a.test's server IP address could not be found.");
        assert_eq!(describe(-202, "a.test").0, "Your connection is not private");
        assert!(describe(-12345, "a.test").1.contains("a.test"));
    }

    #[test]
    fn exceptions_are_per_host_and_error() {
        let mut e = CertExceptions::default();
        assert!(!e.allowed("a.test", -202));
        e.allow("A.Test", -202);
        assert!(e.allowed("a.test", -202));
        assert!(e.allowed("A.TEST", -202));
        assert!(!e.allowed("a.test", -201), "another error");
        assert!(!e.allowed("b.test", -202), "another host");
    }
}
