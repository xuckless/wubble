//! The host's half of drags. One machine owns them (`wubble_core::drag`: a tab row's,
//! `tab_drag.rs`, or a page's own, here); this feeds it the pointer and the window's targets and
//! runs what it returns:
//! - a page's drag runs inside the window: the page under the pointer (the source's or another
//!   pane's) gets CEF's drop-target calls, the shell's targets (the tab list, the New tab row,
//!   the address bar, the Spaces switcher and popup) take a link or text, and the ghost (the
//!   drag image Chromium made, `kit/chrome/drag-ghost.slint`) follows the pointer; it is handed
//!   to the OS when it leaves the window where the platform can (`crate::wl`);
//! - drags from other apps are dropped on the page under the pointer or on the shell's targets
//!   (winit's files; on Wayland, `crate::wl`'s files, links, HTML and text).
//!
//! Nothing about a drag's content is logged.

use std::time::Instant;

use slint::{ComponentHandle, Model as _};

use crate::clipboard::Bridge as _;
use slint::winit_030::winit::event::WindowEvent;
use wubble_core::drag::{self, Drag, Effect, Input};
use wubble_core::model::Command;
use wubble_core::omnibox::{self, Intent};
use wubble_core::overlay::{Kind, Layer, Phase, Pointer, PointerOwner};
use wubble_core::pagedrag::{self, DragOps, Over, PageDrag, Shell};
use wubble_core::tab::TabId;
use wubble_core::tabdrag;
use wubble_engine::input::Point;
use wubble_engine::nav::Cursor;
use wubble_engine::page::DragData;
use wubble_engine::view::DragPhase;
use wubble_ui::{AppWindow, DropMarkKind, DropMarkView, PageGhost};

use super::{Host, new_tab};

/// Where a drag from another app is (moves come from the Wayland worker only).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum External {
    Enter,
    Over,
    Leave,
    Drop,
}

/// A drag from another app over the window: what it carries and where it is.
#[derive(Clone, Debug, Default)]
pub(super) struct ExternalDrag {
    data: DragData,
    over: Option<TabId>,
    /// The shell target under it, when it takes the drop.
    shell: Option<Shell>,
}

/// A page drag's image, decoded once at its start: the image, its logical size and hotspot.
pub(super) struct Ghost {
    image: slint::Image,
    size: (f32, f32),
    hotspot: (f32, f32),
}

/// The pointer during a drag that would do `op`, as Chromium's drag feedback: copy and link
/// have their own, a move the plain arrow.
pub fn cursor(op: DragOps) -> Cursor {
    if op.has(DragOps::COPY) {
        Cursor::Copy
    } else if op.has(DragOps::MOVE) {
        Cursor::Default
    } else if op.has(DragOps::LINK) {
        Cursor::Alias
    } else {
        Cursor::NotAllowed
    }
}

/// The list's insertion index and the row whose middle half is under `y` (window px), from the
/// first row's top, the pitch and the row count.
pub fn list_slot(y: f32, rows_top: f32, pitch: f32, rows: usize) -> (usize, Option<usize>) {
    if pitch <= 0.0 {
        return (rows, None);
    }
    let slot = (y - rows_top) / pitch;
    let row = slot.floor();
    let frac = slot - row;
    if row >= 0.0 && (row as usize) < rows && (0.25..0.75).contains(&frac) {
        let row = row as usize;
        return (row + 1, Some(row));
    }
    (slot.round().clamp(0.0, rows as f32) as usize, None)
}

impl Host {
    /// The page in `tab` started a drag (CEF `StartDragging`). CEF asks a little after the
    /// press and moves that began it; if the button was already released by then, the drag
    /// ends at once with nothing dropped, as an OS drag started after the release does.
    pub(super) fn drag_started(&mut self, tab: TabId, data: DragData, allowed: DragOps) {
        self.drag_window = self.w.id;
        // as an OS drag would, the page's tooltip goes
        self.tip_dismiss();
        if let Some(p) = self.input.pointer {
            self.drag_win = (p.x, p.y);
        }
        self.drag_ghost = data.image.as_ref().and_then(|i| ghost(i));
        let input = pagedrag::Input::Started {
            source: tab,
            data,
            allowed,
            confined: self.agent_drives(tab),
            now: Instant::now(),
        };
        self.run_drag(Input::Page(input));
        if !self
            .input
            .buttons
            .is_down(wubble_engine::input::MouseButton::Left)
        {
            self.drag_cancelled();
        }
    }

    /// The page in `tab` says what a drop at the pointer would do: the page drag's operation,
    /// or the answer to a drag from another app.
    pub(super) fn drag_cursor(&mut self, tab: TabId, op: DragOps) {
        if self.drag.page().is_some() {
            self.run_drag(Input::Page(pagedrag::Input::Cursor { tab, op }));
        } else if self.external_drag.as_ref().and_then(|d| d.over) == Some(tab) {
            self.aux().drop_action(op)
        }
    }

    /// No page drag runs (`--smoke parity`'s reset).
    pub(super) fn drag_idle(&self) -> bool {
        self.drag.page().is_none()
    }

    /// The running page drag's picture (`--smoke parity`): the type its bytes are, its file
    /// name from Chromium, and whether a drag image came with it.
    pub(super) fn drag_picture(&self) -> Option<(Option<&'static str>, Option<String>, bool)> {
        let data = &self.drag.page_dragging()?.data;
        let contents = data.file_contents.as_ref()?;
        Some((
            wubble_core::dnd::sniff_image_mime(&contents.bytes),
            contents.name.clone(),
            data.image.is_some(),
        ))
    }

    /// The page drag's ghost as drawn (`--smoke dnd`): with Chromium's image, and its
    /// logical size; `None` while none is drawn.
    pub(super) fn drag_ghost_shown(&self) -> Option<(bool, (f32, f32))> {
        self.drag.page_dragging()?;
        let window = self.w.window.upgrade()?;
        let g = window.get_page_ghost();
        (self.ui.phase(self.w.id, Layer::new(Kind::PageGhost, None)) == Some(Phase::Open))
            .then_some((g.has_image, (g.width, g.height)))
    }

    /// Pointer input during a page drag goes to the drag, not to the page as mouse input;
    /// false when no drag holds the pointer (none runs, or its drop waits for the page past
    /// the button: the click after a drop is the page's, `PageDrag::takes_pointer`). The pointer
    /// is the window's (`Input::pointer`): the pane that reported it may not be the source's (a
    /// rest on a tab row showed another tab).
    pub(super) fn drag_pointer(&mut self, point: Point, released: bool) -> bool {
        let Some(drag) = self.drag.page().filter(|d| d.takes_pointer()) else {
            return false;
        };
        let source = drag.source();
        // the OS has the pointer: only a button-up the window still saw reaches the drag
        if matches!(drag, PageDrag::Outside { .. }) && !released {
            return true;
        }
        let Some(window) = self.w.window.upgrade() else {
            return false;
        };
        let at = match self.input.pointer {
            Some(p) => (p.x, p.y),
            None => {
                let (x, y) = self.view_origin(&window, source);
                (x + point.x, y + point.y)
            }
        };
        self.drag_win = at;
        let over = self.drag_target(&window, at);
        let input = match (released, over) {
            (true, over) => {
                self.input.buttons = Default::default();
                pagedrag::Input::Released {
                    over: over.unwrap_or(Over::Elsewhere),
                    now: Instant::now(),
                }
            }
            (false, Some(over)) => pagedrag::Input::Moved {
                over,
                now: Instant::now(),
            },
            (false, None) => pagedrag::Input::LeftWindow,
        };
        self.run_drag(Input::Page(input));
        true
    }

    /// The window's own pointer (winit) during a page drag whose source is no longer on screen
    /// (a rest on a tab row showed another tab): no pane holds the pointer then, so the window
    /// reports it. Otherwise the source's pane does (`drag_pointer`).
    pub(super) fn drag_window_pointer(&mut self, released: bool) {
        let Some(PageDrag::Dragging(d)) = self.drag.page() else {
            return;
        };
        if self.shown_tabs().contains(&d.source) {
            return;
        }
        self.drag_pointer(Point { x: 0.0, y: 0.0 }, released);
    }

    /// The window's own left release (winit), after Slint has handed it on: a page drag its pane
    /// did not end (the pane lost the pointer's grab, or no pane holds it) ends here, so a
    /// release always ends a drag. Runs on the next turn of the event loop.
    pub(super) fn drag_window_released(&self) {
        let id = self.w.id;
        let _ = slint::invoke_from_event_loop(move || {
            crate::with_host_in(id, |host| {
                if host.drag.page().is_some_and(PageDrag::takes_pointer) {
                    crate::trace("drag: the window's release ends it");
                    host.drag_pointer(Point { x: 0.0, y: 0.0 }, true);
                }
            })
        });
    }

    /// Escape, or the window lost focus: a page drag ends with nothing dropped; false when no
    /// drag runs in the window.
    pub(super) fn drag_cancelled(&mut self) -> bool {
        if self.drag.page_dragging().is_none() {
            return false;
        }
        self.run_drag(Input::Page(pagedrag::Input::Cancelled));
        true
    }

    /// `tab`'s view closed mid-drag: a page drag whose source it is ends, and a tab row held or
    /// dragged ends when the tab itself is gone (a discarded or slept tab's row stays in the
    /// list, so its drag goes on).
    pub(super) fn drag_gone(&mut self, tab: TabId) {
        match self.drag {
            Drag::Page(_) => self.run_drag(Input::Page(pagedrag::Input::Gone(tab))),
            Drag::Tab(_) if self.model.window_of(tab).is_none() => {
                self.run_drag(Input::Tab(tabdrag::Input::Gone(tab)))
            }
            Drag::Tab(_) => {}
            Drag::Idle => {}
        }
    }

    /// Window `id` lost the keys or `closed`: the drag it runs ends as cancelled (a release can
    /// no longer reach it), and what ends with the window in the store (a swipe, an autoscroll, a
    /// pick) is cancelled. The one entry for the focus and the close.
    pub(super) fn window_lost(&mut self, id: wubble_core::window::WindowId, closed: bool) {
        if let Some(input) = drag::window_lost_actions(&self.drag, id, self.drag_window, closed) {
            self.run_drag(input);
        }
        if !closed {
            let w = self.w.id;
            self.act(wubble_core::overlay::Action::Lost { w });
        }
    }

    /// A left release seen by this window while another window's drag runs: it ends the drag as
    /// cancelled, so no ghost outlives the button.
    pub(super) fn drag_released_elsewhere(&mut self) {
        let same = self.w.id == self.drag_window;
        if let Some(input) = drag::release_input(&self.drag, same, Instant::now()) {
            self.run_drag(input);
        }
    }

    /// The OS drag a page drag became ended with `op` done.
    fn os_drag_ended(&mut self, op: DragOps) {
        self.run_drag(Input::Page(pagedrag::Input::OsEnded(op)));
    }

    /// A hover delay ran out: on a tab row, the Spaces switcher, or off the Spaces popup.
    pub fn drag_tick(&mut self) {
        let now = Instant::now();
        let input = match &self.drag {
            Drag::Tab(_) => Input::Tab(tabdrag::Input::Tick { now }),
            Drag::Page(_) => Input::Page(pagedrag::Input::Tick { now }),
            Drag::Idle => return,
        };
        self.run_drag(input);
    }

    /// Runs the one machine, keeps the hover timer while a rest would still do something, runs
    /// the effects, then draws the drag in the window it started in.
    pub(super) fn run_drag(&mut self, input: Input<DragData>) {
        // the source of a page drag, kept until its end is run (the state may be idle by then)
        let source_tab = self.drag.page().map(PageDrag::source).or(match &input {
            Input::Page(pagedrag::Input::Started { source, .. }) => Some(*source),
            _ => None,
        });
        let (from, by) = (self.drag.name(), input.name());
        let (next, effects) = drag::transition(std::mem::take(&mut self.drag), input);
        self.drag = next;
        if from != self.drag.name() {
            crate::trace(&format!("drag: {from} -> {} on {by}", self.drag.name()));
        }
        let source = self.drag_window;
        // a page drag's rest ends at its due time, which no move pushes back
        let wait = match self.drag.page_dragging() {
            Some(d) => d
                .due()
                .map(|due| due.saturating_duration_since(Instant::now())),
            None => (self.drag.tab().awaits_dwell())
                .then_some(pagedrag::DWELL.max(tabdrag::SPACE_HOVER)),
        };
        match wait {
            Some(wait) => self
                .drag_timer
                .start(slint::TimerMode::SingleShot, wait, move || {
                    crate::with_host_in(source, Host::drag_tick)
                }),
            None => self.drag_timer.stop(),
        }
        for effect in effects {
            match effect {
                Effect::Tab(e) => self.tab_drag_effect(e),
                Effect::Page(e) => self.page_drag_effect(e, source_tab),
            }
        }
        let here = self.w.id;
        if self.enter(source)
            && let Some(window) = self.w.window.upgrade()
        {
            // the tab drag's marks, unless a page drag owns them now
            if self.drag.page().is_none() {
                self.render_tab_drag(&window);
            }
            self.render_page_drag(&window);
            self.sync_drag_surfaces();
        }
        self.enter(here);
        if self.drag.page().is_none() {
            self.drag_ghost = None;
        }
    }

    /// Opens and closes the drag's surfaces in the store to what its state shows (closes first:
    /// a grab opens only once what it ends is closed); a pointer move changes none.
    pub(super) fn sync_drag_surfaces(&mut self) {
        let shown = drag::surfaces(&self.drag);
        for kind in drag::SURFACES {
            if !shown.contains(&kind) {
                self.surface_is(kind, false);
            }
        }
        for &kind in shown {
            self.surface_is(kind, true);
        }
    }

    /// Where `tab`'s page is under the drag's pointer, in its view's px.
    fn drag_point_in(&self, window: &AppWindow, tab: TabId) -> Point {
        self.page_point(window, tab, self.drag_win)
    }

    /// One page-drag effect; `source` is the drag's source page (`None` for a drop from
    /// another app on the shell).
    fn page_drag_effect(&mut self, effect: pagedrag::Effect<DragData>, source: Option<TabId>) {
        use pagedrag::Effect as E;
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let modifiers = self.input.modifiers();
        let page = |host: &mut Self, tab: TabId, phase: DragPhase, allowed: DragOps| {
            let p = host.drag_point_in(&window, tab);
            if let Some(view) = host.tab_view(tab) {
                view.drag(phase, p, &modifiers, allowed);
            }
        };
        match effect {
            E::Enter { tab, allowed } => {
                let phase = if Some(tab) == source {
                    DragPhase::EnterSource
                } else {
                    // another view: the source page's data as CEF holds it, custom types and all
                    let held = source
                        .and_then(|s| self.tab_view(s))
                        .and_then(|v| v.held_drag());
                    match (held, self.drag.page_dragging()) {
                        (Some(held), _) => DragPhase::EnterFrom(held),
                        (None, Some(d)) => DragPhase::Enter(d.data.clone()),
                        (None, None) => return,
                    }
                };
                page(self, tab, phase, allowed);
            }
            E::Over { tab, allowed } => page(self, tab, DragPhase::Over, allowed),
            E::Leave { tab } => page(self, tab, DragPhase::Leave, DragOps::NONE),
            E::Drop { tab } => page(self, tab, DragPhase::Drop, DragOps::NONE),
            E::End(op) => {
                self.set_page_cursor(&window, Cursor::Default);
                window.set_spaces_drop_line(-1);
                if let Some(source) = source {
                    let p = self.drag_point_in(&window, source);
                    if let Some(view) = self.tab_view(source) {
                        view.drag_source_ended(p, op);
                    }
                }
            }
            E::Feedback(op) => self.set_page_cursor(&window, cursor(op)),
            E::Mark(target) => self.drop_mark(&window, target),
            E::OpenTab { text, index } => self.drop_open_tab(&text, index),
            E::FillBar(text) => self.drop_fill_bar(text),
            E::OpenInSpace { text, space } => {
                if let Some(url) = self.dropped_url(&text) {
                    self.dispatch(Command::OpenInSpace { space, url });
                    self.dispatch(Command::SwitchSpace(space));
                }
            }
            E::Bookmark { url, title, folder } => {
                let title = title.unwrap_or_else(|| url.clone());
                self.bookmarks_request(Some(wubble_core::pages::BookmarkEdit::Add {
                    parent: folder,
                    title,
                    url,
                }));
            }
            E::Activate(tab) => self.dispatch(Command::ActivateTab(tab)),
            E::OpenSpaces => self.spaces_menu_requested(),
            E::CloseSpaces => self.spaces_dismissed(),
            E::StartOs(data, allowed) => {
                if !self.aux().start_drag(self.w.id, &data, allowed) {
                    self.os_drag_ended(DragOps::NONE);
                }
            }
            // Slint still holds the pressed pane's grab (it never saw the button come up): the
            // pointer leaving the window lets it go (the pane hears a cancel, the page a capture
            // lost), on the next turn, outside Slint's own dispatch.
            E::LetGo => {
                let _ = self.w.window.upgrade_in_event_loop(|window| {
                    window
                        .window()
                        .dispatch_event(slint::platform::WindowEvent::PointerExited);
                });
            }
        }
    }

    /// The URL a dropped link or text opens: a URL as it is, text searched as the bar searches
    /// it; `None` before a search engine is chosen.
    fn dropped_url(&self, text: &str) -> Option<String> {
        match omnibox::classify(text) {
            Intent::Url(url) => Some(url),
            Intent::Search(q) => self
                .search_engine
                .map(|engine| omnibox::fill(engine.info().search, &q)),
        }
    }

    /// A new tab with the dropped link or text, shown, at the list's `index` (Chrome's
    /// `NavigateToDroppedUrls`); without a search engine yet, text goes to the bar instead.
    fn drop_open_tab(&mut self, text: &str, index: Option<usize>) {
        let Some(url) = self.dropped_url(text) else {
            return self.drop_fill_bar(text.to_owned());
        };
        self.dispatch(new_tab(url, None));
        let (Some(tab), Some(index)) = (self.model.visible_tab(), index) else {
            return;
        };
        if let Some(to) = crate::shell::move_dest(&self.model, tab, index) {
            self.dispatch(Command::MoveTab { tab, to });
        }
    }

    /// The address bar opens holding `text`, all of it selected, for Enter (Chrome's omnibox
    /// drop fills it, it does not navigate).
    fn drop_fill_bar(&mut self, text: String) {
        let win = self.w.id;
        // the bar is open: its field takes the text, its mode stays
        if let Some(window) = self.w.window.upgrade()
            && window.get_command_bar_state() == wubble_ui::CommandBarState::Open
        {
            window.set_command_query(text.as_str().into());
            self.bar_query(&window, &text);
            return;
        }
        let _ = self.w.window.upgrade_in_event_loop(move |window| {
            window.invoke_request_address_focus();
            crate::with_host_in(win, |host| {
                window.set_command_query(text.as_str().into());
                host.bar_query(&window, &text);
            });
        });
    }

    /// What is under `at` (window logical px) for a page drag; `None` outside the window when
    /// the platform can take the drag over (the button is held, so the window still sees the
    /// pointer there).
    fn drag_target(&self, window: &AppWindow, at: (f32, f32)) -> Option<Over> {
        let size = window
            .window()
            .size()
            .to_logical(window.window().scale_factor());
        let inside = (0.0..size.width).contains(&at.0) && (0.0..size.height).contains(&at.1);
        if !inside {
            return if self.aux().drags_out() {
                None
            } else {
                Some(Over::Elsewhere)
            };
        }
        Some(
            self.shell_target(window, at)
                .map(Over::Shell)
                .or_else(|| self.page_under(window, at).map(Over::Page))
                .unwrap_or(Over::Elsewhere),
        )
    }

    /// A page on screen under `at` that may take a drop: a model tab's with a web view (not the
    /// DevTools panel or an extension popup, nor one of wubble's own pages, which are the
    /// shell's), where the store gives the pointer to the page (`Ui::pointer_at`: not under a
    /// surface that takes it over itself, as the peek's card or an autofill part, nor under a
    /// sheet's scrim; a drag's own grab is the drag's). An agent's drag reaches no other page:
    /// [`pagedrag`] confines it.
    fn page_under(&self, window: &AppWindow, at: (f32, f32)) -> Option<TabId> {
        let tab = self
            .pane_under(window, at)
            .filter(|t| self.model.tabs.contains_key(t) && self.views.contains_key(t))?;
        let (_, facts) = self.page_covers(window, tab);
        let p = tabdrag::Point {
            x: f64::from(at.0),
            y: f64::from(at.1),
        };
        let page = match self.ui.pointer_at(&self.env(), &facts, self.w.id, p) {
            PointerOwner::Page => true,
            PointerOwner::Layer(l) => l.kind.row().pointer == Pointer::Grab,
        };
        page.then_some(tab)
    }

    /// The shell's drop target under `at`, while the sidebar shows.
    fn shell_target(&self, window: &AppWindow, at: (f32, f32)) -> Option<Shell> {
        let within = |x: f32, y: f32, w: f32, h: f32| {
            (x..x + w).contains(&at.0) && (y..y + h).contains(&at.1)
        };
        // the open command bar's field, over everything but the menus
        if window.get_command_bar_state() == wubble_ui::CommandBarState::Open
            && within(
                window.get_command_field_x(),
                window.get_command_field_y(),
                window.get_command_field_w(),
                window.get_command_field_h(),
            )
        {
            return Some(Shell::Bar);
        }
        // the bookmarks page, a folder row on it (its pane, in a split)
        let bookmarks = self.shown_page() == Some(wubble_core::pages::InternalPage::Bookmarks)
            && self.pane_under(window, at).is_some_and(|t| Some(t) == self.page_tab());
        self.bookmarks_drop_point(window, bookmarks.then_some(at));
        if bookmarks {
            return Some(Shell::Bookmarks {
                folder: self.bookmark_drop,
            });
        }
        let p = tabdrag::Point {
            x: f64::from(at.0),
            y: f64::from(at.1),
        };
        match tabdrag::hit(p, &self.drop_rects(window)) {
            tabdrag::DropTarget::Space(space) => return Some(Shell::Space(space)),
            tabdrag::DropTarget::SpacesTrigger => return Some(Shell::SpacesTrigger),
            _ => {}
        }
        if window.get_sidebar_width() <= 0.5 {
            return None;
        }
        let g = window.get_drop_geometry();
        if within(g.pill_x, g.pill_y, g.pill_w, g.pill_h) {
            return Some(Shell::Bar);
        }
        if within(g.new_tab_x, g.new_tab_y, g.new_tab_w, g.new_tab_h) {
            return Some(Shell::NewTab);
        }
        // the list and half a gap around it, where the first and last gaps are
        let half = (g.pitch - g.row_h).max(0.0) / 2.0;
        if within(g.list_x, g.list_y - half, g.list_w, g.list_h + 2.0 * half) {
            let rows = window.get_tabs();
            let (index, row) = list_slot(at.1, g.rows_top, g.pitch, rows.row_count());
            let row = row
                .and_then(|i| rows.row_data(i))
                .and_then(|r| crate::shell::tab_id(r.id));
            return Some(Shell::List { index, row });
        }
        None
    }

    /// Tells the bookmarks page where a drag is over it (`None`: not over it), so its folder rows
    /// can say which one is under the pointer ([`Host::bookmark_drop_hovered`]).
    fn bookmarks_drop_point(&self, window: &AppWindow, at: Option<(f32, f32)>) {
        let mut drop = window.get_bookmarks_drop();
        let (on, (x, y)) = (at.is_some(), at.unwrap_or_default());
        if (drop.on, drop.x, drop.y) != (on, x, y) {
            (drop.on, drop.x, drop.y) = (on, x, y);
            window.set_bookmarks_drop(drop);
        }
    }

    /// The bookmarks page's folder row `id` is (`on`) or is no longer under a drag.
    pub fn bookmark_drop_hovered(&mut self, id: i32, on: bool) {
        let id = i64::from(id);
        match (on, self.bookmark_drop) {
            (true, _) => self.bookmark_drop = Some(id),
            (false, Some(was)) if was == id => self.bookmark_drop = None,
            _ => {}
        }
    }

    /// The drop indicator over `target`, or none.
    fn drop_mark(&self, window: &AppWindow, target: Option<Shell>) {
        let g = window.get_drop_geometry();
        let mark = |kind, x, y, width, height| DropMarkView {
            kind,
            x,
            y,
            width,
            height,
        };
        let none = DropMarkView::default();
        window.set_spaces_drop_line(-1);
        let mut drop = window.get_bookmarks_drop();
        drop.folder_on = false;
        let bar_open = window.get_command_bar_state() == wubble_ui::CommandBarState::Open;
        let view = match target {
            Some(Shell::Bar) if bar_open => mark(
                DropMarkKind::Box,
                window.get_command_field_x(),
                window.get_command_field_y(),
                window.get_command_field_w(),
                window.get_command_field_h(),
            ),
            Some(Shell::Bar) => mark(DropMarkKind::Box, g.pill_x, g.pill_y, g.pill_w, g.pill_h),
            // a folder row lights itself; the page's top level: the pane boxed
            Some(Shell::Bookmarks {
                folder: Some(folder),
            }) => {
                drop.folder_on = true;
                drop.folder = i32::try_from(folder).unwrap_or(-1);
                none
            }
            Some(Shell::Bookmarks { folder: None }) => mark(
                DropMarkKind::Box,
                window.get_content_x(),
                window.get_content_y(),
                window.get_content_width(),
                window.get_content_height(),
            ),
            Some(Shell::NewTab) => mark(
                DropMarkKind::Box,
                g.new_tab_x,
                g.new_tab_y,
                g.new_tab_w,
                g.new_tab_h,
            ),
            // the row a rest would show, boxed; else the gap, a line between the rows
            Some(Shell::List { index, row }) => {
                let top = g.rows_top + index as f32 * g.pitch;
                let line = g.line;
                match row {
                    Some(_) => mark(
                        DropMarkKind::Box,
                        g.list_x,
                        top - g.pitch,
                        g.list_w,
                        g.row_h,
                    ),
                    None => mark(
                        DropMarkKind::Line,
                        g.list_x,
                        top - (g.pitch - g.row_h) / 2.0 - line / 2.0,
                        g.list_w,
                        line,
                    ),
                }
            }
            // the popup row lit as the pointer lights one
            Some(Shell::Space(space)) => {
                let geometry = super::tab_drag::spaces_geometry(&window.get_spaces_menu_geometry());
                let popup = self.spaces_popup();
                if let Some((i, _, _)) = popup
                    .drop_rows(&geometry)
                    .into_iter()
                    .find(|(_, s, _)| *s == space)
                {
                    window.set_spaces_drop_line(i32::try_from(i).unwrap_or(-1));
                    window.set_spaces_drop_top(popup.line_top(i, &geometry) as f32);
                }
                none
            }
            Some(Shell::SpacesTrigger) | None => none,
        };
        window.set_bookmarks_drop(drop);
        window.set_drop_mark(view);
    }

    /// The page drag's ghost: inside the window, at the pointer; gone once the OS has the drag.
    fn render_page_drag(&self, window: &AppWindow) {
        let Some(d) = self.drag.page_dragging() else {
            // content only: the store says whether it shows (Slint ignores a value that is the same)
            window.set_page_ghost(PageGhost::default());
            window.set_drop_mark(DropMarkView::default());
            window.set_bookmarks_drop(Default::default());
            return;
        };
        let (x, y) = self.drag_win;
        let mut view = PageGhost {
            x,
            y,
            ..PageGhost::default()
        };
        match &self.drag_ghost {
            Some(g) => {
                view.has_image = true;
                view.image = g.image.clone();
                (view.width, view.height) = g.size;
                (view.hotspot_x, view.hotspot_y) = g.hotspot;
            }
            // Chromium gave no image: the label, the link's title over its URL, or the text
            None => {
                let text = d
                    .data
                    .text
                    .as_deref()
                    .map(|t| t.chars().take(200).collect::<String>());
                view.title = d
                    .data
                    .link_title
                    .clone()
                    .or(text)
                    .unwrap_or_default()
                    .into();
                view.url = d.data.link_url.clone().unwrap_or_default().into();
            }
        }
        window.set_page_ghost(view);
    }

    /// A file from another app over the window (winit's `HoveredFile`, `DroppedFile`,
    /// `HoveredFileCancelled`; X11, macOS and Windows: winit 0.30 has no Wayland drops, which
    /// come through `crate::wl`).
    pub(super) fn file_drop(&mut self, window: &AppWindow, event: &WindowEvent) {
        let at = self.input.pointer.map_or((0.0, 0.0), |p| (p.x, p.y));
        let phase = match event {
            WindowEvent::HoveredFile(path) => {
                // winit reports each file of a drop on its own: they add up.
                let mut files = self
                    .external_drag
                    .as_ref()
                    .map(|d| d.data.files.clone())
                    .unwrap_or_default();
                files.push(path.clone());
                self.external_drag(window, External::Leave, at);
                let data = DragData {
                    files,
                    ..DragData::default()
                };
                self.external_drag = Some(ExternalDrag {
                    data,
                    ..ExternalDrag::default()
                });
                External::Enter
            }
            WindowEvent::DroppedFile(_) => External::Drop,
            WindowEvent::HoveredFileCancelled => External::Leave,
            _ => return,
        };
        self.external_drag(window, phase, at);
    }

    /// A drag from another app at `at` (window logical px). `Enter` starts it with the data
    /// already in [`Host::external_drag`]; the page under the pointer gets it (never a page an
    /// agent session drives: no file reaches an agent's page that way), another page when the
    /// pointer crosses into its pane, and a link or text dropped on the shell does what a page's
    /// drag does there.
    pub(super) fn external_drag(&mut self, window: &AppWindow, phase: External, at: (f32, f32)) {
        let Some(mut drag) = self.external_drag.take() else {
            return;
        };
        let modifiers = self.input.modifiers();
        let shell = self
            .shell_target(window, at)
            .filter(|s| pagedrag::shell_drop::<_, DragData>(*s, &drag.data).is_some());
        let under = self
            .page_under(window, at)
            .filter(|t| shell.is_none() && !self.agent_drives(*t));
        let point_in = |host: &Self, tab: TabId| host.page_point(window, tab, at);
        let (leave, enter) = match phase {
            External::Leave => (drag.over, None),
            _ if drag.over != under || phase == External::Enter => (drag.over, under),
            _ => (None, None),
        };
        if let Some(tab) = leave {
            let p = point_in(self, tab);
            if let Some(view) = self.tab_view(tab) {
                view.drag(DragPhase::Leave, p, &modifiers, DragOps::NONE);
            }
        }
        if let Some(tab) = enter {
            let p = point_in(self, tab);
            if let Some(view) = self.tab_view(tab) {
                view.drag(
                    DragPhase::Enter(drag.data.clone()),
                    p,
                    &modifiers,
                    DragOps::ANY,
                );
            }
        }
        let shell = if phase == External::Leave {
            None
        } else {
            shell
        };
        if shell != drag.shell {
            self.drop_mark(window, shell);
            drag.shell = shell;
        }
        if phase == External::Leave {
            return;
        }
        drag.over = under;
        match (under, shell) {
            (_, Some(s)) => {
                self.aux().drop_action(DragOps::COPY);
                if phase == External::Drop {
                    self.drop_mark(window, None);
                    if let Some(effect) = pagedrag::shell_drop(s, &drag.data) {
                        self.page_drag_effect(effect, None);
                    }
                }
            }
            (None, None) => self.aux().drop_action(DragOps::NONE),
            (Some(tab), None) => {
                let p = point_in(self, tab);
                if let Some(view) = self.tab_view(tab) {
                    view.drag(DragPhase::Over, p, &modifiers, DragOps::ANY);
                    if phase == External::Drop {
                        view.drag(DragPhase::Drop, p, &modifiers, DragOps::NONE);
                    }
                }
            }
        }
        if phase != External::Drop {
            self.external_drag = Some(drag);
        }
    }

    /// The page on screen under `at` (window logical px).
    pub(super) fn pane_under(&self, window: &AppWindow, at: (f32, f32)) -> Option<TabId> {
        let height = window.get_content_height();
        self.pane_rects(window).into_iter().find_map(|r| {
            let p = self.page_point(window, r.tab, at);
            ((0.0..r.width as f32).contains(&p.x) && (0.0..height).contains(&p.y)).then_some(r.tab)
        })
    }

    /// The platform's extras (`crate::wl`), on the clipboard's connection.
    pub(super) fn aux(&self) -> &dyn crate::wl::Aux {
        self.clipboard.extras().unwrap_or(&crate::wl::Null)
    }

    /// Tells the platform's extras about this window once its platform window exists.
    pub(super) fn add_aux_window(&mut self, window: &AppWindow) {
        if !self.w.aux_added {
            self.w.aux_added = true;
            self.aux().add_window(self.w.id, window.window());
        }
    }

    /// What the platform reported for this window (`crate::wl`, on Wayland only).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn aux_event(&mut self, event: crate::wl::AuxEvent) {
        use crate::wl::AuxEvent as A;
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        match event {
            A::DropEnter { x, y, data } => {
                self.external_drag(&window, External::Leave, (x, y));
                self.external_drag = Some(ExternalDrag {
                    data,
                    ..ExternalDrag::default()
                });
                self.external_drag(&window, External::Enter, (x, y));
            }
            A::DropMove { x, y } => self.external_drag(&window, External::Over, (x, y)),
            A::DropLeave => self.external_drag(&window, External::Leave, (0.0, 0.0)),
            A::Dropped { x, y } => self.external_drag(&window, External::Drop, (x, y)),
            A::DragEnded(op) => self.os_drag_ended(op),
            A::Pinch(input) => self.pinch(input),
            A::Swipe { input, at } => self.history_swipe(&window, input, at),
            A::Locks(locks) => self.input.locks = locks,
        }
    }
}

impl Host {
    /// `--smoke gallery`'s page drag held still (no page is dragged: the ghost and the mark as
    /// `render_page_drag` and `drop_mark` draw them).
    pub(super) fn gallery_page_drag(
        &mut self,
        window: &AppWindow,
        at: crate::gallery_smoke::GhostScene,
    ) {
        use crate::gallery_smoke::GhostScene as G;
        let g = window.get_drop_geometry();
        let label = |x, y| PageGhost {
            x,
            y,
            title: "The Wallpaper Project".into(),
            url: "https://example.com/the-wallpaper-project".into(),
            ..PageGhost::default()
        };
        let (cx, cy) = (window.get_content_x(), window.get_content_y());
        let (ghost, mark) = match at {
            G::LabelOverPage => (label(cx + 320.0, cy + 260.0), None),
            G::ImageOverPage => {
                // a picture as Chromium hands one over: 160 x 100 at 75 %
                let (w, h) = (160u32, 100u32);
                let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
                for (i, px) in buffer.make_mut_slice().iter_mut().enumerate() {
                    let (x, y) = (
                        (i as u32 % w) as f32 / w as f32,
                        (i as u32 / w) as f32 / h as f32,
                    );
                    *px = slint::Rgba8Pixel::new(
                        (40.0 + 60.0 * x) as u8,
                        (160.0 - 40.0 * y) as u8,
                        (120.0 + 90.0 * y) as u8,
                        191,
                    );
                }
                let ghost = PageGhost {
                    x: cx + 360.0,
                    y: cy + 280.0,
                    has_image: true,
                    image: slint::Image::from_rgba8(buffer),
                    width: w as f32,
                    height: h as f32,
                    hotspot_x: 60.0,
                    hotspot_y: 40.0,
                    ..PageGhost::default()
                };
                (ghost, None)
            }
            G::OverTabList => (
                label(g.list_x + g.list_w / 2.0, g.rows_top + g.pitch - 2.0),
                Some(Shell::List {
                    index: 1,
                    row: None,
                }),
            ),
            G::OverBar => (
                label(g.pill_x + g.pill_w / 2.0, g.pill_y + g.pill_h / 2.0),
                Some(Shell::Bar),
            ),
        };
        window.set_page_ghost(ghost);
        self.drop_mark(window, mark);
        // the rows the real drag opens (`sync_drag_surfaces`)
        for kind in [Kind::PageGhost, Kind::DropMark] {
            self.surface_is(kind, true);
        }
    }
}

/// The drag image CEF gave (Chromium's own, its opacity in its pixels), decoded for the shell;
/// `None` when it does not decode.
fn ghost(image: &wubble_engine::page::DragImage) -> Option<Ghost> {
    let (width, height, rgba) = crate::shot_io::rgba(&image.png).ok()?;
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(width, height);
    buffer.make_mut_bytes().copy_from_slice(&rgba);
    let (w, h) = match image.size {
        (w, h) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => (width as f32, height as f32),
    };
    Some(Ghost {
        image: slint::Image::from_rgba8(buffer),
        size: (w, h),
        hotspot: (image.hotspot.0 as f32, image.hotspot.1 as f32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_slot_is_a_gap_or_a_row_s_middle() {
        // rows from y 100, 40 apart, 3 of them
        assert_eq!(list_slot(100.0, 100.0, 40.0, 3), (0, None));
        assert_eq!(list_slot(105.0, 100.0, 40.0, 3), (0, None));
        assert_eq!(list_slot(120.0, 100.0, 40.0, 3), (1, Some(0)));
        assert_eq!(list_slot(135.0, 100.0, 40.0, 3), (1, None));
        assert_eq!(list_slot(160.0, 100.0, 40.0, 3), (2, Some(1)));
        // below the last row: the end; above the first: the start
        assert_eq!(list_slot(400.0, 100.0, 40.0, 3), (3, None));
        assert_eq!(list_slot(20.0, 100.0, 40.0, 3), (0, None));
        assert_eq!(list_slot(120.0, 100.0, 0.0, 3), (3, None));
    }

    #[test]
    fn the_pointer_shows_the_operation() {
        assert_eq!(cursor(DragOps::COPY), Cursor::Copy);
        assert_eq!(cursor(DragOps::MOVE), Cursor::Default);
        assert_eq!(cursor(DragOps::LINK), Cursor::Alias);
        assert_eq!(cursor(DragOps::NONE), Cursor::NotAllowed);
    }
}
