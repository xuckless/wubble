//! Command-bar suggestions: which sources to consult for a query.
//!
//! Pure planning only. This module decides *whether* a remote request may be
//! made and builds its URL; the fetch itself is an effect run at the edge.
//! Live suggestions send keystrokes to the search engine, so they are off
//! unless the user turns them on.

use crate::omnibox::{Intent, classify};
use crate::search::Engines;
use crate::space::Side;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveSuggestions {
    /// Nothing leaves the machine until the user presses Enter.
    #[default]
    Off,
    On,
}

impl LiveSuggestions {
    /// What the bar of a window on `side` runs: live only when the user turned it on and a
    /// search engine is set, and never in a private window, whose bar suggests from this
    /// machine only (what is typed there is none of a server's business).
    pub fn of(on: bool, engine_set: bool, side: Side) -> Self {
        if on && engine_set && side == Side::Normal {
            Self::On
        } else {
            Self::Off
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuggestSettings<'a> {
    /// The default engine is asked; a keyword query (`bing.com cats`) asks its engine. An
    /// engine without a suggestions endpoint is never asked.
    pub engines: &'a Engines,
    pub live: LiveSuggestions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Open tabs, history, bookmarks.
    Local,
    /// Fetch this URL and feed the body to [`parse_opensearch`].
    Remote { url: String },
}

/// Anything that could be a (partial) address stays on this machine, even
/// when `classify` would still call it a search (`192.168.0`, `http://`).
/// Input `classify` calls a URL (`about:blank`, `[::1]`) never gets here.
fn looks_like_address(q: &str) -> bool {
    q.contains(['.', '/', '@'])
        || q.contains("://")
        || q.to_ascii_lowercase().starts_with("localhost")
}

/// Sources to consult for `query`, in display order.
pub fn sources(query: &str, settings: &SuggestSettings) -> Vec<Source> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = vec![Source::Local];
    if settings.live == LiveSuggestions::Off {
        return out;
    }
    let (engine, query) = match settings.engines.keyword_search(query) {
        Some((engine, rest)) => (Some(engine), rest),
        None => (settings.engines.default.as_ref(), query),
    };
    if let Some(url) = engine
        .filter(|_| matches!(classify(query), Intent::Search(_)) && !looks_like_address(query))
        .and_then(|e| e.suggest_url(query))
    {
        out.push(Source::Remote { url });
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub struct MalformedSuggestions;

/// Parses an OpenSearch suggestions body: `["query", ["a", "b", ...], ...]`.
pub fn parse_opensearch(body: &str) -> Result<Vec<String>, MalformedSuggestions> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| MalformedSuggestions)?;
    value
        .get(1)
        .and_then(|v| v.as_array())
        .ok_or(MalformedSuggestions)?
        .iter()
        .map(|s| s.as_str().map(str::to_owned).ok_or(MalformedSuggestions))
        .collect()
}

/// ms of quiet typing before a remote request goes out.
pub const DEBOUNCE_MS: u64 = 150;

/// Where the remote fetch is. At most one request is in flight (a blocking fetch cannot be
/// aborted): the next one waits for its reply, and a reply for an older query is dropped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Waiting out the debounce for `url`; nothing in flight.
    Debouncing { url: String },
    /// Request `req` is out.
    InFlight { req: u64 },
    /// Request `req` is out and the user is still typing `url`.
    InFlightDebouncing { req: u64, url: String },
    /// Request `req` is out and `url` is due: it goes when `req` comes back.
    InFlightQueued { req: u64, url: String },
}

/// The fetch machine's state: the phase and the number of the latest query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fetch {
    pub phase: Phase,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchEvent {
    /// The command bar's text changed.
    Typed(String),
    /// The debounce armed for query `seq` ran out.
    DebounceElapsed(u64),
    /// Request `seq` came back: the body, or `None` on any failure.
    Reply { seq: u64, body: Option<String> },
    /// The bar closed.
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchEffect {
    /// Call back with `DebounceElapsed(seq)` after [`DEBOUNCE_MS`] (replaces any earlier one).
    ArmDebounce { seq: u64 },
    /// Fetch `url` and reply with `Reply { seq, .. }`.
    Request { seq: u64, url: String },
    /// The live suggestions for the current query.
    Show(Vec<String>),
    /// No live suggestions.
    Clear,
}

/// The live-suggestion fetch. Which input may leave the machine is [`sources`]'s rule alone:
/// address-like input and the `Off` setting never produce a [`FetchEffect::Request`].
pub fn transition(
    s: Fetch,
    e: FetchEvent,
    settings: &SuggestSettings,
) -> (Fetch, Vec<FetchEffect>) {
    use FetchEffect::*;
    use Phase::*;
    let in_flight = |p: &Phase| match p {
        InFlight { req } | InFlightDebouncing { req, .. } | InFlightQueued { req, .. } => {
            Some(*req)
        }
        Idle | Debouncing { .. } => None,
    };
    match e {
        FetchEvent::Typed(q) => {
            let seq = s.seq + 1;
            let remote = sources(&q, settings).into_iter().find_map(|src| match src {
                Source::Remote { url } => Some(url),
                Source::Local => None,
            });
            let req = in_flight(&s.phase);
            match (remote, req) {
                (None, None) => (Fetch { phase: Idle, seq }, vec![Clear]),
                (None, Some(req)) => (
                    Fetch {
                        phase: InFlight { req },
                        seq,
                    },
                    vec![Clear],
                ),
                (Some(url), None) => (
                    Fetch {
                        phase: Debouncing { url },
                        seq,
                    },
                    vec![ArmDebounce { seq }],
                ),
                (Some(url), Some(req)) => (
                    Fetch {
                        phase: InFlightDebouncing { req, url },
                        seq,
                    },
                    vec![ArmDebounce { seq }],
                ),
            }
        }
        FetchEvent::DebounceElapsed(d) if d == s.seq => match s.phase {
            Debouncing { url } => (
                Fetch {
                    phase: InFlight { req: d },
                    seq: d,
                },
                vec![Request { seq: d, url }],
            ),
            InFlightDebouncing { req, url } => (
                Fetch {
                    phase: InFlightQueued { req, url },
                    seq: d,
                },
                vec![],
            ),
            phase => (Fetch { phase, seq: d }, vec![]),
        },
        FetchEvent::DebounceElapsed(_) => (s, vec![]),
        FetchEvent::Reply { seq: r, body } if in_flight(&s.phase) == Some(r) => {
            let seq = s.seq;
            match s.phase {
                InFlightQueued { url, .. } => (
                    Fetch {
                        phase: InFlight { req: seq },
                        seq,
                    },
                    vec![Request { seq, url }],
                ),
                InFlightDebouncing { url, .. } => (
                    Fetch {
                        phase: Debouncing { url },
                        seq,
                    },
                    vec![],
                ),
                _ => {
                    let phase = Idle;
                    if r != seq {
                        return (Fetch { phase, seq }, vec![]);
                    }
                    let list = body
                        .and_then(|b| parse_opensearch(&b).ok())
                        .unwrap_or_default();
                    (Fetch { phase, seq }, vec![Show(list)])
                }
            }
        }
        FetchEvent::Reply { .. } => (s, vec![]),
        FetchEvent::Closed => {
            let phase = in_flight(&s.phase).map_or(Idle, |req| InFlight { req });
            (
                Fetch {
                    phase,
                    seq: s.seq + 1,
                },
                vec![Clear],
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_bar_never_goes_live() {
        use LiveSuggestions::{Off, On};
        // (setting on, engine set, side, what runs)
        for (on, engine, side, want) in [
            (true, true, Side::Normal, On),
            (true, true, Side::Private, Off),
            (false, true, Side::Normal, Off),
            (true, false, Side::Normal, Off),
            (false, false, Side::Private, Off),
        ] {
            assert_eq!(
                LiveSuggestions::of(on, engine, side),
                want,
                "{on} {engine} {side:?}"
            );
        }
    }

    static DDG: std::sync::LazyLock<Engines> = std::sync::LazyLock::new(|| {
        Engines::resolve(
            "duckduckgo",
            &["Docs | d | https://d.test/?q=%s".to_owned()],
            &[],
        )
    });

    fn settings(live: LiveSuggestions) -> SuggestSettings<'static> {
        SuggestSettings {
            engines: &DDG,
            live,
        }
    }

    #[test]
    fn a_keyword_asks_its_engine_and_one_without_an_endpoint_nothing() {
        let on = settings(LiveSuggestions::On);
        assert_eq!(
            sources("bing.com rust", &on),
            [
                Source::Local,
                Source::Remote {
                    url: "https://www.bing.com/osjson.aspx?query=rust".into()
                },
            ]
        );
        assert_eq!(sources("d rust", &on), [Source::Local]);
        assert_eq!(sources("bing.com a.test/x", &on), [Source::Local]);
        // no default chosen yet: nothing is asked
        let none = Engines::default();
        let unchosen = SuggestSettings {
            engines: &none,
            live: LiveSuggestions::On,
        };
        assert_eq!(sources("rust", &unchosen), [Source::Local]);
    }

    #[test]
    fn off_by_default_and_local_only() {
        assert_eq!(LiveSuggestions::default(), LiveSuggestions::Off);
        assert_eq!(
            sources("rust slint", &settings(LiveSuggestions::Off)),
            [Source::Local]
        );
        assert!(sources("   ", &settings(LiveSuggestions::On)).is_empty());
    }

    #[test]
    fn on_adds_encoded_remote_request() {
        assert_eq!(
            sources("c++ & rust", &settings(LiveSuggestions::On)),
            [
                Source::Local,
                Source::Remote {
                    url: "https://duckduckgo.com/ac/?type=list&q=c%2B%2B%20%26%20rust".into()
                },
            ]
        );
    }

    #[test]
    fn addresses_are_never_sent() {
        for q in [
            "github.com/foo",
            "https://bank.example/login",
            "localhost:3000",
            "192.168.0",
            "http://",
            "foo/bar",
            "foo.",
            "user@host.com",
            "about:blank",
            "about:config",
            "view-source:x",
            "[::1]",
            "[::1]:8080",
        ] {
            assert_eq!(
                sources(q, &settings(LiveSuggestions::On)),
                [Source::Local],
                "{q}"
            );
        }
    }

    #[test]
    fn parses_opensearch_and_rejects_garbage() {
        assert_eq!(
            parse_opensearch(r#"["ru",["rust","ruby"]]"#),
            Ok(vec!["rust".into(), "ruby".into()])
        );
        assert_eq!(parse_opensearch(r#"{"ru":1}"#), Err(MalformedSuggestions));
        assert_eq!(
            parse_opensearch(r#"["ru",[1,2]]"#),
            Err(MalformedSuggestions)
        );
    }

    fn run(s: Fetch, events: Vec<FetchEvent>, live: LiveSuggestions) -> (Fetch, Vec<FetchEffect>) {
        let mut all = vec![];
        let mut s = s;
        for e in events {
            let (n, fx) = transition(s, e, &settings(live));
            s = n;
            all.extend(fx);
        }
        (s, all)
    }

    fn requests(fx: &[FetchEffect]) -> Vec<(u64, String)> {
        fx.iter()
            .filter_map(|e| match e {
                FetchEffect::Request { seq, url } => Some((*seq, url.clone())),
                _ => None,
            })
            .collect()
    }

    use FetchEvent::*;

    #[test]
    fn typing_is_debounced_into_one_request() {
        let (s, fx) = run(
            Fetch::default(),
            vec![
                Typed("r".into()),
                Typed("ru".into()),
                Typed("rus".into()),
                DebounceElapsed(1),
                DebounceElapsed(2),
                DebounceElapsed(3),
            ],
            LiveSuggestions::On,
        );
        assert_eq!(
            requests(&fx),
            vec![(3, "https://duckduckgo.com/ac/?type=list&q=rus".into())]
        );
        let (_, fx) = run(
            s,
            vec![Reply {
                seq: 3,
                body: Some(r#"["rus",["rust"]]"#.into()),
            }],
            LiveSuggestions::On,
        );
        assert_eq!(fx, vec![FetchEffect::Show(vec!["rust".into()])]);
    }

    #[test]
    fn a_stale_reply_is_dropped_and_the_queued_query_goes_out() {
        let (s, fx) = run(
            Fetch::default(),
            vec![
                Typed("ru".into()),
                DebounceElapsed(1),
                Typed("rust".into()),
                DebounceElapsed(2),
            ],
            LiveSuggestions::On,
        );
        assert_eq!(requests(&fx).len(), 1, "one in flight");
        let (s, fx) = run(
            s,
            vec![Reply {
                seq: 1,
                body: Some(r#"["ru",["ruby"]]"#.into()),
            }],
            LiveSuggestions::On,
        );
        assert!(!fx.iter().any(|e| matches!(e, FetchEffect::Show(_))));
        assert_eq!(requests(&fx)[0].0, 2);
        let (s, fx) = run(s, vec![Reply { seq: 2, body: None }], LiveSuggestions::On);
        assert_eq!(fx, vec![FetchEffect::Show(vec![])]);
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn address_like_input_and_off_never_request() {
        for (q, live) in [
            ("github.com/x", LiveSuggestions::On),
            ("https://bank.example", LiveSuggestions::On),
            ("rust", LiveSuggestions::Off),
        ] {
            let (_, fx) = run(
                Fetch::default(),
                vec![Typed(q.into()), DebounceElapsed(1)],
                live,
            );
            assert!(requests(&fx).is_empty(), "{q}");
            assert_eq!(fx, vec![FetchEffect::Clear]);
        }
    }

    #[test]
    fn closing_clears_and_drops_the_answer() {
        let (s, fx) = run(
            Fetch::default(),
            vec![Typed("rust".into()), DebounceElapsed(1), Closed],
            LiveSuggestions::On,
        );
        assert_eq!(fx.last(), Some(&FetchEffect::Clear));
        let (s, fx) = run(
            s,
            vec![Reply {
                seq: 1,
                body: Some(r#"["rust",["rust"]]"#.into()),
            }],
            LiveSuggestions::On,
        );
        assert!(fx.is_empty());
        assert_eq!(s.phase, Phase::Idle);
        // a debounce armed before the close does nothing
        let (_, fx) = run(
            Fetch::default(),
            vec![Typed("rust".into()), Closed, DebounceElapsed(1)],
            LiveSuggestions::On,
        );
        assert!(requests(&fx).is_empty());
        // a query queued behind an in-flight request is dropped by the close
        let (_, fx) = run(
            Fetch::default(),
            vec![
                Typed("rust".into()),
                DebounceElapsed(1),
                Typed("rust slint".into()),
                DebounceElapsed(2),
                Closed,
                Reply { seq: 1, body: None },
            ],
            LiveSuggestions::On,
        );
        assert_eq!(requests(&fx).len(), 1);
    }
}
