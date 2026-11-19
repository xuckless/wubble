//! The page in the scene (wubble, no design counterpart in the port: the web gets it from
//! `backdrop-filter`): overlay glass over a pane blurs the live page, and the window's glass
//! picks up a little of the page's light at its edge (ambience, owner-requested).
//!
//! The scene keeps a second blur pyramid, `P`: the backdrop with every pane's latest frame drawn
//! on it (at the pyramid's half resolution), then blurred like the backdrop's. Overlay glass over
//! a pane reads `P`; the window's glass mixes a little of `P` in. This module is the pure part:
//! where the panes are, which glass is over them, and when `P` is built ([`page_blur`]).

use std::time::Duration;

use super::state::{GlassSurface, Rect};

/// Where an overlay's glass is drawn: in one of the window's images above the page, each at the
/// place in Slint's tree of the overlays it serves (`app.slint`). Glass without one is drawn
/// under the page, in the scene's back-most image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverLayer {
    /// In the page card, over the pages: find bar, toast, sheets, the command bar, video controls.
    Card,
    /// Under the peeking sidebar's content.
    Peek,
    /// Over the whole window: context menus, popovers, prompts.
    Top,
}

impl OverLayer {
    pub const ALL: [OverLayer; 3] = [OverLayer::Card, OverLayer::Peek, OverLayer::Top];

    /// The Slint `GlassItem.layer` value (`GlassOver`, model.slint): 0 is under the page.
    pub fn from_index(i: i32) -> Option<OverLayer> {
        match i {
            1 => Some(OverLayer::Card),
            2 => Some(OverLayer::Peek),
            3 => Some(OverLayer::Top),
            _ => None,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            OverLayer::Card => 0,
            OverLayer::Peek => 1,
            OverLayer::Top => 2,
        }
    }
}

/// One pane's page card in the window, logical px, top-left origin.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageRect {
    pub rect: Rect,
    pub radius: f32,
}

/// Whether `a` and `b` share area (touching edges do not).
fn intersect(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Whether `item` lies over any of the pages, by rectangle (a rounded corner counts).
pub fn overlaps(item: &Rect, pages: &[PageRect]) -> bool {
    pages.iter().any(|p| intersect(item, &p.rect))
}

/// A surface's rectangle.
pub fn rect_of(s: &GlassSurface) -> Rect {
    Rect::new(s.x as f32, s.y as f32, s.w as f32, s.h as f32)
}

/// The surfaces of each layer, by index into `surfaces`, back to front: `[under, card, peek,
/// top]`.
pub fn by_layer(surfaces: &[GlassSurface]) -> [Vec<usize>; 4] {
    let mut out: [Vec<usize>; 4] = Default::default();
    for (i, s) in surfaces.iter().enumerate() {
        out[s.layer.map_or(0, |l| l.index() + 1)].push(i);
    }
    out
}

/// What [`page_blur`] decides from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageBlurIn {
    /// Some pane has a frame to draw.
    pub pages: bool,
    /// An overlay's glass (drawn this frame) lies over a pane.
    pub overlay: bool,
    /// The window's glass picks up the page's light.
    pub ambience: bool,
    /// A pane's frame or place changed since `P` was built.
    pub stale: bool,
    /// The backdrop under the pages was rendered again this frame (palette, size).
    pub backdrop_new: bool,
    /// `P` exists and was built since it was (re)allocated.
    pub built: bool,
    /// The lowest quality level: the glass is a flat fill, `P` is not rebuilt once it exists.
    pub flat: bool,
    /// Since `P` was last built.
    pub since_build: Duration,
    /// The ambience's update period ([`super::AMBIENCE_HZ`]).
    pub ambience_every: Duration,
}

/// Whether to build `P` this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBlur {
    Skip,
    Build,
    /// Build once this much more time has passed (the ambience's rate): the host wakes then.
    After(Duration),
}

/// When the page-inclusive blur is built. Nothing over a page and no ambience costs nothing; an
/// overlay over a page follows every page frame (a video behind a menu moves); the ambience alone
/// follows at its own low rate; at `flat` the last build stays.
pub fn page_blur(i: &PageBlurIn) -> PageBlur {
    if !i.pages || !(i.overlay || i.ambience) {
        return PageBlur::Skip;
    }
    if !i.built {
        return PageBlur::Build;
    }
    if i.flat || !(i.stale || i.backdrop_new) {
        return PageBlur::Skip;
    }
    if i.overlay || i.backdrop_new || i.since_build >= i.ambience_every {
        return PageBlur::Build;
    }
    PageBlur::After(i.ambience_every - i.since_build)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(x: f32, y: f32, w: f32, h: f32) -> PageRect {
        PageRect {
            rect: Rect::new(x, y, w, h),
            radius: 12.0,
        }
    }

    #[test]
    fn an_item_is_over_a_page_when_they_share_area() {
        let pages = [page(300.0, 8.0, 500.0, 600.0)];
        // inside, partly over, beside, touching the edge
        assert!(overlaps(&Rect::new(400.0, 100.0, 100.0, 50.0), &pages));
        assert!(overlaps(&Rect::new(250.0, 100.0, 100.0, 50.0), &pages));
        assert!(!overlaps(&Rect::new(10.0, 100.0, 100.0, 50.0), &pages));
        assert!(!overlaps(&Rect::new(200.0, 100.0, 100.0, 50.0), &pages));
        assert!(!overlaps(&Rect::new(400.0, 608.0, 10.0, 10.0), &pages));
        // a rounded corner counts by its rectangle
        assert!(overlaps(&Rect::new(296.0, 4.0, 6.0, 6.0), &pages));
        // a split: either pane
        let split = [
            page(300.0, 8.0, 240.0, 600.0),
            page(548.0, 8.0, 240.0, 600.0),
        ];
        assert!(!overlaps(&Rect::new(541.0, 50.0, 6.0, 6.0), &split));
        assert!(overlaps(&Rect::new(560.0, 50.0, 6.0, 6.0), &split));
        assert!(!overlaps(&Rect::new(0.0, 0.0, 10.0, 10.0), &[]));
    }

    #[test]
    fn surfaces_split_by_layer_in_order() {
        let s = |layer| GlassSurface {
            layer,
            ..Default::default()
        };
        let list = [
            s(None),
            s(Some(OverLayer::Top)),
            s(None),
            s(Some(OverLayer::Card)),
            s(Some(OverLayer::Top)),
            s(Some(OverLayer::Peek)),
        ];
        assert_eq!(
            by_layer(&list),
            [vec![0usize, 2], vec![3], vec![5], vec![1, 4]]
        );
        assert!(by_layer(&[]).iter().all(Vec::is_empty));
    }

    #[test]
    fn the_slint_layer_index_maps_both_ways() {
        assert_eq!(OverLayer::from_index(0), None);
        assert_eq!(OverLayer::from_index(4), None);
        for (n, l) in OverLayer::ALL.into_iter().enumerate() {
            assert_eq!(OverLayer::from_index(n as i32 + 1), Some(l));
            assert_eq!(l.index(), n);
        }
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn need() -> PageBlurIn {
        PageBlurIn {
            pages: true,
            overlay: true,
            ambience: false,
            stale: false,
            backdrop_new: false,
            built: true,
            flat: false,
            since_build: MS(10),
            ambience_every: MS(125),
        }
    }

    #[test]
    fn nothing_over_the_page_costs_nothing() {
        use PageBlur::*;
        let none = PageBlurIn {
            overlay: false,
            ambience: false,
            stale: true,
            built: false,
            ..need()
        };
        assert_eq!(
            page_blur(&none),
            Skip,
            "no overlay over a pane, no ambience"
        );
        let no_pages = PageBlurIn {
            pages: false,
            stale: true,
            built: false,
            ..need()
        };
        assert_eq!(page_blur(&no_pages), Skip, "no pane has a frame");
        // a page frame while nothing needs it: still nothing
        assert_eq!(
            page_blur(&PageBlurIn {
                stale: true,
                ..none
            }),
            Skip
        );
    }

    #[test]
    fn the_first_need_builds_even_when_flat() {
        let first = PageBlurIn {
            built: false,
            ..need()
        };
        assert_eq!(page_blur(&first), PageBlur::Build);
        assert_eq!(
            page_blur(&PageBlurIn {
                flat: true,
                ..first
            }),
            PageBlur::Build
        );
        let ambience = PageBlurIn {
            overlay: false,
            ambience: true,
            built: false,
            ..need()
        };
        assert_eq!(page_blur(&ambience), PageBlur::Build);
    }

    #[test]
    fn an_overlay_follows_every_page_frame() {
        use PageBlur::*;
        assert_eq!(
            page_blur(&PageBlurIn {
                stale: true,
                ..need()
            }),
            Build
        );
        // no new frame: the last build stays
        assert_eq!(page_blur(&need()), Skip);
        // the backdrop under the pages changed
        assert_eq!(
            page_blur(&PageBlurIn {
                backdrop_new: true,
                ..need()
            }),
            Build
        );
        // flat: never rebuilt once built
        assert_eq!(
            page_blur(&PageBlurIn {
                stale: true,
                flat: true,
                ..need()
            }),
            Skip
        );
    }

    #[test]
    fn the_ambience_alone_follows_at_its_rate() {
        use PageBlur::*;
        let amb = PageBlurIn {
            overlay: false,
            ambience: true,
            stale: true,
            ..need()
        };
        // 10 ms after the last build, 125 ms period: wait 115 ms more
        assert_eq!(page_blur(&amb), After(MS(115)));
        assert_eq!(
            page_blur(&PageBlurIn {
                since_build: MS(125),
                ..amb
            }),
            Build
        );
        assert_eq!(
            page_blur(&PageBlurIn {
                since_build: MS(400),
                ..amb
            }),
            Build
        );
        // a still page: nothing, however long
        assert_eq!(
            page_blur(&PageBlurIn {
                stale: false,
                since_build: MS(400),
                ..amb
            }),
            Skip
        );
        // a new backdrop does not wait
        assert_eq!(
            page_blur(&PageBlurIn {
                stale: false,
                backdrop_new: true,
                ..amb
            }),
            Build
        );
        // an overlay over the page as well: every frame
        assert_eq!(
            page_blur(&PageBlurIn {
                overlay: true,
                ..amb
            }),
            Build
        );
    }
}
