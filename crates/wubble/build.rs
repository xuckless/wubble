//! Embeds the Windows icon in the exe (the crate `winresource`, its README's build script). The
//! target's OS decides, not the host's: `CARGO_CFG_TARGET_OS`. Nothing happens elsewhere.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let icon = "../../assets/icons/windows/wubble.ico";
        println!("cargo:rerun-if-changed={icon}");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon);
        if let Err(e) = res.compile() {
            panic!("wubble: embedding the Windows icon failed: {e}");
        }
    }
}
