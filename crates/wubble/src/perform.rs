//! What each [`Action`] does, as data: [`plan`] maps an action and the
//! model to a [`Plan`], which `Host::perform` executes. Actions the app
//! cannot perform yet are listed once, in [`NOT_YET`], and logged when used.

use wubble_core::action::Action;
use wubble_core::model::{Command, Model};
use wubble_core::space::{MAX_PANES, SpaceId};
use wubble_core::tab::TabId;

use crate::surface::FindEvent;

/// Actions with no implementation yet, each with the reason.
pub const NOT_YET: &[(Action, &str)] = &[
    (Action::NewWindow, "one window only (TODO.md, multi-window)"),
    (Action::NewSpace, "needs a name and tint prompt"),
    (
        Action::ToggleCompactMode,
        "the shell's one sidebar toggle is ToggleSidebar",
    ),
    (Action::DevTools, "DevTools are not wired"),
    (Action::ViewSource, "view-source is not wired"),
];

/// Engine calls on the visible tab's view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewOp {
    Reload,
    HardReload,
    Stop,
    Back,
    Forward,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleMute,
    /// Save the page as a PDF in the downloads folder (`Host::print`).
    Print,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Model(Command),
    View(ViewOp),
    /// Open the command bar on the visible tab's address.
    FocusAddress,
    /// Take the newest entry off the store's recently-closed list and open it.
    ReopenClosed,
    ToggleSidebar,
    /// An event for the visible tab's find bar.
    Find(FindEvent),
    /// F11: the window, or leave page fullscreen (`screen.rs`).
    Fullscreen,
    /// Put the text on the clipboard.
    Copy(String),
    /// Open the command bar to choose a page to put beside the focused one.
    OpenBeside,
    Quit,
    /// Possible in general, but not now (no tab to act on, say).
    Nothing,
    /// In [`NOT_YET`].
    NotYet,
}

/// What `action` does to `model`. Exhaustive, without a catch-all: a new
/// `Action` does not compile until it is placed here.
pub fn plan(action: Action, model: &Model) -> Plan {
    let visible = model.visible_tab();
    let on_visible = |f: fn(_) -> Command| visible.map_or(Plan::Nothing, |t| Plan::Model(f(t)));
    let activate =
        |tab: Option<TabId>| tab.map_or(Plan::Nothing, |t| Plan::Model(Command::ActivateTab(t)));
    let switch = |space: Option<SpaceId>| {
        space.map_or(Plan::Nothing, |s| Plan::Model(Command::SwitchSpace(s)))
    };
    let view = |op| visible.map_or(Plan::Nothing, |_| Plan::View(op));
    let find = |event| visible.map_or(Plan::Nothing, |_| Plan::Find(event));
    match action {
        Action::NewTab => Plan::Model(Command::OpenCommandBar),
        Action::CloseTab => on_visible(Command::CloseTab),
        Action::ReopenClosedTab => Plan::ReopenClosed,
        Action::FocusAddress => Plan::FocusAddress,
        Action::Reload => view(ViewOp::Reload),
        Action::HardReload => view(ViewOp::HardReload),
        Action::Stop => view(ViewOp::Stop),
        Action::Back => view(ViewOp::Back),
        Action::Forward => view(ViewOp::Forward),
        Action::NextTab => activate(model.tab_step(1)),
        Action::PrevTab => activate(model.tab_step(-1)),
        Action::SelectTab(n) => {
            activate(usize::from(n).checked_sub(1).and_then(|i| model.nth_tab(i)))
        }
        Action::SelectLastTab => activate(model.last_tab()),
        Action::SelectSpace(n) => switch(
            usize::from(n)
                .checked_sub(1)
                .and_then(|i| model.nth_space(i)),
        ),
        Action::NextSpace => switch(model.space_step(1)),
        Action::PrevSpace => switch(model.space_step(-1)),
        Action::ToggleSidebar => Plan::ToggleSidebar,
        Action::TogglePinTab => match visible {
            Some(t) if model.is_pinned(t) => Plan::Model(Command::UnpinTab(t)),
            Some(t) => Plan::Model(Command::PinTab(t)),
            None => Plan::Nothing,
        },
        Action::Find => find(FindEvent::Open),
        Action::FindNext => find(FindEvent::Next),
        Action::FindPrev => find(FindEvent::Prev),
        Action::ZoomIn => view(ViewOp::ZoomIn),
        Action::ZoomOut => view(ViewOp::ZoomOut),
        Action::ZoomReset => view(ViewOp::ZoomReset),
        Action::ToggleMute => view(ViewOp::ToggleMute),
        Action::Print => view(ViewOp::Print),
        Action::Fullscreen => Plan::Fullscreen,
        Action::CopyUrl => visible
            .and_then(|t| model.tabs.get(&t))
            .filter(|t| !t.url.is_empty())
            .map_or(Plan::Nothing, |t| Plan::Copy(t.url.clone())),
        // the design's `split_next`: the next tab down, else a page chosen in the bar
        Action::SplitNext => match model.split_candidate() {
            Some(t) => Plan::Model(Command::SplitWith(t)),
            None if visible.is_some() && model.visible_tabs().len() < MAX_PANES => Plan::OpenBeside,
            None => Plan::Nothing,
        },
        Action::Unsplit => match visible {
            Some(t) if model.split_of(t).is_some() => Plan::Model(Command::Unsplit(t)),
            _ => Plan::Nothing,
        },
        Action::Quit => Plan::Quit,
        Action::NewWindow
        | Action::NewSpace
        | Action::ToggleCompactMode
        | Action::DevTools
        | Action::ViewSource => Plan::NotYet,
    }
}

/// Why `action` is not possible yet, if it is in [`NOT_YET`].
pub fn not_yet(action: Action) -> Option<&'static str> {
    NOT_YET
        .iter()
        .find(|(a, _)| *a == action)
        .map(|(_, why)| *why)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::model::apply;
    use wubble_core::space::{ContainerId, Rgb, SpaceId};
    use wubble_core::tab::TabId;

    fn with_tabs(n: usize) -> Model {
        (0..n).fold(Model::new(std::time::Instant::now()), |m, i| {
            apply(
                m,
                Command::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            )
            .0
        })
    }

    #[test]
    fn every_action_is_implemented_or_not_yet() {
        let models = [Model::new(std::time::Instant::now()), with_tabs(2)];
        for action in Action::all() {
            let listed = not_yet(action).is_some();
            for m in &models {
                assert_eq!(
                    plan(action, m) == Plan::NotYet,
                    listed,
                    "{action:?}: plan and NOT_YET disagree"
                );
            }
        }
        let mut listed: Vec<String> = NOT_YET.iter().map(|(a, _)| a.id()).collect();
        let n = listed.len();
        listed.sort();
        listed.dedup();
        assert_eq!(listed.len(), n, "NOT_YET lists an action twice");
    }

    #[test]
    fn tab_actions_follow_the_order() {
        // Tabs 1..=3; the last opened (3) is visible.
        let m = with_tabs(3);
        let go = |t: u64| Plan::Model(Command::ActivateTab(TabId(t)));
        assert_eq!(plan(Action::NextTab, &m), go(1), "wraps to the start");
        assert_eq!(plan(Action::PrevTab, &m), go(2));
        assert_eq!(plan(Action::SelectTab(1), &m), go(1));
        assert_eq!(plan(Action::SelectTab(4), &m), Plan::Nothing);
        assert_eq!(plan(Action::SelectLastTab, &m), go(3));
        assert_eq!(
            plan(Action::CloseTab, &m),
            Plan::Model(Command::CloseTab(TabId(3)))
        );
        assert_eq!(
            plan(Action::TogglePinTab, &m),
            Plan::Model(Command::PinTab(TabId(3)))
        );
        let (pinned, _) = apply(m.clone(), Command::PinTab(TabId(3)));
        assert_eq!(
            plan(Action::TogglePinTab, &pinned),
            Plan::Model(Command::UnpinTab(TabId(3)))
        );
        assert_eq!(plan(Action::Reload, &m), Plan::View(ViewOp::Reload));
        assert_eq!(plan(Action::HardReload, &m), Plan::View(ViewOp::HardReload));
        assert_eq!(plan(Action::ZoomIn, &m), Plan::View(ViewOp::ZoomIn));
        assert_eq!(plan(Action::ToggleMute, &m), Plan::View(ViewOp::ToggleMute));
        assert_eq!(plan(Action::Print, &m), Plan::View(ViewOp::Print));
        assert_eq!(plan(Action::Find, &m), Plan::Find(FindEvent::Open));
        assert_eq!(plan(Action::FindPrev, &m), Plan::Find(FindEvent::Prev));
        assert_eq!(plan(Action::Fullscreen, &m), Plan::Fullscreen);
        assert_eq!(
            plan(Action::CopyUrl, &m),
            Plan::Copy("https://2.test/".into())
        );
    }

    #[test]
    fn without_tabs_tab_actions_do_nothing() {
        let m = Model::new(std::time::Instant::now());
        for action in [
            Action::CloseTab,
            Action::NextTab,
            Action::SelectTab(1),
            Action::SelectLastTab,
            Action::TogglePinTab,
            Action::Reload,
            Action::Back,
            Action::Find,
            Action::ZoomOut,
            Action::ToggleMute,
            Action::Print,
            Action::CopyUrl,
        ] {
            assert_eq!(plan(action, &m), Plan::Nothing, "{action:?}");
        }
        assert_eq!(
            plan(Action::NewTab, &m),
            Plan::Model(Command::OpenCommandBar)
        );
        assert_eq!(plan(Action::ReopenClosedTab, &m), Plan::ReopenClosed);
        assert_eq!(plan(Action::FocusAddress, &m), Plan::FocusAddress);
        assert_eq!(plan(Action::Quit, &m), Plan::Quit);
        assert_eq!(plan(Action::ToggleSidebar, &m), Plan::ToggleSidebar);
    }

    #[test]
    fn space_actions() {
        let (m, _) = apply(
            Model::new(std::time::Instant::now()),
            Command::NewSpace {
                name: "W".into(),
                tint: Rgb(0, 0, 0),
                container: ContainerId::DEFAULT,
            },
        );
        let switch = |s: u32| Plan::Model(Command::SwitchSpace(SpaceId(s)));
        assert_eq!(plan(Action::SelectSpace(2), &m), switch(1));
        assert_eq!(plan(Action::SelectSpace(3), &m), Plan::Nothing);
        assert_eq!(plan(Action::NextSpace, &m), switch(1));
        assert_eq!(plan(Action::PrevSpace, &m), switch(1));
    }

    #[test]
    fn split_actions() {
        let m = with_tabs(1);
        assert_eq!(plan(Action::SplitNext, &m), Plan::OpenBeside);
        assert_eq!(plan(Action::Unsplit, &m), Plan::Nothing);
        let m = with_tabs(3);
        // the last tab is visible: the next one down wraps to the first
        assert_eq!(
            plan(Action::SplitNext, &m),
            Plan::Model(Command::SplitWith(TabId(1)))
        );
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        assert_eq!(
            plan(Action::Unsplit, &m),
            Plan::Model(Command::Unsplit(TabId(1)))
        );
        let (m, _) = apply(m, Command::SplitWith(TabId(2)));
        assert_eq!(plan(Action::SplitNext, &m), Plan::Nothing, "full");
        let empty = Model::new(std::time::Instant::now());
        assert_eq!(plan(Action::SplitNext, &empty), Plan::Nothing);
    }
}
