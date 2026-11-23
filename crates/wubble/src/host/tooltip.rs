//! Tooltips, host side: a page's `title` (`PageEvent::Tooltip`) near the pointer and the shell's
//! icon buttons' names and shortcuts (`Tips.hover`), through the window's tooltip slot of the
//! store (`overlay::Kind::Tooltip`, the design's tooltip timing). The host is the one writer
//! of the window's `TipView`.

use wubble_core::action::Action;
use wubble_core::overlay::{Cue, Key, Kind, Surface};
use wubble_core::tab::TabId;
use wubble_ui::TipView;

use super::Host;

/// Below and right of the pointer, where Chromium puts a page's tooltip (logical px).
const POINTER_OFFSET: (f32, f32) = (12.0, 20.0);

pub(super) struct Tips {
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
        // none while a page drag is on: Chromium's tooltip timer can still fire after the drag
        // began (from the hover that preceded it), and Chrome shows no tooltip mid-drag
        if text.is_empty() || self.drag.page_dragging().is_some() {
            self.tips.page = None;
            self.hover(Kind::Tooltip, Key::Page(tab), Cue::Leave);
        } else {
            let changed = self.tips.page.as_ref().is_none_or(|(_, t)| *t != text);
            self.tips.page = Some((tab, text));
            self.tip_enter(Key::Page(tab), changed);
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
            self.tip_enter(Key::Shell(action), false);
        } else {
            self.hover(Kind::Tooltip, Key::Shell(action), Cue::Leave);
        }
    }

    fn tip_slot(&self) -> Surface {
        Surface {
            w: self.w.id,
            kind: Kind::Tooltip,
        }
    }

    /// The text of a page's tooltip, while one shows.
    pub(super) fn page_tip_shown(&self) -> Option<String> {
        match self.ui.shown(self.tip_slot()) {
            Some(Key::Page(tab)) => self
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
        self.hover(Kind::Tooltip, Key::None, Cue::Dismiss);
    }

    /// `redraw`: the shown tooltip's words changed while its key did not.
    fn tip_enter(&mut self, key: Key, redraw: bool) {
        let shown = self.ui.shown(self.tip_slot()) == Some(key);
        self.hover(Kind::Tooltip, key, Cue::Enter);
        if redraw && shown {
            self.render_tip(Some(key), false);
        }
    }

    /// Draws the tooltip for `key`, or fades it out (it keeps its words while it does).
    pub(super) fn render_tip(&mut self, key: Option<Key>, jump: bool) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let mut view: TipView = window.get_tip();
        let content = match key {
            Some(Key::Page(tab)) => self
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
            Some(Key::Shell(action)) => {
                self.tips
                    .button
                    .filter(|(a, ..)| *a == action)
                    .map(|(_, x, (top, bottom))| {
                        let (name, keys) = self.keymap.tooltip_text(action);
                        (name, keys, (x, bottom, top, true))
                    })
            }
            Some(Key::None | Key::Tab(_) | Key::Notice(_)) | None => None,
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
