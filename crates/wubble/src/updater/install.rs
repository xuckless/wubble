//! The install on disk: one [`Install`] trait, a Linux and a macOS implementation, over the
//! layout and launch decisions of `wubble_core::update::install`. Nothing here runs a byte of a
//! download: an archive reaches [`stage`] only after `verify_archive` passed, and the bundle it
//! holds starts only at the next launch.
//!
//! The macOS swap follows Sparkle's and Zed's: the new `wubble.app` is staged outside the
//! running one and replaces it by renames, never edited (its code signature and notarisation
//! ticket stay as they were signed). Unlike Zed (which rsyncs over the running app) the swap
//! waits for the next launch, before CEF starts: a running CEF browser starts helper processes
//! from its bundle at any time, and they must be its own version's.

use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use wubble_core::update::Version;
use wubble_core::update::install::{
    self as layout, InstallKind, LaunchRecord, LaunchStep, Layout, Os,
};
use wubble_core::update::manifest::{RELEASE_FILE, ReleaseInfo};

/// What differs per OS about putting a version in place.
pub trait Install: Send + Sync {
    fn layout(&self) -> &Layout;

    /// Where a bundle (an unpacked archive's top entry) keeps its release file.
    fn release_file(&self, bundle: &Path) -> PathBuf;

    /// Puts a checked bundle where the next launch finds it.
    fn place(&self, bundle: &Path, version: &Version) -> Result<(), String>;

    /// Carries out a launch's Apply or RollBack; the program to start instead.
    fn switch(&self, step: &LaunchStep) -> Result<PathBuf, String>;

    /// The versions on disk.
    fn installed(&self) -> Vec<Version>;

    fn remove(&self, version: &Version) -> Result<(), String>;
}

/// The install the running binary belongs to, if wubble manages it. `None` for a development
/// build or a system package: then nothing here runs.
pub fn detect() -> (InstallKind, Option<Box<dyn Install>>) {
    let kind = classify();
    let install: Option<Box<dyn Install>> = match &kind {
        InstallKind::Managed(l) => Some(match l.os {
            Os::Linux => Box::new(Linux(l.clone())),
            Os::Mac => Box::new(Mac(l.clone())),
        }),
        _ => None,
    };
    (kind, install)
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

fn classify() -> InstallKind {
    // a debug build is a development build wherever its target directory is
    if cfg!(debug_assertions) {
        return InstallKind::Development;
    }
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|e| std::fs::canonicalize(e).ok())
    else {
        return InstallKind::Elsewhere;
    };
    let Some(home) = env_path("HOME") else {
        return InstallKind::Elsewhere;
    };
    if cfg!(target_os = "macos") {
        let released = exe
            .parent()
            .and_then(Path::parent)
            .is_some_and(|contents| contents.join("Resources").join(RELEASE_FILE).is_file());
        let data = home.join("Library/Application Support/wubble");
        layout::classify_mac(&exe, &home, &data, released)
    } else if cfg!(target_os = "linux") {
        let released = exe.with_file_name(RELEASE_FILE).is_file();
        let data_home = env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        // the data home itself may be a link: compare resolved paths
        let data_home = std::fs::canonicalize(&data_home).unwrap_or(data_home);
        layout::classify_linux(&exe, &data_home, released)
    } else {
        InstallKind::Elsewhere
    }
}

/// macOS's mark on files a quarantining app wrote. wubble's `Info.plist` has
/// `LSFileQuarantineEnabled` (cef-rs's bundler, as Chrome's has), so the update archive wubble
/// downloads, and the app unpacked from it, carry the mark.
const QUARANTINE: &str = "com.apple.quarantine";

fn quarantined(app: &Path) -> bool {
    std::process::Command::new("/usr/bin/xattr")
        .args(["-p", QUARANTINE])
        .arg(app)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Takes the mark off `app` and everything in it (the system's `xattr`; the mark is no part
/// of the code signature). Only for a bundle whose archive passed `verify_archive`, or the
/// install itself. Whether it is gone.
fn clear_quarantine(app: &Path) -> bool {
    let _ = std::process::Command::new("/usr/bin/xattr")
        .args(["-dr", QUARANTINE])
        .arg(app)
        .status();
    !quarantined(app)
}

fn mac_version(app: &Path) -> Option<Version> {
    let text = std::fs::read_to_string(app.join("Contents/Resources").join(RELEASE_FILE)).ok()?;
    ReleaseInfo::parse(&text).ok().map(|i| i.version)
}

/// macOS, first at a launch: an install that an earlier wubble's update left quarantined.
/// Started from its place, the mark is taken off. Started from an App Translocation copy
/// (macOS runs a quarantined app Finder did not move from a random read-only folder, where
/// the updater finds no install: "managed elsewhere", and no update), the mark is taken off
/// the install of this same version and that one starts in this process's place.
pub fn settle_quarantine() {
    if !cfg!(target_os = "macos") || cfg!(debug_assertions) {
        return;
    }
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|e| std::fs::canonicalize(e).ok())
    else {
        return;
    };
    if !layout::translocated(&exe) {
        if let InstallKind::Managed(l) = classify()
            && quarantined(&l.bundle)
        {
            clear_quarantine(&l.bundle);
        }
        return;
    }
    let Some(home) = env_path("HOME") else {
        return;
    };
    let running = wubble_core::update::current_version();
    for app in layout::mac_places(&home) {
        if mac_version(&app).as_ref() != Some(&running) || !quarantined(&app) {
            continue;
        }
        if !clear_quarantine(&app) {
            eprintln!("wubble: {} stays quarantined", app.display());
            continue;
        }
        let exe = app.join("Contents/MacOS/wubble");
        eprintln!(
            "wubble: leaving the translocated copy: starting {}",
            exe.display()
        );
        let e = exec(&exe);
        eprintln!("wubble: cannot start {}: {e}", exe.display());
        return;
    }
}

pub fn read_record(l: &Layout) -> LaunchRecord {
    LaunchRecord::parse(&std::fs::read_to_string(l.record()).unwrap_or_default())
}

/// Written whole beside the old one, then renamed over it.
pub fn write_record(l: &Layout, record: &LaunchRecord) -> Result<(), String> {
    let path = l.record();
    let tmp = path.with_extension("json.new");
    std::fs::create_dir_all(l.updates())
        .and_then(|()| std::fs::write(&tmp, record.to_json()))
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Unpacks a `.tar.xz` into `into` (made fresh). The xz stream is decoded on a thread of its
/// own into a pipe the tar reader drains (lzma-rs writes, tar reads).
pub fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    remove_any(into).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let (reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
    let decoder = std::thread::Builder::new()
        .name("wubble-unxz".into())
        .spawn(move || {
            let mut input = BufReader::new(file);
            let mut output = BufWriter::with_capacity(1 << 16, writer);
            lzma_rs::xz_decompress(&mut input, &mut output).map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(reader);
    tar.set_preserve_permissions(true);
    tar.set_unpack_xattrs(false);
    tar.set_overwrite(true);
    let untarred = tar.unpack(into).map_err(|e| format!("unpack: {e}"));
    // a tar error drops the pipe's reader, so the decoder ends with a broken pipe
    drop(tar);
    let decoded = decoder
        .join()
        .map_err(|_| "the decoder panicked".to_owned())?;
    untarred?;
    decoded.map_err(|e| format!("xz: {e}"))
}

/// The one entry at the top of an unpacked archive.
fn top_entry(dir: &Path) -> Result<PathBuf, String> {
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    match entries.as_slice() {
        [one] if one.is_dir() => Ok(one.clone()),
        _ => Err("the archive does not hold one bundle".into()),
    }
}

/// A verified archive of `version` for `platform`: unpacked in the staging area, its release
/// file checked, put in place, and the launch record told. `running`: this process's version.
pub fn stage(
    install: &dyn Install,
    archive: &Path,
    version: &Version,
    platform: &str,
    running: &Version,
) -> Result<(), String> {
    let l = install.layout();
    let unpacked = l.staging().join(format!("{version}.unpack"));
    let result = (|| {
        unpack(archive, &unpacked)?;
        let bundle = top_entry(&unpacked)?;
        let release = install.release_file(&bundle);
        let text = std::fs::read_to_string(&release)
            .map_err(|_| format!("the bundle has no {RELEASE_FILE}"))?;
        ReleaseInfo::parse(&text)
            .and_then(|info| info.matches(version, platform))
            .map_err(|r| r.to_string())?;
        install.place(&bundle, version)?;
        write_record(l, &layout::on_staged(&read_record(l), running, version))
    })();
    let _ = remove_any(&unpacked);
    let _ = std::fs::remove_file(archive);
    result
}

/// What a launch of a self-managed install does first (`main`, after the single-instance
/// claim, before CEF): apply a staged version or roll back, by starting that version's program
/// in this process's place (`exec`). Returns only to run this version.
pub fn on_launch(install: &dyn Install) {
    let l = install.layout();
    let running = wubble_core::update::current_version();
    let record = read_record(l);
    let (next, step) = layout::on_start(&record, &running);
    if step == LaunchStep::Run {
        if next != record
            && let Err(e) = write_record(l, &next)
        {
            eprintln!("wubble: update record: {e}");
        }
        return;
    }
    match install.switch(&step) {
        Ok(exe) => {
            if let Err(e) = write_record(l, &next) {
                eprintln!("wubble: update record: {e}");
            }
            eprintln!("wubble: {step:?}: starting {}", exe.display());
            let e = exec(&exe);
            // exec returned: the program could not start; run this one
            eprintln!("wubble: cannot start {}: {e}", exe.display());
        }
        Err(e) => eprintln!("wubble: {step:?} failed: {e}"),
    }
    if let Err(e) = write_record(l, &layout::on_switch_failed(&next, &step)) {
        eprintln!("wubble: update record: {e}");
    }
}

#[cfg(unix)]
fn exec(exe: &Path) -> std::io::Error {
    use std::os::unix::process::CommandExt as _;
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .exec()
}

#[cfg(not(unix))]
fn exec(_exe: &Path) -> std::io::Error {
    std::io::Error::other("no self-managed install on this OS")
}

/// This version ran healthy: recorded, and versions no longer needed deleted.
pub fn on_healthy(install: &dyn Install) {
    let l = install.layout();
    let running = wubble_core::update::current_version();
    let record = read_record(l);
    let (next, prune) = layout::on_healthy(&record, &running, &install.installed());
    if next != record
        && let Err(e) = write_record(l, &next)
    {
        eprintln!("wubble: update record: {e}");
    }
    for v in prune {
        if let Err(e) = install.remove(&v) {
            eprintln!("wubble: cannot remove wubble {v}: {e}");
        }
    }
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("no self-managed install on this OS"))
}

/// Linux: versions side by side, `current` switched by a rename.
pub struct Linux(pub Layout);

impl Linux {
    /// `current` -> `versions/<v>` (relative), atomically: a new link renamed over the old.
    fn point_current(&self, v: &Version) -> Result<(), String> {
        let current = self.0.current();
        let fresh = self.0.root.join("current.new");
        let _ = remove_any(&fresh);
        symlink(&Path::new("versions").join(v.to_string()), &fresh)
            .and_then(|()| std::fs::rename(&fresh, &current))
            .map_err(|e| format!("{}: {e}", current.display()))
    }
}

impl Install for Linux {
    fn layout(&self) -> &Layout {
        &self.0
    }

    fn release_file(&self, bundle: &Path) -> PathBuf {
        bundle.join(RELEASE_FILE)
    }

    fn place(&self, bundle: &Path, version: &Version) -> Result<(), String> {
        let dest = self.0.version_dir(version);
        if dest == self.0.bundle {
            return Err("that version is the one running".into());
        }
        remove_any(&dest).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(self.0.versions()).map_err(|e| e.to_string())?;
        std::fs::rename(bundle, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        self.point_current(version)
    }

    fn switch(&self, step: &LaunchStep) -> Result<PathBuf, String> {
        let (LaunchStep::Apply(v) | LaunchStep::RollBack(v)) = step else {
            return Err("nothing to switch".into());
        };
        let exe = self.0.version_dir(v).join("wubble");
        if !exe.is_file() {
            return Err(format!("{} is missing", exe.display()));
        }
        self.point_current(v)?;
        Ok(exe)
    }

    fn installed(&self) -> Vec<Version> {
        std::fs::read_dir(self.0.versions())
            .into_iter()
            .flatten()
            .filter_map(|e| Version::parse(e.ok()?.file_name().to_str()?).ok())
            .collect()
    }

    fn remove(&self, version: &Version) -> Result<(), String> {
        let dir = self.0.version_dir(version);
        if dir == self.0.bundle {
            return Err("that version is the one running".into());
        }
        remove_any(&dir).map_err(|e| e.to_string())
    }
}

/// macOS: one `wubble.app`, replaced whole at the next launch.
pub struct Mac(pub Layout);

impl Mac {
    fn version_of(&self, app: &Path) -> Option<Version> {
        mac_version(app)
    }
}

/// A rename, or a copy that keeps every attribute and signature (`ditto`, the system's) and a
/// delete when the two places are on different volumes.
fn move_bundle(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            let status = std::process::Command::new("/usr/bin/ditto")
                .arg(from)
                .arg(to)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                let _ = remove_any(to);
                return Err(format!("ditto {} failed", from.display()));
            }
            remove_any(from).map_err(|e| e.to_string())
        }
        Err(e) => Err(format!("{} -> {}: {e}", from.display(), to.display())),
    }
}

impl Install for Mac {
    fn layout(&self) -> &Layout {
        &self.0
    }

    fn release_file(&self, bundle: &Path) -> PathBuf {
        bundle.join("Contents/Resources").join(RELEASE_FILE)
    }

    fn place(&self, bundle: &Path, _version: &Version) -> Result<(), String> {
        if bundle.file_name().is_none_or(|n| n != layout::MAC_APP) {
            return Err(format!("the archive does not hold {}", layout::MAC_APP));
        }
        // checked (`verify_archive`, its release file): without this the swapped-in app is
        // quarantined and macOS runs it translocated (`settle_quarantine`)
        clear_quarantine(bundle);
        let staged = self.0.staged_app();
        remove_any(&staged).map_err(|e| e.to_string())?;
        move_bundle(bundle, &staged)
    }

    fn switch(&self, step: &LaunchStep) -> Result<PathBuf, String> {
        let app = &self.0.bundle;
        let previous = self.0.previous_app();
        match step {
            LaunchStep::Apply(v) => {
                let staged = self.0.staged_app();
                if self.version_of(&staged).as_ref() != Some(v) {
                    return Err(format!("no staged wubble {v}"));
                }
                remove_any(&previous).map_err(|e| e.to_string())?;
                move_bundle(app, &previous)?;
                if let Err(e) = move_bundle(&staged, app) {
                    // put the old one back
                    let _ = move_bundle(&previous, app);
                    return Err(e);
                }
            }
            LaunchStep::RollBack(v) => {
                if self.version_of(&previous).as_ref() != Some(v) {
                    return Err(format!("wubble {v} is not kept"));
                }
                let failed = self.0.updates().join("failed").join(layout::MAC_APP);
                remove_any(&failed).map_err(|e| e.to_string())?;
                move_bundle(app, &failed)?;
                if let Err(e) = move_bundle(&previous, app) {
                    let _ = move_bundle(&failed, app);
                    return Err(e);
                }
                let _ = remove_any(&failed);
            }
            LaunchStep::Run => return Err("nothing to switch".into()),
        }
        Ok(self.0.relaunch_exe())
    }

    fn installed(&self) -> Vec<Version> {
        [
            self.0.bundle.clone(),
            self.0.previous_app(),
            self.0.staged_app(),
        ]
        .iter()
        .filter_map(|app| self.version_of(app))
        .collect()
    }

    fn remove(&self, version: &Version) -> Result<(), String> {
        // only the kept previous bundle is ever removed; the running one never
        let previous = self.0.previous_app();
        if self.version_of(&previous).as_ref() == Some(version) {
            remove_any(&previous).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update_smoke::pack;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wubble-install-test-{name}-{}", std::process::id()));
        let _ = remove_any(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// A bundle folder with a program and its release file.
    fn bundle(at: &Path, version: &str) {
        std::fs::create_dir_all(at).unwrap();
        std::fs::write(at.join("wubble"), version).unwrap();
        let info = ReleaseInfo {
            version: v(version),
            platform: "linux-x86_64".into(),
            commit: String::new(),
        };
        std::fs::write(at.join(RELEASE_FILE), info.to_json()).unwrap();
    }

    #[test]
    fn linux_stages_switches_current_and_rolls_back() {
        let root = scratch("linux");
        let running = v("0.1.0");
        bundle(&root.join("versions/0.1.0"), "0.1.0");
        let install = Linux(Layout {
            os: Os::Linux,
            root: root.clone(),
            bundle: root.join("versions/0.1.0"),
        });
        install.point_current(&running).unwrap();
        // an archive of 0.2.0
        let src = scratch("linux-src");
        bundle(&src.join("wubble-0.2.0-linux-x86_64"), "0.2.0");
        let archive = root.join("a.tar.xz");
        pack(&src, "wubble-0.2.0-linux-x86_64", &archive).unwrap();
        stage(&install, &archive, &v("0.2.0"), "linux-x86_64", &running).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("current/wubble")).unwrap(),
            "0.2.0"
        );
        assert_eq!(
            std::fs::read_link(root.join("current")).unwrap(),
            Path::new("versions/0.2.0")
        );
        assert!(!archive.exists(), "the download is removed");
        let record = read_record(install.layout());
        assert_eq!(record.staged, Some(v("0.2.0")));
        assert_eq!(record.good, Some(running.clone()));
        let mut installed = install.installed();
        installed.sort();
        assert_eq!(installed, [v("0.1.0"), v("0.2.0")]);
        // a rollback points current back
        assert_eq!(
            install
                .switch(&LaunchStep::RollBack(running.clone()))
                .unwrap(),
            root.join("versions/0.1.0/wubble")
        );
        assert_eq!(
            std::fs::read_to_string(root.join("current/wubble")).unwrap(),
            "0.1.0"
        );
        let _ = remove_any(&root);
        let _ = remove_any(&src);
    }

    #[test]
    fn a_bundle_of_another_version_or_platform_is_not_placed() {
        let root = scratch("linux-wrong");
        let install = Linux(Layout {
            os: Os::Linux,
            root: root.clone(),
            bundle: root.join("versions/0.1.0"),
        });
        let src = scratch("linux-wrong-src");
        bundle(&src.join("top"), "0.3.0");
        let archive = root.join("a.tar.xz");
        pack(&src, "top", &archive).unwrap();
        let refused = stage(&install, &archive, &v("0.2.0"), "linux-x86_64", &v("0.1.0"));
        assert!(refused.unwrap_err().contains("not 0.2.0"));
        bundle(&src.join("top"), "0.2.0");
        pack(&src, "top", &archive).unwrap();
        let refused = stage(
            &install,
            &archive,
            &v("0.2.0"),
            "macos-aarch64",
            &v("0.1.0"),
        );
        assert!(refused.unwrap_err().contains("macos-aarch64"));
        assert!(install.installed().is_empty());
        let _ = remove_any(&root);
        let _ = remove_any(&src);
    }

    /// The release archives come from `xz -T0` (package-linux.sh): several blocks in one
    /// stream. Skipped where no `xz` is installed.
    #[test]
    fn multi_block_xz_from_the_xz_tool_unpacks() {
        let root = scratch("xz");
        let src = scratch("xz-src");
        bundle(&src.join("top"), "0.2.0");
        std::fs::write(src.join("top/big"), vec![7u8; 100_000]).unwrap();
        let tar_path = root.join("a.tar");
        {
            let mut b = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            b.append_dir_all("top", src.join("top")).unwrap();
            b.finish().unwrap();
        }
        let xz = std::process::Command::new("xz")
            .args(["-T2", "--block-size=4096", "-k"])
            .arg(&tar_path)
            .status();
        if !xz.is_ok_and(|s| s.success()) {
            eprintln!("no xz here: skipped");
            return;
        }
        unpack(&root.join("a.tar.xz"), &root.join("out")).unwrap();
        assert_eq!(
            std::fs::read(root.join("out/top/big")).unwrap().len(),
            100_000
        );
        let _ = remove_any(&root);
        let _ = remove_any(&src);
    }

    #[test]
    fn junk_is_not_unpacked() {
        let root = scratch("junk");
        std::fs::write(root.join("a.tar.xz"), b"not xz").unwrap();
        assert!(unpack(&root.join("a.tar.xz"), &root.join("out")).is_err());
        let _ = remove_any(&root);
    }

    #[test]
    fn mac_swaps_the_staged_app_in_and_back() {
        let root = scratch("mac");
        let apps = root.join("Applications");
        let app = apps.join("wubble.app");
        let mac_bundle = |at: &Path, version: &str| {
            let res = at.join("Contents/Resources");
            std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
            std::fs::create_dir_all(&res).unwrap();
            std::fs::write(at.join("Contents/MacOS/wubble"), version).unwrap();
            let info = ReleaseInfo {
                version: v(version),
                platform: "macos-aarch64".into(),
                commit: String::new(),
            };
            std::fs::write(res.join(RELEASE_FILE), info.to_json()).unwrap();
        };
        mac_bundle(&app, "0.1.0");
        let install = Mac(Layout {
            os: Os::Mac,
            root: root.join("support"),
            bundle: app.clone(),
        });
        let src = scratch("mac-src");
        mac_bundle(&src.join("wubble.app"), "0.2.0");
        let archive = root.join("a.tar.xz");
        pack(&src, "wubble.app", &archive).unwrap();
        stage(
            &install,
            &archive,
            &v("0.2.0"),
            "macos-aarch64",
            &v("0.1.0"),
        )
        .unwrap();
        // staged, not swapped: the running app is untouched until the next launch
        let exe = app.join("Contents/MacOS/wubble");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
        let record = read_record(install.layout());
        let (_, step) = layout::on_start(&record, &v("0.1.0"));
        assert_eq!(step, LaunchStep::Apply(v("0.2.0")));
        assert_eq!(install.switch(&step).unwrap(), exe);
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.2.0");
        assert_eq!(
            install.version_of(&install.0.previous_app()),
            Some(v("0.1.0"))
        );
        install.switch(&LaunchStep::RollBack(v("0.1.0"))).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
        let _ = remove_any(&root);
        let _ = remove_any(&src);
    }
}
