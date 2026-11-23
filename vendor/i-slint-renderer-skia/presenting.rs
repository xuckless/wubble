// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! wubble change (2) (new file): a hook the application sets, called with the window just
//! before the wgpu 30 surface presents a frame of it (beside upstream's `pre_present_callback`,
//! which the winit backend owns). wubble hides a page's OS layer here, so the hide and the
//! shell's frame that replaces it land together. One per thread (the UI thread renders).

use std::cell::RefCell;

use i_slint_core::api::Window;

thread_local! {
    static HOOK: RefCell<Option<Box<dyn Fn(&Window)>>> = const { RefCell::new(None) };
}

/// Calls `hook` before every present of a window's frame on this thread.
pub fn set_hook(hook: impl Fn(&Window) + 'static) {
    HOOK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
}

/// A frame of `window` is about to be presented.
pub(crate) fn notify(window: &Window) {
    HOOK.with(|h| {
        if let Some(hook) = h.borrow().as_ref() {
            hook(window);
        }
    });
}
