//! Pages for screen readers (TODO.md Decided "Accessibility"). While pages' accessibility is on
//! (`a11y.web`: on, or on auto while a screen reader runs, `apply::web_accessibility`) every view
//! has CEF's accessibility tree turned on (`View::set_accessibility`, welding change 50), each
//! tab's changes are mirrored in a `wubble_ax::PageTree`, and each window shows its page's items
//! (`PageTree::flatten`) as Slint's own accessible elements (`page-ax.slint`), so they reach the
//! screen reader through Slint's AccessKit adapter. Off, nothing is asked of CEF and the list is
//! empty. A screen reader's actions come back by item: a click at the item's centre through the
//! pointer path (an item outside the viewport is first scrolled to its middle,
//! `wubble_ax::reach`), a text field's value typed over its text (CEF has no accessibility
//! actions for windowless browsers).
//!
//! A flat list, not the page's tree: Slint 1.19 cannot graft a foreign AccessKit subtree, and has
//! no link or heading role (see `page-ax.slint`); the full mapping (`PageTree::apply`) waits for
//! that (M6).

use std::collections::BTreeMap;

use slint::platform::PointerEventButton;
use wubble_ax::{AxItem, AxValue, Bounds, Kind, PageTree, Reach, value};
use wubble_core::apply::web_accessibility;
use wubble_core::motion::Calm;
use wubble_core::tab::TabId;
use wubble_engine::input::Point;
use wubble_engine::page::EditCommand;
use wubble_engine::page::PageEvent;
use wubble_ui::{AppWindow, AxItemView, AxKind};

use super::Host;
use crate::screen_reader::{self, ScreenReader};

/// Items one page shows at most: each is a Slint element, so a page of thousands of nodes keeps
/// its first ones.
const MAX_ITEMS: usize = 1500;

/// A screen reader's action on an item outside the viewport: the page was asked to scroll it
/// to the middle (`eval` answers with how far it scrolled, clamped at the page's ends; the
/// tree's own update for a scroll alone may never come).
struct Waiting {
    tab: TabId,
    eval: u64,
    bounds: Bounds,
    viewport: Bounds,
    name: String,
    text: String,
}

pub(super) struct WebAx {
    setting: Calm,
    reader: Box<dyn ScreenReader>,
    /// What the views were last told.
    on: bool,
    trees: BTreeMap<TabId, PageTree>,
    waiting: Option<Waiting>,
}

impl Default for WebAx {
    fn default() -> Self {
        Self {
            setting: Calm::Auto,
            reader: screen_reader::source(),
            on: false,
            trees: BTreeMap::new(),
            waiting: None,
        }
    }
}

/// The shell's row for an item: `id` is its place in the list, its rectangle in the content
/// area (the page's view DIP, from the pane's left edge).
fn view_of(index: usize, item: &AxItem, pane_x: f32) -> AxItemView {
    let kind = match item.kind {
        Kind::Heading => AxKind::Heading,
        Kind::Link => AxKind::Link,
        Kind::Button => AxKind::Button,
        Kind::TextField => AxKind::TextField,
        Kind::CheckBox => AxKind::CheckBox,
        Kind::RadioButton => AxKind::RadioButton,
        Kind::Image => AxKind::Image,
        Kind::Text => AxKind::Text,
    };
    let b = item.bounds;
    AxItemView {
        id: i32::try_from(index).unwrap_or(i32::MAX),
        kind,
        label: item.name.as_str().into(),
        value: item.value.as_str().into(),
        level: item.level,
        checked: item.checked == Some(true),
        x: pane_x + b.x as f32,
        y: b.y as f32,
        w: b.width.max(0.0) as f32,
        h: b.height.max(0.0) as f32,
    }
}

/// A page script that scrolls by `(dx, dy)` CSS px and answers with how far it went
/// (`[dx, dy]`, clamped at the page's ends).
pub(super) fn scroll_script(dx: f64, dy: f64) -> String {
    format!(
        "(()=>{{const x=scrollX,y=scrollY;\
         scrollBy({{left:{dx},top:{dy},behavior:'instant'}});\
         return [scrollX-x,scrollY-y]}})()"
    )
}

/// Where an item at `bounds` is reached once the page answered [`scroll_script`] with `result`;
/// `None` when the page did not scroll.
pub(super) fn reach_after(
    bounds: Bounds,
    viewport: Bounds,
    result: &Result<String, String>,
) -> Option<Reach> {
    let [dx, dy] = serde_json::from_str::<[f64; 2]>(result.as_deref().ok()?).ok()?;
    let now = Bounds {
        x: bounds.x - dx,
        y: bounds.y - dy,
        ..bounds
    };
    Some(wubble_ax::reach(now, viewport))
}

impl Host {
    /// `tab`'s mirrored tree, while one is kept (pages' accessibility on, or an agent's tab).
    pub(super) fn ax_tree(&self, tab: TabId) -> Option<&PageTree> {
        self.ax.trees.get(&tab)
    }

    pub(super) fn set_web_ax(&mut self, setting: Calm) {
        self.ax.setting = setting;
        self.ax_follow();
    }

    /// The lifecycle tick: a screen reader may have started or stopped.
    pub(super) fn ax_tick(&mut self) {
        self.ax_follow();
    }

    fn ax_follow(&mut self) {
        let on = web_accessibility(self.ax.setting, self.ax.reader.running());
        if on == self.ax.on {
            return;
        }
        self.ax.on = on;
        self.ax.waiting = None;
        // an agent session's tabs keep theirs (`host/agent.rs`)
        let views: Vec<_> = self.views.iter().map(|(&t, &id)| (t, id)).collect();
        for (tab, id) in views {
            let wanted = on || self.agent_drives(tab);
            if !wanted {
                self.ax.trees.remove(&tab);
            }
            if let Some(view) = self.engine.view(id) {
                view.set_accessibility(wanted);
            }
        }
        self.after_change();
    }

    /// A view opened: it builds its tree from the start while pages' accessibility is on.
    pub(super) fn ax_view_opened(&mut self, tab: TabId) {
        if (self.ax.on || self.agent_drives(tab))
            && let Some(view) = self.tab_view(tab)
        {
            view.set_accessibility(true);
        }
    }

    pub(super) fn ax_view_gone(&mut self, tab: TabId) {
        self.ax.trees.remove(&tab);
    }

    /// A tree or location change of `tab`'s page; its window shows it again.
    pub(super) fn ax_event(&mut self, tab: TabId, json: &str, location: bool) {
        if !self.ax.on && !self.agent_drives(tab) {
            return;
        }
        let Some(v) = AxValue::from_json(json) else {
            return;
        };
        let tree = self.ax.trees.entry(tab).or_default();
        if location {
            tree.move_nodes(&value::parse_locations(&v));
        } else {
            tree.apply(&value::parse_updates(&v));
        }
        let here = self.w.id;
        if self.ax.on
            && let Some(w) = self.model.window_of(tab)
            && self.enter(w)
            && let Some(window) = self.w.window.upgrade()
        {
            self.render_ax(&window);
        }
        self.enter(here);
    }

    /// The page items of the window's page that gets keys, when on.
    fn ax_items(&self) -> Option<(TabId, Vec<AxItem>)> {
        let tab = self.model.window_key_tab(self.w.id)?;
        let tree = self.ax.trees.get(&tab).filter(|_| self.ax.on)?;
        Some((tab, tree.flatten(MAX_ITEMS)))
    }

    pub(super) fn render_ax(&mut self, window: &AppWindow) {
        let rows = match self.ax_items() {
            Some((tab, items)) => {
                let pane_x = self
                    .pane_rects(window)
                    .into_iter()
                    .find(|p| p.tab == tab)
                    .map_or(0.0, |p| p.x as f32);
                items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| view_of(i, item, pane_x))
                    .collect()
            }
            None => vec![],
        };
        crate::bind::sync(window.get_ax_items(), rows, |m| window.set_ax_items(m));
    }

    /// What a screen reader asked of item `index` of the window's page (`page-ax.slint`).
    pub(crate) fn ax_action(&mut self, index: i32, name: &str, text: &str) {
        let Some((tab, items)) = self.ax_items() else {
            return;
        };
        let Some(item) = usize::try_from(index).ok().and_then(|i| items.get(i)) else {
            return;
        };
        let Some(viewport) = self.ax.trees.get(&tab).and_then(PageTree::viewport) else {
            return;
        };
        match wubble_ax::reach(item.bounds, viewport) {
            Reach::At { x, y } => self.ax_act(tab, x, y, name, text),
            Reach::ScrollBy { dx, dy } => {
                let script = scroll_script(dx, dy);
                let Some(id) = self.tab_view(tab).and_then(|v| v.eval(&script)) else {
                    return;
                };
                self.ax.waiting = Some(Waiting {
                    tab,
                    eval: id.0,
                    bounds: item.bounds,
                    viewport,
                    name: name.to_owned(),
                    text: text.to_owned(),
                });
            }
        }
    }

    /// The page's answer to the scroll an action asked for; whether it was one. The action
    /// runs where the item is now, if that is in the viewport.
    pub(super) fn ax_answer(&mut self, event: &PageEvent) -> bool {
        let PageEvent::EvalResult { id, result } = event else {
            return false;
        };
        let Some(w) = self.ax.waiting.take_if(|w| w.eval == id.0) else {
            return false;
        };
        match reach_after(w.bounds, w.viewport, result) {
            Some(Reach::At { x, y }) => self.ax_act(w.tab, x, y, &w.name, &w.text),
            Some(Reach::ScrollBy { .. }) => eprintln!("wubble: ax: the item stays out of view"),
            None => eprintln!("wubble: ax: the page did not scroll: {result:?}"),
        }
        true
    }

    /// The action at `(x, y)` of `tab`'s pane (CSS px).
    fn ax_act(&mut self, tab: TabId, x: f64, y: f64, name: &str, text: &str) {
        let point = Point {
            x: x as f32,
            y: y as f32,
        };
        let pane = crate::shell::pane_id(tab);
        // a click, as the pointer would make it (it focuses the page and the field too)
        for pressed in [true, false] {
            self.pointer_button(pane, PointerEventButton::Left, pressed, point);
        }
        if name == "set-value"
            && let Some(view) = self.tab_view(tab)
        {
            view.edit(EditCommand::SelectAll);
            view.ime_commit(text);
        }
    }
}

impl Host {
    pub(super) fn ax_smoke_worked(&mut self, window: &AppWindow) {
        if self.checks.ax_smoke.is_none() {
            return;
        }
        let items: Vec<crate::ax_smoke::Item> = slint::Model::iter(&window.get_ax_items())
            .map(|i| (i.kind, i.label.to_string(), i.value.to_string()))
            .collect();
        self.ax_smoke_step(crate::ax_smoke::Input::Worked(&items));
    }

    pub(super) fn ax_smoke_step(&mut self, input: crate::ax_smoke::Input) {
        use crate::ax_smoke::{self as smoke, Action};
        let Some(state) = self.checks.ax_smoke.take() else {
            return;
        };
        let (state, actions) = smoke::next(state, input);
        self.checks.ax_smoke = Some(state);
        for action in actions {
            match action {
                Action::TurnOn => {
                    self.write_setting(wubble_core::settings::A11Y_WEB.key, "on");
                }
                Action::Act { index, name, text } => {
                    self.ax_action(i32::try_from(index).unwrap_or(-1), name, text);
                }
                Action::Pass(n) => {
                    self.ax_passed = Some(n);
                    crate::quit();
                }
                Action::Fail(reason) => self.fail(reason),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wubble_ax::Bounds;

    #[test]
    fn items_keep_their_place_and_kind() {
        let item = AxItem {
            tree: "t".into(),
            id: 9,
            kind: Kind::Link,
            role: "link".into(),
            name: "docs".into(),
            value: String::new(),
            url: String::new(),
            level: 0,
            checked: None,
            focused: false,
            bounds: Bounds {
                x: 68.0,
                y: 80.0,
                width: 29.0,
                height: 17.0,
            },
        };
        let row = view_of(3, &item, 100.0);
        assert_eq!(
            (row.id, row.kind, row.label.as_str()),
            (3, AxKind::Link, "docs")
        );
        assert_eq!((row.x, row.y, row.w, row.h), (168.0, 80.0, 29.0, 17.0));
    }

    #[test]
    fn a_scrolled_item_is_reached_where_it_moved() {
        let b = |x, y| Bounds {
            x,
            y,
            width: 20.0,
            height: 10.0,
        };
        let viewport = Bounds {
            width: 800.0,
            height: 600.0,
            ..b(0.0, 0.0)
        };
        let item = b(90.0, 1395.0);
        assert!(matches!(
            reach_after(item, viewport, &Ok("[0,1100]".into())),
            Some(Reach::At { x, y }) if x == 100.0 && y == 300.0
        ));
        assert!(matches!(
            reach_after(item, viewport, &Ok("[0,0]".into())),
            Some(Reach::ScrollBy { .. })
        ));
        assert!(reach_after(item, viewport, &Err("gone".into())).is_none());
        assert!(reach_after(item, viewport, &Ok("null".into())).is_none());
    }
}
