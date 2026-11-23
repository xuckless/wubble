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
//! never left over the shell once the shell draws the page again. The layer is hidden only after
//! the shell was given the page (a `Present` or `PresentLatest` since the last `Show`).

/// How many of the shell's renders the layer stays visible for once the shell has the page
/// again: one for a render already queued before the page was given to it, one for that render's
/// drawable to reach the screen (the shell's layer presents without a transaction). A guess to
/// check on the Mac (a flash of an old page frame when leaving fullscreen means more).
pub const LEAVING_RENDERS: u8 = 2;

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
    /// The layer shows the page; the shell's presenter and render are skipped for it.
    Layer,
    /// The shell was given the page again; the layer stays visible for `renders` more of the
    /// shell's renders ([`LEAVING_RENDERS`]).
    Leaving { renders: u8 },
}

/// A reason the shell must draw the page itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Need {
    /// The page is a pane among the shell's chrome, not the window's whole content (not page
    /// fullscreen, not the picture-in-picture window): stage 1 has no hole to show a layer
    /// through.
    Windowed,
    /// Shell UI over the page: the video controls, an overlay (`crate::overlay`), a toast, a
    /// popup. The layer would cover it.
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

    pub fn has(self, need: Need) -> bool {
        self.0 & (1 << need as u8) != 0
    }

    /// Whether the shell must draw the page.
    pub fn any(self) -> bool {
        self.0 != 0
    }
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
    /// The pane's view painted a frame; `os`: it was copied into an OS buffer.
    Frame { os: bool },
    /// The shell rendered and presented a frame of this window.
    Presented,
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

/// The route after `event` with `needs` holding now, and what the host does (in order).
pub fn transition(route: Route, needs: Needs, event: Event) -> (Route, Vec<Effect>) {
    use {Effect::*, Event::*, Route::*};
    let blocked = needs.any();
    let leaving = Leaving {
        renders: LEAVING_RENDERS,
    };
    match (route, event) {
        // without a layer the shell draws every frame
        (Unsupported, Capable(false) | Changed | Presented | Failed) => (route, vec![]),
        (Unsupported, Frame { .. }) => (route, vec![Present]),
        // a layer to use: the copy goes to OS buffers once nothing needs the shell
        (Unsupported | Composited, Capable(true) | Changed) if blocked => (Composited, vec![]),
        (Unsupported | Composited, Capable(true) | Changed) => (Entering, vec![Buffers(true)]),
        (Composited, Capable(false)) => (Unsupported, vec![]),
        (Composited, Frame { .. }) => (route, vec![Present]),
        // nothing is on the layer to fail
        (Composited, Presented | Failed) => (route, vec![]),

        (Entering, Capable(false)) => (Unsupported, vec![Buffers(false)]),
        (Entering, Capable(true) | Changed) if blocked => (Composited, vec![Buffers(false)]),
        (Entering, Capable(true) | Changed | Presented) => (route, vec![]),
        (Entering, Frame { os: true }) if blocked => (Composited, vec![Buffers(false), Present]),
        (Entering, Frame { os: true }) => (Layer, vec![Show]),
        // copied before the buffers were in place
        (Entering, Frame { os: false }) => (route, vec![Present]),
        // nothing was shown yet
        (Entering, Failed) => (Unsupported, vec![Buffers(false)]),

        // the layer goes now: the window closes or the layer broke
        (Layer, Capable(false) | Failed) => {
            (Unsupported, vec![Buffers(false), PresentLatest, Hide])
        }
        (Layer, Capable(true) | Changed) if blocked => (leaving, vec![PresentLatest]),
        // a render of the shell's chrome (nothing of it shows over the page)
        (Layer, Capable(true) | Changed | Presented) => (route, vec![]),
        (Layer, Frame { os: true }) if blocked => (leaving, vec![Present]),
        (Layer, Frame { os: true }) => (route, vec![Show]),
        // the pool could not take it: the shell has to
        (Layer, Frame { os: false }) => (leaving, vec![Present]),

        (Leaving { .. }, Capable(false) | Failed) => (Unsupported, vec![Buffers(false), Hide]),
        // the needs are read again when the shell has presented
        (Leaving { .. }, Capable(true) | Changed) => (route, vec![]),
        (Leaving { .. }, Frame { .. }) => (route, vec![Present]),
        (Leaving { renders }, Presented) if renders > 1 => (
            Leaving {
                renders: renders - 1,
            },
            vec![Redraw],
        ),
        (Leaving { .. }, Presented) if blocked => (Composited, vec![Hide, Buffers(false)]),
        // nothing needs the shell any more: the next frame goes back to the layer
        (Leaving { .. }, Presented) => (Entering, vec![Hide]),
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
    MODES
        .iter()
        .find_map(|&(n, m)| (n == name).then_some(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use {Effect::*, Event::*, Route::*};

    const L2: Route = Leaving {
        renders: LEAVING_RENDERS,
    };
    const L1: Route = Leaving { renders: 1 };

    fn needs(blocked: bool) -> Needs {
        Needs::NONE.with(Need::Covered, blocked)
    }

    /// Every route, each event, with the needs clear (false) and holding (true).
    #[test]
    fn every_route_and_event() {
        let os = Frame { os: true };
        let copied = Frame { os: false };
        #[rustfmt::skip]
        let rows: &[(&str, Route, bool, Event, Route, &[Effect])] = &[
            // Unsupported: the shell draws every frame, nothing else happens
            ("unsupported: stays without a layer", Unsupported, false, Capable(false), Unsupported, &[]),
            ("unsupported: a layer, nothing needs the shell", Unsupported, false, Capable(true), Entering, &[Buffers(true)]),
            ("unsupported: a layer, the shell is needed", Unsupported, true, Capable(true), Composited, &[]),
            ("unsupported: needs ignored", Unsupported, false, Changed, Unsupported, &[]),
            ("unsupported: needs ignored (holding)", Unsupported, true, Changed, Unsupported, &[]),
            ("unsupported: a frame to the shell", Unsupported, false, copied, Unsupported, &[Present]),
            ("unsupported: an os frame to the shell", Unsupported, false, os, Unsupported, &[Present]),
            ("unsupported: a render ignored", Unsupported, false, Presented, Unsupported, &[]),
            ("unsupported: a failure ignored", Unsupported, false, Failed, Unsupported, &[]),
            // Composited
            ("composited: loses the layer", Composited, false, Capable(false), Unsupported, &[]),
            ("composited: capable again, clear", Composited, false, Capable(true), Entering, &[Buffers(true)]),
            ("composited: capable again, needed", Composited, true, Capable(true), Composited, &[]),
            ("composited: the needs clear", Composited, false, Changed, Entering, &[Buffers(true)]),
            ("composited: still needed", Composited, true, Changed, Composited, &[]),
            ("composited: a frame to the shell", Composited, false, copied, Composited, &[Present]),
            ("composited: a late os frame to the shell", Composited, true, os, Composited, &[Present]),
            ("composited: a render ignored", Composited, false, Presented, Composited, &[]),
            ("composited: nothing on the layer to fail", Composited, false, Failed, Composited, &[]),
            // Entering
            ("entering: loses the layer", Entering, false, Capable(false), Unsupported, &[Buffers(false)]),
            ("entering: capable again, clear", Entering, false, Capable(true), Entering, &[]),
            ("entering: capable again, needed", Entering, true, Capable(true), Composited, &[Buffers(false)]),
            ("entering: still clear", Entering, false, Changed, Entering, &[]),
            ("entering: needed again", Entering, true, Changed, Composited, &[Buffers(false)]),
            ("entering: the first os frame goes on the layer", Entering, false, os, Layer, &[Show]),
            ("entering: an os frame while needed", Entering, true, os, Composited, &[Buffers(false), Present]),
            ("entering: a frame copied before the buffers", Entering, false, copied, Entering, &[Present]),
            ("entering: a render ignored", Entering, false, Presented, Entering, &[]),
            ("entering: a failure gives the layer up", Entering, false, Failed, Unsupported, &[Buffers(false)]),
            // Layer
            ("layer: loses the layer", Layer, false, Capable(false), Unsupported, &[Buffers(false), PresentLatest, Hide]),
            ("layer: capable again, clear", Layer, false, Capable(true), Layer, &[]),
            ("layer: capable again, needed", Layer, true, Capable(true), L2, &[PresentLatest]),
            ("layer: still clear", Layer, false, Changed, Layer, &[]),
            ("layer: shell UI over the page", Layer, true, Changed, L2, &[PresentLatest]),
            ("layer: the next frame", Layer, false, os, Layer, &[Show]),
            ("layer: a frame while needed", Layer, true, os, L2, &[Present]),
            ("layer: a frame the pool could not take", Layer, false, copied, L2, &[Present]),
            ("layer: a render of the chrome", Layer, false, Presented, Layer, &[]),
            ("layer: fails", Layer, false, Failed, Unsupported, &[Buffers(false), PresentLatest, Hide]),
            // Leaving
            ("leaving: loses the layer", L2, false, Capable(false), Unsupported, &[Buffers(false), Hide]),
            ("leaving: capable again", L2, false, Capable(true), L2, &[]),
            ("leaving: the needs clear (read when presented)", L2, false, Changed, L2, &[]),
            ("leaving: still needed", L2, true, Changed, L2, &[]),
            ("leaving: a frame to the shell", L2, false, os, L2, &[Present]),
            ("leaving: a copied frame to the shell", L2, true, copied, L2, &[Present]),
            ("leaving: one render more", L2, false, Presented, L1, &[Redraw]),
            ("leaving: done, still needed", L1, true, Presented, Composited, &[Hide, Buffers(false)]),
            ("leaving: done, nothing needs the shell", L1, false, Presented, Entering, &[Hide]),
            ("leaving: fails", L1, false, Failed, Unsupported, &[Buffers(false), Hide]),
        ];
        for &(name, route, blocked, event, next, effects) in rows {
            assert_eq!(
                transition(route, needs(blocked), event),
                (next, effects.to_vec()),
                "{name}"
            );
        }
    }

    /// What the host's edge holds as the effects run: whether the layer is visible, the copy
    /// uses OS buffers, and the shell was given the page since the layer last showed one.
    #[derive(Default)]
    struct Edge {
        visible: bool,
        buffers: bool,
        shell_fed: bool,
    }

    impl Edge {
        fn run(&mut self, effects: &[Effect]) {
            for effect in effects {
                match effect {
                    Buffers(on) => self.buffers = *on,
                    Show => {
                        assert!(self.buffers, "a frame on the layer without OS buffers");
                        self.visible = true;
                        self.shell_fed = false;
                    }
                    Hide => {
                        assert!(
                            !self.visible || self.shell_fed,
                            "the layer hidden before the shell has the page"
                        );
                        self.visible = false;
                    }
                    Present | PresentLatest => self.shell_fed = true,
                    Redraw => {}
                }
            }
        }
    }

    /// Event sequences, needs as they hold at each event: the layer is visible exactly in
    /// Layer and Leaving, and the OS buffers are asked for exactly while a layer is in play.
    #[test]
    fn the_layer_is_visible_whenever_the_shell_does_not_draw_the_page() {
        let os = Frame { os: true };
        let copied = Frame { os: false };
        let walks: &[(&str, &[(bool, Event)])] = &[
            (
                "fullscreen video, then its controls",
                &[
                    (true, Capable(true)),
                    (false, Changed),
                    (false, copied),
                    (false, os),
                    (false, os),
                    (true, Changed),
                    (true, os),
                    (true, Presented),
                    (true, Presented),
                    (true, os),
                ],
            ),
            (
                "the controls go again",
                &[
                    (false, Capable(true)),
                    (false, os),
                    (true, Changed),
                    (false, Changed),
                    (false, Presented),
                    (false, Presented),
                    (false, os),
                    (false, os),
                ],
            ),
            (
                "the pool runs dry, then the window closes",
                &[
                    (false, Capable(true)),
                    (false, os),
                    (false, copied),
                    (false, Presented),
                    (false, os),
                    (false, os),
                    (false, Capable(false)),
                    (false, os),
                ],
            ),
            (
                "the layer fails, and comes back",
                &[
                    (false, Capable(true)),
                    (false, os),
                    (false, Failed),
                    (false, os),
                    (false, Capable(true)),
                    (false, os),
                    (true, Capable(false)),
                ],
            ),
            (
                "a static page leaves fullscreen",
                &[
                    (false, Capable(true)),
                    (false, os),
                    (true, Changed),
                    (true, Presented),
                    (true, Presented),
                    (true, Presented),
                ],
            ),
        ];
        for &(name, walk) in walks {
            let (mut route, mut edge) = (Route::default(), Edge::default());
            for (i, &(blocked, event)) in walk.iter().enumerate() {
                let (next, effects) = transition(route, needs(blocked), event);
                edge.run(&effects);
                route = next;
                let at = format!("{name}, step {i} ({event:?} -> {route:?})");
                assert_eq!(
                    edge.visible,
                    matches!(route, Layer | Leaving { .. }),
                    "visible: {at}"
                );
                assert_eq!(
                    edge.buffers,
                    matches!(route, Entering | Layer | Leaving { .. }),
                    "buffers: {at}"
                );
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
                Placement { frame, contents, scale },
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
