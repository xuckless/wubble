//! Where the pages on screen sit: one page fills the content area; a shown split is laid out by
//! the split physics (`wubble_core::split`, ported from the design repo's `browser/split.ts`),
//! which the host steps once per rendered frame. Pure: the host feeds it the model and the
//! content area's size (logical pixels) and executes the commands it returns.
//!
//! The glue is the design repo's `slint/driver.rs` `frame_split` (xuckless/the-wallpaper-project
//! @ 6c840da): sync the physics to the model's split, step it, turn what it settled into model
//! commands, write panes and seams. The focus ring is `browser/chrome.tsx` `split-layout`'s.

use wubble_core::bow::{BOWED, Curve, EdgeTable, edge_table};
use wubble_core::devtools::PANEL_TAB;
use wubble_core::model::{Command, Model};
use wubble_core::space::SpaceId;
use wubble_core::split::{Bow, SPLIT, Split, SplitEvent, string_line, string_rect};
use wubble_core::tab::TabId;
use wubble_engine::view::Layout;
use wubble_gfx::FrameSize;

use crate::physical;

/// The panes' corner radius and the strings' (split.ts: 14).
pub const RADIUS: f64 = 14.0;

/// One page's place in the content area, in logical pixels: `x..x + width`, full height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageRect {
    pub tab: TabId,
    pub x: f64,
    pub width: f64,
}

impl PageRect {
    /// The rect with both edges on whole physical pixels at `scale` (the content area's own
    /// left edge already is), so a pane's frame is shown 1:1 while a seam rests between pixels.
    pub fn on_pixels(self, scale: f32) -> Self {
        let snap = |x: f64| (x * f64::from(scale)).round() / f64::from(scale);
        let (x0, x1) = (snap(self.x), snap(self.x + self.width));
        Self {
            x: x0,
            width: x1 - x0,
            ..self
        }
    }

    /// The view's layout for this pane at `scale`: whole physical pixels, so a settled seam
    /// resizes nothing.
    pub fn layout(&self, height: f64, scale: f32) -> Layout {
        Layout {
            size: FrameSize::new(
                physical(self.width as f32, scale),
                physical(height as f32, scale),
            ),
            scale,
            area: None,
        }
    }
}

/// The width the pages share when the docked panel is `panel` wide (0: no panel).
pub fn page_width(width: f64, panel: f64) -> f64 {
    if panel <= 0.0 {
        width
    } else {
        (width - panel - SPLIT::gap).max(0.0)
    }
}

/// The split physics of the active Space, and the Space it was synced for.
#[derive(Debug, Default)]
pub struct Panes {
    split: Split,
    space: Option<SpaceId>,
    /// The physics settled something the model has not been told yet: until the host has
    /// dispatched it (`dispatched`), the physics is ahead of the model and does not adopt it.
    /// (driver.rs applies these to its model at once; the host dispatches them after rendering.)
    pending: bool,
}

/// The split's ids are the tabs' ids.
fn ids(tabs: &[TabId]) -> Vec<u64> {
    tabs.iter().map(|t| t.0).collect()
}

impl Panes {
    /// One frame of `dt` seconds over a content area of `width` x `height`: adopt the model's
    /// shown split (forgetting the layout on a Space switch or when none shows), step the strings,
    /// and return whether they still move and what they settled, as model commands.
    pub fn frame(
        &mut self,
        model: &Model,
        dt: f64,
        width: f64,
        height: f64,
    ) -> (bool, Vec<Command>) {
        let space = model.active_space;
        let Some(group) = model.shown_split() else {
            self.split.reset();
            self.space = None;
            return (false, vec![]);
        };
        if self.space != Some(space) {
            self.split.reset();
            self.space = Some(space);
        }
        // a release or even between frames has already settled new stops
        if !self.pending && self.split.events.is_empty() {
            self.split.sync(&ids(&group.tabs), &group.stops);
        }
        let moving = self.split.step(dt, width, height);
        let commands: Vec<Command> = std::mem::take(&mut self.split.events)
            .into_iter()
            .flat_map(|e| match e {
                SplitEvent::Stops(stops) => vec![Command::ResizeSplit { space, stops }],
                SplitEvent::Closed { id, stops } => vec![
                    Command::Unsplit(TabId(id)),
                    Command::ResizeSplit { space, stops },
                ],
            })
            .collect();
        self.pending |= !commands.is_empty();
        (moving, commands)
    }

    /// The host has dispatched every command `frame` returned: the model has caught up.
    pub fn dispatched(&mut self) {
        self.pending = false;
    }

    fn shown(&self) -> bool {
        self.space.is_some() && self.split.n() >= 2
    }

    /// Each page on screen (`tabs`, left to right) and its rect: the whole content area for one
    /// page, the physics' rect for a pane of the split. A pane with no width (closing) or not
    /// synced yet has none. When the physics has closed a split down to one page before the
    /// model unsplits it, that page is the one on screen.
    pub fn rects(&self, tabs: &[TabId], width: f64, panel: f64) -> Vec<PageRect> {
        if panel <= 0.0 {
            return self.page_rects(tabs, width);
        }
        // The docked panel takes the right edge; the pages share what is left of the width.
        let page_width = page_width(width, panel);
        let mut rects = self.page_rects(tabs, page_width);
        rects.push(PageRect {
            tab: PANEL_TAB,
            x: width - panel,
            width: panel,
        });
        rects
    }

    /// `rects` of the pages alone.
    fn page_rects(&self, tabs: &[TabId], width: f64) -> Vec<PageRect> {
        if !self.shown() {
            let page = match self.split.ids.as_slice() {
                [id] if self.space.is_some() => Some(TabId(*id)),
                _ => tabs.first().copied(),
            };
            return page
                .map(|tab| PageRect { tab, x: 0.0, width })
                .into_iter()
                .collect();
        }
        let panes = self.split.panes();
        tabs.iter()
            .filter_map(|&tab| {
                let p = panes.iter().find(|p| p.id == tab.0)?;
                Some(PageRect {
                    tab,
                    x: p.x0,
                    width: p.x1 - p.x0,
                })
            })
            .collect()
    }

    /// How many split seams are shown (the panel's seam comes after them).
    pub fn seam_count(&self) -> usize {
        if self.shown() { self.split.n() - 1 } else { 0 }
    }

    /// The seams: centre x and the bowed string while held or twanging ("" when straight).
    pub fn seams(&self, height: f64) -> Vec<(f64, String)> {
        if !self.shown() {
            return vec![];
        }
        self.split
            .seams()
            .into_iter()
            .enumerate()
            .map(|(j, (x, bend, gy))| {
                let path = if self.split.held == Some(j) || bend.abs() > BOWED {
                    string_line(x, height, RADIUS, Bow { bend, gy })
                } else {
                    String::new()
                };
                (x, path)
            })
            .collect()
    }

    /// The left and right edges of `tab`'s pane as split strings bend them, pane-local heights
    /// (the content area's, `height` high); `None` for an edge with no string or a straight one.
    /// The same curve `seams` draws.
    pub fn edges(&self, tab: TabId, height: f64) -> (Option<EdgeTable>, Option<EdgeTable>) {
        if !self.shown() {
            return (None, None);
        }
        let Some(p) = self.split.panes().into_iter().find(|p| p.id == tab.0) else {
            return (None, None);
        };
        let table = |b: Option<Bow>| {
            b.filter(|b| b.bend.abs() > BOWED)
                .map(|b| edge_table(Curve::Split, b.bend, b.gy, RADIUS, height - RADIUS, height))
        };
        (table(p.l), table(p.r))
    }

    /// The ring round the focused pane, following its bowed edges; "" for one page.
    pub fn ring(&self, focused: Option<TabId>, height: f64) -> String {
        let Some(f) = focused.filter(|_| self.shown()) else {
            return String::new();
        };
        self.split
            .panes()
            .into_iter()
            .find(|p| p.id == f.0)
            .map(|p| string_rect(p.x0 + 0.75, p.x1 - 0.75, height, RADIUS, p.l, p.r))
            .unwrap_or_default()
    }

    // The seams' gestures and a pane's close button (the physics owns them until they settle).

    pub fn grab(&mut self, seam: usize, x: f64, y: f64) {
        self.split.grab(seam, x, y);
    }

    pub fn drag(&mut self, x: f64, y: f64) {
        self.split.drag(x, y);
    }

    pub fn release(&mut self) {
        self.split.release();
    }

    pub fn even(&mut self) {
        if self.shown() {
            self.split.even();
        }
    }

    /// The pane closes as a drag would: its seam carries it shut, then the model unsplits it.
    pub fn close(&mut self, tab: TabId) {
        self.split.close(tab.0);
    }

    pub fn held(&self) -> bool {
        self.split.held.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use wubble_core::model::apply;

    #[test]
    fn pane_edges_land_on_physical_pixels() {
        let r = PageRect {
            tab: TabId(1),
            x: 300.3,
            width: 299.4,
        }
        .on_pixels(1.5);
        assert_eq!((r.x * 1.5, (r.x + r.width) * 1.5), (450.0, 900.0));
        let whole = PageRect {
            tab: TabId(1),
            x: 0.0,
            width: 640.0,
        };
        assert_eq!(whole.on_pixels(2.0), whole);
    }

    fn with_tabs(n: usize) -> Model {
        (0..n).fold(Model::new(Instant::now()), |m, i| {
            apply(
                m,
                Command::NewTab {
                    url: format!("https://{i}.test/"),
                    container: None,
                    opener: None,
                },
            )
            .0
        })
    }

    const DT: f64 = 1.0 / 60.0;

    /// Frames until the strings rest. Count-bounded.
    fn settle(p: &mut Panes, m: &Model, w: f64, h: f64) -> Vec<Command> {
        let mut out = vec![];
        for _ in 0..2000 {
            let (moving, commands) = p.frame(m, DT, w, h);
            out.extend(commands);
            if !moving {
                return out;
            }
        }
        panic!("never settled");
    }

    #[test]
    fn one_page_fills_the_content_area() {
        let m = with_tabs(2);
        let mut p = Panes::default();
        assert_eq!(p.frame(&m, DT, 1000.0, 700.0), (false, vec![]));
        let t = m.visible_tab().unwrap();
        assert_eq!(
            p.rects(&m.visible_tabs(), 1000.0, 0.0),
            vec![PageRect {
                tab: t,
                x: 0.0,
                width: 1000.0
            }]
        );
        assert!(p.seams(700.0).is_empty());
        assert_eq!(p.ring(Some(t), 700.0), "");
        assert!(p.rects(&[], 1000.0, 0.0).is_empty());
    }

    #[test]
    fn the_docked_panel_takes_the_right_edge_and_the_pages_what_is_left() {
        let m = with_tabs(2);
        let mut p = Panes::default();
        p.frame(&m, DT, page_width(1000.0, 400.0), 700.0);
        let t = m.visible_tab().unwrap();
        assert_eq!(page_width(1000.0, 0.0), 1000.0);
        assert_eq!(
            p.rects(&m.visible_tabs(), 1000.0, 400.0),
            vec![
                PageRect {
                    tab: t,
                    x: 0.0,
                    width: 1000.0 - 400.0 - SPLIT::gap
                },
                PageRect {
                    tab: PANEL_TAB,
                    x: 600.0,
                    width: 400.0
                },
            ]
        );
        // no room left: the page keeps no width, never a negative one
        assert_eq!(page_width(300.0, 400.0), 0.0);
    }

    #[test]
    fn a_split_lays_out_its_panes_at_their_stops() {
        let m = with_tabs(2);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let mut p = Panes::default();
        // the first frame places the seams at their stops, nothing animates
        let (moving, commands) = p.frame(&m, 0.0, 1008.0, 700.0);
        assert!(!moving && commands.is_empty());
        let rects = p.rects(&m.visible_tabs(), 1008.0, 0.0);
        let want = [(TabId(2), 0.0, 500.0), (TabId(1), 508.0, 500.0)]
            .map(|(tab, x, width)| PageRect { tab, x, width });
        assert_eq!(rects, want);
        assert_eq!(p.seams(700.0), vec![(504.0, String::new())]);
        assert!(p.ring(m.visible_tab(), 700.0).starts_with("M"));
        // physical pixels per pane
        let l = rects[1].layout(700.0, 1.5);
        assert_eq!((l.size.width, l.size.height), (750, 1050));
    }

    /// A held split seam bends both panes' facing edges along its string, the same way; at
    /// rest neither bends.
    #[test]
    fn a_bent_seam_bends_both_facing_edges() {
        let m = with_tabs(2);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1008.0, 700.0);
        let [a, b] = [TabId(2), TabId(1)];
        assert_eq!(p.edges(a, 700.0), (None, None));
        p.grab(0, 504.0, 300.0);
        p.drag(380.0, 300.0);
        for _ in 0..5 {
            p.frame(&m, DT, 1008.0, 700.0);
        }
        let (Some(right), None) = (p.edges(a, 700.0).1, p.edges(a, 700.0).0) else {
            panic!("the left pane's right edge bends")
        };
        let (Some(left), None) = p.edges(b, 700.0) else {
            panic!("the right pane's left edge bends")
        };
        assert_eq!(right, left);
        assert!(right.at(300.0) < -1.0, "bowed toward the pointer");
        let (x, path) = p.seams(700.0)[0].clone();
        assert!(!path.is_empty() && x > 380.0);
    }

    #[test]
    fn a_settled_drag_becomes_resize_split() {
        let m = with_tabs(2);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1000.0, 700.0);
        p.grab(0, 500.0, 300.0);
        p.drag(400.0, 300.0);
        assert!(p.held());
        for _ in 0..60 {
            p.frame(&m, DT, 1000.0, 700.0);
        }
        p.release();
        let commands = settle(&mut p, &m, 1000.0, 700.0);
        let [Command::ResizeSplit { space, stops }] = commands.as_slice() else {
            panic!("{commands:?}")
        };
        assert_eq!(*space, m.active_space);
        assert!((stops[0] - 0.4).abs() < 0.01, "{stops:?}");
    }

    #[test]
    fn closing_a_pane_unsplits_it_once_it_has_gone() {
        let m = with_tabs(3);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let (m, _) = apply(m, Command::SplitWith(TabId(2)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1200.0, 700.0);
        p.close(TabId(1));
        let commands = settle(&mut p, &m, 1200.0, 700.0);
        assert_eq!(commands[0], Command::Unsplit(TabId(1)));
        let m = commands.into_iter().fold(m, |m, c| apply(m, c).0);
        p.dispatched();
        assert_eq!(m.shown_split().unwrap().tabs.len(), 2);
        m.check().unwrap();
    }

    /// Until the model has the stops a release settled, the seam keeps heading for them (it
    /// sprang back toward the model's old stops for a frame).
    #[test]
    fn a_released_seam_keeps_its_stops_until_the_model_has_them() {
        let m = with_tabs(2);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1000.0, 700.0);
        p.grab(0, 500.0, 300.0);
        p.drag(400.0, 300.0);
        for _ in 0..60 {
            p.frame(&m, DT, 1000.0, 700.0);
        }
        p.release();
        let (_, commands) = p.frame(&m, DT, 1000.0, 700.0);
        assert_eq!(commands.len(), 1);
        // the model is not told yet: more frames, the same old model
        for _ in 0..3 {
            p.frame(&m, DT, 1000.0, 700.0);
            assert!((p.split.stops[0] - 0.4).abs() < 0.01, "{:?}", p.split.stops);
        }
        let m = commands.into_iter().fold(m, |m, c| apply(m, c).0);
        p.dispatched();
        p.frame(&m, DT, 1000.0, 700.0);
        assert!((p.split.stops[0] - 0.4).abs() < 0.01, "{:?}", p.split.stops);
    }

    /// Closing one page of two: once it has gone, the other fills the content area, before
    /// the model unsplits (the closed page's view was laid out full width for a frame).
    #[test]
    fn closing_to_one_page_shows_the_page_left() {
        let m = with_tabs(2);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1000.0, 700.0);
        let [left, right] = m.visible_tabs()[..] else {
            panic!()
        };
        p.close(left);
        let commands = settle(&mut p, &m, 1000.0, 700.0);
        assert_eq!(commands[0], Command::Unsplit(left));
        assert_eq!(
            p.rects(&m.visible_tabs(), 1000.0, 0.0),
            vec![PageRect {
                tab: right,
                x: 0.0,
                width: 1000.0
            }]
        );
    }

    /// A pane leaving the split while the seam beside it is held: letting go is harmless (it
    /// indexed past the seams and panicked).
    #[test]
    fn a_pane_leaving_while_its_seam_is_held() {
        let m = with_tabs(3);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let (m, _) = apply(m, Command::SplitWith(TabId(2)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1200.0, 700.0);
        p.grab(1, 800.0, 300.0);
        let last = *m.visible_tabs().last().unwrap();
        let (m, _) = apply(m, Command::Unsplit(last));
        p.frame(&m, DT, 1200.0, 700.0);
        assert!(!p.held());
        p.release();
        assert!(settle(&mut p, &m, 1200.0, 700.0).is_empty());
    }

    #[test]
    fn switching_space_forgets_the_layout() {
        let m = with_tabs(2);
        let (m, _) = apply(m, Command::SplitWith(TabId(1)));
        let mut p = Panes::default();
        p.frame(&m, 0.0, 1000.0, 700.0);
        let (m, _) = apply(m, Command::ActivateTab(TabId(1)));
        // still the split (TabId(1) is a pane): nothing to forget
        assert_eq!(p.rects(&m.visible_tabs(), 1000.0, 0.0).len(), 2);
        let (m, _) = apply(
            m,
            Command::NewSpace {
                name: "B".into(),
                tint: wubble_core::space::Rgb(0, 0, 0),
                container: wubble_core::space::ContainerId::DEFAULT,
            },
        );
        let (m, _) = apply(m, Command::SwitchSpace(SpaceId(1)));
        assert_eq!(p.frame(&m, DT, 1000.0, 700.0), (false, vec![]));
        assert!(p.seams(700.0).is_empty());
    }
}
