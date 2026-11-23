use std::collections::BTreeMap;

use super::super::{Esc, Group, Keys, Kind, Layer, ROWS};
use super::*;
use crate::start::rand::{Random, Rng};
use Kind::*;

const W1: WindowId = WindowId(1);
const W2: WindowId = WindowId(2);
const A: TabId = TabId(1);
const B: TabId = TabId(2);
const C: TabId = TabId(3);

#[derive(Clone, Debug)]
struct T {
    shown: BTreeMap<WindowId, Option<TabId>>,
    kinds: BTreeMap<WindowId, WindowKind>,
    page_keys: bool,
    others: Vec<Layer>,
    machine: Vec<Kind>,
}

impl Env for T {
    fn shown(&self, w: WindowId) -> Option<TabId> {
        self.shown.get(&w).copied().flatten()
    }
    fn showing(&self, tab: TabId) -> Option<WindowId> {
        self.shown
            .iter()
            .find(|&(_, &s)| s == Some(tab))
            .map(|(&w, _)| w)
    }
    fn kind(&self, w: WindowId) -> WindowKind {
        self.kinds.get(&w).copied().unwrap_or(WindowKind::Main)
    }
    fn page_has_keys(&self, _: WindowId) -> bool {
        self.page_keys
    }
    fn others(&self, _: WindowId) -> Vec<Layer> {
        self.others.clone()
    }
    fn machine(&self, kind: Kind) -> bool {
        self.machine.contains(&kind)
    }
}

/// Window 1 (a Main window) shows `tab`.
fn env(tab: TabId) -> T {
    T {
        shown: [(W1, Some(tab))].into(),
        kinds: BTreeMap::new(),
        page_keys: false,
        others: vec![],
        machine: vec![],
    }
}

fn on(kind: Kind, tab: TabId) -> Layer {
    Layer::new(kind, Some(tab))
}
fn win(kind: Kind) -> Layer {
    Layer::new(kind, None)
}
fn open(layer: Layer) -> Action {
    Action::Open { w: W1, layer }
}
fn close(layer: Layer) -> Action {
    Action::Close { w: W1, layer }
}
fn exited(layer: Layer) -> Action {
    Action::Exited { w: W1, layer }
}
const ESC: Action = Action::Escape { w: W1 };
fn switched(from: TabId, to: TabId) -> Action {
    Action::Switched {
        w: W1,
        from: Some(from),
        to: Some(to),
    }
}
fn nav(tab: TabId) -> Action {
    Action::Navigated { w: W1, tab }
}
fn focus(owner: Option<Layer>) -> Effect {
    Effect::Focus(W1, owner.map_or(KeyOwner::Page, KeyOwner::Layer))
}
use Effect::{Cancel, Hide, Mount, Reveal, Unmount};

/// Runs `actions` in order; the store after them and the last one's effects.
fn run(env: &T, actions: &[Action]) -> (Ui, Vec<Effect>) {
    actions
        .iter()
        .fold((Ui::default(), vec![]), |(ui, _), &a| reduce(ui, env, a))
}

fn sorted(mut v: Vec<Layer>) -> Vec<Layer> {
    v.sort();
    v
}

/// The overlays: the rows under the rules.
fn ruled() -> impl Iterator<Item = &'static super::super::Row> {
    ROWS.iter().filter(|r| r.group != Group::Free)
}

/// Each overlay alone through its life: open, close, the kit's exit, reopening; and the
/// actions each phase ignores.
#[test]
fn each_kind_through_its_phases() {
    let e = env(A);
    for row in ruled() {
        let l = Layer::new(row.kind, Some(A));
        let name = format!("{:?}", row.kind);
        let takes = (row.keys == Keys::Take).then(|| focus(Some(l)));
        let gives = (row.keys != Keys::Never).then(|| focus(None));
        // absent: Close and Exited are ignored; Open mounts it
        for a in [close(l), exited(l)] {
            assert_eq!(
                run(&e, &[a]),
                (Ui::default(), vec![]),
                "{name} {a:?} absent"
            );
        }
        let (ui, fx) = run(&e, &[open(l)]);
        let want: Vec<_> = [Some(Mount(l)), takes].into_iter().flatten().collect();
        assert_eq!(fx, want, "{name} opens");
        assert_eq!(ui.phase(W1, l), Some(Phase::Open), "{name}");
        // open: again is no effect; Exited is a stale exit
        for a in [open(l), exited(l)] {
            assert_eq!(
                reduce(ui.clone(), &e, a),
                (ui.clone(), vec![]),
                "{name} {a:?} open"
            );
        }
        // the owner's close: no Cancel; leaving with a tail, else gone
        let (ui, fx) = reduce(ui, &e, close(l));
        let (phase, unmount) = match row.tail {
            true => (Some(Phase::Leaving), None),
            false => (None, Some(Unmount(l))),
        };
        let want: Vec<_> = [unmount, gives].into_iter().flatten().collect();
        assert_eq!((ui.phase(W1, l), fx), (phase, want), "{name} closes");
        if !row.tail {
            continue;
        }
        // leaving: Close and Escape are ignored; Open mounts fresh content; Exited ends it
        for a in [close(l), ESC] {
            assert_eq!(
                reduce(ui.clone(), &e, a),
                (ui.clone(), vec![]),
                "{name} {a:?} leaving"
            );
        }
        let (again, fx) = reduce(ui.clone(), &e, open(l));
        let want: Vec<_> = [Some(Unmount(l)), Some(Mount(l)), takes]
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(
            (again.phase(W1, l), fx),
            (Some(Phase::Open), want),
            "{name} reopens"
        );
        let (ui, fx) = reduce(ui, &e, exited(l));
        assert_eq!((ui, fx), (Ui::default(), vec![Unmount(l)]), "{name} exits");
    }
}

/// An overlay is refused in the windows its row does not list (the Spaces popup is the Main
/// window's).
#[test]
fn open_refused_outside_window_kinds() {
    use WindowKind::*;
    let mut refused = vec![];
    for row in ruled() {
        for kind in [Main, Little, Popup, Pip] {
            if row.windows.contains(&kind) {
                continue;
            }
            let e = T {
                kinds: [(W1, kind)].into(),
                ..env(A)
            };
            let (ui, fx) = run(&e, &[open(Layer::new(row.kind, Some(A)))]);
            assert_eq!(
                (ui, fx),
                (Ui::default(), vec![]),
                "{:?} in {kind:?}",
                row.kind
            );
            refused.push((row.kind, kind));
        }
    }
    assert_eq!(refused, [(Spaces, Little), (Spaces, Popup), (Spaces, Pip)]);
}

/// The rules as rows: (name, the window's tab, the page has the keys, the opens before,
/// the action, its effects, what is open after).
#[test]
#[rustfmt::skip]
fn store_table() {
    let (dl, ds, menu, ext) = (win(Downloads), win(Spaces), win(Menu), win(Extensions));
    let agent = win(AgentSheet);
    let rows: Vec<(&str, TabId, bool, Vec<Action>, Action, Vec<Effect>, Vec<Layer>)> = vec![
        // refusals: nothing changes
        ("find refused under a sheet", A, false, vec![open(on(BookmarkSheet, A))], open(on(Find, A)), vec![], vec![on(BookmarkSheet, A)]),
        ("a menu refused under a dialog", A, false, vec![open(on(Dialog, A))], open(menu), vec![], vec![on(Dialog, A)]),
        ("an agent's sheet waits for the user's", A, false, vec![open(on(PrintSheet, A))], open(agent), vec![], vec![on(PrintSheet, A)]),
        ("the user's overlays wait for an agent's sheet", A, false, vec![open(agent)], open(ds), vec![], vec![agent]),
        ("boost refused under a dialog", A, false, vec![open(on(Dialog, A))], open(on(Boost, A)), vec![], vec![on(Dialog, A)]),
        ("a dialog is never refused, nor closes an agent's sheet", A, false, vec![open(agent)], open(on(Dialog, A)),
            vec![Mount(on(Dialog, A)), focus(Some(on(Dialog, A)))], vec![agent, on(Dialog, A)]),
        ("the error panel is never refused", A, false, vec![open(agent), open(on(Dialog, A))], open(on(ErrorPanel, A)),
            vec![Mount(on(ErrorPanel, A)), focus(Some(on(Dialog, A)))], vec![agent, on(Dialog, A), on(ErrorPanel, A)]),
        // replacements: the replaced one is cancelled, leaves, and the keys move
        ("Spaces replaces downloads", A, false, vec![open(dl)], open(ds), vec![Cancel(dl), Mount(ds), focus(Some(ds))], vec![ds]),
        ("a menu replaces extensions", A, false, vec![open(ext)], open(menu), vec![Cancel(ext), Mount(menu), focus(Some(menu))], vec![menu]),
        ("find replaces a menu", A, false, vec![open(menu)], open(on(Find, A)), vec![Cancel(menu), Mount(on(Find, A)), focus(None)], vec![on(Find, A)]),
        ("a menu over find keeps it", A, false, vec![open(on(Find, A))], open(menu), vec![Mount(menu), focus(Some(menu))], vec![menu, on(Find, A)]),
        ("a dialog replaces the user's sheet, keeps find", A, false, vec![open(on(Find, A)), open(on(BookmarkSheet, A))], open(on(Dialog, A)),
            vec![Cancel(on(BookmarkSheet, A)), Mount(on(Dialog, A)), focus(Some(on(Dialog, A)))], vec![on(Find, A), on(Dialog, A)]),
        ("a dialog again is its own", A, false, vec![open(on(Dialog, A))], open(on(Dialog, A)), vec![], vec![on(Dialog, A)]),
        ("print replaces add bookmark", A, false, vec![open(on(BookmarkSheet, A))], open(on(PrintSheet, A)),
            vec![Cancel(on(BookmarkSheet, A)), Mount(on(PrintSheet, A)), focus(Some(on(PrintSheet, A)))], vec![on(PrintSheet, A)]),
        ("one notice in the window, across tabs", A, false, vec![open(on(Notice, B)), open(on(PrintSheet, A))], open(on(Notice, A)),
            vec![Cancel(on(Notice, B)), Unmount(on(Notice, B)), Mount(on(Notice, A))], vec![on(Notice, A), on(PrintSheet, A)]),
        ("a permission prompt coexists", A, false, vec![open(on(Permission, A))], open(menu), vec![Mount(menu), focus(Some(menu))], vec![menu, on(Permission, A)]),
        ("a hidden tab's dialog blocks and closes nothing", A, false, vec![open(menu), open(on(Dialog, B))], open(on(PrintSheet, A)),
            vec![Cancel(menu), Mount(on(PrintSheet, A)), focus(Some(on(PrintSheet, A)))], vec![on(PrintSheet, A), on(Dialog, B)]),
        ("the panels coexist with a menu", A, false, vec![open(on(ErrorPanel, A)), open(on(Boost, A))], open(menu),
            vec![Mount(menu), focus(Some(menu))], vec![menu, on(ErrorPanel, A), on(Boost, A)]),
        // Escape: the topmost first, the page's Escape never answers a notice or a prompt
        ("Escape closes the menu over find", A, false, vec![open(on(Notice, A)), open(on(Permission, A)), open(on(Find, A)), open(menu)], ESC,
            vec![Cancel(menu), focus(None)], vec![on(Notice, A), on(Permission, A), on(Find, A)]),
        ("then find", A, false, vec![open(on(Notice, A)), open(on(Permission, A)), open(on(Find, A))], ESC,
            vec![Cancel(on(Find, A)), focus(None)], vec![on(Notice, A), on(Permission, A)]),
        ("then the prompt", A, false, vec![open(on(Notice, A)), open(on(Permission, A))], ESC, vec![Cancel(on(Permission, A))], vec![on(Notice, A)]),
        ("then the notice", A, false, vec![open(on(Notice, A))], ESC, vec![Cancel(on(Notice, A)), Unmount(on(Notice, A))], vec![]),
        ("then the page", A, false, vec![], ESC, vec![], vec![]),
        ("the page's Escape closes a menu", A, true, vec![open(on(Notice, A)), open(on(Permission, A)), open(menu)], ESC,
            vec![Cancel(menu), focus(None)], vec![on(Notice, A), on(Permission, A)]),
        ("the page's Escape never answers a prompt or a notice", A, true, vec![open(on(Notice, A)), open(on(Permission, A))], ESC,
            vec![], vec![on(Notice, A), on(Permission, A)]),
        ("Escape skips a leaving menu", A, false, vec![open(menu), close(menu)], ESC, vec![], vec![]),
        ("Escape skips a hidden tab's find bar", A, false, vec![open(on(Find, B))], ESC, vec![], vec![on(Find, B)]),
        ("Escape never closes the panels", A, false, vec![open(on(ErrorPanel, A)), open(on(Boost, A))], ESC, vec![], vec![on(ErrorPanel, A), on(Boost, A)]),
        ("Escape cancels an agent's sheet over a dialog", A, false, vec![open(agent), open(on(Dialog, A))], ESC,
            vec![Cancel(agent), focus(Some(on(Dialog, A)))], vec![on(Dialog, A)]),
        // a tab switch (window 1 shows B now)
        ("a switch closes sheets, hides what stays, reveals the new tab's", B, false,
            vec![open(on(Notice, A)), open(on(Find, A)), open(on(Dialog, A)), open(on(BookmarkSheet, A)), open(on(Find, B)), open(dl)], switched(A, B),
            vec![Cancel(on(BookmarkSheet, A)), Unmount(on(BookmarkSheet, A)), Hide(on(Notice, A)), Hide(on(Find, A)), Hide(on(Dialog, A)),
                Reveal(on(Find, B)), focus(Some(dl))],
            vec![dl, on(Notice, A), on(Find, A), on(Dialog, A), on(Find, B)]),
        ("a window's menu closes on any switch", B, false, vec![open(menu)], switched(A, B), vec![Cancel(menu), focus(None)], vec![]),
        ("the picker and link hints close with their tab", B, false, vec![open(on(Region, A)), open(on(Hints, A))], switched(A, B),
            vec![Cancel(on(Region, A)), Unmount(on(Region, A)), Cancel(on(Hints, A)), Unmount(on(Hints, A)), focus(None)], vec![]),
        ("back: the tab's own come back", A, false, vec![open(on(Notice, A)), open(on(Find, B))], switched(B, A),
            vec![Reveal(on(Notice, A)), Hide(on(Find, B))], vec![on(Notice, A), on(Find, B)]),
        ("the panels stay with their tab", B, false, vec![open(on(ErrorPanel, A)), open(on(Boost, A))], switched(A, B),
            vec![Hide(on(ErrorPanel, A)), Hide(on(Boost, A))], vec![on(ErrorPanel, A), on(Boost, A)]),
        ("an agent's sheet stays", B, false, vec![open(agent)], switched(A, B), vec![], vec![agent]),
        ("the same tab again is no switch", A, false, vec![open(menu)], switched(A, A), vec![], vec![menu]),
        // a navigation
        ("navigation closes the notice and find, not a dialog", A, false,
            vec![open(on(Notice, A)), open(on(Find, A)), open(on(Dialog, A)), open(on(Find, B))], nav(A),
            vec![Cancel(on(Notice, A)), Unmount(on(Notice, A)), Cancel(on(Find, A)), focus(Some(on(Dialog, A)))], vec![on(Dialog, A), on(Find, B)]),
        ("navigation closes print", A, false, vec![open(on(PrintSheet, A))], nav(A), vec![Cancel(on(PrintSheet, A)), focus(None)], vec![]),
        ("navigation closes add bookmark", A, false, vec![open(on(BookmarkSheet, A))], nav(A), vec![Cancel(on(BookmarkSheet, A)), focus(None)], vec![]),
        ("navigation closes the picker", A, false, vec![open(on(Region, A))], nav(A), vec![Cancel(on(Region, A)), focus(None)], vec![]),
        ("navigation ends link hints", A, false, vec![open(on(Hints, A))], nav(A), vec![Cancel(on(Hints, A)), Unmount(on(Hints, A)), focus(None)], vec![]),
        ("a hidden tab's navigation unmounts without a tail", B, false, vec![open(on(Find, A))], nav(A),
            vec![Cancel(on(Find, A)), Unmount(on(Find, A)), focus(None)], vec![]),
        ("navigation leaves the window's popover and the panels", A, false, vec![open(on(ErrorPanel, A)), open(on(Boost, A)), open(dl)], nav(A),
            vec![], vec![dl, on(ErrorPanel, A), on(Boost, A)]),
        // the tab's view, the window
        ("a view gone unmounts its tab's, open or leaving, nothing cancelled", A, false,
            vec![open(on(Notice, A)), open(on(Find, A)), open(on(PrintSheet, A)), close(on(PrintSheet, A)), open(menu)], Action::ViewGone(A),
            vec![Unmount(on(Notice, A)), Unmount(on(Find, A)), Unmount(on(PrintSheet, A))], vec![menu]),
        ("a window gone cancels its own, keeps the tab's", A, false, vec![open(on(Find, A)), open(menu)], Action::WindowGone(W1),
            vec![Cancel(menu), Unmount(menu)], vec![on(Find, A)]),
    ];
    for (name, tab, page_keys, setup, action, fx, after) in rows {
        let e = T { page_keys, ..env(tab) };
        let (ui, _) = run(&e, &setup);
        let (ui, got) = reduce(ui, &e, action);
        assert_eq!(got, fx, "{name}: effects");
        assert_eq!(sorted(ui.stack(&e, W1).layers), sorted(after), "{name}: open after");
    }
}

/// Until each kind moves into the store, its machine owns it: the rules see it (`others`), a
/// rule cancels it, an admitted open is a Mount the store keeps no entry for.
#[test]
#[rustfmt::skip]
fn a_machine_s_overlay_meets_the_rules() {
    let with = |others: Vec<Layer>, machine: Vec<Kind>| T { others, machine, ..env(A) };
    // name, env, action, effects, what the store holds after
    let rows: Vec<(&str, T, Action, Vec<Effect>, Vec<Layer>)> = vec![
        ("its dialog refuses Spaces", with(vec![on(Dialog, A)], vec![Dialog]), open(win(Spaces)), vec![], vec![]),
        ("downloads replaces its menu", with(vec![win(Menu)], vec![Menu]), open(win(Downloads)),
            vec![Cancel(win(Menu)), Mount(win(Downloads)), focus(Some(win(Downloads)))], vec![win(Downloads)]),
        ("its kind admitted", with(vec![], vec![Find]), open(on(Find, A)), vec![Mount(on(Find, A))], vec![]),
        ("its kind already open", with(vec![on(Find, A)], vec![Find]), open(on(Find, A)), vec![], vec![]),
        ("Escape cancels it", with(vec![on(Find, A)], vec![Find]), ESC, vec![Cancel(on(Find, A))], vec![]),
    ];
    for (name, e, action, fx, held) in rows {
        let (ui, got) = reduce(Ui::default(), &e, action);
        let own = T { others: vec![], ..e };
        assert_eq!((got, ui.stack(&own, W1).layers), (fx, held), "{name}");
    }
}

#[test]
fn tab_keeps_find_across_windows() {
    let (ui, _) = run(&env(A), &[open(on(Find, A))]);
    // window 1 shows B, window 2 shows A: the find bar went with its tab
    let moved = T {
        shown: [(W1, Some(B)), (W2, Some(A))].into(),
        ..env(A)
    };
    let (ui, fx) = reduce(ui, &moved, switched(A, B));
    assert_eq!(fx, [Hide(on(Find, A))]);
    assert_eq!(ui.stack(&moved, W2).layers, [on(Find, A)]);
    assert_eq!(ui.escape_target(&moved, W2), Some(on(Find, A)));
    assert_eq!(ui.escape_target(&moved, W1), None);
}

#[test]
fn window_entries_are_per_window() {
    let two = T {
        shown: [(W1, Some(A)), (W2, Some(B))].into(),
        ..env(A)
    };
    let (ui, _) = run(&two, &[open(win(Spaces))]);
    assert!(ui.stack(&two, W2).layers.is_empty());
    let (ui, fx) = reduce(
        ui,
        &two,
        Action::Open {
            w: W2,
            layer: win(Spaces),
        },
    );
    assert_eq!(
        fx,
        [
            Mount(win(Spaces)),
            Effect::Focus(W2, KeyOwner::Layer(win(Spaces)))
        ]
    );
    let (ui, fx) = reduce(ui, &two, Action::WindowGone(W2));
    assert_eq!(fx, [Cancel(win(Spaces)), Unmount(win(Spaces))]);
    assert_eq!(ui.phase(W1, win(Spaces)), Some(Phase::Open));
}

#[test]
fn error_panel_and_boost_stay_hidden_with_tab() {
    let (err, boost) = (on(ErrorPanel, A), on(Boost, A));
    let (ui, _) = run(
        &env(A),
        &[open(err), open(boost), open(win(Menu)), ESC, ESC],
    );
    assert_eq!(ui.key_owner(&env(A), W1), KeyOwner::Layer(err));
    // B shown: drawn nowhere, the keys the page's; back on A they are
    assert_eq!(ui.ax_roots(&env(B), W1), AxRoots::default());
    assert_eq!(ui.key_owner(&env(B), W1), KeyOwner::Page);
    let (ui, fx) = reduce(ui, &env(A), switched(B, A));
    assert_eq!(fx, [Reveal(err), Reveal(boost)]);
    assert_eq!(ui.ax_roots(&env(A), W1).roots, [boost, err]);
}

/// The selectors: (name, the opens, the window's tab, what each answers).
#[test]
#[rustfmt::skip]
fn selectors() {
    let (menu, sheet, find, err) = (win(Menu), on(BookmarkSheet, A), on(Find, A), on(ErrorPanel, A));
    let rows: Vec<(&str, Vec<Action>, KeyOwner, PointerOwner, AxRoots, bool)> = vec![
        ("nothing", vec![], KeyOwner::Page, PointerOwner::Page, AxRoots::default(), true),
        ("a menu over the error panel has the keys", vec![open(err), open(menu)], KeyOwner::Layer(menu), PointerOwner::Layer(menu),
            AxRoots { modal: None, roots: vec![err, menu] }, true),
        ("a sheet over the error panel", vec![open(err), open(sheet)], KeyOwner::Layer(sheet), PointerOwner::Layer(sheet),
            AxRoots { modal: Some(sheet), roots: vec![err, sheet] }, true),
        ("the find bar asks for its keys; its rect is its own", vec![open(find)], KeyOwner::Page, PointerOwner::Page,
            AxRoots { modal: None, roots: vec![find] }, true),
        ("a leaving menu has nothing", vec![open(menu), close(menu)], KeyOwner::Page, PointerOwner::Page, AxRoots::default(), false),
    ];
    for (name, setup, keys, pointer, ax, idle) in rows {
        let (ui, _) = run(&env(A), &setup);
        let got = (ui.key_owner(&env(A), W1), ui.pointer_owner(&env(A), W1), ui.ax_roots(&env(A), W1), ui.idle());
        assert_eq!(got, (keys, pointer, ax, idle), "{name}");
    }
}

#[test]
fn gone_lists_undrawn_leaving() {
    let (dl, find) = (win(Downloads), on(Find, A));
    let (ui, _) = run(&env(A), &[open(find), open(dl), close(dl), close(find)]);
    // the kit still draws both
    assert!(ui.gone(&env(A), W1, |_| true).is_empty());
    // closed before the kit ever drew it: gone at once
    assert_eq!(ui.gone(&env(A), W1, |l| l != dl), [dl]);
    // the tab not shown anywhere: its bar is gone whatever the kit says
    let (ui, _) = reduce(ui, &env(A), exited(dl));
    assert_eq!(ui.gone(&env(B), W1, |_| true), [find]);
    let (ui, fx) = reduce(ui, &env(B), exited(find));
    assert_eq!((ui.idle(), fx), (true, vec![Unmount(find)]));
}

/// The rules each overlay's row carries, restated: a change to one is a change here too.
#[test]
#[rustfmt::skip]
fn rows_match_the_rules_table() {
    use Group::*;
    const U: &[Group] = &[Exclusive, Modal];
    const H: &[Group] = &[Holds];
    const M: &[Group] = &[Modal, Holds];
    const P: &[Group] = &[Exclusive];
    const N: &[Group] = &[];
    // kind, group, tail, refused under, closes, Escape, keys, one per window
    let table: &[(Kind, Group, bool, &[Group], &[Group], Esc, Keys, bool)] = &[
        (Notice, Coexist, false, N, N, Esc::ShellOnly, Keys::Never, true),
        (Permission, Coexist, true, N, N, Esc::ShellOnly, Keys::Never, false),
        (Find, Coexist, true, M, P, Esc::Any, Keys::OnAsk, false),
        (Dialog, Holds, true, N, U, Esc::Any, Keys::Take, false),
        (BookmarkSheet, Modal, true, H, U, Esc::Any, Keys::Take, false),
        (PrintSheet, Modal, true, H, U, Esc::Any, Keys::Take, false),
        (ExtensionSheet, Modal, true, H, U, Esc::Any, Keys::Take, false),
        (AgentSheet, Holds, true, M, P, Esc::Any, Keys::Take, false),
        (Region, Exclusive, true, H, U, Esc::Any, Keys::Take, false),
        (Hints, Exclusive, false, H, U, Esc::Any, Keys::Take, false),
        (CommandBar, Exclusive, true, H, U, Esc::Any, Keys::Take, false),
        (Downloads, Exclusive, true, H, U, Esc::Any, Keys::Take, false),
        (Extensions, Exclusive, true, H, U, Esc::Any, Keys::Take, false),
        (Spaces, Exclusive, true, H, U, Esc::Any, Keys::Take, false),
        (Menu, Exclusive, true, H, U, Esc::Any, Keys::Take, false),
        (ErrorPanel, Coexist, true, N, N, Esc::Never, Keys::Take, false),
        (Boost, Coexist, true, H, N, Esc::Never, Keys::OnAsk, false),
    ];
    let got: Vec<_> = ruled()
        .map(|r| (r.kind, r.group, r.tail, r.refused_under, r.closes, r.escape, r.keys, r.one_per_window))
        .collect();
    assert_eq!(got, table);
}

/// Seeded random runs over two windows (one a picture-in-picture window), three tabs and every
/// overlay: the invariants hold after each action, and every run drains to idle once the kit
/// reports each exit.
#[test]
fn store_walk() {
    let kinds: Vec<Kind> = ruled().map(|r| r.kind).collect();
    let tabs = [A, B, C];
    for seed in 1..=200 {
        let mut rng = Rng::new(seed);
        let mut pick = |n: usize| ((rng.random() * n as f64) as usize).min(n - 1);
        let mut e = T {
            shown: [(W1, Some(A)), (W2, Some(B))].into(),
            kinds: [(W2, WindowKind::Pip)].into(),
            ..env(A)
        };
        let mut ui = Ui::default();
        let mut mounts: BTreeMap<(Option<WindowId>, Layer), i32> = BTreeMap::new();
        for step in 0..60 {
            let w = [W1, W2][pick(2)];
            let shown = e.shown(w);
            let kind = kinds[pick(kinds.len())];
            // the user's sheets open on the shown tab; the engine's for any tab
            let tab = match kind {
                Dialog | Permission | Notice | ErrorPanel => Some(tabs[pick(3)]),
                _ => shown,
            };
            let layer = Layer::new(kind, tab);
            e.page_keys = pick(2) == 0;
            let action = match pick(10) {
                0..=2 => Action::Open { w, layer },
                3 => Action::Close { w, layer },
                4 => Action::Escape { w },
                5 => Action::Navigated {
                    w,
                    tab: tabs[pick(3)],
                },
                6 => Action::ViewGone(tabs[pick(3)]),
                7 => Action::WindowGone(w),
                8 => Action::Exited { w, layer },
                _ => {
                    // a tab shown in at most one window
                    let to = Some(tabs[pick(3)]).filter(|t| e.showing(*t).is_none());
                    e.shown.insert(w, to);
                    Action::Switched { w, from: shown, to }
                }
            };
            let (next, fx) = reduce(ui.clone(), &e, action);
            assert_eq!(
                reduce(ui.clone(), &e, action),
                (next.clone(), fx.clone()),
                "pure"
            );
            ui = next;
            for f in &fx {
                let (l, d) = match *f {
                    Mount(l) => (l, 1),
                    Unmount(l) => (l, -1),
                    _ => continue,
                };
                let gone_w = match action {
                    Action::WindowGone(w) => w,
                    _ => w,
                };
                *mounts
                    .entry((l.tab.is_none().then_some(gone_w), l))
                    .or_default() += d;
            }
            let ctx = format!("seed {seed} step {step} {action:?}");
            for (&(at, l), &n) in &mounts {
                let present = ui.phase(at.unwrap_or(W1), l).is_some();
                assert_eq!(n, i32::from(present), "{ctx}: {l:?} mounted {n}");
            }
            for w in [W1, W2] {
                let shown = e.shown(w);
                let visible: Vec<Layer> = ui
                    .stack(&e, w)
                    .layers
                    .into_iter()
                    .filter(|l| l.tab.is_none() || l.tab == shown)
                    .collect();
                for scope in [None, Some(())] {
                    let held = visible.iter().filter(|l| {
                        l.tab.map(|_| ()) == scope
                            && matches!(l.kind.row().group, Group::Modal | Group::Holds)
                    });
                    assert!(held.count() <= 1, "{ctx}: two modals in {w:?}: {visible:?}");
                }
                if let KeyOwner::Layer(l) = ui.key_owner(&e, w) {
                    assert!(visible.contains(&l), "{ctx}: keys outside the stack");
                }
                let (after, fx) = reduce(ui.clone(), &e, Action::Escape { w });
                match ui.escape_target(&e, w) {
                    Some(t) => assert_ne!(
                        after.phase(w, t),
                        Some(Phase::Open),
                        "{ctx}: Escape left {t:?}"
                    ),
                    None => assert_eq!(
                        (after, fx),
                        (ui.clone(), vec![]),
                        "{ctx}: Escape with no target"
                    ),
                }
            }
        }
        for w in [W1, W2] {
            for layer in ui.gone(&e, w, |_| false) {
                ui = reduce(ui, &e, Action::Exited { w, layer }).0;
            }
        }
        assert!(ui.idle(), "seed {seed}: never drained: {ui:?}");
    }
}
