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

/// `r` expanded outward by `pad` on every side.
fn expand(r: &Rect, pad: f32) -> Rect {
    Rect::new(r.x - pad, r.y - pad, r.w + pad * 2.0, r.h + pad * 2.0)
}

/// How far in CSS px page content shows in the blur: three sigma of the chain (about 44 px in
/// all) plus a texel of the coarsest level. A skipped rebuild stays until the next one, so the
/// reach errs wide.
pub const AMBIENCE_REACH: f32 = 150.0;

/// Whether a page damage rectangle `damage` (window logical px) can visibly affect either an
/// overlay glass surface (`layer.is_some()`) or the ambience in the under-page glass around any
/// pane (`pages`). A damage rect deep inside a pane (more than [`AMBIENCE_REACH`] from every edge
/// of its pane) and away from every overlay does not need to rebuild the page blur pyramid `P`.
pub fn damage_reaches_glass(
    damage: &Rect,
    surfaces: &[GlassSurface],
    pages: &[PageRect],
    ambience: bool,
) -> bool {
    if damage.w <= 0.0 || damage.h <= 0.0 {
        return false;
    }
    if surfaces
        .iter()
        .any(|s| s.layer.is_some() && intersect(damage, &expand(&rect_of(s), AMBIENCE_REACH)))
    {
        return true;
    }
    if !ambience || !surfaces.iter().any(|s| s.layer.is_none()) {
        return false;
    }
    // Under-page glass is visible around each pane's edges (sidebar and gutters); damage inside
    // a pane reaches it only when within `AMBIENCE_REACH` of any edge of that pane.
    pages.iter().any(|p| {
        if !intersect(damage, &p.rect) {
            return false;
        }
        let inner_w = p.rect.w - 2.0 * AMBIENCE_REACH;
        let inner_h = p.rect.h - 2.0 * AMBIENCE_REACH;
        if inner_w <= 0.0 || inner_h <= 0.0 {
            return true;
        }
        let inner = Rect::new(
            p.rect.x + AMBIENCE_REACH,
            p.rect.y + AMBIENCE_REACH,
            inner_w,
            inner_h,
        );
        // Contained strictly inside the deep interior => cannot reach any pane edge.
        !(damage.x >= inner.x
            && damage.y >= inner.y
            && damage.x + damage.w <= inner.x + inner.w
            && damage.y + damage.h <= inner.y + inner.h)
    })
}

/// Whether `item` lies over any of the pages, by rectangle (a rounded corner counts).
pub fn overlaps(item: &Rect, pages: &[PageRect]) -> bool {
    pages.iter().any(|p| intersect(item, &p.rect))
}

/// Whether an overlay's glass (one drawn in an [`OverLayer`]) lies over any of `pages`.
pub fn overlay_over(surfaces: &[GlassSurface], pages: &[PageRect]) -> bool {
    surfaces
        .iter()
        .any(|s| s.layer.is_some() && overlaps(&rect_of(s), pages))
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

    #[test]
    fn damage_deep_inside_a_pane_does_not_reach_glass() {
        let pages = [page(240.0, 8.0, 1000.0, 780.0)];
        let window_glass = [GlassSurface {
            x: 0.0,
            y: 0.0,
            w: 1248.0,
            h: 796.0,
            layer: None,
            ..Default::default()
        }];
        // Blinking cursor in the middle of the page: farther than the reach from every pane edge
        let middle = Rect::new(600.0, 350.0, 20.0, 20.0);
        assert!(!damage_reaches_glass(&middle, &window_glass, &pages, true));
        // Damage near the sidebar edge (within the reach of x=240): reaches ambience
        let near_edge = Rect::new(260.0, 350.0, 20.0, 20.0);
        assert!(damage_reaches_glass(
            &near_edge,
            &window_glass,
            &pages,
            true
        ));
        assert!(!damage_reaches_glass(
            &near_edge,
            &window_glass,
            &pages,
            false
        ));
        // An overlay over the middle of the page: middle damage reaches it even with ambience off
        let with_overlay = [
            window_glass[0].clone(),
            GlassSurface {
                x: 550.0,
                y: 300.0,
                w: 120.0,
                h: 80.0,
                layer: Some(OverLayer::Card),
                ..Default::default()
            },
        ];
        assert!(damage_reaches_glass(&middle, &with_overlay, &pages, false));
    }
}
