//! `--measure memory` on the host: what [`crate::memory::next`] sees and its actions run
//! through the same commands the browser uses (new tabs, the lifecycle tick).

use std::time::Instant;

use slint::ComponentHandle;

use wubble_core::busy::Reason;
use wubble_core::model::Command;
use wubble_core::sleep::Pressure;
use wubble_core::tab::Liveness;

use super::{Host, new_tab, now_ms};
use crate::memory::{self, Action, Seen};

impl Host {
    /// One measure tick of the memory run (nothing without one).
    pub fn memory_tick(&mut self) {
        if !memory::active() {
            return;
        }
        let want = crate::nav_smoke::title_of(crate::ARTICLE_PAGE).unwrap_or_default();
        let tabs = self.model.tabs.values();
        let seen = Seen {
            tabs: self.model.tabs.len(),
            loaded: tabs
                .clone()
                .filter(|t| t.title == want && !t.busy.contains(Reason::Loading))
                .count(),
            frozen: tabs
                .filter(|t| t.lifecycle.liveness == Liveness::Frozen)
                .count(),
            views: self.engine.view_count(),
            blocker_busy: self.blocking.is_busy(),
        };
        for action in memory::step(&seen) {
            match action {
                Action::Open(n) => {
                    let first = self.model.tabs.len();
                    for i in first..first + n {
                        if let Some(url) = memory::url(i) {
                            self.dispatch(new_tab(url, None));
                        }
                    }
                }
                Action::Sample { state, ready_after } => {
                    let window = self.w.window.upgrade().map(|w| {
                        let (size, scale) = (w.window().size(), w.window().scale_factor());
                        format!("{}x{}@{scale}", size.width, size.height)
                    });
                    let window = window.unwrap_or_default();
                    let ready_ms = crate::measure::TICK.as_millis() * u128::from(ready_after);
                    memory::sample(state, ready_ms, &seen, &self.tasks.renderer_pids(), &window);
                }
                // The policy's own transitions at its own delays; the pressure stays out of it.
                Action::Tick(sleep) => {
                    let ahead = memory::ahead(&self.model.policy(), sleep);
                    self.dispatch(Command::Tick {
                        now: Instant::now() + ahead,
                        wall_ms: now_ms() + i64::try_from(ahead.as_millis()).unwrap_or(0),
                        pressure: Pressure::None,
                    });
                }
                Action::Finish => crate::quit(),
                Action::Fail(why) => self.fail(why),
            }
        }
    }
}
