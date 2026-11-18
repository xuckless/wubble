use super::*;
use crate::space::{SplitGroup, even};
use crate::tab::Tab;

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
    assert_eq!(m.active_space, s0());
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
    let cmd = match r.below(23) {
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
                let order = m.order(m.active_space);
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
        20 => {
            let pressure = [Pressure::None, Pressure::Moderate, Pressure::Critical][r.below(3)];
            let now = m.clock + Duration::from_secs(r.below(2000) as u64);
            return (Command::Tick { now, pressure }, false);
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
            most_panes = most_panes.max(after.visible_tabs().len());
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
    assert_eq!(m.active_space, s2);
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
    assert_eq!((m.active_space, m.visible_tab()), (s2, Some(w)));
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
        m.active_space,
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
        m.active_space,
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
        m.active_space,
        t0(),
    );
    assert!(r.is_err(), "placed tab missing from tabs");
    let r = Model::from_parts(
        m.spaces.clone(),
        m.containers.clone(),
        vec![],
        SpaceId(99),
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
    m.spaces[m.space_idx(m.active_space).unwrap()].split.clone()
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
            m.active_space,
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
        // A split refuses every internal page.
        let (m2, fx) = run(m.clone(), Command::SplitWith(shown));
        assert!(fx.is_empty() && m2.shown_split().is_none(), "{page:?}");
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

#[test]
fn an_internal_tab_never_joins_a_split() {
    let (m, ids) = with_tabs(1);
    let (m, _) = run(m, Command::OpenInternal(InternalPage::Settings));
    let settings = m.visible_tab().unwrap();
    let (m2, fx) = run(m.clone(), Command::SplitWith(ids[0]));
    assert!(fx.is_empty() && m2.shown_split().is_none());
    let (m, _) = run(m, Command::ActivateTab(ids[0]));
    let (m, fx) = run(m, Command::SplitWith(settings));
    assert!(fx.is_empty() && m.shown_split().is_none());
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
