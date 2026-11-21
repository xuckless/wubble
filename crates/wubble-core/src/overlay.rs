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
//! - A **notice** (SponsorBlock's question) sits under everything, takes Escape only when nothing
//!   above it is open and the shell has the keys, and one shows at a time.
//! - A **permission prompt** belongs to its tab and coexists under other overlays; Escape
//!   dismisses it (denied, not remembered) only when the shell has the keys.
//! - **Escape** closes the topmost visible overlay that takes it; a second Escape the next. A
//!   notice and a permission prompt never take the page's Escape ([`Event::PageEscape`]): a key
//!   meant for the page (leaving fullscreen, the site's own dialog) must not answer them
//!   (no keyjacking, docs/research/mcp-server.md).
//! - **Tab switch**: menus, sheets, link hints and the screenshot picker close; the find bar, a
//!   page's dialog and a notice stay with their tab, hidden, and are revealed only on it. The
//!   window's command bar and downloads popover stay.
//! - **Navigation** (a new document): the tab's sheets, find bar, link hints, screenshot picker
//!   and notice close; a page's dialog is the engine's to drop.
//!
//! A pointer press outside an overlay is the kit's: the menu's and the command bar's scrims
//! dismiss them, a sheet's scrim holds the page (modal); none of it needs a rule here. A
//! dismissing press is consumed whole (kit `Scrim`), so a press on the popup's own toggle (the
//! "^", the downloads button, a chip, a Select's trigger) closes it and is no click that opens
//! it again.

use crate::tab::TabId;

/// An overlay, in drawing order: a later kind is drawn over (and is closed by Escape before) an
/// earlier one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A lasting notice of the page (SponsorBlock's "Skip sponsor segments?").
    Notice,
    /// A tab's permission prompt (camera, mic, notifications, external link).
    Permission,
    /// The find bar.
    Find,
    /// A page's JS dialog or sign-in sheet (the engine waits for its answer).
    Dialog,
    /// Add bookmark (and the bookmarks page's rename, move and new-folder sheets).
    BookmarkSheet,
    PrintSheet,
    /// The extension install prompt: what an extension asks for, Allow or Cancel.
    ExtensionSheet,
    /// An agent session's approval sheet (`agent`): the window's, modal, one at a time.
    AgentSheet,
    /// The screenshot region picker.
    Region,
    /// Keyboard link hints.
    Hints,
    /// The command bar over the page.
    CommandBar,
    /// The sidebar's downloads popover.
    Downloads,
    /// The sidebar footer's extensions popover, and the action popup that replaces its list.
    Extensions,
    /// The sidebar footer's Spaces popup (the "^": every Space, then New Space).
    Spaces,
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
            Kind::ExtensionSheet
            | Kind::AgentSheet
            | Kind::CommandBar
            | Kind::Downloads
            | Kind::Extensions
            | Kind::Spaces
            | Kind::Menu => Scope::Window,
            _ => Scope::Tab,
        }
    }

    /// One at a time.
    pub fn exclusive(self) -> bool {
        self > Kind::Find
    }

    /// Holds the page under a scrim: the find bar waits for it.
    fn modal(self) -> bool {
        matches!(
            self,
            Kind::Dialog
                | Kind::BookmarkSheet
                | Kind::PrintSheet
                | Kind::ExtensionSheet
                | Kind::AgentSheet
        )
    }

    /// Answers to someone outside the window (the page's engine, an agent's client): no other
    /// overlay closes it, and while one shows the user's overlays are refused.
    fn holds(self) -> bool {
        matches!(self, Kind::Dialog | Kind::AgentSheet)
    }

    /// Whether Escape closes it; `from_page`: the page has the keys.
    fn takes_escape(self, from_page: bool) -> bool {
        !(from_page && matches!(self, Kind::Notice | Kind::Permission))
    }

    /// Closes when its tab stops being the visible one (a window menu: on any switch); the rest
    /// stay with their tab, hidden.
    fn closes_on_leave(self) -> bool {
        matches!(
            self,
            Kind::Menu
                | Kind::Spaces
                | Kind::Extensions
                | Kind::ExtensionSheet
                | Kind::BookmarkSheet
                | Kind::PrintSheet
                | Kind::Region
                | Kind::Hints
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
    /// Escape while the shell has the keys.
    Escape,
    /// Escape while the page has the keys.
    PageEscape,
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
        Event::Escape | Event::PageEscape => match stack.top() {
            Some(top) if top.kind.takes_escape(event == Event::PageEscape) => {
                close(stack, vec![top], vec![])
            }
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
        Kind::Notice | Kind::Permission | Kind::Dialog => false,
        Kind::Find => visible.iter().any(|l| l.kind.modal()),
        // an agent's sheet never replaces the user's sheet, which it waits for
        Kind::AgentSheet => visible.iter().any(|l| l.kind.holds() || l.kind.modal()),
        _ => visible.iter().any(|l| l.kind.holds()),
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
        Kind::Permission => vec![],
        Kind::Find => visible
            .into_iter()
            .filter(|l| l.kind.exclusive() && !l.kind.modal())
            .collect(),
        // a held overlay (a page's dialog, an agent's sheet) is never closed by another
        _ => visible
            .into_iter()
            .filter(|l| l.kind.exclusive() && !l.kind.holds())
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
    fn the_spaces_popup_is_one_window_overlay_among_the_others() {
        assert_eq!(Layer::new(Spaces, Some(A)).tab, None);
        // it replaces the downloads popover, and a menu replaces it
        let (layers, fx) = step(&[win(Downloads)], Event::Open(win(Spaces)));
        assert_eq!(layers, [win(Spaces)]);
        assert_eq!(
            fx,
            [Effect::Close(win(Downloads)), Effect::Open(win(Spaces))]
        );
        let (layers, _) = step(&[win(Spaces)], Event::Open(win(Menu)));
        assert_eq!(layers, [win(Menu)]);
        // refused over a page's dialog; Escape closes it first; a tab switch closes it
        let (_, fx) = step(&[on(Dialog, A)], Event::Open(win(Spaces)));
        assert!(fx.is_empty());
        let (layers, fx) = step(&[on(Find, A), win(Spaces)], Event::Escape);
        assert_eq!(layers, [on(Find, A)]);
        assert_eq!(fx, [Effect::Close(win(Spaces))]);
        let (layers, _) = step(&[win(Spaces)], Event::Switched(Some(B)));
        assert!(layers.is_empty());
    }

    #[test]
    fn the_extensions_popover_is_a_window_overlay_that_follows_the_rules() {
        assert_eq!(Layer::new(Extensions, Some(A)).tab, None);
        // it replaces the downloads popover, and the Spaces popup replaces it
        let (layers, _) = step(&[win(Downloads)], Event::Open(win(Extensions)));
        assert_eq!(layers, [win(Extensions)]);
        let (layers, _) = step(&[win(Extensions)], Event::Open(win(Spaces)));
        assert_eq!(layers, [win(Spaces)]);
        // refused over a page's dialog; Escape and a tab switch close it
        let (_, fx) = step(&[on(Dialog, A)], Event::Open(win(Extensions)));
        assert!(fx.is_empty());
        let (layers, fx) = step(&[win(Extensions)], Event::Escape);
        assert!(layers.is_empty());
        assert_eq!(fx, [Effect::Close(win(Extensions))]);
        let (layers, _) = step(&[win(Extensions)], Event::Switched(Some(B)));
        assert!(layers.is_empty());
    }

    #[test]
    fn the_install_prompt_is_a_modal_sheet_that_holds_the_find_bar_off() {
        assert_eq!(Layer::new(ExtensionSheet, Some(A)).tab, None);
        let (_, fx) = step(&[win(ExtensionSheet)], Event::Open(on(Find, A)));
        assert!(fx.is_empty());
        // it replaces a popover; Escape cancels it
        let (layers, _) = step(&[win(Extensions)], Event::Open(win(ExtensionSheet)));
        assert_eq!(layers, [win(ExtensionSheet)]);
        let (layers, fx) = step(&[win(ExtensionSheet)], Event::Escape);
        assert!(layers.is_empty());
        assert_eq!(fx, [Effect::Close(win(ExtensionSheet))]);
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
            win(ExtensionSheet),
            on(Region, A),
            on(Hints, A),
            win(CommandBar),
            win(Downloads),
            win(Extensions),
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
            ExtensionSheet,
            Region,
            Hints,
            CommandBar,
            Downloads,
            Extensions,
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
    fn an_agent_sheet_is_a_window_modal_that_nothing_replaces() {
        let sheet = win(AgentSheet);
        assert_eq!(Layer::new(AgentSheet, Some(A)).tab, None);
        // it replaces a menu or the command bar
        for kind in [Menu, CommandBar, Downloads] {
            let (layers, fx) = step(&[win(kind)], Event::Open(sheet));
            assert_eq!(layers, [sheet]);
            assert_eq!(fx, [Effect::Close(win(kind)), Effect::Open(sheet)]);
        }
        // it waits for the user's sheet and for a page's dialog
        for other in [on(BookmarkSheet, A), on(PrintSheet, A), on(Dialog, A)] {
            let (layers, fx) = step(&[other], Event::Open(sheet));
            assert_eq!(layers, [other]);
            assert!(fx.is_empty());
        }
        // while it shows the user's overlays are refused, and a page's dialog does not close it
        for kind in [Menu, CommandBar, Downloads, Spaces] {
            let (layers, fx) = step(&[sheet], Event::Open(win(kind)));
            assert_eq!(layers, [sheet]);
            assert!(fx.is_empty(), "{kind:?}");
        }
        let (layers, fx) = step(&[sheet], Event::Open(on(Find, A)));
        assert_eq!(layers, [sheet]);
        assert!(fx.is_empty());
        let (layers, fx) = step(&[sheet], Event::Open(on(Dialog, A)));
        assert_eq!(layers, [sheet, on(Dialog, A)]);
        assert_eq!(fx, [Effect::Open(on(Dialog, A))]);
        // Escape closes it first (the topmost); a tab switch or navigation leaves it
        let (layers, fx) = step(&[sheet, on(Dialog, A)], Event::Escape);
        assert_eq!(layers, [on(Dialog, A)]);
        assert_eq!(fx, [Effect::Close(sheet)]);
        let (layers, fx) = step(&[sheet], Event::Switched(Some(B)));
        assert_eq!(layers, [sheet]);
        assert!(fx.is_empty());
        let (layers, fx) = step(&[sheet], Event::Navigated(A));
        assert_eq!(layers, [sheet]);
        assert!(fx.is_empty());
    }

    #[test]
    fn the_install_prompt_and_an_agent_sheet_show_one_at_a_time() {
        let agent = win(AgentSheet);
        let prompt = win(ExtensionSheet);
        // an agent's sheet waits for the install prompt
        let (layers, fx) = step(&[prompt], Event::Open(agent));
        assert_eq!(layers, [prompt]);
        assert!(fx.is_empty());
        // while an agent's sheet shows, the install prompt and the popover are refused
        for kind in [ExtensionSheet, Extensions] {
            let (layers, fx) = step(&[agent], Event::Open(win(kind)));
            assert_eq!(layers, [agent], "{kind:?}");
            assert!(fx.is_empty(), "{kind:?}");
        }
        // Escape closes each
        for sheet in [prompt, agent] {
            let (layers, fx) = step(&[sheet], Event::Escape);
            assert!(layers.is_empty());
            assert_eq!(fx, [Effect::Close(sheet)]);
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
        let layers = [on(Notice, A), on(Permission, A), on(Find, A), win(Menu)];
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(win(Menu))]);
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Find, A))]);
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Permission, A))]);
        // Notice is lowest: Escape dismisses it once nothing sits above it.
        let (layers, fx) = step(&layers, Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Notice, A))]);
        assert!(layers.is_empty());
        // With nothing left, Escape goes on to the page.
        let (layers, fx) = step(&layers, Event::Escape);
        assert!(fx.is_empty());
        assert!(layers.is_empty());
    }

    /// The page's Escape closes a shell overlay above, but never answers a notice or a
    /// permission prompt: it goes on to the page (fullscreen, the site's dialog).
    #[test]
    fn the_pages_escape_never_answers_a_notice_or_a_prompt() {
        let layers = [on(Notice, A), on(Permission, A), win(Menu)];
        let (layers, fx) = step(&layers, Event::PageEscape);
        assert_eq!(fx, [Effect::Close(win(Menu))]);
        let (layers, fx) = step(&layers, Event::PageEscape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Notice, A), on(Permission, A)]);
        let (layers, fx) = step(&[on(Notice, A)], Event::PageEscape);
        assert!(fx.is_empty());
        assert_eq!(layers, [on(Notice, A)]);
        // with the shell's keys, each closes
        let (_, fx) = step(&[on(Permission, A)], Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Permission, A))]);
        let (_, fx) = step(&[on(Notice, A)], Event::Escape);
        assert_eq!(fx, [Effect::Close(on(Notice, A))]);
    }

    #[test]
    fn permission_is_tab_scoped_and_coexists_with_other_overlays() {
        assert_eq!(Layer::new(Permission, Some(A)).tab, Some(A));
        assert!(!Permission.exclusive());
        // Opening a menu or find bar leaves a permission prompt in place.
        let (layers, fx) = step(&[on(Permission, A)], Event::Open(win(Menu)));
        assert_eq!(layers, [on(Permission, A), win(Menu)]);
        assert_eq!(fx, [Effect::Open(win(Menu))]);
        // Switching tabs hides the permission prompt rather than closing it.
        let (next, fx) = transition(stack(&[on(Permission, A)]), Event::Switched(Some(B)));
        assert_eq!(next.layers, [on(Permission, A)]);
        assert_eq!(fx, [Effect::Hide(on(Permission, A))]);
        // Escape on tab B does not touch tab A's permission prompt.
        let (after, fx) = transition(next, Event::Escape);
        assert_eq!(after.layers, [on(Permission, A)]);
        assert!(fx.is_empty());
    }

    #[test]
    fn escape_closes_each_kind() {
        for kind in [
            Notice,
            Permission,
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
