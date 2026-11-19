//! `--smoke gallery --shot-dir DIR` (hand run only): the shell walked through its surfaces on a
//! seeded profile, one PNG each (`01-sidebar-extended.png` ...), to compare with the design
//! repo. [`SCENES`] is the whole list as data; [`next`] is the pure walk (show a scene, wait
//! until it settled, scroll it if it asks, shoot it, show the next). The host (`host/gallery.rs`)
//! puts each scene into its own state and writes the PNG, so what is shot is what the shell
//! draws from host state, not properties poked into the window.
//!
//! Surfaces that need a live page (split panes, DevTools docked, the video strip, reader and
//! boost editor, Little and Popup windows) are not scenes: their own smokes take a `--shot`.

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
}

const fn scene(name: &'static str) -> Scene {
    Scene {
        name,
        sidebar: SidebarMode::Extended,
        page: None,
        overlay: Overlay::None,
        wheel: 0,
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
];

/// What the host sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    /// The window can open views (the frame rate is known).
    pub ready: bool,
    /// The scene's wait is over (the host's timer fired since the last show or scroll).
    pub settled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Start,
    /// Scene `i` is on the shell; waiting for it to settle.
    Shown(usize),
    /// Scene `i` was scrolled; waiting for it to settle again.
    Scrolled(usize),
    /// Scene `i` was written.
    Shot(usize),
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show(usize),
    Scroll(usize),
    Shoot(usize),
    /// Every scene was shot.
    Finish,
}

/// One step. The host calls it again after every action but `Show` and `Scroll` (which wait
/// for the timer), until it returns no action.
pub fn next(stage: Stage, seen: &Seen) -> (Stage, Option<Action>) {
    match stage {
        Stage::Start if seen.ready => (Stage::Shown(0), Some(Action::Show(0))),
        Stage::Shown(i) if seen.settled && SCENES[i].wheel > 0 => {
            (Stage::Scrolled(i), Some(Action::Scroll(i)))
        }
        Stage::Shown(i) | Stage::Scrolled(i) if seen.settled => {
            (Stage::Shot(i), Some(Action::Shoot(i)))
        }
        Stage::Shot(i) if i + 1 < SCENES.len() => (Stage::Shown(i + 1), Some(Action::Show(i + 1))),
        Stage::Shot(_) => (Stage::Done, Some(Action::Finish)),
        stage => (stage, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Runs the walk with a host that settles every wait at once: the actions, in order.
    fn walk() -> Vec<Action> {
        let mut seen = Seen {
            ready: true,
            settled: false,
        };
        let (mut stage, mut out) = (Stage::Start, Vec::new());
        while stage != Stage::Done {
            let (to, action) = next(stage, &seen);
            stage = to;
            match action {
                Some(a @ (Action::Show(_) | Action::Scroll(_))) => {
                    out.push(a);
                    seen.settled = true;
                }
                Some(a) => out.push(a),
                None => panic!("stuck at {stage:?}"),
            }
            if matches!(action, Some(Action::Shoot(_))) {
                seen.settled = false;
            }
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
        let idle = Seen {
            ready: false,
            settled: true,
        };
        assert_eq!(next(Stage::Start, &idle), (Stage::Start, None));
    }

    #[test]
    fn a_scene_is_shot_only_once_it_settled() {
        let waiting = Seen {
            ready: true,
            settled: false,
        };
        for stage in [Stage::Shown(3), Stage::Scrolled(3)] {
            assert_eq!(next(stage, &waiting), (stage, None));
        }
    }

    #[test]
    fn every_scene_is_shown_then_shot_once_in_order() {
        let actions = walk();
        let shown = actions.iter().filter(|a| matches!(a, Action::Show(_)));
        assert_eq!(shown.count(), SCENES.len());
        for i in 0..SCENES.len() {
            let show = actions.iter().position(|a| *a == Action::Show(i));
            let shoot = actions.iter().position(|a| *a == Action::Shoot(i));
            assert!(show < shoot && show.is_some(), "scene {i}");
            if i > 0 {
                let before = actions.iter().position(|a| *a == Action::Shoot(i - 1));
                assert!(before < show, "scene {i} shown before {} was shot", i - 1);
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
