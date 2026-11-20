//! The scripts the built-ins run in the page's main frame (`View::eval`), each one JavaScript
//! expression: a function that declares what it needs and returns plain JSON. Only the bundled
//! sources and JSON-encoded data go into them, never page text.
//!
//! - Reader mode: Mozilla's Readability 0.6.0 (`assets/reader/`, Apache-2.0, vendored
//!   unmodified) and wubble's `assets/reader.js`.
//! - Link hints: `assets/hints.js` (detection ported from Vimium, MIT).

use serde_json::{Value, json};

const READERABLE: &str = include_str!("../assets/reader/Readability-readerable.js");
const READABILITY: &str = include_str!("../assets/reader/Readability.js");
const READER: &str = include_str!("../assets/reader.js");
const HINTS: &str = include_str!("../assets/hints.js");

/// The page's content type (`"application/pdf"` in Chromium's PDF viewer).
pub const CONTENT_TYPE: &str = "document.contentType";
/// [`CONTENT_TYPE`]'s answer, as JSON, for a PDF.
pub const PDF_TYPE: &str = "\"application/pdf\"";

/// `parts` declared in one function scope, then `call`'s value.
fn wrap(parts: &[&str], call: &str) -> String {
    let mut s = String::from("(function(){\n");
    for part in parts {
        s.push_str(part);
        s.push('\n');
    }
    s.push_str("return ");
    s.push_str(call);
    s.push_str(";\n})()");
    s
}

/// Whether the page reads as an article (`isProbablyReaderable`): `true` or `false`.
pub fn reader_probe() -> String {
    wrap(&[READERABLE, READER], "wubbleReader.probe()")
}

/// The CSS custom properties as the scripts' JSON object.
fn vars_json(vars: &[(&str, String)]) -> Value {
    Value::Object(
        vars.iter()
            .map(|(k, v)| ((*k).to_owned(), Value::String(v.clone())))
            .collect(),
    )
}

/// Builds the reader in place: `{title, length, x, y}` (`x`, `y`: where the page was), or
/// `null` when there is no article.
pub fn reader_open(vars: &[(&str, String)]) -> String {
    let call = format!("wubbleReader.open({})", vars_json(vars));
    wrap(&[READERABLE, READABILITY, READER], &call)
}

/// The page's readable text for an agent (`read_text`): `{article, title, byline, text}`,
/// Readability's article (on a copy: the page is not changed) or, with `article` false, the
/// page's whole text.
pub fn reader_text() -> String {
    wrap(&[READABILITY, READER], "wubbleReader.text()")
}

/// Leaving the reader: Chromium's scroll restore off for the reload that follows (it would take
/// the reader's offset); `true`.
pub fn reader_leave() -> String {
    wrap(&[READER], "wubbleReader.leave()")
}

/// The page left for the reader, loaded again: back to where it was, `x`, `y` CSS px; `true`.
pub fn reader_restore(x: i32, y: i32) -> String {
    wrap(&[READER], &format!("wubbleReader.restore({x}, {y})"))
}

/// Where the page was when the reader opened, from [`reader_open`]'s answer: `None` when
/// there was no article.
pub fn reader_opened(answer: &str) -> Option<(i32, i32)> {
    let v: Value = serde_json::from_str(answer).ok()?;
    let at = |k: &str| {
        v.get(k)
            .and_then(Value::as_f64)
            .map_or(0, |n| n.round() as i32)
    };
    v.is_object().then(|| (at("x"), at("y")))
}

/// New typography for the reader on screen: `true`.
pub fn reader_set(vars: &[(&str, String)]) -> String {
    wrap(&[READER], &format!("wubbleReader.set({})", vars_json(vars)))
}

/// The clickable targets on screen: `[{x, y, w, h, href}]`, viewport CSS px, in document order.
pub fn hints_collect() -> String {
    wrap(&[HINTS], "wubbleHints.collect()")
}

/// The labels' look: wubble's tokens, as CSS values.
pub struct HintColors {
    pub label: String,
    pub ink: String,
    pub font: String,
}

/// Draws `items` (viewport CSS px, label), the first `typed` letters marked as typed.
pub fn hints_draw(items: &[((f64, f64), String)], typed: &str, colors: &HintColors) -> String {
    let items: Vec<Value> = items
        .iter()
        .map(|((x, y), label)| json!({ "x": x, "y": y, "label": label }))
        .collect();
    let c = json!({ "label": colors.label, "ink": colors.ink, "font": colors.font });
    let call = format!(
        "wubbleHints.draw({}, {}, {c})",
        Value::Array(items),
        Value::String(typed.to_owned())
    );
    wrap(&[HINTS], &call)
}

pub fn hints_clear() -> String {
    wrap(&[HINTS], "wubbleHints.clear()")
}

/// `#rrggbb` of a Slint colour, for a page's CSS.
pub fn hex(c: slint::Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_sources_are_whole() {
        assert!(READABILITY.contains("function Readability(doc, options)"));
        assert!(READABILITY.contains("module.exports = Readability;"));
        assert!(READERABLE.contains("function isProbablyReaderable(doc, options = {})"));
        let licence = include_str!("../assets/reader/LICENSE.md");
        assert!(licence.contains("Apache License, Version 2.0"));
        for entry in [
            "probe:",
            "open:",
            "set:",
            "leave:",
            "restore:",
            "text:",
            "var wubbleReader",
        ] {
            assert!(READER.contains(entry), "reader.js lacks {entry}");
        }
        for entry in ["collect:", "draw:", "clear:", "var wubbleHints"] {
            assert!(HINTS.contains(entry), "hints.js lacks {entry}");
        }
    }

    #[test]
    fn scripts_are_one_expression_with_their_data_as_json() {
        let open = reader_open(&[("--wb-size", "18px".into())]);
        assert!(open.starts_with("(function(){") && open.ends_with("})()"));
        assert!(open.contains(r#"wubbleReader.open({"--wb-size":"18px"})"#));
        assert!(reader_probe().contains("isProbablyReaderable(doc"));
        assert!(!reader_set(&[]).contains("function Readability("));
        let colors = HintColors {
            label: "#ffd98a".into(),
            ink: "#0e1016".into(),
            font: "monospace".into(),
        };
        // A label cannot break out of its string.
        let draw = hints_draw(&[((1.0, 2.0), "a\"b".into())], "a", &colors);
        assert!(draw.contains(r#"[{"label":"a\"b","x":1.0,"y":2.0}], "a", {"#));
        assert_eq!(hex(slint::Color::from_rgb_u8(255, 217, 138)), "#ffd98a");
    }

    #[test]
    fn the_reader_answers_where_the_page_was() {
        let answer = r#"{"title":"t","length":900,"x":0,"y":1834.5}"#;
        assert_eq!(reader_opened(answer), Some((0, 1835)));
        assert_eq!(reader_opened(r#"{"title":"t"}"#), Some((0, 0)));
        assert_eq!(reader_opened("null"), None);
        assert_eq!(reader_opened("nope"), None);
        assert!(reader_restore(3, 1200).contains("wubbleReader.restore(3, 1200)"));
        assert!(reader_leave().contains("wubbleReader.leave()"));
    }

    #[test]
    fn notices_name_the_copied_code() {
        let notices = include_str!("../../../THIRD_PARTY_NOTICES.md");
        for name in ["Readability", "Vimium", "puppeteer"] {
            assert!(
                notices.contains(name),
                "{name} missing from THIRD_PARTY_NOTICES.md"
            );
        }
    }
}
