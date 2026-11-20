fn main() {
    // Debug builds carry the element ids the headless tests and `--smoke video` look up
    // (`ElementHandle`, i-slint-backend-testing); release builds do not, unless built with Slint's
    // own switch, `SLINT_EMIT_DEBUG_INFO=1` (it also keeps elements and properties the optimiser
    // would drop, so a shipped build goes without). `wubble_ui::ELEMENT_IDS` says which.
    println!("cargo:rerun-if-env-changed=SLINT_EMIT_DEBUG_INFO");
    let ids = std::env::var("PROFILE").is_ok_and(|p| p == "debug")
        || std::env::var_os("SLINT_EMIT_DEBUG_INFO").is_some();
    let config = slint_build::CompilerConfiguration::new().with_debug_info(ids);
    // The same for the `slint!` macros of the tests (the kit specimens, tests/controls_b.rs): the
    // macro reads this from rustc's environment.
    if ids {
        println!("cargo:rustc-env=SLINT_EMIT_DEBUG_INFO=1");
        println!("cargo:rustc-env=WUBBLE_ELEMENT_IDS=1");
    }
    slint_build::compile_with_config("ui/app.slint", config).expect("slint compile failed");
}
