//! Spaces (workspaces): an ordered set of tabs with its own pinned section, tint and
//! default container. Placement of tabs lives only here; see `model` for mutation.

use crate::tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpaceId(pub u32);

/// A cookie/cache context. 0 is the built-in Default and is never deletable
/// (Firefox `userContextId` / Zen `containerTabId` convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerId(pub u32);

impl ContainerId {
    pub const DEFAULT: ContainerId = ContainerId(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: ContainerId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// One level of grouping. Holds unpinned tabs only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
    pub tabs: Vec<TabId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Tab(TabId),
    Folder(Folder),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub tint: Rgb,
    /// Default container of tabs without an override. Spaces may share one.
    pub container: ContainerId,
    pub pinned: Vec<TabId>,
    pub items: Vec<Item>,
    /// `Some` exactly when the Space has a tab; remembered while another Space is shown.
    pub active: Option<TabId>,
}

impl Space {
    pub fn folder_mut(&mut self, id: FolderId) -> Option<&mut Folder> {
        self.items.iter_mut().find_map(|i| match i {
            Item::Folder(f) if f.id == id => Some(f),
            _ => None,
        })
    }
}

/// Hue offset of the second and third dye from the Space's tint, degrees.
pub const DYE_HUE_STEP: f64 = 45.0;
/// The lightness band of the design's dyes (tokens.slint `Spaces.dye`: all
/// between about 0.32 and 0.62, most near 0.4), so the liquid stays dark under
/// the glass whatever the tint.
pub const DYE_LIGHTNESS: (f64, f64) = (0.38, 0.45);
/// The saturation band the dyes are held to: a grey tint still gets colour,
/// a neon one is calmed.
pub const DYE_SATURATION: (f64, f64) = (0.45, 0.65);

/// The three colours the backdrop's liquid is dyed with, derived from a
/// Space's one tint: the tint, then its hue turned by +/- [`DYE_HUE_STEP`],
/// all at the design's lightness and saturation. The design hand-picks three
/// per Space (tokens.slint `Spaces`); wubble's Spaces have one tint, so the
/// other two follow from it.
pub fn dye(tint: Rgb) -> [Rgb; 3] {
    let (h, s, l) = hsl(tint);
    let s = s.clamp(DYE_SATURATION.0, DYE_SATURATION.1);
    let l = l.clamp(DYE_LIGHTNESS.0, DYE_LIGHTNESS.1);
    [0.0, DYE_HUE_STEP, -DYE_HUE_STEP].map(|dh| from_hsl((h + dh).rem_euclid(360.0), s, l))
}

/// RGB to hue (degrees), saturation and lightness (0..1). The CSS formula.
fn hsl(Rgb(r, g, b): Rgb) -> (f64, f64, f64) {
    let [r, g, b] = [r, g, b].map(|c| f64::from(c) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

/// Hue (degrees), saturation and lightness to RGB. The CSS formula.
fn from_hsl(h: f64, s: f64, l: f64) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb(byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dye_keeps_the_tint_hue_in_the_design_band() {
        let tint = Rgb(0x6a, 0x8f, 0xff);
        let [a, b, c] = dye(tint);
        let (h0, ..) = hsl(tint);
        let close = |x: f64, y: f64| (x - y).abs() < 1.5;
        for (d, want) in [(a, h0), (b, h0 + DYE_HUE_STEP), (c, h0 - DYE_HUE_STEP)] {
            let (h, s, l) = hsl(d);
            assert!(close(h, want.rem_euclid(360.0)), "{d:?}: hue {h} vs {want}");
            assert!(
                (DYE_SATURATION.0 - 0.01..=DYE_SATURATION.1 + 0.01).contains(&s),
                "{d:?}: s {s}"
            );
            assert!(
                (DYE_LIGHTNESS.0 - 0.01..=DYE_LIGHTNESS.1 + 0.01).contains(&l),
                "{d:?}: l {l}"
            );
        }
        assert_eq!(dye(tint), [a, b, c], "deterministic");
    }

    #[test]
    fn dye_gives_a_grey_tint_colour_and_wraps_hues() {
        let [a, b, c] = dye(Rgb(128, 128, 128));
        assert!(a != b && b != c && a != c);
        // red: hue 0, the third dye wraps to 315
        let (h, ..) = hsl(dye(Rgb(255, 0, 0))[2]);
        assert!((h - 315.0).abs() < 1.5, "hue {h}");
        // HSL round trip of a mid colour
        assert_eq!(
            from_hsl(
                hsl(Rgb(47, 95, 196)).0,
                hsl(Rgb(47, 95, 196)).1,
                hsl(Rgb(47, 95, 196)).2
            ),
            Rgb(47, 95, 196)
        );
    }

    #[test]
    fn container_ids_compare_by_value() {
        assert_eq!(ContainerId(3), ContainerId(3));
        assert_ne!(ContainerId(3), ContainerId(4));
    }
}
