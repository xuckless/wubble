//! The agent apps ("harnesses") on this computer and wubble's entry in their MCP configuration.
//! No MCP revision defines a way for an app to find a local server it was not configured with
//! (docs/research/mcp-server.md, "Being findable"): each app reads its own file. So turning
//! hosting on adds wubble's stdio entry (`wubble mcp`) to every app found here whose format was
//! verified from its documentation or source, and turning it off removes exactly the entries
//! wubble added; an app the owner took wubble out of stays out. An app whose format could not
//! be verified is never written: Settings offers its entry to copy.
//!
//! Pure: the host says which paths exist and hands the files' text in and out (a backup beside
//! each file before the first write, the file replaced whole and atomically). Edits keep the
//! rest of the text byte for byte (`jsonc`; `toml_edit` for TOML) and refuse a file that does not
//! parse.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::jsonc;

/// The name of wubble's entry in every app's server table.
pub const NAME: &str = "wubble";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Mac,
    Windows,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// How an app's file holds its servers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// A JSON or JSONC object at `path` holds the servers by name.
    Json {
        path: &'static [&'static str],
        entry: Entry,
    },
    /// Codex's TOML: a `[mcp_servers.<name>]` table.
    CodexToml,
}

/// The shape of one server entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `{"command": .., "args": [..]}`
    Plain,
    /// `{"type": "stdio", "command": .., "args": [..]}`
    Stdio,
    /// `{"command": .., "args": [..], "timeout": ms}`: Gemini CLI cuts a call at 10 minutes
    /// by default; an approval can take longer.
    PlainTimeout,
    /// opencode: `{"type": "local", "command": [command, args..]}`.
    OpenCode,
}

/// One app: where its user-level file is per OS (relative to the home directory; the first
/// that exists, else the first), what shows it is installed, and how its file is written
/// (`None`: not verified, never written).
#[derive(Debug)]
pub struct Harness {
    pub id: &'static str,
    pub name: &'static str,
    files: &'static [(Os, &'static str)],
    detect: &'static [(Os, &'static str)],
    pub format: Option<Format>,
}

const LM: [Os; 2] = [Os::Linux, Os::Mac];

macro_rules! both {
    ($p:expr) => {
        &[(LM[0], $p), (LM[1], $p)]
    };
}

/// Every app wubble knows, in the order Settings lists them. Sources: docs/research/mcp-server.md
/// "Being findable".
pub static HARNESSES: &[Harness] = &[
    Harness {
        id: "claude-code",
        name: "Claude Code",
        files: both!(".claude.json"),
        detect: both!(".claude.json"),
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Stdio,
        }),
    },
    Harness {
        id: "claude-desktop",
        name: "Claude (desktop app)",
        files: &[
            (Os::Linux, ".config/Claude/claude_desktop_config.json"),
            (
                Os::Mac,
                "Library/Application Support/Claude/claude_desktop_config.json",
            ),
        ],
        detect: &[
            (Os::Linux, ".config/Claude"),
            (Os::Mac, "Library/Application Support/Claude"),
        ],
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Plain,
        }),
    },
    Harness {
        id: "codex",
        name: "Codex",
        files: both!(".codex/config.toml"),
        detect: both!(".codex"),
        format: Some(Format::CodexToml),
    },
    Harness {
        id: "gemini-cli",
        name: "Gemini CLI",
        files: both!(".gemini/settings.json"),
        detect: both!(".gemini/settings.json"),
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::PlainTimeout,
        }),
    },
    Harness {
        id: "antigravity",
        name: "Antigravity",
        files: both!(".gemini/config/mcp_config.json"),
        detect: both!(".gemini/config"),
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Plain,
        }),
    },
    Harness {
        id: "cursor",
        name: "Cursor",
        files: both!(".cursor/mcp.json"),
        detect: both!(".cursor"),
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Stdio,
        }),
    },
    Harness {
        id: "opencode",
        name: "opencode",
        files: &[
            (Os::Linux, ".config/opencode/opencode.jsonc"),
            (Os::Linux, ".config/opencode/opencode.json"),
            (Os::Mac, ".config/opencode/opencode.jsonc"),
            (Os::Mac, ".config/opencode/opencode.json"),
        ],
        detect: both!(".config/opencode"),
        // `mcp.<name>`: what opencode 1.17 reads (tried: `mcp.servers.<name>`, which its newer
        // published schema shows, is refused there as a server named "servers")
        format: Some(Format::Json {
            path: &["mcp"],
            entry: Entry::OpenCode,
        }),
    },
    Harness {
        id: "zed",
        name: "Zed",
        files: both!(".config/zed/settings.json"),
        detect: both!(".config/zed"),
        format: Some(Format::Json {
            path: &["context_servers"],
            entry: Entry::Plain,
        }),
    },
    Harness {
        id: "windsurf",
        name: "Windsurf (Devin Local)",
        files: &[(Os::Linux, ".config/devin/mcp_config.json")],
        detect: &[(Os::Linux, ".config/devin")],
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Plain,
        }),
    },
    Harness {
        id: "kiro",
        name: "Kiro",
        files: both!(".kiro/settings/mcp.json"),
        detect: both!(".kiro"),
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Plain,
        }),
    },
    Harness {
        id: "amp",
        name: "Amp",
        files: both!(".config/amp/settings.json"),
        detect: both!(".config/amp"),
        format: Some(Format::Json {
            path: &["amp.mcpServers"],
            entry: Entry::Plain,
        }),
    },
    Harness {
        id: "cline",
        name: "Cline (CLI)",
        files: both!(".cline/data/settings/cline_mcp_settings.json"),
        detect: both!(".cline"),
        format: Some(Format::Json {
            path: &["mcpServers"],
            entry: Entry::Plain,
        }),
    },
    // found, never written: the file's name or its entry's shape is not verified
    Harness {
        id: "vscode",
        name: "VS Code",
        files: &[
            (Os::Linux, ".config/Code/User/mcp.json"),
            (Os::Mac, "Library/Application Support/Code/User/mcp.json"),
        ],
        detect: &[
            (Os::Linux, ".config/Code/User"),
            (Os::Mac, "Library/Application Support/Code/User"),
        ],
        format: None,
    },
    Harness {
        id: "copilot-cli",
        name: "Copilot CLI",
        files: both!(".copilot/mcp-config.json"),
        detect: both!(".copilot"),
        format: None,
    },
    Harness {
        id: "goose",
        name: "Goose",
        files: both!(".config/goose/config.yaml"),
        detect: both!(".config/goose"),
        format: None,
    },
];

pub fn find(id: &str) -> Option<&'static Harness> {
    HARNESSES.iter().find(|h| h.id == id)
}

impl Harness {
    /// Whether it is installed, by a file or directory of its own (nothing is run).
    pub fn detected(&self, os: Os, home: &Path, exists: &dyn Fn(&Path) -> bool) -> bool {
        self.detect
            .iter()
            .any(|(o, p)| *o == os && exists(&home.join(p)))
    }

    /// Its user-level file: the first that exists, else the first it would make.
    pub fn file(&self, os: Os, home: &Path, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
        let mine: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|(o, _)| *o == os)
            .map(|(_, p)| home.join(p))
            .collect();
        mine.iter().find(|p| exists(p)).or(mine.first()).cloned()
    }

    /// Whether wubble writes to it.
    pub fn writable(&self) -> bool {
        self.format.is_some()
    }

    /// Its entry for `command` (`[wubble, mcp, ..]`), as it would be written (Codex's TOML table
    /// as the same JSON); for an app wubble does not write, the generic `mcpServers` entry to
    /// copy.
    pub fn entry(&self, command: &[String]) -> Value {
        let (exe, args) = command.split_first().map_or(("", &[][..]), |(e, a)| (e, a));
        match self.format {
            Some(Format::CodexToml) => {
                json!({"command": exe, "args": args, "tool_timeout_sec": APPROVAL_MS / 1000})
            }
            Some(Format::Json {
                entry: Entry::Stdio,
                ..
            }) => json!({"type": "stdio", "command": exe, "args": args}),
            Some(Format::Json {
                entry: Entry::PlainTimeout,
                ..
            }) => json!({"command": exe, "args": args, "timeout": APPROVAL_MS}),
            Some(Format::Json {
                entry: Entry::OpenCode,
                ..
            }) => json!({"type": "local", "command": command}),
            _ => json!({"command": exe, "args": args}),
        }
    }

    /// The text with wubble's entry added (unchanged when one is there); refused when the file
    /// does not parse or the app is not written.
    pub fn add(&self, text: &str, command: &[String]) -> Result<String, EditError> {
        let entry = self.entry(command);
        match self.format.ok_or(EditError::NotVerified)? {
            Format::Json { path, .. } => {
                jsonc::add(text, path, NAME, &entry).map_err(EditError::Parse)
            }
            Format::CodexToml => toml_add(text, &entry),
        }
    }

    /// The text with wubble's entry set to `command`'s, whatever was there before.
    pub fn put(&self, text: &str, command: &[String]) -> Result<String, EditError> {
        self.add(&self.remove(text)?, command)
    }

    /// The entry named wubble in the file, as JSON, if it has one.
    pub fn existing(&self, text: &str) -> Result<Option<Value>, EditError> {
        match self.format.ok_or(EditError::NotVerified)? {
            Format::Json { path, .. } => jsonc::get(text, path, NAME).map_err(EditError::Parse),
            Format::CodexToml => Ok(toml_doc(text)?
                .get("mcp_servers")
                .and_then(|t| t.get(NAME))
                .map(toml_json)),
        }
    }

    /// The text without wubble's entry (unchanged when it has none).
    pub fn remove(&self, text: &str) -> Result<String, EditError> {
        match self.format.ok_or(EditError::NotVerified)? {
            Format::Json { path, .. } => jsonc::remove(text, path, NAME).map_err(EditError::Parse),
            Format::CodexToml => toml_remove(text),
        }
    }

    /// Whether the file has an entry named wubble.
    pub fn has(&self, text: &str) -> Result<bool, EditError> {
        match self.format.ok_or(EditError::NotVerified)? {
            Format::Json { path, .. } => jsonc::has(text, path, NAME).map_err(EditError::Parse),
            Format::CodexToml => Ok(toml_doc(text)?
                .get("mcp_servers")
                .and_then(|t| t.get(NAME))
                .is_some()),
        }
    }
}

/// When wubble looks at an app's entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    /// The owner turned hosting on, or pressed Add on the app: wubble may add itself.
    TurnOn,
    /// A start with hosting already on: wubble only keeps its own entries current.
    Resume,
    /// Hosting turned off, or Remove pressed: wubble's own entry goes.
    TurnOff,
}

/// What to do with one app's entry, and whether wubble owns it afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// No entry: write wubble's.
    Write,
    /// An entry equal to wubble's: take it as wubble's, unchanged.
    Adopt,
    /// wubble's own entry, out of date (the binary or profile moved): write the current one.
    Refresh,
    /// An entry wubble did not write (or the owner changed): never touched.
    NotManaged,
    /// No entry and wubble may not add one now (the owner deleted it): left so.
    Absent,
    /// wubble's own entry: taken out.
    Remove,
}

impl Plan {
    /// Whether wubble owns the entry after the plan ran (records it as written).
    pub fn owned(self) -> bool {
        matches!(self, Plan::Write | Plan::Adopt | Plan::Refresh)
    }
}

/// The ownership rule: wubble owns an entry only if it wrote it (`recorded`, what it wrote last)
/// or found exactly what it would write; it rewrites only its own, removes only one that is still
/// what it wrote, and never adds one back on a start after the owner deleted it.
pub fn plan(
    moment: Moment,
    existing: Option<&Value>,
    recorded: Option<&Value>,
    wanted: &Value,
) -> Plan {
    match (moment, existing) {
        (Moment::TurnOff, Some(e)) if recorded == Some(e) => Plan::Remove,
        (Moment::TurnOff, Some(_)) => Plan::NotManaged,
        (Moment::TurnOff | Moment::Resume, None) => Plan::Absent,
        (Moment::TurnOn, None) => Plan::Write,
        (_, Some(e)) if e == wanted => Plan::Adopt,
        (_, Some(e)) if recorded == Some(e) => Plan::Refresh,
        (_, Some(_)) => Plan::NotManaged,
    }
}

/// A tool-call timeout long enough for the owner to answer the approval sheet: an hour.
const APPROVAL_MS: u64 = 3_600_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The file does not parse: left alone, with why.
    Parse(jsonc::ParseError),
    Toml(String),
    /// The app's format is not verified: wubble never writes it.
    NotVerified,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::Parse(e) => write!(f, "its file does not parse ({e}): left as it is"),
            EditError::Toml(e) => write!(f, "its file does not parse ({e}): left as it is"),
            EditError::NotVerified => f.write_str("its configuration format is not verified"),
        }
    }
}

fn toml_doc(text: &str) -> Result<toml_edit::DocumentMut, EditError> {
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|e| EditError::Toml(e.message().to_owned()))
}

/// A TOML item as JSON (dates as their text), to compare entries and whole files.
fn toml_json(item: &toml_edit::Item) -> Value {
    use toml_edit::Value as V;
    fn value(v: &V) -> Value {
        match v {
            V::String(s) => json!(s.value()),
            V::Integer(i) => json!(i.value()),
            V::Float(f) => json!(f.value()),
            V::Boolean(b) => json!(b.value()),
            V::Datetime(d) => json!(d.value().to_string()),
            V::Array(a) => Value::Array(a.iter().map(value).collect()),
            V::InlineTable(t) => {
                Value::Object(t.iter().map(|(k, v)| (k.to_owned(), value(v))).collect())
            }
        }
    }
    match item {
        toml_edit::Item::None => Value::Null,
        toml_edit::Item::Value(v) => value(v),
        toml_edit::Item::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_owned(), toml_json(v)))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(a) => Value::Array(
            a.iter()
                .map(|t| toml_json(&toml_edit::Item::Table(t.clone())))
                .collect(),
        ),
    }
}

/// The TOML edit from `before` to `after` touched `[mcp_servers.wubble]` only, and it came out
/// as `entry` (or gone).
fn toml_verify(before: &str, after: &str, entry: Option<&Value>) -> Result<(), EditError> {
    let rest = |text: &str| -> Result<(Value, Option<Value>), EditError> {
        let mut v = toml_json(toml_doc(text)?.as_item());
        let mut ours = None;
        if let Some(top) = v.as_object_mut() {
            if let Some(servers) = top.get_mut("mcp_servers").and_then(Value::as_object_mut) {
                ours = servers.remove(NAME);
            }
            if top
                .get("mcp_servers")
                .and_then(Value::as_object)
                .is_some_and(|s| s.is_empty())
            {
                top.remove("mcp_servers");
            }
        }
        Ok((v, ours))
    };
    let ((a, _), (b, ours)) = (rest(before)?, rest(after)?);
    if a != b || ours.as_ref() != entry {
        return Err(EditError::Toml(
            "an edit that would change the rest of the file".into(),
        ));
    }
    Ok(())
}

fn toml_add(text: &str, wanted: &Value) -> Result<String, EditError> {
    let mut doc = toml_doc(text)?;
    if doc.get("mcp_servers").and_then(|t| t.get(NAME)).is_some() {
        return Ok(text.to_owned());
    }
    let servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        })
        .as_table_mut()
        .ok_or_else(|| EditError::Toml("mcp_servers is not a table".into()))?;
    let mut entry = toml_edit::Table::new();
    entry["command"] = toml_edit::value(wanted["command"].as_str().unwrap_or(""));
    entry["args"] = toml_edit::value(
        wanted["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<toml_edit::Array>(),
    );
    // Codex cuts a call at 300 s by default; an approval can take longer
    entry["tool_timeout_sec"] = toml_edit::value(wanted["tool_timeout_sec"].as_i64().unwrap_or(0));
    servers.insert(NAME, toml_edit::Item::Table(entry));
    let out = doc.to_string();
    toml_verify(text, &out, Some(wanted))?;
    Ok(out)
}

fn toml_remove(text: &str) -> Result<String, EditError> {
    let mut doc = toml_doc(text)?;
    let Some(servers) = doc.get_mut("mcp_servers").and_then(|t| t.as_table_mut()) else {
        return Ok(text.to_owned());
    };
    if servers.remove(NAME).is_none() {
        return Ok(text.to_owned());
    }
    if servers.is_empty() && servers.is_implicit() {
        doc.remove("mcp_servers");
    }
    let out = doc.to_string();
    toml_verify(text, &out, None)?;
    Ok(out)
}

/// Where a backup of `file` goes before wubble's first write to it.
pub fn backup_of(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".wubble-backup");
    file.with_file_name(name)
}

/// The generic entry, as JSON a person pastes into any app's `mcpServers`.
pub fn generic_entry(command: &[String]) -> String {
    let (exe, args) = command.split_first().map_or(("", &[][..]), |(e, a)| (e, a));
    let v = json!({ NAME: {"command": exe, "args": args} });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The command line as a shell would take it.
pub fn shell_line(command: &[String]) -> String {
    command
        .iter()
        .map(|a| {
            if !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/._-+=:@".contains(c))
            {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Which apps hosting adds wubble to: found here, written by wubble, not opted out.
pub fn targets(found: &[&'static Harness], opted_out: &[String]) -> Vec<&'static Harness> {
    found
        .iter()
        .copied()
        .filter(|h| h.writable() && !opted_out.iter().any(|o| o == h.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd() -> Vec<String> {
        vec!["/opt/wubble/wubble".into(), "mcp".into()]
    }

    /// Fixture files as each app writes them, with keys and comments wubble must keep.
    fn fixture(id: &str) -> &'static str {
        match id {
            "claude-code" => {
                "{\n  \"numStartups\": 12,\n  \"projects\": {\n    \"/home/u/x\": {\n      \"mcpServers\": {}\n    }\n  },\n  \"mcpServers\": {\n    \"git\": {\n      \"type\": \"stdio\",\n      \"command\": \"git-mcp\",\n      \"args\": []\n    }\n  }\n}\n"
            }
            "claude-desktop" => "{\n  \"mcpServers\": {}\n}\n",
            "codex" => {
                "# my codex\nmodel = \"o4\"\n\n[mcp_servers.docs]\ncommand = \"npx\" # the docs\nargs = [\"-y\", \"docs\"]\n"
            }
            "gemini-cli" => {
                "{\n  // theme\n  \"theme\": \"GitHub\",\n  \"mcpServers\": {\n    \"x\": {\"command\": \"x\"},\n  },\n}\n"
            }
            "zed" => {
                "// Zed settings\n{\n  \"vim_mode\": true,\n  \"context_servers\": {\n    // mine\n    \"a\": {\"command\": \"a\", \"args\": []}\n  }\n}\n"
            }
            "opencode" => {
                "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  // plugins\n  \"plugin\": [\"x\"]\n}\n"
            }
            "amp" => "{\n  \"amp.notifications.enabled\": false\n}\n",
            _ => "",
        }
    }

    #[test]
    fn every_written_app_adds_once_removes_back_and_keeps_the_rest() {
        for h in HARNESSES.iter().filter(|h| h.writable()) {
            let file = fixture(h.id);
            let added = h.add(file, &cmd()).unwrap();
            assert!(h.has(&added).unwrap(), "{}", h.id);
            assert_eq!(h.add(&added, &cmd()).unwrap(), added, "{}: twice", h.id);
            let removed = h.remove(&added).unwrap();
            let had_path = match h.format {
                Some(Format::Json { path, .. }) => file.contains(&format!("\"{}\"", path[0])),
                _ => true,
            };
            if file.is_empty() || !had_path {
                // the objects wubble made stay, empty; everything else as it was
                assert!(!h.has(&removed).unwrap(), "{}", h.id);
                let kept = file.trim_end().trim_end_matches('}').trim_end();
                assert!(removed.starts_with(kept), "{}: {removed}", h.id);
            } else {
                assert_eq!(removed, file, "{}: removed back to the same bytes", h.id);
            }
            assert_eq!(h.remove(file).unwrap(), file, "{}: nothing to remove", h.id);
        }
    }

    #[test]
    fn the_entries_are_each_app_s_shape() {
        let v = |id: &str| -> Value {
            let h = find(id).unwrap();
            let added = h.add(fixture(id), &cmd()).unwrap();
            match h.format.unwrap() {
                Format::Json { path, .. } => {
                    let text: String = added
                        .lines()
                        .filter(|l| !l.trim_start().starts_with("//"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    // the JSONC fixtures with trailing commas are checked by `has`
                    jsonc::get(&text, path, NAME)
                        .unwrap()
                        .unwrap_or(Value::Null)
                }
                Format::CodexToml => Value::String(added),
            }
        };
        assert_eq!(
            v("claude-code"),
            json!({"type": "stdio", "command": "/opt/wubble/wubble", "args": ["mcp"]})
        );
        assert_eq!(
            v("opencode"),
            json!({"type": "local", "command": ["/opt/wubble/wubble", "mcp"]})
        );
        assert_eq!(v("gemini-cli")["timeout"], APPROVAL_MS);
        assert_eq!(
            v("zed"),
            json!({"command": "/opt/wubble/wubble", "args": ["mcp"]})
        );
        let codex = v("codex");
        let codex = codex.as_str().unwrap();
        assert!(codex.starts_with(
            "# my codex\nmodel = \"o4\"\n\n[mcp_servers.docs]\ncommand = \"npx\" # the docs\n"
        ));
        assert!(codex.contains("[mcp_servers.wubble]\ncommand = \"/opt/wubble/wubble\"\nargs = [\"mcp\"]\ntool_timeout_sec = 3600\n"));
        // the Claude Code project's own mcpServers is not touched
        let cc = find("claude-code")
            .unwrap()
            .add(fixture("claude-code"), &cmd())
            .unwrap();
        assert!(cc.contains("\"/home/u/x\": {\n      \"mcpServers\": {}\n    }"));
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        for h in HARNESSES.iter().filter(|h| h.writable()) {
            let bad = if h.format == Some(Format::CodexToml) {
                "[mcp_servers\nx ="
            } else {
                "{\"mcpServers\": {"
            };
            assert!(h.add(bad, &cmd()).is_err(), "{}", h.id);
            assert!(h.remove(bad).is_err(), "{}", h.id);
        }
        let vs = find("vscode").unwrap();
        assert_eq!(vs.add("{}", &cmd()), Err(EditError::NotVerified));
    }

    #[test]
    fn found_by_their_own_paths_and_opted_out_ones_left_alone() {
        let home = Path::new("/home/u");
        let have = [
            "/home/u/.codex",
            "/home/u/.config/opencode",
            "/home/u/.config/opencode/opencode.json",
            "/home/u/.config/Code/User",
        ];
        let exists = |p: &Path| have.iter().any(|h| Path::new(h) == p);
        let found: Vec<&Harness> = HARNESSES
            .iter()
            .filter(|h| h.detected(Os::Linux, home, &exists))
            .collect();
        let ids: Vec<&str> = found.iter().map(|h| h.id).collect();
        assert_eq!(ids, ["codex", "opencode", "vscode"]);
        assert_eq!(
            find("opencode").unwrap().file(Os::Linux, home, &exists),
            Some(PathBuf::from("/home/u/.config/opencode/opencode.json"))
        );
        assert_eq!(
            find("codex").unwrap().file(Os::Mac, home, &exists),
            Some(PathBuf::from("/home/u/.codex/config.toml"))
        );
        let to: Vec<&str> = targets(&found, &["codex".into()])
            .iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(
            to,
            ["opencode"],
            "VS Code is never written, Codex opted out"
        );
        assert_eq!(
            backup_of(Path::new("/a/mcp.json")),
            PathBuf::from("/a/mcp.json.wubble-backup")
        );
        assert_eq!(
            shell_line(&["/opt/my apps/wubble".into(), "mcp".into()]),
            "'/opt/my apps/wubble' mcp"
        );
        assert!(generic_entry(&cmd()).contains("\"command\": \"/opt/wubble/wubble\""));
    }

    #[test]
    fn wubble_owns_only_what_it_wrote() {
        use Moment::*;
        let ours = json!({"command": "/opt/wubble", "args": ["mcp"]});
        let old = json!({"command": "/old/wubble", "args": ["mcp"]});
        let hand = json!({"command": "/home/u/bin/wubble", "args": ["mcp"]});
        // turned on: written, adopted when equal, refreshed only when it is still what wubble wrote
        assert_eq!(plan(TurnOn, None, None, &ours), Plan::Write);
        assert_eq!(plan(TurnOn, Some(&ours), None, &ours), Plan::Adopt);
        assert_eq!(plan(TurnOn, Some(&old), Some(&old), &ours), Plan::Refresh);
        assert_eq!(plan(TurnOn, Some(&hand), None, &ours), Plan::NotManaged);
        assert_eq!(
            plan(TurnOn, Some(&hand), Some(&old), &ours),
            Plan::NotManaged
        );
        // a start: never added back after the owner deleted it
        assert_eq!(plan(Resume, None, Some(&ours), &ours), Plan::Absent);
        assert_eq!(plan(Resume, Some(&old), Some(&old), &ours), Plan::Refresh);
        assert_eq!(plan(Resume, Some(&hand), None, &ours), Plan::NotManaged);
        // turned off: only what wubble wrote, unchanged since, goes
        assert_eq!(plan(TurnOff, Some(&ours), Some(&ours), &ours), Plan::Remove);
        assert_eq!(
            plan(TurnOff, Some(&hand), Some(&ours), &ours),
            Plan::NotManaged
        );
        assert_eq!(plan(TurnOff, Some(&ours), None, &ours), Plan::NotManaged);
        assert_eq!(plan(TurnOff, None, Some(&ours), &ours), Plan::Absent);
        assert!(Plan::Write.owned() && Plan::Refresh.owned() && !Plan::NotManaged.owned());
    }

    /// Two profiles host at once: the default one's entry is in the file; the other finds an
    /// entry it did not write and leaves it, and the first's turning off removes only its own.
    #[test]
    fn a_second_profile_leaves_the_first_one_s_entry() {
        let h = find("cursor").unwrap();
        let a = vec!["/opt/wubble".to_owned(), "mcp".to_owned()];
        let b = manifest_cmd("/tmp/b");
        let file = h.add("{}", &a).unwrap();
        let a_recorded = h.entry(&a);
        let existing = h.existing(&file).unwrap();
        assert_eq!(
            plan(Moment::TurnOn, existing.as_ref(), None, &h.entry(&b)),
            Plan::NotManaged
        );
        assert_eq!(
            plan(Moment::TurnOff, existing.as_ref(), None, &h.entry(&b)),
            Plan::NotManaged
        );
        assert_eq!(
            plan(
                Moment::TurnOff,
                existing.as_ref(),
                Some(&a_recorded),
                &h.entry(&a)
            ),
            Plan::Remove
        );
    }

    fn manifest_cmd(profile: &str) -> Vec<String> {
        super::super::manifest::command("/opt/wubble", profile, false)
    }

    #[test]
    fn every_app_reads_back_and_rewrites_its_entry() {
        for h in HARNESSES.iter().filter(|h| h.writable()) {
            let old = vec!["/old/wubble".to_owned(), "mcp".to_owned()];
            let added = h.add(fixture(h.id), &old).unwrap();
            assert_eq!(h.existing(&added).unwrap(), Some(h.entry(&old)), "{}", h.id);
            let put = h.put(&added, &cmd()).unwrap();
            assert_eq!(h.existing(&put).unwrap(), Some(h.entry(&cmd())), "{}", h.id);
            assert_eq!(h.existing(fixture(h.id)).unwrap(), None, "{}", h.id);
        }
    }
}
