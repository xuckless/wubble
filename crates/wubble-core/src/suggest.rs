//! Command-bar suggestions: which sources to consult for a query.
//!
//! Pure planning only. This module decides *whether* a remote request may be
//! made and builds its URL; the fetch itself is an effect run at the edge.
//! Live suggestions send keystrokes to the search engine, so they are off
//! unless the user turns them on.

pub use crate::omnibox::SearchEngine;
use crate::omnibox::{Intent, classify, suggest_url};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveSuggestions {
    /// Nothing leaves the machine until the user presses Enter.
    #[default]
    Off,
    On,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuggestSettings {
    pub engine: SearchEngine,
    pub live: LiveSuggestions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Open tabs, history, bookmarks.
    Local,
    /// Fetch this URL and feed the body to [`parse_opensearch`].
    Remote { url: String },
}

/// Sources to consult for `query`, in display order.
pub fn sources(query: &str, settings: &SuggestSettings) -> Vec<Source> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = vec![Source::Local];
    if settings.live == LiveSuggestions::On && matches!(classify(query), Intent::Search(_)) {
        out.push(Source::Remote {
            url: suggest_url(settings.engine, query),
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(live: LiveSuggestions) -> SuggestSettings {
        SuggestSettings {
            engine: SearchEngine::DuckDuckGo,
            live,
        }
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
}
