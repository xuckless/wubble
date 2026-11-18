//! Where profiles live, and reading one. The only module that touches the file system.
//!
//! Roots: Zen docs (`~/.zen`, `%APPDATA%/zen`, flatpak `~/.var/app/app.zen_browser.zen`),
//! Zen >= 1.18.6b follows XDG (`~/.config/zen`), Firefox `~/.mozilla/firefox` and (Firefox 147)
//! `~/.config/mozilla/firefox`, plus the macOS/Windows equivalents. The macOS Zen path has no
//! first-party doc (third-party samples and zephium-browser/Zephium
//! `crates/zephium-import/src/firefox.rs` `roots()` agree) and is unverified on real hardware.
//! `profiles.ini` parsing follows mozilla-firefox/firefox
//! `toolkit/profile/nsToolkitProfileService.cpp` (`Profile<N>`, `Install<hash>.Default`) and
//! zephium-browser/Zephium `ini_profiles` (MPL-2.0). Arc paths: mozilla-firefox/firefox
//! `ChromeProfileMigrator.sys.mjs` `_getSidebarPath` (macOS) and Zephium `arc.rs` (Windows).

use crate::{ImportError, Imported, Source, arc, containers, places, session};
use rusqlite::Connection;
use std::path::{Component, Path, PathBuf};
use wubble_core::space::Container;

/// Largest session / sidebar file we will read into memory.
const MAX_FILE: u64 = 32 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

/// Which environment directory a table path is relative to.
#[derive(Debug, Clone, Copy)]
enum Base {
    Home,
    AppData,
    LocalAppData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    pub os: Os,
    pub home: PathBuf,
    /// `%APPDATA%` (Windows).
    pub app_data: Option<PathBuf>,
    /// `%LOCALAPPDATA%` (Windows).
    pub local_app_data: Option<PathBuf>,
}

impl Env {
    pub fn current() -> Option<Env> {
        let var = |k| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
        };
        let os = if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        };
        Some(Env {
            os,
            home: var("HOME").or_else(|| var("USERPROFILE"))?,
            app_data: var("APPDATA"),
            local_app_data: var("LOCALAPPDATA"),
        })
    }

    fn base(&self, b: Base) -> Option<&Path> {
        match b {
            Base::Home => Some(&self.home),
            Base::AppData => self.app_data.as_deref(),
            Base::LocalAppData => self.local_app_data.as_deref(),
        }
    }

    fn resolve(&self, os: Os, base: Base, rel: &str) -> Option<PathBuf> {
        (self.os == os).then_some(())?;
        Some(self.base(base)?.join(rel))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Zen,
    Firefox,
}

impl From<Browser> for Source {
    fn from(b: Browser) -> Source {
        match b {
            Browser::Zen => Source::Zen,
            Browser::Firefox => Source::Firefox,
        }
    }
}

/// Directories holding `profiles.ini`.
const ROOTS: &[(Browser, Os, Base, &str)] = &[
    (Browser::Zen, Os::Linux, Base::Home, ".zen"),
    (Browser::Zen, Os::Linux, Base::Home, ".config/zen"),
    (
        Browser::Zen,
        Os::Linux,
        Base::Home,
        ".var/app/app.zen_browser.zen/.zen",
    ),
    (
        Browser::Zen,
        Os::Linux,
        Base::Home,
        ".var/app/app.zen_browser.zen/zen",
    ),
    (Browser::Firefox, Os::Linux, Base::Home, ".mozilla/firefox"),
    (
        Browser::Firefox,
        Os::Linux,
        Base::Home,
        ".config/mozilla/firefox",
    ),
    (
        Browser::Firefox,
        Os::Linux,
        Base::Home,
        ".var/app/org.mozilla.firefox/.mozilla/firefox",
    ),
    (
        Browser::Firefox,
        Os::Linux,
        Base::Home,
        "snap/firefox/common/.mozilla/firefox",
    ),
    (
        Browser::Zen,
        Os::MacOs,
        Base::Home,
        "Library/Application Support/zen",
    ),
    (
        Browser::Firefox,
        Os::MacOs,
        Base::Home,
        "Library/Application Support/Firefox",
    ),
    (Browser::Zen, Os::Windows, Base::AppData, "zen"),
    (
        Browser::Firefox,
        Os::Windows,
        Base::AppData,
        "Mozilla/Firefox",
    ),
];

/// `StorableSidebar.json` locations.
const ARC_FILES: &[(Os, Base, &str)] = &[
    (
        Os::MacOs,
        Base::Home,
        "Library/Application Support/Arc/StorableSidebar.json",
    ),
    (
        Os::Windows,
        Base::LocalAppData,
        "Packages/TheBrowserCompany.Arc_ttt1ap7aakyb4/LocalCache/Local/Arc/StorableSidebar.json",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniProfile {
    pub name: String,
    pub path: String,
    pub relative: bool,
    pub default: bool,
}

/// `profiles.ini`: `Profile<N>` sections (`Name`, `Path`, `IsRelative`, `Default`) and
/// `Install<hash>` sections whose `Default=<path>` names an install's profile.
pub fn parse_profiles_ini(text: &str) -> Vec<IniProfile> {
    let mut profiles: Vec<IniProfile> = Vec::new();
    let mut install_defaults: Vec<&str> = Vec::new();
    let (mut section, mut cur) = ("", None::<IniProfile>);
    let flush = |cur: &mut Option<IniProfile>, profiles: &mut Vec<IniProfile>| {
        profiles.extend(cur.take().filter(|p| !p.path.is_empty()));
    };
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&mut cur, &mut profiles);
            section = name;
            if name.starts_with("Profile") {
                cur = Some(IniProfile {
                    name: String::new(),
                    path: String::new(),
                    relative: true,
                    default: false,
                });
            }
        } else if let Some((k, v)) = line.split_once('=') {
            match (section.starts_with("Install"), k, &mut cur) {
                (true, "Default", _) => install_defaults.push(v),
                (_, "Name", Some(p)) => p.name = v.to_owned(),
                (_, "Path", Some(p)) => p.path = v.to_owned(),
                (_, "IsRelative", Some(p)) => p.relative = v != "0",
                (_, "Default", Some(p)) => p.default = v == "1",
                _ => {}
            }
        }
    }
    flush(&mut cur, &mut profiles);
    for p in &mut profiles {
        p.default |= install_defaults.contains(&p.path.as_str());
    }
    profiles
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundProfile {
    pub browser: Browser,
    pub name: String,
    pub dir: PathBuf,
    pub default: bool,
}

/// Every profile listed by an existing `profiles.ini` under the roots of `env.os`.
pub fn find_profiles(env: &Env) -> Vec<FoundProfile> {
    let mut out = Vec::new();
    for &(browser, os, base, rel) in ROOTS {
        let Some(root) = env.resolve(os, base, rel) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(root.join("profiles.ini")) else {
            continue;
        };
        for p in parse_profiles_ini(&text) {
            let path = Path::new(&p.path);
            // A relative path must stay inside the root.
            if p.relative
                && path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                continue;
            }
            let dir = if p.relative {
                root.join(path)
            } else {
                path.to_path_buf()
            };
            if dir.is_dir() && !out.iter().any(|f: &FoundProfile| f.dir == dir) {
                out.push(FoundProfile {
                    browser,
                    name: p.name,
                    dir,
                    default: p.default,
                });
            }
        }
    }
    out
}

pub fn find_arc(env: &Env) -> Option<PathBuf> {
    ARC_FILES
        .iter()
        .filter_map(|&(os, base, rel)| env.resolve(os, base, rel))
        .find(|p| p.is_file())
}

fn read_capped(path: &Path) -> Result<Option<Vec<u8>>, ImportError> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_FILE => Err(ImportError::Unreadable("file too large")),
        Ok(_) => Ok(Some(std::fs::read(path)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Copy `places.sqlite` and its `-wal` to a temp dir, let SQLite replay the WAL there and
/// switch the copy to rollback-journal mode, then parse the resulting single file. The
/// browser may hold the original open; it is never opened by us.
pub fn snapshot_places(dir: &Path) -> Result<places::Places, ImportError> {
    let db = dir.join("places.sqlite");
    if !db.is_file() {
        return Ok(places::Places::default());
    }
    let tmp = tempfile::tempdir()?;
    let copy = tmp.path().join("places.sqlite");
    let wal = dir.join("places.sqlite-wal");
    let wal_copy = tmp.path().join("places.sqlite-wal");
    // The browser may checkpoint between the two copies; if the WAL changed while we
    // copied, take both again (bounded, no waiting).
    let stamp = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    for _ in 0..3 {
        let before = stamp(&wal);
        std::fs::copy(&db, &copy)?;
        if before.is_some() && std::fs::copy(&wal, &wal_copy).is_err() {
            continue;
        }
        if stamp(&wal) == before {
            break;
        }
    }
    {
        let conn = Connection::open(&copy)?;
        conn.query_row("PRAGMA journal_mode=DELETE", [], |_| Ok(()))?;
    }
    places::parse(&std::fs::read(&copy)?)
}

const FIREFOX_SESSIONS: &[&str] = &[
    "sessionstore-backups/recovery.jsonlz4",
    "sessionstore.jsonlz4",
    "sessionstore-backups/recovery.baklz4",
    "sessionstore-backups/previous.jsonlz4",
];

pub fn read_profile(p: &FoundProfile) -> Result<Imported, ImportError> {
    let containers: Vec<Container> = match read_capped(&p.dir.join("containers.json"))? {
        Some(b) => containers::parse(&b)?,
        None => Vec::new(),
    };
    // Zen keeps spaces, essentials and folders in its own file; fall back to the plain
    // Firefox session (a Zen profile that never ran the session manager).
    let zen = match p.browser {
        Browser::Zen => read_capped(&p.dir.join("zen-sessions.jsonlz4"))?,
        Browser::Firefox => None,
    };
    // A session file that is present but unusable (truncated, empty, encrypted) is a
    // warning, not a failure: bookmarks, history and containers still import.
    let mut warnings = Vec::new();
    let mut layout = None;
    if let Some(b) = zen {
        match session::parse_zen(&b) {
            Ok(l) => layout = Some(l),
            Err(e) => warnings.push(format!("zen-sessions.jsonlz4: {e}")),
        }
    }
    for name in FIREFOX_SESSIONS {
        if layout.is_some() {
            break;
        }
        let Some(b) = read_capped(&p.dir.join(name))? else {
            continue;
        };
        match session::parse_firefox(&b) {
            Ok(l) => layout = Some(l),
            Err(e) => warnings.push(format!("{name}: {e}")),
        }
    }
    let pl = snapshot_places(&p.dir)?;
    let mut imported = Imported::new(
        p.browser.into(),
        containers,
        layout.unwrap_or_default(),
        pl.bookmarks,
        pl.history,
    );
    imported.warnings = warnings;
    Ok(imported)
}

pub fn read_arc(path: &Path) -> Result<Imported, ImportError> {
    let bytes =
        read_capped(path)?.ok_or(ImportError::Unreadable("StorableSidebar.json missing"))?;
    Ok(Imported::new(
        Source::Arc,
        Vec::new(),
        arc::parse(&bytes)?,
        Vec::new(),
        Vec::new(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mozlz4;
    use std::fs;

    #[test]
    fn ini_relative_absolute_default_and_missing_path() {
        let ini = "\
[Install4F96D1932A9F858E]
Default=Profiles/abc.default-release
Locked=1

[Profile2]
Name=other
IsRelative=0
Path=/abs/other

[Profile1]
Name=default-release
IsRelative=1
Path=Profiles/abc.default-release

[Profile0]
Name=nopath
IsRelative=1

[Profile3]
Name=flagged
Path=Profiles/flagged
Default=1

[General]
StartWithLastProfile=1
";
        let got = parse_profiles_ini(ini);
        let v: Vec<_> = got
            .iter()
            .map(|p| (p.name.as_str(), p.path.as_str(), p.relative, p.default))
            .collect();
        assert_eq!(
            v,
            [
                ("other", "/abs/other", false, false),
                (
                    "default-release",
                    "Profiles/abc.default-release",
                    true,
                    true
                ),
                ("flagged", "Profiles/flagged", true, true),
            ]
        );
    }

    fn env(dir: &Path, os: Os) -> Env {
        Env {
            os,
            home: dir.to_path_buf(),
            app_data: Some(dir.join("AppData/Roaming")),
            local_app_data: Some(dir.join("AppData/Local")),
        }
    }

    fn make_root(dir: &Path, rel: &str, profile: &str) {
        let root = dir.join(rel);
        fs::create_dir_all(root.join("Profiles").join(profile)).unwrap();
        fs::write(
            root.join("profiles.ini"),
            format!(
                "[Profile0]\nName={profile}\nIsRelative=1\nPath=Profiles/{profile}\nDefault=1\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn every_root_row_is_found_on_its_own_os() {
        for &(browser, os, base, rel) in ROOTS {
            let dir = tempfile::tempdir().unwrap();
            let e = env(dir.path(), os);
            let root = e.resolve(os, base, rel).unwrap();
            make_root(&root, "", "p1");
            let found = find_profiles(&e);
            assert_eq!(found.len(), 1, "{rel}");
            assert_eq!(found[0].browser, browser);
            assert_eq!(found[0].dir, root.join("Profiles/p1"));
            // the same layout on another OS is not probed
            let other = Env {
                os: if os == Os::Linux {
                    Os::MacOs
                } else {
                    Os::Linux
                },
                ..e
            };
            assert!(find_profiles(&other).is_empty());
        }
    }

    #[test]
    fn path_escaping_the_root_is_rejected_and_absolute_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let e = env(dir.path(), Os::Linux);
        let root = dir.path().join(".zen");
        let abs = dir.path().join("elsewhere");
        fs::create_dir_all(&abs).unwrap();
        fs::create_dir_all(dir.path().join("secret")).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("profiles.ini"),
            format!(
                "[Profile0]\nName=esc\nIsRelative=1\nPath=../secret\n\n[Profile1]\nName=abs\nIsRelative=0\nPath={}\n",
                abs.display()
            ),
        )
        .unwrap();
        let found = find_profiles(&e);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "abs");
        assert_eq!(found[0].dir, abs);
    }

    #[test]
    fn arc_paths() {
        let dir = tempfile::tempdir().unwrap();
        for (os, base, rel) in ARC_FILES.iter().copied() {
            let e = env(dir.path(), os);
            assert_eq!(find_arc(&e), None);
            let path = e.resolve(os, base, rel).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "{}").unwrap();
            assert_eq!(find_arc(&e), Some(path));
        }
        assert_eq!(find_arc(&env(dir.path(), Os::Linux)), None);
    }

    #[test]
    fn snapshot_sees_uncheckpointed_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("places.sqlite");
        let live = Connection::open(&path).unwrap();
        live.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()))
            .unwrap();
        live.query_row("PRAGMA wal_autocheckpoint=0", [], |_| Ok(()))
            .unwrap();
        live.execute_batch(
            "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT, title TEXT, hidden INTEGER DEFAULT 0);
             CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, place_id INTEGER, visit_date INTEGER, visit_type INTEGER);
             CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title TEXT, dateAdded INTEGER, guid TEXT);
             INSERT INTO moz_places (id, url, title) VALUES (1, 'https://wal.example/', 'W');
             INSERT INTO moz_historyvisits (place_id, visit_date, visit_type) VALUES (1, 5000, 1);",
        )
        .unwrap();
        // the connection stays open: rows live only in the -wal file
        assert!(dir.path().join("places.sqlite-wal").is_file());
        let p = snapshot_places(dir.path()).unwrap();
        assert_eq!(p.history.len(), 1);
        assert_eq!(p.history[0].url, "https://wal.example/");
        drop(live);
    }

    #[test]
    fn read_profile_zen_and_firefox_fallback() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("containers.json"),
            include_bytes!("../tests/fixtures/containers.json"),
        )
        .unwrap();
        fs::write(
            dir.path().join("zen-sessions.jsonlz4"),
            mozlz4::encode(include_bytes!("../tests/fixtures/zen_session.json")),
        )
        .unwrap();
        let p = FoundProfile {
            browser: Browser::Zen,
            name: "x".into(),
            dir: dir.path().to_path_buf(),
            default: true,
        };
        let i = read_profile(&p).unwrap();
        assert_eq!(i.source, Source::Zen);
        assert_eq!(i.containers.len(), 5);
        assert_eq!(i.spaces.len(), 2);
        assert!(i.bookmarks.is_empty() && i.history.is_empty());

        // no zen file: the Firefox session is used
        fs::remove_file(dir.path().join("zen-sessions.jsonlz4")).unwrap();
        fs::create_dir(dir.path().join("sessionstore-backups")).unwrap();
        fs::write(
            dir.path().join("sessionstore-backups/recovery.jsonlz4"),
            mozlz4::encode(include_bytes!("../tests/fixtures/firefox_session.json")),
        )
        .unwrap();
        let i = read_profile(&p).unwrap();
        assert_eq!(i.spaces.len(), 1);
        assert_eq!(i.tabs.len(), 4);
    }

    #[test]
    fn bad_session_files_warn_and_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("containers.json"),
            include_bytes!("../tests/fixtures/containers.json"),
        )
        .unwrap();
        fs::write(dir.path().join("zen-sessions.jsonlz4"), b"").unwrap();
        fs::write(
            dir.path().join("sessionstore.jsonlz4"),
            mozlz4::encode(include_bytes!("../tests/fixtures/firefox_session.json")),
        )
        .unwrap();
        let p = FoundProfile {
            browser: Browser::Zen,
            name: "x".into(),
            dir: dir.path().to_path_buf(),
            default: true,
        };
        let i = read_profile(&p).unwrap();
        assert_eq!(i.containers.len(), 5);
        assert_eq!(i.tabs.len(), 4);
        assert_eq!(i.warnings.len(), 1);

        // every session file unusable: still Ok, with no layout
        fs::remove_file(dir.path().join("sessionstore.jsonlz4")).unwrap();
        let i = read_profile(&p).unwrap();
        assert_eq!(i.containers.len(), 5);
        assert!(i.tabs.is_empty());
        assert_eq!(i.warnings.len(), 1);
    }

    #[test]
    fn read_arc_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("StorableSidebar.json");
        assert!(read_arc(&f).is_err());
        fs::write(&f, include_bytes!("../tests/fixtures/arc_sidebar.json")).unwrap();
        let i = read_arc(&f).unwrap();
        assert_eq!(i.source, Source::Arc);
        assert!(i.containers.is_empty());
        assert_eq!(i.tabs.len(), 8);
    }
}
