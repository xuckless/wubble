//! The cosmetic channel's wire format: what the helper's `cosmetic.js` asks
//! and what the engine answers. Pure text and JSON, no CEF types.
//!
//! The page side sends `hello` when a frame's script context is created and,
//! once generic hiding applies, `{"c":[class names],"i":[ids]}` batches of
//! names it has not asked about yet. The answer is `{"css":[rules],
//! "procedural":[filters],"generic":bool}`: stylesheet rules to add, procedural
//! filters (adblock's JSON) for the page's executor, and whether to keep
//! asking. The answer is data only; scriptlets travel by `run_script_in`.

/// Names per batch the engine reads; a longer batch is cut.
pub const MAX_NAMES: usize = 512;
/// Longest class name or id read; a longer one is skipped.
pub const MAX_NAME_LEN: usize = 128;

/// One request from a frame's `cosmetic.js`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// The frame's context was created: send its rules.
    Hello,
    /// Class names and ids the frame has not asked about.
    Names {
        classes: Vec<String>,
        ids: Vec<String>,
    },
}

impl Request {
    /// Reads a message body; `None` for anything else (a page may send any text).
    pub fn parse(body: &str) -> Option<Self> {
        if body == "hello" {
            return Some(Self::Hello);
        }
        let value: serde_json::Value = serde_json::from_str(body).ok()?;
        let names = |key: &str| -> Option<Vec<String>> {
            let list = value.get(key)?.as_array()?;
            Some(
                list.iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| !s.is_empty() && s.len() <= MAX_NAME_LEN)
                    .take(MAX_NAMES)
                    .map(str::to_owned)
                    .collect(),
            )
        };
        Some(Self::Names {
            classes: names("c").unwrap_or_default(),
            ids: names("i").unwrap_or_default(),
        })
        .filter(
            |r| !matches!(r, Self::Names { classes, ids } if classes.is_empty() && ids.is_empty()),
        )
    }
}

/// The answer to a [`Request`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    /// Stylesheet rules to add.
    pub css: Vec<String>,
    /// Procedural filters for the page's executor (adblock's JSON).
    pub procedural: Vec<String>,
    /// The page should keep asking about new class names and ids.
    pub generic: bool,
}

impl Reply {
    pub fn to_text(&self) -> String {
        serde_json::json!({
            "css": self.css,
            "procedural": self.procedural,
            "generic": self.generic,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(classes: &[&str], ids: &[&str]) -> Request {
        Request::Names {
            classes: classes.iter().map(|s| (*s).into()).collect(),
            ids: ids.iter().map(|s| (*s).into()).collect(),
        }
    }

    #[test]
    fn requests_parse() {
        assert_eq!(Request::parse("hello"), Some(Request::Hello));
        assert_eq!(
            Request::parse(r#"{"c":["a","b"],"i":["x"]}"#),
            Some(names(&["a", "b"], &["x"]))
        );
        assert_eq!(Request::parse(r#"{"c":["a"]}"#), Some(names(&["a"], &[])));
    }

    #[test]
    fn junk_is_ignored() {
        for body in [
            "",
            "garbage",
            "[]",
            "{}",
            r#"{"c":"a"}"#,
            r#"{"c":[1,null]}"#,
            r#"{"c":[],"i":[]}"#,
        ] {
            assert_eq!(Request::parse(body), None, "{body}");
        }
    }

    #[test]
    fn batches_and_names_are_capped() {
        let huge = "x".repeat(MAX_NAME_LEN + 1);
        let many: Vec<String> = (0..MAX_NAMES + 10).map(|i| format!("c{i}")).collect();
        let body = serde_json::json!({"c": many, "i": [huge, "ok", ""]}).to_string();
        let Some(Request::Names { classes, ids }) = Request::parse(&body) else {
            panic!("names")
        };
        assert_eq!(classes.len(), MAX_NAMES);
        assert_eq!(ids, ["ok"]);
    }

    #[test]
    fn replies_round_trip_through_json() {
        let reply = Reply {
            css: vec![".a{display:none!important}".into()],
            procedural: vec![r#"{"selector":[]}"#.into()],
            generic: true,
        };
        let value: serde_json::Value = serde_json::from_str(&reply.to_text()).unwrap();
        assert_eq!(value["css"][0], ".a{display:none!important}");
        assert_eq!(value["procedural"][0], r#"{"selector":[]}"#);
        assert_eq!(value["generic"], true);
    }
}
