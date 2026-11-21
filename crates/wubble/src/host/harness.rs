//! The agent apps' MCP configuration (`wubble_core::agent::harness`), at the edge: which apps
//! are here, their files read and written (a backup beside each before wubble's first write, the
//! new text replaced in whole), and the Settings group that shows them.

use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use serde_json::Value;
use wubble_core::agent::harness::{self, HARNESSES, Harness, Moment, Os, Plan};
use wubble_core::agent::hosting::Hosting;
use wubble_core::apply::value_of;
use wubble_core::settings as defs;
use wubble_ui::{AgentAppItem, AgentsView, AppWindow};

use super::Host;

fn exists(p: &Path) -> bool {
    p.exists()
}

/// Why a write did not happen.
#[derive(Debug)]
enum WriteError {
    /// The file changed between wubble's read and its write: nothing was written.
    Changed,
    Io(std::io::Error),
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self {
        WriteError::Io(e)
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Changed => f.write_str("the file changed while wubble wrote it"),
            WriteError::Io(e) => e.fmt(f),
        }
    }
}

/// A new file created with exactly `mode` (never wider, not even briefly), or failing.
fn create_with_mode(path: &Path, mode: u32) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(mode);
        let file = options.open(path)?;
        // the umask may have narrowed it: exactly the original's
        if let Err(e) = file.set_permissions(std::fs::Permissions::from_mode(mode)) {
            let _ = std::fs::remove_file(path);
            return Err(e);
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        options.open(path)
    }
}

/// `text` written to `file` in whole, `before` being what wubble read (`None`: no file). A
/// symlinked file is written at its target, the link kept; the new file and the backup have the
/// original's mode (0600 for a new one) from their creation; `backup` (the text holds no entry of
/// wubble's yet) saves `before` beside it once, never over an older backup; the new text is
/// synced before it replaces the file, and nothing is written if the file changed since it was
/// read.
fn write_file(
    file: &Path,
    before: Option<&str>,
    backup: bool,
    text: &str,
) -> Result<(), WriteError> {
    use std::io::Write;
    let target = match std::fs::symlink_metadata(file) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(file)?,
        _ => file.to_owned(),
    };
    let dir = target
        .parent()
        .ok_or_else(|| std::io::Error::other("a file with no directory"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&target).map_or(0o600, |m| m.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let mode = 0o600;
    if let Some(before) = before
        && backup
    {
        let path = harness::backup_of(&target);
        match create_with_mode(&path, mode) {
            Ok(mut f) => {
                f.write_all(before.as_bytes())?;
                f.sync_all()?;
            }
            // an older backup is the pre-wubble state: kept
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.wubble-tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let written = (|| -> Result<(), WriteError> {
        let mut f = create_with_mode(&tmp, mode)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        let now = match read(&target)? {
            Found::Missing => None,
            Found::Text(t) => Some(t),
        };
        if now.as_deref() != before {
            return Err(WriteError::Changed);
        }
        std::fs::rename(&tmp, &target)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// What one app's file says now.
enum Found {
    Missing,
    Text(String),
}

fn read(file: &Path) -> std::io::Result<Found> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(Found::Text(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Found::Missing),
        Err(e) => Err(e),
    }
}

impl Host {
    /// The home directory the apps' files are under: none (nothing is read or written) when it
    /// could not be resolved to an absolute path.
    fn harness_home(&self) -> Option<PathBuf> {
        self.hosting
            .place
            .as_ref()
            .and_then(|p| p.home.clone())
            .filter(|h| h.is_absolute())
    }

    fn stored_list(&self, def: &defs::SettingDef) -> Vec<String> {
        serde_json::from_str(value_of(def, &self.settings.stored)).unwrap_or_default()
    }

    fn store_list(&mut self, def: &defs::SettingDef, list: &[String]) {
        let text = serde_json::to_string(list).unwrap_or_else(|_| "[]".into());
        self.write_setting(def.key, &text);
    }

    /// The apps found here, in the table's order.
    fn harnesses_found(&self) -> Vec<&'static Harness> {
        let Some(home) = self.harness_home() else {
            return vec![];
        };
        HARNESSES
            .iter()
            .filter(|h| h.detected(Os::current(), &home, &exists))
            .collect()
    }

    fn mcp_command(&self) -> Vec<String> {
        self.hosting.place.as_ref().map_or_else(Vec::new, |p| {
            let profile = p.profile.to_string_lossy();
            wubble_core::agent::manifest::command(
                &p.exe.to_string_lossy(),
                &profile,
                p.default_profile,
            )
        })
    }

    /// The entries wubble owns, by app: what it wrote (`agent.mcp_added`).
    fn records(&self) -> BTreeMap<String, Value> {
        serde_json::from_str(value_of(&defs::AGENT_MCP_ADDED, &self.settings.stored))
            .unwrap_or_default()
    }

    fn store_records(&mut self, records: &BTreeMap<String, Value>) {
        let text = serde_json::to_string(records).unwrap_or_else(|_| "{}".into());
        if text != value_of(&defs::AGENT_MCP_ADDED, &self.settings.stored) {
            self.write_setting(defs::AGENT_MCP_ADDED.key, &text);
        }
    }

    /// One app's entry at `moment`, by the ownership rule (`harness::plan`): written, refreshed
    /// or removed only when wubble owns it; what it owns afterwards is recorded.
    fn harness_apply(&mut self, h: &'static Harness, moment: Moment) {
        let result = self.harness_plan_run(h, moment);
        match result {
            Ok(_) => {
                self.hosting.notes.remove(h.id);
            }
            Err(e) => {
                eprintln!("wubble: {} left as it is: {e}", h.name);
                self.hosting.notes.insert(h.id, e);
            }
        }
    }

    fn harness_plan_run(&mut self, h: &'static Harness, moment: Moment) -> Result<Plan, String> {
        // a file that changed under wubble is read again once
        match self.harness_plan_try(h, moment) {
            Err(WriteError::Changed) => self.harness_plan_try(h, moment),
            other => other,
        }
        .map_err(|e| e.to_string())
    }

    fn harness_plan_try(
        &mut self,
        h: &'static Harness,
        moment: Moment,
    ) -> Result<Plan, WriteError> {
        let fail = |e: String| WriteError::Io(std::io::Error::other(e));
        let home = self
            .harness_home()
            .ok_or_else(|| fail("no home directory".into()))?;
        let file = h
            .file(Os::current(), &home, &exists)
            .ok_or_else(|| fail("no file".into()))?;
        let before = match read(&file)? {
            Found::Missing => None,
            Found::Text(t) => Some(t),
        };
        let text = before.as_deref().unwrap_or("");
        let existing = h.existing(text).map_err(|e| fail(e.to_string()))?;
        let mut records = self.records();
        let wanted = h.entry(&self.mcp_command());
        let plan = harness::plan(moment, existing.as_ref(), records.get(h.id), &wanted);
        let after = match plan {
            Plan::Write => Some(h.add(text, &self.mcp_command())),
            Plan::Refresh => Some(h.put(text, &self.mcp_command())),
            Plan::Remove => Some(h.remove(text)),
            Plan::Adopt | Plan::NotManaged | Plan::Absent => None,
        };
        if let Some(after) = after {
            let after = after.map_err(|e| fail(e.to_string()))?;
            if after != text {
                // a backup only of a state without wubble's entry
                write_file(&file, before.as_deref(), existing.is_none(), &after)?;
            }
        }
        if plan.owned() {
            records.insert(h.id.to_owned(), wanted);
        } else {
            records.remove(h.id);
        }
        self.store_records(&records);
        Ok(plan)
    }

    /// `Effect::Register`: wubble into every app found here that it writes and the owner has
    /// not taken it out of (an entry it did not write is left alone).
    pub(super) fn mcp_register(&mut self) {
        let opted_out = self.stored_list(&defs::AGENT_MCP_OPTED_OUT);
        let found = self.harnesses_found();
        for h in harness::targets(&found, &opted_out) {
            self.harness_apply(h, Moment::TurnOn);
        }
    }

    /// `Effect::Refresh`: a start with hosting on: wubble's own entries kept current (the binary
    /// or profile may have moved), none added; one the owner deleted stays deleted (its row
    /// offers Add).
    pub(super) fn mcp_refresh(&mut self) {
        let opted_out = self.stored_list(&defs::AGENT_MCP_OPTED_OUT);
        let found = self.harnesses_found();
        for h in harness::targets(&found, &opted_out) {
            self.harness_apply(h, Moment::Resume);
        }
    }

    /// `Effect::Unregister`: exactly the entries wubble owns, still as it wrote them, come out.
    pub(super) fn mcp_unregister(&mut self) {
        for id in self.records().into_keys() {
            if let Some(h) = harness::find(&id) {
                self.harness_apply(h, Moment::TurnOff);
            }
        }
    }

    /// Settings: "Remove" on one app: wubble's own entry out of its file now, and the app left
    /// out from now on.
    pub(crate) fn agent_app_removed(&mut self, id: &str) {
        let Some(h) = harness::find(id) else {
            return;
        };
        let mut opted_out = self.stored_list(&defs::AGENT_MCP_OPTED_OUT);
        if !opted_out.iter().any(|o| o == id) {
            opted_out.push(id.to_owned());
        }
        self.store_list(&defs::AGENT_MCP_OPTED_OUT, &opted_out);
        self.harness_apply(h, Moment::TurnOff);
        self.render_settings();
    }

    /// Settings: "Add" on one app (one removed by hand, or taken out before): in again, and
    /// into its file now while hosting.
    pub(crate) fn agent_app_added(&mut self, id: &str) {
        let Some(h) = harness::find(id) else {
            return;
        };
        let opted_out: Vec<String> = self
            .stored_list(&defs::AGENT_MCP_OPTED_OUT)
            .into_iter()
            .filter(|o| o != id)
            .collect();
        self.store_list(&defs::AGENT_MCP_OPTED_OUT, &opted_out);
        if self.hosting.state.serving() {
            self.harness_apply(h, Moment::TurnOn);
        }
        self.render_settings();
    }

    /// Settings: the switch.
    pub(crate) fn agents_toggled(&mut self, on: bool) {
        self.write_setting(defs::AGENT_MCP_ENABLED.key, if on { "1" } else { "0" });
    }

    /// Settings: a Copy button.
    pub(crate) fn agents_copy(&mut self, text: &str) {
        if let Some(window) = self.w.window.upgrade() {
            window.invoke_copy_text(text.into());
        }
    }

    /// `--smoke gallery`: three agent apps in the run's scratch home, then hosting on (which
    /// adds wubble to the two it writes).
    pub(super) fn gallery_agents(&mut self) {
        if let Some(home) = self.harness_home() {
            let _ = std::fs::create_dir_all(home.join(".codex"));
            let _ = std::fs::create_dir_all(home.join(".config/Code/User"));
            let _ = std::fs::write(home.join(".claude.json"), "{}\n");
        }
        self.agents_toggled(true);
    }

    /// The Agents (MCP) group: read now, so an app installed since shows up.
    pub(super) fn render_agents(&self, window: &AppWindow) {
        let state = &self.hosting.state;
        // the switch is the hosting machine's state (one owner), not the stored setting
        let on = state.switch_on();
        let remote_port = self.debug_port();
        let opted_out = self.stored_list(&defs::AGENT_MCP_OPTED_OUT);
        let home = self.harness_home();
        let found = self.harnesses_found();
        let command = self.mcp_command();
        let records = self.records();
        let apps: Vec<AgentAppItem> = found
            .iter()
            .map(|h| {
                let file = home
                    .as_ref()
                    .and_then(|home| h.file(Os::current(), home, &exists));
                // owned: its entry is still what wubble recorded writing; foreign: one it did not
                let existing = if h.writable() {
                    file.as_ref()
                        .and_then(|f| std::fs::read_to_string(f).ok())
                        .and_then(|t| h.existing(&t).ok().flatten())
                } else {
                    None
                };
                let added = existing.is_some() && existing.as_ref() == records.get(h.id);
                let foreign = existing.is_some() && !added;
                let out = opted_out.iter().any(|o| o == h.id);
                let where_ = file.map(|f| f.display().to_string()).unwrap_or_default();
                let note = match (self.hosting.notes.get(h.id), h.writable(), added, out) {
                    (Some(e), ..) => format!("{where_}: {e}"),
                    (None, true, false, _) if foreign => format!(
                        "{where_}: has its own wubble entry (not managed by wubble, left as it is)"
                    ),
                    (None, false, ..) => format!(
                        "{where_}: wubble does not write this app's file (its format is not \
                         verified); copy the entry into it"
                    ),
                    (None, true, true, _) => format!("Added: {where_}"),
                    (None, true, false, true) => {
                        format!("Left out: turning hosting on does not add wubble here ({where_})")
                    }
                    (None, true, false, false) if on => format!("Not added: {where_}"),
                    (None, true, false, false) => format!("Will be added: {where_}"),
                };
                AgentAppItem {
                    id: h.id.into(),
                    name: h.name.into(),
                    note: note.into(),
                    added,
                    foreign,
                    writable: h.writable(),
                    opted_out: out,
                }
            })
            .collect();
        let targets: Vec<&str> = harness::targets(&found, &opted_out)
            .iter()
            .map(|h| h.name)
            .collect();
        let adds = if found.is_empty() {
            "No agent app found on this computer: copy the entry below into yours.".to_owned()
        } else if *state == Hosting::On {
            "wubble is in the apps marked Added; turning hosting off takes it out of them."
                .to_owned()
        } else if targets.is_empty() {
            "Turning this on adds wubble to no app: copy the entry below.".to_owned()
        } else {
            format!(
                "Turning this on adds wubble to: {}. Turning it off removes it again.",
                targets.join(", ")
            )
        };
        window.set_settings_agents(AgentsView {
            on,
            status: state.status(remote_port).into(),
            adds: adds.into(),
            command: harness::shell_line(&command).into(),
            entry: harness::generic_entry(&command).into(),
        });
        crate::bind::sync(window.get_settings_agent_apps(), apps, |m| {
            window.set_settings_agent_apps(m)
        });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wubble-harness-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_link_is_kept_and_its_target_written_with_its_mode() {
        let dir = scratch("link");
        let real = dir.join("real.json");
        std::fs::write(&real, "{}").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_file(&link, Some("{}"), true, "{\"a\": 1}").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "{\"a\": 1}");
        assert_eq!(mode(&real), 0o600);
        let backup = harness::backup_of(&real);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{}");
        assert_eq!(mode(&backup), 0o600);
        // a later write keeps the first backup
        write_file(&link, Some("{\"a\": 1}"), true, "{\"a\": 2}").unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_changed_since_it_was_read_is_not_written() {
        let dir = scratch("changed");
        let file = dir.join("c.json");
        std::fs::write(&file, "{\"theirs\": 1}").unwrap();
        let r = write_file(&file, Some("{}"), false, "{\"ours\": 1}");
        assert!(matches!(r, Err(WriteError::Changed)));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{\"theirs\": 1}");
        // and no temporary file is left
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        // a new file: 0600, no backup
        let new = dir.join("n.json");
        write_file(&new, None, true, "{}").unwrap();
        assert_eq!(mode(&new), 0o600);
        assert!(!harness::backup_of(&new).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
