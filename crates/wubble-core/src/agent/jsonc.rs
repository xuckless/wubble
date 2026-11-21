//! A member of a JSON or JSONC object added or removed in place, the rest of the text kept byte
//! for byte: comments, key order, unknown keys, indentation and line ends. Other programs' MCP
//! configuration files (`agent::harness`) are edited with it, so it never re-serialises a file.
//!
//! The scanner reads JSON with comments (`//`, `/* */`) and trailing commas, which is what the
//! JSONC files these programs keep allow; anything else is a [`ParseError`] and nothing is
//! written.

use serde_json::Value;

/// Why a file was left alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset where reading stopped.
    pub at: usize,
    pub what: &'static str,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.what, self.at)
    }
}

/// One member of an object, as spans of the text.
#[derive(Clone, Debug)]
struct Member {
    key: String,
    /// Where the key's opening quote is.
    start: usize,
    /// Just past the value.
    end: usize,
    value: Node,
}

#[derive(Clone, Debug)]
enum Node {
    /// `open` is the `{`, `close` the `}`.
    Object {
        open: usize,
        close: usize,
        members: Vec<Member>,
    },
    Other,
}

struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
}

impl Scanner<'_> {
    fn fail<T>(&self, what: &'static str) -> Result<T, ParseError> {
        Err(ParseError { at: self.i, what })
    }

    /// Past whitespace and comments.
    fn skip(&mut self) -> Result<(), ParseError> {
        loop {
            match self.s.get(self.i) {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.i += 1,
                Some(b'/') => match self.s.get(self.i + 1) {
                    Some(b'/') => {
                        while self.s.get(self.i).is_some_and(|c| *c != b'\n') {
                            self.i += 1;
                        }
                    }
                    Some(b'*') => {
                        self.i += 2;
                        loop {
                            match self.s.get(self.i) {
                                None => return self.fail("an unclosed comment"),
                                Some(b'*') if self.s.get(self.i + 1) == Some(&b'/') => {
                                    self.i += 2;
                                    break;
                                }
                                Some(_) => self.i += 1,
                            }
                        }
                    }
                    _ => return self.fail("a stray /"),
                },
                _ => return Ok(()),
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        self.i += 1;
        loop {
            match self.s.get(self.i) {
                None => return self.fail("an unclosed string"),
                Some(b'\\') => self.i += 2,
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(_) => self.i += 1,
            }
        }
        let raw = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| ParseError {
            at: start,
            what: "a string that is not UTF-8",
        })?;
        serde_json::from_str::<String>(raw).map_err(|_| ParseError {
            at: start,
            what: "a bad string",
        })
    }

    fn value(&mut self) -> Result<Node, ParseError> {
        self.skip()?;
        match self.s.get(self.i) {
            Some(b'{') => self.object(),
            Some(b'[') => {
                self.i += 1;
                loop {
                    self.skip()?;
                    match self.s.get(self.i) {
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Node::Other);
                        }
                        Some(b',') => self.i += 1,
                        None => return self.fail("an unclosed array"),
                        Some(_) => {
                            self.value()?;
                        }
                    }
                }
            }
            Some(b'"') => self.string().map(|_| Node::Other),
            Some(c) if c.is_ascii_alphanumeric() || *c == b'-' => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || b"+-.".contains(c))
                {
                    self.i += 1;
                }
                let word = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                if matches!(word, "true" | "false" | "null") || word.parse::<f64>().is_ok() {
                    Ok(Node::Other)
                } else {
                    Err(ParseError {
                        at: start,
                        what: "a bad value",
                    })
                }
            }
            None => self.fail("the end of the file where a value was due"),
            Some(_) => self.fail("an unexpected character"),
        }
    }

    fn object(&mut self) -> Result<Node, ParseError> {
        let open = self.i;
        self.i += 1;
        let mut members = vec![];
        loop {
            self.skip()?;
            match self.s.get(self.i) {
                Some(b'}') => {
                    let close = self.i;
                    self.i += 1;
                    return Ok(Node::Object {
                        open,
                        close,
                        members,
                    });
                }
                Some(b',') if !members.is_empty() => self.i += 1,
                Some(b'"') => {
                    let start = self.i;
                    let key = self.string()?;
                    self.skip()?;
                    if self.s.get(self.i) != Some(&b':') {
                        return self.fail("a key with no colon");
                    }
                    self.i += 1;
                    let value = self.value()?;
                    members.push(Member {
                        key,
                        start,
                        end: self.i,
                        value,
                    });
                    self.skip()?;
                    match self.s.get(self.i) {
                        Some(b',' | b'}') => {}
                        _ => return self.fail("members with no comma between them"),
                    }
                }
                None => return self.fail("an unclosed object"),
                Some(_) => return self.fail("an unexpected character in an object"),
            }
        }
    }
}

/// The whole text as a tree of spans; the top level must be an object.
fn parse(text: &str) -> Result<Node, ParseError> {
    let mut sc = Scanner {
        s: text.as_bytes(),
        i: 0,
    };
    sc.skip()?;
    if sc.s.get(sc.i) != Some(&b'{') {
        return sc.fail("a file that is not an object");
    }
    let node = sc.object()?;
    sc.skip()?;
    if sc.i != sc.s.len() {
        return sc.fail("text after the object");
    }
    Ok(node)
}

/// Whether the text parses as JSONC with an object at the top (an empty file counts as `{}`).
pub fn check(text: &str) -> Result<(), ParseError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    parse(text).map(|_| ())
}

fn members_of(node: &Node) -> Option<&Vec<Member>> {
    match node {
        Node::Object { members, .. } => Some(members),
        Node::Other => None,
    }
}

/// The object at `path` (keys from the top), if every step is an object.
fn find<'a>(root: &'a Node, path: &[&str]) -> Option<&'a Node> {
    let mut at = root;
    for key in path {
        at = &members_of(at)?.iter().rev().find(|m| m.key == *key)?.value;
    }
    Some(at)
}

/// Whether the object at `path` has a member `name`.
pub fn has(text: &str, path: &[&str], name: &str) -> Result<bool, ParseError> {
    if text.trim().is_empty() {
        return Ok(false);
    }
    let root = parse(text)?;
    Ok(find(&root, path)
        .and_then(members_of)
        .is_some_and(|m| m.iter().any(|m| m.key == name)))
}

/// The value of member `name` of the object at `path`, as JSON, if it is plain JSON.
pub fn get(text: &str, path: &[&str], name: &str) -> Result<Option<Value>, ParseError> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let root = parse(text)?;
    let member = find(&root, path)
        .and_then(members_of)
        .and_then(|m| m.iter().rev().find(|m| m.key == name).cloned());
    Ok(member.and_then(|m| {
        let raw = &text[m.start..m.end];
        let value = raw.split_once(':').map(|(_, v)| v)?;
        serde_json::from_str(value.trim()).ok()
    }))
}

/// The indentation of the line `at` is on.
fn indent_at(text: &str, at: usize) -> &str {
    let line = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let rest = &text[line..];
    &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
}

/// One indentation step as the file writes it (two spaces if it shows none).
fn step_of(text: &str) -> String {
    text.lines()
        .map(|l| &l[..l.len() - l.trim_start_matches([' ', '\t']).len()])
        .filter(|i| !i.is_empty())
        .min_by_key(|i| i.len())
        .unwrap_or("  ")
        .to_owned()
}

/// `value` as pretty JSON whose lines after the first start with `indent`.
fn pretty(value: &Value, indent: &str, step: &str) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    text.lines()
        .enumerate()
        .map(|(i, line)| {
            let inner = line.trim_start_matches(' ');
            let depth = (line.len() - inner.len()) / 2;
            if i == 0 {
                inner.to_owned()
            } else {
                format!("{indent}{}{inner}", step.repeat(depth))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text with member `name: value` in the object at `path`, made where missing; unchanged
/// when the object already has a member of that name (whatever its value: the file's owner may
/// have edited it).
pub fn add(text: &str, path: &[&str], name: &str, value: &Value) -> Result<String, ParseError> {
    if text.trim().is_empty() {
        // build {path..: {name: value}} from the inside out
        let mut v = serde_json::json!({ name: value.clone() });
        for key in path.iter().rev() {
            v = serde_json::json!({ *key: v });
        }
        return Ok(format!("{}\n", pretty(&v, "", "  ")));
    }
    let root = parse(text)?;
    let step = step_of(text);
    // the deepest object of `path` that exists, and what is left to make
    let mut at = &root;
    let mut made = 0;
    for key in path {
        match members_of(at).and_then(|m| m.iter().rev().find(|m| m.key == *key)) {
            Some(m) if matches!(m.value, Node::Object { .. }) => {
                at = &m.value;
                made += 1;
            }
            Some(_) => {
                return Err(ParseError {
                    at: 0,
                    what: "a key on the way that is not an object",
                });
            }
            None => break,
        }
    }
    let Node::Object {
        open,
        close,
        members,
    } = at
    else {
        return Err(ParseError {
            at: 0,
            what: "no object",
        });
    };
    if made == path.len() && members.iter().any(|m| m.key == name) {
        return Ok(text.to_owned());
    }
    // the member to write: the rest of the path around `name: value`
    let (mut key, mut v) = (name, value.clone());
    for k in path[made..].iter().rev() {
        v = serde_json::json!({ key: v });
        key = k;
    }
    let parent_indent = indent_at(text, *open).to_owned();
    match members.last() {
        Some(last) => {
            let indent = indent_at(text, last.start).to_owned();
            let member = format!(
                "{}: {}",
                serde_json::to_string(key).unwrap_or_default(),
                pretty(&v, &indent, &step)
            );
            // after the last member (and its trailing comma, if the file has one), and after a
            // line comment that ends its line: the comment stays with its member
            let after = &text[last.end..*close];
            let comma = after.trim_start().starts_with(',');
            let past = if comma {
                last.end + after.find(',').unwrap_or(0) + 1
            } else {
                last.end
            };
            let line_end = text[past..]
                .find('\n')
                .map_or(*close, |n| past + n)
                .min(*close);
            let rest = text[past..line_end].trim();
            let at_end = if rest.is_empty() || rest.starts_with("//") {
                line_end
            } else {
                past
            };
            let (before, between) = if comma {
                (String::new(), format!("\n{indent}{member},"))
            } else {
                (",".to_owned(), format!("\n{indent}{member}"))
            };
            Ok(format!(
                "{}{before}{}{between}{}",
                &text[..last.end],
                &text[last.end..at_end],
                &text[at_end..]
            ))
        }
        None => {
            let indent = format!("{parent_indent}{step}");
            let member = format!(
                "{}: {}",
                serde_json::to_string(key).unwrap_or_default(),
                pretty(&v, &indent, &step)
            );
            let inside = &text[open + 1..*close];
            Ok(format!(
                "{}\n{indent}{member}\n{parent_indent}{}{}",
                &text[..open + 1],
                if inside.trim().is_empty() {
                    ""
                } else {
                    inside.trim()
                },
                &text[*close..]
            ))
        }
    }
}

/// The text without member `name` of the object at `path`; unchanged when it has none.
pub fn remove(text: &str, path: &[&str], name: &str) -> Result<String, ParseError> {
    if text.trim().is_empty() {
        return Ok(text.to_owned());
    }
    let root = parse(text)?;
    let Some(Node::Object {
        members,
        open,
        close,
    }) = find(&root, path)
    else {
        return Ok(text.to_owned());
    };
    let Some(i) = members.iter().position(|m| m.key == name) else {
        return Ok(text.to_owned());
    };
    let m = &members[i];
    let line_start = |at: usize| text[..at].rfind('\n').unwrap_or(at);
    let (from, to) = if let Some(next) = members.get(i + 1) {
        // up to the next member, from the end of the line before this one
        (line_start(m.start), line_start(next.start))
    } else if let Some(prev) = i.checked_sub(1).map(|p| &members[p]) {
        // from the previous member's end (its comma goes), through a trailing comma of ours
        let after = &text[m.end..];
        let trailing = after.trim_start().starts_with(',');
        let to = if trailing {
            m.end + after.find(',').unwrap_or(0) + 1
        } else {
            m.end
        };
        // keep a trailing comma style: if ours had one, the previous member's comma stays
        let from = if trailing {
            line_start(m.start)
        } else {
            prev.end
        };
        (from, to)
    } else {
        // the only member: the object becomes empty
        let after = &text[m.end..];
        let to = if after.trim_start().starts_with(',') {
            m.end + after.find(',').unwrap_or(0) + 1
        } else {
            m.end
        };
        let from = line_start(m.start);
        // nothing else inside: back to `{}`
        if (text[open + 1..from].to_owned() + &text[to..*close])
            .trim()
            .is_empty()
        {
            return Ok(format!("{}{}", &text[..=*open], &text[*close..]));
        }
        (from, to)
    };
    Ok(format!("{}{}", &text[..from], &text[to..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PATH: &[&str] = &["mcpServers"];

    fn entry() -> Value {
        json!({"type": "stdio", "command": "/opt/wubble/wubble", "args": ["mcp"]})
    }

    #[test]
    fn added_beside_the_others_and_removed_back_to_the_same_bytes() {
        let file = "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"git\": {\n      \"command\": \"git-mcp\"\n    }\n  },\n  \"tips\": 3\n}\n";
        let added = add(file, PATH, "wubble", &entry()).unwrap();
        let v: Value = serde_json::from_str(&added).unwrap();
        assert_eq!(v["mcpServers"]["wubble"], entry());
        assert_eq!(v["mcpServers"]["git"]["command"], "git-mcp");
        assert_eq!(v["tips"], 3);
        assert!(
            added.contains("\n    \"wubble\": {\n      \"args\": ["),
            "{added}"
        );
        assert_eq!(
            add(&added, PATH, "wubble", &entry()).unwrap(),
            added,
            "twice: no-op"
        );
        assert_eq!(remove(&added, PATH, "wubble").unwrap(), file);
        assert_eq!(
            remove(file, PATH, "wubble").unwrap(),
            file,
            "none: unchanged"
        );
    }

    #[test]
    fn comments_trailing_commas_and_unknown_keys_survive() {
        let file = "// my settings\n{\n\t\"context_servers\": {\n\t\t/* mine */\n\t\t\"a\": {\"command\": \"a\"}, // a\n\t},\n\t\"vim_mode\": true, // yes\n}\n";
        let added = add(file, &["context_servers"], "wubble", &entry()).unwrap();
        assert!(added.starts_with("// my settings\n{\n\t\"context_servers\": {\n\t\t/* mine */\n\t\t\"a\": {\"command\": \"a\"},"));
        assert!(added.contains("\"vim_mode\": true, // yes"));
        assert!(
            added.contains(
                "\t\t\"a\": {\"command\": \"a\"}, // a\n\t\t\"wubble\": {\n\t\t\t\"args\": ["
            ),
            "{added}"
        );
        assert!(has(&added, &["context_servers"], "wubble").unwrap());
        check(&added).unwrap();
        assert_eq!(
            remove(&added, &["context_servers"], "wubble").unwrap(),
            file
        );
    }

    #[test]
    fn a_missing_object_is_made_and_an_empty_file_written_whole() {
        let file = "{\n  \"theme\": \"dark\"\n}\n";
        let added = add(file, &["mcp", "servers"], "wubble", &entry()).unwrap();
        let v: Value = serde_json::from_str(&added).unwrap();
        assert_eq!(v["mcp"]["servers"]["wubble"], entry());
        assert_eq!(v["theme"], "dark");
        let empty = add("", PATH, "wubble", &entry()).unwrap();
        let v: Value = serde_json::from_str(&empty).unwrap();
        assert_eq!(v["mcpServers"]["wubble"], entry());
        let into_empty = add("{}", PATH, "wubble", &entry()).unwrap();
        let v: Value = serde_json::from_str(&into_empty).unwrap();
        assert_eq!(v["mcpServers"]["wubble"], entry());
        // the only member out: an empty object, the rest as it was
        let only = "{\n  \"mcpServers\": {\n    \"wubble\": {}\n  }\n}\n";
        assert_eq!(
            remove(only, PATH, "wubble").unwrap(),
            "{\n  \"mcpServers\": {}\n}\n"
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_refused() {
        for bad in [
            "{",
            "{\"a\": }",
            "[1, 2]",
            "{\"a\": 1} x",
            "{\"a\" 1}",
            "{/* open",
        ] {
            assert!(add(bad, PATH, "wubble", &entry()).is_err(), "{bad}");
            assert!(remove(bad, PATH, "wubble").is_err(), "{bad}");
            assert!(check(bad).is_err(), "{bad}");
        }
        // a key on the way that is not an object
        assert!(add("{\"mcpServers\": 3}", PATH, "wubble", &entry()).is_err());
    }

    #[test]
    fn a_member_is_read_back() {
        let file = "{\"mcpServers\": {\"wubble\": {\"command\": \"x\"} /* c */}}";
        assert_eq!(
            get(file, PATH, "wubble").unwrap(),
            Some(json!({"command": "x"}))
        );
        assert_eq!(get(file, PATH, "other").unwrap(), None);
    }
}
