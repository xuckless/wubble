// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! wubble change (3) (new file): the page hole. An image element showing [`image`] clears the
//! pixels under it to transparent instead of drawing (`itemrenderer.rs` `draw_image`), in the
//! shape of the clip it is drawn in (a rounded rectangle's clip: rounded, anti-aliased corners).
//! wubble draws it over a page pane whose page shows on an OS layer under the window's surface
//! (`wubble_core::page_layer`, stage 2), so that layer shows through the transparent window.
//! Inside a Skia layer (an element with opacity below 1 over several children) it clears only
//! that layer. Other renderers draw the image as one transparent pixel: nothing.

use i_slint_core::graphics::{Image, Rgba8Pixel, SharedPixelBuffer};

thread_local! {
    /// One per thread (the UI thread renders); compared by its buffer, so its clones are it.
    static HOLE: Image = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(1, 1));
}

/// The image an element shows to be a hole.
pub fn image() -> Image {
    HOLE.with(Clone::clone)
}

/// Whether `image` is the hole.
pub(crate) fn is(image: &Image) -> bool {
    HOLE.with(|hole| hole == image)
}

/// The paint the hole is drawn with: whatever is under it goes.
pub(crate) fn paint() -> skia_safe::Paint {
    let mut paint = skia_safe::Paint::default();
    paint.set_blend_mode(skia_safe::BlendMode::Clear);
    paint
}
