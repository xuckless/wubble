//! The page on its own OS layer (macOS stage 1; decisions.md "Page layer"): a pane
//! whose frames go to a layer of the OS compositor (a `CALayer` whose contents is the frame's
//! IOSurface on macOS, a Wayland `wl_subsurface` showing it as a dmabuf on Linux) instead of
//! through the shell's presenter and a Slint render. A page-only frame then costs no shell
//! render at all.
//!
//! One [`Route`] per pane and one pure [`transition`]. What keeps the page on the shell's path
//! ([`Needs`]) is not stored here: the host derives it each time from the owners of that state
//! (the screen's fullscreen, the picture-in-picture window, the overlays, the pane's stretch, the
//! snapshots) and passes it in, so it has no second copy.
//!
//! Each mode is a row of [`MODES`]: where its layer sits ([`Order`]) and what each need does to a
//! page on it ([`Hold`]). Stage 1 (`fullscreen`) has no hole in the shell: the layer sits over the
//! shell's whole window content (page fullscreen, the picture-in-picture window) and anything the
//! shell must draw over the page ([`Need::Covered`]) sends the page back to the shell. Stage 2
//! (`window`) puts the layer under the shell, which clears a hole where the pane shows
//! ([`Route::holed`]): every pane may use one, the shell's UI stays drawn over it, glass over it
//! blurs a slowly refreshed snapshot of the layer ([`Hold::Frost`], [`Effect::Snapshot`]), and
//! only what needs the page's live pixels in the shell (a resize's stretch, an entrance, a shot)
//! holds the page on the shell, briefly ([`Hold::Brief`]: the OS buffers stay for its return, for
//! at most [`BRIEF_MS`], after which the hold is a long one: it never costs more than no layer).
//!
//! Time is the host's clock in ms (`now`), read once per event at the edge; the deadlines it is
//! compared with are carried in the route.
//!
//! The invariant (tests walk it): the layer is visible exactly while the route is [`Route::Layer`]
//! or [`Route::Leaving`], so it is visible whenever the shell does not draw the page, and it is
//! never left over the shell once the shell draws the page again. The layer is hidden only once
//! the shell presents a render that drew the page's frame at least as new as the layer's
//! ([`Event::Presenting`]): a generation (CEF's paint count), not a guessed number of renders.

use crate::overlay::Coverage;

/// Where a pane's frames go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Route {
    /// No layer for this pane (another platform, the mode is off, or the layer failed): the
    /// shell draws every frame.
    #[default]
    Unsupported,
    /// The shell draws the page (today's path); the layer is hidden and the frames are copied
    /// into textures of the shell's own.
    Composited,
    /// The OS buffers were asked for; the shell draws the page until a frame in one comes while
    /// nothing holds it there. A brief hold keeps the page here, buffers and all, until `expires`
    /// (ms, set as the hold begins here; `None` while nothing holds it briefly).
    Entering { expires: Option<u64> },
    /// The layer shows the page's frame of generation `shown`; the shell's presenter and render
    /// are skipped for it. `snap`: the snapshot the shell's glass over it blurs, while glass is
    /// over it ([`Hold::Frost`]).
    Layer { shown: u64, snap: Option<Snap> },
    /// The shell was given the page's frame of generation `until` (or the layer's own, handed
    /// over); the layer stays visible until the shell presents a render that drew it.
    Leaving { until: u64 },
}

/// The layer's frame last copied for the shell's glass ([`Effect::Snapshot`]): its generation
/// and when (ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snap {
    pub painted: u64,
    pub at: u64,
}

/// How often glass over a page on its layer sees the page again, at most (ms): a few times a
/// second, low resolution through the glass's own blur. wubble's own value, judged by the
/// owner's eye (docs/research/page-layer.md, "Overlays over a layered page").
pub const SNAPSHOT_MS: u64 = 250;

/// How long a brief hold keeps the OS buffers asked for while the shell draws the page (ms); past
/// it the hold is a long one ([`Route::hold`]).
pub const BRIEF_MS: u64 = 1_000;

impl Route {
    /// `hold` as it stands for this route at `now`: a brief hold kept past its deadline is a long
    /// one, so the OS buffers go at the next event.
    pub fn hold(self, hold: Hold, now: u64) -> Hold {
        match self {
            Route::Entering { expires: Some(at) } if hold == Hold::Brief && now >= at => Hold::Long,
            _ => hold,
        }
    }

    /// The layer is visible (the invariant): the shell's drawable must land with its changes
    /// (`presentsWithTransaction` on macOS).
    pub fn on_layer(self) -> bool {
        matches!(self, Route::Layer { .. } | Route::Leaving { .. })
    }

    /// The shell draws the pane's hole: the layer is under the shell in `mode` and shows the page
    /// the shell does not (`Leaving` handed its frame to the shell, drawn over the layer). Within
    /// [`Route::on_layer`], so the layer is visible wherever a hole is drawn.
    pub fn holed(self, mode: Mode) -> bool {
        mode.row().order == Order::Below && matches!(self, Route::Layer { .. })
    }

    /// The route without its generation: a change of name is a change of route (the host's log).
    pub fn name(self) -> &'static str {
        match self {
            Route::Unsupported => "Unsupported",
            Route::Composited => "Composited",
            Route::Entering { .. } => "Entering",
            Route::Layer { .. } => "Layer",
            Route::Leaving { .. } => "Leaving",
        }
    }
}

/// A reason the shell must draw the page itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Need {
    /// The page is a pane among the shell's chrome, not the window's whole content (not page
    /// fullscreen, not the picture-in-picture window): stage 1 has no hole to show a layer
    /// through.
    Windowed,
    /// Shell UI over the page, a leaving one included: `overlay::covers` is not
    /// [`Coverage::None`] ([`Needs::covered`]). The layer would cover it.
    Covered,
    /// The pane's size is changing: the shell stretches its stale frame (`crate::stretch`).
    Resizing,
    /// Something wants the page's pixels in the shell's own textures: a snapshot, a shot.
    Pixels,
    /// The shell draws the page's own pixels under its UI: an entrance moving it, a mask cutting
    /// it (`overlay::covers` is [`Coverage::Pixels`]; [`Needs::covered`]).
    Sampled,
    /// The tab has no pane of its own in the shell (the peek's page, an extension's popup), or its
    /// pane is a built-in page the shell draws (Settings, a start page, alone or in a split): there
    /// is no place for a layer.
    Unplaced,
    /// Glass over the page blurs it (`overlay::covers` is [`Coverage::Glass`]): a snapshot of the
    /// page will do.
    Frosted,
}

/// How many [`Need`]s there are (a [`ModeRow`] has a hold for each).
pub const NEEDS: usize = Need::Frosted as usize + 1;

impl Need {
    /// Every need, in bit order.
    pub const ALL: [Need; NEEDS] = [
        Need::Windowed,
        Need::Covered,
        Need::Resizing,
        Need::Pixels,
        Need::Sampled,
        Need::Unplaced,
        Need::Frosted,
    ];
}

/// What a need does to a page on its layer, in a mode. Ordered: the strongest need present wins
/// ([`Needs::hold`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Hold {
    /// Nothing: the page may stay on its layer.
    Free,
    /// The page stays on its layer; the shell's glass over it blurs the layer's frame, copied to
    /// the shell at most every [`SNAPSHOT_MS`] ([`Effect::Snapshot`]).
    Frost,
    /// The shell draws the page for a while (a resize, an entrance, a shot); the OS buffers stay
    /// asked for, and the page goes back to its layer at its first frame after, unless the hold
    /// lasts [`BRIEF_MS`]: then it is a long one.
    Brief,
    /// The shell draws the page, and the OS buffers go.
    Long,
}

/// The set of [`Need`]s that hold now. Any of them keeps the page on the shell's path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Needs(u8);

impl Needs {
    pub const NONE: Needs = Needs(0);

    /// This set with `need` in it (`on`) or not.
    pub fn with(self, need: Need, on: bool) -> Needs {
        let bit = 1 << need as u8;
        Needs(if on { self.0 | bit } else { self.0 & !bit })
    }

    /// This set with [`Need::Covered`], [`Need::Frosted`] and [`Need::Sampled`] as the pane's
    /// `coverage` says.
    pub fn covered(self, coverage: Coverage) -> Needs {
        self.with(Need::Covered, coverage != Coverage::None)
            .with(Need::Frosted, coverage == Coverage::Glass)
            .with(Need::Sampled, coverage == Coverage::Pixels)
    }

    pub fn has(self, need: Need) -> bool {
        self.0 & (1 << need as u8) != 0
    }

    /// Whether the shell must draw the page.
    pub fn any(self) -> bool {
        self.0 != 0
    }

    /// The needs as the owners' facts say now; the host reads them on each call and keeps none.
    pub fn from_owners(o: &Owners) -> Needs {
        Needs::from_place(o.page_fullscreen || o.pip_window, o.placed)
            .covered(o.coverage)
            .with(Need::Resizing, !o.view_live)
            .with(Need::Pixels, o.pixels)
    }

    /// The needs of where the page is: the window's whole content (`whole`) or a pane among the
    /// chrome, and whether it has a pane of its own (`placed`). All the host reads before a pane
    /// has a layer ([`layer_unused`]).
    pub fn from_place(whole: bool, placed: bool) -> Needs {
        Needs::NONE
            .with(Need::Windowed, !whole)
            .with(Need::Unplaced, !placed)
    }

    /// The strongest hold of these needs in `mode` ([`Hold::Free`] for none).
    pub fn hold(self, mode: Mode) -> Hold {
        let holds = &mode.row().holds;
        (0..NEEDS)
            .filter(|&n| self.0 & (1 << n) != 0)
            .map(|n| holds[n])
            .max()
            .unwrap_or(Hold::Free)
    }
}

/// The needs that say where the page is ([`layer_unused`]).
const PLACE: Needs = Needs(1 << Need::Windowed as u8 | 1 << Need::Unplaced as u8);

/// What the needs are derived from, each read from its one owner by the host on each call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owners {
    /// The pane's tab is its window's page fullscreen (`Screen::Page`).
    pub page_fullscreen: bool,
    /// The window is the picture-in-picture window.
    pub pip_window: bool,
    /// The tab is one of the shell's panes with a page of its own: a `SplitPane` whose page is a
    /// web view's (a built-in page's pane, a start page's, is the shell's own drawing).
    pub placed: bool,
    /// What the shell draws over the pane (`overlay::covers`).
    pub coverage: Coverage,
    /// The pane has a frame shown 1:1 (`stretch::PaneView::is_live`; for a frame, after it).
    pub view_live: bool,
    /// A shot, a pixel check or a snapshot wants the page's pixels in the shell's textures.
    pub pixels: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The window got a page layer while the mode allows one (true), or lost it (false). Sent
    /// on a change only: after [`Event::Failed`] the layer is given up until the next `true`.
    Capable(bool),
    /// The needs were derived again and may have changed (fullscreen or picture-in-picture
    /// entered or left, shell UI over the page shown or gone, a resize begun or settled, a shot
    /// asked or taken).
    Changed,
    /// The pane's view painted the frame of generation `painted` (CEF's paint count,
    /// `NativeFrame::generation`); `os`: it was copied into an OS buffer.
    Frame { os: bool, painted: u64 },
    /// The shell is presenting a render of this window that drew the page's frame of `drawn`
    /// (the generation its presenter held as the render began; none: no frame of the page).
    Presenting { drawn: Option<u64> },
    /// The layer refused a frame.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the view's frame copy for OS buffers (true) or for the shell's own textures again.
    Buffers(bool),
    /// This frame onto the layer, shown; the host keeps it as the layer's latest.
    Show,
    /// Hide the layer and drop its frame, now: the shell's last present drew no hole over it.
    Hide,
    /// The layer goes with its frame, but stays under the shell until the window's next present,
    /// whose render draws no hole for it, and is hidden with that present: the shell's last
    /// present may still have its hole (`Layer`, and `Leaving` until the shell presents its frame).
    Retire,
    /// This frame to the shell (today's path: presenter, damage, render).
    Present,
    /// The layer's latest frame to the shell, as `Present` does with a new one.
    PresentLatest,
    /// The shell renders and presents the page's area once more (no new frame may come).
    Redraw,
    /// The page paints its frame again (CEF's invalidate): it may go on its layer now, and only a
    /// frame in an OS buffer takes it there, which a still page never sends by itself.
    Repaint,
    /// The layer's latest frame copied into the shell's texture of the pane, which only its glass
    /// reads (the pane is a hole): the glass blurs it, nothing else is drawn again.
    Snapshot,
}

/// The route after `event` at `now` (ms) with `hold` the strongest of the needs holding now
/// (`Needs::hold`), and what the host does (in order). Every (route, event) pair is an arm: a new
/// behaviour is a new arm or route, not a check beside.
pub fn transition(route: Route, hold: Hold, now: u64, event: Event) -> (Route, Vec<Effect>) {
    use {Effect::*, Event::*, Route::*};
    // a brief hold keeps the buffers until its deadline, set as it begins in `Entering`
    let deadline = match route {
        Entering { expires: Some(at) } => at,
        _ => now + BRIEF_MS,
    };
    let entering = Entering {
        expires: (hold == Hold::Brief).then_some(deadline),
    };
    let hold = route.hold(hold, now);
    // anything but glass keeps the page on the shell's path; a lasting hold lets the OS buffers
    // go too; glass over the page blurs a snapshot of its layer
    let (held, lasting, frost) = (hold >= Hold::Brief, hold == Hold::Long, hold == Hold::Frost);
    match (route, event) {
        // without a layer the shell draws every frame
        (Unsupported, Capable(false) | Changed | Presenting { .. } | Failed) => (route, vec![]),
        (Unsupported, Frame { .. }) => (route, vec![Present]),
        // a layer to use: the copy goes to OS buffers once nothing holds the page on the shell
        (Unsupported | Composited, Capable(true) | Changed) if held => (Composited, vec![]),
        // and the page paints once into them: a still page sends no frame by itself
        (Unsupported | Composited, Capable(true) | Changed) => {
            (entering, vec![Buffers(true), Repaint])
        }
        (Composited, Capable(false)) => (Unsupported, vec![]),
        (Composited, Frame { .. }) => (route, vec![Present]),
        // nothing is on the layer to fail
        (Composited, Presenting { .. } | Failed) => (route, vec![]),

        (Entering { .. }, Capable(false)) => (Unsupported, vec![Buffers(false)]),
        (Entering { .. }, Capable(true) | Changed) if lasting => (Composited, vec![Buffers(false)]),
        // a brief hold ended: the page paints again for its way back to the layer
        (Entering { expires: Some(_) }, Capable(true) | Changed | Presenting { .. }) if !held => {
            (entering, vec![Repaint])
        }
        (Entering { .. }, Capable(true) | Changed | Presenting { .. }) => (entering, vec![]),
        (Entering { .. }, Frame { os: true, .. }) if lasting => {
            (Composited, vec![Buffers(false), Present])
        }
        // held briefly: the shell draws it, the buffers stay for its return
        (Entering { .. }, Frame { os: true, .. }) if held => (entering, vec![Present]),
        (Entering { .. }, Frame { os: true, painted }) if frost => {
            frosted(None, painted, now, vec![Show])
        }
        (Entering { .. }, Frame { os: true, painted }) => (
            Layer {
                shown: painted,
                snap: None,
            },
            vec![Show],
        ),
        // copied before the buffers were in place
        (Entering { .. }, Frame { os: false, .. }) => (entering, vec![Present]),
        // nothing was shown yet
        (Entering { .. }, Failed) => (Unsupported, vec![Buffers(false)]),

        // the layer goes now: the window closes or the layer broke
        (Layer { .. }, Capable(false) | Failed) => {
            (Unsupported, vec![Buffers(false), PresentLatest, Retire])
        }
        (Layer { shown, .. }, Capable(true) | Changed) if held => {
            (Leaving { until: shown }, vec![PresentLatest])
        }
        // glass over the page: it stays, the glass sees it again when due
        (Layer { shown, snap }, Capable(true) | Changed) if frost => {
            frosted(snap, shown, now, vec![])
        }
        // no glass over it: no snapshot is kept for one
        (Layer { shown, .. }, Capable(true) | Changed) => (Layer { shown, snap: None }, vec![]),
        // a render of the shell's chrome (nothing of it reads the page)
        (Layer { .. }, Presenting { .. }) => (route, vec![]),
        (Layer { .. }, Frame { os: true, painted }) if held => {
            (Leaving { until: painted }, vec![Present])
        }
        (Layer { snap, .. }, Frame { os: true, painted }) if frost => {
            frosted(snap, painted, now, vec![Show])
        }
        (Layer { .. }, Frame { os: true, painted }) => (
            Layer {
                shown: painted,
                snap: None,
            },
            vec![Show],
        ),
        // the pool could not take it: the shell has to
        (Layer { .. }, Frame { os: false, painted }) => (Leaving { until: painted }, vec![Present]),

        // the shell may not have presented its frame yet: the hole may still be on screen
        (Leaving { .. }, Capable(false) | Failed) => (Unsupported, vec![Buffers(false), Retire]),
        // the needs are read again when the shell has drawn the page
        (Leaving { .. }, Capable(true) | Changed) => (route, vec![]),
        // a newer frame to the shell; the one it must draw before the layer goes stays
        (Leaving { .. }, Frame { .. }) => (route, vec![Present]),
        (Leaving { until }, Presenting { drawn: Some(d) }) if d >= until && lasting => {
            (Composited, vec![Hide, Buffers(false)])
        }
        // held briefly: the next frame free of it goes back to the layer
        (Leaving { until }, Presenting { drawn: Some(d) }) if d >= until && held => {
            (entering, vec![Hide])
        }
        // free: back to the layer at the page's next frame, which it paints for that
        (Leaving { until }, Presenting { drawn: Some(d) }) if d >= until => {
            (entering, vec![Hide, Repaint])
        }
        // the shell has not drawn it yet: once more, the layer stays
        (Leaving { .. }, Presenting { .. }) => (route, vec![Redraw]),
    }
}

/// The layer showing `shown` under glass, after `effects`: the glass's snapshot (`snap`) taken
/// again when there is none yet, or when the layer shows a newer frame and [`SNAPSHOT_MS`] have
/// passed since it was taken.
fn frosted(
    snap: Option<Snap>,
    shown: u64,
    now: u64,
    mut effects: Vec<Effect>,
) -> (Route, Vec<Effect>) {
    let due = snap.is_none_or(|s| s.painted < shown && now >= s.at + SNAPSHOT_MS);
    let snap = if due {
        effects.push(Effect::Snapshot);
        Some(Snap {
            painted: shown,
            at: now,
        })
    } else {
        snap
    };
    (Route::Layer { shown, snap }, effects)
}

/// Whether a pane's layer has no use left: it is hidden (not [`Route::on_layer`]) and where the
/// page is holds it off a layer for long in `mode` (a pane among the chrome in `fullscreen`, no
/// pane of its own in any mode). The host then drops it (`Capable(false)`, its entry with it), so
/// such a page pays nothing for it, and makes a new one at a frame of the page where it may use
/// one (asked with [`Route::Unsupported`] and [`Needs::from_place`]).
pub fn layer_unused(route: Route, needs: Needs, mode: Mode) -> bool {
    !route.on_layer() && Needs(needs.0 & PLACE.0).hold(mode) == Hold::Long
}

/// OS buffers per view: the one on the layer (also the one the glass's snapshot and the shell's
/// presenter read, by reference, each blit submitted at once), the one before it, two the
/// compositor may still read (frames that come in a burst, several within one display frame: the
/// M2 run's `pool_busy` with four), one to copy into. Each is the frame's size: 20 MB for a pane
/// of 2732 x 1866 px.
pub const POOL_SLOTS: usize = 5;

/// The pool slot the next frame is copied into: the first one after `latest` (round robin, so
/// the one used longest ago) that nobody holds (`in_use`) and is not `latest` (on the layer or
/// waiting to be taken). `None`: all are busy and the frame is dropped.
pub fn pick_slot(in_use: &[bool], latest: Option<usize>) -> Option<usize> {
    let n = in_use.len();
    let start = latest.map_or(0, |l| l + 1);
    (0..n)
        .map(|k| (start + k) % n)
        .find(|&i| !in_use[i] && Some(i) != latest)
}

/// A rectangle: in points (the window's logical px) or, for [`Placement::contents`], in the
/// unit square of the buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where the layer sits in its window and what of its buffer it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The layer's frame in its superlayer, points.
    pub frame: Rect,
    /// `contentsRect`: the part of the buffer shown, unit square.
    pub contents: Rect,
    /// `contentsScale`: buffer pixels per point.
    pub scale: f64,
}

/// The layer's placement for a pane at `pane` (the window's points, origin top left) in a layer
/// tree `root_h` points tall whose geometry is `flipped` (origin top left, as winit's view; else
/// bottom left), at `scale` pixels per point, showing the top left `clip` pixels of a `buffer`
/// pixels large. The frame is snapped to whole pixels, as the shell's panes are.
pub fn place(
    pane: Rect,
    root_h: f64,
    flipped: bool,
    scale: f64,
    buffer: (u32, u32),
    clip: (u32, u32),
) -> Placement {
    let snap = |v: f64| (v * scale).round() / scale;
    let (x, top, w, h) = (snap(pane.x), snap(pane.y), snap(pane.w), snap(pane.h));
    let y = if flipped { top } else { root_h - top - h };
    let part = |c: u32, b: u32| {
        if b == 0 {
            1.0
        } else {
            f64::from(c.min(b)) / f64::from(b)
        }
    };
    let (cw, ch) = (part(clip.0, buffer.0), part(clip.1, buffer.1));
    Placement {
        frame: Rect { x, y, w, h },
        contents: Rect {
            x: 0.0,
            y: if flipped { 0.0 } else { 1.0 - ch },
            w: cw,
            h: ch,
        },
        scale,
    }
}

/// A layer's `frame` (points, from [`place`]) in the whole points a Wayland subsurface takes
/// (`wl_subsurface.set_position`, `wp_viewport.set_destination`): `[x, y, w, h]`, rounded
/// outwards, so at a fractional scale the layer covers its pane and the shell's pixels around
/// the hole hide any edge of it (the hole is never wider than the pane).
pub fn covering(frame: Rect) -> [i32; 4] {
    let (x0, y0) = (frame.x.floor(), frame.y.floor());
    let (x1, y1) = ((frame.x + frame.w).ceil(), (frame.y + frame.h).ceil());
    [x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32]
}

/// How a window's shell presents its frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Present {
    /// The platform's vsync (FIFO: the present waits for the display).
    Vsync,
    /// Mailbox, paced by the shell itself: never more often than the display's rate.
    Paced,
}

/// The shell's present while a layer of its window exists, by platform (`std::env::consts::OS`);
/// [`Present::Vsync`] for one not named, and always without a layer. Linux: FIFO with a synced
/// subsurface stalled mutter's headless display (docs/research/linux-page-layer.md section 7);
/// Mailbox is clean there and on the desktop, but presents as often as it is asked (7.x), so the
/// shell paces it. macOS keeps vsync (Core Animation's transaction, no stall).
pub const LAYER_PRESENT: &[(&str, Present)] = &[("linux", Present::Paced)];

/// How a window of platform `os` presents, with a page layer of it (`layers`) or none.
pub fn shell_present(os: &str, layers: bool) -> Present {
    LAYER_PRESENT
        .iter()
        .find(|&&(name, _)| layers && name == os)
        .map_or(Present::Vsync, |&(_, present)| present)
}

/// `--page-layer`: which panes may go to a layer; each is a row of [`MODES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    /// None: the shell's path everywhere (the default off macOS, [`PLATFORM_MODES`]).
    #[default]
    Off,
    /// A page in fullscreen and the picture-in-picture window, over the shell (stage 1, macOS).
    Fullscreen,
    /// Every pane, under the shell, through its hole (stage 2, macOS).
    Window,
}

impl Mode {
    /// Its row.
    pub fn row(self) -> &'static ModeRow {
        &MODES[self as usize]
    }
}

/// Where a mode's layer sits in its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Over the shell's layer: shown only while the shell draws nothing over the page.
    Above,
    /// Under it: the shell clears a hole where the page shows ([`Route::holed`]).
    Below,
}

/// One mode: its flag value, where its layer sits, and the hold of each need (by [`Need`]'s
/// index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRow {
    pub name: &'static str,
    pub mode: Mode,
    pub order: Order,
    pub holds: [Hold; NEEDS],
}

/// The flag's values, in [`Mode`]'s order. A mode is a row here; what a need does in it is a
/// cell, never a branch on the mode elsewhere.
#[rustfmt::skip]
pub const MODES: &[ModeRow] = &[
    // no pane gets a layer: the holds are never read
    ModeRow { name: "off", mode: Mode::Off, order: Order::Above, holds: [Hold::Long; NEEDS] },
    // over the shell: anything of the shell over the page, and any pane among the chrome, sends
    // it back for long
    ModeRow { name: "fullscreen", mode: Mode::Fullscreen, order: Order::Above, holds: [Hold::Long; NEEDS] },
    // under the shell: its UI is drawn over the hole, its glass blurs a snapshot of the layer;
    // what needs the live page in the shell holds it briefly
    //                                                         Windowed    Covered     Resizing     Pixels       Sampled      Unplaced    Frosted
    ModeRow { name: "window", mode: Mode::Window, order: Order::Below, holds: [Hold::Free, Hold::Free, Hold::Brief, Hold::Brief, Hold::Brief, Hold::Long, Hold::Frost] },
];

/// The mode `name` names.
pub fn mode(name: &str) -> Option<Mode> {
    MODES.iter().find(|r| r.name == name).map(|r| r.mode)
}

/// The mode the browser starts in on each platform (`std::env::consts::OS`) without
/// `--page-layer`, when Settings has it on (`settings::PAGE_LAYER`): macOS `window` from 0.6.2
/// (decisions.md, on by default); Linux `window` (a Wayland subsurface; X11 and XWayland have no
/// layer and keep the shell's path), off by default there until the owner measured it (the
/// setting's default is per platform); [`Mode::Off`] for a platform the table does not name.
/// Smokes and measurements start off unless they ask.
pub const PLATFORM_MODES: &[(&str, Mode)] = &[("macos", Mode::Window), ("linux", Mode::Window)];

/// The browser's mode on platform `os` ([`PLATFORM_MODES`]).
pub fn platform_mode(os: &str) -> Mode {
    PLATFORM_MODES
        .iter()
        .find_map(|&(name, mode)| (name == os).then_some(mode))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use {Effect::*, Event::*, Route::*};

    /// The layer on frame 5 (or 7), no glass over it; leaving for its own frame 5, handed over,
    /// or for a newer 7; entering with no brief hold running.
    const ON5: Route = Layer {
        shown: 5,
        snap: None,
    };
    const ON7: Route = Layer {
        shown: 7,
        snap: None,
    };
    const IN: Route = Entering { expires: None };
    const FOR5: Route = Leaving { until: 5 };
    const FOR7: Route = Leaving { until: 7 };

    /// The host's clock at the event (ms), and a brief hold begun then: held until `T + BRIEF_MS`.
    const T: u64 = 5_000;
    const HELD: Route = Entering {
        expires: Some(T + BRIEF_MS),
    };

    /// The layer on `shown` with the glass's snapshot of `painted` taken at `at`.
    fn snapped(shown: u64, painted: u64, at: u64) -> Route {
        Layer {
            shown,
            snap: Some(Snap { painted, at }),
        }
    }

    /// Stage 1's two cases: nothing holds the page (false), or something holds it for long.
    fn needs(blocked: bool) -> Hold {
        if blocked { Hold::Long } else { Hold::Free }
    }

    fn os(painted: u64) -> Event {
        Frame { os: true, painted }
    }

    fn copied(painted: u64) -> Event {
        Frame { os: false, painted }
    }

    fn drew(d: u64) -> Event {
        Presenting { drawn: Some(d) }
    }

    const BLANK: Event = Presenting { drawn: None };

    /// Every route, each event, with the needs clear (false) and holding (true).
    #[test]
    fn every_route_and_event() {
        type RouteRow = (&'static str, Route, bool, Event, Route, &'static [Effect]);
        #[rustfmt::skip]
        let rows: &[RouteRow] = &[
            // Unsupported: the shell draws every frame, nothing else happens
            ("unsupported: stays without a layer", Unsupported, false, Capable(false), Unsupported, &[]),
            ("unsupported: a layer, nothing needs the shell: buffers, and a frame into them", Unsupported, false, Capable(true), IN, &[Buffers(true), Repaint]),
            ("unsupported: a layer, the shell is needed", Unsupported, true, Capable(true), Composited, &[]),
            ("unsupported: needs ignored", Unsupported, false, Changed, Unsupported, &[]),
            ("unsupported: needs ignored (holding)", Unsupported, true, Changed, Unsupported, &[]),
            ("unsupported: a frame to the shell", Unsupported, false, copied(7), Unsupported, &[Present]),
            ("unsupported: an os frame to the shell", Unsupported, false, os(7), Unsupported, &[Present]),
            ("unsupported: a render ignored", Unsupported, false, drew(7), Unsupported, &[]),
            ("unsupported: a failure ignored", Unsupported, false, Failed, Unsupported, &[]),
            // Composited
            ("composited: loses the layer", Composited, false, Capable(false), Unsupported, &[]),
            ("composited: capable again, clear", Composited, false, Capable(true), IN, &[Buffers(true), Repaint]),
            ("composited: capable again, needed", Composited, true, Capable(true), Composited, &[]),
            ("composited: the needs clear", Composited, false, Changed, IN, &[Buffers(true), Repaint]),
            ("composited: still needed", Composited, true, Changed, Composited, &[]),
            ("composited: a frame to the shell", Composited, false, copied(7), Composited, &[Present]),
            ("composited: a late os frame to the shell", Composited, true, os(7), Composited, &[Present]),
            ("composited: a render ignored", Composited, false, drew(7), Composited, &[]),
            ("composited: nothing on the layer to fail", Composited, false, Failed, Composited, &[]),
            // Entering
            ("entering: loses the layer", IN, false, Capable(false), Unsupported, &[Buffers(false)]),
            ("entering: capable again, clear", IN, false, Capable(true), IN, &[]),
            ("entering: capable again, needed", IN, true, Capable(true), Composited, &[Buffers(false)]),
            ("entering: still clear", IN, false, Changed, IN, &[]),
            ("entering: needed again", IN, true, Changed, Composited, &[Buffers(false)]),
            ("entering: the first os frame goes on the layer", IN, false, os(7), ON7, &[Show]),
            ("entering: an os frame while needed", IN, true, os(7), Composited, &[Buffers(false), Present]),
            ("entering: a frame copied before the buffers", IN, false, copied(7), IN, &[Present]),
            ("entering: a render ignored", IN, false, BLANK, IN, &[]),
            ("entering: a failure gives the layer up", IN, false, Failed, Unsupported, &[Buffers(false)]),
            // Layer
            ("layer: loses the layer", ON5, false, Capable(false), Unsupported, &[Buffers(false), PresentLatest, Retire]),
            ("layer: capable again, clear", ON5, false, Capable(true), ON5, &[]),
            ("layer: capable again, needed: its frame to the shell", ON5, true, Capable(true), FOR5, &[PresentLatest]),
            ("layer: still clear", ON5, false, Changed, ON5, &[]),
            ("layer: shell UI over the page", ON5, true, Changed, FOR5, &[PresentLatest]),
            ("layer: the next frame", ON5, false, os(7), ON7, &[Show]),
            ("layer: a frame while needed waits to be drawn", ON5, true, os(7), FOR7, &[Present]),
            ("layer: a frame the pool could not take", ON5, false, copied(7), FOR7, &[Present]),
            ("layer: a render of the chrome", ON5, false, drew(5), ON5, &[]),
            ("layer: fails", ON5, false, Failed, Unsupported, &[Buffers(false), PresentLatest, Retire]),
            // Leaving: hidden only once a render drew the frame given (or a newer one)
            ("leaving: the tab goes before the shell presented its frame: retired, its hole may be on screen", FOR5, false, Capable(false), Unsupported, &[Buffers(false), Retire]),
            ("leaving: capable again", FOR5, false, Capable(true), FOR5, &[]),
            ("leaving: the needs clear (read once drawn)", FOR5, false, Changed, FOR5, &[]),
            ("leaving: still needed", FOR5, true, Changed, FOR5, &[]),
            ("leaving: a newer frame to the shell, the same wait", FOR5, false, os(7), FOR5, &[Present]),
            ("leaving: a copied frame to the shell", FOR5, true, copied(7), FOR5, &[Present]),
            ("leaving: a render without the page: once more", FOR5, false, BLANK, FOR5, &[Redraw]),
            ("leaving: a render of an older frame: once more", FOR7, false, drew(5), FOR7, &[Redraw]),
            ("leaving: one short, still needed", FOR7, true, drew(6), FOR7, &[Redraw]),
            ("leaving: drawn, still needed", FOR5, true, drew(5), Composited, &[Hide, Buffers(false)]),
            ("leaving: drawn, nothing needs the shell: the page paints for its return", FOR5, false, drew(5), IN, &[Hide, Repaint]),
            ("leaving: a newer one drawn", FOR5, false, drew(7), IN, &[Hide, Repaint]),
            ("leaving: fails", FOR7, false, Failed, Unsupported, &[Buffers(false), Retire]),
        ];
        for &(name, route, blocked, event, next, effects) in rows {
            assert_eq!(
                transition(route, needs(blocked), T, event),
                (next, effects.to_vec()),
                "{name}"
            );
        }
    }

    /// `window`'s brief holds (a resize, an entrance, a shot): off the layer as a long hold
    /// would, but the OS buffers stay (from the hold's start in `Entering`, `HELD`) and the page
    /// comes back without asking for them; a pane already off its layer stays there.
    #[test]
    fn page_layer_every_route_and_event_held_briefly() {
        #[rustfmt::skip]
        let rows: &[(&str, Route, Event, Route, &[Effect])] = &[
            ("unsupported: a layer while held: composited until free", Unsupported, Capable(true), Composited, &[]),
            ("unsupported: needs ignored", Unsupported, Changed, Unsupported, &[]),
            ("unsupported: a frame to the shell", Unsupported, os(7), Unsupported, &[Present]),
            ("unsupported: stays without a layer", Unsupported, Capable(false), Unsupported, &[]),
            ("unsupported: a render ignored", Unsupported, drew(7), Unsupported, &[]),
            ("unsupported: a failure ignored", Unsupported, Failed, Unsupported, &[]),
            ("composited: a long hold became brief: stays", Composited, Changed, Composited, &[]),
            ("composited: capable again", Composited, Capable(true), Composited, &[]),
            ("composited: loses the layer", Composited, Capable(false), Unsupported, &[]),
            ("composited: a frame to the shell", Composited, copied(7), Composited, &[Present]),
            ("composited: a render ignored", Composited, drew(7), Composited, &[]),
            ("composited: nothing to fail", Composited, Failed, Composited, &[]),
            ("entering: held, the buffers stay, the deadline set", IN, Changed, HELD, &[]),
            ("entering: capable again", IN, Capable(true), HELD, &[]),
            ("entering: an os frame to the shell, the buffers stay", IN, os(7), HELD, &[Present]),
            ("entering: a copied frame to the shell", IN, copied(7), HELD, &[Present]),
            ("entering: a render ignored", IN, BLANK, HELD, &[]),
            ("entering: still held, the deadline kept", Entering { expires: Some(T + 1) }, Changed, Entering { expires: Some(T + 1) }, &[]),
            ("entering: loses the layer", IN, Capable(false), Unsupported, &[Buffers(false)]),
            ("entering: fails", IN, Failed, Unsupported, &[Buffers(false)]),
            ("layer: a resize: its frame to the shell", ON5, Changed, FOR5, &[PresentLatest]),
            ("layer: capable again, held", ON5, Capable(true), FOR5, &[PresentLatest]),
            ("layer: a frame while held waits to be drawn", ON5, os(7), FOR7, &[Present]),
            ("layer: a frame the pool could not take", ON5, copied(7), FOR7, &[Present]),
            ("layer: a render of the chrome", ON5, drew(5), ON5, &[]),
            ("layer: under glass, then held: off it", snapped(5, 5, T), Changed, FOR5, &[PresentLatest]),
            ("layer: loses the layer", ON5, Capable(false), Unsupported, &[Buffers(false), PresentLatest, Retire]),
            ("layer: fails", ON5, Failed, Unsupported, &[Buffers(false), PresentLatest, Retire]),
            ("leaving: still held", FOR5, Changed, FOR5, &[]),
            ("leaving: capable again", FOR5, Capable(true), FOR5, &[]),
            ("leaving: a newer frame to the shell", FOR5, os(7), FOR5, &[Present]),
            ("leaving: a render without the page", FOR5, BLANK, FOR5, &[Redraw]),
            ("leaving: an older frame drawn", FOR7, drew(5), FOR7, &[Redraw]),
            ("leaving: drawn: hidden, the buffers stay, the deadline set", FOR5, drew(5), HELD, &[Hide]),
            ("leaving: a newer one drawn", FOR5, drew(7), HELD, &[Hide]),
            ("leaving: the tab goes before the shell presented its frame: retired, its hole may be on screen", FOR5, Capable(false), Unsupported, &[Buffers(false), Retire]),
            ("leaving: fails", FOR7, Failed, Unsupported, &[Buffers(false), Retire]),
        ];
        for &(name, route, event, next, effects) in rows {
            assert_eq!(
                transition(route, Hold::Brief, T, event),
                (next, effects.to_vec()),
                "{name}"
            );
        }
    }

    /// A row of the timed tables: name, route, hold, now (ms), event, route after, effects.
    type HoldRow = (
        &'static str,
        Route,
        Hold,
        u64,
        Event,
        Route,
        &'static [Effect],
    );

    /// A brief hold that outlasts [`BRIEF_MS`] in `Entering` is a long one: the OS buffers go at
    /// the next event that reads it, and the pane stays composited until nothing holds it.
    #[test]
    fn page_layer_a_brief_hold_expires_to_long() {
        let due = Entering { expires: Some(T) };
        #[rustfmt::skip]
        let rows: &[HoldRow] = &[
            ("a ms before its deadline: still brief", due, Hold::Brief, T - 1, Changed, due, &[]),
            ("at its deadline: long, the buffers go", due, Hold::Brief, T, Changed, Composited, &[Buffers(false)]),
            ("past it, at a frame", due, Hold::Brief, T + 500, os(7), Composited, &[Buffers(false), Present]),
            ("past it, a render: the deadline kept for the next read", due, Hold::Brief, T + 500, BLANK, due, &[]),
            ("past it, a frame copied without buffers", due, Hold::Brief, T + 500, copied(7), due, &[Present]),
            ("the hold ended late: nothing expired, the page paints", due, Hold::Free, T + 500, Changed, IN, &[Repaint]),
            ("the hold ended: a still page paints for its way back", HELD, Hold::Free, T + 1, Changed, IN, &[Repaint]),
            ("the hold ended at a render", HELD, Hold::Free, T + 1, BLANK, IN, &[Repaint]),
            ("the hold ended, capable again", HELD, Hold::Free, T + 1, Capable(true), IN, &[Repaint]),
            ("the hold ended, only glass left", HELD, Hold::Frost, T + 1, Changed, IN, &[Repaint]),
            ("asked once: entering with no hold asks no more", IN, Hold::Free, T + 1, Changed, IN, &[]),
            ("the hold ended late, a frame: on the layer", due, Hold::Free, T + 500, os(7), ON7, &[Show]),
            ("only glass left: on the layer with a snapshot", due, Hold::Frost, T, os(7), snapped(7, 7, T), &[Show, Snapshot]),
            ("a long hold needs no deadline", due, Hold::Long, T - 1, Changed, Composited, &[Buffers(false)]),
            ("expired and composited: stays while held", Composited, Hold::Brief, T, Changed, Composited, &[]),
            ("free again: the buffers asked for", Composited, Hold::Free, T, Changed, IN, &[Buffers(true), Repaint]),
            ("a new brief hold gets a new deadline", IN, Hold::Brief, T, Changed, HELD, &[]),
        ];
        for &(name, route, hold, now, event, next, effects) in rows {
            assert_eq!(
                transition(route, hold, now, event),
                (next, effects.to_vec()),
                "{name}"
            );
        }
        // the effective hold the host's log names
        #[rustfmt::skip]
        let holds: &[(Route, Hold, u64, Hold)] = &[
            (due, Hold::Brief, T - 1, Hold::Brief), (due, Hold::Brief, T, Hold::Long),
            (due, Hold::Free, T, Hold::Free), (due, Hold::Frost, T, Hold::Frost),
            (IN, Hold::Brief, T, Hold::Brief), (ON5, Hold::Brief, T + BRIEF_MS, Hold::Brief),
            (FOR5, Hold::Brief, T + BRIEF_MS, Hold::Brief),
        ];
        for &(route, hold, now, want) in holds {
            assert_eq!(route.hold(hold, now), want, "{route:?} {hold:?} at {now}");
        }
    }

    /// Glass over a page on its layer (`Hold::Frost`): the page stays there; the glass's snapshot
    /// is taken at once, then again at most every [`SNAPSHOT_MS`] and only for a newer frame.
    #[test]
    fn page_layer_glass_over_a_layered_page_blurs_a_slow_snapshot() {
        let s = SNAPSHOT_MS;
        #[rustfmt::skip]
        let rows: &[HoldRow] = &[
            ("glass comes over a still page: a snapshot at once", ON5, Hold::Frost, T, Changed, snapped(5, 5, T), &[Snapshot]),
            ("glass over a playing page: the frame and a snapshot", ON5, Hold::Frost, T, os(6), snapped(6, 6, T), &[Show, Snapshot]),
            ("a frame before it is due: on the layer only", snapped(5, 5, T), Hold::Frost, T + 1, os(6), snapped(6, 5, T), &[Show]),
            ("a frame once due", snapped(5, 5, T), Hold::Frost, T + s, os(6), snapped(6, 6, T + s), &[Show, Snapshot]),
            ("a render, nothing newer: none", snapped(5, 5, T), Hold::Frost, T + 4 * s, Changed, snapped(5, 5, T), &[]),
            ("a render, newer, not due: none", snapped(7, 5, T), Hold::Frost, T + s - 1, Changed, snapped(7, 5, T), &[]),
            ("a paused video's last frame at the next render once due", snapped(7, 5, T), Hold::Frost, T + s, Changed, snapped(7, 7, T + s), &[Snapshot]),
            ("its present: nothing", snapped(5, 5, T), Hold::Frost, T + s, BLANK, snapped(5, 5, T), &[]),
            ("the glass gone: the snapshot let go", snapped(5, 5, T), Hold::Free, T + 1, Changed, ON5, &[]),
            ("the glass gone, a frame", snapped(5, 5, T), Hold::Free, T + 1, os(6), Layer { shown: 6, snap: None }, &[Show]),
            ("the pool could not take a frame: to the shell", snapped(5, 5, T), Hold::Frost, T, copied(6), Leaving { until: 6 }, &[Present]),
            ("the tab leaves under glass", snapped(5, 5, T), Hold::Frost, T, Capable(false), Unsupported, &[Buffers(false), PresentLatest, Retire]),
            ("entering under glass: stays for its frame", IN, Hold::Frost, T, Changed, IN, &[]),
            ("entering under glass: on the layer with a snapshot", IN, Hold::Frost, T, os(6), snapped(6, 6, T), &[Show, Snapshot]),
            ("composited under glass: the buffers asked for", Composited, Hold::Frost, T, Changed, IN, &[Buffers(true), Repaint]),
            ("leaving under glass: back to it when drawn", FOR5, Hold::Frost, T, drew(5), IN, &[Hide, Repaint]),
        ];
        for &(name, route, hold, now, event, next, effects) in rows {
            assert_eq!(
                transition(route, hold, now, event),
                (next, effects.to_vec()),
                "{name}"
            );
        }
    }

    /// What the host's edge holds as the effects run: whether the layer is visible and with
    /// which frame, whether the copy uses OS buffers, the newest frame the shell was given and
    /// the newest it presented.
    #[derive(Default)]
    struct Edge {
        visible: bool,
        buffers: bool,
        layer: Option<u64>,
        given: Option<u64>,
        drew: Option<u64>,
    }

    impl Edge {
        fn run(&mut self, event: Event, effects: &[Effect]) {
            if let Presenting { drawn } = event {
                assert!(
                    drawn <= self.given,
                    "the walk drew {drawn:?}, given {:?}",
                    self.given
                );
                self.drew = drawn.max(self.drew);
            }
            let painted = match event {
                Frame { painted, .. } => Some(painted),
                _ => None,
            };
            for effect in effects {
                match effect {
                    Buffers(on) => self.buffers = *on,
                    Show => {
                        assert!(self.buffers, "a frame on the layer without OS buffers");
                        self.visible = true;
                        self.layer = painted;
                    }
                    Hide => {
                        assert!(
                            !self.visible || self.drew >= self.layer,
                            "the layer hidden before the shell drew its frame"
                        );
                        self.visible = false;
                    }
                    // the window closed or the layer broke: it goes, drawn or not, with the
                    // next present (which draws no hole for it)
                    Retire => self.visible = false,
                    Present => self.given = painted.max(self.given),
                    PresentLatest => self.given = self.layer.max(self.given),
                    Redraw | Repaint => {}
                    // the layer's frame into the shell's texture, for its glass
                    Snapshot => {
                        assert!(
                            self.visible && self.layer.is_some(),
                            "a snapshot of no layer's frame"
                        );
                        self.given = self.layer.max(self.given);
                    }
                }
            }
        }
    }

    /// Event sequences, needs as they hold at each event: the layer is visible exactly in
    /// Layer and Leaving, the OS buffers are asked for exactly while a layer is in play, and
    /// the layer is hidden only after a render drew a frame as new as its own.
    #[test]
    fn the_layer_is_visible_whenever_the_shell_does_not_draw_the_page() {
        let walks: &[(&str, &[(bool, Event)])] = &[
            (
                "fullscreen video, then its controls",
                &[
                    (true, Capable(true)),
                    (false, Changed),
                    (false, copied(1)),
                    (false, os(2)),
                    (false, os(3)),
                    (true, Changed),
                    (true, os(4)),
                    (true, BLANK),
                    (true, drew(3)),
                    (true, copied(5)),
                ],
            ),
            (
                "the controls go again while it leaves",
                &[
                    (false, Capable(true)),
                    (false, os(1)),
                    (true, Changed),
                    (false, Changed),
                    (false, drew(1)),
                    (false, os(2)),
                    (false, os(3)),
                ],
            ),
            (
                "the pool runs dry, then the window closes",
                &[
                    (false, Capable(true)),
                    (false, os(1)),
                    (false, copied(2)),
                    (false, drew(1)),
                    (false, drew(2)),
                    (false, os(3)),
                    (false, os(4)),
                    (false, Capable(false)),
                    (false, copied(5)),
                ],
            ),
            (
                "the layer fails, and comes back",
                &[
                    (false, Capable(true)),
                    (false, os(1)),
                    (false, Failed),
                    (false, copied(2)),
                    (false, Capable(true)),
                    (false, os(3)),
                    (true, Capable(false)),
                ],
            ),
            (
                "a still page leaves fullscreen",
                &[
                    (false, Capable(true)),
                    (false, os(1)),
                    (true, Changed),
                    (true, BLANK),
                    (true, BLANK),
                    (true, drew(1)),
                ],
            ),
            (
                "frames keep coming while it leaves",
                &[
                    (false, Capable(true)),
                    (false, os(1)),
                    (false, os(2)),
                    (true, Changed),
                    (true, os(3)),
                    (true, drew(2)),
                    (true, copied(4)),
                ],
            ),
            (
                "handed over by shell UI, then the first render drew it",
                &[
                    (false, Capable(true)),
                    (false, os(5)),
                    (true, Changed),
                    (true, drew(5)),
                ],
            ),
            (
                "renders without the page twice, a frame meanwhile, then drawn",
                &[
                    (false, Capable(true)),
                    (false, os(3)),
                    (true, Changed),
                    (true, os(4)),
                    (true, BLANK),
                    (true, BLANK),
                    (true, drew(3)),
                    (true, drew(4)),
                ],
            ),
            (
                "the layer breaks while it leaves",
                &[
                    (false, Capable(true)),
                    (false, os(1)),
                    (true, Changed),
                    (true, Failed),
                    (true, copied(2)),
                ],
            ),
        ];
        // `window`: an entrance, a resize and a shot hold it briefly (for a second at most), glass
        // over it leaves it there; each walk with its time between events (ms)
        let (free, brief, frost) = (Hold::Free, Hold::Brief, Hold::Frost);
        type HeldWalk<'a> = (&'a str, u64, &'a [(Hold, Event)]);
        let held: &[HeldWalk<'_>] = &[
            (
                "an entrance over the page, then gone",
                16,
                &[
                    (free, Capable(true)),
                    (free, os(1)),
                    (brief, Changed),
                    (brief, os(2)),
                    (brief, drew(1)),
                    (brief, drew(2)),
                    (brief, os(3)),
                    (free, Changed),
                    (free, os(4)),
                    (free, os(5)),
                ],
            ),
            (
                "the sidebar springs open over a still page",
                16,
                &[
                    (free, Capable(true)),
                    (free, os(1)),
                    (brief, Changed),
                    (brief, BLANK),
                    (brief, drew(1)),
                    (brief, copied(2)),
                    (free, Changed),
                    (free, os(3)),
                ],
            ),
            (
                "the tab leaves while on its layer",
                16,
                &[
                    (free, Capable(true)),
                    (free, os(1)),
                    (free, Capable(false)),
                    (free, copied(2)),
                ],
            ),
            (
                "a brief hold turns long (the peek takes the page), then the page comes back",
                16,
                &[
                    (free, Capable(true)),
                    (free, os(1)),
                    (brief, Changed),
                    (Hold::Long, drew(1)),
                    (Hold::Long, copied(2)),
                    (free, Changed),
                    (free, copied(3)),
                    (free, os(4)),
                ],
            ),
            (
                "a toast over a playing video: the page stays on its layer",
                100,
                &[
                    (free, Capable(true)),
                    (free, os(1)),
                    (frost, Changed),
                    (frost, os(2)),
                    (frost, os(3)),
                    (frost, os(4)),
                    (frost, BLANK),
                    (frost, Changed),
                    (free, Changed),
                    (free, os(5)),
                ],
            ),
            (
                "a still page enters: its first frame copied, the one it repaints on the layer",
                16,
                &[
                    (free, Capable(true)),
                    (free, copied(1)),
                    (free, os(2)),
                    (free, drew(1)),
                ],
            ),
            (
                "a resize that holds for more than a second lets the buffers go",
                300,
                &[
                    (free, Capable(true)),
                    (free, os(1)),
                    (brief, Changed),
                    (brief, drew(1)),
                    (brief, os(2)),
                    (brief, os(3)),
                    (brief, os(4)),
                    (brief, os(5)),
                    (brief, Changed),
                    (free, Changed),
                    (free, os(6)),
                ],
            ),
        ];
        let walks = walks
            .iter()
            .map(|&(name, walk)| {
                let walk = walk.iter().map(|&(b, e)| (needs(b), e)).collect();
                (name, 16, walk)
            })
            .chain(
                held.iter()
                    .map(|&(name, step, walk)| (name, step, walk.to_vec())),
            );
        for (name, step, walk) in walks {
            let walk: Vec<(Hold, Event)> = walk;
            let (mut route, mut edge) = (Route::default(), Edge::default());
            for (i, &(hold, event)) in walk.iter().enumerate() {
                let (next, effects) = transition(route, hold, i as u64 * step, event);
                edge.run(event, &effects);
                route = next;
                let at = format!("{name}, step {i} ({event:?} -> {route:?})");
                assert_eq!(
                    edge.visible,
                    matches!(route, Layer { .. } | Leaving { .. }),
                    "visible: {at}"
                );
                assert_eq!(
                    edge.buffers,
                    matches!(route, Entering { .. } | Layer { .. } | Leaving { .. }),
                    "buffers: {at}"
                );
            }
        }
    }

    /// Each need from its owner's fact, and a pixel reader sends the layer's frame to the shell.
    #[test]
    fn page_layer_needs_from_owners() {
        use Need::*;
        let clear = Owners {
            page_fullscreen: true,
            pip_window: false,
            placed: true,
            coverage: Coverage::None,
            view_live: true,
            pixels: false,
        };
        let only = |n: &[Need]| n.iter().fold(Needs::NONE, |s, &n| s.with(n, true));
        #[rustfmt::skip]
        let rows: &[(&str, Owners, Needs)] = &[
            ("fullscreen, nothing over it", clear, Needs::NONE),
            ("fullscreen, resizing", Owners { view_live: false, ..clear }, only(&[Resizing])),
            ("fullscreen, shell pixels over it", Owners { coverage: Coverage::Part, ..clear }, only(&[Covered])),
            ("fullscreen, glass over it", Owners { coverage: Coverage::Glass, ..clear }, only(&[Covered, Frosted])),
            ("fullscreen, an entrance moving it", Owners { coverage: Coverage::Pixels, ..clear }, only(&[Covered, Sampled])),
            ("no pane of its own (the peek's page)", Owners { page_fullscreen: false, placed: false, ..clear }, only(&[Windowed, Unplaced])),
            ("the picture-in-picture window", Owners { page_fullscreen: false, pip_window: true, ..clear }, Needs::NONE),
            ("a pane among the chrome", Owners { page_fullscreen: false, ..clear }, only(&[Windowed])),
            ("a shot pending", Owners { pixels: true, ..clear }, only(&[Pixels])),
            ("everything", Owners { page_fullscreen: false, pip_window: false, placed: false, coverage: Coverage::Pixels, view_live: false, pixels: true },
                only(&[Windowed, Covered, Resizing, Pixels, Sampled, Unplaced])),
        ];
        for &(name, owners, want) in rows {
            assert_eq!(Needs::from_owners(&owners), want, "{name}");
        }
        let pixels = Needs::from_owners(&Owners {
            pixels: true,
            ..clear
        });
        for mode in [Mode::Fullscreen, Mode::Window] {
            assert_eq!(
                transition(ON5, pixels.hold(mode), T, Changed),
                (FOR5, vec![PresentLatest]),
                "{mode:?}"
            );
        }
    }

    /// What each need does in each mode: every stage 1 need holds for long in `fullscreen`;
    /// under the shell glass blurs a snapshot, only what needs the live page in the shell holds
    /// it, briefly, and only a page without a pane for long.
    #[test]
    fn page_layer_holds_by_mode() {
        use Hold::{Brief, Free, Frost, Long};
        use Need::*;
        let only = |n: &[Need]| n.iter().fold(Needs::NONE, |s, &n| s.with(n, true));
        #[rustfmt::skip]
        let rows: &[(&str, Needs, Hold, Hold)] = &[
            // (name, needs, fullscreen, window)
            ("nothing", Needs::NONE, Free, Free),
            ("a pane among the chrome", only(&[Windowed]), Long, Free),
            ("a tooltip over it", only(&[Covered]), Long, Free),
            ("glass over it", only(&[Covered, Frosted]), Long, Frost),
            ("an entrance moving it", only(&[Covered, Sampled]), Long, Brief),
            ("glass over a resizing page", only(&[Covered, Frosted, Resizing]), Long, Brief),
            ("a resize", only(&[Resizing]), Long, Brief),
            ("a shot", only(&[Pixels]), Long, Brief),
            ("no pane of its own", only(&[Windowed, Unplaced]), Long, Long),
            ("the strongest wins", only(&[Covered, Resizing, Unplaced]), Long, Long),
            ("a windowed pane under glass", only(&[Windowed, Covered, Frosted]), Long, Frost),
        ];
        for &(name, needs, fullscreen, window) in rows {
            assert_eq!(
                (needs.hold(Mode::Fullscreen), needs.hold(Mode::Window)),
                (fullscreen, window),
                "{name}"
            );
        }
    }

    /// The hole is drawn exactly while the page is on a layer under the shell and the shell does
    /// not draw it.
    #[test]
    fn page_layer_route_holed() {
        #[rustfmt::skip]
        let rows: &[(Route, bool, bool)] = &[
            // (route, fullscreen, window)
            (Unsupported, false, false), (Composited, false, false), (IN, false, false),
            (ON5, false, true), (FOR5, false, false),
        ];
        for &(route, fullscreen, window) in rows {
            assert_eq!(
                (
                    route.holed(Mode::Fullscreen),
                    route.holed(Mode::Window),
                    route.holed(Mode::Off)
                ),
                (fullscreen, window, false),
                "{route:?}"
            );
            assert!(
                !route.holed(Mode::Window) || route.on_layer(),
                "a hole over no layer: {route:?}"
            );
        }
    }

    #[test]
    fn page_layer_route_on_layer() {
        #[rustfmt::skip]
        let rows: &[(Route, bool)] = &[
            (Unsupported, false), (Composited, false), (IN, false), (ON5, true), (FOR5, true),
        ];
        for &(route, on) in rows {
            assert_eq!(route.on_layer(), on, "{route:?}");
        }
    }

    #[test]
    fn page_layer_a_hidden_layer_of_a_windowed_page_goes() {
        use Mode::{Fullscreen, Window};
        let windowed = Needs::NONE.with(Need::Windowed, true);
        let unplaced = windowed.with(Need::Unplaced, true);
        let covered = Needs::NONE.with(Need::Covered, true);
        #[rustfmt::skip]
        let rows: &[(Route, Needs, Mode, bool)] = &[
            (Composited, windowed, Fullscreen, true), (IN, windowed, Fullscreen, true),
            (Unsupported, windowed, Fullscreen, true),
            (Composited, windowed.with(Need::Covered, true), Fullscreen, true),
            // still visible: it goes once the shell has drawn the page (`Presenting`)
            (ON5, windowed, Fullscreen, false), (FOR5, windowed, Fullscreen, false),
            // fullscreen or picture-in-picture: the layer comes back when the cover goes
            (Composited, covered, Fullscreen, false), (IN, Needs::NONE, Fullscreen, false),
            (ON5, Needs::NONE, Fullscreen, false),
            // under the shell a pane among the chrome keeps its layer; a page without a pane does not
            (IN, windowed, Window, false), (Unsupported, windowed, Window, false),
            (IN, windowed.with(Need::Sampled, true), Window, false),
            (IN, unplaced, Window, true), (Unsupported, unplaced, Window, true),
            (ON5, unplaced, Window, false), (FOR5, unplaced, Window, false),
            // a new pane's first frame asks with where it is only
            (Unsupported, Needs::from_place(true, true), Fullscreen, false),
            (Unsupported, Needs::from_place(false, true), Fullscreen, true),
            (Unsupported, Needs::from_place(false, true), Window, false),
            (Unsupported, Needs::from_place(false, false), Window, true),
            // a built-in page's split pane has no page of its own: never a layer, in any mode
            (Unsupported, Needs::from_place(false, false), Fullscreen, true),
        ];
        for &(route, needs, mode, gone) in rows {
            assert_eq!(
                layer_unused(route, needs, mode),
                gone,
                "{route:?} {needs:?} {mode:?}"
            );
        }
    }

    /// Every surface that can appear over a fullscreen page (open, or still drawn as it leaves)
    /// sends the layer's frame to the shell before the layer goes (`fullscreen`); under the shell
    /// (`window`) the page stays on its layer, and those whose glass reads it take its snapshot;
    /// the page's own link hints never count.
    #[test]
    fn page_layer_a_surface_over_a_fullscreen_page_makes_the_pane_leave() {
        use crate::overlay::{Facts, Kind::*, Layer as Surface, Stack, covers};
        use crate::tab::TabId;
        let tab = TabId(1);
        let facts = Facts {
            glass: true,
            ..Facts::default()
        };
        let leaves = (FOR5, vec![PresentLatest]);
        let frosted = (snapped(5, 5, T), vec![Snapshot]);
        let stays = (ON5, vec![]);
        #[rustfmt::skip]
        let rows = [
            // (kind, leaves in fullscreen, glass in window: the layer stays, its snapshot taken)
            (VideoStrip, true, true), (VideoTab, true, true), (VideoSpeeds, true, true), (Toast, true, true),
            (Popup, true, false), (AutofillChip, true, false), (AutofillMenu, true, true),
            (AutofillPrompt, true, true), (Tooltip, true, false), (TabGhost, true, false),
            (PageGhost, true, false), (PipStrip, true, true), (Hints, false, false),
            (Notice, true, true), (Menu, true, true), (Preview, true, true),
        ];
        for (kind, fullscreen, window) in rows {
            let layer = vec![Surface::new(kind, Some(tab))];
            let open = Stack {
                shown: Some(tab),
                layers: layer.clone(),
                leaving: vec![],
                ..Stack::default()
            };
            let leaving = Stack {
                shown: Some(tab),
                layers: vec![],
                leaving: layer,
                ..Stack::default()
            };
            for (state, stack) in [("open", open), ("leaving", leaving)] {
                let needs = Needs::NONE.covered(covers(&stack, &facts, tab));
                let fullscreen_wants = if fullscreen { &leaves } else { &stays };
                let window_wants = if window { &frosted } else { &stays };
                for (mode, want) in [
                    (Mode::Fullscreen, fullscreen_wants),
                    (Mode::Window, window_wants),
                ] {
                    let got = transition(ON5, needs.hold(mode), T, Changed);
                    assert_eq!(&got, want, "{kind:?} {state} {mode:?}");
                }
            }
        }
    }

    #[test]
    fn needs_are_a_set() {
        let n = Needs::NONE
            .with(Need::Windowed, true)
            .with(Need::Pixels, true);
        assert!(n.has(Need::Windowed) && n.has(Need::Pixels) && !n.has(Need::Covered));
        assert!(n.any());
        let n = n.with(Need::Windowed, false).with(Need::Pixels, false);
        assert_eq!(n, Needs::NONE);
        assert!(!n.any());
        assert!(!Needs::NONE.with(Need::Resizing, false).any());
        // `ALL` is every need once, in bit order (the host's log walks it)
        for (bit, need) in Need::ALL.into_iter().enumerate() {
            assert_eq!(need as usize, bit);
        }
    }

    /// Any shell UI over the page, a leaving one included, is `Covered`.
    #[test]
    fn coverage_is_the_covered_need() {
        let covered = Needs::NONE.with(Need::Covered, true);
        #[rustfmt::skip]
        let rows: &[(&str, Needs, Coverage, Needs)] = &[
            ("nothing over the page", Needs::NONE, Coverage::None, Needs::NONE),
            ("shell pixels over it", Needs::NONE, Coverage::Part, covered),
            ("glass blurring it", Needs::NONE, Coverage::Glass, covered.with(Need::Frosted, true)),
            ("an entrance moving it", Needs::NONE, Coverage::Pixels, covered.with(Need::Sampled, true)),
            ("the glass gone, a cover stays", covered.with(Need::Frosted, true), Coverage::Part, covered),
            ("glass, then an entrance too", covered.with(Need::Frosted, true), Coverage::Pixels, covered.with(Need::Sampled, true)),
            ("the cover gone", covered, Coverage::None, Needs::NONE),
            ("others kept", Needs::NONE.with(Need::Pixels, true), Coverage::Part, covered.with(Need::Pixels, true)),
        ];
        for &(name, before, coverage, after) in rows {
            assert_eq!(before.covered(coverage), after, "{name}");
        }
    }

    #[test]
    fn the_copy_takes_the_free_slot_used_longest_ago() {
        type SlotRow = (&'static str, &'static [bool], Option<usize>, Option<usize>);
        #[rustfmt::skip]
        let rows: &[SlotRow] = &[
            ("the first frame", &[false; 4], None, Some(0)),
            ("after the latest", &[false; 4], Some(0), Some(1)),
            ("wraps around", &[false; 4], Some(3), Some(0)),
            ("skips the ones held", &[false, true, true, false], Some(0), Some(3)),
            ("never the latest, even free", &[true, false, true, true], Some(1), None),
            ("all held: the frame is dropped", &[true; 4], Some(2), None),
            ("all held, no latest", &[true; 4], None, None),
            ("one slot, latest: dropped", &[false], Some(0), None),
            ("one slot, free", &[false], None, Some(0)),
            ("no slots", &[], None, None),
        ];
        for &(name, in_use, latest, slot) in rows {
            assert_eq!(pick_slot(in_use, latest), slot, "{name}");
        }
    }

    #[test]
    fn every_holder_at_once_leaves_the_next_frame_a_slot() {
        // the slots held as a frame is copied; the latest is the one copied last
        #[rustfmt::skip]
        let rows: &[(&str, &[usize], usize)] = &[
            ("the layer's frame and the one before", &[3, 2], 3),
            ("a snapshot blitting the layer's frame", &[3, 2, 3], 3),
            ("the compositor still on the one before that", &[3, 2, 1], 3),
            ("a burst: the compositor holds two it was given", &[3, 2, 1, 0], 3),
            ("a burst with a snapshot in flight", &[3, 2, 1, 0, 3], 3),
            ("a frame not yet taken, the layer's two, the compositor's", &[4, 3, 2, 1], 4),
        ];
        for &(name, held, latest) in rows {
            let in_use: Vec<bool> = (0..POOL_SLOTS).map(|i| held.contains(&i)).collect();
            let slot = pick_slot(&in_use, Some(latest));
            assert!(slot.is_some_and(|s| !held.contains(&s)), "{name}");
        }
    }

    #[test]
    fn the_layer_covers_its_pane_and_shows_the_pane_s_part_of_the_buffer() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        type PlacementRow = (
            &'static str,
            Rect,
            f64,
            bool,
            f64,
            (u32, u32),
            (u32, u32),
            Rect,
            Rect,
        );
        #[rustfmt::skip]
        let rows: &[PlacementRow] = &[
            ("fullscreen at 2x, flipped", r(0.0, 0.0, 1512.0, 982.0), 982.0, true, 2.0,
                (3024, 1964), (3024, 1964), r(0.0, 0.0, 1512.0, 982.0), r(0.0, 0.0, 1.0, 1.0)),
            ("CEF painted a pixel more", r(0.0, 0.0, 800.0, 600.0), 600.0, true, 1.0,
                (801, 600), (800, 600), r(0.0, 0.0, 800.0, 600.0), r(0.0, 0.0, 800.0 / 801.0, 1.0)),
            ("unflipped: y from the bottom", r(10.0, 20.0, 100.0, 50.0), 200.0, false, 1.0,
                (100, 52), (100, 50), r(10.0, 130.0, 100.0, 50.0), r(0.0, 1.0 - 50.0 / 52.0, 1.0, 50.0 / 52.0)),
            ("snapped to whole pixels", r(0.3, 0.26, 99.9, 50.1), 100.0, true, 2.0,
                (200, 100), (200, 100), r(0.5, 0.5, 100.0, 50.0), r(0.0, 0.0, 1.0, 1.0)),
            ("a clip past the buffer is the whole buffer", r(0.0, 0.0, 10.0, 10.0), 10.0, true, 1.0,
                (5, 5), (10, 10), r(0.0, 0.0, 10.0, 10.0), r(0.0, 0.0, 1.0, 1.0)),
            ("an empty buffer shows its unit square", r(0.0, 0.0, 10.0, 10.0), 10.0, true, 1.0,
                (0, 0), (0, 0), r(0.0, 0.0, 10.0, 10.0), r(0.0, 0.0, 1.0, 1.0)),
        ];
        for &(name, pane, root_h, flipped, scale, buffer, clip, frame, contents) in rows {
            assert_eq!(
                place(pane, root_h, flipped, scale, buffer, clip),
                Placement {
                    frame,
                    contents,
                    scale
                },
                "{name}"
            );
        }
    }

    #[test]
    fn a_subsurface_covers_its_frame_in_whole_points() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        #[rustfmt::skip]
        let rows: &[(&str, Rect, [i32; 4])] = &[
            ("whole points as they are", r(220.0, 60.0, 1100.0, 760.0), [220, 60, 1100, 760]),
            ("at 1.25 a pane edge between points: rounded outwards", r(10.4, 20.8, 100.4, 50.0), [10, 20, 101, 51]),
            ("both edges inside the same points", r(0.2, 0.2, 0.4, 0.4), [0, 0, 1, 1]),
            ("empty at a whole point stays empty", r(5.0, 5.0, 0.0, 0.0), [5, 5, 0, 0]),
            ("negative origin (a pane sliding in)", r(-3.5, 0.0, 10.0, 10.0), [-4, 0, 11, 10]),
        ];
        for &(name, frame, want) in rows {
            assert_eq!(covering(frame), want, "{name}");
        }
    }

    #[test]
    fn the_shell_paces_its_own_presents_only_with_a_layer_on_linux() {
        #[rustfmt::skip]
        let rows = [
            ("linux with a layer: Mailbox, paced by the shell", "linux", true, super::Present::Paced),
            ("linux without one: vsync", "linux", false, super::Present::Vsync),
            ("macos with a layer: vsync (its transaction)", "macos", true, super::Present::Vsync),
            ("macos without one", "macos", false, super::Present::Vsync),
            ("a platform the table does not name", "freebsd", true, super::Present::Vsync),
        ];
        for (name, os, layers, want) in rows {
            assert_eq!(shell_present(os, layers), want, "{name}");
        }
    }

    #[test]
    fn the_flag_names_each_mode_and_defaults_off() {
        assert_eq!(mode("off"), Some(Mode::Off));
        assert_eq!(mode("fullscreen"), Some(Mode::Fullscreen));
        assert_eq!(mode("window"), Some(Mode::Window));
        assert_eq!(mode("on"), None);
        assert_eq!(Mode::default(), Mode::Off);
        #[rustfmt::skip]
        let platforms = [
            ("macos: under the shell", "macos", Mode::Window),
            ("linux: under the shell (Wayland; the setting is off by default there)", "linux", Mode::Window),
            ("windows: no layer", "windows", Mode::Off),
            ("one the table does not name", "freebsd", Mode::Off),
        ];
        for (name, os, want) in platforms {
            assert_eq!(platform_mode(os), want, "{name}");
        }
        // each row at its mode's index (`Mode::row` reads it so)
        for (i, row) in MODES.iter().enumerate() {
            assert_eq!(row.mode as usize, i, "{}", row.name);
        }
        assert_eq!(MODES.len(), Mode::Window as usize + 1);
        let orders: Vec<Order> = MODES.iter().map(|r| r.order).collect();
        assert_eq!(orders, [Order::Above, Order::Above, Order::Below]);
    }
}
