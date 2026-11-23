//! The page on its own OS layer (macOS stage 1; decisions.md "Page layer"): a pane
//! whose frames go to a layer of the OS compositor (a `CALayer` whose contents is the frame's
//! IOSurface) instead of through the shell's presenter and a Slint render. A page-only frame then
//! costs no shell render at all.
//!
//! One [`Route`] per pane and one pure [`transition`]. What keeps the page on the shell's path
//! ([`Needs`]) is not stored here: the host derives it each time from the owners of that state
//! (the screen's fullscreen, the picture-in-picture window, the overlays, the pane's stretch, the
//! snapshots) and passes it in, so it has no second copy.
//!
//! Stage 1 has no hole in the shell: the layer sits over the shell's whole window content (page
//! fullscreen, the picture-in-picture window) and anything the shell must draw over the page
//! ([`Need::Covered`]) sends the page back to the shell. Stage 2 adds the hole as rows (a
//! windowed pane routed to the layer, with `Hole` effects), not as flags.
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
    /// The OS buffers were asked for; the shell draws the page until the first frame in one.
    Entering,
    /// The layer shows the page's frame of generation `shown`; the shell's presenter and render
    /// are skipped for it.
    Layer { shown: u64 },
    /// The shell was given the page's frame of generation `until` (or the layer's own, handed
    /// over); the layer stays visible until the shell presents a render that drew it.
    Leaving { until: u64 },
}

impl Route {
    /// The layer is visible (the invariant): the shell's drawable must land with its changes
    /// (`presentsWithTransaction` on macOS).
    pub fn on_layer(self) -> bool {
        matches!(self, Route::Layer { .. } | Route::Leaving { .. })
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

    /// This set with [`Need::Covered`] as the pane's `coverage` says.
    pub fn covered(self, coverage: Coverage) -> Needs {
        self.with(Need::Covered, coverage != Coverage::None)
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
        Needs::NONE
            .with(Need::Windowed, !(o.page_fullscreen || o.pip_window))
            .covered(o.coverage)
            .with(Need::Resizing, !o.view_live)
            .with(Need::Pixels, o.pixels)
    }
}

/// What the needs are derived from, each read from its one owner by the host on each call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owners {
    /// The pane's tab is its window's page fullscreen (`Screen::Page`).
    pub page_fullscreen: bool,
    /// The window is the picture-in-picture window.
    pub pip_window: bool,
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
    /// Hide the layer and drop its frame.
    Hide,
    /// This frame to the shell (today's path: presenter, damage, render).
    Present,
    /// The layer's latest frame to the shell, as `Present` does with a new one.
    PresentLatest,
    /// The shell renders and presents the page's area once more (no new frame may come).
    Redraw,
}

/// The route after `event` with `needs` holding now, and what the host does (in order). Every
/// (route, event) pair is an arm: a new behaviour is a new arm or route, not a check beside.
pub fn transition(route: Route, needs: Needs, event: Event) -> (Route, Vec<Effect>) {
    use {Effect::*, Event::*, Route::*};
    let blocked = needs.any();
    match (route, event) {
        // without a layer the shell draws every frame
        (Unsupported, Capable(false) | Changed | Presenting { .. } | Failed) => (route, vec![]),
        (Unsupported, Frame { .. }) => (route, vec![Present]),
        // a layer to use: the copy goes to OS buffers once nothing needs the shell
        (Unsupported | Composited, Capable(true) | Changed) if blocked => (Composited, vec![]),
        (Unsupported | Composited, Capable(true) | Changed) => (Entering, vec![Buffers(true)]),
        (Composited, Capable(false)) => (Unsupported, vec![]),
        (Composited, Frame { .. }) => (route, vec![Present]),
        // nothing is on the layer to fail
        (Composited, Presenting { .. } | Failed) => (route, vec![]),

        (Entering, Capable(false)) => (Unsupported, vec![Buffers(false)]),
        (Entering, Capable(true) | Changed) if blocked => (Composited, vec![Buffers(false)]),
        (Entering, Capable(true) | Changed | Presenting { .. }) => (route, vec![]),
        (Entering, Frame { os: true, .. }) if blocked => {
            (Composited, vec![Buffers(false), Present])
        }
        (Entering, Frame { os: true, painted }) => (Layer { shown: painted }, vec![Show]),
        // copied before the buffers were in place
        (Entering, Frame { os: false, .. }) => (route, vec![Present]),
        // nothing was shown yet
        (Entering, Failed) => (Unsupported, vec![Buffers(false)]),

        // the layer goes now: the window closes or the layer broke
        (Layer { .. }, Capable(false) | Failed) => {
            (Unsupported, vec![Buffers(false), PresentLatest, Hide])
        }
        (Layer { shown }, Capable(true) | Changed) if blocked => {
            (Leaving { until: shown }, vec![PresentLatest])
        }
        // a render of the shell's chrome (nothing of it shows over the page)
        (Layer { .. }, Capable(true) | Changed | Presenting { .. }) => (route, vec![]),
        (Layer { .. }, Frame { os: true, painted }) if blocked => {
            (Leaving { until: painted }, vec![Present])
        }
        (Layer { .. }, Frame { os: true, painted }) => (Layer { shown: painted }, vec![Show]),
        // the pool could not take it: the shell has to
        (Layer { .. }, Frame { os: false, painted }) => (Leaving { until: painted }, vec![Present]),

        (Leaving { .. }, Capable(false) | Failed) => (Unsupported, vec![Buffers(false), Hide]),
        // the needs are read again when the shell has drawn the page
        (Leaving { .. }, Capable(true) | Changed) => (route, vec![]),
        // a newer frame to the shell; the one it must draw before the layer goes stays
        (Leaving { .. }, Frame { .. }) => (route, vec![Present]),
        (Leaving { until }, Presenting { drawn: Some(d) }) if d >= until && blocked => {
            (Composited, vec![Hide, Buffers(false)])
        }
        // nothing needs the shell any more: the next frame goes back to the layer
        (Leaving { until }, Presenting { drawn: Some(d) }) if d >= until => (Entering, vec![Hide]),
        // the shell has not drawn it yet: once more, the layer stays
        (Leaving { .. }, Presenting { .. }) => (route, vec![Redraw]),
    }
}

/// OS buffers per view: the one on the layer, one the compositor may still read, one the shell
/// may still be sampling (`Entering`, `Leaving`), one to copy into.
pub const POOL_SLOTS: usize = 4;

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

/// `--page-layer`: which panes may go to a layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// None (the default): today's path everywhere.
    #[default]
    Off,
    /// A page in fullscreen and the picture-in-picture window (stage 1, macOS).
    Fullscreen,
}

/// The flag's values.
pub const MODES: &[(&str, Mode)] = &[("off", Mode::Off), ("fullscreen", Mode::Fullscreen)];

/// The mode `name` names.
pub fn mode(name: &str) -> Option<Mode> {
    MODES.iter().find_map(|&(n, m)| (n == name).then_some(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use {Effect::*, Event::*, Route::*};

    /// The layer on frame 5; leaving for its own frame 5, handed over, or for a newer 7.
    const ON5: Route = Layer { shown: 5 };
    const FOR5: Route = Leaving { until: 5 };
    const FOR7: Route = Leaving { until: 7 };

    fn needs(blocked: bool) -> Needs {
        Needs::NONE.with(Need::Covered, blocked)
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
        #[rustfmt::skip]
        let rows: &[(&str, Route, bool, Event, Route, &[Effect])] = &[
            // Unsupported: the shell draws every frame, nothing else happens
            ("unsupported: stays without a layer", Unsupported, false, Capable(false), Unsupported, &[]),
            ("unsupported: a layer, nothing needs the shell", Unsupported, false, Capable(true), Entering, &[Buffers(true)]),
            ("unsupported: a layer, the shell is needed", Unsupported, true, Capable(true), Composited, &[]),
            ("unsupported: needs ignored", Unsupported, false, Changed, Unsupported, &[]),
            ("unsupported: needs ignored (holding)", Unsupported, true, Changed, Unsupported, &[]),
            ("unsupported: a frame to the shell", Unsupported, false, copied(7), Unsupported, &[Present]),
            ("unsupported: an os frame to the shell", Unsupported, false, os(7), Unsupported, &[Present]),
            ("unsupported: a render ignored", Unsupported, false, drew(7), Unsupported, &[]),
            ("unsupported: a failure ignored", Unsupported, false, Failed, Unsupported, &[]),
            // Composited
            ("composited: loses the layer", Composited, false, Capable(false), Unsupported, &[]),
            ("composited: capable again, clear", Composited, false, Capable(true), Entering, &[Buffers(true)]),
            ("composited: capable again, needed", Composited, true, Capable(true), Composited, &[]),
            ("composited: the needs clear", Composited, false, Changed, Entering, &[Buffers(true)]),
            ("composited: still needed", Composited, true, Changed, Composited, &[]),
            ("composited: a frame to the shell", Composited, false, copied(7), Composited, &[Present]),
            ("composited: a late os frame to the shell", Composited, true, os(7), Composited, &[Present]),
            ("composited: a render ignored", Composited, false, drew(7), Composited, &[]),
            ("composited: nothing on the layer to fail", Composited, false, Failed, Composited, &[]),
            // Entering
            ("entering: loses the layer", Entering, false, Capable(false), Unsupported, &[Buffers(false)]),
            ("entering: capable again, clear", Entering, false, Capable(true), Entering, &[]),
            ("entering: capable again, needed", Entering, true, Capable(true), Composited, &[Buffers(false)]),
            ("entering: still clear", Entering, false, Changed, Entering, &[]),
            ("entering: needed again", Entering, true, Changed, Composited, &[Buffers(false)]),
            ("entering: the first os frame goes on the layer", Entering, false, os(7), Layer { shown: 7 }, &[Show]),
            ("entering: an os frame while needed", Entering, true, os(7), Composited, &[Buffers(false), Present]),
            ("entering: a frame copied before the buffers", Entering, false, copied(7), Entering, &[Present]),
            ("entering: a render ignored", Entering, false, BLANK, Entering, &[]),
            ("entering: a failure gives the layer up", Entering, false, Failed, Unsupported, &[Buffers(false)]),
            // Layer
            ("layer: loses the layer", ON5, false, Capable(false), Unsupported, &[Buffers(false), PresentLatest, Hide]),
            ("layer: capable again, clear", ON5, false, Capable(true), ON5, &[]),
            ("layer: capable again, needed: its frame to the shell", ON5, true, Capable(true), FOR5, &[PresentLatest]),
            ("layer: still clear", ON5, false, Changed, ON5, &[]),
            ("layer: shell UI over the page", ON5, true, Changed, FOR5, &[PresentLatest]),
            ("layer: the next frame", ON5, false, os(7), Layer { shown: 7 }, &[Show]),
            ("layer: a frame while needed waits to be drawn", ON5, true, os(7), FOR7, &[Present]),
            ("layer: a frame the pool could not take", ON5, false, copied(7), FOR7, &[Present]),
            ("layer: a render of the chrome", ON5, false, drew(5), ON5, &[]),
            ("layer: fails", ON5, false, Failed, Unsupported, &[Buffers(false), PresentLatest, Hide]),
            // Leaving: hidden only once a render drew the frame given (or a newer one)
            ("leaving: loses the layer", FOR5, false, Capable(false), Unsupported, &[Buffers(false), Hide]),
            ("leaving: capable again", FOR5, false, Capable(true), FOR5, &[]),
            ("leaving: the needs clear (read once drawn)", FOR5, false, Changed, FOR5, &[]),
            ("leaving: still needed", FOR5, true, Changed, FOR5, &[]),
            ("leaving: a newer frame to the shell, the same wait", FOR5, false, os(7), FOR5, &[Present]),
            ("leaving: a copied frame to the shell", FOR5, true, copied(7), FOR5, &[Present]),
            ("leaving: a render without the page: once more", FOR5, false, BLANK, FOR5, &[Redraw]),
            ("leaving: a render of an older frame: once more", FOR7, false, drew(5), FOR7, &[Redraw]),
            ("leaving: one short, still needed", FOR7, true, drew(6), FOR7, &[Redraw]),
            ("leaving: drawn, still needed", FOR5, true, drew(5), Composited, &[Hide, Buffers(false)]),
            ("leaving: drawn, nothing needs the shell", FOR5, false, drew(5), Entering, &[Hide]),
            ("leaving: a newer one drawn", FOR5, false, drew(7), Entering, &[Hide]),
            ("leaving: fails", FOR7, false, Failed, Unsupported, &[Buffers(false), Hide]),
        ];
        for &(name, route, blocked, event, next, effects) in rows {
            assert_eq!(
                transition(route, needs(blocked), event),
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
            // the window closed or the layer broke: it goes at once, drawn or not
            let forced = matches!(event, Capable(false) | Failed);
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
                            forced || !self.visible || self.drew >= self.layer,
                            "the layer hidden before the shell drew its frame"
                        );
                        self.visible = false;
                    }
                    Present => self.given = painted.max(self.given),
                    PresentLatest => self.given = self.layer.max(self.given),
                    Redraw => {}
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
        for &(name, walk) in walks {
            let (mut route, mut edge) = (Route::default(), Edge::default());
            for (i, &(blocked, event)) in walk.iter().enumerate() {
                let (next, effects) = transition(route, needs(blocked), event);
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
                    matches!(route, Entering | Layer { .. } | Leaving { .. }),
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
            ("fullscreen, glass over it", Owners { coverage: Coverage::Pixels, ..clear }, only(&[Covered])),
            ("the picture-in-picture window", Owners { page_fullscreen: false, pip_window: true, ..clear }, Needs::NONE),
            ("a pane among the chrome", Owners { page_fullscreen: false, ..clear }, only(&[Windowed])),
            ("a shot pending", Owners { pixels: true, ..clear }, only(&[Pixels])),
            ("everything", Owners { page_fullscreen: false, pip_window: false, coverage: Coverage::Part, view_live: false, pixels: true },
                only(&[Windowed, Covered, Resizing, Pixels])),
        ];
        for &(name, owners, want) in rows {
            assert_eq!(Needs::from_owners(&owners), want, "{name}");
        }
        let pixels = Needs::from_owners(&Owners {
            pixels: true,
            ..clear
        });
        assert_eq!(
            transition(ON5, pixels, Changed),
            (FOR5, vec![PresentLatest])
        );
    }

    #[test]
    fn page_layer_route_on_layer() {
        #[rustfmt::skip]
        let rows: &[(Route, bool)] = &[
            (Unsupported, false), (Composited, false), (Entering, false), (ON5, true), (FOR5, true),
        ];
        for &(route, on) in rows {
            assert_eq!(route.on_layer(), on, "{route:?}");
        }
    }

    /// Every surface that can appear over a fullscreen page (open, or still drawn as it leaves)
    /// sends the layer's frame to the shell before the layer goes; the page's own link hints
    /// do not.
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
        #[rustfmt::skip]
        let rows = [
            (VideoStrip, true), (VideoTab, true), (VideoSpeeds, true), (Toast, true), (Popup, true),
            (AutofillChip, true), (AutofillMenu, true), (AutofillPrompt, true), (Tooltip, true),
            (TabGhost, true), (PageGhost, true), (PipStrip, true), (Hints, false),
        ];
        for (kind, blocks) in rows {
            let layer = vec![Surface::new(kind, Some(tab))];
            let open = Stack {
                shown: Some(tab),
                layers: layer.clone(),
                leaving: vec![],
            };
            let leaving = Stack {
                shown: Some(tab),
                layers: vec![],
                leaving: layer,
            };
            for (state, stack) in [("open", open), ("leaving", leaving)] {
                let needs = Needs::NONE.covered(covers(&stack, &facts, tab));
                let want = if blocks {
                    leaves.clone()
                } else {
                    (ON5, vec![])
                };
                assert_eq!(transition(ON5, needs, Changed), want, "{kind:?} {state}");
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
    }

    /// Any shell UI over the page, a leaving one included, is `Covered`.
    #[test]
    fn coverage_is_the_covered_need() {
        let covered = Needs::NONE.with(Need::Covered, true);
        #[rustfmt::skip]
        let rows: &[(&str, Needs, Coverage, Needs)] = &[
            ("nothing over the page", Needs::NONE, Coverage::None, Needs::NONE),
            ("shell pixels over it", Needs::NONE, Coverage::Part, covered),
            ("glass reading it", Needs::NONE, Coverage::Pixels, covered),
            ("the cover gone", covered, Coverage::None, Needs::NONE),
            ("others kept", Needs::NONE.with(Need::Pixels, true), Coverage::Part, covered.with(Need::Pixels, true)),
        ];
        for &(name, before, coverage, after) in rows {
            assert_eq!(before.covered(coverage), after, "{name}");
        }
    }

    #[test]
    fn the_copy_takes_the_free_slot_used_longest_ago() {
        #[rustfmt::skip]
        let rows: &[(&str, &[bool], Option<usize>, Option<usize>)] = &[
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
    fn the_layer_covers_its_pane_and_shows_the_pane_s_part_of_the_buffer() {
        let r = |x, y, w, h| Rect { x, y, w, h };
        #[rustfmt::skip]
        let rows: &[(&str, Rect, f64, bool, f64, (u32, u32), (u32, u32), Rect, Rect)] = &[
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
    fn the_flag_names_each_mode_and_defaults_off() {
        assert_eq!(mode("off"), Some(Mode::Off));
        assert_eq!(mode("fullscreen"), Some(Mode::Fullscreen));
        assert_eq!(mode("on"), None);
        assert_eq!(Mode::default(), Mode::Off);
    }
}
