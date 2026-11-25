// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! wubble change (5) (new file): how a window's wgpu 30 surface presents, asked of the
//! application before each frame. Upstream always presents `AutoVsync` (FIFO). wubble's Wayland
//! page layer needs Mailbox while a page's subsurface sits under the window: FIFO with a synced
//! subsurface stalled mutter's headless display (wubble docs/research/linux-page-layer.md).
//! Mailbox presents as often as it is asked, so the surface paces it itself: a frame less than
//! one refresh period after the last present waits out the rest, as FIFO's acquire would. One
//! hook per thread (the UI thread renders).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use i_slint_core::api::Window;
use wgpu_30 as wgpu;

/// How a window's frames are presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentMode {
    /// Upstream's `AutoVsync`.
    Vsync,
    /// Mailbox where the surface has it, at most one present per `period` (the display's).
    Paced(Duration),
}

thread_local! {
    static HOOK: RefCell<Option<Box<dyn Fn(&Window) -> Option<PresentMode>>>> =
        const { RefCell::new(None) };
}

thread_local! {
    /// The present modes the last surface made on this thread offers (its swapchain's).
    static OFFERED: RefCell<Option<Vec<wgpu::PresentMode>>> = const { RefCell::new(None) };
}

/// A surface on this thread offers `modes`.
pub(crate) fn offered(modes: &[wgpu::PresentMode]) {
    OFFERED.with(|o| *o.borrow_mut() = Some(modes.to_vec()));
}

/// What the surface offers and which mode each [`PresentMode`] takes there (`AutoVsync` resolved
/// as wgpu does: FifoRelaxed where offered, else Fifo), for a log line; `None` before a surface.
pub fn summary() -> Option<String> {
    use wgpu::PresentMode::{AutoVsync, Fifo, FifoRelaxed};
    let offers = OFFERED.with(|o| o.borrow().clone())?;
    let taken = |mode| match wgpu_mode(mode, &offers) {
        AutoVsync if offers.contains(&FifoRelaxed) => FifoRelaxed,
        AutoVsync => Fifo,
        other => other,
    };
    let paced = PresentMode::Paced(Duration::ZERO);
    Some(format!(
        "present modes offered {offers:?}; vsync takes {:?}, paced takes {:?}",
        taken(PresentMode::Vsync),
        taken(paced)
    ))
}

/// `hook` says, before each frame of a window on this thread, how it presents; `None`: as the
/// frame before (the application could not tell now).
pub fn set_hook(hook: impl Fn(&Window) -> Option<PresentMode> + 'static) {
    HOOK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
}

/// The application's answer for `window`, `None` without a hook or an answer.
pub(crate) fn asked(window: &Window) -> Option<PresentMode> {
    HOOK.with(|h| h.borrow().as_ref().and_then(|hook| hook(window)))
}

/// How long a paced frame waits before it is presented: what is left of `period` since the
/// last present at `last` (nothing for the first, or once the period has passed).
pub(crate) fn wait(last: Option<Instant>, period: Duration, now: Instant) -> Duration {
    last.map_or(Duration::ZERO, |last| period.saturating_sub(now.saturating_duration_since(last)))
}

/// The wgpu mode for `mode` on a surface that `offers` these.
pub(crate) fn wgpu_mode(mode: PresentMode, offers: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    match mode {
        PresentMode::Paced(_) if offers.contains(&wgpu::PresentMode::Mailbox) => {
            wgpu::PresentMode::Mailbox
        }
        _ => wgpu::PresentMode::AutoVsync,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paced_frame_waits_out_the_period() {
        let t = Instant::now();
        let ms = Duration::from_millis;
        let rows = [
            ("the first frame", None, ms(10), t, ms(0)),
            ("right after the last", Some(t), ms(10), t, ms(10)),
            ("part way", Some(t), ms(10), t + ms(4), ms(6)),
            ("a period on", Some(t), ms(10), t + ms(10), ms(0)),
            ("long after", Some(t), ms(10), t + ms(50), ms(0)),
            ("a clock behind the last present", Some(t + ms(5)), ms(10), t, ms(10)),
        ];
        for (name, last, period, now, want) in rows {
            assert_eq!(wait(last, period, now), want, "{name}");
        }
    }

    #[test]
    fn mailbox_only_where_offered() {
        use wgpu::PresentMode::{AutoVsync, Fifo, Mailbox};
        let paced = PresentMode::Paced(Duration::from_millis(3));
        let rows: [(&str, PresentMode, &[wgpu::PresentMode], wgpu::PresentMode); 3] = [
            ("paced, offered", paced, &[Fifo, Mailbox], Mailbox),
            ("paced, not offered: vsync", paced, &[Fifo], AutoVsync),
            ("vsync", PresentMode::Vsync, &[Fifo, Mailbox], AutoVsync),
        ];
        for (name, mode, offers, want) in rows {
            assert_eq!(wgpu_mode(mode, offers), want, "{name}");
        }
    }
}
