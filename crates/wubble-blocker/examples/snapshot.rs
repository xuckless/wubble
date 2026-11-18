//! Refreshes the lists bundled into wubble-blocker from their catalog URLs
//! (`catalog::LISTS` rows with `bundled`). Run by hand, never by the build:
//!
//!     cargo run -p wubble-blocker --example snapshot
//!
//! then rebuild (the lists are `include_str!`ed) and update `data/README.md`.

use std::path::Path;

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/lists");
    let mut failed = false;
    for list in wubble_blocker::catalog::LISTS
        .iter()
        .filter(|l| l.bundled.is_some())
    {
        match wubble_blocker::fetch_http(list.url) {
            Ok(text) => {
                let path = out.join(format!("{}.txt", list.id));
                std::fs::write(&path, &text).expect("data/lists is writable");
                println!("{} {} bytes", list.id, text.len());
            }
            Err(e) => {
                eprintln!("{}: {e}", list.id);
                failed = true;
            }
        }
    }
    std::process::exit(i32::from(failed));
}
