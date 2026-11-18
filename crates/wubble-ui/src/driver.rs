//! Effect side of the spring-driven shell: steps the `wubble_core::shell`
//! bank once per rendered frame and writes the values into Slint properties.
//!
//! The per-frame hook is Slint's rendering notifier, copied from the idiom in
//! `examples/wgpu_texture/lib.rs` (slint-ui/slint v1.19.0, MIT): set
//! properties in `BeforeRendering`, then `request_redraw()`. A window accepts
//! only one notifier, so [`install`] is the single owner; anything else that
//! needs a per-frame callback extends the `match` there. While every spring
//! is at rest nothing requests a redraw, so an idle shell costs nothing.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use slint::{ComponentHandle, Model, SetRenderingNotifierError, Weak};
use wubble_core::shell::{self, Bank, Event, Id, Limits, Shell, index_of};

use crate::{AppWindow, Sizes};

/// The declared table: the Slint setter of each animated value, indexed by
/// [`Id`]. Targets and presets live in `shell::targets`.
const SETTERS: [fn(&AppWindow, f32); shell::N] = {
    let mut t: [fn(&AppWindow, f32); shell::N] = [|_, _| {}; shell::N];
    t[Id::SidebarWidth as usize] = |w, v| w.set_sidebar_width(v);
    t[Id::Slide as usize] = |w, v| w.set_sidebar_slide(v);
    t[Id::ContentInset as usize] = |w, v| w.set_content_inset(v);
    t[Id::SpaceIndicator as usize] = |w, v| w.set_space_indicator(v);
    t
};

impl From<shell::CompactMode> for crate::CompactMode {
    fn from(m: shell::CompactMode) -> Self {
        match m {
            shell::CompactMode::Expanded => Self::Expanded,
            shell::CompactMode::Hidden => Self::Hidden,
            shell::CompactMode::Peeking => Self::Peeking,
        }
    }
}

pub struct Driver {
    shell: Shell,
    limits: Limits,
    bank: Bank,
    /// Time of the previous frame while moving; `None` while idle, so the
    /// first frame after a wake steps with dt 0.
    last: Option<Instant>,
}

impl Driver {
    fn new(window: &AppWindow) -> Self {
        let sizes = window.global::<Sizes>();
        let limits = Limits {
            min: sizes.get_sidebar_min(),
            max: sizes.get_sidebar_max(),
        };
        let mut shell = Shell::new(window.get_sidebar_width(), limits);
        shell.space_index = space_index(window, window.get_active_space_id());
        let bank = Bank::new(&shell::targets(&shell, limits));
        let mut driver = Self {
            shell,
            limits,
            bank,
            last: None,
        };
        driver.advance(window, 0.0);
        driver
    }

    /// Apply an event: pure transition, retarget the springs, wake the frame loop.
    pub fn dispatch(&mut self, window: &AppWindow, event: Event) {
        self.shell = shell::transition(self.shell, event, self.limits);
        self.bank
            .retarget(&shell::targets(&self.shell, self.limits));
        window.set_compact_mode(self.shell.compact.into());
        window.window().request_redraw();
    }

    /// Step the springs by `dt` seconds and write the values.
    /// Returns true while another frame is needed.
    pub fn advance(&mut self, window: &AppWindow, dt: f32) -> bool {
        let moving = self.bank.step(dt);
        // Slint's `Property::set` ignores an unchanged value, so idle writes are free.
        for (set, value) in SETTERS.iter().zip(self.bank.values()) {
            set(window, value);
        }
        moving
    }

    /// One rendered frame at `now`. Returns true while another frame is
    /// needed; once idle it forgets the last frame time, so the next wake
    /// steps with dt 0.
    pub fn frame(&mut self, window: &AppWindow, now: Instant) -> bool {
        let dt = self
            .last
            .replace(now)
            .map_or(0.0, |prev| now.duration_since(prev).as_secs_f32());
        let moving = self.advance(window, dt);
        if !moving {
            self.last = None;
        }
        moving
    }

    /// Press on the seam: starts from the width on screen, not the committed one.
    fn press(&mut self, window: &AppWindow, x: f32) {
        let width = self.bank.values()[Id::SidebarWidth as usize];
        self.dispatch(window, Event::SeamPressed { x, width });
    }
}

fn space_index(window: &AppWindow, id: i32) -> usize {
    index_of(window.get_spaces().iter().map(|s| s.id), id)
}

/// Connect the Slint callbacks to a new driver. Separate from [`install`] so
/// tests can drive it without a renderer.
pub fn connect(window: &AppWindow) -> Rc<RefCell<Driver>> {
    let driver = Rc::new(RefCell::new(Driver::new(window)));
    let fixed = |event: Event| {
        let (driver, weak) = (Rc::clone(&driver), window.as_weak());
        move || dispatch(&driver, &weak, event)
    };
    let (d, weak) = (Rc::clone(&driver), window.as_weak());
    window.on_seam_pressed(move |x| {
        if let Some(window) = weak.upgrade() {
            d.borrow_mut().press(&window, x);
        }
    });
    let (d, weak) = (Rc::clone(&driver), window.as_weak());
    window.on_seam_dragged(move |x| dispatch(&d, &weak, Event::SeamDragged { x }));
    window.on_seam_released(fixed(Event::SeamReleased));
    window.on_compact_toggle_requested(fixed(Event::ToggleCompact));
    let (d, weak) = (Rc::clone(&driver), window.as_weak());
    window.on_zone_hover_changed(move |hover| {
        let event = if hover {
            Event::ZoneEntered
        } else {
            Event::ZoneLeft
        };
        dispatch(&d, &weak, event);
    });
    let (d, weak) = (Rc::clone(&driver), window.as_weak());
    window.on_space_changed(move |id| {
        if let Some(window) = weak.upgrade() {
            let index = space_index(&window, id);
            dispatch(&d, &weak, Event::SpaceChanged(index));
        }
    });
    driver
}

fn dispatch(driver: &RefCell<Driver>, window: &Weak<AppWindow>, event: Event) {
    if let Some(window) = window.upgrade() {
        driver.borrow_mut().dispatch(&window, event);
    }
}

/// Wire the shell driver and register the window's one rendering notifier.
pub fn install(window: &AppWindow) -> Result<(), SetRenderingNotifierError> {
    let driver = connect(window);
    let weak = window.as_weak();
    window.window().set_rendering_notifier(move |state, _api| {
        if let slint::RenderingState::BeforeRendering = state
            && let Some(window) = weak.upgrade()
            && driver.borrow_mut().frame(&window, Instant::now())
        {
            window.window().request_redraw();
        }
    })
}
