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
    /// Where the value starts (past the colon and any blanks).
    value_start: usize,
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
                    self.skip()?;
                    let value_start = self.i;
                    let value = self.value()?;
                    members.push(Member {
                        key,
                        start,
                        value_start,
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

const BOM: &str = "\u{feff}";

/// A leading byte-order mark, kept as it is, and the text after it.
fn split_bom(text: &str) -> (&str, &str) {
    match text.strip_prefix(BOM) {
        Some(body) => (BOM, body),
        None => ("", text),
    }
}

/// The line end the file uses: CRLF if it has one, else LF.
fn line_end_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Where the next token is at or after `at` (past blanks and comments).
fn next_token(text: &str, at: usize) -> Result<usize, ParseError> {
    let mut sc = Scanner {
        s: text.as_bytes(),
        i: at,
    };
    sc.skip()?;
    Ok(sc.i)
}

/// Whether the text parses as JSONC with an object at the top (an empty file counts as `{}`).
pub fn check(text: &str) -> Result<(), ParseError> {
    let (_, body) = split_bom(text);
    if body.trim().is_empty() {
        return Ok(());
    }
    parse(body).map(|_| ())
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

/// The index of member `name` in `members`; two of that name are refused (which one a program
/// reads is its own choice).
fn index_of(members: &[Member], name: &str) -> Result<Option<usize>, ParseError> {
    let mut found = members.iter().enumerate().filter(|(_, m)| m.key == name);
    let first = found.next();
    match (first, found.next()) {
        (Some(_), Some((_, m))) => Err(ParseError {
            at: m.start,
            what: "a second member of the same name",
        }),
        (first, _) => Ok(first.map(|(i, _)| i)),
    }
}

/// Whether the object at `path` has a member `name`.
pub fn has(text: &str, path: &[&str], name: &str) -> Result<bool, ParseError> {
    get_raw(text, path, name).map(|raw| raw.is_some())
}

/// Member `name`'s value text, if the object at `path` has it.
fn get_raw<'a>(text: &'a str, path: &[&str], name: &str) -> Result<Option<&'a str>, ParseError> {
    let (_, body) = split_bom(text);
    if body.trim().is_empty() {
        return Ok(None);
    }
    let root = parse(body)?;
    let Some(members) = find(&root, path).and_then(members_of) else {
        return Ok(None);
    };
    Ok(index_of(members, name)?
        .and_then(|i| members.get(i))
        .and_then(|m| body.get(m.value_start..m.end)))
}

/// The value of member `name` of the object at `path`, as JSON (comments in it dropped).
pub fn get(text: &str, path: &[&str], name: &str) -> Result<Option<Value>, ParseError> {
    match get_raw(text, path, name)? {
        None => Ok(None),
        Some(raw) => serde_json::from_str(&strip_comments(raw))
            .map(Some)
            .map_err(|_| ParseError {
                at: 0,
                what: "a value that is not JSON",
            }),
    }
}

/// JSONC as JSON: comments blanked, trailing commas dropped; strings kept as they are.
fn strip_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'"' => {
                out.push(c);
                i += 1;
                while let Some(&d) = b.get(i) {
                    out.push(d);
                    i += 1;
                    if d == b'\\' {
                        if let Some(&e) = b.get(i) {
                            out.push(e);
                            i += 1;
                        }
                    } else if d == b'"' {
                        break;
                    }
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while b.get(i).is_some_and(|d| *d != b'\n') {
                    out.push(b' ');
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                out.extend_from_slice(b"  ");
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
                out.extend_from_slice(b"  ");
                i += 2;
            }
            b',' => {
                // a trailing comma: the next token closes
                let next = next_token(text, i + 1).unwrap_or(b.len());
                out.push(if matches!(b.get(next), Some(b'}' | b']')) {
                    b' '
                } else {
                    b','
                });
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The whole file as JSON (an empty one as `{}`).
fn to_value(text: &str) -> Result<Value, ParseError> {
    let (_, body) = split_bom(text);
    if body.trim().is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    parse(body)?;
    serde_json::from_str(&strip_comments(body)).map_err(|_| ParseError {
        at: 0,
        what: "a file that is not JSON once its comments are gone",
    })
}

/// `v` without member `name` at `path`, and without the objects of `path` that are then empty.
fn without(v: &mut Value, path: &[&str], name: &str) {
    let Some(obj) = v.as_object_mut() else {
        return;
    };
    match path.split_first() {
        None => {
            obj.remove(name);
        }
        Some((key, rest)) => {
            if let Some(child) = obj.get_mut(*key) {
                without(child, rest, name);
                if child.as_object().is_some_and(|o| o.is_empty()) {
                    obj.remove(*key);
                }
            }
        }
    }
}

/// The edit from `before` to `after` touched member `name` at `path` only: `after` parses, the
/// rest of both is the same, and `name` is `value` (added) or gone (`None`).
fn verify(
    before: &str,
    after: &str,
    path: &[&str],
    name: &str,
    value: Option<&Value>,
) -> Result<(), ParseError> {
    let fail = |what| Err(ParseError { at: 0, what });
    let (mut a, mut b) = (to_value(before)?, to_value(after)?);
    without(&mut a, path, name);
    without(&mut b, path, name);
    if a != b {
        return fail("an edit that would change the rest of the file");
    }
    if get(after, path, name)?.as_ref() != value {
        return fail("an edit that did not come out as asked");
    }
    Ok(())
}

/// The indentation of the line `at` is on.
fn indent_at(text: &str, at: usize) -> &str {
    let line = text
        .get(..at)
        .and_then(|t| t.rfind('\n'))
        .map_or(0, |i| i + 1);
    let rest = text.get(line..).unwrap_or("");
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

/// `value` as pretty JSON whose lines after the first start with `indent`, ended by `nl`.
fn pretty(value: &Value, indent: &str, step: &str, nl: &str) -> String {
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
        .join(nl)
}

fn slice(text: &str, from: usize, to: usize) -> Result<&str, ParseError> {
    text.get(from..to).ok_or(ParseError {
        at: from,
        what: "a span that does not fit the text",
    })
}

/// The text with member `name: value` in the object at `path`, made where missing; unchanged
/// when the object already has a member of that name (whatever its value: the file's owner may
/// have edited it). The result is checked: it parses and nothing else in it changed.
pub fn add(text: &str, path: &[&str], name: &str, value: &Value) -> Result<String, ParseError> {
    let (bom, body) = split_bom(text);
    let out = add_body(body, path, name, value)?;
    let Some(out) = out else {
        return Ok(text.to_owned());
    };
    let out = format!("{bom}{out}");
    verify(text, &out, path, name, Some(value))?;
    Ok(out)
}

fn add_body(
    text: &str,
    path: &[&str],
    name: &str,
    value: &Value,
) -> Result<Option<String>, ParseError> {
    let nl = line_end_of(text);
    if text.trim().is_empty() {
        // {path..: {name: value}} from the inside out
        let mut v = serde_json::json!({ name: value.clone() });
        for key in path.iter().rev() {
            v = serde_json::json!({ *key: v });
        }
        return Ok(Some(format!("{}{nl}", pretty(&v, "", "  ", nl))));
    }
    let root = parse(text)?;
    let step = step_of(text);
    // the deepest object of `path` that exists, and what is left to make
    let mut at = &root;
    let mut made = 0;
    for key in path {
        let members = members_of(at).ok_or(ParseError {
            at: 0,
            what: "no object",
        })?;
        match index_of(members, key)?.and_then(|i| members.get(i)) {
            Some(m) if matches!(m.value, Node::Object { .. }) => {
                at = &m.value;
                made += 1;
            }
            Some(m) => {
                return Err(ParseError {
                    at: m.start,
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
    let (open, close) = (*open, *close);
    if made == path.len() && index_of(members, name)?.is_some() {
        return Ok(None);
    }
    // the member to write: the rest of the path around `name: value`
    let (mut key, mut v) = (name, value.clone());
    for k in path.get(made..).unwrap_or(&[]).iter().rev() {
        v = serde_json::json!({ key: v });
        key = k;
    }
    let key = serde_json::to_string(key).unwrap_or_default();
    let parent_indent = indent_at(text, open).to_owned();
    match members.last() {
        Some(last) => {
            let indent = indent_at(text, last.start).to_owned();
            let member = format!("{key}: {}", pretty(&v, &indent, &step, nl));
            // after the last member (and its trailing comma, if the file has one), and after a
            // line comment that ends its line: the comment stays with its member
            let next = next_token(text, last.end)?;
            let comma = text.as_bytes().get(next) == Some(&b',');
            let past = if comma { next + 1 } else { last.end };
            let rest = slice(text, past, close)?;
            let line_end = match rest.find('\n') {
                Some(n) if rest.get(..n).is_some_and(|r| r.ends_with('\r')) => past + n - 1,
                Some(n) => past + n,
                None => close,
            };
            let tail = slice(text, past, line_end)?.trim();
            let at_end = if tail.is_empty() || (tail.starts_with("//") && !tail.contains('\n')) {
                line_end
            } else {
                past
            };
            let (before, between) = if comma {
                (String::new(), format!("{nl}{indent}{member},"))
            } else {
                (",".to_owned(), format!("{nl}{indent}{member}"))
            };
            Ok(Some(format!(
                "{}{before}{}{between}{}",
                slice(text, 0, last.end)?,
                slice(text, last.end, at_end)?,
                slice(text, at_end, text.len())?
            )))
        }
        None => {
            let indent = format!("{parent_indent}{step}");
            let member = format!("{key}: {}", pretty(&v, &indent, &step, nl));
            let inside = slice(text, open + 1, close)?.trim();
            Ok(Some(format!(
                "{}{nl}{indent}{member}{nl}{parent_indent}{inside}{}",
                slice(text, 0, open + 1)?,
                slice(text, close, text.len())?
            )))
        }
    }
}

/// The text without member `name` of the object at `path`; unchanged when it has none. The cut
/// follows the members and their commas, never whole lines, so a member that shares its line
/// with others goes alone. The result is checked as [`add`]'s is.
pub fn remove(text: &str, path: &[&str], name: &str) -> Result<String, ParseError> {
    let (bom, body) = split_bom(text);
    let Some(out) = remove_body(body, path, name)? else {
        return Ok(text.to_owned());
    };
    let out = format!("{bom}{out}");
    verify(text, &out, path, name, None)?;
    Ok(out)
}

fn remove_body(text: &str, path: &[&str], name: &str) -> Result<Option<String>, ParseError> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let root = parse(text)?;
    let Some(Node::Object {
        members,
        open,
        close,
    }) = find(&root, path)
    else {
        return Ok(None);
    };
    let (open, close) = (*open, *close);
    let Some(i) = index_of(members, name)? else {
        return Ok(None);
    };
    let bytes = text.as_bytes();
    let m = &members[i];
    // the comma after this member, if any
    let after = next_token(text, m.end)?;
    let comma = (bytes.get(after) == Some(&b',')).then_some(after);
    // the blanks just before `at`, back to `floor` (never into a comment or a sibling)
    let blanks_from = |at: usize, floor: usize| {
        let mut from = at;
        while from > floor && matches!(bytes.get(from - 1), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            from -= 1;
        }
        from
    };
    let (from, to) = if members.get(i + 1).is_some() {
        // first or middle: this member, its comma and the blanks after it; the next member
        // takes its place
        let comma = comma.ok_or(ParseError {
            at: m.end,
            what: "members with no comma between them",
        })?;
        let mut to = comma + 1;
        while matches!(bytes.get(to), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            to += 1;
        }
        (m.start, to)
    } else if let Some(prev) = i.checked_sub(1).and_then(|p| members.get(p)) {
        // last: the comma between the two goes, unless this one had its own trailing comma
        let separator = next_token(text, prev.end)?;
        if bytes.get(separator) != Some(&b',') {
            return Err(ParseError {
                at: prev.end,
                what: "members with no comma between them",
            });
        }
        let from = blanks_from(m.start, separator + 1);
        match comma {
            Some(own) => (from, own + 1),
            None => {
                let kept = format!(
                    "{}{}{}",
                    slice(text, 0, separator)?,
                    slice(text, separator + 1, from)?,
                    slice(text, m.end, text.len())?
                );
                return Ok(Some(kept));
            }
        }
    } else {
        // the only member: the object becomes empty, `{}` when nothing else is inside
        let from = blanks_from(m.start, open + 1);
        let to = comma.map_or(m.end, |c| c + 1);
        let left = format!(
            "{}{}",
            slice(text, open + 1, from)?,
            slice(text, to, close)?
        );
        if left.trim().is_empty() {
            return Ok(Some(format!(
                "{}{}",
                slice(text, 0, open + 1)?,
                slice(text, close, text.len())?
            )));
        }
        (from, to)
    };
    Ok(Some(format!(
        "{}{}",
        slice(text, 0, from)?,
        slice(text, to, text.len())?
    )))
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

    /// What the review found: a member sharing its line with others.
    #[test]
    fn a_member_on_a_shared_line_goes_alone() {
        // (a) the only member, on one line with its object's key
        let a = "{\n  \"mcpServers\": {\"wubble\": {\"command\": \"x\"}}\n}\n";
        assert_eq!(
            remove(a, PATH, "wubble").unwrap(),
            "{\n  \"mcpServers\": {}\n}\n"
        );
        // (b) last, with a trailing comma, after a sibling on the same line
        let b =
            "{\"mcpServers\": {\"git\": {\"command\": \"g\"}, \"wubble\": {\"command\": \"x\"},}}";
        assert_eq!(
            remove(b, PATH, "wubble").unwrap(),
            "{\"mcpServers\": {\"git\": {\"command\": \"g\"},}}"
        );
        // (c) first, on the line of its object's key, the next member below
        let c =
            "{\n  \"mcpServers\": {\"wubble\": {\"command\": \"x\"},\n    \"git\": {}\n  }\n}\n";
        assert_eq!(
            remove(c, PATH, "wubble").unwrap(),
            "{\n  \"mcpServers\": {\"git\": {}\n  }\n}\n"
        );
    }

    #[test]
    fn single_line_files_and_every_position() {
        let cases = [
            ("{\"mcpServers\":{\"wubble\":1}}", "{\"mcpServers\":{}}"),
            ("{\"mcpServers\":{\"wubble\":1,}}", "{\"mcpServers\":{}}"),
            (
                "{\"mcpServers\":{\"wubble\":1,\"b\":2}}",
                "{\"mcpServers\":{\"b\":2}}",
            ),
            (
                "{\"mcpServers\":{\"a\":0,\"wubble\":1,\"b\":2}}",
                "{\"mcpServers\":{\"a\":0,\"b\":2}}",
            ),
            (
                "{\"mcpServers\":{\"a\":0, \"wubble\":1}}",
                "{\"mcpServers\":{\"a\":0}}",
            ),
            (
                "{\"mcpServers\":{\"a\":0, \"wubble\":1,}}",
                "{\"mcpServers\":{\"a\":0,}}",
            ),
            (
                "{\"mcpServers\":{\"a\":0, /* c */ \"wubble\":1 /* d */, \"b\":2}}",
                "{\"mcpServers\":{\"a\":0, /* c */ \"b\":2}}",
            ),
        ];
        for (file, want) in cases {
            assert_eq!(remove(file, PATH, "wubble").unwrap(), want, "{file}");
        }
    }

    #[test]
    fn crlf_files_get_crlf_lines_and_a_bom_stays() {
        let file = "\u{feff}{\r\n  \"mcpServers\": {\r\n    \"git\": {}\r\n  }\r\n}\r\n";
        let added = add(file, PATH, "wubble", &entry()).unwrap();
        assert!(added.starts_with('\u{feff}'));
        assert!(!added.replace("\r\n", "").contains('\n'), "{added:?}");
        assert!(has(&added, PATH, "wubble").unwrap());
        assert_eq!(remove(&added, PATH, "wubble").unwrap(), file);
        let empty = add("\u{feff}", PATH, "wubble", &entry()).unwrap();
        assert!(empty.starts_with('\u{feff}') && has(&empty, PATH, "wubble").unwrap());
    }

    #[test]
    fn two_members_of_that_name_refuse_the_file() {
        let file = "{\"mcpServers\": {\"wubble\": 1, \"wubble\": 2}}";
        assert!(add(file, PATH, "wubble", &entry()).is_err());
        assert!(remove(file, PATH, "wubble").is_err());
        assert!(has(file, PATH, "wubble").is_err());
    }

    /// Every layout of a few members, separators, comments and line ends: removing `wubble` from
    /// any position parses and keeps the rest; adding to a layout without it and removing again
    /// gives the same bytes.
    #[test]
    fn generated_layouts_keep_the_rest() {
        let members = |n: usize, at: Option<usize>| -> Vec<String> {
            (0..n)
                .map(|i| {
                    if Some(i) == at {
                        "\"wubble\": {\"command\": \"w\"}".to_owned()
                    } else {
                        format!("\"m{i}\": [{i}, {{\"k\": \"v,}}\"}}]")
                    }
                })
                .collect()
        };
        let seps = [
            ",",
            ", ",
            ",\n    ",
            ",\r\n    ",
            ", // c\n    ",
            " /* c */ ,\n    ",
        ];
        let mut count = 0;
        for n in 1..=4 {
            for at in (0..n).map(Some).chain([None]) {
                for sep in seps {
                    for trailing in [false, true] {
                        for open in ["{", "{\n    ", "{ // o\n    "] {
                            let body = members(n, at).join(sep);
                            let tail = if trailing { "," } else { "" };
                            let file = format!(
                                "{{\n  \"x\": 1,\n  \"mcpServers\": {open}{body}{tail}\n  }}\n}}\n"
                            );
                            check(&file).unwrap();
                            let removed = remove(&file, PATH, "wubble").unwrap();
                            assert!(!has(&removed, PATH, "wubble").unwrap(), "{file}");
                            if at.is_none() {
                                let added = add(&file, PATH, "wubble", &entry()).unwrap();
                                assert_eq!(remove(&added, PATH, "wubble").unwrap(), file);
                            }
                            count += 1;
                        }
                    }
                }
            }
        }
        assert!(count > 400);
    }
}
