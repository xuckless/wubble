//! Keybindings as data: chords, a default table, user overrides.
//!
//! No windowing dependency: the shell turns a key event into a [`Chord`]
//! (Slint `control`/`meta` map to `ctrl`/`cmd` per platform there) and asks
//! [`Keymap::action_for`]. Executing the action and forwarding the key to the
//! page is the router's job, driven by [`Route`].

use std::collections::HashMap;
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

const NAMED_KEYS: [(&str, Key); 23] = [
    ("tab", Key::Tab),
    ("enter", Key::Enter),
    ("return", Key::Enter),
    ("escape", Key::Escape),
    ("esc", Key::Escape),
    ("space", Key::Space),
    ("backspace", Key::Backspace),
    ("delete", Key::Delete),
    ("del", Key::Delete),
    ("left", Key::Left),
    ("right", Key::Right),
    ("up", Key::Up),
    ("down", Key::Down),
    ("home", Key::Home),
    ("end", Key::End),
    ("pageup", Key::PageUp),
    ("pgup", Key::PageUp),
    ("pagedown", Key::PageDown),
    ("pgdn", Key::PageDown),
    ("plus", Key::Char('+')),
    ("minus", Key::Char('-')),
    ("equal", Key::Char('=')),
    ("equals", Key::Char('=')),
];

fn parse_key(tok: &str) -> Result<Key, ChordError> {
    let lower = tok.to_lowercase();
    if let Some((_, k)) = NAMED_KEYS.iter().find(|(n, _)| *n == lower) {
        return Ok(*k);
    }
    let fn_key = lower.strip_prefix('f').and_then(|d| d.parse::<u8>().ok());
    if let Some(n) = fn_key.filter(|n| (1..=24).contains(n)) {
        return Ok(Key::F(n));
    }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(Key::Char(c)),
        _ => Err(ChordError::UnknownKey(tok.to_owned())),
    }
}

impl Chord {
    /// Case-insensitive, `+`-joined (`ctrl+shift+t`). Accepts aliases such as
    /// Control, Command, Meta, Super, Option, Esc, PgUp. The result is normalised.
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
                "ctrl" | "control" => Some(&mut c.ctrl),
                "alt" | "option" | "opt" => Some(&mut c.alt),
                "shift" => Some(&mut c.shift),
                "cmd" | "command" | "meta" | "super" => Some(&mut c.cmd),
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
            Key::Char('+') => write!(f, "Plus"),
            Key::Char(c) => write!(f, "{}", c.to_uppercase()),
            Key::F(n) => write!(f, "F{n}"),
            Key::Tab => write!(f, "Tab"),
            Key::Enter => write!(f, "Enter"),
            Key::Escape => write!(f, "Escape"),
            Key::Space => write!(f, "Space"),
            Key::Backspace => write!(f, "Backspace"),
            Key::Delete => write!(f, "Delete"),
            Key::Left => write!(f, "Left"),
            Key::Right => write!(f, "Right"),
            Key::Up => write!(f, "Up"),
            Key::Down => write!(f, "Down"),
            Key::Home => write!(f, "Home"),
            Key::End => write!(f, "End"),
            Key::PageUp => write!(f, "PageUp"),
            Key::PageDown => write!(f, "PageDown"),
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
/// Chromium (spaces on Alt+N / Ctrl+N, compact mode Mod+S, Mod+Alt+Left/Right).
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
    "Mod+S", ALL, Action::ToggleCompactMode, PageFirst;
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

#[derive(Debug, Clone)]
pub struct Keymap {
    by_chord: HashMap<Chord, (Action, Route)>,
    /// Binding order; the first chord of an action is its displayed shortcut.
    order: Vec<Chord>,
    conflicts: Vec<Conflict>,
}

impl Keymap {
    pub fn defaults(platform: Platform) -> Keymap {
        let mut km = Keymap {
            by_chord: HashMap::new(),
            order: Vec::new(),
            conflicts: Vec::new(),
        };
        for b in DEFAULTS.iter().filter(|b| b.platforms.contains(&platform)) {
            let text = expand(b.chord, platform);
            let chord =
                Chord::parse(&text).unwrap_or_else(|e| panic!("bad default chord {text:?}: {e}"));
            match km.by_chord.get(&chord) {
                Some(&(existing, _)) => km.conflicts.push(Conflict {
                    chord,
                    existing,
                    other: b.action,
                }),
                None => {
                    km.by_chord.insert(chord, (b.action, b.route));
                    km.order.push(chord);
                }
            }
        }
        km
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
            let old = self.by_chord.remove(&chord);
            self.order.retain(|c| *c != chord);
            if let Some(action) = o.action {
                if let Some((existing, _)) = old.filter(|(a, _)| *a != action) {
                    report.push(Conflict {
                        chord,
                        existing,
                        other: action,
                    });
                }
                self.by_chord.insert(chord, (action, default_route(action)));
                self.order.insert(0, chord);
            }
        }
        (self, report)
    }

    pub fn action_for(&self, chord: Chord) -> Option<(Action, Route)> {
        self.by_chord.get(&chord.normalize()).copied()
    }

    /// First binding of the action, for the command bar and menus.
    pub fn shortcut_for(&self, action: Action) -> Option<Chord> {
        self.order
            .iter()
            .find(|c| self.by_chord[*c].0 == action)
            .copied()
    }

    /// Chords bound twice in the default rows (empty for a valid table).
    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
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
            ("Command+Option+Right", "Alt+Cmd+Right"),
            ("Ctrl+Plus", "Ctrl+Plus"),
            ("f12", "F12"),
            ("Ctrl+PgDn", "Ctrl+PageDown"),
            ("Shift+Ctrl+Alt+Meta+Esc", "Ctrl+Alt+Shift+Cmd+Escape"),
            ("Ctrl+-", "Ctrl+-"),
        ] {
            assert_eq!(ch(input).to_string(), canon, "{input}");
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
            assert_eq!(Keymap::defaults(p).conflicts(), &[], "{p:?}");
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
    }

    #[test]
    fn every_action_is_bound_except_allow_list() {
        let unbound = [Action::NewSpace];
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
}
