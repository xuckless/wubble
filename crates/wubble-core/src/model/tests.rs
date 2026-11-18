use super::*;

const D: ContainerId = ContainerId::DEFAULT;

fn run(m: Model, c: Command) -> (Model, Vec<Effect>) {
    let (m, fx) = apply(m, c);
    m.check().expect("invariants");
    (m, fx)
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
    let mut m = Model::new();
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
    m.spaces[m.space_idx(m.active_space).unwrap()].active
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
    let m = Model::new();
    let (m2, fx) = run(m.clone(), Command::OpenCommandBar);
    assert_eq!(m2, m);
    assert_eq!(fx, vec![Effect::ShowCommandBar]);
}

#[test]
fn new_tab_into_empty_space_is_active() {
    let (m, fx) = run(
        Model::new(),
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
fn new_tab_container_override_and_unknown() {
    let (m, _) = run(
        Model::new(),
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
    assert_eq!(m.tabs[&TabId(1)].container, Some(work));
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
    // opener child (set up directly: NewTab always activates, so a live opener
    // relation to a background tab arises only transiently)
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
fn activate_switches_space_and_forgets_openers() {
    let (m, ids) = with_tabs(1);
    let (m, t) = open(m, Some(ids[0]));
    assert_eq!(m.tabs[&t].opener, Some(ids[0]));
    let (m, s2) = add_space(m, D);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, fx) = run(m, Command::ActivateTab(ids[0]));
    assert_eq!(m.active_space, s0());
    assert_eq!(m.tabs[&t].opener, None);
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
    assert_eq!(m.tabs[&ids[1]].lifecycle.pin, Pin::Pinned);
    let (m, _) = run(m, Command::UnpinTab(ids[1]));
    assert_eq!(m.order(s0()), vec![ids[2], ids[1], ids[0]]);
    assert!(matches!(m.location(ids[1]), Some((_, Place::Normal(0)))));
    assert_eq!(m.tabs[&ids[1]].lifecycle.pin, Pin::Unpinned);
    // Moving into the pinned section pins, into the normal section unpins.
    let to = Dest::Pinned {
        space: s0(),
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[0], to });
    assert_eq!(m.tabs[&ids[0]].lifecycle.pin, Pin::Pinned);
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
    assert_eq!(m.tabs[&ids[1]].lifecycle.pin, Pin::Unpinned);
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
        Model::new(),
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
    assert_eq!(m.active_space, s0());
    let to = Dest::Normal {
        space: s2,
        index: 0,
    };
    let (m, fx) = run(m, Command::MoveTab { tab: ids[1], to });
    // Default-container tab stays Default inside the Work Space.
    assert_eq!(m.effective_container(ids[1]), D);
    assert_eq!(m.tabs[&ids[1]].container, Some(D));
    // It was the active tab of the shown Space: source falls back to the other tab.
    assert_eq!(active(&m), Some(ids[0]));
    assert_eq!(fx, vec![Effect::ActivateTab(ids[0])]);
    // The destination had no active tab; it remembers the moved one.
    assert_eq!(m.spaces[1].active, Some(ids[1]));
    // Moving back to the Default Space normalises the override away.
    let to = Dest::Normal {
        space: s0(),
        index: 0,
    };
    let (m, _) = run(m, Command::MoveTab { tab: ids[1], to });
    assert_eq!(m.tabs[&ids[1]].container, None);
}

#[test]
fn delete_space_closes_tabs_and_falls_back() {
    let (m, _) = run(Model::new(), Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, s2) = add_space(m, work);
    let (m, ids) = with_tabs_in(m, 2);
    let (m, _) = run(m, Command::PinTab(ids[0]));
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, fx) = run(m, Command::DeleteSpace(s2));
    assert_eq!(m.spaces.len(), 1);
    assert_eq!(m.active_space, s0());
    assert!(fx.contains(&Effect::SpaceChanged(s0())));
    assert_eq!(m.containers.len(), 2, "shared container untouched");
    // Now delete the only tabs-owning Space's tabs with it: active falls back to next.
    let (m, s3) = add_space(m, D);
    let (m, fx) = run(m, Command::DeleteSpace(s0()));
    assert_eq!(m.active_space, s3);
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
        vec![Effect::SpaceChanged(s2)],
        "empty Space pops no bar"
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
    let (m, _) = run(Model::new(), Command::NewContainer { name: "W".into() });
    let work = m.containers[1].id;
    let (m, s2) = add_space(m, work);
    let (m, a) = open(m, None);
    let (m, _) = run(m, Command::SwitchSpace(s2));
    let (m, b) = open(m, None);
    assert_eq!(m.effective_container(b), work);
    let (m, _) = run(
        m,
        Command::ReopenInContainer {
            tab: a,
            container: work,
        },
    );
    let (m, _) = run(m, Command::DeleteContainer(work));
    assert_eq!(m.containers.len(), 1);
    assert_eq!(m.spaces[1].container, D);
    assert!(m.tabs.values().all(|t| t.container.is_none()));
    // Default is not deletable.
    let (m2, fx) = run(m.clone(), Command::DeleteContainer(D));
    assert_eq!((m2, fx), (m, vec![]));
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
    assert_eq!(m.effective_container(ids[0]), D);
    let (m, t) = open(m, None);
    assert_eq!(m.effective_container(t), work);
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
    assert_eq!(m.effective_container(new), work);
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
        Model::new(),
        Command::RenameSpace {
            space: SpaceId(0),
            name: "Home".into(),
        },
    );
    assert_eq!(m.spaces[0].name, "Home");
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
    let cmd = match r.below(18) {
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
        _ => tab(r)
            .zip(container(r))
            .map(|(tab, container)| Command::ReopenInContainer { tab, container }),
    };
    let named = cmd.is_some();
    (cmd.unwrap_or(Command::OpenCommandBar), bogus && named)
}

#[test]
fn random_sequences_keep_invariants() {
    for seed in 1..=200u64 {
        let mut r = Xorshift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut m = Model::new();
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
            m = after;
        }
    }
}
