use slint::ComponentHandle;
use wubble_ui::AppWindow;

fn main() -> Result<(), slint::PlatformError> {
    // M0: initialise CEF (wubble-engine) and hand frames to the window (wubble-gfx).
    let window = AppWindow::new()?;
    window.run()
}
