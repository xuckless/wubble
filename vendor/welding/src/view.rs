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
/// `<select>` popup outside the page rectangle, and (with `desktop`) gives
/// the page Chrome's `screen.*`, `window.screenX/Y` and `outerWidth/Height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenArea {
    /// The host window's size in physical pixels.
    pub window: PhysicalSize<u32>,
    /// The view's top-left corner inside the window, in physical pixels.
    pub origin: (i32, i32),
    /// Where the window and its monitor are; `None` makes the window the
    /// screen, at 0,0.
    pub desktop: Option<Desktop>,
}

/// wubble change (58): the window's monitor and the window on the desktop,
/// physical pixels in the platform's global space (the host reads them from
/// winit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Desktop {
    pub monitor_origin: (i32, i32),
    pub monitor_size: PhysicalSize<u32>,
    /// The window's top-left; `None` where the platform will not say
    /// (Wayland): the window is then taken at the monitor's origin, as Chrome
    /// reports it there.
    pub window_origin: Option<(i32, i32)>,
}

impl ScreenArea {
    /// The window's top-left on the desktop (physical): the platform's, else
    /// the monitor's origin, else 0,0 (the window is the screen).
    fn window_origin(&self) -> (i32, i32) {
        self.desktop
            .and_then(|d| d.window_origin.or(Some(d.monitor_origin)))
            .unwrap_or((0, 0))
    }
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
    /// place on the desktop (its offset in the window, from the window's
    /// origin), or 0,0 without a screen area.
    pub(crate) fn view_rect_dip(&self) -> (i32, i32, i32, i32) {
        let (w, h) = self.logical();
        let (x, y) = self.area.map_or((0, 0), |a| {
            let o = a.window_origin();
            self.point_to_dip(o.0 + a.origin.0, o.1 + a.origin.1)
        });
        (x, y, w, h)
    }

    /// A physical rectangle in DIP, at least 1x1.
    fn rect_dip(&self, x: i32, y: i32, w: u32, h: u32) -> (i32, i32, i32, i32) {
        let (x, y) = self.point_to_dip(x, y);
        let dip = |n: u32| (n as f32 / self.scale).round().max(1.0) as i32;
        (x, y, dip(w), dip(h))
    }

    /// The screen for `GetScreenInfo` (rect and available rect) in DIP: the
    /// window's monitor (winit has no work area, so both are the monitor), or
    /// the window itself without one. `None` without a screen area, which
    /// leaves CEF's default of using the view.
    ///
    /// Source: cefclient `osr_window_win.cc` `GetScreenInfo` (BSD-3-Clause).
    pub(crate) fn screen_rect_dip(&self) -> Option<(i32, i32, i32, i32)> {
        let a = self.area?;
        Some(match a.desktop {
            Some(d) => self.rect_dip(
                d.monitor_origin.0,
                d.monitor_origin.1,
                d.monitor_size.width,
                d.monitor_size.height,
            ),
            None => self.rect_dip(0, 0, a.window.width, a.window.height),
        })
    }

    /// `GetRootScreenRect`: the window's outer rectangle on the desktop in
    /// DIP (`window.screenX/Y`, `outerWidth/Height`). `None` without a screen
    /// area.
    ///
    /// Source: cefclient `osr_window_win.cc` `GetRootScreenRect`
    /// (BSD-3-Clause).
    pub(crate) fn root_rect_dip(&self) -> Option<(i32, i32, i32, i32)> {
        let a = self.area?;
        let o = a.window_origin();
        Some(self.rect_dip(o.0, o.1, a.window.width, a.window.height))
    }

    /// `GetScreenPoint`: a point in the view (DIP) as a screen point in
    /// device pixels (Windows and Linux; macOS wants DIP, see
    /// [`Self::screen_point_dip`]). `None` without a screen area.
    pub(crate) fn screen_point(&self, view_x: i32, view_y: i32) -> Option<(i32, i32)> {
        let a = self.area?;
        let o = a.window_origin();
        Some((
            (view_x as f32 * self.scale).round() as i32 + o.0 + a.origin.0,
            (view_y as f32 * self.scale).round() as i32 + o.1 + a.origin.1,
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
            desktop: None,
        }
    }

    /// wubble change (58): one row per desktop layout; every rectangle in DIP,
    /// the screen point in device pixels (view point 10,20).
    struct Desk {
        name: &'static str,
        scale: f32,
        view: (u32, u32),
        window: (u32, u32),
        origin: (i32, i32),
        monitor: Option<((i32, i32), (u32, u32))>,
        window_origin: Option<(i32, i32)>,
        screen: (i32, i32, i32, i32),
        root: (i32, i32, i32, i32),
        view_rect: (i32, i32, i32, i32),
        point: (i32, i32),
    }

    #[test]
    fn the_page_sees_its_monitor_and_window_on_the_desktop() {
        let rows = [
            Desk {
                name: "1x primary, window inside it",
                scale: 1.0,
                view: (1000, 620),
                window: (1000, 700),
                origin: (0, 80),
                monitor: Some(((0, 0), (1920, 1080))),
                window_origin: Some((100, 50)),
                screen: (0, 0, 1920, 1080),
                root: (100, 50, 1000, 700),
                view_rect: (100, 130, 1000, 620),
                point: (110, 150),
            },
            Desk {
                name: "monitor left of the primary, negative origin",
                scale: 1.0,
                view: (900, 600),
                window: (1000, 700),
                origin: (50, 80),
                monitor: Some(((-1920, 0), (1920, 1080))),
                window_origin: Some((-1800, 40)),
                screen: (-1920, 0, 1920, 1080),
                root: (-1800, 40, 1000, 700),
                view_rect: (-1750, 120, 900, 600),
                point: (-1740, 140),
            },
            Desk {
                name: "1.25x monitor right of the primary",
                scale: 1.25,
                view: (1250, 775),
                window: (1250, 875),
                origin: (0, 100),
                monitor: Some(((1920, 0), (2560, 1440))),
                window_origin: Some((2000, 100)),
                screen: (1536, 0, 2048, 1152),
                root: (1600, 80, 1000, 700),
                view_rect: (1600, 160, 1000, 620),
                point: (2013, 225),
            },
            Desk {
                name: "2x",
                scale: 2.0,
                view: (1600, 1200),
                window: (2000, 1400),
                origin: (400, 120),
                monitor: Some(((0, 0), (3840, 2160))),
                window_origin: Some((200, 100)),
                screen: (0, 0, 1920, 1080),
                root: (100, 50, 1000, 700),
                view_rect: (300, 110, 800, 600),
                point: (620, 260),
            },
            Desk {
                name: "Wayland: no position, the window sits at the monitor's origin",
                scale: 1.0,
                view: (1000, 620),
                window: (1000, 700),
                origin: (0, 80),
                monitor: Some(((0, 0), (2560, 1440))),
                window_origin: None,
                screen: (0, 0, 2560, 1440),
                root: (0, 0, 1000, 700),
                view_rect: (0, 80, 1000, 620),
                point: (10, 100),
            },
            Desk {
                name: "Wayland on a second monitor",
                scale: 1.0,
                view: (1000, 620),
                window: (1000, 700),
                origin: (0, 80),
                monitor: Some(((1920, 0), (1920, 1080))),
                window_origin: None,
                screen: (1920, 0, 1920, 1080),
                root: (1920, 0, 1000, 700),
                view_rect: (1920, 80, 1000, 620),
                point: (1930, 100),
            },
            Desk {
                name: "no monitor known: the window is the screen",
                scale: 1.5,
                view: (1200, 600),
                window: (1920, 1080),
                origin: (150, 75),
                monitor: None,
                window_origin: None,
                screen: (0, 0, 1280, 720),
                root: (0, 0, 1280, 720),
                view_rect: (100, 50, 800, 400),
                point: (165, 105),
            },
        ];
        for r in rows {
            let mut m = ViewMetrics::new(PhysicalSize::new(r.view.0, r.view.1), r.scale);
            m.set_area(ScreenArea {
                window: PhysicalSize::new(r.window.0, r.window.1),
                origin: r.origin,
                desktop: r.monitor.map(|(origin, size)| Desktop {
                    monitor_origin: origin,
                    monitor_size: PhysicalSize::new(size.0, size.1),
                    window_origin: r.window_origin,
                }),
            });
            assert_eq!(m.screen_rect_dip(), Some(r.screen), "{}: screen", r.name);
            assert_eq!(m.root_rect_dip(), Some(r.root), "{}: root", r.name);
            assert_eq!(m.view_rect_dip(), r.view_rect, "{}: view", r.name);
            assert_eq!(m.screen_point(10, 20), Some(r.point), "{}: point", r.name);
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
