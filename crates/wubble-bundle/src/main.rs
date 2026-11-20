//! Builds `wubble.app` from the binaries `cargo build --workspace` left in the
//! target directory, for the same profile this tool was built with:
//!
//!     cargo build --workspace && cargo run -p wubble-bundle
//!     target/debug/bundle/wubble.app/Contents/MacOS/wubble --smoke boot
//!
//! `--exe <path>` puts another binary (an example such as `frame_smoke`) in
//! place of `Contents/MacOS/wubble`, so it runs with the bundle's framework and
//! helper apps.
//!
//! The app icon is `assets/icons/macos/mac/wubble.icns`; Finder and the Dock show it for
//! `wubble.app` (a changed icon can need `touch wubble.app` or a Finder relaunch to refresh).
//!
//! The bundling itself is cef-rs's `cef::build_util::mac::bundle`
//! (tauri-apps/cef-rs, MIT OR Apache-2.0), used as a library. The shape of a
//! workspace bin calling it is from wgpu-weld `demo-weld-mac/src/bundle.rs`
//! (merely-made/wgpu-weld, MPL-2.0).

#[cfg(target_os = "macos")]
fn main() {
    use cef::build_util::mac::{BundleInfo, bundle};
    use std::path::PathBuf;

    let mut args = std::env::args().skip(1);
    let mut exe: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--exe", Some(path)) => exe = Some(path.into()),
            _ => fail(&format!("usage: wubble-bundle [--exe <path>] (got {arg})")),
        }
    }

    // This tool sits in target/<profile>/, next to wubble and wubble-helper,
    // whatever the profile or CARGO_TARGET_DIR.
    let current = std::env::current_exe().unwrap_or_else(|e| fail(&e.to_string()));
    let target = current
        .parent()
        .unwrap_or_else(|| fail("the bundler has no directory"))
        .to_path_buf();
    for bin in ["wubble", "wubble-helper"] {
        if !target.join(bin).is_file() {
            fail(&format!(
                "{} is missing; run `cargo build --workspace` first",
                target.join(bin).display()
            ));
        }
    }
    let out = target.join("bundle");
    std::fs::create_dir_all(&out).unwrap_or_else(|e| fail(&e.to_string()));
    let info = BundleInfo::new(
        "wubble",
        "dev.wubble.browser",
        "wubble",
        "English",
        // the workspace version, the one the binary and the updater use
        semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .unwrap_or_else(|e| fail(&format!("the workspace version: {e}"))),
    );
    // `bundle` copies `<resources>/mac/*` into Contents/Resources and, when
    // `mac/wubble.icns` exists, sets CFBundleIconFile in the app's Info.plist (the helper
    // apps get no icon). The .icns is made by scripts/make-icons.py.
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/macos");
    let app = bundle(
        &out,
        &target,
        "wubble",
        "wubble-helper",
        Some(resources),
        info,
    )
    .unwrap_or_else(|e| fail(&e.to_string()));
    add_plist_keys(&app.join("Contents/Info.plist"));
    if let Some(exe) = exe {
        let to = app.join("Contents/MacOS/wubble");
        std::fs::copy(&exe, &to)
            .unwrap_or_else(|e| fail(&format!("cannot copy {}: {e}", exe.display())));
    }
    println!("{}", app.display());
}

/// Keys `cef::build_util::mac`'s fixed Info.plist has no field for.
///
/// `NSLocalNetworkUsageDescription`: since macOS 15 a connection to the local network (a dev
/// server or NAS at 192.168.x.x, a `name.local` host; loopback is exempt) needs the user's
/// Local Network permission, and macOS shows this sentence when it asks. Chrome's Info.plist
/// carries the same key. No `NSBonjourServices`: wubble browses no Bonjour service itself.
#[cfg(target_os = "macos")]
fn add_plist_keys(path: &std::path::Path) {
    let mut info = plist::Value::from_file(path)
        .ok()
        .and_then(plist::Value::into_dictionary)
        .unwrap_or_else(|| fail(&format!("cannot read {}", path.display())));
    info.insert(
        "NSLocalNetworkUsageDescription".into(),
        "wubble opens the web pages you ask for, including ones served by devices and \
         development servers on your local network."
            .into(),
    );
    plist::Value::Dictionary(info)
        .to_file_xml(path)
        .unwrap_or_else(|e| fail(&format!("cannot write {}: {e}", path.display())));
}

#[cfg(target_os = "macos")]
fn fail(message: &str) -> ! {
    eprintln!("wubble-bundle: {message}");
    std::process::exit(1)
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("wubble-bundle: macOS only");
    std::process::exit(1)
}
