//! The chrome's look that follows the user and the OS: reduced motion ("Calmer motion": the
//! setting, on auto the OS preference) and touch sizing (44 px rows under a finger: the setting,
//! on auto the input kind). Each is one flag with one writer here, pushed to every window's
//! Slint globals (`Calm.reduced`, `Shell.touch`) and, for motion, to `wubble_core::motion`.

use slint::ComponentHandle;
use wubble_core::motion::{self, Calm};
use wubble_core::touch::{self, InputEvent, InputKind, Sizing};
use wubble_ui::AppWindow;

use super::Host;
use crate::os_motion::{self, MotionSource};

/// What the look is made of.
pub(super) struct Look {
    calm: Calm,
    os: Box<dyn MotionSource>,
    sizing: Sizing,
    kind: InputKind,
    /// What every window was last given: (reduced, touch).
    pushed: Option<(bool, bool)>,
}

impl Look {
    pub fn new() -> Self {
        Self {
            calm: Calm::Auto,
            os: os_motion::source(),
            sizing: Sizing::Auto,
            kind: InputKind::Mouse,
            pushed: None,
        }
    }

    /// (reduced motion, touch sizing) as they stand.
    fn now(&self) -> (bool, bool) {
        (
            self.calm.reduced(self.os.reduced()),
            touch::is_touch(self.sizing, self.kind),
        )
    }
}

/// Writes the look into one window's globals.
pub(super) fn paint(handle: &AppWindow, (reduced, touch): (bool, bool)) {
    handle.global::<wubble_ui::Calm>().set_reduced(reduced);
    handle.global::<wubble_ui::Shell>().set_touch(touch);
}

impl Host {
    pub(super) fn set_calm(&mut self, calm: Calm) {
        self.look.calm = calm;
        self.push_look();
    }

    pub(super) fn set_touch_sizing(&mut self, sizing: Sizing) {
        self.look.sizing = sizing;
        self.push_look();
    }

    /// A pointer event from a window (`window_event`): a touch flips to touch sizing, a real
    /// mouse move back.
    pub(super) fn input_kind(&mut self, event: InputEvent) {
        let kind = touch::next(self.look.kind, event);
        if kind != self.look.kind {
            self.look.kind = kind;
            self.push_look();
        }
    }

    /// The lifecycle tick: the OS preference may have changed.
    pub(super) fn look_tick(&mut self) {
        self.push_look();
    }

    /// The look as it stands now, for a window that opens.
    pub(super) fn look(&self) -> (bool, bool) {
        self.look.now()
    }

    /// Pushes the look to the motion flag and every window, when it changed.
    fn push_look(&mut self) {
        let now = self.look.now();
        if self.look.pushed == Some(now) {
            return;
        }
        self.look.pushed = Some(now);
        motion::set_reduced(now.0);
        paint(&self.w.handle, now);
        for w in self.parked.values() {
            paint(&w.handle, now);
        }
    }
}
