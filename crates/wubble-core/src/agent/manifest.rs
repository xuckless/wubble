//! The file a hosting browser writes so a relay (`wubble mcp`, the npm package) or any tool can
//! find it without running anything: one per hosting profile in the per-user runtime directory
//! (`$XDG_RUNTIME_DIR/wubble/mcp/`, the temp directory where there is none), removed when
//! hosting stops and on exit, stale when its process is gone. No secret is in it: the socket is
//! private to the user (0600) and every session needs the owner's approval anyway.
//!
//! The runtime directory, not the config directory: it is per login, private to the user, and
//! cleared at logout or reboot, so a crashed browser's manifest cannot outlive the session it
//! describes; the hosting sockets live there too when a profile path is too long for one.

use serde_json::{Value, json};

/// Where the manifests live, under the runtime directory.
pub const DIR: &[&str] = &["wubble", "mcp"];

/// One hosting browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub pid: u32,
    /// The profile directory.
    pub profile: String,
    /// Whether it is the per-user default profile (a relay without `--profile` prefers it).
    pub default_profile: bool,
    /// The private socket (a named pipe on Windows).
    pub socket: String,
    /// The relay's command line: this browser's binary, `mcp`, and `--profile` when needed.
    pub command: Vec<String>,
    pub version: String,
}

impl Manifest {
    pub fn to_json(&self) -> String {
        let v = json!({
            "name": "wubble",
            "version": self.version,
            "pid": self.pid,
            "profile": self.profile,
            "defaultProfile": self.default_profile,
            "socket": self.socket,
            "transport": "stdio",
            "command": self.command,
            "protocolVersions": protocol_versions(),
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<Manifest> {
        let v: Value = serde_json::from_str(text).ok()?;
        Some(Manifest {
            pid: u32::try_from(v["pid"].as_u64()?).ok()?,
            profile: v["profile"].as_str()?.to_owned(),
            default_profile: v["defaultProfile"].as_bool().unwrap_or(false),
            socket: v["socket"].as_str()?.to_owned(),
            command: v["command"]
                .as_array()?
                .iter()
                .filter_map(|c| c.as_str().map(str::to_owned))
                .collect(),
            version: v["version"].as_str().unwrap_or_default().to_owned(),
        })
    }
}

fn protocol_versions() -> Vec<&'static str> {
    // the newest first: what `server/discover` and `initialize` serve
    let mut all = vec!["2026-07-28"];
    all.extend(["2025-11-25", "2025-06-18", "2025-03-26"]);
    all
}

/// The relay's command line for a browser at `exe` on `profile` (`--profile` only when it is
/// not the default, so the entry stays the same for the usual case).
pub fn command(exe: &str, profile: &str, default_profile: bool) -> Vec<String> {
    let mut c = vec![exe.to_owned(), "mcp".to_owned()];
    if !default_profile {
        c.extend(["--profile".to_owned(), profile.to_owned()]);
    }
    c
}

/// Which hosting browser a relay without `--profile` connects to, among the live ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    One(Manifest),
    /// None hosts.
    None,
    /// Several do and none is the default profile's: the relay names them.
    Several(Vec<Manifest>),
}

pub fn pick(mut live: Vec<Manifest>) -> Pick {
    match live.len() {
        0 => Pick::None,
        1 => Pick::One(live.remove(0)),
        _ => match live.iter().position(|m| m.default_profile) {
            Some(i) => Pick::One(live.remove(i)),
            None => {
                live.sort_by(|a, b| a.profile.cmp(&b.profile));
                Pick::Several(live)
            }
        },
    }
}

/// The error a relay answers with when it has no single browser to relay to.
pub fn pick_error(pick: &Pick) -> Option<String> {
    match pick {
        Pick::One(_) => None,
        Pick::None => Some(
            "wubble is not hosting its MCP server: start wubble and turn it on in Settings > \
             Agents (MCP); or it runs on another profile (pass --profile DIR)"
                .into(),
        ),
        Pick::Several(all) => {
            let list: Vec<String> = all
                .iter()
                .map(|m| format!("{} (pid {})", m.profile, m.pid))
                .collect();
            Some(format!(
                "several wubble browsers host MCP: {}; choose one with `wubble mcp --profile DIR`",
                list.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(profile: &str, default_profile: bool) -> Manifest {
        Manifest {
            pid: 42,
            profile: profile.into(),
            default_profile,
            socket: format!("{profile}/mcp.sock"),
            command: command("/opt/wubble/wubble", profile, default_profile),
            version: "0.1.0".into(),
        }
    }

    #[test]
    fn a_manifest_reads_back_and_carries_no_secret() {
        let a = m("/home/u/.local/share/wubble", true);
        let text = a.to_json();
        assert_eq!(Manifest::from_json(&text), Some(a.clone()));
        assert!(text.contains("\"2026-07-28\"") && text.contains("\"2025-11-25\""));
        assert!(!text.to_lowercase().contains("token"));
        assert_eq!(a.command, ["/opt/wubble/wubble", "mcp"]);
        assert_eq!(
            m("/tmp/p", false).command,
            ["/opt/wubble/wubble", "mcp", "--profile", "/tmp/p"]
        );
        assert_eq!(Manifest::from_json("{\"pid\": \"x\"}"), None);
    }

    #[test]
    fn a_relay_takes_the_one_or_the_default_else_names_them() {
        assert_eq!(pick(vec![]), Pick::None);
        assert!(
            pick_error(&Pick::None)
                .unwrap()
                .contains("Settings > Agents (MCP)")
        );
        assert_eq!(pick(vec![m("/a", false)]), Pick::One(m("/a", false)));
        assert_eq!(
            pick(vec![m("/b", false), m("/d", true)]),
            Pick::One(m("/d", true))
        );
        let several = pick(vec![m("/b", false), m("/a", false)]);
        let error = pick_error(&several).unwrap();
        assert!(error.contains("/a (pid 42), /b (pid 42)") && error.contains("--profile"));
    }
}
