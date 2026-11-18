// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! View geometry: physical size plus the display scale factor.
//!
//! CEF's windowless model separates the two, and getting it wrong is quiet
//! rather than loud:
//!
//! - `GetViewRect` is answered in **DIP** (device-independent pixels, i.e. CSS
//!   pixels), not physical pixels.
//! - `GetScreenInfo::device_scale_factor` is the multiplier CEF applies to
//!   produce the physical texture it paints.
//! - Mouse coordinates handed to CEF are in DIP as well.
//!
//! Answer `GetViewRect` in physical pixels with a scale of 1, as welding did
//! before, and a 2x display gets a page laid out at twice the CSS width, so
//! everything renders at half its intended size. Feed it physical mouse
//! coordinates under a real scale and clicks land at the wrong place. Both
//! failure modes look like a working browser, which is why this lives in one
//! place with tests rather than being open-coded per platform.
//!
//! Hosts talk to welding in **physical** pixels throughout, because that is
//! what a window system hands them; the DIP conversion happens here.

use dpi::PhysicalSize;

use crate::surface::PopupRect;

/// Where the view sits inside the host window, both physical. wubble change:
/// reporting this (instead of only the scale) is what lets Blink place a
/// `<select>` popup outside the page rectangle. The window stands in for the
/// screen, so no global window position is needed (Wayland has none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenArea {
    /// The host window's size in physical pixels.
    pub window: PhysicalSize<u32>,
    /// The view's top-left corner inside the window, in physical pixels.
    pub origin: (i32, i32),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewMetrics {
    size: PhysicalSize<u32>,
    scale: f32,
    /// `None` keeps the plain behaviour: the view is the whole screen.
    area: Option<ScreenArea>,
}

impl ViewMetrics {
    pub(crate) fn new(size: PhysicalSize<u32>, scale: f32) -> Self {
        Self {
            size,
            scale: sane_scale(scale),
            area: None,
        }
    }

    pub(crate) fn set_area(&mut self, area: ScreenArea) {
        self.area = Some(area);
    }

    pub(crate) fn area(&self) -> Option<ScreenArea> {
        self.area
    }

    pub(crate) fn set_size(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
    }

    pub(crate) fn set_scale(&mut self, scale: f32) {
        self.scale = sane_scale(scale);
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// What `GetViewRect` should report: the view in DIP.
    pub(crate) fn logical(&self) -> (i32, i32) {
        let w = (self.size.width as f32 / self.scale).round().max(1.0);
        let h = (self.size.height as f32 / self.scale).round().max(1.0);
        (w as i32, h as i32)
    }

    /// What `GetViewRect` reports as x, y, width, height (DIP): the view's
    /// offset in the window, or 0,0 without a screen area.
    pub(crate) fn view_rect_dip(&self) -> (i32, i32, i32, i32) {
        let (w, h) = self.logical();
        let (x, y) = self.area.map_or((0, 0), |a| self.point_to_dip(a.origin.0, a.origin.1));
        (x, y, w, h)
    }

    /// The "screen" for `GetScreenInfo` (rect and available rect) and
    /// `GetRootScreenRect`: the window in DIP. `None` without a screen area,
    /// which leaves CEF's default of using the view.
    ///
    /// Source: cefclient `osr_window_win.cc` `GetRootScreenRect` /
    /// `GetScreenInfo` in its `real_screen_bounds` mode (BSD-3-Clause).
    pub(crate) fn screen_rect_dip(&self) -> Option<(i32, i32, i32, i32)> {
        let a = self.area?;
        let w = (a.window.width as f32 / self.scale).round().max(1.0) as i32;
        let h = (a.window.height as f32 / self.scale).round().max(1.0) as i32;
        Some((0, 0, w, h))
    }

    /// `GetScreenPoint`: a point in the view (DIP) as a screen point in
    /// device pixels (Windows and Linux; macOS wants DIP, see
    /// [`Self::screen_point_dip`]). `None` without a screen area.
    pub(crate) fn screen_point(&self, view_x: i32, view_y: i32) -> Option<(i32, i32)> {
        let a = self.area?;
        Some((
            (view_x as f32 * self.scale).round() as i32 + a.origin.0,
            (view_y as f32 * self.scale).round() as i32 + a.origin.1,
        ))
    }

    /// [`Self::screen_point`] in DIP, the unit macOS CEF expects.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn screen_point_dip(&self, view_x: i32, view_y: i32) -> Option<(i32, i32)> {
        let (x, y) = self.screen_point(view_x, view_y)?;
        Some(self.point_to_dip(x, y))
    }

    /// Host physical coordinates to the DIP that CEF expects for input.
    pub(crate) fn point_to_dip(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (x as f32 / self.scale).round() as i32,
            (y as f32 / self.scale).round() as i32,
        )
    }

    /// CEF hands popup geometry back in DIP; hosts draw in physical pixels.
    pub(crate) fn rect_to_physical(&self, rect: PopupRect) -> PopupRect {
        PopupRect {
            x: (rect.x as f32 * self.scale).round() as i32,
            y: (rect.y as f32 * self.scale).round() as i32,
            width: (rect.width as f32 * self.scale).round() as u32,
            height: (rect.height as f32 * self.scale).round() as u32,
        }
    }
}

/// A zero or negative scale would divide the view to nothing, and CEF rejects
/// absurd factors anyway. Clamp rather than panic: a bad scale from a host is
/// not worth taking the browser down for.
fn sane_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.25, 8.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unscaled_view_is_reported_as_is() {
        let m = ViewMetrics::new(PhysicalSize::new(1280, 800), 1.0);
        assert_eq!(m.logical(), (1280, 800));
        assert_eq!(m.point_to_dip(640, 400), (640, 400));
    }

    #[test]
    fn hidpi_view_is_reported_in_dip() {
        let m = ViewMetrics::new(PhysicalSize::new(2560, 1600), 2.0);
        // CEF lays out 1280x800 CSS pixels and paints a 2560x1600 texture.
        assert_eq!(m.logical(), (1280, 800));
        // A click at the physical centre is the DIP centre.
        assert_eq!(m.point_to_dip(1280, 800), (640, 400));
        // A popup CEF places at DIP 40,80 is drawn at physical 80,160.
        assert_eq!(
            m.rect_to_physical(PopupRect {
                x: 40,
                y: 80,
                width: 200,
                height: 95
            }),
            PopupRect {
                x: 80,
                y: 160,
                width: 400,
                height: 190
            }
        );
    }

    fn area(w: u32, h: u32, x: i32, y: i32) -> ScreenArea {
        ScreenArea {
            window: PhysicalSize::new(w, h),
            origin: (x, y),
        }
    }

    #[test]
    fn default_area_is_todays_behaviour() {
        let m = ViewMetrics::new(PhysicalSize::new(1280, 800), 1.0);
        assert_eq!(m.view_rect_dip(), (0, 0, 1280, 800));
        assert_eq!(m.screen_rect_dip(), None);
        assert_eq!(m.screen_point(10, 20), None);
    }

    #[test]
    fn view_rect_carries_origin() {
        let mut m = ViewMetrics::new(PhysicalSize::new(2000, 1000), 2.0);
        m.set_area(area(2560, 1600, 200, 120));
        assert_eq!(m.view_rect_dip(), (100, 60, 1000, 500));
    }

    #[test]
    fn screen_rect_is_window_in_dip() {
        let mut m = ViewMetrics::new(PhysicalSize::new(800, 600), 1.0);
        m.set_area(area(1000, 700, 10, 20));
        assert_eq!(m.screen_rect_dip(), Some((0, 0, 1000, 700)));
        let mut m = ViewMetrics::new(PhysicalSize::new(800, 600), 2.0);
        m.set_area(area(2000, 1400, 10, 20));
        assert_eq!(m.screen_rect_dip(), Some((0, 0, 1000, 700)));
        let mut m = ViewMetrics::new(PhysicalSize::new(800, 600), 1.5);
        m.set_area(area(1920, 1080, 0, 0));
        assert_eq!(m.screen_rect_dip(), Some((0, 0, 1280, 720)));
    }

    #[test]
    fn screen_point_adds_origin_in_device_px() {
        let mut m = ViewMetrics::new(PhysicalSize::new(2000, 1000), 2.0);
        m.set_area(area(2560, 1600, 200, 120));
        // View DIP 50,10 is device 100,20 in the view, 300,140 in the window.
        assert_eq!(m.screen_point(50, 10), Some((300, 140)));
        assert_eq!(m.screen_point_dip(50, 10), Some((150, 70)));
    }

    #[test]
    fn fractional_scale_rounds_without_collapsing() {
        let m = ViewMetrics::new(PhysicalSize::new(1920, 1080), 1.5);
        assert_eq!(m.logical(), (1280, 720));
    }

    #[test]
    fn nonsense_scales_are_clamped_not_fatal() {
        assert_eq!(
            ViewMetrics::new(PhysicalSize::new(800, 600), 0.0).scale(),
            0.25
        );
        assert_eq!(
            ViewMetrics::new(PhysicalSize::new(800, 600), -3.0).scale(),
            0.25
        );
        assert_eq!(
            ViewMetrics::new(PhysicalSize::new(800, 600), f32::NAN).scale(),
            1.0
        );
        // A degenerate view still reports at least one DIP.
        let m = ViewMetrics::new(PhysicalSize::new(1, 1), 8.0);
        assert_eq!(m.logical(), (1, 1));
    }
}
