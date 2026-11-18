//! Slint shell. Generated component types are re-exported from here.

slint::include_modules!();

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time proof that `Colors` is exported and its `light` switch is
    /// reachable from Rust. Never constructs a window.
    #[allow(dead_code)]
    fn theme_switch_is_reachable(app: &AppWindow) {
        app.global::<Colors>().set_light(true);
    }

    #[test]
    fn app_exports_colors() {
        let src = include_str!("../ui/app.slint");
        assert!(src.contains("export { TabItem, SpaceItem, Colors }"));
    }
}
