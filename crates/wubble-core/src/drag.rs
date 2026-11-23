//! Every drag in a window, one at a time: a tab row dragged out of the list
//! ([`crate::tabdrag`]) or a page's own drag ([`crate::pagedrag`]). One state, one pure
//! transition, effects out; the host feeds it the pointer and runs what it returns.
//!
//! A page drag that starts while a tab is dragged is refused (it ends at once with nothing
//! dropped, as an OS refuses a second drag), and tab-row input during a page drag is not a
//! tab drag: the pointer belongs to the drag that has it.

use crate::overlay::Kind;
use crate::pagedrag::{self, DragOps, Over, PageDrag, Payload};
use crate::tabdrag::{self, DropTarget};
use crate::window::WindowId;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Drag<P> {
    #[default]
    Idle,
    /// A tab row pressed or dragged (never `tabdrag::Drag::Idle`).
    Tab(tabdrag::Drag),
    Page(PageDrag<P>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input<P> {
    Tab(tabdrag::Input),
    Page(pagedrag::Input<P>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect<P> {
    Tab(tabdrag::Effect),
    Page(pagedrag::Effect<P>),
}

impl<P> Drag<P> {
    /// The tab drag's state (`Idle` when none runs).
    pub fn tab(&self) -> tabdrag::Drag {
        match self {
            Drag::Tab(d) => *d,
            _ => tabdrag::Drag::Idle,
        }
    }

    pub fn page(&self) -> Option<&PageDrag<P>> {
        match self {
            Drag::Page(d) => Some(d),
            _ => None,
        }
    }

    /// The page drag running inside the window.
    pub fn page_dragging(&self) -> Option<&pagedrag::Dragging<P>> {
        match self {
            Drag::Page(PageDrag::Dragging(d)) => Some(d),
            _ => None,
        }
    }
}

/// Every surface a drag can show: the host closes the ones [`surfaces`] does not name.
pub const SURFACES: [Kind; 4] = [
    Kind::TabGhost,
    Kind::DropZone,
    Kind::PageGhost,
    Kind::DropMark,
];

/// The surfaces the drag shows now, a total function of its state (the store holds which are
/// open, the machines their content): a tab held and not yet dragged is a click and shows
/// nothing; a tab drag shows its ghost (it draws nothing while the pointer is over the list)
/// and, over a content edge, the zone it would split into; a page drag shows its ghost and the
/// mark (drawn only over a shell target), until its button is released; one the OS has taken
/// shows nothing of ours.
pub fn surfaces<P>(d: &Drag<P>) -> &'static [Kind] {
    match d {
        Drag::Idle
        | Drag::Tab(tabdrag::Drag::Idle | tabdrag::Drag::Pressed { .. })
        | Drag::Page(PageDrag::Outside { .. }) => &[],
        Drag::Tab(tabdrag::Drag::Dragging {
            over: DropTarget::SplitEdge(_),
            ..
        }) => &[Kind::TabGhost, Kind::DropZone],
        Drag::Tab(tabdrag::Drag::Dragging { .. }) => &[Kind::TabGhost],
        // released over a page: the drop waits for the page's answer, the button is up
        Drag::Page(PageDrag::Dragging(d)) if d.dropping => &[],
        Drag::Page(PageDrag::Dragging(_)) => &[Kind::PageGhost, Kind::DropMark],
    }
}

/// A left release seen by a window other than the drag's own (`same_window` false) ends the
/// drag as cancelled: a release always ends a drag, whichever window sees it. The drag's own
/// window routes its release by the pointer (`None`).
pub fn release_input<P>(d: &Drag<P>, same_window: bool) -> Option<Input<P>> {
    match d {
        _ if same_window => None,
        Drag::Idle | Drag::Page(PageDrag::Outside { .. }) => None,
        Drag::Tab(_) => Some(Input::Tab(tabdrag::Input::Cancel)),
        Drag::Page(PageDrag::Dragging(_)) => {
            Some(Input::Page(pagedrag::Input::Released(Over::Elsewhere)))
        }
    }
}

/// What window `id` losing the keys (`closed`: it went away) does to the drag: it ends as
/// cancelled when it is the drag's window. A drag the OS has (a page drag outside) is the OS's
/// to end, and a tab dragged out to tear off keeps going when only the focus moved.
pub fn window_lost_actions<P>(
    d: &Drag<P>,
    id: WindowId,
    drag_window: WindowId,
    closed: bool,
) -> Option<Input<P>> {
    match d {
        _ if id != drag_window => None,
        Drag::Idle | Drag::Page(PageDrag::Outside { .. }) => None,
        Drag::Tab(tabdrag::Drag::Dragging {
            over: DropTarget::Outside,
            ..
        }) if !closed => None,
        Drag::Tab(_) => Some(Input::Tab(tabdrag::Input::Cancel)),
        Drag::Page(PageDrag::Dragging(_)) => Some(Input::Page(pagedrag::Input::Cancelled)),
    }
}

fn tab<P>(d: tabdrag::Drag) -> Drag<P> {
    match d {
        tabdrag::Drag::Idle => Drag::Idle,
        d => Drag::Tab(d),
    }
}

/// The one transition.
pub fn transition<P: Payload>(state: Drag<P>, input: Input<P>) -> (Drag<P>, Vec<Effect<P>>) {
    match (state, input) {
        (Drag::Idle, Input::Tab(i)) => {
            let (d, fx) = tabdrag::transition(tabdrag::Drag::Idle, i);
            (tab(d), fx.into_iter().map(Effect::Tab).collect())
        }
        (Drag::Tab(d), Input::Tab(i)) => {
            let (d, fx) = tabdrag::transition(d, i);
            (tab(d), fx.into_iter().map(Effect::Tab).collect())
        }
        (Drag::Idle, Input::Page(i)) => match pagedrag::start(i) {
            Some((d, fx)) => (Drag::Page(d), fx.into_iter().map(Effect::Page).collect()),
            None => (Drag::Idle, vec![]),
        },
        (Drag::Page(d), Input::Page(i)) => {
            let (d, fx) = pagedrag::transition(d, i);
            (
                d.map_or(Drag::Idle, Drag::Page),
                fx.into_iter().map(Effect::Page).collect(),
            )
        }
        // a page drag while a tab row is held: ended with nothing done
        (state @ Drag::Tab(_), Input::Page(pagedrag::Input::Started { .. })) => (
            state,
            vec![Effect::Page(pagedrag::Effect::End(DragOps::NONE))],
        ),
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::pagedrag::{Effect as PE, Input as PI, Over};
    use crate::tab::TabId;
    use crate::tabdrag::{DropTarget, Input as TI, Point};

    #[derive(Clone, Debug, PartialEq, Default)]
    struct Data(Option<String>);

    impl Payload for Data {
        fn link(&self) -> Option<&str> {
            self.0.as_deref()
        }
        fn text(&self) -> Option<&str> {
            None
        }
    }

    fn page_start() -> Input<Data> {
        Input::Page(PI::Started {
            source: TabId(1),
            data: Data(Some("https://a.test/".into())),
            allowed: DragOps::COPY,
            confined: false,
            now: Instant::now(),
        })
    }

    fn press() -> Input<Data> {
        Input::Tab(TI::Press {
            tab: TabId(9),
            at: Point { x: 0.0, y: 10.0 },
        })
    }

    #[test]
    fn a_tab_drag_runs_through_its_own_machine() {
        let (d, fx) = transition(Drag::<Data>::Idle, press());
        assert!(matches!(d, Drag::Tab(tabdrag::Drag::Pressed { .. })));
        assert!(fx.is_empty());
        let (d, _) = transition(d, Input::Tab(TI::Cancel));
        assert_eq!(d, Drag::Idle);
        assert_eq!(d.tab(), tabdrag::Drag::Idle);
    }

    #[test]
    fn a_page_drag_runs_and_ends_idle() {
        let (d, fx) = transition(Drag::Idle, page_start());
        assert!(d.page_dragging().is_some());
        assert_eq!(fx.len(), 2);
        let (d, fx) = transition(d, Input::Page(PI::Released(Over::Elsewhere)));
        assert_eq!(d, Drag::Idle);
        assert_eq!(fx.last(), Some(&Effect::Page(PE::End(DragOps::NONE))));
    }

    #[test]
    fn one_drag_at_a_time() {
        // a page drag while a tab row is held is ended with nothing
        let (held, _) = transition(Drag::<Data>::Idle, press());
        let (d, fx) = transition(held.clone(), page_start());
        assert_eq!(d, held);
        assert_eq!(fx, [Effect::Page(PE::End(DragOps::NONE))]);
        // a tab press during a page drag is no tab drag
        let (page, _) = transition(Drag::Idle, page_start());
        let (d, fx) = transition(page.clone(), press());
        assert_eq!(d, page);
        assert!(fx.is_empty());
        let (d, fx) = transition(
            page.clone(),
            Input::Tab(TI::Release {
                hit: DropTarget::Outside,
            }),
        );
        assert_eq!(d, page);
        assert!(fx.is_empty());
        // page input while idle or during a tab drag is nothing
        let (d, fx) = transition(held.clone(), Input::Page(PI::Cancelled));
        assert_eq!((d, fx), (held, vec![]));
        let (d, fx) = transition(Drag::<Data>::Idle, Input::Page(PI::OsEnded(DragOps::COPY)));
        assert_eq!((d, fx), (Drag::Idle, vec![]));
    }

    use crate::pagedrag::Shell;
    use crate::space::SpaceId;
    use crate::tabdrag::Side;

    fn now() -> Instant {
        static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *T0.get_or_init(Instant::now)
    }

    fn feed(d: Drag<Data>, input: Input<Data>) -> Drag<Data> {
        transition(d, input).0
    }

    /// A tab pressed at the origin and dragged past the threshold over `hit`.
    fn tab_dragging(hit: DropTarget) -> Drag<Data> {
        let d = feed(Drag::Idle, press());
        feed(
            d,
            Input::Tab(TI::Move {
                at: Point { x: 80.0, y: 10.0 },
                hit,
                now: now(),
            }),
        )
    }

    fn page_dragging() -> Drag<Data> {
        feed(Drag::Idle, page_start())
    }

    fn page_outside() -> Drag<Data> {
        Drag::Page(PageDrag::Outside { source: TabId(1) })
    }

    /// A page drag released over a page: it waits for the page's answer.
    fn page_dropping() -> Drag<Data> {
        feed(
            page_dragging(),
            Input::Page(PI::Released(Over::Page(TabId(1)))),
        )
    }

    const TAB: Kind = Kind::TabGhost;
    const ZONE: Kind = Kind::DropZone;
    const PAGE: Kind = Kind::PageGhost;
    const MARK: Kind = Kind::DropMark;

    const TARGETS: [DropTarget; 9] = [
        DropTarget::None,
        DropTarget::List,
        DropTarget::SpacesTrigger,
        DropTarget::SpacesMenu,
        DropTarget::Space(SpaceId(2)),
        DropTarget::NewSpace,
        DropTarget::SplitEdge(Side::Left),
        DropTarget::Window(WindowId(2)),
        DropTarget::Outside,
    ];

    const OVERS: [Over; 3] = [
        Over::Page(TabId(1)),
        Over::Shell(Shell::Bar),
        Over::Elsewhere,
    ];

    /// What each drag state shows: a press shows nothing, a tab drag its ghost (and the zone over a
    /// content edge), a page drag its ghost and mark; none once the button is up or the OS has it.
    #[test]
    fn drag_surfaces() {
        let table: [(&str, Drag<Data>, &[Kind]); 8] = [
            ("idle", Drag::Idle, &[]),
            ("tab pressed", feed(Drag::Idle, press()), &[]),
            ("tab over the list", tab_dragging(DropTarget::List), &[TAB]),
            ("tab over nothing", tab_dragging(DropTarget::None), &[TAB]),
            (
                "tab over an edge",
                tab_dragging(DropTarget::SplitEdge(Side::Right)),
                &[TAB, ZONE],
            ),
            ("page dragging", page_dragging(), &[PAGE, MARK]),
            ("page dropping", page_dropping(), &[]),
            ("page outside", page_outside(), &[]),
        ];
        for (name, d, want) in table {
            assert_eq!(surfaces(&d), want, "{name}");
        }
    }

    /// A pointer move is no action: twenty moves inside one target never change the set.
    #[test]
    fn moves_do_not_change_the_surfaces() {
        let mut d = tab_dragging(DropTarget::SplitEdge(Side::Left));
        let set = surfaces(&d);
        for i in 0..20 {
            d = feed(
                d,
                Input::Tab(TI::Move {
                    at: Point {
                        x: 90.0 + f64::from(i),
                        y: 12.0,
                    },
                    hit: DropTarget::SplitEdge(Side::Left),
                    now: now(),
                }),
            );
            assert_eq!(surfaces(&d), set, "tab move {i}");
        }
        let mut d = page_dragging();
        for i in 0..20 {
            d = feed(
                d,
                Input::Page(PI::Moved {
                    over: Over::Elsewhere,
                    now: now(),
                }),
            );
            assert_eq!(surfaces(&d), [PAGE, MARK], "page move {i}");
        }
    }

    /// A release ends a drag whatever the target, and a cancel whatever the state: no surface is
    /// left, and nothing is dropped by a cancel.
    #[test]
    fn a_release_or_a_cancel_leaves_no_surface() {
        let mut ends: Vec<(String, Drag<Data>, Input<Data>)> = vec![];
        for hit in TARGETS {
            ends.push((
                format!("tab release over {hit:?}"),
                tab_dragging(hit),
                Input::Tab(TI::Release { hit }),
            ));
            ends.push((
                format!("tab cancel over {hit:?}"),
                tab_dragging(hit),
                Input::Tab(TI::Cancel),
            ));
            ends.push((
                format!("tab gone over {hit:?}"),
                tab_dragging(hit),
                Input::Tab(TI::Gone(TabId(9))),
            ));
        }
        for over in OVERS {
            ends.push((
                format!("page release over {over:?}"),
                page_dragging(),
                Input::Page(PI::Released(over)),
            ));
        }
        ends.push((
            "tab pressed, cancel".into(),
            feed(Drag::Idle, press()),
            Input::Tab(TI::Cancel),
        ));
        ends.push((
            "tab pressed, gone".into(),
            feed(Drag::Idle, press()),
            Input::Tab(TI::Gone(TabId(9))),
        ));
        ends.push((
            "page cancelled".into(),
            page_dragging(),
            Input::Page(PI::Cancelled),
        ));
        ends.push((
            "page source gone".into(),
            page_dragging(),
            Input::Page(PI::Gone(TabId(1))),
        ));
        ends.push((
            "page dropping, cancelled".into(),
            page_dropping(),
            Input::Page(PI::Cancelled),
        ));
        ends.push((
            "page outside, OS ended".into(),
            page_outside(),
            Input::Page(PI::OsEnded(DragOps::NONE)),
        ));
        ends.push((
            "page outside, cancelled".into(),
            page_outside(),
            Input::Page(PI::Cancelled),
        ));
        for (name, d, input) in ends {
            let cancel = matches!(
                input,
                Input::Tab(TI::Cancel | TI::Gone(_)) | Input::Page(PI::Cancelled | PI::Gone(_))
            );
            let (next, fx) = transition(d, input);
            // a release over a page waits for the page's answer: its ghost is gone at once
            if !matches!(next, Drag::Page(_)) {
                assert_eq!(next, Drag::Idle, "{name}");
            }
            assert!(surfaces(&next).is_empty(), "{name}: {next:?}");
            if cancel {
                let dropped = fx.iter().any(|e| {
                    matches!(
                        e,
                        Effect::Tab(
                            tabdrag::Effect::MoveToSpace { .. }
                                | tabdrag::Effect::MoveToNewSpace { .. }
                                | tabdrag::Effect::SplitInto { .. }
                                | tabdrag::Effect::MoveToWindow { .. }
                        ) | Effect::Page(PE::Drop { .. })
                    )
                });
                assert!(!dropped, "{name}: a cancel drops nothing: {fx:?}");
            }
        }
    }

    /// A release seen by another window ends the drag as cancelled; the drag's own window routes it.
    #[test]
    fn a_release_in_another_window_cancels() {
        let table: [(&str, Drag<Data>, bool, Option<Input<Data>>); 7] = [
            (
                "tab, own window",
                tab_dragging(DropTarget::None),
                true,
                None,
            ),
            (
                "tab, other window",
                tab_dragging(DropTarget::None),
                false,
                Some(Input::Tab(TI::Cancel)),
            ),
            (
                "tab pressed, other window",
                feed(Drag::Idle, press()),
                false,
                Some(Input::Tab(TI::Cancel)),
            ),
            ("page, own window", page_dragging(), true, None),
            (
                "page, other window",
                page_dragging(),
                false,
                Some(Input::Page(PI::Released(Over::Elsewhere))),
            ),
            ("page outside, other window", page_outside(), false, None),
            ("idle, other window", Drag::Idle, false, None),
        ];
        for (name, d, same, want) in table {
            assert_eq!(release_input(&d, same), want, "{name}");
        }
    }

    /// The window the drag runs in losing the keys, or closing, cancels it; another window's
    /// does not. A tab torn off keeps going when only the focus moved.
    #[test]
    fn a_window_lost_ends_the_drag_of_that_window() {
        let (own, other) = (WindowId(1), WindowId(2));
        let cancel = Some(Input::Tab(TI::Cancel));
        let page = Some(Input::Page(PI::Cancelled));
        // (name, state, lost window, closed, what runs)
        let table: [(&str, Drag<Data>, WindowId, bool, Option<Input<Data>>); 14] = [
            ("idle", Drag::Idle, own, true, None),
            (
                "tab pressed",
                feed(Drag::Idle, press()),
                own,
                false,
                cancel.clone(),
            ),
            (
                "tab pressed, closed",
                feed(Drag::Idle, press()),
                own,
                true,
                cancel.clone(),
            ),
            (
                "tab dragging",
                tab_dragging(DropTarget::None),
                own,
                false,
                cancel.clone(),
            ),
            (
                "tab dragging, closed",
                tab_dragging(DropTarget::None),
                own,
                true,
                cancel.clone(),
            ),
            (
                "tab dragging, other window",
                tab_dragging(DropTarget::None),
                other,
                true,
                None,
            ),
            (
                "tab torn off, focus",
                tab_dragging(DropTarget::Outside),
                own,
                false,
                None,
            ),
            (
                "tab torn off, closed",
                tab_dragging(DropTarget::Outside),
                own,
                true,
                cancel,
            ),
            ("page dragging", page_dragging(), own, false, page.clone()),
            ("page dragging, closed", page_dragging(), own, true, page),
            (
                "page dragging, other window",
                page_dragging(),
                other,
                true,
                None,
            ),
            ("page outside", page_outside(), own, false, None),
            ("page outside, closed", page_outside(), own, true, None),
            (
                "page outside, other window",
                page_outside(),
                other,
                false,
                None,
            ),
        ];
        for (name, d, id, closed, want) in table {
            assert_eq!(window_lost_actions(&d, id, own, closed), want, "{name}");
            // running it leaves no drag of ours behind
            if let Some(input) = want {
                assert!(surfaces(&feed(d, input)).is_empty(), "{name}");
            }
        }
    }
}
