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

    /// Compile-time proof that the command bar surface is reachable from Rust.
    #[allow(dead_code)]
    fn command_bar_is_reachable(app: &AppWindow) {
        app.set_command_bar_state(CommandBarState::Open);
        app.set_command_results(slint::ModelRc::from(
            &[CommandResult {
                kind: CommandKind::OpenTab,
                title: "t".into(),
                subtitle: "s".into(),
                shortcut: "k".into(),
            }][..],
        ));
        app.set_command_query("q".into());
        app.on_command_query_changed(|_| {});
        app.on_command_result_chosen(|_| {});
        app.on_command_dismissed(|| {});
        app.on_new_tab_requested(|| {});
        app.on_address_focus_requested(|| {});
    }

    #[test]
    fn command_bar_state_is_one_enum() {
        let src = include_str!("../ui/components/command-bar.slint");
        assert!(src.contains("property <CommandBarState> state"));
        for flag in ["open", "closing", "closed"] {
            assert!(!src.contains(&format!("property <bool> {flag}")));
        }
    }

    #[test]
    fn app_exports_colors() {
        let src = include_str!("../ui/app.slint");
        assert!(src.contains(
            "export { TabItem, SpaceItem, Colors, CommandResult, CommandKind, CommandBarState }"
        ));
    }
}
