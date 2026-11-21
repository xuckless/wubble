//! Where a self-managed install lives, and what a launch does about a staged update or a
//! version that keeps failing: pure decisions, executed by the binary's `updater::install`.
//!
//! Linux: `<data>/wubble/versions/<version>/` holds a whole bundle (wubble, wubble-helper, CEF),
//! `<data>/wubble/current` links to one of them and is switched by a rename, and the launcher
//! `~/.local/bin/wubble` links to `current/wubble`. macOS: `wubble.app` in `/Applications` or
//! `~/Applications`, replaced as a whole at the next launch. Both keep their state in
//! `<data>/wubble/updates/` (`launch.json`, the staging area).
//!
//! The launch record follows Sparkle's and Zed's model only in outline (stage now, swap when
//! the app is not running, on its next start); the rollback after repeated failed starts is
//! wubble's own. No code copied.

use std::path::{Path, PathBuf};

use semver::Version;
use serde_json::{Value, json};

/// Starts of a new version that may end before it is healthy; one more and the launch falls
/// back to the version before it.
pub const MAX_ATTEMPTS: u32 = 3;
/// A start counts as healthy once wubble has run this long.
pub const HEALTHY_AFTER_SECS: u64 = 20;

/// The app bundle's name on macOS (`wubble-bundle`).
pub const MAC_APP: &str = "wubble.app";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Mac,
}

/// What the running binary is part of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed by `install.sh` (or the updater) in the layout above.
    Managed(Layout),
    /// Run from a cargo target directory.
    Development,
    /// Anywhere else: a system package, a copy by hand, a mounted disk image.
    Elsewhere,
}

/// A self-managed install's places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub os: Os,
    /// `<data>/wubble`: also the default profile's folder, so only the names below are ours.
    pub root: PathBuf,
    /// Linux: the running version's folder; macOS: the running `wubble.app`.
    pub bundle: PathBuf,
}

impl Layout {
    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }
    pub fn version_dir(&self, v: &Version) -> PathBuf {
        self.versions().join(v.to_string())
    }
    /// Linux: the link the launcher follows.
    pub fn current(&self) -> PathBuf {
        self.root.join("current")
    }
    pub fn updates(&self) -> PathBuf {
        self.root.join("updates")
    }
    pub fn record(&self) -> PathBuf {
        self.updates().join("launch.json")
    }
    /// Downloads and unpacked archives before they are checked.
    pub fn staging(&self) -> PathBuf {
        self.updates().join("staging")
    }
    /// macOS: the checked bundle waiting for the next launch.
    pub fn staged_app(&self) -> PathBuf {
        self.updates().join("staged").join(MAC_APP)
    }
    /// macOS: the bundle the last swap replaced, kept for a rollback.
    pub fn previous_app(&self) -> PathBuf {
        self.updates().join("previous").join(MAC_APP)
    }
    /// The program a restart runs: Linux the `current` link's, macOS the bundle's (which
    /// swaps a staged bundle in as it starts).
    pub fn relaunch_exe(&self) -> PathBuf {
        match self.os {
            Os::Linux => self.current().join("wubble"),
            Os::Mac => self.bundle.join("Contents/MacOS/wubble"),
        }
    }
}

/// Linux: `exe` (with symlinks resolved) is `<data_home>/wubble/versions/<v>/wubble` of a
/// bundle with its release file (`released`).
pub fn classify_linux(exe: &Path, data_home: &Path, released: bool) -> InstallKind {
    let root = data_home.join("wubble");
    let bundle = exe.parent();
    let in_versions = bundle
        .and_then(Path::parent)
        .is_some_and(|p| p == root.join("versions"));
    match bundle {
        Some(bundle) if in_versions && released => InstallKind::Managed(Layout {
            os: Os::Linux,
            root,
            bundle: bundle.to_path_buf(),
        }),
        _ => development_or_elsewhere(exe),
    }
}

/// macOS: `exe` is `wubble.app/Contents/MacOS/wubble` in `/Applications` or `<home>/Applications`,
/// with its release file (`released`); `data` is `~/Library/Application Support/wubble`.
pub fn classify_mac(exe: &Path, home: &Path, data: &Path, released: bool) -> InstallKind {
    let app = exe
        .parent()
        .filter(|p| p.ends_with("Contents/MacOS"))
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|app| app.file_name().is_some_and(|n| n == MAC_APP));
    let placed = app
        .and_then(Path::parent)
        .is_some_and(|dir| dir == Path::new("/Applications") || dir == home.join("Applications"));
    match app {
        Some(app) if placed && released => InstallKind::Managed(Layout {
            os: Os::Mac,
            root: data.to_path_buf(),
            bundle: app.to_path_buf(),
        }),
        _ => development_or_elsewhere(exe),
    }
}

/// macOS: whether `exe` runs from an App Translocation copy. macOS starts a quarantined app
/// that Finder did not move from a read-only copy under a random
/// `.../AppTranslocation/<id>/d/wubble.app`; from there the install is not in its place.
pub fn translocated(exe: &Path) -> bool {
    exe.components()
        .any(|c| c.as_os_str() == "AppTranslocation")
}

/// macOS: where a managed `wubble.app` can be.
pub fn mac_places(home: &Path) -> [PathBuf; 2] {
    [
        Path::new("/Applications").join(MAC_APP),
        home.join("Applications").join(MAC_APP),
    ]
}

fn development_or_elsewhere(exe: &Path) -> InstallKind {
    if exe.components().any(|c| c.as_os_str() == "target") {
        InstallKind::Development
    } else {
        InstallKind::Elsewhere
    }
}

/// A version on trial: started since it was installed, not yet healthy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    pub version: Version,
    pub attempts: u32,
    /// What to fall back to.
    pub previous: Option<Version>,
}

/// `updates/launch.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchRecord {
    /// The last version that ran healthy.
    pub good: Option<Version>,
    /// Installed and waiting for the next launch.
    pub staged: Option<Version>,
    pub trial: Option<Trial>,
    /// A version rolled back from: never offered again.
    pub bad: Option<Version>,
}

fn version_at(v: &Value, k: &str) -> Option<Version> {
    v.get(k)
        .and_then(Value::as_str)
        .and_then(|s| Version::parse(s).ok())
}

impl LaunchRecord {
    /// A missing or unreadable record is an empty one: nothing staged, nothing on trial.
    pub fn parse(text: &str) -> LaunchRecord {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return LaunchRecord::default();
        };
        LaunchRecord {
            good: version_at(&v, "good"),
            staged: version_at(&v, "staged"),
            trial: v.get("trial").and_then(|t| {
                Some(Trial {
                    version: version_at(t, "version")?,
                    attempts: t
                        .get("attempts")
                        .and_then(Value::as_u64)
                        .and_then(|n| u32::try_from(n).ok())?,
                    previous: version_at(t, "previous"),
                })
            }),
            bad: version_at(&v, "bad"),
        }
    }

    pub fn to_json(&self) -> String {
        let s = |v: &Option<Version>| v.as_ref().map(Version::to_string);
        json!({
            "good": s(&self.good),
            "staged": s(&self.staged),
            "trial": self.trial.as_ref().map(|t| json!({
                "version": t.version.to_string(),
                "attempts": t.attempts,
                "previous": s(&t.previous),
            })),
            "bad": s(&self.bad),
        })
        .to_string()
    }
}

/// What a launch does before anything else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchStep {
    Run,
    /// Put the staged version in place and start it instead.
    Apply(Version),
    /// The running version failed too often: go back to this one and start it.
    RollBack(Version),
}

/// A launch of `running`.
pub fn on_start(record: &LaunchRecord, running: &Version) -> (LaunchRecord, LaunchStep) {
    let mut next = record.clone();
    if let Some(staged) = &record.staged {
        if staged > running && record.bad.as_ref() != Some(staged) {
            return (next, LaunchStep::Apply(staged.clone()));
        }
        if staged == running {
            next.staged = None;
            next.trial = Some(Trial {
                version: running.clone(),
                attempts: 1,
                previous: record.good.clone().filter(|g| g != running),
            });
            return (next, LaunchStep::Run);
        }
        // older than what runs: forgotten
        next.staged = None;
    }
    if let Some(trial) = &record.trial
        && &trial.version == running
    {
        let attempts = trial.attempts + 1;
        if attempts > MAX_ATTEMPTS
            && let Some(previous) = &trial.previous
        {
            next.trial = None;
            next.bad = Some(running.clone());
            return (next, LaunchStep::RollBack(previous.clone()));
        }
        next.trial = Some(Trial {
            attempts,
            ..trial.clone()
        });
    }
    (next, LaunchStep::Run)
}

/// An Apply or RollBack that could not be carried out: the launch runs what it is.
pub fn on_switch_failed(record: &LaunchRecord, step: &LaunchStep) -> LaunchRecord {
    let mut next = record.clone();
    match step {
        LaunchStep::Apply(_) => next.staged = None,
        LaunchStep::RollBack(_) | LaunchStep::Run => {}
    }
    next
}

/// A new version was installed while `running` runs.
pub fn on_staged(record: &LaunchRecord, running: &Version, staged: &Version) -> LaunchRecord {
    LaunchRecord {
        good: record.good.clone().or_else(|| Some(running.clone())),
        staged: Some(staged.clone()),
        ..record.clone()
    }
}

/// `running` has run healthy: it is the good version, and the versions to delete are every
/// installed one but it and the one before it (`installed`: what is on disk).
pub fn on_healthy(
    record: &LaunchRecord,
    running: &Version,
    installed: &[Version],
) -> (LaunchRecord, Vec<Version>) {
    let mut next = record.clone();
    let mut keep = vec![running.clone()];
    match &record.trial {
        Some(trial) if &trial.version == running => {
            keep.extend(trial.previous.clone());
            next.trial = None;
        }
        _ => keep.extend(record.good.clone()),
    }
    keep.extend(record.staged.clone());
    next.good = Some(running.clone());
    let prune = installed
        .iter()
        .filter(|v| !keep.contains(v))
        .cloned()
        .collect();
    (next, prune)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn a_translocated_copy_is_told_from_an_install() {
        let copy = Path::new(
            "/private/var/folders/ab/T/AppTranslocation/1F2E/d/wubble.app/Contents/MacOS/wubble",
        );
        assert!(translocated(copy));
        assert!(!translocated(Path::new(
            "/Applications/wubble.app/Contents/MacOS/wubble"
        )));
        assert_eq!(
            mac_places(Path::new("/Users/u")),
            [
                PathBuf::from("/Applications/wubble.app"),
                PathBuf::from("/Users/u/Applications/wubble.app")
            ]
        );
    }

    #[test]
    fn linux_installs_are_found_by_their_place() {
        let data = Path::new("/home/u/.local/share");
        let exe = Path::new("/home/u/.local/share/wubble/versions/0.1.0/wubble");
        let InstallKind::Managed(l) = classify_linux(exe, data, true) else {
            panic!("not managed");
        };
        assert_eq!(
            l.bundle,
            Path::new("/home/u/.local/share/wubble/versions/0.1.0")
        );
        assert_eq!(
            l.version_dir(&v("0.2.0")),
            data.join("wubble/versions/0.2.0")
        );
        assert_eq!(l.relaunch_exe(), data.join("wubble/current/wubble"));
        assert_eq!(l.record(), data.join("wubble/updates/launch.json"));
        // no release file: a copy by hand
        assert_eq!(classify_linux(exe, data, false), InstallKind::Elsewhere);
        assert_eq!(
            classify_linux(Path::new("/home/u/wubble/target/debug/wubble"), data, true),
            InstallKind::Development
        );
        assert_eq!(
            classify_linux(Path::new("/usr/lib/wubble/wubble"), data, true),
            InstallKind::Elsewhere
        );
        assert_eq!(
            classify_linux(Path::new("/home/u/.local/share/wubble/wubble"), data, true),
            InstallKind::Elsewhere
        );
    }

    #[test]
    fn mac_installs_are_found_by_their_place() {
        let home = Path::new("/Users/u");
        let data = Path::new("/Users/u/Library/Application Support/wubble");
        for app in [
            "/Applications/wubble.app",
            "/Users/u/Applications/wubble.app",
        ] {
            let exe = Path::new(app).join("Contents/MacOS/wubble");
            let InstallKind::Managed(l) = classify_mac(&exe, home, data, true) else {
                panic!("{app} not managed");
            };
            assert_eq!(l.bundle, Path::new(app));
            assert_eq!(l.relaunch_exe(), exe);
            assert_eq!(l.staged_app(), data.join("updates/staged/wubble.app"));
        }
        let dmg = Path::new("/Volumes/wubble/wubble.app/Contents/MacOS/wubble");
        assert_eq!(classify_mac(dmg, home, data, true), InstallKind::Elsewhere);
        let dev = Path::new("/Users/u/w/target/debug/bundle/wubble.app/Contents/MacOS/wubble");
        assert_eq!(
            classify_mac(dev, home, data, true),
            InstallKind::Development
        );
        let unreleased = Path::new("/Applications/wubble.app/Contents/MacOS/wubble");
        assert_eq!(
            classify_mac(unreleased, home, data, false),
            InstallKind::Elsewhere
        );
    }

    #[test]
    fn records_round_trip_and_junk_is_empty() {
        let r = LaunchRecord {
            good: Some(v("0.1.0")),
            staged: Some(v("0.2.0-preview.1")),
            trial: Some(Trial {
                version: v("0.1.0"),
                attempts: 2,
                previous: None,
            }),
            bad: Some(v("0.1.5")),
        };
        assert_eq!(LaunchRecord::parse(&r.to_json()), r);
        assert_eq!(LaunchRecord::parse("not json"), LaunchRecord::default());
        assert_eq!(LaunchRecord::parse(""), LaunchRecord::default());
    }

    #[test]
    fn a_staged_version_applies_then_goes_on_trial() {
        let old = v("0.1.0");
        let new = v("0.2.0");
        let r = on_staged(&LaunchRecord::default(), &old, &new);
        assert_eq!(
            (r.good.clone(), r.staged.clone()),
            (Some(old.clone()), Some(new.clone()))
        );
        // the old binary starts (macOS, or a Linux start of the old version's own path)
        let (r1, step) = on_start(&r, &old);
        assert_eq!(step, LaunchStep::Apply(new.clone()));
        // the new one starts: on trial, the old one to fall back to
        let (r2, step) = on_start(&r1, &new);
        assert_eq!(step, LaunchStep::Run);
        assert_eq!(r2.staged, None);
        assert_eq!(
            r2.trial,
            Some(Trial {
                version: new.clone(),
                attempts: 1,
                previous: Some(old.clone()),
            })
        );
        // healthy: the new one is good, the one before it kept, older ones pruned
        let installed = [v("0.0.9"), old.clone(), new.clone()];
        let (r3, prune) = on_healthy(&r2, &new, &installed);
        assert_eq!(r3.good, Some(new.clone()));
        assert_eq!(r3.trial, None);
        assert_eq!(prune, [v("0.0.9")]);
    }

    #[test]
    fn a_version_failing_to_start_rolls_back_once() {
        let old = v("0.1.0");
        let new = v("0.2.0");
        let mut r = on_staged(&LaunchRecord::default(), &old, &new);
        let mut steps = vec![];
        for _ in 0..=MAX_ATTEMPTS {
            let (next, step) = on_start(&r, &new);
            r = next;
            steps.push(step);
        }
        assert_eq!(
            steps,
            [
                LaunchStep::Run,
                LaunchStep::Run,
                LaunchStep::Run,
                LaunchStep::RollBack(old.clone())
            ]
        );
        assert_eq!((r.trial.clone(), r.bad.clone()), (None, Some(new.clone())));
        // the old version runs normally after that, and the bad one is not applied again
        assert_eq!(on_start(&r, &old).1, LaunchStep::Run);
        let r = on_staged(&r, &old, &new);
        assert_eq!(on_start(&r, &old).1, LaunchStep::Run);
    }

    #[test]
    fn a_failed_trial_without_a_previous_version_keeps_running() {
        let new = v("0.2.0");
        let r = LaunchRecord {
            trial: Some(Trial {
                version: new.clone(),
                attempts: MAX_ATTEMPTS,
                previous: None,
            }),
            ..Default::default()
        };
        assert_eq!(on_start(&r, &new).1, LaunchStep::Run);
    }

    #[test]
    fn a_failed_switch_forgets_the_staged_version() {
        let r = on_staged(&LaunchRecord::default(), &v("0.1.0"), &v("0.2.0"));
        let (r, step) = on_start(&r, &v("0.1.0"));
        let r = on_switch_failed(&r, &step);
        assert_eq!(on_start(&r, &v("0.1.0")).1, LaunchStep::Run);
    }

    #[test]
    fn a_first_healthy_start_prunes_nothing_it_needs() {
        let only = v("0.1.0");
        let (r, prune) = on_healthy(&LaunchRecord::default(), &only, std::slice::from_ref(&only));
        assert_eq!(r.good, Some(only));
        assert!(prune.is_empty());
    }
}
