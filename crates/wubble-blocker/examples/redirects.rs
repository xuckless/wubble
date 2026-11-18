//! Writes uBlock Origin's redirect resources (`$redirect=`) as adblock-rust
//! resources, for `data/resources/redirects.json`. Run by hand when updating
//! uBO (see `data/README.md`):
//!
//!     cargo run -p wubble-blocker --example redirects -- <uBlock checkout> \
//!       > crates/wubble-blocker/data/resources/redirects.json
//!
//! adblock's own assembler (`resource_assembler`, the Rust behind adblock-rs
//! `uBlockResources`) reads uBO's `src/web_accessible_resources` and
//! `src/js/redirect-resources.js` (gorhill/uBlock, GPL-3.0). As in
//! brave-core-crx-packager `lib/adBlockRustUtils.js` `generateResources`
//! (MPL-2.0), `google-ima-dai.js` is left out.

use std::path::Path;

use adblock::resources::resource_assembler::assemble_web_accessible_resources;

fn main() {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: redirects <uBlock checkout>");
        std::process::exit(2);
    };
    let root = Path::new(&root);
    let mut resources = assemble_web_accessible_resources(
        &root.join("src/web_accessible_resources"),
        &root.join("src/js/redirect-resources.js"),
    );
    resources.retain(|r| r.name != "google-ima-dai.js");
    resources.sort_by(|a, b| a.name.cmp(&b.name));
    println!(
        "{}",
        serde_json::to_string(&resources).expect("resources serialise")
    );
}
