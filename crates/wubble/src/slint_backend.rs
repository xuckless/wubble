//! Slint's platform: the winit backend `slint::BackendSelector` would build (Skia on the shared
//! wgpu 30 device, wubble's window attributes), wrapped so that Slint's own text fields copy and
//! paste through the host's clipboard where it has one (`crate::clipboard::field_load`).
//!
//! On Wayland the compositor offers the selection to one `wl_data_device` per client: the host's
//! worker (`crate::wl::wayland`), which the pages need. The one Slint's winit backend opens for
//! itself (smithay-clipboard) is then never offered anything, so every shell field (the command
//! bar, the start page's capsule, the find bar, Settings' search, the sheets) pasted nothing
//! while its copies still went out. Everything else is the winit backend's own.

use std::rc::Rc;

use i_slint_core::{InternalToken, SlintContextWeak};
use slint::PlatformError;
use slint::platform::{Clipboard, EventLoopProxy, Platform, WindowAdapter};

use crate::clipboard::{self, Buffer};

/// Builds the backend and makes it Slint's platform, as `BackendSelector::select` does.
pub fn select(
    wgpu: slint::wgpu_30::WGPUConfiguration,
    renderer: &str,
    attributes: fn(
        slint::winit_030::winit::window::WindowAttributes,
    ) -> slint::winit_030::winit::window::WindowAttributes,
) -> Result<(), PlatformError> {
    // The selector's: the system's fonts load on their own thread while the backend starts.
    let _prefetch = i_slint_core::font_collection::prefetch();
    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name(renderer)
        .request_graphics_api(i_slint_core::graphics::RequestedGraphicsAPI::WGPU30(wgpu))
        .with_window_attributes_hook(attributes)
        .build()?;
    slint::platform::set_platform(Box::new(ShellPlatform(backend)))
        .map_err(PlatformError::SetPlatformError)
}

fn buffer(clipboard: &Clipboard) -> Option<Buffer> {
    match clipboard {
        Clipboard::DefaultClipboard => Some(Buffer::Clipboard),
        Clipboard::SelectionClipboard => Some(Buffer::Primary),
        _ => None,
    }
}

struct ShellPlatform(i_slint_backend_winit::Backend);

impl Platform for ShellPlatform {
    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if !buffer(&clipboard).is_some_and(|b| clipboard::field_store(b, text)) {
            self.0.set_clipboard_text(text, clipboard);
        }
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        match buffer(&clipboard).and_then(clipboard::field_load) {
            Some(text) => text,
            None => self.0.clipboard_text(clipboard),
        }
    }

    // The rest is the winit backend's.

    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        self.0.create_window_adapter()
    }

    fn run_event_loop(&self) -> Result<(), PlatformError> {
        self.0.run_event_loop()
    }

    fn process_events(
        &self,
        timeout: Option<std::time::Duration>,
        token: InternalToken,
    ) -> Result<std::ops::ControlFlow<()>, PlatformError> {
        self.0.process_events(timeout, token)
    }

    fn bind_context(&self, ctx: SlintContextWeak, token: InternalToken) {
        self.0.bind_context(ctx, token);
    }

    #[allow(deprecated)]
    fn set_event_loop_quit_on_last_window_closed(&self, quit_on_last_window_closed: bool) {
        self.0
            .set_event_loop_quit_on_last_window_closed(quit_on_last_window_closed);
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        self.0.new_event_loop_proxy()
    }

    fn duration_since_start(&self) -> std::time::Duration {
        self.0.duration_since_start()
    }

    fn click_interval(&self) -> std::time::Duration {
        self.0.click_interval()
    }

    fn cursor_flash_cycle(&self) -> std::time::Duration {
        self.0.cursor_flash_cycle()
    }

    fn debug_log(&self, arguments: std::fmt::Arguments) {
        self.0.debug_log(arguments);
    }

    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.0.open_url(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slints_clipboards_map_to_the_hosts_buffers() {
        assert_eq!(
            buffer(&Clipboard::DefaultClipboard),
            Some(Buffer::Clipboard)
        );
        assert_eq!(
            buffer(&Clipboard::SelectionClipboard),
            Some(Buffer::Primary)
        );
    }
}
