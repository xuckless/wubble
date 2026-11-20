//! Which shell overlay is on top, and what opening one, Escape, a tab switch or a navigation does
//! to the others: the one owner of the overlays' order and scope.
//!
//! Each overlay keeps its own machine for what it shows (the menu's request, the print sheet's
//! settings, the find bar's query); this module owns the rules between them. The host reads the
//! [`Stack`] from those machines (what is open, for which tab) and runs [`transition`]; the
//! effects close, hide or reveal overlays through their own machines. So whether an overlay is
//! open has one owner (its machine), and which may be open together has one owner (here).
//!
//! The rules (Chromium's: one tab-modal sheet or bubble at a time, the find bar belongs to its
//! tab, Escape closes the innermost):
//! - **Exclusive** overlays (every kind above [`Kind::Find`]) show one at a time: opening one
//!   closes the other visible one. A page's dialog is never closed by another: while one shows,
//!   the user's overlays are refused (as Chromium's tab-modal dialog), and the dialog itself
//!   closes the user's overlays when it arrives.
//! - The **find bar** coexists under any exclusive overlay but a modal sheet (dialog, Add
//!   bookmark, Print): it is refused under one, and opening it closes a menu, the command bar,
//!   link hints or the screenshot picker.
//! - A **notice** (SponsorBlock's question) sits under everything, never takes Escape (a key meant
//!   for the page must not answer it, as the permission prompt), and one shows at a time.
//! - **Escape** closes the topmost visible overlay that takes it; a second Escape the next.
//! - **Tab switch**: menus, sheets, link hints and the screenshot picker close; the find bar, a
//!   page's dialog and a notice stay with their tab, hidden, and are revealed only on it. The
//!   window's command bar and downloads popover stay.
//! - **Navigation** (a new document): the tab's sheets, find bar, link hints, screenshot picker
//!   and notice close; a page's dialog is the engine's to drop.
//!
//! A pointer press outside an overlay is the kit's: the menu's and the command bar's scrims
//! dismiss them, a sheet's scrim holds the page (modal); none of it needs a rule here.

use crate::tab::TabId;

/// An overlay, in drawing order: a later kind is drawn over (and is closed by Escape before) an
/// earlier one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A lasting notice of the page (SponsorBlock's "Skip sponsor segments?").
    Notice,
    /// The find bar.
    Find,
    /// A page's JS dialog or sign-in sheet (the engine waits for its answer).
    Dialog,
    /// Add bookmark (and the bookmarks page's rename, move and new-folder sheets).
    BookmarkSheet,
    PrintSheet,
    /// The screenshot region picker.
    Region,
    /// Keyboard link hints.
    Hints,
    /// The command bar over the page.
    CommandBar,
    /// The sidebar's downloads popover.
    Downloads,
    /// A context menu: a tab's, a folder's or the page's.
    Menu,
}

/// Whom an overlay belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The window: shown over whichever tab is visible.
    Window,
    /// One tab: shown only while it is visible.
    Tab,
}

impl Kind {
    pub fn scope(self) -> Scope {
        match self {
            Kind::CommandBar | Kind::Downloads | Kind::Menu => Scope::Window,
            _ => Scope::Tab,
        }
    }

    /// One at a time.
    pub fn exclusive(self) -> bool {
        self > Kind::Find
    }

    /// Holds the page under a scrim: the find bar waits for it.
    fn modal(self) -> bool {
        matches!(self, Kind::Dialog | Kind::BookmarkSheet | Kind::PrintSheet)
    }

    fn takes_escape(self) -> bool {
        self != Kind::Notice
    }

    /// Closes when its tab stops being the visible one (a window menu: on any switch); the rest
    /// stay with their tab, hidden.
    fn closes_on_leave(self) -> bool {
        matches!(
            self,
            Kind::Menu | Kind::BookmarkSheet | Kind::PrintSheet | Kind::Region | Kind::Hints
        )
    }

    fn closes_on_navigation(self) -> bool {
        matches!(
            self,
            Kind::Notice
                | Kind::Find
                | Kind::BookmarkSheet
                | Kind::PrintSheet
                | Kind::Region
                | Kind::Hints
        )
    }
}

/// One open overlay: its kind and, when tab-scoped, its tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layer {
    pub kind: Kind,
    pub tab: Option<TabId>,
}

impl Layer {
    /// `kind` for `tab`; a window overlay keeps no tab.
    pub fn new(kind: Kind, tab: Option<TabId>) -> Self {
        let tab = match kind.scope() {
            Scope::Window => None,
            Scope::Tab => tab,
        };
        Layer { kind, tab }
    }
}

/// What is open, and which tab is visible.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    pub shown: Option<TabId>,
    pub layers: Vec<Layer>,
}

impl Stack {
    fn visible(&self, layer: &Layer) -> bool {
        layer.tab.is_none() || layer.tab == self.shown
    }

    /// The topmost visible overlay.
    pub fn top(&self) -> Option<Layer> {
        self.layers
            .iter()
            .filter(|l| self.visible(l))
            .max_by_key(|l| l.kind)
            .copied()
    }

    pub fn is_open(&self, kind: Kind) -> bool {
        self.layers.iter().any(|l| l.kind == kind)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user, a key or the page asks for an overlay.
    Open(Layer),
    Escape,
    /// This tab is now the visible one (none: the window shows no tab).
    Switched(Option<TabId>),
    /// The tab's main frame committed a new document.
    Navigated(TabId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Close it through its machine (a sheet or dialog as cancelled, a menu as dismissed).
    Close(Layer),
    /// It stays with its tab, which is no longer visible.
    Hide(Layer),
    /// Its tab is visible again.
    Reveal(Layer),
    /// The asked overlay may open (no effect at all: refused).
    Open(Layer),
}

/// One step of the overlays.
pub fn transition(stack: Stack, event: Event) -> (Stack, Vec<Effect>) {
    match event {
        Event::Open(layer) => open(stack, layer),
        Event::Escape => match stack.top() {
            Some(top) if top.kind.takes_escape() => close(stack, vec![top], vec![]),
            _ => (stack, vec![]),
        },
        Event::Switched(to) => switched(stack, to),
        Event::Navigated(tab) => {
            let gone = stack
                .layers
                .iter()
                .filter(|l| l.tab == Some(tab) && l.kind.closes_on_navigation())
                .copied()
                .collect();
            close(stack, gone, vec![])
        }
    }
}

fn open(stack: Stack, layer: Layer) -> (Stack, Vec<Effect>) {
    let layer = Layer::new(layer.kind, layer.tab);
    // already open: its machine decides (Mod+F again selects the query)
    if stack.layers.contains(&layer) {
        return (stack, vec![Effect::Open(layer)]);
    }
    // where it shows: a hidden tab's overlay meets nothing that is visible
    let shows = layer.tab.is_none() || layer.tab == stack.shown;
    let visible: Vec<Layer> = stack
        .layers
        .iter()
        .filter(|l| shows && stack.visible(l))
        .copied()
        .collect();
    let refused = match layer.kind {
        Kind::Notice | Kind::Dialog => false,
        Kind::Find => visible.iter().any(|l| l.kind.modal()),
        _ => visible.iter().any(|l| l.kind == Kind::Dialog),
    };
    if refused {
        return (stack, vec![]);
    }
    let gone: Vec<Layer> = match layer.kind {
        // one notice in the window
        Kind::Notice => stack
            .layers
            .iter()
            .filter(|l| l.kind == Kind::Notice)
            .copied()
            .collect(),
        Kind::Find => visible
            .into_iter()
            .filter(|l| l.kind.exclusive() && !l.kind.modal())
            .collect(),
        // the page's dialog never closes another page's dialog
        _ => visible
            .into_iter()
            .filter(|l| l.kind.exclusive() && l.kind != Kind::Dialog)
            .collect(),
    };
    let (mut next, effects) = close(stack, gone, vec![]);
    next.layers.push(layer);
    let mut effects = effects;
    effects.push(Effect::Open(layer));
    (next, effects)
}

fn switched(stack: Stack, to: Option<TabId>) -> (Stack, Vec<Effect>) {
    let from = stack.shown;
    if from == to {
        return (stack, vec![]);
    }
    let mut gone = vec![];
    let mut effects = vec![];
    for l in &stack.layers {
        let mine = l.tab.is_some() && l.tab == from;
        if l.kind.closes_on_leave() && (mine || l.tab.is_none()) {
            gone.push(*l);
        } else if mine {
            effects.push(Effect::Hide(*l));
        } else if l.tab.is_some() && l.tab == to {
            effects.push(Effect::Reveal(*l));
        }
    }
    close(Stack { shown: to, ..stack }, gone, effects)
}

/// Removes `gone` from the stack, each a [`Effect::Close`], before `then`.
fn close(stack: Stack, gone: Vec<Layer>, then: Vec<Effect>) -> (Stack, Vec<Effect>) {
    let layers = stack
        .layers
        .into_iter()
        .filter(|l| !gone.contains(l))
        .collect();
    let mut effects: Vec<Effect> = gone.into_iter().map(Effect::Close).collect();
    effects.extend(then);
    (
        Stack {
            shown: stack.shown,
            layers,
        },
        effects,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use Kind::*;

    const A: TabId = TabId(1);
    const B: TabId = TabId(2);

    fn on(kind: Kind, tab: TabId) -> Layer {
        Layer::new(kind, Some(tab))
    }

    fn win(kind: Kind) -> Layer {
        Layer::new(kind, None)
    }

    fn stack(layers: &[Layer]) -> Stack {
        Stack {
            shown: Some(A),
            layers: layers.to_vec(),
        }
    }

    fn step(layers: &[Layer], event: Event) -> (Vec<Layer>, Vec<Effect>) {
        let (next, effects) = transition(stack(layers), event);
        (next.layers, effects)
    }

    #[test]
    fn a_window_overlay_keeps_no_tab() {
        assert_eq!(Layer::new(Menu, Some(A)).tab, None);
        assert_eq!(Layer::new(Find, Some(A)).tab, Some(A));
    }

    #[test]
    fn print_over_add_bookmark_replaces_it() {
        let (layers, fx) = step(&[on(BookmarkSheet, A)], Event::Open(on(PrintSheet, A)));
        assert_eq!(layers, [on(PrintSheet, A)]);
        assert_eq!(
            fx,
            [
                Effect::Close(on(BookmarkSheet, A)),
                Effect::Open(on(PrintSheet, A))
            ]
        );
    }

    #[test]
    fn every_exclusive_overlay_replaces_the_shown_one() {
        let exclusive = [
            on(BookmarkSheet, A),
            on(PrintSheet, A),
            on(Region, A),
            on(Hints, A),
            win(CommandBar),
            win(Downloads),
            win(Menu),
        ];
        for old in exclusive {
            for new in exclusive.iter().filter(|n| **n != old) {
                let (layers, fx) = step(&[old], Event::Open(*new));
                assert_eq!(layers, [*new], "{new:?} over {old:?}");
                assert_eq!(fx, [Effect::Close(old), Effect::Open(*new)]);
            }
        }
    }

    #[test]
    fn opening_what_is_open_closes_nothing() {
        let (layers, fx) = step(&[on(Find, A)], Event::Open(on(Find, A)));
        assert_eq!(layers, [on(Find, A)]);
        assert_eq!(fx, [Effect::Open(on(Find, A))]);
    }

    #[test]
    fn a_page_dialog_refuses_the_user_s_overlays_and_closes_them_on_arrival() {
        for kind in [
            BookmarkSheet,
            PrintSheet,
            Region,
            Hints,
            CommandBar,
            Downloads,
            Menu,
            Find,
        ] {
            let layer = Layer::new(kind, Some(A));
            let (layers, fx) = step(&[on(Dialog, A)], Event::Open(layer));
            assert_eq!(layers, [on(Dialog, A)], "{kind:?}");
            assert!(fx.is_empty(), "{kind:?} opened over a dialog");
        }
        // it closes the user's sheet and menu, keeps the find bar under it
        let (layers, fx) = step(
            &[on(Find, A), on(BookmarkSheet, A)],
            Event::Open(on(Dialog, A)),
        );
        assert_eq!(layers, [on(Find, A), on(Dialog, A)]);
        assert_eq!(
            fx,
            [
                Effect::Close(on(BookmarkSheet, A)),
                Effect::Open(on(Dialog, A))
            ]
        );
    }

    #[test]
    fn a_hidden_tab_s_dialog_blocks_nothing_and_closes_nothing() {
        let (layers, fx) = step(&[on(Dialog, B)], Event::Open(on(PrintSheet, A)));
        assert_eq!(layers, [on(Dialog, B), on(PrintSheet, A)]);
        assert_eq!(fx, [Effect::Open(on(PrintSheet, A))]);
        let (layers, fx) = step(&[win(Menu)], Event::Open(on(Dialog, B)));
        assert_eq!(layers, [win(Menu), on(Dialog, B)]);
        assert_eq!(fx, [Effect::Open(on(Dialog, B))]);
    }

    #[test]
    fn the_find_bar_stays_under_a_menu_and_waits_for_a_sheet() {
        // a menu over the find bar: both
        let (layers, fx) = step(&[on(Find, A)], Event::Open(win(Menu)));
        assert_eq!(layers, [on(Find, A), win(Menu)]);
        assert_eq!(fx, [Effect::Open(win(Menu))]);
        // the find bar opened under a menu or the command bar closes them
        for kind in [Menu, CommandBar, Downloads] {
            let (layers, fx) = step(&[win(kind)], Event::Open(on(Find, A)));
            assert_eq!(layers, [on(Find, A)]);
            assert_eq!(fx, [Effect::Close(win(kind)), Effect::Open(on(Find, A))]);
        }
        // refused under a modal sheet
        for kind in [BookmarkSheet, PrintSheet] {
            let (layers, fx) = step(&[on(kind, A)], Event::Open(on(Find, A)));
            assert_eq!(layers, [on(kind, A)]);
            assert!(fx.is_empty());
        }
    }

    #[test]
    fn one_notice_in_the_window_under_everything() {
        let (layers, fx) = step(
            &[on(Notice, B), on(PrintSheet, A)],
            Event::Open(on(Notice, A)),
        );
        assert_eq!(layers, [on(PrintSheet, A), on(Notice, A)]);
        assert_eq!(
            fx,
            [Effect::Close(on(Notice, B)), Effect::Open(on(Notice, A))]
        );
        // it opens over a dialog too, and is never the top
        let s = stack(&[on(Dialog, A), on(Notice, A)]);
        assert_eq!(s.top(), Some(on(Dialog, A)));
    }

    #[test]
    fn escape_closes_the_topmost_then_the_next() {
        let layers = [on(Notice, A), on(Find, A), win(Menu)];
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(win(Menu))]);
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Find, A))]);
        // a notice never takes Escape: the key goes on to the page
        let (layers, fx) = step(&layers, Event::Escape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Notice, A)]);
    }

    #[test]
    fn escape_closes_each_kind() {
        for kind in [
            Find,
            Dialog,
            BookmarkSheet,
            PrintSheet,
            Region,
            Hints,
            CommandBar,
            Downloads,
            Menu,
        ] {
            let layer = Layer::new(kind, Some(A));
            let (layers, fx) = step(&[layer], Event::Escape);
            assert!(layers.is_empty(), "{kind:?}");
            assert_eq!(fx, [Effect::Close(layer)]);
        }
    }

    #[test]
    fn escape_skips_a_hidden_tab_s_overlays() {
        let (layers, fx) = step(&[on(Find, B)], Event::Escape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Find, B)]);
    }

    #[test]
    fn a_tab_switch_closes_sheets_and_menus_and_hides_what_stays() {
        let layers = [
            on(Notice, A),
            on(Find, A),
            on(Dialog, A),
            on(BookmarkSheet, A),
            win(CommandBar),
            win(Menu),
            on(Find, B),
        ];
        let (next, fx) = transition(stack(&layers), Event::Switched(Some(B)));
        assert_eq!(next.shown, Some(B));
        assert_eq!(
            next.layers,
            [
                on(Notice, A),
                on(Find, A),
                on(Dialog, A),
                win(CommandBar),
                on(Find, B)
            ]
        );
        assert_eq!(
            fx,
            [
                Effect::Close(on(BookmarkSheet, A)),
                Effect::Close(win(Menu)),
                Effect::Hide(on(Notice, A)),
                Effect::Hide(on(Find, A)),
                Effect::Hide(on(Dialog, A)),
                Effect::Reveal(on(Find, B)),
            ]
        );
        // back: A's find bar, dialog and notice come back, B's find bar hides
        let (back, fx) = transition(next, Event::Switched(Some(A)));
        assert_eq!(back.shown, Some(A));
        assert_eq!(
            fx,
            [
                Effect::Reveal(on(Notice, A)),
                Effect::Reveal(on(Find, A)),
                Effect::Reveal(on(Dialog, A)),
                Effect::Hide(on(Find, B)),
            ]
        );
        // the other tab's overlays never show on this one
        assert_eq!(back.top(), Some(win(CommandBar)));
    }

    #[test]
    fn print_hints_and_region_close_with_their_tab() {
        for kind in [PrintSheet, Hints, Region] {
            let (next, fx) = transition(stack(&[on(kind, A)]), Event::Switched(Some(B)));
            assert!(next.layers.is_empty(), "{kind:?}");
            assert_eq!(fx, [Effect::Close(on(kind, A))]);
        }
    }

    #[test]
    fn the_same_tab_again_is_no_switch() {
        let (next, fx) = transition(stack(&[win(Menu)]), Event::Switched(Some(A)));
        assert_eq!(next.layers, [win(Menu)]);
        assert!(fx.is_empty());
    }

    #[test]
    fn navigation_closes_the_tab_s_overlays() {
        let layers = [
            on(Notice, A),
            on(Find, A),
            on(Dialog, A),
            on(PrintSheet, A),
            win(Downloads),
            on(Find, B),
        ];
        let (layers, fx) = step(&layers, Event::Navigated(A));
        assert_eq!(layers, [on(Dialog, A), win(Downloads), on(Find, B)]);
        assert_eq!(
            fx,
            [
                Effect::Close(on(Notice, A)),
                Effect::Close(on(Find, A)),
                Effect::Close(on(PrintSheet, A)),
            ]
        );
        for kind in [BookmarkSheet, Hints, Region] {
            let (layers, fx) = step(&[on(kind, A)], Event::Navigated(A));
            assert!(layers.is_empty());
            assert_eq!(fx, [Effect::Close(on(kind, A))]);
        }
    }

    #[test]
    fn the_top_is_the_latest_drawn_visible_kind() {
        let s = stack(&[win(Menu), on(Find, A), on(PrintSheet, B)]);
        assert_eq!(s.top(), Some(win(Menu)));
        assert!(s.is_open(PrintSheet));
        assert_eq!(Stack::default().top(), None);
    }
}
