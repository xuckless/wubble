//! Keybindings as data: chords, a default table, user overrides.
//!
//! No windowing dependency: the shell turns a key event into a [`Chord`]
//! (Slint `control`/`meta` map to `ctrl`/`cmd` per platform there) and asks
//! [`Keymap::action_for`]. Executing the action and forwarding the key to the
//! page is the router's job, driven by [`Route`].

use std::fmt;

use serde_json::{Value, json};

use crate::action::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Linux,
    Windows,
    Mac,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

// ------------------------------------------------------------------ chords

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// Lowercase base-layer character as produced (`=`, `+`, `a`, `1`).
    Char(char),
    F(u8),
    Tab,
    Enter,
    Escape,
    Space,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
    pub key: Key,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    Empty,
    /// Trailing or doubled `+` (write the plus key as `Plus`).
    EmptyPart,
    DuplicateModifier(String),
    NoKey,
    TwoKeys,
    UnknownKey(String),
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordError::Empty => write!(f, "empty chord"),
            ChordError::EmptyPart => write!(f, "empty part (spell the plus key as Plus)"),
            ChordError::DuplicateModifier(m) => write!(f, "duplicate modifier {m}"),
            ChordError::NoKey => write!(f, "no key"),
            ChordError::TwoKeys => write!(f, "more than one key"),
            ChordError::UnknownKey(k) => write!(f, "unknown key {k}"),
        }
    }
}

impl std::error::Error for ChordError {}

/// Canonical names, as printed by `Display`; parsing is case-insensitive.
/// `+` is spelled `Plus` because `+` separates parts.
const NAMED_KEYS: [(&str, Key); 15] = [
    ("Tab", Key::Tab),
    ("Enter", Key::Enter),
    ("Escape", Key::Escape),
    ("Space", Key::Space),
    ("Backspace", Key::Backspace),
    ("Delete", Key::Delete),
    ("Left", Key::Left),
    ("Right", Key::Right),
    ("Up", Key::Up),
    ("Down", Key::Down),
    ("Home", Key::Home),
    ("End", Key::End),
    ("PageUp", Key::PageUp),
    ("PageDown", Key::PageDown),
    ("Plus", Key::Char('+')),
];

fn parse_key(tok: &str) -> Result<Key, ChordError> {
    if let Some((_, k)) = NAMED_KEYS.iter().find(|(n, _)| n.eq_ignore_ascii_case(tok)) {
        return Ok(*k);
    }
    let fn_key = tok
        .strip_prefix(['f', 'F'])
        .and_then(|d| d.parse::<u8>().ok());
    if let Some(n) = fn_key.filter(|n| (1..=24).contains(n)) {
        return Ok(Key::F(n));
    }
    let mut chars = tok.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(Key::Char(c)),
        _ => Err(ChordError::UnknownKey(tok.to_owned())),
    }
}

impl Chord {
    /// Case-insensitive, `+`-joined (`ctrl+shift+t`); names are those `Display`
    /// prints. The result is normalised.
    pub fn parse(s: &str) -> Result<Chord, ChordError> {
        if s.trim().is_empty() {
            return Err(ChordError::Empty);
        }
        let mut c = Chord {
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
            key: Key::Tab,
        };
        let mut key = None;
        for part in s.split('+') {
            let tok = part.trim();
            if tok.is_empty() {
                return Err(ChordError::EmptyPart);
            }
            let flag = match tok.to_lowercase().as_str() {
                "ctrl" => Some(&mut c.ctrl),
                "alt" => Some(&mut c.alt),
                "shift" => Some(&mut c.shift),
                "cmd" => Some(&mut c.cmd),
                _ => None,
            };
            match flag {
                Some(f) if *f => return Err(ChordError::DuplicateModifier(tok.to_owned())),
                Some(f) => *f = true,
                None if key.is_some() => return Err(ChordError::TwoKeys),
                None => key = Some(parse_key(tok)?),
            }
        }
        c.key = key.ok_or(ChordError::NoKey)?;
        Ok(c.normalize())
    }

    /// Letters are lowercased; shift is dropped for non-alphanumeric
    /// characters because the produced character already encodes it.
    pub fn normalize(mut self) -> Chord {
        if let Key::Char(c) = self.key {
            self.key = Key::Char(c.to_lowercase().next().unwrap_or(c));
            if !c.is_alphanumeric() {
                self.shift = false;
            }
        }
        self
    }
}

impl fmt::Display for Chord {
    /// Canonical form `Ctrl+Alt+Shift+Cmd+Key`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.cmd, "Cmd"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match self.key {
            Key::F(n) => write!(f, "F{n}"),
            // Upper-case only when it stays one character (not `ß` -> `SS`).
            Key::Char(c) if c != '+' && c.to_uppercase().count() == 1 => {
                write!(f, "{}", c.to_uppercase())
            }
            Key::Char(c) if c != '+' => write!(f, "{c}"),
            key => {
                let (name, _) = NAMED_KEYS.iter().find(|(_, k)| *k == key).unwrap();
                write!(f, "{name}")
            }
        }
    }
}

// ------------------------------------------------------------------- table

/// Who sees the key first. Copied policy: Chromium `kReservedCommandIds` /
/// `IsReservedCommandOrKey` and Firefox `reserved="true"`.
///
/// - `Shell`: the shell always takes the chord; the page never sees it.
/// - `PageFirst`: the key goes to the page; if the page does not handle it
///   (CEF `OnKeyEvent`), the shell runs the action. Web apps may own it.
///
/// An override rebinding an action to a new chord takes the route of that
/// action's default row. In page fullscreen only `Fullscreen` stays `Shell`
/// (the router checks that, not this table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    Shell,
    PageFirst,
}

/// One default row. A leading `Mod+` in `chord` means Ctrl on Linux/Windows
/// and Cmd on macOS; per-platform differences are separate rows with their
/// own `platforms`.
pub struct Binding {
    pub chord: &'static str,
    pub platforms: &'static [Platform],
    pub action: Action,
    pub route: Route,
}

const ALL: &[Platform] = &[Platform::Linux, Platform::Windows, Platform::Mac];
const LW: &[Platform] = &[Platform::Linux, Platform::Windows];
const LINUX: &[Platform] = &[Platform::Linux];
const WIN: &[Platform] = &[Platform::Windows];
const MAC: &[Platform] = &[Platform::Mac];

macro_rules! rows {
    ($($chord:literal, $plat:ident, $action:expr, $route:ident;)*) => {
        &[$(Binding {
            chord: $chord,
            platforms: $plat,
            action: $action,
            route: Route::$route,
        }),*]
    };
}

/// Default shortcut set. Zen is preferred where it differs from Firefox and
/// Chromium (spaces on Alt+N / Ctrl+N, compact mode Mod+Alt+S, Mod+Alt+Left/Right).
/// Only the chord-to-command data is taken; no code is copied.
///
/// Sources (see THIRD_PARTY_NOTICES.md):
/// - Zen `src/browser/base/content/zen-keysets.inc.xhtml`, MPL-2.0
/// - Firefox `browser/base/content/browser-sets.inc.xhtml` and
///   `browser/locales/en-US/browser/browserSets.ftl`, MPL-2.0
/// - Chromium `chrome/browser/ui/accelerator_table.cc`, BSD-3-Clause
pub const DEFAULTS: &[Binding] = rows![
    "Mod+T", ALL, Action::NewTab, Shell;
    "Mod+W", ALL, Action::CloseTab, Shell;
    "Ctrl+F4", LW, Action::CloseTab, Shell;
    "Mod+Shift+T", ALL, Action::ReopenClosedTab, Shell;
    "Mod+N", ALL, Action::NewWindow, Shell;
    "Mod+L", ALL, Action::FocusAddress, PageFirst;
    "Alt+D", LW, Action::FocusAddress, PageFirst;
    "F6", LW, Action::FocusAddress, PageFirst;
    "Mod+K", ALL, Action::FocusAddress, PageFirst;
    "Mod+R", ALL, Action::Reload, PageFirst;
    "F5", LW, Action::Reload, PageFirst;
    "Mod+Shift+R", ALL, Action::HardReload, PageFirst;
    "Ctrl+F5", LW, Action::HardReload, PageFirst;
    "Shift+F5", LW, Action::HardReload, PageFirst;
    "Escape", LW, Action::Stop, PageFirst;
    "Cmd+.", MAC, Action::Stop, PageFirst;
    "Alt+Left", LW, Action::Back, PageFirst;
    "Ctrl+[", LW, Action::Back, PageFirst;
    "Cmd+Left", MAC, Action::Back, PageFirst;
    "Cmd+[", MAC, Action::Back, PageFirst;
    "Alt+Right", LW, Action::Forward, PageFirst;
    "Ctrl+]", LW, Action::Forward, PageFirst;
    "Cmd+Right", MAC, Action::Forward, PageFirst;
    "Cmd+]", MAC, Action::Forward, PageFirst;
    "Ctrl+Tab", ALL, Action::NextTab, Shell;
    "Ctrl+PageDown", ALL, Action::NextTab, Shell;
    "Ctrl+Shift+Tab", ALL, Action::PrevTab, Shell;
    "Ctrl+PageUp", ALL, Action::PrevTab, Shell;
    "Mod+1", ALL, Action::SelectTab(1), PageFirst;
    "Mod+2", ALL, Action::SelectTab(2), PageFirst;
    "Mod+3", ALL, Action::SelectTab(3), PageFirst;
    "Mod+4", ALL, Action::SelectTab(4), PageFirst;
    "Mod+5", ALL, Action::SelectTab(5), PageFirst;
    "Mod+6", ALL, Action::SelectTab(6), PageFirst;
    "Mod+7", ALL, Action::SelectTab(7), PageFirst;
    "Mod+8", ALL, Action::SelectTab(8), PageFirst;
    "Mod+9", ALL, Action::SelectLastTab, PageFirst;
    "Alt+1", LW, Action::SelectSpace(1), Shell;
    "Alt+2", LW, Action::SelectSpace(2), Shell;
    "Alt+3", LW, Action::SelectSpace(3), Shell;
    "Alt+4", LW, Action::SelectSpace(4), Shell;
    "Alt+5", LW, Action::SelectSpace(5), Shell;
    "Alt+6", LW, Action::SelectSpace(6), Shell;
    "Alt+7", LW, Action::SelectSpace(7), Shell;
    "Alt+8", LW, Action::SelectSpace(8), Shell;
    "Alt+9", LW, Action::SelectSpace(9), Shell;
    "Alt+0", LW, Action::SelectSpace(10), Shell;
    "Ctrl+1", MAC, Action::SelectSpace(1), Shell;
    "Ctrl+2", MAC, Action::SelectSpace(2), Shell;
    "Ctrl+3", MAC, Action::SelectSpace(3), Shell;
    "Ctrl+4", MAC, Action::SelectSpace(4), Shell;
    "Ctrl+5", MAC, Action::SelectSpace(5), Shell;
    "Ctrl+6", MAC, Action::SelectSpace(6), Shell;
    "Ctrl+7", MAC, Action::SelectSpace(7), Shell;
    "Ctrl+8", MAC, Action::SelectSpace(8), Shell;
    "Ctrl+9", MAC, Action::SelectSpace(9), Shell;
    "Ctrl+0", MAC, Action::SelectSpace(10), Shell;
    "Mod+Alt+Right", ALL, Action::NextSpace, Shell;
    "Mod+Alt+Left", ALL, Action::PrevSpace, Shell;
    "Mod+Alt+S", ALL, Action::ToggleCompactMode, PageFirst;
    "Ctrl+Alt+Z", LW, Action::ToggleSidebar, Shell;
    "Ctrl+Z", MAC, Action::ToggleSidebar, Shell;
    "Mod+F", ALL, Action::Find, PageFirst;
    "Mod+G", ALL, Action::FindNext, PageFirst;
    "F3", LW, Action::FindNext, PageFirst;
    "Mod+Shift+G", ALL, Action::FindPrev, PageFirst;
    "Shift+F3", LW, Action::FindPrev, PageFirst;
    "Mod+=", ALL, Action::ZoomIn, PageFirst;
    "Mod+Plus", ALL, Action::ZoomIn, PageFirst;
    "Mod+-", ALL, Action::ZoomOut, PageFirst;
    "Mod+0", ALL, Action::ZoomReset, PageFirst;
    "F12", LW, Action::DevTools, PageFirst;
    "Ctrl+Shift+I", LW, Action::DevTools, PageFirst;
    "Cmd+Alt+I", MAC, Action::DevTools, PageFirst;
    "Ctrl+U", LW, Action::ViewSource, PageFirst;
    "Cmd+Alt+U", MAC, Action::ViewSource, PageFirst;
    "F11", LW, Action::Fullscreen, Shell;
    "Cmd+Ctrl+F", MAC, Action::Fullscreen, Shell;
    "Mod+Shift+D", ALL, Action::TogglePinTab, PageFirst;
    "Mod+Shift+C", ALL, Action::CopyUrl, PageFirst;
    "Mod+P", ALL, Action::Print, PageFirst;
    // the design's split chord; Shift+\ types `|`, so the unsplit row names that
    "Mod+\\", ALL, Action::SplitNext, Shell;
    "Mod+|", ALL, Action::Unsplit, Shell;
    // Chromium and Firefox on macOS (Cmd+,); Linux and Windows take the same chord on Ctrl
    "Mod+,", ALL, Action::OpenSettings, Shell;
    // Chromium's chords; Cmd+H hides the app on macOS, so its rows differ
    "Ctrl+H", LW, Action::OpenHistory, Shell;
    "Cmd+Y", MAC, Action::OpenHistory, Shell;
    "Ctrl+Shift+O", LW, Action::OpenBookmarks, Shell;
    "Cmd+Alt+B", MAC, Action::OpenBookmarks, Shell;
    "Ctrl+J", LW, Action::OpenDownloads, Shell;
    "Cmd+Shift+J", MAC, Action::OpenDownloads, Shell;
    "Mod+D", ALL, Action::BookmarkPage, Shell;
    // Built-ins. Link hints take a modifier, never a bare letter, so typing in a field is safe.
    "Alt+F", ALL, Action::LinkHints, Shell;
    "Mod+Alt+R", ALL, Action::ToggleReader, Shell;
    "Mod+Shift+S", ALL, Action::ScreenshotRegion, Shell;
    "Ctrl+Q", LINUX, Action::Quit, Shell;
    "Ctrl+Shift+Q", WIN, Action::Quit, Shell;
    "Cmd+Q", MAC, Action::Quit, Shell;
];

fn expand(chord: &str, p: Platform) -> String {
    match chord.strip_prefix("Mod+") {
        Some(rest) => format!("{}+{rest}", if p == Platform::Mac { "Cmd" } else { "Ctrl" }),
        None => chord.to_owned(),
    }
}

fn default_route(action: Action) -> Route {
    DEFAULTS
        .iter()
        .find(|b| b.action == action)
        .map_or(Route::PageFirst, |b| b.route)
}

// ------------------------------------------------------------------ keymap

/// A chord that would run two different actions: `existing` was bound first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conflict {
    pub chord: Chord,
    pub existing: Action,
    pub other: Action,
}

/// A user change: bind `chord` to `action`, or unbind it (`None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Override {
    pub chord: Chord,
    pub action: Option<Action>,
}

/// Bound chords in binding order; the first chord of an action is its
/// displayed shortcut, and the first row for a chord wins.
#[derive(Debug, Clone)]
pub struct Keymap {
    rows: Vec<(Chord, Action, Route)>,
}

fn default_rows(platform: Platform) -> impl Iterator<Item = (Chord, Action, Route)> {
    DEFAULTS
        .iter()
        .filter(move |b| b.platforms.contains(&platform))
        .map(move |b| {
            let text = expand(b.chord, platform);
            let chord =
                Chord::parse(&text).unwrap_or_else(|e| panic!("bad default chord {text:?}: {e}"));
            (chord, b.action, b.route)
        })
}

/// Chords bound twice in the default rows of a platform (empty when valid).
pub fn default_conflicts(platform: Platform) -> Vec<Conflict> {
    let mut seen: Vec<(Chord, Action)> = Vec::new();
    let mut out = Vec::new();
    for (chord, action, _) in default_rows(platform) {
        match seen.iter().find(|(c, _)| *c == chord) {
            Some(&(_, existing)) => out.push(Conflict {
                chord,
                existing,
                other: action,
            }),
            None => seen.push((chord, action)),
        }
    }
    out
}

impl Keymap {
    pub fn defaults(platform: Platform) -> Keymap {
        Keymap {
            rows: default_rows(platform).collect(),
        }
    }

    /// Apply overrides in order; a later one beats an earlier one.
    pub fn with_overrides(self, overrides: &[Override]) -> Keymap {
        self.with_overrides_reporting(overrides).0
    }

    /// The conflicts the overrides would cause: chords they take over from a
    /// different action (a default or an earlier override).
    pub fn conflicts_with(&self, overrides: &[Override]) -> Vec<Conflict> {
        self.clone().with_overrides_reporting(overrides).1
    }

    fn with_overrides_reporting(mut self, overrides: &[Override]) -> (Keymap, Vec<Conflict>) {
        let mut report = Vec::new();
        for o in overrides {
            let chord = o.chord.normalize();
            let old = self.action_for(chord);
            self.rows.retain(|(c, _, _)| *c != chord);
            if let Some(action) = o.action {
                if let Some((existing, _)) = old.filter(|(a, _)| *a != action) {
                    report.push(Conflict {
                        chord,
                        existing,
                        other: action,
                    });
                }
                self.rows.insert(0, (chord, action, default_route(action)));
            }
        }
        (self, report)
    }

    pub fn action_for(&self, chord: Chord) -> Option<(Action, Route)> {
        let chord = chord.normalize();
        self.rows
            .iter()
            .find(|(c, _, _)| *c == chord)
            .map(|&(_, a, r)| (a, r))
    }

    /// First binding of the action, for the command bar and menus.
    pub fn shortcut_for(&self, action: Action) -> Option<Chord> {
        self.rows
            .iter()
            .find(|(_, a, _)| *a == action)
            .map(|&(c, _, _)| c)
    }
}

// ----------------------------------------------------------------- editor

/// One row of the keybinding editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingRow {
    pub action: Action,
    /// The shortcut it runs on now (overrides applied).
    pub chord: Option<Chord>,
    /// The shortcut of the default table.
    pub default: Option<Chord>,
    /// The user changed it.
    pub overridden: bool,
}

/// A row per action of [`Action::all`], in that order.
pub fn binding_rows(platform: Platform, overrides: &[Override]) -> Vec<BindingRow> {
    let defaults = Keymap::defaults(platform);
    let now = defaults.clone().with_overrides(overrides);
    Action::all()
        .into_iter()
        .map(|action| {
            let chord = now.shortcut_for(action);
            let default = defaults.shortcut_for(action);
            BindingRow {
                action,
                chord,
                default,
                overridden: chord != default,
            }
        })
        .collect()
}

/// A chord the editor accepts: one with Ctrl, Alt or Cmd, or a function key. A bare letter
/// would take the key from every text field.
pub fn bindable(chord: Chord) -> bool {
    chord.ctrl || chord.alt || chord.cmd || matches!(chord.key, Key::F(_))
}

/// The overrides after binding `chord` to `action` in place of the chords it runs on now, and
/// the action that ran on `chord` until then (the conflict the editor shows), if another did.
pub fn set_binding(
    overrides: &[Override],
    platform: Platform,
    action: Action,
    chord: Chord,
) -> (Vec<Override>, Option<Action>) {
    let chord = chord.normalize();
    let now = Keymap::defaults(platform).with_overrides(overrides);
    let previous = now
        .action_for(chord)
        .map(|(a, _)| a)
        .filter(|a| *a != action);
    // Without the action's own rebinding, the chords it still runs on are unbound.
    let mut next: Vec<Override> = overrides
        .iter()
        .filter(|o| o.action != Some(action) && o.chord != chord)
        .copied()
        .collect();
    let base = Keymap::defaults(platform).with_overrides(&next);
    next.extend(
        base.rows
            .iter()
            .filter(|(c, a, _)| *a == action && *c != chord)
            .map(|&(chord, _, _)| Override {
                chord,
                action: None,
            }),
    );
    next.push(Override {
        chord,
        action: Some(action),
    });
    (next, previous)
}

/// The overrides without the user's changes to `action`: it runs on its default chords again
/// (unless another action was bound to one of them since).
pub fn reset_binding(overrides: &[Override], platform: Platform, action: Action) -> Vec<Override> {
    let defaults: Vec<Chord> = default_rows(platform)
        .filter(|(_, a, _)| *a == action)
        .map(|(c, _, _)| c)
        .collect();
    overrides
        .iter()
        .filter(|o| {
            o.action != Some(action) && !(o.action.is_none() && defaults.contains(&o.chord))
        })
        .copied()
        .collect()
}

/// Recording a new chord for an action in the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    Idle,
    /// The next chord pressed is the action's.
    Armed(Action),
    /// `chord` runs `other` now: binding it takes it from `other`, once confirmed.
    Conflict {
        action: Action,
        chord: Chord,
        other: Action,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvent {
    Arm(Action),
    /// A key pressed while armed (the host routes every key here first while not `Idle`).
    Pressed(Chord),
    /// Take the chord from the other action.
    Confirm,
    /// Escape, a click elsewhere, or the page closing.
    Cancel,
}

/// The one transition of [`Capture`]; the effect is the binding to store, if any.
pub fn capture(
    state: Capture,
    event: CaptureEvent,
    keymap: &Keymap,
) -> (Capture, Option<(Action, Chord)>) {
    match (state, event) {
        (_, CaptureEvent::Arm(action)) => (Capture::Armed(action), None),
        (_, CaptureEvent::Cancel) => (Capture::Idle, None),
        (Capture::Armed(action), CaptureEvent::Pressed(chord)) => {
            let chord = chord.normalize();
            if !bindable(chord) {
                return (state, None);
            }
            match keymap.action_for(chord).map(|(a, _)| a) {
                Some(other) if other != action => (
                    Capture::Conflict {
                        action,
                        chord,
                        other,
                    },
                    None,
                ),
                _ => (Capture::Idle, Some((action, chord))),
            }
        }
        (Capture::Conflict { action, chord, .. }, CaptureEvent::Confirm) => {
            (Capture::Idle, Some((action, chord)))
        }
        (state, _) => (state, None),
    }
}

// ------------------------------------------------------------- persistence

/// `[["Ctrl+Shift+K","close-tab"],["Ctrl+B",null]]`; null unbinds.
pub fn overrides_to_json(overrides: &[Override]) -> String {
    let rows: Vec<Value> = overrides
        .iter()
        .map(|o| json!([o.chord.to_string(), o.action.map(|a| a.id())]))
        .collect();
    Value::Array(rows).to_string()
}

/// The overrides the stored `keys.overrides` entries hold (each one an [`overrides_to_json`]
/// list), in order, and how many entries did not parse (skipped, so a bad row never blocks
/// startup).
pub fn overrides_from_entries(entries: &[String]) -> (Vec<Override>, usize) {
    let mut bad = 0;
    let overrides = entries
        .iter()
        .filter_map(|entry| overrides_from_json(entry).inspect_err(|_| bad += 1).ok())
        .flatten()
        .collect();
    (overrides, bad)
}

/// What the editor stores: one entry, or none without overrides.
pub fn overrides_to_entries(overrides: &[Override]) -> Vec<String> {
    if overrides.is_empty() {
        vec![]
    } else {
        vec![overrides_to_json(overrides)]
    }
}

/// Entries with an unknown action id or an unparseable chord are skipped so
/// settings written by another app version still load. Malformed JSON errors.
pub fn overrides_from_json(s: &str) -> Result<Vec<Override>, serde_json::Error> {
    let rows: Vec<Value> = serde_json::from_str(s)?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let [chord, action] = row.as_array()?.as_slice() else {
                return None;
            };
            let chord = Chord::parse(chord.as_str()?).ok()?;
            let action = match action {
                Value::Null => None,
                v => Some(Action::from_id(v.as_str()?)?),
            };
            Some(Override { chord, action })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLATFORMS: [Platform; 3] = [Platform::Linux, Platform::Windows, Platform::Mac];

    fn ch(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    #[test]
    fn default_chords_round_trip() {
        for p in PLATFORMS {
            for b in DEFAULTS.iter().filter(|b| b.platforms.contains(&p)) {
                let c = ch(&expand(b.chord, p));
                assert_eq!(ch(&c.to_string()), c, "{}", b.chord);
            }
        }
    }

    #[test]
    fn parse_and_format_hand_cases() {
        for (input, canon) in [
            ("ctrl+shift+t", "Ctrl+Shift+T"),
            ("Cmd+Alt+Right", "Alt+Cmd+Right"),
            ("Ctrl+Plus", "Ctrl+Plus"),
            ("f12", "F12"),
            ("ctrl+pagedown", "Ctrl+PageDown"),
            ("Shift+Ctrl+Alt+Cmd+Escape", "Ctrl+Alt+Shift+Cmd+Escape"),
            ("Ctrl+-", "Ctrl+-"),
        ] {
            assert_eq!(ch(input).to_string(), canon, "{input}");
        }
    }

    #[test]
    fn every_named_key_round_trips() {
        for (name, key) in NAMED_KEYS {
            let c = ch(&format!("Ctrl+{name}"));
            assert_eq!(c.key, key);
            assert_eq!(c.to_string(), format!("Ctrl+{name}"));
        }
    }

    #[test]
    fn multi_char_uppercase_keys_round_trip() {
        // 'ß' upper-cases to "SS"; it must print as itself and parse back.
        let c = Chord {
            ctrl: true,
            alt: false,
            shift: false,
            cmd: false,
            key: Key::Char('ß'),
        };
        assert_eq!(c.to_string(), "Ctrl+ß");
        assert_eq!(ch(&c.to_string()), c);
        let ov = [Override {
            chord: c,
            action: Some(Action::Find),
        }];
        assert_eq!(overrides_from_json(&overrides_to_json(&ov)).unwrap(), ov);
    }

    #[test]
    fn compact_mode_does_not_shadow_save_page() {
        for p in PLATFORMS {
            let km = Keymap::defaults(p);
            let save = if p == Platform::Mac {
                "Cmd+S"
            } else {
                "Ctrl+S"
            };
            assert_eq!(km.action_for(ch(save)), None, "{p:?}");
            assert_eq!(
                km.action_for(ch(&expand("Mod+Alt+S", p))).map(|r| r.0),
                Some(Action::ToggleCompactMode)
            );
        }
    }

    #[test]
    fn bad_chords_are_rejected() {
        for bad in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+T",
            "Foo+T",
            "Ctrl+A+B",
            "Control+T",
            "Esc",
            "F0",
            "F25",
            "Ctrl",
            "Ctrl++",
        ] {
            assert!(Chord::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn normalisation() {
        assert_eq!(ch("Ctrl+Shift+="), ch("Ctrl+="));
        assert_eq!(ch("Ctrl+Shift+Plus"), ch("Ctrl+Plus"));
        assert_ne!(ch("Ctrl+="), ch("Ctrl+Plus"));
        assert_eq!(ch("Ctrl+T"), ch("ctrl+t"));
        assert_ne!(ch("Ctrl+Shift+T"), ch("Ctrl+T"));
        assert_ne!(ch("Ctrl+Shift+1"), ch("Ctrl+1"));
    }

    #[test]
    fn default_tables_have_no_conflicts() {
        for p in PLATFORMS {
            assert_eq!(default_conflicts(p), [], "{p:?}");
        }
    }

    #[test]
    fn mod_expands_per_platform() {
        let sc = |p, a| Keymap::defaults(p).shortcut_for(a).unwrap().to_string();
        assert_eq!(sc(Platform::Linux, Action::NewTab), "Ctrl+T");
        assert_eq!(sc(Platform::Windows, Action::NewTab), "Ctrl+T");
        assert_eq!(sc(Platform::Mac, Action::NewTab), "Cmd+T");
        assert_eq!(sc(Platform::Linux, Action::SelectSpace(1)), "Alt+1");
        assert_eq!(sc(Platform::Mac, Action::SelectSpace(1)), "Ctrl+1");
        assert_eq!(sc(Platform::Linux, Action::SelectTab(1)), "Ctrl+1");
        assert_eq!(sc(Platform::Mac, Action::SelectTab(1)), "Cmd+1");
        assert_eq!(sc(Platform::Windows, Action::Quit), "Ctrl+Shift+Q");
        assert_eq!(sc(Platform::Linux, Action::SplitNext), "Ctrl+\\");
        assert_eq!(sc(Platform::Mac, Action::SplitNext), "Cmd+\\");
        assert_eq!(sc(Platform::Linux, Action::Unsplit), "Ctrl+|");
    }

    #[test]
    fn every_action_is_bound_except_allow_list() {
        // Chromium binds no key to muting a tab either.
        let unbound = [
            Action::NewSpace,
            Action::ToggleMute,
            Action::OpenPasswords,
            Action::CyclePageTheme,
            Action::ScreenshotVisible,
            Action::ScreenshotFullPage,
        ];
        for p in PLATFORMS {
            let km = Keymap::defaults(p);
            for a in Action::all() {
                assert_eq!(
                    km.shortcut_for(a).is_none(),
                    unbound.contains(&a),
                    "{p:?} {a:?}"
                );
            }
        }
    }

    #[test]
    fn lookup_and_routes() {
        let km = Keymap::defaults(Platform::Linux);
        assert_eq!(
            km.action_for(ch("Ctrl+Shift+T")),
            Some((Action::ReopenClosedTab, Route::Shell))
        );
        assert_eq!(km.action_for(ch("Cmd+Shift+T")), None);
        assert_eq!(km.action_for(ch("Ctrl+T")).unwrap().1, Route::Shell);
        assert_eq!(
            km.action_for(ch("Ctrl+R")),
            Some((Action::Reload, Route::PageFirst))
        );
        assert_eq!(km.action_for(ch("Ctrl+Alt+Q")), None);
        assert_eq!(
            Keymap::defaults(Platform::Mac).action_for(ch("Cmd+Shift+T")),
            Some((Action::ReopenClosedTab, Route::Shell))
        );
    }

    #[test]
    fn builtin_chords_are_shell_chords() {
        for (p, hints, reader, shot) in [
            (Platform::Linux, "Alt+F", "Ctrl+Alt+R", "Ctrl+Shift+S"),
            (Platform::Mac, "Alt+F", "Cmd+Alt+R", "Cmd+Shift+S"),
        ] {
            let km = Keymap::defaults(p);
            for (chord, action) in [
                (hints, Action::LinkHints),
                (reader, Action::ToggleReader),
                (shot, Action::ScreenshotRegion),
            ] {
                assert_eq!(
                    km.action_for(ch(chord)),
                    Some((action, Route::Shell)),
                    "{p:?}"
                );
            }
        }
    }

    #[test]
    fn override_rebinds_unbinds_and_later_wins() {
        let base = Keymap::defaults(Platform::Linux);
        let ov = |c: &str, a| Override {
            chord: ch(c),
            action: a,
        };

        let km = base
            .clone()
            .with_overrides(&[ov("Ctrl+K", Some(Action::CloseTab))]);
        assert_eq!(km.action_for(ch("Ctrl+K")).unwrap().0, Action::CloseTab);
        assert_eq!(km.shortcut_for(Action::CloseTab), Some(ch("Ctrl+K")));
        assert_eq!(km.action_for(ch("Ctrl+W")).unwrap().0, Action::CloseTab);

        let km = base.clone().with_overrides(&[ov("Ctrl+T", None)]);
        assert_eq!(km.action_for(ch("Ctrl+T")), None);

        let km = base.clone().with_overrides(&[
            ov("Ctrl+B", Some(Action::Find)),
            ov("Ctrl+B", Some(Action::Reload)),
        ]);
        assert_eq!(km.action_for(ch("Ctrl+B")).unwrap().0, Action::Reload);

        // New chord takes the action's default route.
        let km = base
            .clone()
            .with_overrides(&[ov("Ctrl+B", Some(Action::NewTab))]);
        assert_eq!(km.action_for(ch("Ctrl+B")).unwrap().1, Route::Shell);

        let km = base.clone().with_overrides(&[]);
        for a in Action::all() {
            assert_eq!(km.shortcut_for(a), base.shortcut_for(a));
        }
    }

    #[test]
    fn conflicts_with_reports_shadowed_default() {
        let base = Keymap::defaults(Platform::Linux);
        let c = base.conflicts_with(&[
            Override {
                chord: ch("Ctrl+R"),
                action: Some(Action::Find),
            },
            Override {
                chord: ch("Ctrl+Q"),
                action: Some(Action::Quit),
            },
            Override {
                chord: ch("Ctrl+B"),
                action: Some(Action::Find),
            },
        ]);
        assert_eq!(
            c,
            [Conflict {
                chord: ch("Ctrl+R"),
                existing: Action::Reload,
                other: Action::Find
            }]
        );
    }

    #[test]
    fn overrides_json_round_trip_and_tolerance() {
        let ov = vec![
            Override {
                chord: ch("Ctrl+Shift+K"),
                action: Some(Action::CloseTab),
            },
            Override {
                chord: ch("Ctrl+B"),
                action: None,
            },
            Override {
                chord: ch("Alt+3"),
                action: Some(Action::SelectTab(3)),
            },
        ];
        assert_eq!(overrides_from_json(&overrides_to_json(&ov)).unwrap(), ov);
        assert_eq!(overrides_to_json(&[]), "[]");

        let json = r#"[["Ctrl+K","no-such"],["Bogus+K","close-tab"],["Ctrl+J","find"],[1],"x",["Ctrl+H"]]"#;
        assert_eq!(
            overrides_from_json(json).unwrap(),
            [Override {
                chord: ch("Ctrl+J"),
                action: Some(Action::Find)
            }]
        );
        assert!(overrides_from_json("{not json").is_err());
        assert!(overrides_from_json("{}").is_err());
    }

    #[test]
    fn page_chords_open_their_pages() {
        let linux = Keymap::defaults(Platform::Linux);
        let mac = Keymap::defaults(Platform::Mac);
        let action = |m: &Keymap, c: &str| m.action_for(ch(c)).map(|r| r.0);
        for (chord, want) in [
            ("Ctrl+H", Action::OpenHistory),
            ("Ctrl+Shift+O", Action::OpenBookmarks),
            ("Ctrl+J", Action::OpenDownloads),
            ("Ctrl+D", Action::BookmarkPage),
        ] {
            assert_eq!(ch(&ch(chord).to_string()), ch(chord));
            assert_eq!(action(&linux, chord), Some(want));
        }
        for (chord, want) in [
            ("Cmd+Y", Action::OpenHistory),
            ("Cmd+Alt+B", Action::OpenBookmarks),
            ("Cmd+Shift+J", Action::OpenDownloads),
            ("Cmd+D", Action::BookmarkPage),
        ] {
            assert_eq!(action(&mac, chord), Some(want));
        }
    }

    #[test]
    fn ctrl_comma_opens_settings() {
        let c = ch("Ctrl+,");
        assert_eq!(c.key, Key::Char(','));
        assert_eq!(ch(&c.to_string()), c);
        assert_eq!(
            Keymap::defaults(Platform::Linux).action_for(c).map(|r| r.0),
            Some(Action::OpenSettings)
        );
        assert_eq!(
            Keymap::defaults(Platform::Mac)
                .action_for(ch("Cmd+,"))
                .map(|r| r.0),
            Some(Action::OpenSettings)
        );
        let ov = [Override {
            chord: c,
            action: Some(Action::Find),
        }];
        assert_eq!(overrides_from_json(&overrides_to_json(&ov)).unwrap(), ov);
    }

    #[test]
    fn set_binding_replaces_the_actions_chords_and_names_the_previous_owner() {
        let p = Platform::Linux;
        let (ov, prev) = set_binding(&[], p, Action::CloseTab, ch("Ctrl+K"));
        assert_eq!(prev, Some(Action::FocusAddress));
        let km = Keymap::defaults(p).with_overrides(&ov);
        assert_eq!(km.action_for(ch("Ctrl+K")).unwrap().0, Action::CloseTab);
        assert_eq!(km.action_for(ch("Ctrl+W")), None, "the old chord is gone");
        assert_eq!(km.action_for(ch("Ctrl+F4")), None);
        assert_eq!(km.shortcut_for(Action::CloseTab), Some(ch("Ctrl+K")));
        // Rebinding again replaces the first rebinding.
        let (ov, prev) = set_binding(&ov, p, Action::CloseTab, ch("Ctrl+Alt+J"));
        assert_eq!(prev, None);
        let km = Keymap::defaults(p).with_overrides(&ov);
        assert_eq!(km.action_for(ch("Ctrl+K")).unwrap().0, Action::FocusAddress);
        assert_eq!(km.shortcut_for(Action::CloseTab), Some(ch("Ctrl+Alt+J")));
        // The chord the action already has is no conflict.
        assert_eq!(
            set_binding(&ov, p, Action::CloseTab, ch("Ctrl+Alt+J")).1,
            None
        );
    }

    #[test]
    fn reset_binding_restores_the_default() {
        let p = Platform::Linux;
        let (ov, _) = set_binding(&[], p, Action::CloseTab, ch("Ctrl+B"));
        let (ov, _) = set_binding(&ov, p, Action::Find, ch("Ctrl+Alt+J"));
        let ov = reset_binding(&ov, p, Action::CloseTab);
        let km = Keymap::defaults(p).with_overrides(&ov);
        assert_eq!(km.action_for(ch("Ctrl+W")).unwrap().0, Action::CloseTab);
        assert_eq!(km.action_for(ch("Ctrl+B")), None);
        assert_eq!(km.shortcut_for(Action::Find), Some(ch("Ctrl+Alt+J")));
        assert_eq!(reset_binding(&ov, p, Action::Find), []);
    }

    #[test]
    fn binding_rows_cover_every_action_and_mark_changes() {
        let p = Platform::Linux;
        let (ov, _) = set_binding(&[], p, Action::Reload, ch("Ctrl+B"));
        let rows = binding_rows(p, &ov);
        assert_eq!(
            rows.iter().map(|r| r.action).collect::<Vec<_>>(),
            Action::all()
        );
        let reload = rows.iter().find(|r| r.action == Action::Reload).unwrap();
        assert!(reload.overridden);
        assert_eq!(
            (reload.chord, reload.default),
            (Some(ch("Ctrl+B")), Some(ch("Ctrl+R")))
        );
        assert_eq!(rows.iter().filter(|r| r.overridden).count(), 1);
        assert!(binding_rows(p, &[]).iter().all(|r| !r.overridden));
    }

    #[test]
    fn stored_entries_decode_in_order_and_skip_bad_ones() {
        let a = [Override {
            chord: ch("Ctrl+B"),
            action: Some(Action::Find),
        }];
        let b = [Override {
            chord: ch("Ctrl+B"),
            action: None,
        }];
        let entries = vec![overrides_to_json(&a), "{bad".into(), overrides_to_json(&b)];
        let (ov, bad) = overrides_from_entries(&entries);
        assert_eq!(bad, 1);
        assert_eq!(ov, [a[0], b[0]]);
        assert_eq!(overrides_to_entries(&[]), Vec::<String>::new());
        assert_eq!(overrides_from_entries(&overrides_to_entries(&a)).0, a);
    }

    #[test]
    fn capture_records_confirms_and_cancels() {
        let km = Keymap::defaults(Platform::Linux);
        let (s, out) = capture(Capture::Idle, CaptureEvent::Arm(Action::Find), &km);
        assert_eq!((s, out), (Capture::Armed(Action::Find), None));
        // A bare letter is not taken.
        let (s, out) = capture(s, CaptureEvent::Pressed(ch("K")), &km);
        assert_eq!((s, out), (Capture::Armed(Action::Find), None));
        let (s, out) = capture(s, CaptureEvent::Pressed(ch("Ctrl+Alt+J")), &km);
        assert_eq!(
            (s, out),
            (Capture::Idle, Some((Action::Find, ch("Ctrl+Alt+J"))))
        );
        // A chord another action runs on asks first.
        let (s, _) = capture(s, CaptureEvent::Arm(Action::Find), &km);
        let (s, out) = capture(s, CaptureEvent::Pressed(ch("Ctrl+R")), &km);
        assert_eq!(
            s,
            Capture::Conflict {
                action: Action::Find,
                chord: ch("Ctrl+R"),
                other: Action::Reload
            }
        );
        assert_eq!(out, None);
        let (done, out) = capture(s, CaptureEvent::Confirm, &km);
        assert_eq!(
            (done, out),
            (Capture::Idle, Some((Action::Find, ch("Ctrl+R"))))
        );
        assert_eq!(capture(s, CaptureEvent::Cancel, &km), (Capture::Idle, None));
        // Pressing keys while idle does nothing.
        assert_eq!(
            capture(Capture::Idle, CaptureEvent::Pressed(ch("Ctrl+Alt+J")), &km),
            (Capture::Idle, None)
        );
    }
}
