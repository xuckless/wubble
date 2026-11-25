//! The cosmetic channel's wire format: what the helper's `cosmetic.js` asks
//! and what the engine answers. Pure text and JSON, no CEF types.
//!
//! The page side sends `hello` when a frame's script context is created and,
//! once generic hiding applies, `{"c":[class names],"i":[ids]}` batches of
//! names it has not asked about yet. The answer is `{"css":[rules],
//! "procedural":[filters],"generic":bool}`: stylesheet rules to add, procedural
//! filters (adblock's JSON) for the page's executor, and whether to keep
//! asking. The answer is data only; scriptlets travel by `run_script_in`.
//!
//! Each pointer press also reports what it landed on, `{"click":{...}}`
//! (`wubble_core::click_guard::Click`); the engine may answer [`unclick_text`]
//! to have the page remove the invisible overlay it landed on.

use wubble_core::click_guard::{self, Click};

use crate::filter::FrameCosmetic;

/// Longest link a click report keeps; a longer one is dropped.
pub const MAX_HREF_LEN: usize = 8192;

/// Names per batch the engine reads; a longer batch is cut.
pub const MAX_NAMES: usize = 512;
/// Longest class name or id read; a longer one is skipped.
pub const MAX_NAME_LEN: usize = 128;

/// One request from a frame's `cosmetic.js`.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// The frame's context was created: send its rules.
    Hello,
    /// The pointer went down on this.
    Click(Click),
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
        if let Some(c) = value.get("click") {
            let share = |key: &str| c.get(key)?.as_f64().map(|v| v.clamp(0.0, 1.0) as f32);
            let flag = |key: &str| c.get(key).and_then(serde_json::Value::as_bool);
            return Some(Self::Click(Click {
                opacity: share("opacity")?,
                cover: share("cover")?,
                painted: flag("painted")?,
                control: flag("control")?,
                href: c
                    .get("href")
                    .and_then(serde_json::Value::as_str)
                    .filter(|h| !h.is_empty() && h.len() <= MAX_HREF_LEN)
                    .map(str::to_owned),
                download: flag("download").unwrap_or(false),
            }));
        }
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

/// The message that has the frame remove the invisible overlay its last press landed on, with
/// [`Click::unseen`]'s numbers (the page measures again before removing anything).
pub fn unclick_text() -> String {
    serde_json::json!({
        "unclick": {
            "opacity": f64::from(click_guard::UNSEEN_OPACITY),
            "cover": f64::from(click_guard::OVERLAY_COVER),
        }
    })
    .to_string()
}

/// The answer to a [`Request`]: the frame's rules as JSON, scriptlets left out
/// (they run from the browser, not through the page).
pub fn reply_text(rules: &FrameCosmetic) -> String {
    serde_json::json!({
        "css": rules.css,
        "procedural": rules.procedural,
        "generic": rules.generic,
    })
    .to_string()
}

/// `script` (a frame's scriptlets) as a document-start script for the page at `page_url`: it
/// runs only in a document of that page's host, since every frame of the view runs it and
/// scriptlets are per host. `None` for a page without a host.
pub fn document_start(script: &str, page_url: &str) -> Option<String> {
    // `host_key` reads a bare host when there is no scheme separator
    let host = page_url
        .contains("://")
        .then(|| wubble_core::routing::host_key(page_url))
        .flatten()?;
    let host = serde_json::Value::String(host).to_string();
    Some(format!(
        "if(location.hostname.toLowerCase().replace(/\\.$/,'')==={host}){{\n{script}}}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_start_scripts_run_on_their_host_only() {
        assert_eq!(
            document_start("x();", "https://News.Test/a?b").as_deref(),
            Some(
                "if(location.hostname.toLowerCase().replace(/\\.$/,'')===\"news.test\"){\nx();}\n"
            )
        );
        assert_eq!(document_start("x();", "about:blank"), None);
        assert_eq!(document_start("x();", "data:text/html,a"), None);
        assert_eq!(document_start("x();", "file:///tmp/a.html"), None);
    }

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
    fn clicks_parse() {
        let body = r#"{"click":{"opacity":0.5,"cover":1.2,"painted":false,"control":true,"href":"https://a.test/","download":true}}"#;
        assert_eq!(
            Request::parse(body),
            Some(Request::Click(Click {
                opacity: 0.5,
                cover: 1.0,
                painted: false,
                control: true,
                href: Some("https://a.test/".into()),
                download: true,
            }))
        );
        let plain =
            r#"{"click":{"opacity":1,"cover":0,"painted":true,"control":false,"href":null}}"#;
        assert!(matches!(
            Request::parse(plain),
            Some(Request::Click(Click {
                href: None,
                download: false,
                ..
            }))
        ));
        let long = format!(
            r#"{{"click":{{"opacity":1,"cover":0,"painted":true,"control":true,"href":"{}"}}}}"#,
            "x".repeat(MAX_HREF_LEN + 1)
        );
        assert!(matches!(
            Request::parse(&long),
            Some(Request::Click(Click { href: None, .. }))
        ));
        // a click missing a measure is no click
        assert_eq!(
            Request::parse(r#"{"click":{"opacity":1,"painted":true,"control":true}}"#),
            None
        );
        assert_eq!(Request::parse(r#"{"click":"x"}"#), None);
    }

    #[test]
    fn unclick_carries_the_guard_s_numbers() {
        let v: serde_json::Value = serde_json::from_str(&unclick_text()).unwrap();
        assert_eq!(
            v["unclick"]["opacity"],
            f64::from(click_guard::UNSEEN_OPACITY)
        );
        assert_eq!(v["unclick"]["cover"], f64::from(click_guard::OVERLAY_COVER));
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
        let reply = FrameCosmetic {
            css: vec![".a{display:none!important}".into()],
            procedural: vec![r#"{"selector":[]}"#.into()],
            script: Some("never sent".into()),
            generic: true,
        };
        let value: serde_json::Value = serde_json::from_str(&reply_text(&reply)).unwrap();
        assert!(value.get("script").is_none());
        assert_eq!(value["css"][0], ".a{display:none!important}");
        assert_eq!(value["procedural"][0], r#"{"selector":[]}"#);
        assert_eq!(value["generic"], true);
    }
}
