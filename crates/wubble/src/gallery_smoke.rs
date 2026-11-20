//! `--smoke gallery --shot-dir DIR` (hand run only): the shell walked through its surfaces on a
//! seeded profile, one PNG each (`01-sidebar-extended.png` ...), to compare with the design
//! repo. [`SCENES`] is the whole list as data; [`next`] is the pure walk (show a scene, wait
//! until it settled, scroll it if it asks, shoot it, show the next). The host (`host/gallery.rs`)
//! puts each scene into its own state and writes the PNG, so what is shot is what the shell
//! draws from host state, not properties poked into the window.
//!
//! Surfaces that need a live page (split panes, DevTools docked, the video strip, reader and
//! boost editor, Little and Popup windows) are not scenes: their own smokes take a `--shot`.
//!
//! For the design-fidelity harness (`tools/fidelity`): `--parity` seeds the design's Spaces and
//! tabs before the first scene ([`Action::Seed`]), a scene may hold the pointer somewhere
//! ([`Scene::pointer`], hover states), and a scene with [`Scene::frames`] is shot as a burst
//! of frames from the moment it is shown (its motion), each with its time since the show.

use wubble_core::pages::InternalPage;
use wubble_core::seam::SidebarMode;

/// One surface, as what the host puts on the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene {
    pub name: &'static str,
    pub sidebar: SidebarMode,
    /// The native page shown; `None` is the bundled web page.
    pub page: Option<InternalPage>,
    /// What opens over it.
    pub overlay: Overlay,
    /// Scrolls the page this many logical px (a wheel over it) once it settled, then settles
    /// again.
    pub wheel: u32,
    /// Where the pointer rests while the scene is shot, logical px in the window; `None`
    /// moves it out of the window.
    pub pointer: Option<(u16, u16)>,
    /// Shoot this many frames back to back from the show instead of one settled shot.
    pub frames: u32,
    /// Once it settled, a click here (logical px in the window), then it settles again: a row
    /// selected, a section picked in a page's list. Before `wheel`.
    pub click: Option<(u16, u16)>,
    /// A second click after the first settled (a row selected in the section the first
    /// picked).
    pub then: Option<(u16, u16)>,
    /// The web page under it.
    pub ground: Ground,
}

/// The web page a scene is shot over: the glass takes its light from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    /// The bundled animated page (a bright pastel gradient).
    Bundled,
    /// A plain white article (`assets/ground.html`): the brightest backdrop glass meets.
    White,
    /// The same article, near-black.
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    /// The command bar, with this typed into it.
    CommandBar(&'static str),
    /// The tab row's context menu.
    TabMenu,
    /// The address pill's blocker and page-theme menu.
    SiteMenu,
    Downloads,
    Find,
    Permission,
    Dialog,
    /// A prompt dialog, its field holding the page's text.
    Prompt,
    /// Mod+D's bookmark sheet (name, folders).
    Bookmark,
    /// The print sheet.
    Print,
    LoadError,
    CertError,
    AutofillMenu,
    AutofillSave,
    /// The passwords page after Unlock.
    Unlock,
    /// The notice a saved video frame or screenshot brings up.
    Toast,
    /// A notice waiting for an answer (SponsorBlock's question), at the page's bottom.
    Question,
    /// The link status bubble on the web page, for this address.
    Status(&'static str),
    /// A page's own right-click menu (back on, forward off: a disabled row).
    PageMenu,
    /// The sidebar's reload button's tooltip (it hangs over the sidebar's edge and the page).
    Tooltip,
    /// Settings' lists filled: routing rules, sites never put to sleep, zoom levels, remembered
    /// permissions (the list editors with rows).
    Lists,
    /// The first web tab shown again at a long address, with the blocker's count and the
    /// reader in the pill (wubble's layout cases: QA HIGH-01, CRIT-05, HIGH-03, MED-05).
    Long(Long),
}

/// What a [`Overlay::Long`] scene opens over the long address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Long {
    /// Nothing: the pill and its trailing chips.
    Pill,
    /// The tab row's menu (its rows that name the Space and the site).
    TabMenu,
    /// The blocker's menu under the pill's badge.
    SiteMenu,
    /// Enough tabs in the Space that the list scrolls.
    Crowd,
}

const fn scene(name: &'static str) -> Scene {
    Scene {
        name,
        sidebar: SidebarMode::Extended,
        page: None,
        overlay: Overlay::None,
        wheel: 0,
        pointer: None,
        click: None,
        then: None,
        frames: 0,
        ground: Ground::Bundled,
    }
}

const fn on(name: &'static str, page: InternalPage) -> Scene {
    Scene {
        page: Some(page),
        ..scene(name)
    }
}

const fn on_lists(name: &'static str) -> Scene {
    Scene {
        overlay: Overlay::Lists,
        ..on(name, InternalPage::Settings)
    }
}

const fn over(name: &'static str, overlay: Overlay) -> Scene {
    Scene {
        overlay,
        ..scene(name)
    }
}

/// `over` on a plain page: white, or near-black.
const fn over_on(name: &'static str, overlay: Overlay, ground: Ground) -> Scene {
    Scene {
        ground,
        ..over(name, overlay)
    }
}

/// Every scene, in the order they are shot. Web page scenes first: native pages replace the
/// visible tab, and nothing here goes back.
pub const SCENES: &[Scene] = &[
    scene("sidebar-extended"),
    Scene {
        sidebar: SidebarMode::Rail,
        ..scene("sidebar-rail")
    },
    Scene {
        sidebar: SidebarMode::Hidden,
        ..scene("sidebar-hidden")
    },
    // Hover states and motion, for the fidelity harness.
    Scene {
        pointer: Some(TAB_ROW),
        ..scene("tab-row-hover")
    },
    Scene {
        pointer: Some(NEW_TAB_ROW),
        ..scene("new-tab-hover")
    },
    Scene {
        frames: BURST,
        ..over("command-bar-open", Overlay::CommandBar(""))
    },
    over("command-bar", Overlay::CommandBar("")),
    over("command-bar-results", Overlay::CommandBar("test")),
    over("tab-row-menu", Overlay::TabMenu),
    over("site-menu", Overlay::SiteMenu),
    over("downloads-popover", Overlay::Downloads),
    Scene {
        pointer: Some(DOWNLOAD_ROW),
        ..over("downloads-popover-hover", Overlay::Downloads)
    },
    over("find-bar", Overlay::Find),
    over("permission-prompt", Overlay::Permission),
    Scene {
        pointer: Some(PERMISSION_BLOCK),
        ..over("permission-prompt-hover", Overlay::Permission)
    },
    over("autofill-menu", Overlay::AutofillMenu),
    Scene {
        pointer: Some(AUTOFILL_ROW),
        ..over("autofill-menu-hover", Overlay::AutofillMenu)
    },
    over("autofill-save", Overlay::AutofillSave),
    over("dialog-sheet", Overlay::Dialog),
    over("dialog-prompt", Overlay::Prompt),
    over("bookmark-sheet", Overlay::Bookmark),
    over("print-sheet", Overlay::Print),
    over("toast", Overlay::Toast),
    over("toast-question", Overlay::Question),
    over(
        "status-bubble",
        Overlay::Status(
            "https://github.com/xuckless/the-wallpaper-project/blob/main/src/wubble/pages/settings.tsx",
        ),
    ),
    over("error-page", Overlay::LoadError),
    over("cert-error-page", Overlay::CertError),
    // the glass over the brightest page and over a dark one: its ink must stay legible on both
    over_on("white-sidebar", Overlay::None, Ground::White),
    over_on("white-page-menu", Overlay::PageMenu, Ground::White),
    over_on("white-tab-menu", Overlay::TabMenu, Ground::White),
    over_on("white-command-bar", Overlay::CommandBar(""), Ground::White),
    over_on("white-find-bar", Overlay::Find, Ground::White),
    over_on("white-bookmark-sheet", Overlay::Bookmark, Ground::White),
    over_on("white-tooltip", Overlay::Tooltip, Ground::White),
    Scene {
        sidebar: SidebarMode::Hidden,
        pointer: Some(PEEK_EDGE),
        ..over_on("white-sidebar-peek", Overlay::None, Ground::White)
    },
    over_on("dark-page-menu", Overlay::PageMenu, Ground::Dark),
    over_on("dark-tab-menu", Overlay::TabMenu, Ground::Dark),
    over_on("dark-command-bar", Overlay::CommandBar(""), Ground::Dark),
    over_on("dark-find-bar", Overlay::Find, Ground::Dark),
    on("settings", InternalPage::Settings),
    Scene {
        wheel: 700,
        ..on("settings-2", InternalPage::Settings)
    },
    Scene {
        wheel: 700,
        ..on("settings-3", InternalPage::Settings)
    },
    Scene {
        wheel: 700,
        ..on("settings-4", InternalPage::Settings)
    },
    // the list editors: General's routing rules (a row selected, then a new rule's draft), the
    // Spaces section (its second Space selected), the Sites lists
    Scene {
        click: Some(NAV_GENERAL),
        then: Some(ROUTING_RULE),
        ..on_lists("settings-lists")
    },
    Scene {
        click: Some(NAV_GENERAL),
        then: Some(ROUTING_ADD),
        ..on_lists("settings-list-draft")
    },
    Scene {
        click: Some(NAV_SPACES),
        then: Some(SECOND_SPACE),
        ..on_lists("settings-spaces")
    },
    Scene {
        click: Some(NAV_SITES),
        ..on_lists("settings-sites")
    },
    on("history", InternalPage::History),
    on("bookmarks", InternalPage::Bookmarks),
    on("downloads", InternalPage::Downloads),
    on("passwords-locked", InternalPage::Passwords),
    Scene {
        overlay: Overlay::Unlock,
        ..on("passwords", InternalPage::Passwords)
    },
    on("archive", InternalPage::Archive),
    on("task-manager", InternalPage::Tasks),
    // the new-tab page: under `--frozen-backdrop` its liquid holds and its bubble steps on the
    // harness's fixed clock (backdrop.rs), so it is shot at rest the same way every run
    on("start", InternalPage::Start),
    Scene {
        pointer: Some(HISTORY_ROW),
        ..on("history-hover", InternalPage::History)
    },
    // the rail with its tab hovered: the close badge on the icon's corner (derived)
    Scene {
        sidebar: SidebarMode::Rail,
        pointer: Some(RAIL_TAB_ROW),
        ..on("sidebar-rail-hover", InternalPage::Start)
    },
    // wubble's layout cases: a long address and its chips, menus with long rows, a crowded
    // Space (extended and rail). The sidebar mid-collapse is crates/wubble-ui/tests/
    // layout_shot.rs's (the seam moves only on presented frames, which a headless run lacks).
    over("long-url-badge", Overlay::Long(Long::Pill)),
    over("long-menu-rows", Overlay::Long(Long::TabMenu)),
    over("site-popover-long-host", Overlay::Long(Long::SiteMenu)),
    over("tabs-crowded", Overlay::Long(Long::Crowd)),
    Scene {
        sidebar: SidebarMode::Rail,
        pointer: Some(RAIL_TAB_ROW),
        ..over("tabs-crowded-rail", Overlay::Long(Long::Crowd))
    },
    // Settings' Updates group, picked in the section list, then the page's end (the jump's
    // spring has not landed when the shot is taken): a development build, so its status says
    // updates come from elsewhere
    Scene {
        click: Some(NAV_UPDATES),
        wheel: 20_000,
        ..on("settings-updates", InternalPage::Settings)
    },
];

/// The second tab row and the new-tab row of the extended sidebar on the parity profile
/// (`--parity`: four favourites, the Space name, five tabs), logical px.
const TAB_ROW: (u16, u16) = (130, 248);
const NEW_TAB_ROW: (u16, u16) = (130, 408);
/// A point at the window's left edge that peeks the hidden sidebar, logical px.
const PEEK_EDGE: (u16, u16) = (3, 300);
/// The first tab of the rail on the gallery's own profile (one tab), logical px.
const RAIL_TAB_ROW: (u16, u16) = (36, 122);
// Overlays A at the design's 1360x880: a download row, the permission prompt's Block, the
// autofill menu's first login.
const DOWNLOAD_ROW: (u16, u16) = (330, 835);
const PERMISSION_BLOCK: (u16, u16) = (204, 298);
const AUTOFILL_ROW: (u16, u16) = (120, 162);
// Settings at the design's 1360x880 (extended sidebar): a routing rule's row in General, the
// section list's Spaces, Sites and Updates, the Spaces list's second row.
const NAV_GENERAL: (u16, u16) = (420, 197);
const NAV_SPACES: (u16, u16) = (420, 387);
const NAV_SITES: (u16, u16) = (420, 425);
const NAV_UPDATES: (u16, u16) = (420, 501);
const ROUTING_RULE: (u16, u16) = (900, 560);
const ROUTING_ADD: (u16, u16) = (632, 656);
const SECOND_SPACE: (u16, u16) = (900, 185);
/// The first visit's row on the history page (its actions show on hover), logical px.
const HISTORY_ROW: (u16, u16) = (800, 304);
/// Frames in a motion burst (about the length of an entrance at the snapshot rate).
const BURST: u32 = 24;

/// What the host reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The window can open views (the frame rate is known).
    Ready,
    /// The settle timer of the last show or scroll fired.
    Settled,
    /// The shot was written.
    Shot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// The profile is being seeded (`--parity`); waiting for its pages to load.
    Seeding,
    /// Scene `i` is on the shell; waiting for it to settle.
    Shown(usize),
    /// Scene `i` was scrolled; waiting for it to settle again.
    Scrolled(usize),
    /// Scene `i` took its second click; waiting for it to settle again.
    Clicked(usize),
    /// Scene `i` settled and the pointer was put on it again; waiting for the hover look.
    Pointed(usize),
    /// Scene `i` was written.
    Shot(usize),
    /// Frame `k` of scene `i`'s burst was asked for.
    Burst(usize, u32),
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Seed the profile (the design's Spaces and tabs with `--parity`; nothing without).
    Seed,
    Show(usize),
    /// Click (`Scene::click`) and scroll (`Scene::wheel`) scene `i`.
    Scroll(usize),
    /// Scene `i`'s second click (`Scene::then`).
    Click(usize),
    /// Puts the pointer where scene `i` says, on the settled layout: a hover is only worked out
    /// on a pointer event, and the one sent with the show met the layout the scene started from
    /// (the sidebar still springing out of the previous scene's mode).
    Point(usize),
    Shoot(usize),
    /// Frame `k` of scene `i`'s burst.
    ShootFrame(usize, u32),
    /// Every scene was shot.
    Finish,
}

/// One step: the stage and what just happened give the next stage and what to do.
pub fn next(stage: Stage, event: Event) -> (Stage, Option<Action>) {
    // After scene `i` was written: the next one, or the end.
    let after = |i: usize| {
        if i + 1 < SCENES.len() {
            (Stage::Shown(i + 1), Some(Action::Show(i + 1)))
        } else {
            (Stage::Done, Some(Action::Finish))
        }
    };
    match (stage, event) {
        (Stage::Start, Event::Ready) => (Stage::Seeding, Some(Action::Seed)),
        (Stage::Seeding, Event::Settled) => (Stage::Shown(0), Some(Action::Show(0))),
        (Stage::Shown(i), Event::Settled) if SCENES[i].frames > 0 => {
            (Stage::Burst(i, 0), Some(Action::ShootFrame(i, 0)))
        }
        (Stage::Shown(i), Event::Settled) if SCENES[i].wheel > 0 || SCENES[i].click.is_some() => {
            (Stage::Scrolled(i), Some(Action::Scroll(i)))
        }
        (Stage::Scrolled(i), Event::Settled) if SCENES[i].then.is_some() => {
            (Stage::Clicked(i), Some(Action::Click(i)))
        }
        (Stage::Shown(i), Event::Settled) if SCENES[i].pointer.is_some() => {
            (Stage::Pointed(i), Some(Action::Point(i)))
        }
        (
            Stage::Shown(i) | Stage::Scrolled(i) | Stage::Clicked(i) | Stage::Pointed(i),
            Event::Settled,
        ) => (Stage::Shot(i), Some(Action::Shoot(i))),
        (Stage::Burst(i, k), Event::Shot) if k + 1 < SCENES[i].frames => {
            (Stage::Burst(i, k + 1), Some(Action::ShootFrame(i, k + 1)))
        }
        (Stage::Shot(i) | Stage::Burst(i, _), Event::Shot) => after(i),
        (stage, _) => (stage, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Runs the walk with a host that settles every wait at once: the actions, in order.
    fn walk() -> Vec<Action> {
        let (mut stage, mut out, mut event) = (Stage::Start, Vec::new(), Event::Ready);
        while stage != Stage::Done {
            let (to, action) = next(stage, event);
            stage = to;
            let a = action.unwrap_or_else(|| panic!("stuck at {stage:?}"));
            out.push(a);
            event = match a {
                Action::Seed
                | Action::Show(_)
                | Action::Scroll(_)
                | Action::Click(_)
                | Action::Point(_) => Event::Settled,
                _ => Event::Shot,
            };
        }
        out
    }

    #[test]
    fn scene_names_are_unique_and_file_safe() {
        let names: HashSet<_> = SCENES.iter().map(|s| s.name).collect();
        assert_eq!(names.len(), SCENES.len());
        assert!(
            SCENES.len() < 100,
            "file names are numbered with two digits"
        );
        assert!(SCENES.iter().all(|s| {
            s.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }));
    }

    #[test]
    fn nothing_starts_before_the_window_is_ready() {
        for event in [Event::Settled, Event::Shot] {
            assert_eq!(next(Stage::Start, event), (Stage::Start, None));
        }
    }

    #[test]
    fn the_profile_is_seeded_before_the_first_scene() {
        let actions = walk();
        assert_eq!(actions.first(), Some(&Action::Seed));
        assert_eq!(actions.get(1), Some(&Action::Show(0)));
    }

    #[test]
    fn a_burst_shoots_each_of_its_frames_once_in_order() {
        let actions = walk();
        for (i, scene) in SCENES.iter().enumerate() {
            let frames: Vec<u32> = actions
                .iter()
                .filter_map(|a| match a {
                    Action::ShootFrame(j, k) if *j == i => Some(*k),
                    _ => None,
                })
                .collect();
            assert_eq!(
                frames,
                (0..scene.frames).collect::<Vec<_>>(),
                "{}",
                scene.name
            );
            assert_eq!(
                actions.contains(&Action::Shoot(i)),
                scene.frames == 0,
                "{}",
                scene.name
            );
        }
    }

    #[test]
    fn a_scene_is_shot_only_once_it_settled() {
        for stage in [
            Stage::Shown(3),
            Stage::Scrolled(3),
            Stage::Clicked(3),
            Stage::Pointed(3),
        ] {
            for event in [Event::Ready, Event::Shot] {
                assert_eq!(next(stage, event), (stage, None));
            }
        }
    }

    #[test]
    fn a_hover_scene_puts_the_pointer_again_once_settled() {
        let actions = walk();
        for (i, scene) in SCENES.iter().enumerate() {
            let point = actions.iter().position(|a| *a == Action::Point(i));
            assert_eq!(point.is_some(), scene.pointer.is_some(), "{}", scene.name);
            if let Some(p) = point {
                assert_eq!(
                    actions.get(p + 1),
                    Some(&Action::Shoot(i)),
                    "{}",
                    scene.name
                );
            }
        }
    }

    #[test]
    fn every_scene_is_shown_then_shot_once_in_order() {
        let actions = walk();
        let shown = actions.iter().filter(|a| matches!(a, Action::Show(_)));
        assert_eq!(shown.count(), SCENES.len());
        // The scene's single shot, or its burst's last frame.
        let shot = |i: usize| {
            actions.iter().rposition(|a| {
                *a == Action::Shoot(i) || matches!(a, Action::ShootFrame(j, _) if *j == i)
            })
        };
        for i in 0..SCENES.len() {
            let show = actions.iter().position(|a| *a == Action::Show(i));
            assert!(show.is_some() && show < shot(i), "scene {i}");
            if i > 0 {
                assert!(
                    shot(i - 1) < show,
                    "scene {i} shown before {} was shot",
                    i - 1
                );
            }
        }
        assert_eq!(actions.last(), Some(&Action::Finish));
    }

    #[test]
    fn only_scenes_that_ask_are_scrolled_or_clicked() {
        let actions = walk();
        for (i, scene) in SCENES.iter().enumerate() {
            assert_eq!(
                actions.contains(&Action::Scroll(i)),
                scene.wheel > 0 || scene.click.is_some(),
                "{}",
                scene.name
            );
            assert_eq!(
                actions.contains(&Action::Click(i)),
                scene.then.is_some(),
                "{}",
                scene.name
            );
            assert!(
                scene.then.is_none() || scene.click.is_some(),
                "{}: a second click comes after a first",
                scene.name
            );
        }
    }
}
