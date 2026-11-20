//! `--smoke overlay` (hand run only): overlay glass over the live page frosts the page, and the
//! window's glass picks up the page's light (the scene's page blur, `wubble_gfx::scene::pages`).
//! The bundled `overlay.html` is green across its top and red below (`#dark`: the same, dark).
//! [`STEPS`] is the whole run as data: each step puts something on the shell, waits for it to
//! settle, snapshots the window and checks it ([`check`], pure). Optionally each step's snapshot
//! is written to `--shot-dir`.
//!
//! - the glass in the gutter between the sidebar and the page, ambience off then on: on, it is
//!   redder (the page's red leaks into it);
//! - a context menu over the red part, the command bar over the green top, the peeking sidebar
//!   over the red part: the glass under each has the page's hue, not the backdrop's;
//! - with nothing open and no ambience, the page's own repaints (its animated box) build no
//!   page blur: a count, not a clock;
//! - the overlays' rules (`wubble_core::overlay`): Print over Add bookmark leaves Print alone,
//!   Escape closes each kind (the topmost first), a tab switch closes the sheet and hides the
//!   find bar, which shows on its own tab only, and a navigation closes it.

/// What a step puts on the shell before it settles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    /// The ambience on or off, nothing open.
    Ambience(bool),
    /// The tab row's context menu, over the page's red part.
    Menu,
    /// The command bar (over the page's green top).
    Bar,
    /// The sidebar hidden, peeking over the page.
    Peek,
    /// Nothing open, the sidebar back, no ambience.
    Quiet,
    /// Nothing new: the page keeps repainting.
    Wait,
    /// Add bookmark (Mod+D's sheet).
    Bookmark,
    /// The print sheet (Mod+P).
    Print,
    /// The find bar (Mod+F).
    Find,
    /// The screenshot region picker.
    Region,
    /// One Escape press, as the keyboard sends it.
    Escape,
    /// A second tab on the same page, shown.
    OtherTab,
    /// The first tab shown again.
    FirstTab,
    /// The shown tab loads its page again (a new document).
    Navigate,
}

/// Which overlays the shell shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub bookmark: bool,
    pub print: bool,
    pub find: bool,
    pub menu: bool,
    pub bar: bool,
    pub region: bool,
}

const NONE: Shown = Shown {
    bookmark: false,
    print: false,
    find: false,
    menu: false,
    bar: false,
    region: false,
};
const FIND: Shown = Shown { find: true, ..NONE };

/// The overlay panels a step checks the glass of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Menu,
    Bar,
    Peek,
}

/// What a step checks once settled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Check {
    /// Notes the gutter's redness.
    GutterBase,
    /// The gutter is redder than the noted one.
    GutterRedder,
    /// The glass under the panel has this hue (degrees).
    Hue(Panel, f32),
    /// Notes the page blur's build count.
    Builds,
    /// The build count is the noted one.
    BuildsHeld,
    /// Exactly these overlays show.
    Overlays(Shown),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub name: &'static str,
    pub setup: Setup,
    pub check: Check,
}

pub const RED: f32 = 0.0;
pub const GREEN: f32 = 120.0;
/// How far the glass's hue may be from the page's (degrees).
pub const HUE_TOLERANCE: f32 = 25.0;
/// The least chroma (0-255) a coloured glass has: the backdrop's frost is far greyer.
pub const MIN_CHROMA: f32 = 20.0;
/// How much redder (0-255) the gutter is with the ambience on.
pub const MIN_AMBIENCE: f32 = 2.0;

const fn step(name: &'static str, setup: Setup, check: Check) -> Step {
    Step { name, setup, check }
}

pub const STEPS: &[Step] = &[
    step("ambience-off", Setup::Ambience(false), Check::GutterBase),
    step("ambience-on", Setup::Ambience(true), Check::GutterRedder),
    step("menu-over-page", Setup::Menu, Check::Hue(Panel::Menu, RED)),
    step(
        "command-bar-over-page",
        Setup::Bar,
        Check::Hue(Panel::Bar, GREEN),
    ),
    step("peek-over-page", Setup::Peek, Check::Hue(Panel::Peek, RED)),
    step("quiet", Setup::Quiet, Check::Builds),
    step("repaints", Setup::Wait, Check::BuildsHeld),
    step(
        "add-bookmark",
        Setup::Bookmark,
        Check::Overlays(Shown {
            bookmark: true,
            ..NONE
        }),
    ),
    step(
        "print-replaces-bookmark",
        Setup::Print,
        Check::Overlays(Shown {
            print: true,
            ..NONE
        }),
    ),
    step("escape-print", Setup::Escape, Check::Overlays(NONE)),
    step("find", Setup::Find, Check::Overlays(FIND)),
    step(
        "menu-over-find",
        Setup::Menu,
        Check::Overlays(Shown { menu: true, ..FIND }),
    ),
    step("escape-menu", Setup::Escape, Check::Overlays(FIND)),
    step(
        "bookmark-over-find",
        Setup::Bookmark,
        Check::Overlays(Shown {
            bookmark: true,
            ..FIND
        }),
    ),
    step("escape-bookmark", Setup::Escape, Check::Overlays(FIND)),
    step("escape-find", Setup::Escape, Check::Overlays(NONE)),
    step(
        "region",
        Setup::Region,
        Check::Overlays(Shown {
            region: true,
            ..NONE
        }),
    ),
    step("escape-region", Setup::Escape, Check::Overlays(NONE)),
    step(
        "bar",
        Setup::Bar,
        Check::Overlays(Shown { bar: true, ..NONE }),
    ),
    step("escape-bar", Setup::Escape, Check::Overlays(NONE)),
    step("find-again", Setup::Find, Check::Overlays(FIND)),
    step(
        "bookmark-again",
        Setup::Bookmark,
        Check::Overlays(Shown {
            bookmark: true,
            ..FIND
        }),
    ),
    step("other-tab", Setup::OtherTab, Check::Overlays(NONE)),
    step("first-tab", Setup::FirstTab, Check::Overlays(FIND)),
    step("navigate", Setup::Navigate, Check::Overlays(NONE)),
];

/// What the host measured of a settled step's snapshot. RGB means, 0-255.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seen {
    /// The glass in the gutter left of the page, at the height of its red part.
    pub gutter: [f32; 3],
    /// Inside the checked panel's glass, when the step checks one and it is there.
    pub panel: Option<[f32; 3]>,
    /// The scene's image of the checked panel's layer is on screen (`over-*` in app.slint): the
    /// panel's own fill is transparent while the scene draws its glass, so without the image
    /// the sharp page shows through with the page's hue as well.
    pub layer_image: bool,
    /// The page blur's build count.
    pub builds: u64,
    pub overlays: Shown,
}

/// What the run has noted so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Notes {
    pub gutter: Option<f32>,
    pub builds: Option<u64>,
}

/// Hue (degrees, 0-360) and chroma (max - min) of an RGB colour.
pub fn hue_of([r, g, b]: [f32; 3]) -> (f32, f32) {
    let max = r.max(g).max(b);
    let chroma = max - r.min(g).min(b);
    if chroma <= f32::EPSILON {
        return (0.0, 0.0);
    }
    let h = if max == r {
        ((g - b) / chroma).rem_euclid(6.0)
    } else if max == g {
        (b - r) / chroma + 2.0
    } else {
        (r - g) / chroma + 4.0
    };
    (h * 60.0, chroma)
}

/// Whether two hues are within `deg` of each other, round the circle.
pub fn hue_close(a: f32, b: f32, deg: f32) -> bool {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d) <= deg
}

/// How much redder than grey: red over the mean of green and blue.
pub fn redness([r, g, b]: [f32; 3]) -> f32 {
    r - (g + b) / 2.0
}

/// A settled step's check: the notes after it, or why it failed.
pub fn check(step: &Step, seen: &Seen, notes: Notes) -> Result<Notes, String> {
    match step.check {
        Check::GutterBase => Ok(Notes {
            gutter: Some(redness(seen.gutter)),
            ..notes
        }),
        Check::GutterRedder => {
            let base = notes.gutter.ok_or("no ambience-off gutter noted")?;
            let now = redness(seen.gutter);
            if now >= base + MIN_AMBIENCE {
                Ok(notes)
            } else {
                Err(format!(
                    "the gutter is not redder with the ambience: {now:.1} against {base:.1} \
                     ({:?})",
                    seen.gutter
                ))
            }
        }
        Check::Hue(panel, want) => {
            let rgb = seen.panel.ok_or(format!("no {panel:?} glass on screen"))?;
            if !seen.layer_image {
                return Err(format!(
                    "the scene's glass of the {panel:?}'s layer is not on screen"
                ));
            }
            let (hue, chroma) = hue_of(rgb);
            if chroma >= MIN_CHROMA && hue_close(hue, want, HUE_TOLERANCE) {
                Ok(notes)
            } else {
                Err(format!(
                    "the glass under the {panel:?} is not the page's: {rgb:?} hue {hue:.0} \
                     chroma {chroma:.0}, want hue {want:.0}"
                ))
            }
        }
        Check::Builds => Ok(Notes {
            builds: Some(seen.builds),
            ..notes
        }),
        Check::Overlays(want) => {
            if seen.overlays == want {
                Ok(notes)
            } else {
                Err(format!(
                    "the shell shows {:?}, want {want:?}",
                    seen.overlays
                ))
            }
        }
        Check::BuildsHeld => {
            let noted = notes.builds.ok_or("no build count noted")?;
            if seen.builds == noted {
                Ok(notes)
            } else {
                Err(format!(
                    "the page blur was built {} times while nothing was over the page",
                    seen.builds - noted
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hues_of_the_primaries_and_grey() {
        assert_eq!(hue_of([255.0, 0.0, 0.0]), (0.0, 255.0));
        assert_eq!(hue_of([0.0, 255.0, 0.0]), (120.0, 255.0));
        assert_eq!(hue_of([0.0, 0.0, 255.0]), (240.0, 255.0));
        assert_eq!(hue_of([255.0, 0.0, 255.0]).0, 300.0);
        assert_eq!(hue_of([90.0, 90.0, 90.0]), (0.0, 0.0));
        // a milky frost of red keeps red's hue at a lower chroma
        let (h, c) = hue_of([240.0, 150.0, 150.0]);
        assert_eq!((h, c), (0.0, 90.0));
    }

    #[test]
    fn hues_are_close_round_the_circle() {
        assert!(hue_close(350.0, 0.0, 25.0));
        assert!(hue_close(10.0, 350.0, 25.0));
        assert!(hue_close(110.0, 120.0, 25.0));
        assert!(!hue_close(60.0, 120.0, 25.0));
        assert!(!hue_close(200.0, 0.0, 25.0));
    }

    #[test]
    fn the_steps_check_what_they_promise() {
        let seen = |gutter, panel| Seen {
            gutter,
            panel,
            layer_image: true,
            builds: 3,
            overlays: NONE,
        };
        let base = check(&STEPS[0], &seen([60.0, 60.0, 80.0], None), Notes::default()).unwrap();
        assert_eq!(base.gutter, Some(-10.0));
        // redder by more than the floor passes; the same fails
        assert!(check(&STEPS[1], &seen([70.0, 58.0, 78.0], None), base).is_ok());
        assert!(check(&STEPS[1], &seen([60.0, 60.0, 80.0], None), base).is_err());
        // the menu's glass: red frost passes; the backdrop's blue-grey frost fails
        let menu = &STEPS[2];
        assert!(check(menu, &seen([0.0; 3], Some([220.0, 120.0, 125.0])), base).is_ok());
        assert!(check(menu, &seen([0.0; 3], Some([90.0, 100.0, 140.0])), base).is_err());
        assert!(check(menu, &seen([0.0; 3], Some([128.0, 128.0, 130.0])), base).is_err());
        assert!(check(menu, &seen([0.0; 3], None), base).is_err());
        // the page's red through a panel with no scene glass under it
        let bare = Seen {
            layer_image: false,
            ..seen([0.0; 3], Some([224.0, 24.0, 24.0]))
        };
        assert!(check(menu, &bare, base).is_err());
        let bar = &STEPS[3];
        assert!(check(bar, &seen([0.0; 3], Some([120.0, 210.0, 130.0])), base).is_ok());
        // the build count holds or fails
        let noted = check(&STEPS[5], &seen([0.0; 3], None), base).unwrap();
        assert!(check(&STEPS[6], &seen([0.0; 3], None), noted).is_ok());
        let more = Seen {
            builds: 5,
            ..seen([0.0; 3], None)
        };
        assert!(check(&STEPS[6], &more, noted).is_err());
        // the overlays: exactly the ones the step names
        let print = STEPS.iter().find(|s| s.name == "print-replaces-bookmark");
        let print = print.expect("print step");
        let only = |overlays| Seen {
            overlays,
            ..seen([0.0; 3], None)
        };
        let want = Shown {
            print: true,
            ..NONE
        };
        assert!(check(print, &only(want), base).is_ok());
        let both = Shown {
            bookmark: true,
            ..want
        };
        assert!(check(print, &only(both), base).is_err());
    }

    #[test]
    fn the_bundled_page_is_green_on_top_and_red_below() {
        let page = include_str!("../assets/overlay.html");
        for want in ["#dark", "--top", "--rest", "@keyframes"] {
            assert!(page.contains(want), "{want}");
        }
    }
}
