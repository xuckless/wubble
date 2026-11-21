// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! wubble change (new file): partial rendering on the wgpu 30 surface, its switch and its counts.
//!
//! The wgpu surface keeps the frame Skia draws into between frames (see `WGPUSurface::render`),
//! so a frame redraws only the region the partial renderer found dirty and is then copied to
//! the swapchain image. `SLINT_SKIA_PARTIAL_RENDERING` selects the mode: unset (or any value
//! but these) on; `off` draws every frame whole into the swapchain image as upstream does;
//! `verify` is on and also re-renders every frame whole and compares it with what was
//! presented (a readback per frame: for checks only).

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The partial rendering mode, read once from `SLINT_SKIA_PARTIAL_RENDERING`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    On,
    Off,
    Verify,
}

pub fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("SLINT_SKIA_PARTIAL_RENDERING").as_deref() {
        Ok("off") => Mode::Off,
        Ok("verify") => Mode::Verify,
        _ => Mode::On,
    })
}

pub fn enabled() -> bool {
    mode() != Mode::Off
}

/// One rendered frame: the share of the window it redrew and how many items it drew.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// The dirty region's area over the window's, 0 to 1 (1 for a whole frame).
    pub dirty_share: f32,
    /// Items whose draw call reached Skia (rectangles, images, text, paths, shadows).
    pub items_drawn: u32,
}

/// Every frame rendered in the process so far (`--measure` prints them).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Totals {
    pub frames: u64,
    /// Frames that redrew less than the whole window.
    pub partial: u64,
    /// The sum of [`Frame::dirty_share`], in millionths.
    pub dirty_share_millionths: u64,
    pub items_drawn: u64,
    /// `verify` only: frames whose presented pixels differed from a whole redraw by more than 2.
    pub verify_mismatches: u64,
}

static FRAMES: AtomicU64 = AtomicU64::new(0);
static PARTIAL: AtomicU64 = AtomicU64::new(0);
static DIRTY: AtomicU64 = AtomicU64::new(0);
static ITEMS: AtomicU64 = AtomicU64::new(0);
static MISMATCHES: AtomicU64 = AtomicU64::new(0);

pub fn totals() -> Totals {
    Totals {
        frames: FRAMES.load(Ordering::Relaxed),
        partial: PARTIAL.load(Ordering::Relaxed),
        dirty_share_millionths: DIRTY.load(Ordering::Relaxed),
        items_drawn: ITEMS.load(Ordering::Relaxed),
        verify_mismatches: MISMATCHES.load(Ordering::Relaxed),
    }
}

pub(crate) fn record(frame: Frame) {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    if frame.dirty_share < 1.0 {
        PARTIAL.fetch_add(1, Ordering::Relaxed);
    }
    DIRTY.fetch_add((f64::from(frame.dirty_share) * 1e6) as u64, Ordering::Relaxed);
    ITEMS.fetch_add(u64::from(frame.items_drawn), Ordering::Relaxed);
}

/// Counts a mismatch; returns how many there were before it.
pub(crate) fn record_mismatch() -> u64 {
    MISMATCHES.fetch_add(1, Ordering::Relaxed)
}

/// `verify` with `SLINT_SKIA_VERIFY_DIR=DIR`: the first ten mismatching frames as raw RGBA
/// (premultiplied) files, `N-presented.rgba` and `N-whole.rgba`, with `N.size` holding
/// "width height".
pub(crate) fn dump(n: u64, width: u32, height: u32, presented: &[u8], whole: &[u8]) {
    let Some(dir) = std::env::var_os("SLINT_SKIA_VERIFY_DIR") else { return };
    if n >= 10 {
        return;
    }
    let dir = std::path::PathBuf::from(dir);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{n}.size")), format!("{width} {height}"));
    let _ = std::fs::write(dir.join(format!("{n}-presented.rgba")), presented);
    let _ = std::fs::write(dir.join(format!("{n}-whole.rgba")), whole);
}

thread_local! {
    static DRAWN: Cell<u32> = const { Cell::new(0) };
}

/// One item's draw call reached Skia.
pub(crate) fn drawn() {
    DRAWN.with(|d| d.set(d.get().saturating_add(1)));
}

/// The draws counted on this thread since the last call.
pub(crate) fn take_drawn() -> u32 {
    DRAWN.with(|d| d.replace(0))
}

/// Pixels of two RGBA8 frames of one size whose channels differ by more than 2, and their
/// bounding box (x0, y0, x1, y1), for `verify`.
pub(crate) fn compare(
    a: &[u8],
    b: &[u8],
    width: usize,
) -> (usize, Option<(usize, usize, usize, usize)>) {
    let mut n = 0;
    let mut bbox: Option<(usize, usize, usize, usize)> = None;
    for (i, (pa, pb)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if pa.iter().zip(pb).any(|(x, y)| x.abs_diff(*y) > 2) {
            n += 1;
            let (x, y) = (i % width, i / width);
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    (n, bbox)
}
