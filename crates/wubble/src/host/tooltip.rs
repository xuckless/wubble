//! Tooltips, host side: a page's `title` (`PageEvent::Tooltip`) near the pointer and the shell's
//! icon buttons' names and shortcuts (`Tips.hover`), through one `hovercard` machine with the
//! design's tooltip timing (`TOOLTIP`). The host is the one writer of the window's `TipView`.

use wubble_core::action::Action;
use wubble_core::hovercard::{self, Effect, Event, Hover, TOOLTIP};
use wubble_core::tab::TabId;
use wubble_ui::TipView;

use super::Host;
use super::hover::HoverClock;

/// What a tooltip is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TipKey {
    /// The page in this tab's `title` under the pointer.
    Page(TabId),
    /// A shell button that runs this action.
    Shell(Action),
}

/// Below and right of the pointer, where Chromium puts a page's tooltip (logical px).
const POINTER_OFFSET: (f32, f32) = (12.0, 20.0);

pub(super) struct Tips {
    clock: HoverClock<TipKey>,
    /// The page's tooltip text, per tab that has one.
    page: Option<(TabId, String)>,
    /// The shell button hovered last and where its tooltip hangs (window px: centre, top,
    /// bottom).
    button: Option<(Action, f32, (f32, f32))>,
    /// The pointer over a page: (tab, pane px), where its tooltip hangs.
    pointer: Option<(TabId, f32, f32)>,
}

impl Tips {
    pub fn new() -> Self {
        Self {
            clock: HoverClock::new(TOOLTIP),
            page: None,
            button: None,
            pointer: None,
        }
    }
}

impl Host {
    /// The pointer moved over `tab`'s page, at this point in its pane.
    pub(super) fn tip_pointer(&mut self, tab: TabId, x: f32, y: f32) {
        self.tips.pointer = Some((tab, x, y));
    }

    /// The page in `tab` shows a tooltip under the pointer (`text`), or none (empty).
    pub(super) fn page_tooltip(&mut self, tab: TabId, text: String) {
        if text.is_empty() {
            self.tips.page = None;
            self.tip_leave(TipKey::Page(tab));
        } else {
            let changed = self.tips.page.as_ref().is_none_or(|(_, t)| *t != text);
            self.tips.page = Some((tab, text));
            self.tip_enter(TipKey::Page(tab), changed);
        }
    }

    /// A shell button's hover (`Tips.hover` in tooltip.slint): its action id and where its
    /// tooltip hangs (`ys`: the button's top and bottom).
    pub(crate) fn shell_tip(&mut self, id: &str, x: f32, ys: (f32, f32), on: bool) {
        let Some(action) = Action::from_id(id) else {
            return;
        };
        if on {
            self.tips.button = Some((action, x, ys));
            self.tip_enter(TipKey::Shell(action), false);
        } else {
            self.tip_leave(TipKey::Shell(action));
        }
    }

    /// The text of a page's tooltip, while one shows.
    pub(super) fn page_tip_shown(&self) -> Option<String> {
        match hovercard::shown(self.tips.clock.state()) {
            Some(TipKey::Page(tab)) => self
                .tips
                .page
                .as_ref()
                .filter(|(t, _)| *t == tab)
                .map(|(_, text)| text.clone()),
            _ => None,
        }
    }

    /// A press, a key or the window losing focus: no tooltip.
    pub(super) fn tip_dismiss(&mut self) {
        if self.tips.clock.state() != Hover::Idle {
            self.tip_step(Event::Dismiss);
        }
    }

    /// `redraw`: the shown tooltip's words changed while its key did not.
    fn tip_enter(&mut self, key: TipKey, redraw: bool) {
        let now = self.tips.clock.now();
        let shown = hovercard::shown(self.tips.clock.state()) == Some(key);
        self.tip_step(Event::Enter { key, now });
        if redraw && shown {
            self.render_tip(Some(key), false);
        }
    }

    fn tip_leave(&mut self, key: TipKey) {
        self.tips.clock.leave_if(key, Self::tip_tick);
    }

    fn tip_tick(&mut self) {
        let now = self.tips.clock.now();
        self.tip_step(Event::Tick { now });
    }

    fn tip_step(&mut self, event: Event<TipKey>) {
        let (before, effect) = self.tips.clock.step(event, Self::tip_tick);
        match effect {
            // From rest it appears in place; from another tooltip it glides.
            Some(Effect::Show(key)) => {
                let warm =
                    hovercard::shown(before).is_some() || matches!(before, Hover::Cooling { .. });
                self.render_tip(Some(key), !warm);
            }
            Some(Effect::Hide) => self.render_tip(None, false),
            None => {}
        }
    }

    /// Draws the tooltip for `key`, or fades it out (it keeps its words while it does).
    fn render_tip(&mut self, key: Option<TipKey>, jump: bool) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let mut view: TipView = window.get_tip();
        let content = match key {
            Some(TipKey::Page(tab)) => self
                .tips
                .page
                .as_ref()
                .filter(|(t, _)| *t == tab)
                .zip(self.tips.pointer.filter(|(t, ..)| *t == tab))
                .map(|((_, text), (_, x, y))| {
                    let (ox, oy) = self.view_origin(&window, tab);
                    let (x, y) = (ox + x, oy + y);
                    let at = (x + POINTER_OFFSET.0, y + POINTER_OFFSET.1, y, false);
                    (text.clone(), String::new(), at)
                }),
            Some(TipKey::Shell(action)) => {
                self.tips
                    .button
                    .filter(|(a, ..)| *a == action)
                    .map(|(_, x, (top, bottom))| {
                        let (name, keys) = self.keymap.tooltip_text(action);
                        (name, keys, (x, bottom, top, true))
                    })
            }
            None => None,
        };
        match content {
            Some((text, shortcut, (x, y, top, centered))) => {
                view = TipView {
                    shown: true,
                    text: text.into(),
                    shortcut: shortcut.into(),
                    anchor_x: x,
                    anchor_y: y,
                    anchor_top: top,
                    centered,
                    jump,
                };
            }
            None => view.shown = false,
        }
        window.set_tip(view);
    }
}
