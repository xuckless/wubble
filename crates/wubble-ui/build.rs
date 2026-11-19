fn main() {
    // Debug builds carry the element ids the headless tests look up (`ElementHandle`, i-slint-
    // backend-testing); release builds do not.
    let debug = std::env::var("PROFILE").is_ok_and(|p| p == "debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    // The same for the `slint!` macros of the tests (the kit specimens, tests/controls_b.rs): the
    // macro reads this from rustc's environment.
    if debug {
        println!("cargo:rustc-env=SLINT_EMIT_DEBUG_INFO=1");
    }
    slint_build::compile_with_config("ui/app.slint", config).expect("slint compile failed");
}
