//! `--smoke pinch`'s host side (`pinch_smoke.rs`): the pinch goes in through [`Host::pinch`],
//! the entry point the Wayland and macOS gesture events call, one input per run of CEF work,
//! with the pointer where a hand would hold it; the pages are asked through `View::eval`.

use std::collections::BTreeMap;

use slint::LogicalPosition;
use wubble_engine::cdp::EvalId;
use wubble_engine::page::PageEvent;
use wubble_ui::AppWindow;

use crate::pinch_smoke::{self, Action as A, Drip, Input, Look};

use super::{Command, Host, new_tab};
use wubble_core::action::Action;

/// The fixture's URL on the smokes' loopback server; `prevent`: it keeps every Ctrl+wheel.
pub(crate) fn fixture(port: u16, prevent: bool) -> String {
    let query = if prevent { "?prevent" } else { "" };
    format!("http://127.0.0.1:{port}/pinch{query}")
}

/// The smoke's pending questions: eval id to (pane, asked during a gesture).
#[derive(Default)]
pub(crate) struct Asked(BTreeMap<EvalId, (usize, bool)>);

impl Host {
    fn pinch_tab(&self, pane: usize) -> Option<wubble_core::tab::TabId> {
        self.model
            .order(self.model.active_space())
            .get(pane)
            .copied()
    }

    fn pinch_look(&mut self, pane: usize, during: bool) {
        let Some(tab) = self.pinch_tab(pane) else {
            return self.fail(format!("pinch: no tab for pane {pane}"));
        };
        match self.tab_view(tab).and_then(|v| v.eval("pinchLook()")) {
            Some(id) => {
                self.pinch_asked.0.insert(id, (pane, during));
            }
            None => self.fail(format!("pinch: pane {pane} cannot be asked")),
        }
    }

    /// One run of CEF work: one pinch input fed, then the machine.
    pub(super) fn pinch_smoke_worked(&mut self, window: &AppWindow) {
        let Some(mut run) = self.checks.pinch_smoke.take() else {
            return;
        };
        if let Some((pane, drip)) = run.drips.pop_front() {
            match drip {
                Drip::Gesture(input) => self.pinch(input),
                Drip::Look => self.pinch_look(pane, true),
            }
        }
        let order = self.model.order(self.model.active_space());
        let framed = self
            .pane_rects(window)
            .iter()
            .filter(|r| self.w.frames.contains_key(&r.tab))
            .count();
        let actions = run.smoke.next(Input::Worked {
            tabs: order.len(),
            framed,
            fed: run.drips.is_empty(),
        });
        self.checks.pinch_smoke = Some(run);
        self.pinch_smoke_act(window, actions);
    }

    /// A page's answer; true when it was the smoke's.
    pub(super) fn pinch_smoke_answer(&mut self, event: &PageEvent) -> bool {
        let PageEvent::EvalResult { id, result } = event else {
            return false;
        };
        let Some((pane, during)) = self.pinch_asked.0.remove(id) else {
            return false;
        };
        let look = result.as_deref().ok().and_then(Look::parse);
        if look.is_none() {
            eprintln!("smoke: pinch: pane {pane} answered {result:?}");
        }
        let Some(mut run) = self.checks.pinch_smoke.take() else {
            return true;
        };
        let actions = run.smoke.next(Input::Looked {
            pane,
            during,
            look: look.as_ref(),
        });
        self.checks.pinch_smoke = Some(run);
        if let Some(window) = self.w.window.upgrade() {
            self.pinch_smoke_act(&window, actions);
        }
        true
    }

    fn pinch_smoke_act(&mut self, window: &AppWindow, actions: Vec<A>) {
        for action in actions {
            match action {
                A::Look(pane) => self.pinch_look(pane, false),
                A::Pinch(pane, drips) => {
                    let Some(tab) = self.pinch_tab(pane) else {
                        return self.fail(format!("pinch: no tab for pane {pane}"));
                    };
                    // The pointer over the page, as a hand rests it before pinching.
                    let (x, y) = self.view_origin(window, tab);
                    let at = pinch_smoke::HOVER_AT;
                    let pane_id = crate::shell::pane_id(tab);
                    self.pointer_moved(pane_id, at);
                    self.input.pointer = Some(LogicalPosition::new(x + at.x, y + at.y));
                    if let Some(run) = self.checks.pinch_smoke.as_mut() {
                        run.drips.extend(drips.into_iter().map(|d| (pane, d)));
                    }
                }
                A::ZoomIn => self.perform(Action::ZoomIn),
                A::ZoomReset => self.perform(Action::ZoomReset),
                A::LoadPlain => {
                    let url = self.pinch_url(false);
                    match self.pinch_tab(0).and_then(|t| self.tab_view(t)) {
                        Some(view) => view.load(&url),
                        None => self.fail("pinch: no first tab to load".into()),
                    }
                }
                A::OpenSecond => {
                    let url = self.pinch_url(false).replace("127.0.0.1", "localhost");
                    self.dispatch(new_tab(url, None));
                }
                A::Split => match self.pinch_tab(0) {
                    Some(first) => self.dispatch(Command::SplitWith(first)),
                    None => self.fail("pinch: no first tab to split with".into()),
                },
                A::Note(line) => println!("smoke: pinch: {line}"),
                A::Pass(line) => {
                    self.pinch_passed = Some(line);
                    crate::quit();
                }
                A::Fail(why) => self.fail(why),
            }
        }
    }

    /// The fixture on the server the first tab came from.
    fn pinch_url(&self, prevent: bool) -> String {
        let port = self
            .pinch_tab(0)
            .and_then(|t| self.model.tabs.get(&t))
            .and_then(|t| url_port(&t.url))
            .unwrap_or(0);
        fixture(port, prevent)
    }
}

fn url_port(url: &str) -> Option<u16> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split('/').next()?;
    host.rsplit_once(':')?.1.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_and_its_port() {
        assert_eq!(fixture(80, true), "http://127.0.0.1:80/pinch?prevent");
        assert_eq!(url_port(&fixture(4321, false)), Some(4321));
        assert_eq!(url_port("about:blank"), None);
    }
}
