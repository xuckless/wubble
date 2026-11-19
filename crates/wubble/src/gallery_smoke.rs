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
    LoadError,
    CertError,
    AutofillMenu,
    AutofillSave,
    /// The passwords page after Unlock.
    Unlock,
    /// The notice a saved video frame or screenshot brings up.
    Toast,
}

const fn scene(name: &'static str) -> Scene {
    Scene {
        name,
        sidebar: SidebarMode::Extended,
        page: None,
        overlay: Overlay::None,
        wheel: 0,
        pointer: None,
        frames: 0,
    }
}

const fn on(name: &'static str, page: InternalPage) -> Scene {
    Scene {
        page: Some(page),
        ..scene(name)
    }
}

const fn over(name: &'static str, overlay: Overlay) -> Scene {
    Scene {
        overlay,
        ..scene(name)
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
    over("find-bar", Overlay::Find),
    over("permission-prompt", Overlay::Permission),
    over("autofill-menu", Overlay::AutofillMenu),
    over("autofill-save", Overlay::AutofillSave),
    over("dialog-sheet", Overlay::Dialog),
    over("toast", Overlay::Toast),
    over("error-page", Overlay::LoadError),
    over("cert-error-page", Overlay::CertError),
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
    Scene {
        pointer: Some(HISTORY_ROW),
        ..on("history-hover", InternalPage::History)
    },
];

/// The second tab row and the new-tab row of the extended sidebar on the parity profile
/// (`--parity`: four favourites, the Space name, five tabs), logical px.
const TAB_ROW: (u16, u16) = (130, 248);
const NEW_TAB_ROW: (u16, u16) = (130, 408);
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
    Scroll(usize),
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
        (Stage::Shown(i), Event::Settled) if SCENES[i].wheel > 0 => {
            (Stage::Scrolled(i), Some(Action::Scroll(i)))
        }
        (Stage::Shown(i) | Stage::Scrolled(i), Event::Settled) => {
            (Stage::Shot(i), Some(Action::Shoot(i)))
        }
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
                Action::Seed | Action::Show(_) | Action::Scroll(_) => Event::Settled,
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
        for stage in [Stage::Shown(3), Stage::Scrolled(3)] {
            for event in [Event::Ready, Event::Shot] {
                assert_eq!(next(stage, event), (stage, None));
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
    fn only_scenes_that_ask_are_scrolled() {
        let actions = walk();
        for (i, scene) in SCENES.iter().enumerate() {
            assert_eq!(
                actions.contains(&Action::Scroll(i)),
                scene.wheel > 0,
                "{}",
                scene.name
            );
        }
    }
}
