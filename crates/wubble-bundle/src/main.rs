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
        semver::Version::new(0, 0, 0),
    );
    let app = bundle(&out, &target, "wubble", "wubble-helper", None, info)
        .unwrap_or_else(|e| fail(&e.to_string()));
    if let Some(exe) = exe {
        let to = app.join("Contents/MacOS/wubble");
        std::fs::copy(&exe, &to)
            .unwrap_or_else(|e| fail(&format!("cannot copy {}: {e}", exe.display())));
    }
    println!("{}", app.display());
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
