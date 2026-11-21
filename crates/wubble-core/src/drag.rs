//! Every drag in a window, one at a time: a tab row dragged out of the list
//! ([`crate::tabdrag`]) or a page's own drag ([`crate::pagedrag`]). One state, one pure
//! transition, effects out; the host feeds it the pointer and runs what it returns.
//!
//! A page drag that starts while a tab is dragged is refused (it ends at once with nothing
//! dropped, as an OS refuses a second drag), and tab-row input during a page drag is not a
//! tab drag: the pointer belongs to the drag that has it.

use crate::pagedrag::{self, DragOps, PageDrag, Payload};
use crate::tabdrag;

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
}
