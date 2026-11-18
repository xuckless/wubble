//! The sidebar's two-finger horizontal swipe switches the Space (`wubble_core::touch::SpaceSwipe`,
//! the rule; written new: the design bucket has no Space swipe). Linux and macOS touchpads
//! reach Slint as horizontal scroll deltas; winit's `PanGesture` (macOS only) is not used.

use std::time::Instant;

use wubble_core::model::Command;
use wubble_core::touch::SpaceSwipe;

use super::Host;

/// The gesture's state and the clock its times count from (the rule takes ms as numbers).
pub struct Swipe {
    state: SpaceSwipe,
    since: Instant,
}

impl Default for Swipe {
    fn default() -> Self {
        Self {
            state: SpaceSwipe::default(),
            since: Instant::now(),
        }
    }
}

impl Host {
    /// A scroll over the sidebar (px, web sign; see [`SpaceSwipe::scroll`]).
    pub fn space_swipe(&mut self, swipe: &mut Swipe, dx: f32, dy: f32) {
        let now = swipe.since.elapsed().as_secs_f64() * 1000.0;
        let ids: Vec<_> = self.model.spaces.iter().map(|s| s.id).collect();
        let Some(active) = ids.iter().position(|&s| s == self.model.active_space()) else {
            return;
        };
        if let Some(to) = swipe
            .state
            .scroll(f64::from(dx), f64::from(dy), now, active, ids.len())
        {
            self.dispatch(Command::SwitchSpace(ids[to]));
        }
    }
}
