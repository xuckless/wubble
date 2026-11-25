//! What each [`Action`] does, as data: [`plan`] maps an action and the
//! model to a [`Plan`], which `Host::perform` executes.

use wubble_core::action::Action;
use wubble_core::model::{Command, Model};
use wubble_core::pages::InternalPage;
use wubble_core::space::SpaceId;
use wubble_core::tab::TabId;

use wubble_core::shot::ShotRequest;
use wubble_core::window::{WindowId, WindowTarget};

use wubble_engine::page::EditCommand;

use crate::host::Builtin;
use crate::surface::FindEvent;

/// Engine calls on the visible tab's view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewOp {
    Reload,
    HardReload,
    /// The HTTP cache cleared (CDP `Network.clearBrowserCache`), then a hard reload.
    EmptyCacheHardReload,
    Stop,
    Back,
    Forward,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleMute,
    /// An edit command on the view's focused frame.
    Edit(EditCommand),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Model(Command),
    View(ViewOp),
    /// Save the visible page as a PDF in the downloads folder (`Host::print`).
    Print,
    /// Open the command bar on the visible tab's address.
    FocusAddress,
    /// Open the command bar on every open and recently closed tab (tab search).
    SearchTabs,
    /// Load the clipboard's text in the visible tab as typed (a new tab without one).
    PasteAndGo,
    /// Add this Space (a `Command::NewSpace`), then show it.
    NewSpace(Command),
    /// F6 / Shift+F6 (`true`: back): keyboard focus to the next region (`wubble_core::region`).
    CycleRegion(bool),
    /// Take the newest entry off the store's recently-closed list and open it.
    ReopenClosed,
    /// The last tab closed this run back, if there is one (`wubble_core::closed`); run from
    /// the command bar, where no field is the key's.
    UndoClose,
    ToggleSidebar,
    /// An event for the visible tab's find bar.
    Find(FindEvent),
    /// F11: the window, or leave page fullscreen (`screen.rs`).
    Fullscreen,
    /// Put the text on the clipboard.
    Copy(String),
    /// Mod+D: the bookmark sheet for the visible page.
    BookmarkPage,
    /// Open the command bar to choose a page to put beside the focused one.
    OpenBeside,
    /// Put the tabs back as the host kept them before the last tidy.
    UndoTidy,
    /// Close these tabs (each goes on the recently-closed list).
    CloseTabs(Vec<TabId>),
    /// F12: DevTools docked or detached for the visible tab (`wubble_core::devtools`).
    DevTools,
    /// Ctrl+Shift+J: DevTools on its Console panel for the visible tab.
    DevToolsConsole,
    /// Ctrl+E: the command bar on the visible tab, every query a search.
    FocusSearch,
    /// Ctrl+S: download this tab's page, asking where (the menu's Save Page As).
    SavePage {
        tab: TabId,
        url: String,
    },
    /// Ctrl+O: the OS dialog for local files, each opened in a new tab.
    OpenFile,
    /// F7: caret browsing on or off (`wubble_core::caret`).
    CaretBrowsing,
    /// Raise and focus this window (Cmd+`).
    RaiseWindow(WindowId),
    /// Cmd+M: the focused window to the dock or taskbar.
    Minimize,
    /// The docked panel a step wider (1) or narrower (-1).
    DockNudge(i8),
    /// A built-in on the visible page (`host/builtins.rs`).
    Builtin(Builtin),
    /// End every agent session (`host/agent.rs`).
    StopAgents,
    Quit,
    /// Possible in general, but not now (no tab to act on, say).
    Nothing,
}

/// The action a click on a shell button runs, from the one it names and whether Shift, Ctrl or
/// Cmd was held: Chrome's reload button hard-reloads on a Shift- or Ctrl-click
/// (`ReloadButton::ButtonPressed`, the event's flags). Every other button runs its own.
pub fn clicked(action: Action, held: bool) -> Action {
    match (action, held) {
        (Action::Reload, true) => Action::HardReload,
        _ => action,
    }
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
    let builtin = |b| visible.map_or(Plan::Nothing, |_| Plan::Builtin(b));
    match action {
        // the start page (TODO.md Decided "Start page"); its search capsule opens the bar
        Action::NewTab => Plan::Model(Command::NewTab {
            url: InternalPage::Start.url().into(),
            container: None,
            opener: None,
        }),
        Action::CloseTab => on_visible(Command::CloseTab),
        Action::DuplicateTab => on_visible(Command::DuplicateTab),
        Action::ReopenClosedTab => Plan::ReopenClosed,
        // its key, unhandled by the page, goes to `Host::perform_unhandled` instead
        Action::UndoCloseTab => Plan::UndoClose,
        Action::FocusAddress => Plan::FocusAddress,
        Action::SearchTabs => Plan::SearchTabs,
        Action::FocusSearch => Plan::FocusSearch,
        Action::NextRegion => Plan::CycleRegion(false),
        Action::PrevRegion => Plan::CycleRegion(true),
        Action::Reload => view(ViewOp::Reload),
        Action::HardReload => view(ViewOp::HardReload),
        Action::EmptyCacheHardReload => view(ViewOp::EmptyCacheHardReload),
        Action::Stop => view(ViewOp::Stop),
        Action::Back => view(ViewOp::Back),
        Action::Forward => view(ViewOp::Forward),
        Action::NextTab => activate(model.tab_step(1)),
        Action::PrevTab => activate(model.tab_step(-1)),
        Action::MoveTabUp | Action::MoveTabDown => {
            let delta = if action == Action::MoveTabUp { -1 } else { 1 };
            visible
                .and_then(|tab| Some((tab, model.tab_shift(tab, delta)?)))
                .map_or(Plan::Nothing, |(tab, to)| {
                    Plan::Model(Command::MoveTab { tab, to })
                })
        }
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
        // Zen's compact mode (Mod+Alt+S) hides the sidebar and peeks it at the window edge: the
        // design's hidden mode, which the sidebar toggle reaches (`SidebarMode::toggled`)
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
        Action::Undo => view(ViewOp::Edit(EditCommand::Undo)),
        Action::Redo => view(ViewOp::Edit(EditCommand::Redo)),
        Action::Cut => view(ViewOp::Edit(EditCommand::Cut)),
        Action::Copy => view(ViewOp::Edit(EditCommand::Copy)),
        Action::Paste => view(ViewOp::Edit(EditCommand::Paste)),
        Action::PasteAndMatchStyle => view(ViewOp::Edit(EditCommand::PasteMatchStyle)),
        Action::SelectAll => view(ViewOp::Edit(EditCommand::SelectAll)),
        Action::Print => visible.map_or(Plan::Nothing, |_| Plan::Print),
        // a web or file page; the shell's own pages (wubble://) have nothing to download
        Action::SavePage => visible
            .and_then(|t| model.tabs.get(&t))
            .filter(|t| !t.url.is_empty() && InternalPage::from_url(&t.url).is_none())
            .map_or(Plan::Nothing, |t| Plan::SavePage {
                tab: t.id,
                url: t.url.clone(),
            }),
        Action::OpenFile => Plan::OpenFile,
        Action::ToggleCaretBrowsing => visible.map_or(Plan::Nothing, |_| Plan::CaretBrowsing),
        Action::Fullscreen => Plan::Fullscreen,
        Action::PasteAndGo => Plan::PasteAndGo,
        Action::CopyUrl => visible
            .and_then(|t| model.tabs.get(&t))
            .filter(|t| !t.url.is_empty())
            .map_or(Plan::Nothing, |t| Plan::Copy(t.url.clone())),
        // the design's `split_next`: the next tab down, else a page chosen in the bar
        Action::SplitNext => match model.split_candidate() {
            Some(t) => Plan::Model(Command::SplitWith(t)),
            None if visible.is_some() && model.pane_room() => Plan::OpenBeside,
            None => Plan::Nothing,
        },
        Action::Unsplit => match visible {
            Some(t) if model.split_of(t).is_some() => Plan::Model(Command::Unsplit(t)),
            _ => Plan::Nothing,
        },
        Action::OpenSettings => Plan::Model(Command::OpenInternal(InternalPage::Settings)),
        Action::OpenHistory => Plan::Model(Command::OpenInternal(InternalPage::History)),
        Action::OpenBookmarks => Plan::Model(Command::OpenInternal(InternalPage::Bookmarks)),
        Action::OpenDownloads => Plan::Model(Command::OpenInternal(InternalPage::Downloads)),
        Action::OpenPasswords => Plan::Model(Command::OpenInternal(InternalPage::Passwords)),
        Action::OpenArchive => Plan::Model(Command::OpenInternal(InternalPage::Archive)),
        Action::TidyTabs => Plan::Model(Command::TidyTabs {
            space: model.active_space(),
        }),
        Action::UndoTidyTabs => Plan::UndoTidy,
        Action::CloseDuplicateTabs => {
            let dupes = wubble_core::tidy::duplicates(model, model.active_space());
            if dupes.is_empty() {
                Plan::Nothing
            } else {
                Plan::CloseTabs(dupes)
            }
        }
        Action::OpenTasks => Plan::Model(Command::OpenInternal(InternalPage::Tasks)),
        Action::OpenAgentLog => Plan::Model(Command::OpenInternal(InternalPage::AgentLog)),
        Action::StopAgents => Plan::StopAgents,
        Action::BookmarkPage => visible
            .and_then(|t| model.tabs.get(&t))
            .filter(|t| wubble_core::url::is_recordable(&t.url))
            .map_or(Plan::Nothing, |_| Plan::BookmarkPage),
        Action::DevTools => visible.map_or(Plan::Nothing, |_| Plan::DevTools),
        Action::DevToolsConsole => visible.map_or(Plan::Nothing, |_| Plan::DevToolsConsole),
        // Chromium's Ctrl+U: the page's source in a new tab beside it
        Action::ViewSource => visible
            .and_then(|t| model.tabs.get(&t))
            .and_then(|t| Some((t.id, wubble_core::url::view_source(&t.url)?)))
            .map_or(Plan::Nothing, |(opener, url)| {
                Plan::Model(Command::NewTab {
                    url,
                    container: None,
                    opener: Some(opener),
                })
            }),
        Action::CyclePageTheme => builtin(Builtin::CyclePageTheme),
        Action::LinkHints => builtin(Builtin::LinkHints),
        Action::ToggleReader => builtin(Builtin::ToggleReader),
        Action::ScreenshotRegion => builtin(Builtin::Shot(ShotRequest::Region)),
        Action::ScreenshotVisible => builtin(Builtin::Shot(ShotRequest::Visible)),
        Action::ScreenshotFullPage => builtin(Builtin::Shot(ShotRequest::FullPage)),
        Action::NewBoost => builtin(Builtin::NewBoost),
        Action::DockWider => Plan::DockNudge(1),
        Action::DockNarrower => Plan::DockNudge(-1),
        Action::ClearBrowsingData => builtin(Builtin::ClearData),
        Action::Quit => Plan::Quit,
        Action::NewWindow => Plan::Model(Command::NewWindow { space: None }),
        Action::NewPrivateWindow => Plan::Model(Command::NewPrivateWindow { url: None }),
        Action::CloseWindow => Plan::Model(Command::CloseWindow(model.focused())),
        Action::MinimizeWindow => Plan::Minimize,
        Action::NextWindow => model
            .window_step(1)
            .map_or(Plan::Nothing, Plan::RaiseWindow),
        Action::PrevWindow => model
            .window_step(-1)
            .map_or(Plan::Nothing, Plan::RaiseWindow),
        Action::MoveTabToNewWindow => visible.map_or(Plan::Nothing, |tab| {
            Plan::Model(Command::MoveTabToWindow {
                tab,
                to: WindowTarget::New,
            })
        }),
        // "Space N" in the next unworn swatch (`Model::new_space`), then shown; Settings' Spaces
        // list adds one the same way without switching
        Action::NewSpace => Plan::NewSpace(model.new_space()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_core::model::apply;
    use wubble_core::space::{ContainerId, Rgb};
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
    fn the_agent_actions_are_planned() {
        let m = with_tabs(1);
        assert_eq!(plan(Action::StopAgents, &m), Plan::StopAgents);
        assert_eq!(
            plan(Action::OpenAgentLog, &m),
            Plan::Model(Command::OpenInternal(InternalPage::AgentLog))
        );
    }

    #[test]
    fn devtools_and_view_source_are_planned() {
        let m = with_tabs(1);
        assert_eq!(plan(Action::DevTools, &m), Plan::DevTools);
        assert_eq!(plan(Action::DockWider, &m), Plan::DockNudge(1));
        assert_eq!(plan(Action::DockNarrower, &m), Plan::DockNudge(-1));
        assert_eq!(
            plan(Action::DevTools, &Model::new(std::time::Instant::now())),
            Plan::Nothing
        );
        assert_eq!(
            plan(Action::ViewSource, &m),
            Plan::Model(Command::NewTab {
                url: "view-source:https://0.test/".into(),
                container: None,
                opener: Some(TabId(1)),
            })
        );
        // nothing to show the source of
        assert_eq!(
            plan(Action::ViewSource, &Model::new(std::time::Instant::now())),
            Plan::Nothing
        );
        let (blank, _) = apply(
            Model::new(std::time::Instant::now()),
            Command::NewTab {
                url: "about:blank".into(),
                container: None,
                opener: None,
            },
        );
        assert_eq!(plan(Action::ViewSource, &blank), Plan::Nothing);
        let (source, _) = apply(
            m,
            Command::NewTab {
                url: "view-source:https://0.test/".into(),
                container: None,
                opener: None,
            },
        );
        assert_eq!(plan(Action::ViewSource, &source), Plan::Nothing);
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
            plan(Action::DuplicateTab, &m),
            Plan::Model(Command::DuplicateTab(TabId(3)))
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
        assert_eq!(
            plan(Action::EmptyCacheHardReload, &m),
            Plan::View(ViewOp::EmptyCacheHardReload)
        );
        for (action, held, want) in [
            (Action::Reload, false, Action::Reload),
            (Action::Reload, true, Action::HardReload),
            (Action::Stop, true, Action::Stop),
            (Action::Back, true, Action::Back),
            (Action::ToggleSidebar, true, Action::ToggleSidebar),
        ] {
            assert_eq!(clicked(action, held), want, "{action:?} {held}");
        }
        assert_eq!(plan(Action::ZoomIn, &m), Plan::View(ViewOp::ZoomIn));
        assert_eq!(plan(Action::ToggleMute, &m), Plan::View(ViewOp::ToggleMute));
        assert_eq!(plan(Action::Print, &m), Plan::Print);
        assert_eq!(plan(Action::Find, &m), Plan::Find(FindEvent::Open));
        assert_eq!(plan(Action::FindPrev, &m), Plan::Find(FindEvent::Prev));
        assert_eq!(plan(Action::Fullscreen, &m), Plan::Fullscreen);
        assert_eq!(
            plan(Action::CopyUrl, &m),
            Plan::Copy("https://2.test/".into())
        );
        for (action, want) in [
            (Action::CyclePageTheme, Builtin::CyclePageTheme),
            (Action::LinkHints, Builtin::LinkHints),
            (Action::ToggleReader, Builtin::ToggleReader),
            (Action::ScreenshotRegion, Builtin::Shot(ShotRequest::Region)),
            (
                Action::ScreenshotVisible,
                Builtin::Shot(ShotRequest::Visible),
            ),
            (
                Action::ScreenshotFullPage,
                Builtin::Shot(ShotRequest::FullPage),
            ),
            (Action::NewBoost, Builtin::NewBoost),
            (Action::ClearBrowsingData, Builtin::ClearData),
        ] {
            assert_eq!(plan(action, &m), Plan::Builtin(want), "{action:?}");
        }
    }

    #[test]
    fn without_tabs_tab_actions_do_nothing() {
        let m = Model::new(std::time::Instant::now());
        for action in [
            Action::CloseTab,
            Action::DuplicateTab,
            Action::NextTab,
            Action::SelectTab(1),
            Action::SelectLastTab,
            Action::TogglePinTab,
            Action::Reload,
            Action::EmptyCacheHardReload,
            Action::Back,
            Action::Find,
            Action::ZoomOut,
            Action::ToggleMute,
            Action::Print,
            Action::CopyUrl,
            Action::LinkHints,
            Action::ToggleReader,
            Action::ScreenshotRegion,
            Action::CyclePageTheme,
            Action::NewBoost,
            Action::ClearBrowsingData,
        ] {
            assert_eq!(plan(action, &m), Plan::Nothing, "{action:?}");
        }
        assert_eq!(
            plan(Action::NewTab, &m),
            Plan::Model(Command::NewTab {
                url: "wubble://start".into(),
                container: None,
                opener: None,
            })
        );
        assert_eq!(plan(Action::ReopenClosedTab, &m), Plan::ReopenClosed);
        assert_eq!(plan(Action::UndoCloseTab, &m), Plan::UndoClose);
        assert_eq!(plan(Action::FocusAddress, &m), Plan::FocusAddress);
        assert_eq!(plan(Action::PasteAndGo, &m), Plan::PasteAndGo);
        assert_eq!(plan(Action::NextRegion, &m), Plan::CycleRegion(false));
        assert_eq!(plan(Action::PrevRegion, &m), Plan::CycleRegion(true));
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
        assert_eq!(
            plan(Action::NewSpace, &m),
            Plan::NewSpace(m.new_space()),
            "a Space with the next name and swatch, then shown"
        );
    }

    #[test]
    fn window_actions() {
        let m = with_tabs(1);
        assert_eq!(
            plan(Action::NewWindow, &m),
            Plan::Model(Command::NewWindow { space: None })
        );
        assert_eq!(
            plan(Action::CloseWindow, &m),
            Plan::Model(Command::CloseWindow(m.focused()))
        );
        assert_eq!(
            plan(Action::MoveTabToNewWindow, &m),
            Plan::Model(Command::MoveTabToWindow {
                tab: TabId(1),
                to: WindowTarget::New
            })
        );
        let empty = Model::new(std::time::Instant::now());
        assert_eq!(plan(Action::MoveTabToNewWindow, &empty), Plan::Nothing);
    }

    #[test]
    fn archive_and_tidy_actions() {
        let m = with_tabs(1);
        assert_eq!(
            plan(Action::OpenArchive, &m),
            Plan::Model(Command::OpenInternal(InternalPage::Archive))
        );
        assert_eq!(
            plan(Action::TidyTabs, &m),
            Plan::Model(Command::TidyTabs {
                space: m.active_space()
            })
        );
        assert_eq!(plan(Action::UndoTidyTabs, &m), Plan::UndoTidy);
        assert_eq!(plan(Action::CloseDuplicateTabs, &m), Plan::Nothing);
        let (m, _) = apply(
            m,
            Command::NewTab {
                url: "https://0.test/".into(),
                container: None,
                opener: None,
            },
        );
        assert_eq!(
            plan(Action::CloseDuplicateTabs, &m),
            Plan::CloseTabs(vec![TabId(1)]),
            "the visible one stays"
        );
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
