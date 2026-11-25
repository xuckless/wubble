use super::*;
use crate::space::{Side, SplitGroup, Storage, even};
use crate::tab::Tab;
use crate::window::{Window, WindowId};

const D: ContainerId = ContainerId::DEFAULT;

/// One fixed start time for every model in these tests, so two new models compare equal.
fn t0() -> Instant {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *T0.get_or_init(Instant::now)
}

/// `apply` with the invariants checked, returning every effect.
fn run_all(m: Model, c: Command) -> (Model, Vec<Effect>) {
    let (m, fx) = apply(m, c);
    m.check().expect("invariants");
    (m, fx)
}

/// `run_all` without the lifecycle effects, which the lifecycle tests cover.
fn run(m: Model, c: Command) -> (Model, Vec<Effect>) {
    let (m, fx) = run_all(m, c);
    (m, core_effects(fx))
}

fn core_effects(fx: Vec<Effect>) -> Vec<Effect> {
    fx.into_iter()
        .filter(|e| !matches!(e, Effect::Lifecycle { .. }))
        .collect()
}

fn open(m: Model, opener: Option<TabId>) -> (Model, TabId) {
    let (m, fx) = run(
        m,
        Command::NewTab {
            url: "https://a.test".into(),
            container: None,
            opener,
        },
    );
    let Effect::CreateTab { tab, .. } = fx[0] else {
        panic!("no CreateTab: {fx:?}")
    };
    (m, tab)
}

/// A model with `n` plain tabs in the first Space; the last one is active.
fn with_tabs(n: usize) -> (Model, Vec<TabId>) {
    let mut m = Model::new(t0());
    let mut ids = vec![];
    for _ in 0..n {
        let (m2, t) = open(m, None);
        m = m2;
        ids.push(t);
    }
    (m, ids)
}

/// The Space `Model::new` creates.
fn s0() -> SpaceId {
    SpaceId(0)
}

fn active(m: &Model) -> Option<TabId> {
    m.spaces[m.space_idx(m.active_space()).unwrap()].active
}

fn add_space(m: Model, container: ContainerId) -> (Model, SpaceId) {
    let (m, _) = run(
        m,
        Command::NewSpace {
            name: "S".into(),
            tint: Rgb(1, 2, 3),
            container,
        },
    );
    let id = m.spaces.last().unwrap().id;
    (m, id)
}

#[test]
fn open_command_bar_creates_nothing() {
    let m = Model::new(t0());
    let (m2, fx) = run(m.clone(), Command::OpenCommandBar);
    assert_eq!(m2, m);
    assert_eq!(fx, vec![Effect::ShowCommandBar]);
}

#[test]
fn new_tab_into_empty_space_is_active() {
    let (m, fx) = run(
        Model::new(t0()),
        Command::NewTab {
            url: "u".into(),
            container: None,
            opener: None,
        },
    );
    let t = TabId(1);
    assert_eq!(active(&m), Some(t));
    assert_eq!(
        fx,
        vec![
            Effect::CreateTab {
                tab: t,
                url: "u".into(),
                container: D
            },
            Effect::ActivateTab(t)
        ]
    );
}

#[test]
fn new_tab_goes_after_opener_else_last() {
    let (m, ids) = with_tabs(3);
    let (m, t) = open(m, Some(ids[0]));
    assert_eq!(m.order(s0()), vec![ids[0], t, ids[1], ids[2]]);
    let (m, u) = open(m, None);
    assert_eq!(m.order(s0()).last(), Some(&u));
    // A pinned opener does not pull the tab into the pinned section.
    let (m, _) = run(m, Command::PinTab(ids[1]));
    let (m, v) = open(m, Some(ids[1]));
    assert!(matches!(m.location(v), Some((_, Place::Normal(_)))));
}

#[test]
fn open_in_space_puts_a_hidden_tab_in_that_space_and_its_container() {
    let m = Model::new(t0());
    let (m, _) = run(m, Command::NewContainer { name: "A".into() });
    let c = m.containers.last().unwrap().id;
    let (m, s) = add_space(m, c);
    let before = m.active_space();
    let (m, fx) = run_all(
        m,
        Command::OpenInSpace {
            space: s,
            url: "u".into(),
        },
    );
    let t = *m.order(s).last().unwrap();
    assert_eq!(m.tabs[&t].container, c);
    assert_eq!(m.active_space(), before, "the user's Space stays shown");
    assert_eq!(m.spaces[m.space_idx(s).unwrap()].active, Some(t));
    assert!(!m.visible_tabs().contains(&t));
    assert_eq!(
        core_effects(fx),
        vec![Effect::CreateTab {
            tab: t,
            url: "u".into(),
            container: c
        }]
    );
    // A second tab goes last and leaves the active one.
    let (m, _) = run(
        m,
        Command::OpenInSpace {
            space: s,
            url: "v".into(),
        },
    );
    assert_eq!(m.order(s).len(), 2);
    assert_eq!(m.spaces[m.space_idx(s).unwrap()].active, Some(t));
    // An unknown Space: nothing.
    let (m2, fx) = run(
        m.clone(),
        Command::OpenInSpace {
            space: SpaceId(99),
            url: "w".into(),
        },
    );
    assert_eq!(m2, m);
    assert!(fx.is_empty());
}

#[test]
fn new_tab_container_override_and_unknown() {
    let (m, _) = run(
        Model::new(t0()),
        Command::NewContainer {
            name: "Work".into(),
        },
    );
    let work = m.containers[1].id;
    let (m, fx) = run(
        m,
        Command::NewTab {
            url: "u".into(),
            container: Some(work),
            opener: None,
        },
    );
    assert!(matches!(fx[0], Effect::CreateTab { container, .. } if container == work));
    assert_eq!(m.tabs[&TabId(1)].container, work);
    let (m2, fx) = run(
        m.clone(),
        Command::NewTab {
            url: "u".into(),
            container: Some(ContainerId(99)),
            opener: None,
        },
    );
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn close_prefers_opener_child_then_opener_then_next_then_previous() {
    // opener child (set up directly: a shown NewTab becomes the active tab, so
    // a live opener relation to a background tab arises only transiently)
    let (mut m, ids) = with_tabs(3);
    m.tabs.get_mut(&ids[2]).unwrap().opener = Some(ids[0]);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    m.check().unwrap();
    let mut m = m;
    m.tabs.get_mut(&ids[2]).unwrap().opener = Some(ids[0]);
    let (m, fx) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(active(&m), Some(ids[2]));
    assert_eq!(
        fx,
        vec![Effect::CloseTab(ids[0]), Effect::ActivateTab(ids[2])]
    );

    // opener: closing the opened tab returns to the opener, not the next tab
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, t) = open(m, Some(ids[0]));
    let (m, fx) = run(m, Command::CloseTab(t));
    assert_eq!(active(&m), Some(ids[0]));
    assert_eq!(fx[1], Effect::ActivateTab(ids[0]));

    // next, then previous for the last tab
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::ActivateTab(ids[1]));
    let (m, _) = run(m, Command::CloseTab(ids[1]));
    assert_eq!(active(&m), Some(ids[2]));
    let (m, _) = run(m, Command::CloseTab(ids[2]));
    assert_eq!(active(&m), Some(ids[0]));
}

#[test]
fn close_last_tab_shows_command_bar() {
    let (m, ids) = with_tabs(1);
    let (m, fx) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(active(&m), None);
    assert!(m.tabs.is_empty());
    assert_eq!(fx, vec![Effect::CloseTab(ids[0]), Effect::ShowCommandBar]);
}

#[test]
fn close_inactive_keeps_active() {
    let (m, ids) = with_tabs(3);
    let (m, fx) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(active(&m), Some(ids[2]));
    assert_eq!(fx, vec![Effect::CloseTab(ids[0])]);
}

#[test]
fn activate_switches_space_and_keeps_openers_between_child_and_opener() {
    let (m, ids) = with_tabs(1);
    let (m, t) = open(m, Some(ids[0]));
    assert_eq!(m.tabs[&t].opener, Some(ids[0]));
    let (m, s2) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, fx) = run(m, Command::ActivateTab(ids[0]));
    assert_eq!(m.active_space(), s0());
    // from the child to its opener: Chromium keeps the relation
    assert_eq!(m.tabs[&t].opener, Some(ids[0]));
    assert_eq!(
        fx,
        vec![Effect::SpaceChanged(s0()), Effect::ActivateTab(ids[0])]
    );
    // Re-activating the shown tab is a no-op.
    let (m2, fx) = run(m.clone(), Command::ActivateTab(ids[0]));
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn move_reorders_and_clamps() {
    let (m, ids) = with_tabs(3);
    let to = Dest::Normal {
        space: s0(),
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[2], to });
    assert_eq!(m.order(s0()), vec![ids[2], ids[0], ids[1]]);
    let to = Dest::Normal {
        space: s0(),
        index: 999,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[2], to });
    assert_eq!(m.order(s0()), vec![ids[0], ids[1], ids[2]]);
}

#[test]
fn pin_unpin_positions_and_flag() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::PinTab(ids[1]));
    let (m, _) = run(m, Command::PinTab(ids[2]));
    assert_eq!(m.order(s0()), vec![ids[1], ids[2], ids[0]]);
    assert!(m.is_pinned(ids[1]));
    let (m, _) = run(m, Command::UnpinTab(ids[1]));
    assert_eq!(m.order(s0()), vec![ids[2], ids[1], ids[0]]);
    assert!(matches!(m.location(ids[1]), Some((_, Place::Normal(0)))));
    assert!(!m.is_pinned(ids[1]));
    // Moving into the pinned section pins, into the normal section unpins.
    let to = Dest::Pinned {
        space: s0(),
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[0], to });
    assert!(m.is_pinned(ids[0]));
    // Pinning twice is a no-op.
    let (m2, fx) = run(m.clone(), Command::PinTab(ids[0]));
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn folders_hold_tabs_and_unpin_pinned_ones() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(
        m,
        Command::NewFolder {
            space: s0(),
            name: "F".into(),
        },
    );
    let Item::Folder(f) = m.spaces[0].items.last().unwrap().clone() else {
        panic!()
    };
    let to = Dest::Folder {
        folder: f.id,
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[0], to });
    assert!(matches!(m.location(ids[0]), Some((_, Place::Folder(id, 0))) if id == f.id));
    let (m, _) = run(m, Command::PinTab(ids[1]));
    let to = Dest::Folder {
        folder: f.id,
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[1], to });
    assert!(!m.is_pinned(ids[1]));
    // Out of the folder.
    let to = Dest::Normal {
        space: s0(),
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[0], to });
    assert!(matches!(m.location(ids[0]), Some((_, Place::Normal(0)))));
    // Deleting the folder keeps its tab, at the folder's position.
    let (m, _) = run(m, Command::DeleteFolder(f.id));
    assert!(m.spaces[0].items.iter().all(|i| matches!(i, Item::Tab(_))));
    assert_eq!(m.order(s0()).len(), 3);
}

#[test]
fn move_across_spaces_keeps_effective_container_and_reselects_source() {
    let (m, _) = run(
        Model::new(t0()),
        Command::NewContainer {
            name: "Work".into(),
        },
    );
    let work = m.containers[1].id;
    let (m, s2) = add_space(m, work);
    let (m, ids) = {
        let mut m = m;
        let mut ids = vec![];
        for _ in 0..2 {
            let (m2, t) = open(m, None);
            m = m2;
            ids.push(t);
        }
        (m, ids)
    };
    assert_eq!(m.active_space(), s0());
    let to = Dest::Normal {
        space: s2,
        index: 0,
    };
    let (m, fx) = run(m, Command::MoveTab { tab: ids[1], to });
    // Default-container tab stays Default inside the Work Space.
    assert_eq!(m.tabs[&ids[1]].container, D);
    // It was the active tab of the shown Space: source falls back to the other tab.
    assert_eq!(active(&m), Some(ids[0]));
    assert_eq!(fx, vec![Effect::ActivateTab(ids[0])]);
    // The destination had no active tab; it remembers the moved one.
    assert_eq!(m.spaces[1].active, Some(ids[1]));
    // Moving back keeps it too.
    let to = Dest::Normal {
        space: s0(),
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[1], to });
    assert_eq!(m.tabs[&ids[1]].container, D);
}

#[test]
fn delete_space_closes_tabs_and_falls_back() {
    let (m, _) = run(Model::new(t0()), Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, s2) = add_space(m, work);
    let (m, ids) = with_tabs_in(m, 2);
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, fx) = run(m, Command::DeleteSpace(s2));
    assert_eq!(m.spaces.len(), 1);
    assert_eq!(m.active_space(), s0());
    assert!(fx.contains(&Effect::SpaceChanged(s0())));
    assert_eq!(m.containers.len(), 2, "shared container untouched");
    // Now delete the only tabs-owning Space's tabs with it: active falls back to next.
    let (m, s3) = add_space(m, D);
    let (m, fx) = run(m, Command::DeleteSpace(s0()));
    assert_eq!(m.active_space(), s3);
    assert!(m.tabs.is_empty());
    let closed = fx
        .iter()
        .filter(|e| matches!(e, Effect::CloseTab(_)))
        .count();
    assert_eq!(closed, ids.len());
    // Last Space refused.
    let (m2, fx) = run(m.clone(), Command::DeleteSpace(s3));
    assert_eq!((m2, fx), (m, vec![]));
}

fn with_tabs_in(mut m: Model, n: usize) -> (Model, Vec<TabId>) {
    let mut ids = vec![];
    for _ in 0..n {
        let (m2, t) = open(m, None);
        m = m2;
        ids.push(t);
    }
    (m, ids)
}

#[test]
fn switch_space_restores_remembered_active() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, s2) = add_space(m, D);
    let (m, fx) = run(m, Command::SwitchSpace(s2));
    assert_eq!(
        fx,
        vec![Effect::SpaceChanged(s2), Effect::ShowCommandBar],
        "empty Space hides the previous tab"
    );
    let (m, fx) = run(m, Command::SwitchSpace(s0()));
    assert_eq!(
        fx,
        vec![Effect::SpaceChanged(s0()), Effect::ActivateTab(ids[0])]
    );
    assert_eq!(active(&m), Some(ids[0]));
}

#[test]
fn delete_container_resets_spaces_and_overrides() {
    let (m, _) = run(Model::new(t0()), Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, s2) = add_space(m, work);
    let (m, a) = open(m, None);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, b) = open(m, None);
    assert_eq!(m.tabs[&b].container, work);
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: a,
            container: work,
        },
    );
    let old: Vec<TabId> = m.tabs.keys().copied().collect();
    let (m, fx) = run(m, Command::DeleteContainer(work));
    assert_eq!(m.containers.len(), 1);
    assert_eq!(m.spaces[1].container, D);
    assert!(m.tabs.values().all(|t| t.container == D));
    // Every live page in the dead context is replaced by a Default one.
    let closed: Vec<TabId> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::CloseTab(t) => Some(*t),
            _ => None,
        })
        .collect();
    let created = fx
        .iter()
        .filter(|e| matches!(e, Effect::CreateTab { container, .. } if *container == D))
        .count();
    assert_eq!((closed.len(), created), (old.len(), old.len()));
    assert!(
        old.iter()
            .all(|t| closed.contains(t) && !m.tabs.contains_key(t))
    );
    // Its storage goes too, after its pages closed.
    assert_eq!(
        fx.last(),
        Some(&Effect::ContainerDeleted {
            id: work,
            storage: Storage::Disk,
        })
    );
    // Default is not deletable.
    let (m2, fx) = run(m.clone(), Command::DeleteContainer(D));
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn pages_opened_from_a_tab_keep_its_container() {
    let (m, _) = run(Model::new(t0()), Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, a) = open(m, None);
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: a,
            container: work,
        },
    );
    let opener = m.visible_tab().expect("the reopened tab");
    assert_eq!(m.tabs[&opener].container, work);
    // A link or pop-up shown as a tab, one in the background, one in a Popup window.
    let (m, child) = open(m, Some(opener));
    let (m, fx) = run(
        m,
        Command::BackgroundTab {
            url: "https://bg.test".into(),
            opener,
        },
    );
    let Effect::CreateTab {
        tab: bg, container, ..
    } = fx[0]
    else {
        panic!("{fx:?}")
    };
    assert_eq!(container, work);
    let (m, fx) = run(
        m,
        Command::OpenPopupWindow {
            url: "https://pop.test".into(),
            opener,
            size: None,
            pos: None,
        },
    );
    let popup = fx
        .iter()
        .find_map(|e| match e {
            Effect::CreateTab { tab, .. } => Some(*tab),
            _ => None,
        })
        .expect("the popup's tab");
    assert!(
        [child, bg, popup]
            .iter()
            .all(|t| m.tabs[t].container == work)
    );
    // An explicit container still wins, and a tab without an opener takes the Space's.
    let (m, fx) = run(
        m,
        Command::NewTab {
            url: "https://d.test".into(),
            container: Some(D),
            opener: Some(opener),
        },
    );
    assert!(matches!(fx[0], Effect::CreateTab { container: D, .. }));
    let (m, plain) = open(m, None);
    assert_eq!(m.tabs[&plain].container, D);
}

#[test]
fn set_space_container_keeps_existing_tabs_context() {
    let (m, ids) = with_tabs(1);
    let (m, _) = run(m, Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, _) = run(
        m,
        Command::SetSpaceContainer {
            space: s0(),
            container: work,
        },
    );
    assert_eq!(m.tabs[&ids[0]].container, D);
    let (m, t) = open(m, None);
    assert_eq!(m.tabs[&t].container, work);
}

#[test]
fn reopen_replaces_in_place() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, _) = run(m, Command::ActivateTab(ids[1]));
    let (m, fx) = run(
        m,
        Command::ReopenInContainer {
            tab: ids[1],
            container: work,
        },
    );
    let order = m.order(s0());
    let new = order[1];
    assert_ne!(new, ids[1]);
    assert_eq!([order[0], order[2]], [ids[0], ids[2]]);
    assert_eq!(active(&m), Some(new));
    assert_eq!(m.tabs[&new].container, work);
    assert_eq!(
        fx,
        vec![
            Effect::CloseTab(ids[1]),
            Effect::CreateTab {
                tab: new,
                url: "https://a.test".into(),
                container: work
            },
            Effect::ActivateTab(new)
        ]
    );
    // Same container again: no-op.
    let (m2, fx) = run(
        m.clone(),
        Command::ReopenInContainer {
            tab: new,
            container: work,
        },
    );
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn rename_space() {
    let (m, _) = run(
        Model::new(t0()),
        Command::RenameSpace {
            space: SpaceId(0),
            name: "Home".into(),
        },
    );
    assert_eq!(m.spaces[0].name, "Home");
}

#[test]
fn folder_with_tab_takes_its_place_and_renames() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(
        m,
        Command::FolderWithTab {
            tab: ids[1],
            name: "Reading".into(),
        },
    );
    let Item::Folder(f) = &m.spaces[0].items[1] else {
        panic!("a folder where the tab was: {:?}", m.spaces[0].items);
    };
    assert_eq!(
        (f.name.as_str(), f.tabs.as_slice()),
        ("Reading", &ids[1..2])
    );
    let folder = f.id;
    assert_eq!(m.spaces[0].items.len(), 3);
    // a tab already in a folder goes into a new one at the end
    let (m, _) = run(
        m,
        Command::FolderWithTab {
            tab: ids[1],
            name: "Later".into(),
        },
    );
    let Some(Item::Folder(g)) = m.spaces[0].items.last() else {
        panic!("a new folder at the end");
    };
    assert_eq!((g.name.as_str(), g.tabs.as_slice()), ("Later", &ids[1..2]));
    let (m, _) = run(
        m,
        Command::RenameFolder {
            folder,
            name: "Read".into(),
        },
    );
    assert!(
        m.spaces[0]
            .items
            .iter()
            .any(|i| matches!(i, Item::Folder(f) if f.id == folder && f.name == "Read"))
    );
}

// ----- peeks

fn peek(m: Model, opener: TabId) -> (Model, TabId) {
    let w = m.focused();
    let (m, fx) = run(
        m,
        Command::OpenGlance {
            window: w,
            opener,
            url: "https://peek.test/".into(),
        },
    );
    let Some(Effect::CreateTab { tab, .. }) = fx.last().cloned() else {
        panic!("no CreateTab: {fx:?}")
    };
    (m, tab)
}

#[test]
fn a_peek_is_on_screen_over_its_opener_and_closes() {
    let (m, ids) = with_tabs(2);
    let opener = ids[1];
    let (m, g) = peek(m, opener);
    let w = m.focused();
    assert_eq!(m.glance_of(w).map(|g| g.tab), Some(g));
    assert_eq!(m.window_tabs(w), [opener, g], "the peek is on screen, last");
    assert_eq!(m.tabs[&g].lifecycle.liveness, Liveness::Active);
    assert_eq!(m.visible_tab(), Some(opener), "the opener keeps the keys");
    // a second peek replaces the first
    let (m, g2) = peek(m, opener);
    assert!(!m.tabs.contains_key(&g));
    assert_eq!(m.glance_of(w).map(|g| g.tab), Some(g2));
    let (m, fx) = run(m, Command::CloseGlance(w));
    assert_eq!(fx, [Effect::CloseTab(g2)]);
    assert_eq!(m.glance_of(w), None);
    assert_eq!(m.visible_tab(), Some(opener));
    // only over a page on screen: a hidden opener's link is a tab
    let (m, fx) = run(
        m,
        Command::OpenGlance {
            window: w,
            opener: ids[0],
            url: "https://x.test/".into(),
        },
    );
    assert!(matches!(fx.as_slice(), [Effect::CreateTab { .. }]));
    assert!(m.glances.is_empty());
}

#[test]
fn a_link_in_a_peek_opens_a_tab() {
    let (m, ids) = with_tabs(2);
    let (m, g) = peek(m, ids[1]);
    let w = m.focused();
    let before = m.tabs.len();
    let (m, fx) = run(
        m,
        Command::OpenGlance {
            window: w,
            opener: g,
            url: "https://inner.test/".into(),
        },
    );
    assert!(
        matches!(fx.first(), Some(Effect::CreateTab { .. })),
        "{fx:?}"
    );
    assert_eq!(m.tabs.len(), before + 1, "the link is not dropped");
    assert_eq!(m.glance_of(w).map(|x| x.tab), Some(g), "the peek stays");
}

#[test]
fn a_peek_from_a_little_window_opens_a_tab() {
    let (m, _) = with_tabs(1);
    let (m, fx) = run(
        m,
        Command::OpenLittle {
            url: "https://l.test".into(),
            space: None,
            container: None,
        },
    );
    let l = opened(&fx);
    let lt = m.window(l).unwrap().tab().unwrap();
    let before = m.tabs.len();
    let (m, fx) = run(
        m,
        Command::OpenGlance {
            window: l,
            opener: lt,
            url: "https://peek.test/".into(),
        },
    );
    assert!(
        matches!(fx.first(), Some(Effect::CreateTab { .. })),
        "{fx:?}"
    );
    assert_eq!(m.tabs.len(), before + 1, "the link is not dropped");
    assert!(m.glances.is_empty());
    m.check().expect("model holds");
}

#[test]
fn a_peek_closes_when_its_opener_leaves_the_screen() {
    let (m, ids) = with_tabs(2);
    let (m, g) = peek(m, ids[1]);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    assert!(m.glances.is_empty() && !m.tabs.contains_key(&g));
    let (m, g) = peek(m, ids[0]);
    let (m, _) = run(m, Command::CloseTab(ids[0]));
    assert!(m.glances.is_empty() && !m.tabs.contains_key(&g));
}

#[test]
fn a_peek_becomes_a_tab_or_a_pane() {
    let (m, ids) = with_tabs(2);
    let w = m.focused();
    let (m, g) = peek(m, ids[1]);
    let (m, _) = run(
        m,
        Command::PromoteGlance {
            window: w,
            to: GlanceTo::Tab,
        },
    );
    assert!(m.glances.is_empty());
    assert_eq!(m.visible_tab(), Some(g));
    let (m, g2) = peek(m, g);
    let (m, _) = run(
        m,
        Command::PromoteGlance {
            window: w,
            to: GlanceTo::Split,
        },
    );
    assert!(m.glances.is_empty());
    assert_eq!(m.window_tabs(w), [g, g2], "beside its opener");
}

#[test]
fn a_peek_is_no_part_of_a_session() {
    let (m, ids) = with_tabs(2);
    let (m, g) = peek(m, ids[1]);
    let saved = m.for_session(&[]);
    assert!(saved.glances.is_empty());
    assert!(!saved.tabs.contains_key(&g));
    assert!(!saved.order(s0()).contains(&g));
    saved.check().expect("a saved model holds");
    assert!(m.tabs.contains_key(&g), "the live model keeps it");
}

#[test]
fn a_transient_space_is_no_part_of_a_session() {
    let (m, ids) = with_tabs(1);
    let (m, _) = run(m, Command::NewContainer { name: "A".into() });
    let c = m.containers.last().unwrap().id;
    let (m, s) = add_space(m, c);
    let (m, _) = run(
        m,
        Command::OpenInSpace {
            space: s,
            url: "https://b.test".into(),
        },
    );
    let (m, _) = run(m, Command::SwitchSpace(s));
    let agent_tab = *m.order(s).first().unwrap();
    let saved = m.for_session(&[s]);
    saved.check().expect("a saved model holds");
    assert!(saved.space_idx(s).is_none());
    assert!(!saved.has_container(c));
    assert!(!saved.tabs.contains_key(&agent_tab));
    assert!(saved.tabs.contains_key(&ids[0]), "the user's tab stays");
    assert_eq!(saved.active_space(), s0());
    assert!(m.space_idx(s).is_some(), "the live model keeps it");
}

#[test]
fn a_tab_moved_into_a_space_is_not_held_by_its_container() {
    let (m, ids) = with_tabs(1);
    let (m, _) = run(m, Command::NewContainer { name: "A".into() });
    let c = m.containers.last().unwrap().id;
    let (m, s) = add_space(m, c);
    let (m, _) = run(
        m,
        Command::OpenInSpace {
            space: s,
            url: "https://b.test".into(),
        },
    );
    let own = *m.order(s).first().unwrap();
    assert!(m.holds(s, c, own));
    let (m, _) = run(
        m,
        Command::MoveTab {
            tab: ids[0],
            to: Dest::Normal { space: s, index: 0 },
        },
    );
    assert_eq!(m.location(ids[0]).map(|(sp, _)| sp), Some(s));
    assert!(!m.holds(s, c, ids[0]), "the user's tab keeps Default");
    assert!(!m.holds(s0(), D, own));
}

// ----- random sequences

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> Option<T> {
        (!v.is_empty()).then(|| v[self.below(v.len())])
    }
}

const BOGUS: u32 = 1_000_000;

/// A random command, and whether it names an id that does not exist (must be rejected).
fn random_command(r: &mut Xorshift, m: &Model) -> (Command, bool) {
    let bogus = r.below(5) == 0;
    let tabs: Vec<TabId> = m.tabs.keys().copied().collect();
    let spaces: Vec<SpaceId> = m.spaces.iter().map(|s| s.id).collect();
    let folders: Vec<FolderId> = m
        .spaces
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| {
            if let Item::Folder(f) = i {
                Some(f.id)
            } else {
                None
            }
        })
        .collect();
    let containers: Vec<ContainerId> = m.containers.iter().map(|c| c.id).collect();
    let tab = |r: &mut Xorshift| {
        if bogus {
            Some(TabId(u64::from(BOGUS)))
        } else {
            r.pick(&tabs)
        }
    };
    let space = |r: &mut Xorshift| {
        if bogus {
            Some(SpaceId(BOGUS))
        } else {
            r.pick(&spaces)
        }
    };
    let container = |r: &mut Xorshift| {
        if bogus {
            Some(ContainerId(BOGUS))
        } else {
            r.pick(&containers)
        }
    };
    let index = r.below(8);
    let cmd = match r.below(24) {
        0 => return (Command::OpenCommandBar, false),
        1 | 2 => {
            let c = if r.below(3) == 0 { container(r) } else { None };
            let opener = r.pick(&tabs);
            return (
                Command::NewTab {
                    url: "u".into(),
                    container: c,
                    opener,
                },
                bogus && c.is_some(),
            );
        }
        3 | 4 => tab(r).map(Command::CloseTab),
        5 | 6 => tab(r).map(Command::ActivateTab),
        7..=9 => {
            let to = match r.below(3) {
                0 => space(r).map(|space| Dest::Pinned { space, index }),
                1 => space(r).map(|space| Dest::Normal { space, index }),
                _ => if bogus {
                    Some(FolderId(BOGUS))
                } else {
                    r.pick(&folders)
                }
                .map(|folder| Dest::Folder { folder, index }),
            };
            tab(r).zip(to).map(|(tab, to)| Command::MoveTab { tab, to })
        }
        10 => tab(r).map(|t| {
            if r.below(2) == 0 {
                Command::PinTab(t)
            } else {
                Command::UnpinTab(t)
            }
        }),
        11 => space(r).map(|space| Command::NewFolder {
            space,
            name: "f".into(),
        }),
        12 => if bogus {
            Some(FolderId(BOGUS))
        } else {
            r.pick(&folders)
        }
        .map(Command::DeleteFolder),
        13 => match r.below(5) {
            0 => container(r).map(|container| Command::NewSpace {
                name: "s".into(),
                tint: Rgb(0, 0, 0),
                container,
            }),
            1 => space(r).map(|space| Command::RenameSpace {
                space,
                name: "r".into(),
            }),
            2 => space(r)
                .zip(container(r))
                .map(|(space, container)| Command::SetSpaceContainer { space, container }),
            3 => space(r).map(Command::DeleteSpace),
            _ => space(r).map(Command::SwitchSpace),
        },
        14 => return (Command::NewContainer { name: "c".into() }, false),
        15 => container(r).map(Command::DeleteContainer),
        16 => tab(r).map(|tab| Command::OpenPip { tab }),
        17 => {
            let c = if r.below(2) == 0 {
                Command::ClosePip
            } else {
                Command::ReturnPip
            };
            return (c, false);
        }
        18 => tab(r).map(Command::RendererGone),
        19 => tab(r).map(|tab| match r.below(3) {
            0 => Command::Navigate {
                tab,
                url: "n".into(),
            },
            1 => Command::UrlChanged {
                tab,
                url: "c".into(),
            },
            _ => Command::TitleChanged {
                tab,
                title: "t".into(),
            },
        }),
        21 => match r.below(4) {
            0 => {
                return (Command::OpenBeside { url: "b".into() }, false);
            }
            1 => tab(r).map(Command::Unsplit),
            // most often from the active Space, so splits form
            _ => {
                let order = m.order(m.active_space());
                if bogus {
                    tab(r)
                } else {
                    r.pick(&order).or_else(|| tab(r))
                }
                .map(Command::SplitWith)
            }
        },
        22 => {
            if r.below(2) == 0 {
                tab(r).map(|tab| Command::SwapPane {
                    tab,
                    right: r.below(2) == 0,
                })
            } else {
                let n = r.below(4) + 1;
                let stops = if r.below(2) == 0 {
                    crate::space::even(n)
                } else {
                    (1..n).map(|_| r.below(11) as f64 / 10.0).collect()
                };
                space(r).map(|space| Command::ResizeSplit { space, stops })
            }
        }
        23 => match r.below(4) {
            0 => tab(r).map(Command::DuplicateTab),
            1 => tab(r).map(Command::CloseOthers),
            2 => tab(r).map(|tab| Command::Busy {
                tab,
                reason: Reason::NeverSleep,
                on: r.below(2) == 0,
            }),
            _ => if bogus {
                Some(FolderId(BOGUS))
            } else {
                r.pick(&folders)
            }
            .map(|folder| Command::SetFolderCollapsed {
                folder,
                collapsed: r.below(2) == 0,
            }),
        },
        20 => {
            let pressure = [Pressure::None, Pressure::Moderate, Pressure::Critical][r.below(3)];
            let now = m.clock + Duration::from_secs(r.below(2000) as u64);
            return (
                Command::Tick {
                    now,
                    wall_ms: 0,
                    pressure,
                },
                false,
            );
        }
        _ => tab(r)
            .zip(container(r))
            .map(|(tab, container)| Command::ReopenInContainer { tab, container }),
    };
    let named = cmd.is_some();
    (cmd.unwrap_or(Command::OpenCommandBar), bogus && named)
}

#[test]
fn random_sequences_keep_invariants() {
    let mut most_panes = 0;
    for seed in 1..=200u64 {
        let mut r = Xorshift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut m = Model::new(t0());
        for step in 0..300 {
            let (cmd, must_reject) = random_command(&mut r, &m);
            let before = m.clone();
            let label = format!("seed {seed} step {step}: {cmd:?}");
            let (after, fx) = apply(m, cmd);
            after.check().unwrap_or_else(|e| panic!("{label}: {e}"));
            if must_reject {
                assert_eq!(after, before, "{label}: rejected command changed the model");
                assert!(fx.is_empty(), "{label}: rejected command emitted {fx:?}");
            }
            let was: Vec<TabId> = before.tabs.keys().copied().collect();
            let now: Vec<TabId> = after.tabs.keys().copied().collect();
            let mut closed: Vec<TabId> = fx
                .iter()
                .filter_map(|e| {
                    if let Effect::CloseTab(t) = e {
                        Some(*t)
                    } else {
                        None
                    }
                })
                .collect();
            let mut created: Vec<TabId> = fx
                .iter()
                .filter_map(|e| {
                    if let Effect::CreateTab { tab, .. } = e {
                        Some(*tab)
                    } else {
                        None
                    }
                })
                .collect();
            let mut removed: Vec<TabId> =
                was.iter().filter(|t| !now.contains(t)).copied().collect();
            let mut added: Vec<TabId> = now.iter().filter(|t| !was.contains(t)).copied().collect();
            closed.sort();
            created.sort();
            removed.sort();
            added.sort();
            assert_eq!(
                (closed, created),
                (removed, added),
                "{label}: effects vs tab set"
            );
            let panes = after.windows.iter().map(|w| after.window_tabs(w.id).len());
            most_panes = most_panes.max(panes.max().unwrap_or(0));
            m = after;
        }
    }
    assert_eq!(most_panes, MAX_PANES, "the sequences reach a full split");
}

#[test]
fn switching_to_an_empty_space_hides_the_previous_tab() {
    let (m, _) = with_tabs(1);
    let (m, s2) = add_space(m, D);
    let (m, fx) = run(m, Command::SwitchSpace(s2));
    assert_eq!(fx, vec![Effect::SpaceChanged(s2), Effect::ShowCommandBar]);
    // Deleting the active Space onto an empty neighbour does the same.
    let (m, _) = run(m, Command::SwitchSpace(s0()));
    let (m, s3) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s3));
    let (m, fx) = run(m, Command::DeleteSpace(s3));
    assert_eq!(m.active_space(), s2);
    assert_eq!(fx, vec![Effect::SpaceChanged(s2), Effect::ShowCommandBar]);
}

// ----- tab fields from the engine, navigation and lifecycle

use crate::lifecycle::Effect as Life;
use crate::sleep::Pressure;
use crate::tab::Liveness;
use std::time::Duration;

fn life(tab: TabId, effect: Life) -> Effect {
    Effect::Lifecycle { tab, effect }
}

/// Only the lifecycle effects, in order.
fn lifecycle_effects(fx: &[Effect]) -> Vec<Effect> {
    fx.iter()
        .filter(|e| matches!(e, Effect::Lifecycle { .. }))
        .cloned()
        .collect()
}

fn liveness(m: &Model, t: TabId) -> Liveness {
    m.tabs[&t].lifecycle.liveness
}

/// A tick `after` the model's clock.
fn tick(m: Model, after: Duration) -> (Model, Vec<Effect>) {
    let now = m.clock + after;
    run_all(
        m,
        Command::Tick {
            wall_ms: 0,
            now,
            pressure: Pressure::None,
        },
    )
}

#[test]
fn engine_reports_set_url_and_title_without_effects() {
    let (m, ids) = with_tabs(2);
    let (m, fx) = run_all(
        m,
        Command::UrlChanged {
            tab: ids[0],
            url: "https://b.test/x".into(),
        },
    );
    assert_eq!(fx, vec![]);
    let (m, fx) = run_all(
        m,
        Command::TitleChanged {
            tab: ids[0],
            title: "B".into(),
        },
    );
    assert_eq!(fx, vec![]);
    assert_eq!(m.tabs[&ids[0]].url, "https://b.test/x");
    assert_eq!(m.tabs[&ids[0]].title, "B");
    assert_eq!(m.tabs[&ids[1]].url, "https://a.test");
}

#[test]
fn navigate_sets_the_url_and_asks_the_engine() {
    let (m, ids) = with_tabs(1);
    let url = "https://c.test/".to_string();
    let (m, fx) = run_all(
        m,
        Command::Navigate {
            tab: ids[0],
            url: url.clone(),
        },
    );
    assert_eq!(fx, vec![Effect::Navigate { tab: ids[0], url }]);
    assert_eq!(m.tabs[&ids[0]].url, "https://c.test/");
}

#[test]
fn tab_commands_for_unknown_tabs_do_nothing() {
    let (m, _) = with_tabs(1);
    let ghost = TabId(99);
    for cmd in [
        Command::Navigate {
            tab: ghost,
            url: "x".into(),
        },
        Command::UrlChanged {
            tab: ghost,
            url: "x".into(),
        },
        Command::TitleChanged {
            tab: ghost,
            title: "x".into(),
        },
        Command::RendererGone(ghost),
    ] {
        let (m2, fx) = run_all(m.clone(), cmd.clone());
        assert_eq!((m2, fx), (m.clone(), vec![]), "{cmd:?}");
    }
}

#[test]
fn visible_tab_and_live_tabs() {
    let m = Model::new(t0());
    assert_eq!((m.visible_tab(), m.live_tabs()), (None, 0));
    let (m, ids) = with_tabs(3);
    assert_eq!((m.visible_tab(), m.live_tabs()), (Some(ids[2]), 3));
    let (m, s2) = add_space(m, D);
    let (m, _) = run_all(m, Command::SwitchSpace(s2));
    assert_eq!(m.visible_tab(), None);
}

#[test]
fn a_new_tab_hides_the_previous_one() {
    let (m, ids) = with_tabs(1);
    let (m, fx) = run_all(
        m,
        Command::NewTab {
            url: "u".into(),
            container: None,
            opener: None,
        },
    );
    let new = TabId(2);
    assert_eq!(
        lifecycle_effects(&fx),
        vec![
            life(ids[0], Life::CaptureSnapshot),
            life(ids[0], Life::Hide)
        ]
    );
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    assert_eq!(liveness(&m, new), Liveness::Active);
    // The first tab shows nothing: it was never hidden.
    let (_, fx) = run_all(
        Model::new(t0()),
        Command::NewTab {
            url: "u".into(),
            container: None,
            opener: None,
        },
    );
    assert_eq!(lifecycle_effects(&fx), vec![]);
}

#[test]
fn activating_hides_the_old_tab_then_shows_the_new() {
    let (m, ids) = with_tabs(2);
    let (m, fx) = run_all(m, Command::ActivateTab(ids[0]));
    assert_eq!(
        fx,
        vec![
            Effect::ActivateTab(ids[0]),
            life(ids[1], Life::CaptureSnapshot),
            life(ids[1], Life::Hide),
            life(ids[0], Life::Show),
        ]
    );
    assert_eq!(liveness(&m, ids[0]), Liveness::Active);
    assert_eq!(liveness(&m, ids[1]), Liveness::Background);
}

#[test]
fn closing_the_active_tab_releases_it_then_shows_the_successor() {
    let (m, ids) = with_tabs(2);
    let (m, fx) = run_all(m, Command::CloseTab(ids[1]));
    assert_eq!(
        fx,
        vec![
            Effect::CloseTab(ids[1]),
            Effect::ActivateTab(ids[0]),
            life(ids[1], Life::ReleaseTexture),
            life(ids[1], Life::ReleaseSnapshot),
            life(ids[0], Life::Show),
        ]
    );
    assert_eq!(m.live_tabs(), 1);
    // A hidden tab also drops its snapshot.
    let (m, ids) = with_tabs(2);
    let (_, fx) = run_all(m, Command::CloseTab(ids[0]));
    assert_eq!(
        fx,
        vec![
            Effect::CloseTab(ids[0]),
            life(ids[0], Life::ReleaseTexture),
            life(ids[0], Life::ReleaseSnapshot),
        ]
    );
}

#[test]
fn switching_space_hides_and_shows_across_spaces() {
    let (m, ids) = with_tabs(1);
    let (m, s2) = add_space(m, D);
    // Into an empty Space: only the hide.
    let (m, fx) = run_all(m, Command::SwitchSpace(s2));
    assert_eq!(
        lifecycle_effects(&fx),
        vec![
            life(ids[0], Life::CaptureSnapshot),
            life(ids[0], Life::Hide)
        ]
    );
    let (m, b) = open(m, None);
    // Back into a Space with a tab: hide the other Space's tab, show ours.
    let (m, fx) = run_all(m, Command::SwitchSpace(s0()));
    assert_eq!(
        lifecycle_effects(&fx),
        vec![
            life(b, Life::CaptureSnapshot),
            life(b, Life::Hide),
            life(ids[0], Life::Show),
        ]
    );
    assert_eq!(m.visible_tab(), Some(ids[0]));
}

#[test]
fn ticks_freeze_then_discard_hidden_tabs_but_keep_pinned_ones() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run_all(m, Command::PinTab(ids[0]));
    assert_eq!(m.tabs[&ids[0]].lifecycle.pin, crate::lifecycle::Pin::Pinned);
    let (m, fx) = tick(m, Duration::from_secs(61));
    assert_eq!(
        fx,
        vec![life(ids[0], Life::Freeze), life(ids[1], Life::Freeze)]
    );
    let (m, fx) = tick(m, Duration::from_secs(16 * 60));
    assert_eq!(
        fx,
        vec![
            life(ids[1], Life::Discard),
            life(ids[1], Life::ReleaseTexture)
        ]
    );
    assert_eq!(liveness(&m, ids[0]), Liveness::Frozen);
    assert_eq!(liveness(&m, ids[1]), Liveness::Discarded);
    assert_eq!(liveness(&m, ids[2]), Liveness::Active);
    assert_eq!(m.live_tabs(), 2);
}

#[test]
fn activating_a_discarded_tab_restores_it() {
    let (m, ids) = with_tabs(2);
    let (m, _) = tick(m, Duration::from_secs(16 * 60));
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
    let (m, fx) = run_all(m, Command::ActivateTab(ids[0]));
    assert_eq!(
        lifecycle_effects(&fx),
        vec![
            life(ids[1], Life::CaptureSnapshot),
            life(ids[1], Life::Hide),
            life(ids[0], Life::Restore),
            life(ids[0], Life::Show),
        ]
    );
    assert_eq!(m.live_tabs(), 2);
}

#[test]
fn a_hidden_tab_whose_renderer_died_is_discarded() {
    let (m, ids) = with_tabs(2);
    let (m, fx) = run_all(m, Command::RendererGone(ids[0]));
    assert_eq!(fx, vec![life(ids[0], Life::ReleaseTexture)]);
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
    // The visible one reloads in place.
    let (m, fx) = run_all(m, Command::RendererGone(ids[1]));
    assert_eq!(fx, vec![life(ids[1], Life::Restore)]);
    assert_eq!(liveness(&m, ids[1]), Liveness::Active);
}

#[test]
fn hides_after_a_tick_use_the_ticked_clock() {
    let (m, ids) = with_tabs(1);
    let later = m.clock + Duration::from_secs(3600);
    let (m, _) = run_all(
        m,
        Command::Tick {
            wall_ms: 0,
            now: later,
            pressure: Pressure::None,
        },
    );
    let (m, _) = open(m, None);
    assert_eq!(m.tabs[&ids[0]].lifecycle.hidden_since, Some(later));
}

#[test]
fn a_popup_from_a_hidden_tab_goes_next_to_it_and_stays_hidden() {
    let (m, ids) = with_tabs(2);
    // Same Space, opener hidden: placed after it, the visible tab stays.
    let (m, p) = open(m, Some(ids[0]));
    assert_eq!(m.order(s0()), vec![ids[0], p, ids[1]]);
    assert_eq!(m.visible_tab(), Some(ids[1]));
    assert_eq!(liveness(&m, p), Liveness::Background);
    // Opener in another Space: into that Space, the active Space unchanged.
    let (m, s2) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, w) = open(m, None);
    let (m, fx) = run_all(
        m,
        Command::NewTab {
            url: "https://pop.test".into(),
            container: None,
            opener: Some(ids[1]),
        },
    );
    let q = TabId(m.next_tab - 1);
    assert!(matches!(core_effects(fx.clone())[..], [Effect::CreateTab { tab, .. }] if tab == q));
    assert_eq!(
        lifecycle_effects(&fx),
        vec![life(q, Life::CaptureSnapshot), life(q, Life::Hide)]
    );
    assert_eq!(m.order(s0()), vec![ids[0], p, ids[1], q]);
    assert_eq!(m.order(s2), vec![w]);
    assert_eq!((m.active_space(), m.visible_tab()), (s2, Some(w)));
    assert_eq!(active(&m), Some(w));
    assert_eq!(m.spaces[0].active, Some(ids[1]));
}

/// Tab `ids[0]` of two starts loading while hidden.
fn hidden_loading_tab() -> (Model, Vec<TabId>) {
    let (m, ids) = with_tabs(2);
    let (m, fx) = run_all(m, loading(ids[0], true));
    assert_eq!(fx, vec![]);
    (m, ids)
}

fn loading(tab: TabId, loading: bool) -> Command {
    busy(tab, Reason::Loading, loading)
}

fn busy(tab: TabId, reason: Reason, on: bool) -> Command {
    Command::Busy { tab, reason, on }
}

#[test]
fn two_reasons_keep_a_tab_busy_until_both_end() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run_all(m, busy(ids[0], Reason::Audible, true));
    let (m, _) = run_all(m, busy(ids[0], Reason::Download, true));
    let (m, fx) = run_all(m, busy(ids[0], Reason::Audible, false));
    assert_eq!(fx, vec![]);
    assert_eq!(m.tabs[&ids[0]].lifecycle.activity, Activity::Busy);
    let (m, _) = tick(m, Duration::from_secs(16 * 60));
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    // The last reason ends: judged at once by the last tick, no new tick needed.
    let (m, fx) = run_all(m, busy(ids[0], Reason::Download, false));
    assert_eq!(
        fx,
        vec![
            life(ids[0], Life::Discard),
            life(ids[0], Life::ReleaseTexture)
        ]
    );
    assert!(m.tabs[&ids[0]].busy.is_empty());
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
}

#[test]
fn a_frozen_tab_gaining_any_reason_is_unfrozen() {
    for reason in crate::busy::REASONS.map(|(r, _, _)| r) {
        let (m, ids) = with_tabs(2);
        let (m, _) = tick(m, Duration::from_secs(2 * 60));
        assert_eq!(liveness(&m, ids[0]), Liveness::Frozen);
        let (m, fx) = run_all(m, busy(ids[0], reason, true));
        assert_eq!(fx, vec![life(ids[0], Life::Unfreeze)], "{reason:?}");
        assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    }
}

#[test]
fn never_sleep_protects_like_audio() {
    for reason in [Reason::NeverSleep, Reason::Audible] {
        let (m, ids) = with_tabs(2);
        let (m, _) = run_all(m, busy(ids[0], reason, true));
        let now = m.clock + Duration::from_secs(3600);
        let (m, fx) = run_all(
            m,
            Command::Tick {
                now,
                wall_ms: 0,
                pressure: Pressure::Critical,
            },
        );
        assert_eq!(fx, vec![], "{reason:?}");
        assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    }
}

#[test]
fn discard_now_takes_only_a_hidden_idle_tab() {
    let (m, ids) = with_tabs(3);
    // Visible: refused.
    let (m, fx) = run_all(m, Command::DiscardNow(ids[2]));
    assert_eq!(fx, vec![]);
    // Busy: refused.
    let (m, _) = run_all(m, busy(ids[0], Reason::Capture, true));
    let (m, fx) = run_all(m, Command::DiscardNow(ids[0]));
    assert_eq!(fx, vec![]);
    // Pinned and kept alive: refused, not frozen.
    let (m, _) = run(m, Command::PinTab(ids[1]));
    let (m, fx) = run_all(m, Command::DiscardNow(ids[1]));
    assert_eq!(fx, vec![]);
    let (m, _) = run(m, Command::UnpinTab(ids[1]));
    let (m, fx) = run_all(m, Command::DiscardNow(ids[1]));
    assert_eq!(
        fx,
        vec![
            life(ids[1], Life::Discard),
            life(ids[1], Life::ReleaseTexture)
        ]
    );
    assert_eq!(liveness(&m, ids[1]), Liveness::Discarded);
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
}

#[test]
fn a_critical_tick_discards_the_oldest_hidden_tab_first() {
    // Tabs are hidden newest id first: 3 (hidden at +2 s), then 2 (+1 s)... so the order of
    // the discards must differ from the id order.
    let (mut m, ids) = with_tabs(4);
    for (i, &t) in ids.iter().rev().enumerate() {
        let now = t0() + Duration::from_secs(i as u64);
        m = run_all(
            m,
            Command::Tick {
                now,
                wall_ms: 0,
                pressure: Pressure::None,
            },
        )
        .0;
        m = run(m, Command::ActivateTab(t)).0;
    }
    // ids[0] is visible now; ids[3] was hidden first, then ids[2], then ids[1].
    let now = m.clock;
    let (_, fx) = run_all(
        m,
        Command::Tick {
            now,
            wall_ms: 0,
            pressure: Pressure::Critical,
        },
    );
    let order: Vec<TabId> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Lifecycle {
                tab,
                effect: Life::Discard,
            } => Some(*tab),
            _ => None,
        })
        .collect();
    assert_eq!(order, vec![ids[3], ids[2], ids[1]]);
}

/// Regression for the tabs smoke flake: the one tick came while the hidden tab was
/// still loading, and nothing judged it again once the load ended.
#[test]
fn a_hidden_tab_that_stops_loading_is_judged_by_the_last_tick() {
    let (m, ids) = hidden_loading_tab();
    let (m, fx) = tick(m, Duration::from_secs(16 * 60));
    assert_eq!(fx, vec![]);
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    let (m, fx) = run_all(m, loading(ids[0], false));
    assert_eq!(
        fx,
        vec![
            life(ids[0], Life::Discard),
            life(ids[0], Life::ReleaseTexture)
        ]
    );
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
}

#[test]
fn a_hidden_tab_that_stops_loading_before_any_tick_stays_in_background() {
    let (m, ids) = hidden_loading_tab();
    let (m, fx) = run_all(m, loading(ids[0], false));
    assert_eq!(fx, vec![]);
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
}

#[test]
fn critical_pressure_of_the_last_tick_discards_a_tab_that_stops_loading() {
    let (m, ids) = hidden_loading_tab();
    let now = m.clock;
    let (m, _) = run_all(
        m,
        Command::Tick {
            wall_ms: 0,
            now,
            pressure: Pressure::Critical,
        },
    );
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    let (m, _) = run_all(m, loading(ids[0], false));
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
}

#[test]
fn unpinning_a_long_hidden_tab_discards_it() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, _) = tick(m, Duration::from_secs(16 * 60));
    assert_eq!(liveness(&m, ids[0]), Liveness::Frozen);
    let (m, _) = run(m, Command::UnpinTab(ids[0]));
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
}

/// Regression: the re-judge after unpinning ran before visibility settled, so a
/// frozen pinned tab dragged into view was discarded and then restored.
#[test]
fn moving_a_frozen_pinned_tab_into_view_unfreezes_it() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, s1) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s1));
    let (m, _) = tick(m, Duration::from_secs(16 * 60));
    assert_eq!(liveness(&m, ids[0]), Liveness::Frozen);
    let (m, fx) = run_all(
        m,
        Command::MoveTab {
            tab: ids[0],
            to: Dest::Normal {
                space: s1,
                index: 0,
            },
        },
    );
    assert_eq!(
        lifecycle_effects(&fx),
        vec![life(ids[0], Life::Unfreeze), life(ids[0], Life::Show)]
    );
    assert_eq!(m.visible_tab(), Some(ids[0]));
    assert_eq!(liveness(&m, ids[0]), Liveness::Active);
}

#[test]
fn new_and_from_parts_stamp_hides_with_the_given_time() {
    let start = t0() + Duration::from_secs(5);
    let (m, a) = open(Model::new(start), None);
    let (m, b) = open(m, None);
    assert_eq!(m.tabs[&a].lifecycle.hidden_since, Some(start));
    let restart = start + Duration::from_secs(1);
    let m = Model::from_parts(
        m.spaces.clone(),
        m.containers.clone(),
        m.tabs.values().cloned().collect(),
        m.windows.clone(),
        restart,
    )
    .unwrap();
    let (m, _) = open(m, None);
    assert_eq!(m.tabs[&b].lifecycle.hidden_since, Some(restart));
}

#[test]
fn reopening_a_pinned_tab_keeps_its_lifecycle_pinned() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, _) = run(m, Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: ids[0],
            container: work,
        },
    );
    let new = m.spaces[0].pinned[0];
    assert_eq!(m.tabs[&new].lifecycle.pin, crate::lifecycle::Pin::Pinned);
    let (m, _) = run(m, Command::UnpinTab(new));
    assert_eq!(m.tabs[&new].lifecycle.pin, crate::lifecycle::Pin::Unpinned);
}

/// The clock is not stored, so a rebuilt model is given the original's.
fn rebuild(m: &Model) -> Result<Model, String> {
    Model::from_parts(
        m.spaces.clone(),
        m.containers.clone(),
        m.tabs.values().cloned().collect(),
        m.windows.clone(),
        m.clock,
    )
}

#[test]
fn from_parts_of_new_is_new() {
    let m = Model::new(t0());
    assert_eq!(rebuild(&m), Ok(m));
}

#[test]
fn from_parts_counters_continue_after_max_id() {
    let (m, _) = with_tabs(3);
    let (m, _) = run(
        m,
        Command::NewFolder {
            space: s0(),
            name: "F".into(),
        },
    );
    let (m, _) = run(m, Command::NewContainer { name: "C".into() });
    let (m, _) = add_space(m, D);
    let rebuilt = rebuild(&m).unwrap();
    assert_eq!(rebuilt, m);
    // The same command on both yields the same ids.
    let cmds = [
        Command::NewFolder {
            space: s0(),
            name: "G".into(),
        },
        Command::NewContainer { name: "D".into() },
        Command::NewTab {
            url: "https://b.test".into(),
            container: None,
            opener: None,
        },
        Command::NewSpace {
            name: "T".into(),
            tint: Rgb(0, 0, 0),
            container: D,
        },
    ];
    let (mut a, mut b) = (m, rebuilt);
    for c in cmds {
        a = apply(a, c.clone()).0;
        b = apply(b, c).0;
        assert_eq!(a, b);
    }
}

#[test]
fn from_parts_rejects_invalid_parts() {
    let (m, _) = with_tabs(1);
    let mut tabs: Vec<Tab> = m.tabs.values().cloned().collect();
    tabs.clear();
    let r = Model::from_parts(
        m.spaces.clone(),
        m.containers.clone(),
        tabs,
        m.windows.clone(),
        t0(),
    );
    assert!(r.is_err(), "placed tab missing from tabs");
    let r = Model::from_parts(
        m.spaces.clone(),
        m.containers.clone(),
        vec![],
        vec![Window::main(WindowId(0), SpaceId(99))],
        t0(),
    );
    assert!(r.is_err(), "unknown active space");
}

#[test]
fn cycle_wraps_and_handles_missing_and_empty() {
    let items = [1, 2, 3];
    assert_eq!(cycle(&items, Some(1), 1), Some(2));
    assert_eq!(cycle(&items, Some(3), 1), Some(1));
    assert_eq!(cycle(&items, Some(1), -1), Some(3));
    assert_eq!(cycle(&items, Some(2), 4), Some(3));
    assert_eq!(cycle(&items, None, 1), Some(1));
    assert_eq!(cycle(&items, Some(9), -1), Some(3));
    assert_eq!(cycle::<i32>(&[], Some(1), 1), None);
}

#[test]
fn tab_order_helpers_follow_the_sidebar_order() {
    let (m, ids) = with_tabs(3);
    // Pinning moves the last tab to the front of the order.
    let (m, _) = run(m, Command::PinTab(ids[2]));
    let (m, _) = run(m, Command::ActivateTab(ids[2]));
    assert_eq!(m.order(s0()), vec![ids[2], ids[0], ids[1]]);
    assert_eq!(m.tab_step(1), Some(ids[0]));
    assert_eq!(m.tab_step(-1), Some(ids[1]), "wraps back to the end");
    assert_eq!(m.nth_tab(0), Some(ids[2]));
    assert_eq!(m.nth_tab(2), Some(ids[1]));
    assert_eq!(m.nth_tab(3), None);
    assert_eq!(m.last_tab(), Some(ids[1]));
}

#[test]
fn tab_order_helpers_on_an_empty_space() {
    let m = Model::new(t0());
    assert_eq!(m.tab_step(1), None);
    assert_eq!(m.nth_tab(0), None);
    assert_eq!(m.last_tab(), None);
}

#[test]
fn space_helpers_wrap() {
    let (m, s1) = add_space(Model::new(t0()), D);
    assert_eq!(m.space_step(1), Some(s1));
    assert_eq!(m.space_step(-1), Some(s1));
    let (m, _) = run(m, Command::SwitchSpace(s1));
    assert_eq!(m.space_step(1), Some(s0()));
    assert_eq!(m.nth_space(0), Some(s0()));
    assert_eq!(m.nth_space(1), Some(s1));
    assert_eq!(m.nth_space(2), None);
}

// ----- split view

fn group(m: &Model) -> Option<SplitGroup> {
    m.spaces[m.space_idx(m.active_space()).unwrap()]
        .split
        .clone()
}

/// `n` tabs with the first two split (the second focused).
fn split_of_two(n: usize) -> (Model, Vec<TabId>) {
    let (m, ids) = with_tabs(n);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::SplitWith(ids[1]));
    (m, ids)
}

#[test]
fn split_with_opens_beside_the_focused_page_and_focuses_the_new_one() {
    let (m, ids) = with_tabs(4);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, fx) = run(m, Command::SplitWith(ids[1]));
    assert_eq!(fx, vec![Effect::ActivateTab(ids[1])]);
    assert_eq!(
        group(&m),
        Some(SplitGroup {
            tabs: vec![ids[0], ids[1]],
            stops: vec![0.5]
        })
    );
    assert_eq!(m.visible_tab(), Some(ids[1]));
    assert_eq!(m.visible_tabs(), vec![ids[0], ids[1]]);
    assert_eq!(liveness(&m, ids[0]), Liveness::Active);
    assert_eq!(liveness(&m, ids[1]), Liveness::Active);
    // a third pane goes after the focused one and the shares even out
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::SplitWith(ids[2]));
    let g = group(&m).unwrap();
    assert_eq!(g.tabs, vec![ids[0], ids[2], ids[1]]);
    assert_eq!(g.stops, even(3));
    // never a fourth
    let (m2, fx) = run(m.clone(), Command::SplitWith(ids[3]));
    assert_eq!((m2, fx), (m.clone(), vec![]));
    assert_eq!(m.split_candidate(), None);
}

#[test]
fn split_with_refuses_itself_a_member_and_another_spaces_tab() {
    let (m, ids) = split_of_two(3);
    for t in [ids[1], ids[0]] {
        let (m2, fx) = run(m.clone(), Command::SplitWith(t));
        assert_eq!((m2, fx), (m.clone(), vec![]), "{t:?}");
    }
    let (m, s2) = add_space(m, D);
    let (m, other) = open(m, None);
    let (m, _) = run(
        m,
        Command::MoveTab {
            tab: other,
            to: Dest::Normal {
                space: s2,
                index: 0,
            },
        },
    );
    let (m2, fx) = run(m.clone(), Command::SplitWith(other));
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn unsplit_passes_focus_to_the_heir_and_a_split_of_one_is_none() {
    // the design's model_actions case
    let (m, ids) = with_tabs(4);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::SplitWith(ids[1]));
    assert_eq!(m.split_candidate(), Some(ids[2]));
    let (m, _) = run(m, Command::SplitWith(ids[2]));
    assert_eq!(group(&m).unwrap().tabs, ids[..3].to_vec());
    assert_eq!(active(&m), Some(ids[2]));
    let (m, fx) = run(m, Command::Unsplit(ids[2]));
    assert_eq!(fx, vec![Effect::ActivateTab(ids[1])]);
    assert_eq!(group(&m).unwrap().tabs, ids[..2].to_vec());
    assert_eq!(active(&m), Some(ids[1]));
    // the tab stays open, hidden
    assert!(m.tabs.contains_key(&ids[2]));
    assert_ne!(liveness(&m, ids[2]), Liveness::Active);
    let (m, fx) = run(m, Command::Unsplit(ids[0]));
    assert_eq!(fx, vec![]);
    assert_eq!(group(&m), None);
    assert_eq!(m.visible_tabs(), vec![ids[1]]);
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    // nothing to take out
    let (m2, fx) = run(m.clone(), Command::Unsplit(ids[1]));
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn unsplit_keeps_the_far_seam() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::SplitWith(ids[1]));
    let (m, _) = run(m, Command::SplitWith(ids[2]));
    let (m, _) = run(
        m,
        Command::ResizeSplit {
            space: s0(),
            stops: vec![0.2, 0.7],
        },
    );
    let (m0, _) = run(m.clone(), Command::Unsplit(ids[0]));
    assert_eq!(group(&m0).unwrap().stops, vec![0.7]);
    let (m1, _) = run(m.clone(), Command::Unsplit(ids[1]));
    assert_eq!(group(&m1).unwrap().stops, vec![0.2]);
    let (m2, _) = run(m, Command::Unsplit(ids[2]));
    assert_eq!(group(&m2).unwrap().stops, vec![0.2]);
}

#[test]
fn closing_panes_shrinks_the_split_then_unsplits() {
    let (m, ids) = with_tabs(4);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::SplitWith(ids[1]));
    let (m, _) = run(m, Command::SplitWith(ids[2]));
    // the focused pane closes: its heir in the split takes focus, not the next tab down
    let (m, _) = run(m, Command::CloseTab(ids[2]));
    assert_eq!(group(&m).unwrap().tabs, vec![ids[0], ids[1]]);
    assert_eq!(active(&m), Some(ids[1]));
    // closing the last-but-one pane unsplits
    let (m, _) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(group(&m), None);
    assert_eq!(m.visible_tabs(), vec![ids[1]]);
}

#[test]
fn moving_a_pane_to_another_space_takes_it_out() {
    let (m, ids) = split_of_two(3);
    let (m, s2) = add_space(m, D);
    let (m, _) = run(
        m,
        Command::MoveTab {
            tab: ids[1],
            to: Dest::Normal {
                space: s2,
                index: 0,
            },
        },
    );
    assert_eq!(group(&m), None);
    assert_eq!(active(&m), Some(ids[0]));
    // moving within the Space keeps it
    let (m, ids) = split_of_two(3);
    let (m, _) = run(
        m,
        Command::MoveTab {
            tab: ids[1],
            to: Dest::Pinned {
                space: s0(),
                index: 0,
            },
        },
    );
    assert_eq!(group(&m).unwrap().tabs, vec![ids[0], ids[1]]);
}

#[test]
fn deleting_a_space_and_reopening_a_pane_keep_splits_valid() {
    let (m, ids) = split_of_two(3);
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: ids[0],
            container: ContainerId(0),
        },
    );
    let (m, _) = run(m, Command::NewContainer { name: "c".into() });
    let (m, fx) = run(
        m,
        Command::ReopenInContainer {
            tab: ids[0],
            container: ContainerId(1),
        },
    );
    let Some(Effect::CreateTab { tab: new, .. }) = fx.get(1).cloned() else {
        panic!("{fx:?}")
    };
    assert_eq!(group(&m).unwrap().tabs, vec![new, ids[1]]);
    assert_eq!(liveness(&m, ids[1]), Liveness::Active);
    let (m, s2) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, _) = run(m, Command::DeleteSpace(s0()));
    assert!(m.spaces.iter().all(|s| s.split.is_none()));
}

#[test]
fn resize_split_takes_only_stops_that_fit() {
    let (m, _) = split_of_two(3);
    for bad in [vec![], vec![0.2, 0.6], vec![0.0], vec![1.0], vec![f64::NAN]] {
        let (m2, fx) = run(
            m.clone(),
            Command::ResizeSplit {
                space: s0(),
                stops: bad.clone(),
            },
        );
        assert_eq!((m2, fx), (m.clone(), vec![]), "{bad:?}");
    }
    let (m, _) = run(
        m,
        Command::ResizeSplit {
            space: s0(),
            stops: vec![0.3],
        },
    );
    assert_eq!(group(&m).unwrap().stops, vec![0.3]);
    assert_eq!(group(&m).unwrap().ratios(), vec![0.3, 0.7]);
}

#[test]
fn swap_pane_swaps_neighbours_and_keeps_seams_and_focus() {
    let (m, ids) = split_of_two(3);
    let (m, _) = run(
        m,
        Command::ResizeSplit {
            space: s0(),
            stops: vec![0.3],
        },
    );
    let (m, _) = run(
        m,
        Command::SwapPane {
            tab: ids[0],
            right: true,
        },
    );
    assert_eq!(group(&m).unwrap().tabs, vec![ids[1], ids[0]]);
    assert_eq!(group(&m).unwrap().stops, vec![0.3]);
    assert_eq!(active(&m), Some(ids[1]));
    // nothing to its left
    let (m2, fx) = run(
        m.clone(),
        Command::SwapPane {
            tab: ids[1],
            right: false,
        },
    );
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn activating_outside_the_split_hides_every_pane_and_back_shows_them() {
    let (m, ids) = split_of_two(3);
    let (m, fx) = run_all(m, Command::ActivateTab(ids[2]));
    let hidden: Vec<TabId> = lifecycle_effects(&fx)
        .iter()
        .filter_map(|e| match e {
            Effect::Lifecycle {
                tab,
                effect: Life::Hide,
            } => Some(*tab),
            _ => None,
        })
        .collect();
    assert_eq!(hidden, vec![ids[0], ids[1]]);
    assert_eq!(m.visible_tabs(), vec![ids[2]]);
    // the split is remembered
    assert_eq!(group(&m).unwrap().tabs, vec![ids[0], ids[1]]);
    // a long time later, under pressure, the hidden panes are discarded like any tab
    let now = m.clock() + Duration::from_secs(3600 * 24);
    let (m, _) = run(
        m,
        Command::Tick {
            wall_ms: 0,
            now,
            pressure: Pressure::Critical,
        },
    );
    assert_eq!(liveness(&m, ids[0]), Liveness::Discarded);
    assert_eq!(liveness(&m, ids[2]), Liveness::Active);
    // focusing one pane brings the whole split back: both restore
    let (m, fx) = run_all(m, Command::ActivateTab(ids[0]));
    for t in [ids[0], ids[1]] {
        assert!(fx.contains(&life(t, Life::Restore)), "{t:?}: {fx:?}");
        assert!(fx.contains(&life(t, Life::Show)), "{t:?}: {fx:?}");
        assert_eq!(liveness(&m, t), Liveness::Active);
    }
    // visible panes never sleep
    let now = m.clock() + Duration::from_secs(3600 * 24);
    let (m, _) = run(
        m,
        Command::Tick {
            wall_ms: 0,
            now,
            pressure: Pressure::Critical,
        },
    );
    assert_eq!(liveness(&m, ids[1]), Liveness::Active);
}

#[test]
fn split_candidate_wraps_and_skips_panes() {
    let (m, ids) = with_tabs(3);
    assert_eq!(m.split_candidate(), Some(ids[0]), "wraps from the last");
    let (m, _) = run(m, Command::ActivateTab(ids[1]));
    let (m, _) = run(m, Command::SplitWith(ids[2]));
    assert_eq!(m.split_candidate(), Some(ids[0]));
    let (m, _) = with_tabs(1);
    assert_eq!(m.split_candidate(), None);
    assert_eq!(Model::new(t0()).split_candidate(), None);
}

#[test]
fn open_beside_adds_a_new_page_to_the_split() {
    let (m, ids) = with_tabs(1);
    let (m, fx) = run(
        m,
        Command::OpenBeside {
            url: "https://b.test".into(),
        },
    );
    let Effect::CreateTab { tab: new, .. } = fx[0] else {
        panic!("{fx:?}")
    };
    assert_eq!(fx[1..], [Effect::ActivateTab(new)]);
    assert_eq!(group(&m).unwrap().tabs, vec![ids[0], new]);
    // into an empty Space it is a plain new tab
    let (m, s2) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, _) = run(m, Command::OpenBeside { url: "u".into() });
    assert_eq!(group(&m), None);
    assert_eq!(m.visible_tabs().len(), 1);
}

#[test]
fn from_parts_rejects_bad_splits() {
    let (m, ids) = split_of_two(4);
    let mut spaces = m.spaces.clone();
    let tabs: Vec<Tab> = m.tabs.values().cloned().collect();
    let parts = |spaces: Vec<Space>| {
        Model::from_parts(
            spaces,
            m.containers.clone(),
            tabs.clone(),
            m.windows.clone(),
            t0(),
        )
    };
    assert!(parts(spaces.clone()).is_ok());
    let bad = [
        SplitGroup {
            tabs: vec![ids[0]],
            stops: vec![],
        },
        SplitGroup {
            tabs: vec![ids[0], ids[1], ids[2], ids[3]],
            stops: even(4),
        },
        SplitGroup {
            tabs: vec![ids[0], ids[1]],
            stops: vec![0.5, 0.6],
        },
        SplitGroup {
            tabs: vec![ids[0], ids[0]],
            stops: vec![0.5],
        },
        SplitGroup {
            tabs: vec![ids[0], TabId(99)],
            stops: vec![0.5],
        },
    ];
    for g in bad {
        spaces[0].split = Some(g.clone());
        assert!(parts(spaces.clone()).is_err(), "{g:?}");
    }
}

#[test]
fn set_sleep_policy_changes_when_a_hidden_tab_sleeps() {
    let (m, ids) = with_tabs(2);
    // ids[0] is hidden; the default freezes it after 60 s.
    let (m, _) = tick(m, Duration::from_secs(20));
    assert_eq!(liveness(&m, ids[0]), Liveness::Background);
    let (m, _) = run_all(
        m,
        Command::SetSleepPolicy(SleepPolicy::from_settings(10, 60, true)),
    );
    let (m, _) = tick(m, Duration::from_secs(1));
    assert_eq!(liveness(&m, ids[0]), Liveness::Frozen);
    assert_eq!(m.policy().freeze_after, Duration::from_secs(10));
}

#[test]
fn open_internal_focuses_the_tab_already_on_it() {
    for page in InternalPage::ALL {
        let (m, ids) = with_tabs(1);
        let (m, fx) = run(m, Command::OpenInternal(page));
        let Effect::CreateTab {
            tab: shown, url, ..
        } = fx[0].clone()
        else {
            panic!("no CreateTab: {fx:?}")
        };
        assert_eq!(url, page.url());
        assert!(m.is_internal(shown) && !m.is_internal(ids[0]));
        let (m, _) = run(m, Command::ActivateTab(ids[0]));
        let (m, fx) = run(m, Command::OpenInternal(page));
        assert_eq!(fx, [Effect::ActivateTab(shown)], "{page:?}");
        assert_eq!(m.tabs.len(), 2);
        // Every internal page is a split pane beside a web page.
        let (m2, _) = run(m.clone(), Command::ActivateTab(ids[0]));
        let (m2, fx) = run(m2, Command::SplitWith(shown));
        assert_eq!(fx, [Effect::ActivateTab(shown)], "{page:?}");
        assert_eq!(
            m2.shown_split().map(|g| g.tabs.clone()),
            Some(vec![ids[0], shown]),
            "{page:?}"
        );
    }
}

#[test]
fn an_internal_page_typed_into_a_web_tab_opens_its_own_tab() {
    let (m, ids) = with_tabs(1);
    let web_url = m.tabs[&ids[0]].url.clone();
    let go = |tab| Command::Navigate {
        tab,
        url: InternalPage::Settings.url().into(),
    };
    // The web tab keeps its URL (and so its view and history); the page opens beside it.
    let (m, fx) = run(m, go(ids[0]));
    assert!(
        matches!(&fx[0], Effect::CreateTab { url, .. } if url == InternalPage::Settings.url()),
        "{fx:?}"
    );
    assert_eq!(m.tabs[&ids[0]].url, web_url);
    let settings = m.visible_tab().unwrap();
    assert!(m.is_internal(settings));
    // Again from the web tab: the open page is focused, no second one.
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, fx) = run(m, go(ids[0]));
    assert_eq!(fx, [Effect::ActivateTab(settings)]);
    // An internal tab navigates in place; an unknown tab does nothing.
    let (m, fx) = run(m, go(settings));
    assert!(matches!(&fx[..], [Effect::Navigate { tab, .. }] if *tab == settings));
    let (_, fx) = run(m, go(TabId(999)));
    assert!(fx.is_empty());
}

/// Built-in pages are split panes, the shell drawing one per split: beside web pages, from
/// either side, but never two in one split.
#[test]
fn a_split_holds_one_internal_page() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run(m, Command::OpenInternal(InternalPage::Settings));
    let settings = m.visible_tab().unwrap();
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::OpenInternal(InternalPage::History));
    let history = m.visible_tab().unwrap();
    /// (name, focused, split with, the split's panes after: none, refused)
    type Row = (&'static str, TabId, TabId, Option<Vec<TabId>>);
    let rows: [Row; 4] = [
        ("the focused page is internal", settings, ids[0], Some(vec![settings, ids[0]])),
        ("the page split with is", ids[0], settings, Some(vec![ids[0], settings])),
        ("both are", settings, history, None),
        ("two web pages", ids[0], ids[1], Some(vec![ids[0], ids[1]])),
    ];
    for (name, focus, with, panes) in rows {
        let (m, _) = run(m.clone(), Command::ActivateTab(focus));
        let (m, fx) = run(m, Command::SplitWith(with));
        assert_eq!(m.shown_split().map(|g| g.tabs.clone()), panes, "{name}");
        assert_eq!(fx.is_empty(), panes.is_none(), "{name}");
    }
    // a second internal page joining a split that holds one is refused; a web page is not
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::SplitWith(settings));
    let (m2, fx) = run(m.clone(), Command::SplitWith(history));
    assert!(fx.is_empty());
    assert_eq!(m2.shown_split().map(|g| g.tabs.len()), Some(2));
    let (m, fx) = run(m, Command::SplitWith(ids[1]));
    assert_eq!(fx, [Effect::ActivateTab(ids[1])]);
    assert_eq!(m.shown_split().map(|g| g.tabs.len()), Some(3));
}

#[test]
fn space_tint_and_icon_change_in_place() {
    let m = Model::new(t0());
    let (m, _) = run(
        m,
        Command::SetSpaceTint {
            space: s0(),
            tint: Rgb(9, 8, 7),
        },
    );
    let (m, _) = run(
        m,
        Command::SetSpaceIcon {
            space: s0(),
            icon: SpaceIcon::Study,
        },
    );
    assert_eq!(m.spaces[0].tint, Rgb(9, 8, 7));
    assert_eq!(m.spaces[0].icon, SpaceIcon::Study);
    assert_eq!(m.spaces[0].id, s0());
    // Unknown Spaces are ignored.
    let (m2, fx) = run(
        m.clone(),
        Command::SetSpaceIcon {
            space: SpaceId(99),
            icon: SpaceIcon::Work,
        },
    );
    assert!(fx.is_empty() && m2 == m);
    // New Spaces take the icons in turn.
    let (m, s1) = add_space(m, D);
    assert_eq!(
        m.spaces.iter().find(|s| s.id == s1).unwrap().icon,
        SpaceIcon::Work
    );
}

// ----- windows

use crate::window::{Geometry, Shown, WindowKind, WindowTarget};

fn opened(fx: &[Effect]) -> WindowId {
    fx.iter()
        .find_map(|e| match e {
            Effect::WindowOpened(w) => Some(*w),
            _ => None,
        })
        .expect("a window opened")
}

/// Every tab is on screen in at most one window (the spec's invariant, apart from `check`).
fn shown_once(m: &Model) {
    let mut all: Vec<TabId> = m.windows.iter().flat_map(|w| m.window_tabs(w.id)).collect();
    let n = all.len();
    all.sort_by_key(|t| t.0);
    all.dedup();
    assert_eq!(all.len(), n, "a tab in two windows: {:?}", m.windows);
}

#[test]
fn a_new_window_takes_a_space_no_window_shows_else_a_new_one() {
    let (m, _) = with_tabs(1);
    let (m, s1) = add_space(m, D);
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w = opened(&fx);
    assert_eq!(m.focused(), w);
    assert_eq!(m.active_space(), s1, "the free Space");
    assert!(
        fx.contains(&Effect::ShowCommandBar),
        "an empty Space opens the bar"
    );
    // every Space is shown now: a third window makes one
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w3 = opened(&fx);
    assert_eq!(m.spaces.len(), 3);
    assert_eq!(m.window(w3).unwrap().space(), Some(m.spaces[2].id));
    assert_eq!(m.windows.len(), 3);
    // asking for a shown Space focuses the window showing it
    let (m, fx) = run(m, Command::NewWindow { space: Some(s0()) });
    assert_eq!(fx, vec![Effect::FocusWindow(WindowId(0))]);
    assert_eq!((m.windows.len(), m.active_space()), (3, s0()));
}

#[test]
fn two_main_windows_never_show_one_space() {
    let (m, _) = with_tabs(1);
    let (m, s1) = add_space(m, D);
    let (m, _) = run(m, Command::NewWindow { space: Some(s1) });
    let mut bad = m.clone();
    bad.windows[1].shown = Shown::Space(s0());
    assert!(bad.check().is_err());
    // switching to the Space the other window shows focuses that window instead
    let (m, fx) = run(m, Command::SwitchSpace(s0()));
    assert!(fx.contains(&Effect::FocusWindow(WindowId(0))), "{fx:?}");
    assert_eq!(m.focused(), WindowId(0));
    assert_eq!(m.window(WindowId(1)).unwrap().space(), Some(s1));
}

#[test]
fn a_tab_is_on_screen_in_at_most_one_window() {
    let (m, t) = with_tabs(3);
    let commands = [
        Command::NewWindow { space: None },
        Command::MoveTabToWindow {
            tab: t[0],
            to: WindowTarget::Existing(WindowId(1)),
        },
        Command::ActivateTab(t[1]),
        Command::SplitWith(t[2]),
        Command::MoveTabToWindow {
            tab: t[2],
            to: WindowTarget::New,
        },
        Command::OpenPopupWindow {
            url: "https://p.test".into(),
            opener: t[1],
            size: Some((400, 300)),
            pos: None,
        },
        Command::ActivateTab(t[0]),
        Command::SwitchSpace(s0()),
        Command::OpenLittle {
            url: "https://l.test".into(),
            space: None,
            container: None,
        },
        Command::FocusWindow(WindowId(1)),
        Command::CloseWindow(WindowId(2)),
        Command::MoveTabToWindow {
            tab: t[1],
            to: WindowTarget::Existing(WindowId(1)),
        },
        Command::CloseTab(t[0]),
    ];
    let mut m = m;
    for c in commands {
        m = run_all(m, c).0;
        shown_once(&m);
    }
}

#[test]
fn moving_a_tab_to_a_window_fixes_the_source_space() {
    let (m, t) = with_tabs(3);
    let (m, _) = run(m, Command::SplitWith(t[0]));
    // t[2] | t[0] shown, t[0] focused
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w = opened(&fx);
    let (m, fx) = run(
        m,
        Command::MoveTabToWindow {
            tab: t[0],
            to: WindowTarget::Existing(w),
        },
    );
    assert!(fx.contains(&Effect::FocusWindow(w)) || m.focused() == w);
    assert_eq!(m.window_tabs(w), vec![t[0]]);
    assert_eq!(m.window_tabs(WindowId(0)), vec![t[2]], "the split's heir");
    assert!(m.spaces[0].split.is_none(), "a split of one is no split");
    // a tear-out makes a window on a fresh Space
    let (m, fx) = run(
        m,
        Command::MoveTabToWindow {
            tab: t[1],
            to: WindowTarget::New,
        },
    );
    let w2 = opened(&fx);
    assert_eq!((m.focused(), m.window_tabs(w2)), (w2, vec![t[1]]));
    assert_eq!(m.spaces.len(), 3);
    assert!(!fx.contains(&Effect::ShowCommandBar));
    // a Popup window refuses to be a target
    let (m2, fx) = run(
        m.clone(),
        Command::OpenPopupWindow {
            url: "https://p.test".into(),
            opener: t[2],
            size: None,
            pos: None,
        },
    );
    let p = opened(&fx);
    let (m3, fx) = run(
        m2.clone(),
        Command::MoveTabToWindow {
            tab: t[2],
            to: WindowTarget::Existing(p),
        },
    );
    assert!(fx.is_empty() && m3 == m2);
}

#[test]
fn every_window_keeps_its_tabs_awake() {
    let (m, t) = with_tabs(2);
    let (m, _) = run(m, Command::NewWindow { space: None });
    let (m, _) = run(
        m,
        Command::MoveTabToWindow {
            tab: t[0],
            to: WindowTarget::Existing(WindowId(1)),
        },
    );
    let (m, _) = run(
        m,
        Command::Tick {
            wall_ms: 0,
            now: t0() + std::time::Duration::from_secs(3600),
            pressure: Pressure::None,
        },
    );
    for tab in [t[0], t[1]] {
        assert_eq!(m.tabs[&tab].lifecycle.liveness, Liveness::Active, "{tab:?}");
    }
    assert_eq!(m.visible_tabs().len(), 2);
}

#[test]
fn closing_windows() {
    let (m, t) = with_tabs(1);
    let (m, fx) = run(m, Command::CloseWindow(WindowId(0)));
    assert_eq!(fx, vec![Effect::LastWindowClosed]);
    assert_eq!(
        m.windows.len(),
        1,
        "the last Main window stays until the host quits"
    );
    let (m, _) = run(m, Command::NewWindow { space: None });
    let (m, fx) = run(m, Command::CloseWindow(WindowId(0)));
    assert!(fx.contains(&Effect::WindowClosed(WindowId(0))));
    assert_eq!(m.windows.len(), 1);
    assert!(m.tabs.contains_key(&t[0]), "the Space and its tabs stay");
    assert_ne!(m.tabs[&t[0]].lifecycle.liveness, Liveness::Active, "hidden");
    let (_, fx) = run(m, Command::CloseWindow(WindowId(1)));
    assert_eq!(fx, vec![Effect::LastWindowClosed]);
}

#[test]
fn a_new_window_opens_cascaded_from_the_focused_main_window() {
    let (m, _) = with_tabs(1);
    let g = Geometry {
        width: 800,
        height: 600,
        pos: Some((100, 50)),
        monitor: Some("A".into()),
        ..Geometry::default()
    };
    let (m, _) = run(
        m,
        Command::ResizeWindow {
            window: WindowId(0),
            geometry: g.clone(),
        },
    );
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w = opened(&fx);
    assert_eq!(m.window(w).unwrap().geometry, Some(g.cascaded()));
}

#[test]
fn popup_and_little_windows_own_their_tab() {
    let (m, t) = with_tabs(1);
    let (m, fx) = run(
        m,
        Command::OpenPopupWindow {
            url: "https://p.test".into(),
            opener: t[0],
            size: Some((400, 300)),
            pos: None,
        },
    );
    let p = opened(&fx);
    let pt = m.window(p).unwrap().tab().unwrap();
    assert_eq!(m.window(p).unwrap().kind(), WindowKind::Popup);
    assert_eq!(
        m.window(p).unwrap().geometry,
        Some(Geometry {
            width: 400,
            height: 300,
            ..Geometry::default()
        })
    );
    // `left`/`top` place the window; without a size there is no geometry to put them in
    let (m2, fx) = run(
        m.clone(),
        Command::OpenPopupWindow {
            url: "https://q.test".into(),
            opener: t[0],
            size: Some((400, 300)),
            pos: Some((-1800, 40)),
        },
    );
    assert_eq!(
        m2.window(opened(&fx)).unwrap().geometry,
        Some(Geometry {
            width: 400,
            height: 300,
            pos: Some((-1800, 40)),
            ..Geometry::default()
        })
    );
    let (m2, fx) = run(
        m.clone(),
        Command::OpenPopupWindow {
            url: "https://q.test".into(),
            opener: t[0],
            size: None,
            pos: Some((40, 60)),
        },
    );
    assert_eq!(m2.window(opened(&fx)).unwrap().geometry, None);
    assert_eq!(m.visible_tab(), Some(pt), "the popup has focus");
    assert_eq!(m.spaces[0].active, Some(t[0]), "the Space keeps its page");
    assert_eq!(m.order(s0()), vec![t[0], pt], "beside its opener");
    // activating its row brings the window forward
    let (m, _) = run(m, Command::FocusWindow(WindowId(0)));
    let (m, fx) = run(m, Command::ActivateTab(pt));
    assert_eq!(fx, vec![Effect::FocusWindow(p)]);
    // a popup tab that closes takes its window
    let (m2, fx) = run(m.clone(), Command::CloseTab(pt));
    assert!(fx.contains(&Effect::WindowClosed(p)));
    assert_eq!(m2.windows.len(), 1);
    // closing the window closes the tab
    let (m2, fx) = run(m, Command::CloseWindow(p));
    assert!(fx.contains(&Effect::CloseTab(pt)) && fx.contains(&Effect::WindowClosed(p)));
    assert!(!m2.tabs.contains_key(&pt));
    // a windowed tab never joins a split
    let (m, fx) = run(
        m2,
        Command::OpenLittle {
            url: "https://l.test".into(),
            space: None,
            container: None,
        },
    );
    let l = opened(&fx);
    let lt = m.window(l).unwrap().tab().unwrap();
    let (m, _) = run(m, Command::FocusWindow(WindowId(0)));
    let (m2, fx) = run(m.clone(), Command::SplitWith(lt));
    assert!(fx.is_empty() && m2 == m);
}

#[test]
fn a_little_window_promotes_into_a_space() {
    let (m, _) = with_tabs(1);
    let (m, s1) = add_space(m, D);
    let (m, fx) = run(
        m,
        Command::OpenLittle {
            url: "https://l.test".into(),
            space: None,
            container: None,
        },
    );
    let l = opened(&fx);
    let lt = m.window(l).unwrap().tab().unwrap();
    assert_eq!(m.window(l).unwrap().kind(), WindowKind::Little);
    let (m, fx) = run(
        m,
        Command::PromoteLittle {
            window: l,
            space: s1,
        },
    );
    assert!(fx.contains(&Effect::WindowClosed(l)), "{fx:?}");
    assert_eq!(m.windows.len(), 1);
    assert_eq!((m.active_space(), m.visible_tab()), (s1, Some(lt)));
    assert_eq!(m.order(s1), vec![lt]);
    // a Popup window is not promoted
    let (m, fx) = run(
        m,
        Command::OpenPopupWindow {
            url: "https://p.test".into(),
            opener: lt,
            size: None,
            pos: None,
        },
    );
    let p = opened(&fx);
    let (m2, fx) = run(
        m.clone(),
        Command::PromoteLittle {
            window: p,
            space: s1,
        },
    );
    assert!(fx.is_empty() && m2 == m);
}

#[test]
fn deleting_a_space_another_window_shows() {
    let (m, _) = with_tabs(1);
    let (m, s1) = add_space(m, D);
    let (m, s2) = add_space(m, D);
    let (m, _) = run(m, Command::NewWindow { space: Some(s1) });
    let (m, _) = run(m, Command::FocusWindow(WindowId(0)));
    let (m, _) = run(m, Command::DeleteSpace(s1));
    assert_eq!(
        m.window(WindowId(1)).unwrap().space(),
        Some(s2),
        "takes the free one"
    );
    let (m, fx) = run(m, Command::DeleteSpace(s2));
    assert!(
        fx.contains(&Effect::WindowClosed(WindowId(1))),
        "no Space left for it"
    );
    assert_eq!(m.windows.len(), 1);
}

#[test]
fn from_parts_checks_the_windows() {
    let (m, _) = with_tabs(2);
    let (m, s1) = add_space(m, D);
    let parts = |windows: Vec<Window>| {
        Model::from_parts(
            m.spaces.clone(),
            m.containers.clone(),
            m.tabs.values().cloned().collect(),
            windows,
            t0(),
        )
    };
    assert!(parts(vec![]).is_err(), "no window");
    assert!(
        parts(vec![
            Window::main(WindowId(0), s0()),
            Window::main(WindowId(0), s1)
        ])
        .is_err(),
        "duplicate id"
    );
    assert!(
        parts(vec![
            Window::main(WindowId(0), s0()),
            Window::main(WindowId(1), s0())
        ])
        .is_err(),
        "one Space twice"
    );
    let popup_only = Window {
        id: WindowId(0),
        shown: Shown::Popup(TabId(1)),
        geometry: None,
    };
    assert!(parts(vec![popup_only]).is_err(), "no Main window");
    let m2 = parts(vec![
        Window::main(WindowId(4), s1),
        Window::main(WindowId(2), s0()),
    ])
    .unwrap();
    assert_eq!(m2.active_space(), s0());
    let (_, fx) = run(m2, Command::NewWindow { space: None });
    assert_eq!(
        opened(&fx),
        WindowId(5),
        "ids continue past the restored ones"
    );
}

#[test]
fn the_pane_limit_counts_only_the_focused_window() {
    // three Main windows, each showing a tab; the focused one has a second tab to split with
    let (m, _) = with_tabs(1);
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let _ = opened(&fx);
    let (m, _) = open(m, None);
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let _ = opened(&fx);
    let (m, a) = open(m, None);
    let (m, b) = open(m, None);
    assert_eq!(m.visible_tabs().len(), 3, "one tab in each window");
    assert!(m.pane_room());
    assert_eq!(m.split_candidate(), Some(a));
    // a 2-pane split may take a third pane while other windows show their tabs
    let (m, _) = run(m, Command::SplitWith(a));
    let (m, c) = open(m, None);
    let (m, _) = run(m, Command::ActivateTab(b));
    assert_eq!(m.window_tabs(m.focused()), vec![b, a]);
    assert_eq!(m.split_candidate(), Some(c));
    let (m, _) = run(m, Command::SplitWith(c));
    assert_eq!(m.window_tabs(m.focused()).len(), MAX_PANES);
    assert!(!m.pane_room() && m.split_candidate().is_none(), "full");
}

#[test]
fn the_main_window_is_the_one_showing_the_active_space() {
    let (m, _) = with_tabs(1);
    assert_eq!(m.main_window(), WindowId(0));
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w = opened(&fx);
    assert!(
        fx.contains(&Effect::ShowCommandBar),
        "an empty Space opens the bar"
    );
    assert_eq!(m.main_window(), w, "the bar goes to the new window");
    // a Little window's focus leaves the Main window as it was
    let (m, _) = run(
        m,
        Command::OpenLittle {
            url: "https://l.test".into(),
            space: None,
            container: None,
        },
    );
    assert_ne!(m.focused(), w);
    assert_eq!(m.main_window(), w);
    assert_eq!(
        m.window(m.main_window()).unwrap().space(),
        Some(m.active_space())
    );
}

#[test]
fn a_background_tab_follows_its_opener_unshown() {
    let (m, ids) = with_tabs(2);
    let (m, child) = open(m, Some(ids[1]));
    let (m, fx) = run(m, Command::ActivateTab(ids[0]));
    assert!(!fx.is_empty());
    let before = m.clone();
    let (m, fx) = run_all(
        m,
        Command::BackgroundTab {
            url: "https://bg.test".into(),
            opener: ids[0],
        },
    );
    let Effect::CreateTab { tab: bg, .. } = fx[0] else {
        panic!("{fx:?}")
    };
    assert!(
        !fx.iter().any(|e| matches!(
            e,
            Effect::ActivateTab(_)
                | Effect::Lifecycle {
                    effect: crate::lifecycle::Effect::Show,
                    ..
                }
        )),
        "never shown: {fx:?}"
    );
    assert_eq!(m.visible_tab(), Some(ids[0]));
    assert_eq!(m.tabs[&bg].opener, Some(ids[0]));
    assert_eq!(
        m.tabs[&child].opener, before.tabs[&child].opener,
        "other openers stay"
    );
    assert_eq!(m.order(s0())[1], bg, "right after its opener");
}

// ----- never sleep, wall-clock stamps, tidy

use crate::lifecycle::Keep;

fn tab_at(m: Model, url: &str) -> (Model, TabId) {
    let (m, fx) = run(
        m,
        Command::NewTab {
            url: url.into(),
            container: None,
            opener: None,
        },
    );
    let Effect::CreateTab { tab, .. } = fx[0] else {
        panic!("no CreateTab: {fx:?}")
    };
    (m, tab)
}

fn tick_wall(m: Model, after: Duration, wall_ms: i64) -> Model {
    let now = m.clock + after;
    run_all(
        m,
        Command::Tick {
            now,
            wall_ms,
            pressure: Pressure::None,
        },
    )
    .0
}

#[test]
fn hiding_a_tab_stamps_the_wall_clock_and_a_first_tick_stamps_unknown_ones() {
    let m = Model::new(t0());
    let (m, a) = tab_at(m, "https://a.test/");
    let m = tick_wall(m, Duration::ZERO, 1_000);
    assert_eq!(m.tabs[&a].last_active_ms, 0, "a visible tab is not stamped");
    let (m, b) = tab_at(m, "https://b.test/"); // hides a at wall 1000
    assert_eq!(m.tabs[&a].last_active_ms, 1_000);
    assert_eq!(m.tabs[&b].last_active_ms, 0);
    let m = tick_wall(m, Duration::from_secs(5), 9_000);
    assert_eq!(m.tabs[&a].last_active_ms, 1_000, "a known stamp stays");
    assert_eq!(
        m.tabs[&b].last_active_ms, 0,
        "the visible tab stays unknown"
    );

    // restored tabs (stamp 0, hidden) start their clock at the first tick
    let restored = Model::from_parts(
        m.spaces.clone(),
        m.containers.clone(),
        m.tabs
            .values()
            .map(|t| Tab {
                last_active_ms: 0,
                ..t.clone()
            })
            .collect(),
        m.windows.clone(),
        t0(),
    )
    .unwrap();
    let m = tick_wall(restored, Duration::ZERO, 50_000);
    assert_eq!(m.tabs[&a].last_active_ms, 50_000);
    let m = tick_wall(m, Duration::ZERO, 0);
    assert_eq!(
        m.tabs[&a].last_active_ms, 50_000,
        "an unknown wall clock stamps nothing"
    );
}

/// Two tabs, the first hidden; the Space's tabs sleep after the default policy's 15 minutes.
fn hidden_pair() -> (Model, TabId, TabId) {
    let (m, ids) = with_tabs(2);
    (m, ids[0], ids[1])
}

#[test]
fn a_never_sleep_space_keeps_its_hidden_tab_awake_until_switched_off() {
    let (m, a, _) = hidden_pair();
    let (m, _) = run(
        m,
        Command::SetSpaceNeverSleep {
            space: s0(),
            on: true,
        },
    );
    assert_eq!(m.tabs[&a].lifecycle.keep, Keep::Awake);
    let (m, _) = tick(m, Duration::from_secs(3600));
    assert_eq!(liveness(&m, a), Liveness::Background);
    let (m, fx) = run_all(
        m,
        Command::SetSpaceNeverSleep {
            space: s0(),
            on: false,
        },
    );
    assert_eq!(m.tabs[&a].lifecycle.keep, Keep::Normal);
    assert_eq!(
        liveness(&m, a),
        Liveness::Discarded,
        "overdue: asleep at once"
    );
    assert!(lifecycle_effects(&fx).contains(&life(a, Life::Discard)));
}

#[test]
fn never_sleep_sites_cover_subdomains_and_follow_the_url() {
    let m = Model::new(t0());
    let (m, a) = tab_at(m, "https://docs.example.com/x");
    let (m, b) = tab_at(m, "https://other.test/");
    let (m, _) = run(
        m,
        Command::SetNeverSleepSites {
            side: Side::Normal,
            sites: vec!["example.com".into()],
        },
    );
    assert_eq!(m.tabs[&a].lifecycle.keep, Keep::Awake);
    assert_eq!(m.tabs[&b].lifecycle.keep, Keep::Normal);
    let (m, _) = tick(m, Duration::from_secs(3600));
    assert_eq!(liveness(&m, a), Liveness::Background);
    // navigating away ends the exemption
    let (m, _) = run(
        m,
        Command::UrlChanged {
            tab: a,
            url: "https://elsewhere.test/".into(),
        },
    );
    assert_eq!(liveness(&m, a), Liveness::Discarded);
}

#[test]
fn a_frozen_tab_turned_awake_is_unfrozen() {
    let (m, a, _) = hidden_pair();
    let (m, _) = tick(m, Duration::from_secs(120));
    assert_eq!(liveness(&m, a), Liveness::Frozen);
    let (m, fx) = run_all(
        m,
        Command::SetNeverSleepSites {
            side: Side::Normal,
            sites: vec!["a.test".into()],
        },
    );
    assert_eq!(liveness(&m, a), Liveness::Background);
    assert_eq!(lifecycle_effects(&fx), vec![life(a, Life::Unfreeze)]);
}

fn layout(m: &Model) -> Vec<Item> {
    m.spaces[0].items.clone()
}

fn names(m: &Model) -> Vec<(String, Vec<TabId>)> {
    m.spaces[0]
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Folder(f) => Some((f.name.clone(), f.tabs.clone())),
            Item::Tab(_) => None,
        })
        .collect()
}

#[test]
fn tidy_groups_by_site_and_set_layout_undoes_it() {
    let mut m = Model::new(t0());
    let mut ids = vec![];
    for url in [
        "https://a.example.com/1",
        "https://solo.test/",
        "https://b.example.com/2",
        "https://other.org/",
        "https://example.com/3",
        "https://other.org/2",
    ] {
        let (m2, t) = tab_at(m, url);
        m = m2;
        ids.push(t);
    }
    let (m, _) = run(m, Command::PinTab(ids[3]));
    let before = layout(&m);
    let (tidied, fx) = run(m.clone(), Command::TidyTabs { space: s0() });
    assert_eq!(
        fx,
        vec![Effect::Tidied {
            space: s0(),
            before: before.clone()
        }]
    );
    // other.org has one unpinned tab left (the other is pinned): not grouped
    assert_eq!(
        names(&tidied),
        vec![("example.com".to_string(), vec![ids[0], ids[2], ids[4]])]
    );
    // the folder stands where the site's first tab stood
    assert!(matches!(tidied.spaces[0].items[0], Item::Folder(_)));
    let (undone, fx) = run(
        tidied,
        Command::SetLayout {
            space: s0(),
            items: before,
        },
    );
    assert_eq!(fx, vec![]);
    // everything but the folder id counter, which never goes back
    assert_eq!((&undone.spaces, &undone.tabs), (&m.spaces, &m.tabs));
}

#[test]
fn tidy_reuses_a_folder_of_the_same_name_and_does_nothing_when_there_is_nothing() {
    let m = Model::new(t0());
    let (m, a) = tab_at(m, "https://x.test/1");
    let (m, b) = tab_at(m, "https://www.x.test/2");
    let (m, _) = run(
        m,
        Command::NewFolder {
            space: s0(),
            name: "X.Test".into(),
        },
    );
    let (m, fx) = run(m, Command::TidyTabs { space: s0() });
    assert!(matches!(fx[..], [Effect::Tidied { .. }]));
    assert_eq!(names(&m), vec![("X.Test".to_string(), vec![a, b])]);
    assert_eq!(m.spaces[0].items.len(), 1);
    let (m2, fx) = run(m.clone(), Command::TidyTabs { space: s0() });
    assert_eq!((m2, fx), (m, vec![]));
}

#[test]
fn set_layout_stays_valid_after_a_close_or_a_new_tab() {
    let m = Model::new(t0());
    let (m, a) = tab_at(m, "https://x.test/1");
    let (m, b) = tab_at(m, "https://x.test/2");
    let (m, c) = tab_at(m, "https://y.test/");
    let (m, fx) = run(m, Command::TidyTabs { space: s0() });
    let Effect::Tidied { before, .. } = fx[0].clone() else {
        panic!()
    };
    let (m, _) = run(m, Command::CloseTab(a));
    let (m, d) = tab_at(m, "https://z.test/");
    let (m, _) = run(
        m,
        Command::SetLayout {
            space: s0(),
            items: before,
        },
    );
    m.check().unwrap();
    assert_eq!(
        layout(&m),
        vec![Item::Tab(b), Item::Tab(c), Item::Tab(d)],
        "the closed tab is gone, the new one follows, the emptied folder is dropped"
    );
    let (unchanged, _) = run(
        m.clone(),
        Command::SetLayout {
            space: SpaceId(99),
            items: vec![],
        },
    );
    assert_eq!(unchanged, m);
}

#[test]
fn duplicate_opens_the_same_page_after_it_and_shows_it() {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, fx) = run(m, Command::DuplicateTab(ids[0]));
    let Effect::CreateTab {
        tab,
        ref url,
        container,
    } = fx[0]
    else {
        panic!("{fx:?}")
    };
    assert_eq!((url.as_str(), container), ("https://a.test", D));
    assert_eq!(m.order(s0()), vec![ids[0], tab, ids[1], ids[2]]);
    assert_eq!(m.visible_tab(), Some(tab));
}

#[test]
fn close_others_keeps_the_pinned_and_shows_the_tab() {
    let (m, ids) = with_tabs(4);
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, fx) = run(m, Command::CloseOthers(ids[2]));
    assert_eq!(m.order(s0()), vec![ids[0], ids[2]]);
    assert_eq!(m.visible_tab(), Some(ids[2]));
    assert!(fx.contains(&Effect::CloseTab(ids[1])) && fx.contains(&Effect::CloseTab(ids[3])));
    assert!(!fx.contains(&Effect::CloseTab(ids[0])));
}

#[test]
fn a_folder_collapses_and_expands() {
    let (m, _) = with_tabs(1);
    let (m, _) = run(
        m,
        Command::NewFolder {
            space: s0(),
            name: "f".into(),
        },
    );
    let f = FolderId(0);
    let (m, _) = run(
        m,
        Command::SetFolderCollapsed {
            folder: f,
            collapsed: true,
        },
    );
    let collapsed = |m: &Model| {
        m.spaces[0]
            .items
            .iter()
            .any(|i| matches!(i, Item::Folder(x) if x.collapsed))
    };
    assert!(collapsed(&m));
    let (m, _) = run(
        m,
        Command::SetFolderCollapsed {
            folder: f,
            collapsed: false,
        },
    );
    assert!(!collapsed(&m));
}

// ----- picture-in-picture windows

fn pip(m: Model, tab: TabId) -> (Model, WindowId) {
    let (m, fx) = run(m, Command::OpenPip { tab });
    let w = opened(&fx);
    (m, w)
}

#[test]
fn a_pip_window_frames_its_tab_and_its_main_window_leaves_it_out() {
    let (m, t) = with_tabs(2);
    let main = m.focused();
    let (m, p) = pip(m, t[1]);
    assert_eq!(m.window(p).unwrap().kind(), WindowKind::Pip);
    assert_eq!(m.window(p).unwrap().owner(), Some(t[1]));
    assert_eq!(
        m.window(p).unwrap().tab(),
        None,
        "not a Little or Popup tab"
    );
    assert_eq!(m.pip_tab(), Some(t[1]));
    assert_eq!(m.window_tabs(p), [t[1]]);
    assert_eq!(m.window_tabs(main), Vec::<TabId>::new(), "a card there");
    assert_eq!(active(&m), Some(t[1]), "still its Space's active tab");
    assert_eq!(m.visible_tabs(), [t[1]]);
    assert_eq!(m.tabs[&t[1]].lifecycle.liveness, Liveness::Active);
    assert_eq!(m.focused(), main, "it takes no focus");
    assert_eq!(m.window_of(t[1]), Some(p));
    shown_once(&m);
    // The Main window keeps it as its tab (the address, the card) but sends it no keys.
    assert_eq!(m.window_tab(main), Some(t[1]));
    assert_eq!(m.window_key_tab(main), None);
    assert_eq!(m.window_key_tab(p), Some(t[1]));
    // Another tab in the Main window: both on screen, each in one window.
    let (m, _) = run(m, Command::ActivateTab(t[0]));
    assert_eq!(m.window_tabs(main), [t[0]]);
    assert_eq!(m.window_tabs(p), [t[1]]);
    assert_eq!(m.tabs[&t[1]].lifecycle.liveness, Liveness::Active);
    shown_once(&m);
}

#[test]
fn pip_is_refused_for_what_it_cannot_show() {
    let (m, t) = with_tabs(2);
    let (m, _) = pip(m, t[1]);
    let before = m.clone();
    // One at a time.
    let (m, fx) = run(m, Command::OpenPip { tab: t[0] });
    assert!(fx.is_empty());
    assert_eq!(m, before);
    let (m, t) = with_tabs(2);
    let unknown = TabId(999);
    let (m2, fx) = run(m.clone(), Command::OpenPip { tab: unknown });
    assert!(fx.is_empty());
    assert_eq!(m2, m);
    // A peek.
    let (m, g) = peek(m, t[1]);
    let before = m.clone();
    let (m, fx) = run(m, Command::OpenPip { tab: g });
    assert!(fx.is_empty());
    assert_eq!(m, before);
    // A Popup window's tab.
    let (m, fx) = run(
        m,
        Command::OpenPopupWindow {
            url: "https://p.test".into(),
            opener: t[0],
            size: None,
            pos: None,
        },
    );
    let popup_tab = m.window(opened(&fx)).unwrap().tab().unwrap();
    let before = m.clone();
    let (m, fx) = run(m, Command::OpenPip { tab: popup_tab });
    assert!(fx.is_empty());
    assert_eq!(m, before);
}

#[test]
fn closing_the_pip_window_puts_the_tab_back() {
    let (m, t) = with_tabs(2);
    let main = m.focused();
    let (m, p) = pip(m, t[1]);
    let (m, fx) = run(m, Command::ClosePip);
    assert_eq!(fx, vec![Effect::WindowClosed(p)]);
    assert_eq!(m.window_tabs(main), [t[1]]);
    assert_eq!(m.pip_tab(), None);
    // The window's own close does the same, and the tab stays.
    let (m, p) = pip(m, t[1]);
    let (m, fx) = run(m, Command::CloseWindow(p));
    assert_eq!(fx, vec![Effect::WindowClosed(p)]);
    assert!(m.tabs.contains_key(&t[1]));
    assert_eq!(m.window_tabs(main), [t[1]]);
    // Nothing to close.
    let (_, fx) = run(m, Command::ClosePip);
    assert!(fx.is_empty());
}

#[test]
fn return_shows_the_tab_where_it_lives() {
    let (m, t) = with_tabs(2);
    let main = m.focused();
    let (m, _) = pip(m, t[1]);
    let (m, _) = run(m, Command::ActivateTab(t[0]));
    let (m, fx) = run(m, Command::ReturnPip);
    assert!(fx.contains(&Effect::ActivateTab(t[1])));
    assert_eq!(m.window_tabs(main), [t[1]]);
    assert_ne!(m.tabs[&t[0]].lifecycle.liveness, Liveness::Active);
    // From another Space: the window goes back to the tab's.
    let (m, s1) = add_space(m, D);
    let (m, _) = pip(m, t[1]);
    let (m, _) = run(m, Command::SwitchSpace(s1));
    assert_eq!(m.window_tabs(main), Vec::<TabId>::new());
    assert_eq!(m.visible_tabs(), [t[1]], "still in its window");
    let (m, _) = run(m, Command::ReturnPip);
    assert_eq!(m.active_space(), s0());
    assert_eq!(m.window_tabs(main), [t[1]]);
    assert_eq!(m.windows.len(), 1);
}

#[test]
fn the_pip_window_goes_with_its_tab_and_remembers_its_place() {
    let (m, t) = with_tabs(2);
    let (m, p) = pip(m, t[1]);
    let g = Geometry {
        width: 480,
        height: 270,
        pos: Some((10, 20)),
        ..Geometry::default()
    };
    let (m, _) = run(
        m,
        Command::ResizeWindow {
            window: p,
            geometry: g.clone(),
        },
    );
    let (m, fx) = run(m, Command::CloseTab(t[1]));
    assert!(fx.contains(&Effect::WindowClosed(p)));
    assert_eq!(m.pip_window(), None);
    let (m, p) = pip(m, t[0]);
    assert_eq!(
        m.window(p).unwrap().geometry,
        Some(g),
        "where the last one was"
    );
    // Not part of a session: only Main windows are kept (wubble-store `session`).
    assert!(m.window(p).unwrap().space().is_none());
}

#[test]
fn a_split_pane_on_screen_never_goes_to_pip_and_the_pip_tab_never_splits() {
    let (m, t) = with_tabs(3);
    let main = m.focused();
    let (m, _) = run(m, Command::SplitWith(t[1]));
    assert_eq!(m.window_tabs(main).len(), 2);
    // A pane, focused or not: its half of the split would be a hole.
    for pane in [t[1], t[2]] {
        assert!(!m.can_pip(pane));
        let before = m.clone();
        let (m2, fx) = run(m.clone(), Command::OpenPip { tab: pane });
        assert!(fx.is_empty());
        assert_eq!(m2, before);
    }
    assert!(m.can_pip(t[0]), "not in the split");
    // The split left for another tab: each of its panes may go (a tab switch's auto
    // picture-in-picture), and the split coming back with it out is a model that holds (the
    // host returns it then).
    let (m, _) = run(m, Command::ActivateTab(t[0]));
    for pane in [t[1], t[2]] {
        assert!(m.can_pip(pane), "{pane:?} of a split not shown");
    }
    let (m, _) = pip(m, t[1]);
    assert_eq!(m.pip_tab(), Some(t[1]));
    let (m, _) = run(m, Command::ActivateTab(t[2]));
    assert_eq!(m.window_tabs(main), [t[2]], "the pane out is left out");
    assert_eq!(m.pip_tab(), Some(t[1]));
    // The tab in picture-in-picture is not split with, nor split beside.
    let (m, t) = with_tabs(3);
    let (m, _) = pip(m, t[2]);
    for (focus, with) in [(t[2], t[1]), (t[1], t[2])] {
        let (m, _) = run(m.clone(), Command::ActivateTab(focus));
        let before = m.clone();
        let (m, fx) = run(m, Command::SplitWith(with));
        assert!(fx.is_empty());
        assert_eq!(m, before);
        assert_eq!(m.split_of(t[2]), None);
    }
}

#[test]
fn check_holds_one_pip_window_of_a_placed_tab() {
    let (m, t) = with_tabs(2);
    let (mut m, _) = pip(m, t[1]);
    m.check().expect("one Pip window");
    let mut two = m.clone();
    two.windows.insert(
        0,
        Window {
            id: WindowId(9),
            shown: Shown::Pip(t[0]),
            geometry: None,
        },
    );
    two.next_window = 10;
    assert!(two.check().is_err(), "two Pip windows");
    let mut split = m.clone();
    split.spaces[0].split = Some(SplitGroup {
        tabs: vec![t[0], t[1]],
        stops: even(2),
    });
    let err = split.check().expect_err("a split pane");
    assert!(err.contains("Pip window"), "{err}");
    m.windows[0].shown = Shown::Pip(TabId(99));
    assert!(m.check().is_err(), "an unknown tab");
}

// ----- openers (Chromium `tab_strip_model.cc`: DetermineNewSelectedIndex, FixOpeners,
// InsertTabAtImpl, OnActiveTabChanged)

fn background(m: Model, opener: TabId) -> (Model, TabId) {
    let (m, fx) = run(
        m,
        Command::BackgroundTab {
            url: "https://b.test".into(),
            opener,
        },
    );
    let Effect::CreateTab { tab, .. } = fx[0] else {
        panic!("no CreateTab: {fx:?}")
    };
    (m, tab)
}

/// Three tabs, the first one shown.
fn three_on_first() -> (Model, Vec<TabId>) {
    let (m, ids) = with_tabs(3);
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    (m, ids)
}

#[test]
fn a_closed_child_returns_to_its_opener_past_the_tab_above_it() {
    // the owner's case: YouTube (pinned) opens a shop link, which goes last, under another tab
    let (m, ids) = three_on_first();
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, shop) = open(m, Some(ids[0]));
    let order = m.order(s0());
    assert_eq!(
        order[order.len() - 2],
        ids[2],
        "another tab sits right above it"
    );
    let (m, fx) = run(m, Command::CloseTab(shop));
    assert_eq!(active(&m), Some(ids[0]));
    assert_eq!(
        fx,
        vec![Effect::CloseTab(shop), Effect::ActivateTab(ids[0])]
    );
}

#[test]
fn an_opener_closed_first_leaves_its_child_to_close_to_its_neighbour() {
    let (m, ids) = three_on_first();
    let (m, child) = open(m, Some(ids[0]));
    let (m, _) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(m.tabs[&child].opener, None);
    let (m, _) = run(m, Command::ActivateTab(child));
    // [child, 1, 2]: the next one
    let (m, _) = run(m, Command::CloseTab(child));
    assert_eq!(active(&m), Some(ids[1]));
}

#[test]
fn a_switch_to_an_unrelated_tab_forgets_the_opener() {
    let (m, ids) = three_on_first();
    let (m, child) = open(m, Some(ids[0]));
    // [0, child, 1, 2]: away to 2, back to the child
    let (m, _) = run(m, Command::ActivateTab(ids[2]));
    assert_eq!(m.tabs[&child].opener, None);
    let (m, _) = run(m, Command::ActivateTab(child));
    let (m, _) = run(m, Command::CloseTab(child));
    assert_eq!(active(&m), Some(ids[1]), "its neighbour, as today");
}

#[test]
fn switching_between_a_child_and_its_opener_keeps_the_relation() {
    let (m, ids) = three_on_first();
    let (m, child) = open(m, Some(ids[0]));
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, _) = run(m, Command::ActivateTab(child));
    assert_eq!(m.tabs[&child].opener, Some(ids[0]));
    let (m, _) = run(m, Command::CloseTab(child));
    assert_eq!(active(&m), Some(ids[0]));
}

#[test]
fn a_background_tab_selected_from_its_opener_closes_back_to_it() {
    let (m, ids) = three_on_first();
    let (m, b) = background(m, ids[0]);
    assert_eq!(active(&m), Some(ids[0]), "opened in the background");
    let (m, _) = run(m, Command::ActivateTab(b));
    let (m, _) = run(m, Command::CloseTab(b));
    assert_eq!(active(&m), Some(ids[0]));
}

#[test]
fn a_closed_child_picks_its_sibling_before_its_opener() {
    // [0, c2, c1, 1, 2]: two background links from 0, then the user goes to c1
    let (m, ids) = three_on_first();
    let (m, c1) = background(m, ids[0]);
    let (m, c2) = background(m, ids[0]);
    assert_eq!(&m.order(s0())[..3], &[ids[0], c2, c1]);
    let (m, _) = run(m, Command::ActivateTab(c2));
    let (m, _) = run(m, Command::CloseTab(c2));
    assert_eq!(active(&m), Some(c1), "a tab of the same opener");
    let (m, _) = run(m, Command::CloseTab(c1));
    assert_eq!(active(&m), Some(ids[0]), "then the opener");
}

#[test]
fn closing_the_opener_selects_the_tab_it_opened() {
    let (m, ids) = three_on_first();
    let (m, b) = background(m, ids[0]);
    let (m, _) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(active(&m), Some(b));
}

#[test]
fn closing_a_hidden_tab_never_moves_the_selection() {
    let (m, ids) = three_on_first();
    let (m, child) = open(m, Some(ids[0]));
    let (m, fx) = run(m, Command::CloseTab(ids[0]));
    assert_eq!(active(&m), Some(child));
    assert_eq!(fx, vec![Effect::CloseTab(ids[0])]);
}

#[test]
fn a_chain_closes_back_up_as_chromium_does() {
    // A opens B, B opens C, all in the foreground: opening C makes B forget A (one opener
    // relation at a time), so C closes to B and B to its neighbour.
    let (m, ids) = three_on_first();
    let (m, b) = open(m, Some(ids[0]));
    let (m, c) = open(m, Some(b));
    assert_eq!((m.tabs[&b].opener, m.tabs[&c].opener), (None, Some(b)));
    let (m, _) = run(m, Command::CloseTab(c));
    assert_eq!(active(&m), Some(b));
    let (m, _) = run(m, Command::CloseTab(b));
    assert_eq!(active(&m), Some(ids[1]));
    // in the background the chain stays: B closed first hands C to A (FixOpeners)
    let (m, ids) = three_on_first();
    let (m, b) = background(m, ids[0]);
    let (m, _) = run(m, Command::ActivateTab(b));
    let (m, c) = background(m, b);
    let (m, _) = run(m, Command::CloseTab(b));
    assert_eq!(active(&m), Some(c), "B's child first");
    assert_eq!(m.tabs[&c].opener, Some(ids[0]), "C inherits B's opener");
    let (m, _) = run(m, Command::CloseTab(c));
    assert_eq!(active(&m), Some(ids[0]));
}

#[test]
fn a_promoted_peek_closes_back_to_its_page() {
    let (m, ids) = three_on_first();
    let w = m.windows[0].id;
    let (m, _) = run(
        m,
        Command::OpenGlance {
            window: w,
            opener: ids[0],
            url: "https://p.test".into(),
        },
    );
    let g = m.glance_of(w).expect("a peek").tab;
    let (m, _) = run(
        m,
        Command::PromoteGlance {
            window: w,
            to: GlanceTo::Tab,
        },
    );
    assert_eq!(active(&m), Some(g));
    let (m, _) = run(m, Command::CloseTab(g));
    assert_eq!(active(&m), Some(ids[0]));
}

#[test]
fn shell_tabs_have_no_opener_and_close_to_their_neighbour() {
    // [0, 1, 2, new]: Ctrl+T's tab goes last and closes to the one above it
    let (m, ids) = three_on_first();
    let (m, t) = open(m, None);
    assert_eq!(m.tabs[&t].opener, None);
    let (m, _) = run(m, Command::CloseTab(t));
    assert_eq!(active(&m), Some(ids[2]));
}

fn extension_tab(container: ContainerId, space: Option<SpaceId>, show: bool) -> Command {
    Command::ExtensionTab {
        url: "https://ext.test".into(),
        container,
        space,
        show,
    }
}

#[test]
fn an_extension_tab_takes_the_asking_extensions_container_not_the_shown_tabs() {
    let (m, ids) = with_tabs(2);
    let (m, _) = run(m, Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, fx) = run(m, extension_tab(work, None, false));
    let [Effect::CreateTab { tab, container, .. }] = fx[..] else {
        panic!("{fx:?}")
    };
    assert_eq!(container, work);
    // Last in the active Space (tabs.create's default index), the shown tab still shown.
    let space = &m.spaces[m.space_idx(m.active_space()).unwrap()];
    assert_eq!(space.items.last(), Some(&Item::Tab(tab)));
    assert_eq!(active(&m), Some(ids[1]));
    assert_eq!(m.tabs[&tab].opener, None);
}

#[test]
fn a_shown_extension_tab_goes_to_its_space_and_becomes_its_active_tab() {
    let (m, _) = with_tabs(1);
    let (m, s1) = add_space(m, D);
    let (m, fx) = run(m, extension_tab(D, Some(s1), true));
    let Some(Effect::CreateTab { tab, .. }) = fx.first().cloned() else {
        panic!("{fx:?}")
    };
    assert!(fx.contains(&Effect::ActivateTab(tab)));
    let space = &m.spaces[m.space_idx(s1).unwrap()];
    assert_eq!(space.active, Some(tab));
    assert!(space.items.contains(&Item::Tab(tab)));
}

#[test]
fn an_extension_tab_in_a_gone_container_or_space_opens_nothing() {
    let (m, _) = with_tabs(1);
    let (m2, fx) = run(m.clone(), extension_tab(ContainerId(99), None, true));
    assert!(fx.is_empty());
    assert_eq!(m2.tabs.len(), m.tabs.len());
    let (_, fx) = run(m, extension_tab(D, Some(SpaceId(99)), true));
    assert!(fx.is_empty());
}

#[test]
fn an_agent_space_stays_one() {
    let (m, _) = with_tabs(1);
    assert!(m.spaces[0].kind != SpaceKind::Agent);
    let (m, fx) = run(m, Command::MarkAgentSpace(s0()));
    assert!(m.spaces[0].kind == SpaceKind::Agent && fx.is_empty());
    // the session's Space outlives it: nothing but deleting the Space ends the mark
    let (m, _) = run(
        m,
        Command::SetSpaceNeverSleep {
            space: s0(),
            on: false,
        },
    );
    assert!(m.spaces[0].kind == SpaceKind::Agent);
    assert!(
        m.for_session(&[]).spaces[0].kind == SpaceKind::Agent,
        "saved like any Space"
    );
}

/// A link from another app that a routing rule sends to a container opens in that container from
/// its first request, whatever the Space's own container is (cookies never cross).
#[test]
fn a_little_window_opens_in_the_container_it_is_given() {
    let m = Model::new(Instant::now());
    let (m, _) = apply(
        m,
        Command::NewContainer {
            name: "Bank".into(),
        },
    );
    let bank = m.containers[1].id;
    let (m, fx) = apply(
        m,
        Command::OpenLittle {
            url: "https://bank.test".into(),
            space: None,
            container: Some(bank),
        },
    );
    let creates: Vec<_> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::CreateTab { container, .. } => Some(*container),
            _ => None,
        })
        .collect();
    assert_eq!(creates, [bank]);
    let tab = m.window(m.focused()).unwrap().tab().unwrap();
    assert_eq!(m.tabs[&tab].container, bank);
    // an unknown container opens nothing (never a silent fall back to another jar)
    let (m2, fx) = apply(
        m.clone(),
        Command::OpenLittle {
            url: "https://x.test".into(),
            space: None,
            container: Some(ContainerId(99)),
        },
    );
    assert!(fx.is_empty());
    assert_eq!(m2.tabs.len(), m.tabs.len());
    // none: the Space's own
    let (m3, _) = apply(
        m,
        Command::OpenLittle {
            url: "https://y.test".into(),
            space: None,
            container: None,
        },
    );
    let tab = m3.window(m3.focused()).unwrap().tab().unwrap();
    assert_eq!(m3.tabs[&tab].container, ContainerId::DEFAULT);
}

#[test]
fn open_beside_does_not_inject_windowed_tab_into_space_split() {
    let m = Model::new(Instant::now());
    let (m, _) = apply(
        m,
        Command::NewTab {
            url: "https://a.test".into(),
            opener: None,
            container: None,
        },
    );
    let (m, _) = apply(
        m,
        Command::OpenLittle {
            url: "https://little.test".into(),
            space: None,
            container: None,
        },
    );
    let little_tab = m.window(m.focused()).unwrap().tab().unwrap();
    assert!(m.windowed(little_tab).is_some());
    let (m, _) = apply(
        m,
        Command::OpenBeside {
            url: "https://b.test".into(),
        },
    );
    assert!(m.check().is_ok(), "{:?}", m.check());
    let split = m.spaces[0].split.as_ref().expect("split in active space");
    assert!(!split.tabs.contains(&little_tab));
}

#[test]
fn discard_now_refuses_internal_pages() {
    let m = Model::new(Instant::now());
    let (m, _) = apply(m, Command::OpenInternal(InternalPage::Settings));
    let settings_tab = m.visible_tab().unwrap();
    let (m, _) = apply(
        m,
        Command::NewTab {
            url: "https://a.test".into(),
            opener: None,
            container: None,
        },
    );
    let (m, fx) = apply(m, Command::DiscardNow(settings_tab));
    assert!(fx.is_empty());
    assert_ne!(
        m.tabs[&settings_tab].lifecycle.liveness,
        Liveness::Discarded
    );
    assert!(m.check().is_ok(), "{:?}", m.check());
}

/// Deleting an agent Space (a session's teardown, the dustbin) records none of its tabs as the
/// user's closed tabs; deleting a user's Space, or closing a tab of an agent Space by itself,
/// does.
#[test]
fn an_agent_space_s_tabs_are_not_the_user_s_closed_tabs() {
    let (m, ids) = with_tabs(1);
    let (m, s) = add_space(m, ContainerId::DEFAULT);
    let (m, _) = run(
        m,
        Command::OpenInSpace {
            space: s,
            url: "https://agent.test/".into(),
        },
    );
    let tab = *m.order(s).first().unwrap();
    assert!(
        m.closes_into_recent(tab, &Command::DeleteSpace(s)),
        "a user's Space"
    );
    let (m, _) = run(m, Command::MarkAgentSpace(s));
    assert!(!m.closes_into_recent(tab, &Command::DeleteSpace(s)));
    assert!(
        m.closes_into_recent(tab, &Command::CloseTab(tab)),
        "a kept tab closed by hand"
    );
    // another Space's deletion leaves the rule for this one's tabs alone
    assert!(m.closes_into_recent(ids[0], &Command::DeleteSpace(s)));
}

#[test]
fn reopen_in_container_remaps_pip_and_glances() {
    let (m, ids) = with_tabs(1);
    let (m, _) = run(
        m,
        Command::NewContainer {
            name: "Work".into(),
        },
    );
    let work = m.containers[1].id;
    let win = m.main_window();

    // PiP tab stays in PiP after ReopenInContainer.
    let (m, _) = run(m, Command::OpenPip { tab: ids[0] });
    assert_eq!(m.pip_tab(), Some(ids[0]));
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: ids[0],
            container: work,
        },
    );
    let new_pip = m.pip_tab().expect("PiP window stays open");
    assert_ne!(new_pip, ids[0]);
    assert_eq!(m.tabs[&new_pip].container, work);
    assert!(m.check().is_ok(), "{:?}", m.check());

    // Glance opener and glance tab both survive ReopenInContainer.
    let (m, _) = run(m, Command::ClosePip);
    let (m, _) = run(
        m,
        Command::OpenGlance {
            window: win,
            opener: new_pip,
            url: "https://peek.test".into(),
        },
    );
    let g_tab = m.glance_of(win).unwrap().tab;
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: g_tab,
            container: ContainerId::DEFAULT,
        },
    );
    let g = m.glance_of(win).expect("peek stays open");
    assert_ne!(g.tab, g_tab);
    assert_eq!(m.tabs[&g.tab].container, ContainerId::DEFAULT);
    assert!(m.check().is_ok(), "{:?}", m.check());
}

#[test]
fn moving_a_glance_tab_to_another_space_or_window_promotes_it() {
    let (m, ids) = with_tabs(1);
    let (m, s2) = add_space(m, ContainerId::DEFAULT);
    let win = m.main_window();
    let (m, _) = run(
        m,
        Command::OpenGlance {
            window: win,
            opener: ids[0],
            url: "https://peek.test".into(),
        },
    );
    let g_tab = m.glance_of(win).unwrap().tab;
    let (m, _) = run(
        m,
        Command::MoveTab {
            tab: g_tab,
            to: Dest::Normal {
                space: s2,
                index: 0,
            },
        },
    );
    assert!(m.glance_of(win).is_none());
    assert_eq!(m.spaces[1].active, Some(g_tab));
    assert!(m.check().is_ok(), "{:?}", m.check());
}

/// A private window on `url` (focused), its window and Space.
fn private_window(m: Model, url: &str) -> (Model, WindowId, SpaceId) {
    let (m, fx) = run(
        m,
        Command::NewPrivateWindow {
            url: Some(url.into()),
        },
    );
    let w = opened(&fx);
    let s = m.window(w).and_then(Window::space).expect("a Main window");
    (m, w, s)
}

/// Each window lists the Spaces of its own side of the private line, and a normal window made
/// while every normal Space is shown is named among the normal Spaces only.
#[test]
fn a_window_lists_the_spaces_of_its_own_side() {
    let (m, _) = with_tabs(1);
    let (m, s1) = add_space(m, D);
    let (m, _, p1) = private_window(m, "https://p.test/");
    let (m, _, p2) = private_window(m, "https://q.test/");
    let listed =
        |m: &Model, s: SpaceId| -> Vec<SpaceId> { m.listed_spaces(s).map(|s| s.id).collect() };
    for (from, want) in [
        (s0(), vec![s0(), s1]),
        (s1, vec![s0(), s1]),
        (p1, vec![p1]),
        (p2, vec![p2]),
    ] {
        assert_eq!(listed(&m, from), want, "from {from:?}");
    }
    let (m, _) = run(m, Command::NewWindow { space: Some(s1) });
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w = opened(&fx);
    let s = m.window(w).and_then(Window::space).unwrap();
    let si = m.space_idx(s).unwrap();
    assert_eq!(
        m.spaces[si].name, "Space 3",
        "the two private Spaces are not counted"
    );
}

/// The in-memory container lives exactly as long as a private window does.
#[test]
fn the_ephemeral_container_lives_with_the_private_windows() {
    let (m, _) = with_tabs(1);
    let (m, w, _) = private_window(m, "https://p.test/");
    let c = m
        .containers
        .iter()
        .find(|c| c.storage == Storage::Memory)
        .map(|c| c.id)
        .unwrap();
    let (m, fx) = run(m, Command::CloseWindow(w));
    let gone = Effect::ContainerDeleted {
        id: c,
        storage: Storage::Memory,
    };
    assert!(fx.contains(&gone), "{fx:?}");
    assert!(!m.spaces.iter().any(|s| s.kind == SpaceKind::Private));
    assert!(!m.containers.iter().any(|c| c.storage == Storage::Memory));
    let mut bad = m.clone();
    bad.containers.push(crate::space::Container {
        id: ContainerId(99),
        name: "Private".into(),
        storage: Storage::Memory,
    });
    assert!(
        bad.check().is_err(),
        "an ephemeral container without a private window"
    );
}

/// Moving the last tab out of a private window closes that window (into another private
/// window, or torn out into a new one); a normal window emptied so stays on its Space.
#[test]
fn an_emptied_private_window_closes_and_a_normal_one_stays() {
    let (m, t) = with_tabs(1);
    let (m, w1, s1) = private_window(m, "https://p.test/");
    let (m, w2, s2) = private_window(m, "https://q.test/");
    let moved = m.order(s2)[0];
    let (m, fx) = run(
        m,
        Command::MoveTabToWindow {
            tab: moved,
            to: WindowTarget::Existing(w1),
        },
    );
    assert!(fx.contains(&Effect::WindowClosed(w2)), "{fx:?}");
    assert!(m.window(w2).is_none() && m.space_idx(s2).is_none());
    assert_eq!((m.window_tab(w1), m.order(s1).len()), (Some(moved), 2));
    // one of two torn out: the window keeps the other
    let (m, fx) = run(
        m,
        Command::MoveTabToWindow {
            tab: moved,
            to: WindowTarget::New,
        },
    );
    let w3 = opened(&fx);
    assert!(m.side_of_window(w3) == Side::Private && m.window(w1).is_some());
    // the last one torn out: a new private window takes it and the old one closes, the
    // container staying with the new one
    let last = m.order(s1)[0];
    let (m, fx) = run(
        m,
        Command::MoveTabToWindow {
            tab: last,
            to: WindowTarget::New,
        },
    );
    let w4 = opened(&fx);
    assert!(fx.contains(&Effect::WindowClosed(w1)), "{fx:?}");
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::ContainerDeleted { .. }))
    );
    assert!(m.side_of_window(w4) == Side::Private && m.side_of_tab(last) == Side::Private);
    // a normal window emptied so stays, on its empty Space
    let (m, fx) = run(
        m,
        Command::MoveTabToWindow {
            tab: t[0],
            to: WindowTarget::New,
        },
    );
    assert!(!fx.contains(&Effect::WindowClosed(WindowId(0))), "{fx:?}");
    assert_eq!(m.window(WindowId(0)).and_then(Window::space), Some(s0()));
    assert!(m.order(s0()).is_empty());
}

/// Mod+N in a private window opens a normal window on a normal Space (neither the private
/// Space nor refused); Mod+Shift+N opens another private window sharing the one container.
#[test]
fn a_new_window_from_a_private_window_is_a_normal_one() {
    let (m, _) = with_tabs(1);
    let (m, p, _) = private_window(m, "https://p.test/");
    assert_eq!(m.focused(), p);
    let (m, fx) = run(m, Command::NewWindow { space: None });
    let w = opened(&fx);
    let s = m.active_space();
    assert_eq!(m.focused(), w);
    assert!(
        m.side_of_window(w) != Side::Private && m.side_of_space(s) != Side::Private && s != s0()
    );
    assert_eq!(m.spaces[m.space_idx(s).unwrap()].container, D);
    let (m, t) = open(m, None);
    assert!(m.side_of_tab(t) != Side::Private && m.location(t).map(|(s, _)| s) == Some(s));
    let (m, fx) = run(m, Command::NewPrivateWindow { url: None });
    let w2 = opened(&fx);
    assert!(m.side_of_window(w2) == Side::Private && fx.contains(&Effect::ShowCommandBar));
    assert_eq!(
        m.spaces
            .iter()
            .filter(|s| s.kind == SpaceKind::Private)
            .count(),
        2
    );
    assert_eq!(
        m.containers
            .iter()
            .filter(|c| c.storage == Storage::Memory)
            .count(),
        1
    );
}

/// Which private tabs a close ends (their downloads ask first): a private window's own, every
/// one when the app quits or its last Main window closes, none for a normal window.
#[test]
fn a_close_ends_the_private_tabs_it_takes_with_it() {
    use crate::window::Closing;
    let (m, _) = with_tabs(1);
    let (m, w1, s1) = private_window(m, "https://p.test/");
    let (m, w2, s2) = private_window(m, "https://q.test/");
    let (m, _) = open(m, None);
    let p1 = m.order(s1);
    let p2 = m.order(s2);
    let p12 = [p1.clone(), p2.clone()].concat();
    let sorted = |mut v: Vec<TabId>| {
        v.sort();
        v
    };
    for (closing, want) in [
        (Closing::Window(w1), p1.clone()),
        (Closing::Window(w2), p2.clone()),
        (Closing::Window(WindowId(0)), vec![]),
        (Closing::Window(WindowId(99)), vec![]),
        (Closing::App, p12.clone()),
        (Closing::Restart, p12.clone()),
    ] {
        assert_eq!(
            sorted(m.private_tabs_ended(closing)),
            sorted(want),
            "{closing:?}"
        );
    }
    // with the normal window gone, closing the one Main window left quits: every private tab
    let (m, _) = run(m, Command::CloseWindow(WindowId(0)));
    let (m, _) = run(m, Command::CloseWindow(w2));
    assert_eq!(
        sorted(m.private_tabs_ended(Closing::Window(w1))),
        sorted(p1)
    );
}

/// What a command came to across the private line ([`crossing`]).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Came {
    /// Refused: the model is as it was and nothing is owed to the engine (no effect).
    Refused,
    Applied,
    /// Done in the last normal window (window 0), which shows the Space, focused.
    ToNormalWindow,
}

/// Every command that could cross the private line, from each side to each: a normal window
/// (0) on s0 with tab n0, a normal Space s1 (tab n1) and a Work container; a private window
/// on ps with tab p0 in the Memory container. One row per command x side pair.
#[test]
fn every_command_keeps_to_its_side_of_the_private_line() {
    use Came::*;
    let (m, n) = with_tabs(1);
    let n0 = n[0];
    let (m, _) = run(
        m,
        Command::NewContainer {
            name: "Work".into(),
        },
    );
    let work = m.containers.last().unwrap().id;
    let (m, s1) = add_space(m, D);
    let (m, _) = run(
        m,
        Command::OpenInSpace {
            space: s1,
            url: "https://n1.test/".into(),
        },
    );
    let (m, pw, ps) = private_window(m, "https://p.test/");
    let (m, pw2, ps2) = private_window(m, "https://q.test/");
    let p0 = m.order(ps)[0];
    let n1 = m.order(s1)[0];
    let mem = m
        .containers
        .iter()
        .find(|c| c.storage == Storage::Memory)
        .unwrap()
        .id;
    let w0 = WindowId(0);
    let url = || "https://x.test/".to_string();
    let to = |space| Dest::Normal { space, index: 0 };
    // (row, the window focused first, command, what it comes to)
    let rows: Vec<(&str, WindowId, Command, Came)> = vec![
        (
            "move N->P",
            w0,
            Command::MoveTab {
                tab: n0,
                to: to(ps),
            },
            Refused,
        ),
        (
            "move P->N",
            pw,
            Command::MoveTab {
                tab: p0,
                to: to(s0()),
            },
            Refused,
        ),
        (
            "move N->N",
            w0,
            Command::MoveTab {
                tab: n0,
                to: to(s1),
            },
            Applied,
        ),
        (
            "move P->P",
            pw,
            Command::MoveTab {
                tab: p0,
                to: to(ps2),
            },
            Applied,
        ),
        (
            "to window P->P",
            pw,
            Command::MoveTabToWindow {
                tab: p0,
                to: WindowTarget::Existing(pw2),
            },
            Applied,
        ),
        (
            "tear out P",
            pw,
            Command::MoveTabToWindow {
                tab: p0,
                to: WindowTarget::New,
            },
            Applied,
        ),
        ("activate P from N", w0, Command::ActivateTab(p0), Applied),
        (
            "activate N from P",
            pw,
            Command::ActivateTab(n1),
            ToNormalWindow,
        ),
        ("activate N from N", w0, Command::ActivateTab(n1), Applied),
        (
            "new space in the memory container",
            w0,
            Command::NewSpace {
                name: "X".into(),
                tint: DEFAULT_TINT,
                container: mem,
            },
            Refused,
        ),
        (
            "new space in a normal container",
            w0,
            Command::NewSpace {
                name: "X".into(),
                tint: DEFAULT_TINT,
                container: work,
            },
            Applied,
        ),
        (
            "new window on P from N",
            w0,
            Command::NewWindow { space: Some(ps) },
            Refused,
        ),
        (
            "new window on N from P",
            pw,
            Command::NewWindow { space: Some(s1) },
            Applied,
        ),
        (
            "to window N->P",
            w0,
            Command::MoveTabToWindow {
                tab: n0,
                to: WindowTarget::Existing(pw),
            },
            Refused,
        ),
        (
            "to window P->N",
            pw,
            Command::MoveTabToWindow {
                tab: p0,
                to: WindowTarget::Existing(w0),
            },
            Refused,
        ),
        ("switch N->P", w0, Command::SwitchSpace(ps), Refused),
        ("switch P->N", pw, Command::SwitchSpace(s1), ToNormalWindow),
        ("switch N->N", w0, Command::SwitchSpace(s1), Applied),
        (
            "new tab, normal container, in P",
            pw,
            Command::NewTab {
                url: url(),
                container: Some(work),
                opener: None,
            },
            Refused,
        ),
        (
            "new tab, memory container, in N",
            w0,
            Command::NewTab {
                url: url(),
                container: Some(mem),
                opener: None,
            },
            Refused,
        ),
        (
            "new tab, memory container, in P",
            pw,
            Command::NewTab {
                url: url(),
                container: Some(mem),
                opener: None,
            },
            Applied,
        ),
        (
            "space container P<-N",
            w0,
            Command::SetSpaceContainer {
                space: ps,
                container: work,
            },
            Refused,
        ),
        (
            "space container N<-P",
            w0,
            Command::SetSpaceContainer {
                space: s1,
                container: mem,
            },
            Refused,
        ),
        (
            "space container N<-N",
            w0,
            Command::SetSpaceContainer {
                space: s1,
                container: work,
            },
            Applied,
        ),
        (
            "reopen P in N",
            pw,
            Command::ReopenInContainer {
                tab: p0,
                container: work,
            },
            Refused,
        ),
        (
            "reopen N in P",
            w0,
            Command::ReopenInContainer {
                tab: n0,
                container: mem,
            },
            Refused,
        ),
        (
            "reopen N in N",
            w0,
            Command::ReopenInContainer {
                tab: n0,
                container: work,
            },
            Applied,
        ),
        (
            "delete container P",
            w0,
            Command::DeleteContainer(mem),
            Refused,
        ),
        (
            "delete container N",
            w0,
            Command::DeleteContainer(work),
            Applied,
        ),
        ("delete space P", w0, Command::DeleteSpace(ps), Refused),
        ("delete space N", w0, Command::DeleteSpace(s1), Applied),
        ("mark agent P", w0, Command::MarkAgentSpace(ps), Refused),
        ("mark agent N", w0, Command::MarkAgentSpace(s1), Applied),
        (
            "agent tab in P",
            w0,
            Command::OpenInSpace {
                space: ps,
                url: url(),
            },
            Refused,
        ),
        (
            "agent tab in N",
            pw,
            Command::OpenInSpace {
                space: s1,
                url: url(),
            },
            Applied,
        ),
        (
            "extension tab in P",
            pw,
            Command::ExtensionTab {
                url: url(),
                container: work,
                space: Some(ps),
                show: true,
            },
            Refused,
        ),
        (
            "extension tab of the memory profile",
            w0,
            Command::ExtensionTab {
                url: url(),
                container: mem,
                space: None,
                show: true,
            },
            Refused,
        ),
        (
            "extension tab, no Space, from P",
            pw,
            Command::ExtensionTab {
                url: url(),
                container: work,
                space: None,
                show: false,
            },
            Applied,
        ),
        (
            "app link in P",
            pw,
            Command::OpenLittle {
                url: url(),
                space: Some(ps),
                container: None,
            },
            Refused,
        ),
        (
            "app link, no Space, from P",
            pw,
            Command::OpenLittle {
                url: url(),
                space: None,
                container: None,
            },
            Applied,
        ),
    ];
    for (row, focus, command, want) in rows {
        let (before, _) = run(m.clone(), Command::FocusWindow(focus));
        let (after, fx) = run(before.clone(), command);
        let came = if after == before && fx.is_empty() {
            Refused
        } else if focus == pw
            && after.focused() == w0
            && after.window(w0).and_then(Window::space) == Some(s1)
        {
            ToNormalWindow
        } else {
            Applied
        };
        assert_eq!(came, want, "{row}: {fx:?}");
        // whatever was applied, nothing crossed: every private tab is still in Memory
        for t in after.tabs.keys() {
            let private = after
                .location(*t)
                .is_some_and(|(s, _)| after.side_of_space(s) == Side::Private);
            assert_eq!(
                after.side_of_tab(*t) == Side::Private,
                private,
                "{row}: {t:?}"
            );
        }
    }
    // a Little window's page never becomes a private window's tab
    let (m, fx) = run(
        m,
        Command::OpenLittle {
            url: url(),
            space: None,
            container: None,
        },
    );
    let little = opened(&fx);
    let (same, fx) = run(
        m.clone(),
        Command::PromoteLittle {
            window: little,
            space: ps,
        },
    );
    assert!(same == m && fx.is_empty(), "{fx:?}");
    let (m, _) = run(
        m,
        Command::PromoteLittle {
            window: little,
            space: s1,
        },
    );
    assert!(m.window(little).is_none());
}

/// A session (what the store writes, the import's write too) holds nothing of a private
/// window: its Space, tabs, window and in-memory container go, focused or not, and the last
/// normal window stays (or is made, when every normal window was closed).
#[test]
fn a_private_window_is_no_part_of_a_session() {
    let (m, n) = with_tabs(1);
    let (m, _, ps) = private_window(m, "https://p.test/");
    let p0 = m.order(ps)[0];
    let (alone, _) = run(m.clone(), Command::CloseWindow(WindowId(0)));
    let mut windowless = m.clone();
    windowless.windows.retain(|w| w.space() != Some(ps));
    // (row, the live model)
    for (row, live) in [
        ("focused", m.clone()),
        ("the only Main window", alone),
        ("a private Space without its window", windowless),
    ] {
        let saved = live.for_session(&[]);
        saved.check().unwrap_or_else(|e| panic!("{row}: {e}"));
        assert!(saved.space_idx(ps).is_none(), "{row}");
        assert!(!saved.tabs.contains_key(&p0), "{row}");
        assert!(
            saved.containers.iter().all(|c| c.storage == Storage::Disk),
            "{row}"
        );
        assert!(
            saved.spaces.iter().all(|s| s.kind != SpaceKind::Private),
            "{row}"
        );
        assert!(
            saved.windows.iter().any(|w| w.kind() == WindowKind::Main),
            "{row}: a normal window to come back to"
        );
        assert_eq!(
            saved.side_of_space(saved.active_space()),
            Side::Normal,
            "{row}"
        );
        assert!(
            live.tabs.contains_key(&p0),
            "{row}: the live model keeps it"
        );
    }
    assert!(
        m.for_session(&[]).tabs.contains_key(&n[0]),
        "the normal tab stays"
    );
}

/// A download row is the private side's only while its container is a known Memory one; a row
/// of a deleted container stays with the normal side, where it was stored.
#[test]
fn a_record_is_private_only_in_a_known_memory_container() {
    let (m, _) = with_tabs(1);
    let (m, _) = run(
        m,
        Command::NewContainer {
            name: "Work".into(),
        },
    );
    let work = m.containers.last().unwrap().id;
    let (m, _, _) = private_window(m, "https://p.test/");
    let mem = m
        .containers
        .iter()
        .find(|c| c.storage == Storage::Memory)
        .unwrap()
        .id;
    let (gone, _) = run(m.clone(), Command::DeleteContainer(work));
    // (row, model, container, the record's side, the container's own side)
    for (row, m, c, record, own) in [
        ("default", &m, D, Side::Normal, Side::Normal),
        ("memory", &m, mem, Side::Private, Side::Private),
        ("deleted", &gone, work, Side::Normal, Side::Private),
        (
            "never known",
            &m,
            ContainerId(999),
            Side::Normal,
            Side::Private,
        ),
    ] {
        assert_eq!(m.side_of_record(c), record, "{row}");
        assert_eq!(m.side_of_container(c), own, "{row}");
    }
}

/// Each side's tabs run its own never-sleep list: a site kept awake from a private window
/// leaves the same site's normal tab to sleep, and the reverse.
#[test]
fn never_sleep_sites_hold_per_side() {
    let m = Model::new(t0());
    let (m, n) = tab_at(m, "https://a.test/");
    let (m, _, ps) = private_window(m, "https://a.test/");
    let p = m.order(ps)[0];
    // (row, side the list is set for, normal tab's keep, private tab's keep)
    for (row, side, normal, private) in [
        ("normal list", Side::Normal, Keep::Awake, Keep::Normal),
        ("private list", Side::Private, Keep::Normal, Keep::Awake),
    ] {
        let (m, _) = run(
            m.clone(),
            Command::SetNeverSleepSites {
                side,
                sites: vec!["a.test".into()],
            },
        );
        assert_eq!(m.tabs[&n].lifecycle.keep, normal, "{row}");
        assert_eq!(m.tabs[&p].lifecycle.keep, private, "{row}");
    }
}
