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

/// A Space's drawn icon: the design's three (`slint/space-icons.json`, the shell's
/// `SpaceIconPaths`), by the id the shell draws it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpaceIcon {
    Personal,
    Work,
    Study,
}

impl SpaceIcon {
    pub const ALL: [SpaceIcon; 3] = [SpaceIcon::Personal, SpaceIcon::Work, SpaceIcon::Study];

    pub fn id(self) -> &'static str {
        match self {
            SpaceIcon::Personal => "personal",
            SpaceIcon::Work => "work",
            SpaceIcon::Study => "study",
        }
    }

    pub fn from_id(id: &str) -> Option<SpaceIcon> {
        Self::ALL.into_iter().find(|i| i.id() == id)
    }

    /// The icon a new Space at `index` gets: the three in turn, as the sidebar drew them
    /// before Spaces had their own.
    pub fn nth(index: usize) -> SpaceIcon {
        Self::ALL[index % Self::ALL.len()]
    }
}

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
    pub icon: SpaceIcon,
    /// Default container of tabs without an override. Spaces may share one.
    pub container: ContainerId,
    pub pinned: Vec<TabId>,
    pub items: Vec<Item>,
    /// `Some` exactly when the Space has a tab; remembered while another Space is shown.
    pub active: Option<TabId>,
    /// Pages shown side by side; shown while `active` is one of them, remembered otherwise.
    pub split: Option<SplitGroup>,
}

/// Most pages side by side (the design's `SPLIT.maxPanes`).
pub const MAX_PANES: usize = crate::split::SPLIT::maxPanes;

/// 2 to [`MAX_PANES`] tabs of one Space, left to right, and where the seams between them rest.
/// The design's split group (browser/model.ts). `stops` are the only stored layout: seam
/// positions as fractions of the width, strictly increasing in (0, 1), one fewer than `tabs`;
/// the panes' shares are derived from them ([`SplitGroup::ratios`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitGroup {
    pub tabs: Vec<TabId>,
    pub stops: Vec<f64>,
}

impl SplitGroup {
    /// Each pane's share of the width, left to right. Sums to 1 by construction.
    pub fn ratios(&self) -> Vec<f64> {
        let edges: Vec<f64> = std::iter::once(0.0)
            .chain(self.stops.iter().copied())
            .chain(std::iter::once(1.0))
            .collect();
        edges.windows(2).map(|w| w[1] - w[0]).collect()
    }

    /// Whether `stops` fit `n` panes: n - 1 of them, strictly increasing inside (0, 1).
    pub fn stops_fit(stops: &[f64], n: usize) -> bool {
        stops.len() + 1 == n
            && stops.iter().all(|s| s.is_finite() && *s > 0.0 && *s < 1.0)
            && stops.windows(2).all(|w| w[0] < w[1])
    }
}

/// `even(n)`: n - 1 stops at equal shares. Source: the design repo, `slint/split.rs` `even`.
pub fn even(n: usize) -> Vec<f64> {
    (0..n.saturating_sub(1))
        .map(|j| (j as f64 + 1.0) / n as f64)
        .collect()
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
    fn space_icons_round_trip_and_rotate() {
        for i in SpaceIcon::ALL {
            assert_eq!(SpaceIcon::from_id(i.id()), Some(i));
        }
        assert_eq!(SpaceIcon::from_id("rocket"), None);
        assert_eq!(SpaceIcon::nth(0), SpaceIcon::Personal);
        assert_eq!(SpaceIcon::nth(4), SpaceIcon::Work);
    }

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
    fn ratios_are_derived_from_stops_and_sum_to_one() {
        let g = SplitGroup {
            tabs: vec![TabId(1), TabId(2), TabId(3)],
            stops: vec![0.25, 0.75],
        };
        assert_eq!(g.ratios(), vec![0.25, 0.5, 0.25]);
        assert_eq!(g.ratios().iter().sum::<f64>(), 1.0);
        assert_eq!(even(3), vec![1.0 / 3.0, 2.0 / 3.0]);
        assert!(SplitGroup::stops_fit(&even(3), 3));
        assert!(!SplitGroup::stops_fit(&[0.5], 3));
        assert!(!SplitGroup::stops_fit(&[0.6, 0.4], 3));
        assert!(!SplitGroup::stops_fit(&[0.0], 2));
        assert!(!SplitGroup::stops_fit(&[f64::NAN], 2));
    }

    #[test]
    fn container_ids_compare_by_value() {
        assert_eq!(ContainerId(3), ContainerId(3));
        assert_ne!(ContainerId(3), ContainerId(4));
    }
}
