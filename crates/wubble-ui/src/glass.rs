//! The scene's glass as data: every `GlassPanel` reports its [`GlassItem`] through
//! `GlassLayer.report`, and [`GlassRegistry`] holds the list the live ones make, back to front.
//! The driver hands that list to the host when it changed, which passes it to the scene's
//! `set_glass`. No widget calls into the scene.
//!
//! The list is derived whole on every rendered frame, never patched: the driver opens a sweep,
//! bumps `GlassLayer.generation` (every live panel on the scene reports again) and runs Slint's
//! change handlers, and the list is what the panels on screen reported in that sweep. So a key
//! several panels share (`GlassKeys.menu`: the context menu and every Select's menu) never has its
//! glass taken out by another panel's `present: false`, the order reports arrive in does not
//! matter, and a panel a `for` or `if` destroyed simply stops reporting. Slint has no "destroyed"
//! callback; this needs none. A report between frames only says whether the list is out of date,
//! so the driver asks for a frame.

use crate::GlassItem;

#[derive(Debug, Default)]
pub struct GlassRegistry {
    /// The last sweep's list, back to front.
    items: Vec<GlassItem>,
    /// The sweep in progress: what the live panels reported so far.
    sweep: Option<Vec<GlassItem>>,
    /// The list the host was last handed: a sweep that finds the same glass hands nothing, so it
    /// does not wake a frozen scene.
    sent: Option<Vec<GlassItem>>,
}

impl GlassRegistry {
    /// A panel's current item. In a sweep a present one joins the list (a panel reporting twice
    /// in one sweep, for its item and for the generation, counts once). Between sweeps it changes
    /// nothing and returns whether the list is out of date: a frame should sweep again.
    pub fn report(&mut self, item: GlassItem) -> bool {
        match &mut self.sweep {
            Some(list) => {
                if item.present && !list.contains(&item) {
                    list.push(item);
                }
                false
            }
            None if item.present => !self.items.contains(&item),
            None => self.items.iter().any(|i| i.key == item.key),
        }
    }

    /// The live items, back to front (by key).
    pub fn snapshot(&self) -> Vec<GlassItem> {
        self.items.clone()
    }

    pub fn get(&self, key: i32) -> Option<&GlassItem> {
        self.items.iter().find(|i| i.key == key)
    }

    /// Start a frame's sweep: until [`Self::end`], reports build the list from nothing.
    pub fn begin(&mut self) {
        self.sweep = Some(Vec::new());
    }

    /// End the sweep: its reports are the list. Returns the list when it differs from the one
    /// the host holds.
    pub fn end(&mut self) -> Option<Vec<GlassItem>> {
        let mut list = self.sweep.take()?;
        // back to front by key; panels sharing a key in a fixed order whatever order they
        // reported in, so the same glass is the same list
        list.sort_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.y.total_cmp(&b.y))
                .then(a.x.total_cmp(&b.x))
                .then(a.width.total_cmp(&b.width))
                .then(a.height.total_cmp(&b.height))
        });
        self.items = list;
        (self.sent.as_ref() != Some(&self.items)).then(|| {
            self.sent = Some(self.items.clone());
            self.items.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: i32, x: f32) -> GlassItem {
        GlassItem {
            key,
            x,
            width: 10.0,
            height: 10.0,
            present: true,
            ..Default::default()
        }
    }

    fn gone(key: i32, x: f32) -> GlassItem {
        GlassItem {
            present: false,
            ..item(key, x)
        }
    }

    fn keys(v: &[GlassItem]) -> Vec<i32> {
        v.iter().map(|i| i.key).collect()
    }

    /// One frame: the given panels report, in this order.
    fn sweep(r: &mut GlassRegistry, reports: &[GlassItem]) -> Option<Vec<GlassItem>> {
        r.begin();
        for i in reports {
            r.report(i.clone());
        }
        r.end()
    }

    #[test]
    fn the_list_is_what_reported_back_to_front() {
        let mut r = GlassRegistry::default();
        let list = sweep(&mut r, &[item(101, 0.0), item(0, 0.0), item(1, 0.0)]);
        assert_eq!(keys(&list.unwrap()), [0, 1, 101]);
        // the same glass again hands nothing
        assert_eq!(
            sweep(&mut r, &[item(0, 0.0), item(1, 0.0), item(101, 0.0)]),
            None
        );
        // a panel that no longer reports (destroyed) is gone
        assert_eq!(keys(&sweep(&mut r, &[item(0, 0.0)]).unwrap()), [0]);
        assert_eq!(sweep(&mut r, &[]), Some(vec![]));
    }

    /// The owner's missing menu glass: the context menu and a closed Select's menu share a key;
    /// the closed one's report, before or after, never takes the open one's glass out.
    #[test]
    fn a_shared_key_keeps_every_present_panel_in_any_order() {
        let mut r = GlassRegistry::default();
        let open = item(7, 40.0);
        for reports in [
            [open.clone(), gone(7, 999.0)],
            [gone(7, 999.0), open.clone()],
        ] {
            sweep(&mut r, &reports);
            assert_eq!(r.snapshot(), std::slice::from_ref(&open));
        }
        // two present at once (one fading out, one opening): both, in a fixed order
        let a = sweep(&mut r, &[item(7, 300.0), item(7, 40.0)]).unwrap();
        assert_eq!(a, [item(7, 40.0), item(7, 300.0)]);
        assert_eq!(sweep(&mut r, &[item(7, 40.0), item(7, 300.0)]), None);
    }

    #[test]
    fn a_panel_reporting_twice_in_a_sweep_counts_once() {
        let mut r = GlassRegistry::default();
        assert_eq!(
            sweep(&mut r, &[item(3, 1.0), item(3, 1.0)]).unwrap().len(),
            1
        );
    }

    #[test]
    fn a_report_between_frames_only_says_whether_to_sweep() {
        let mut r = GlassRegistry::default();
        sweep(&mut r, &[item(1, 0.0)]);
        // the same item, or a panel absent that holds no glass: nothing to do
        assert!(!r.report(item(1, 0.0)));
        assert!(!r.report(gone(9, 0.0)));
        // moved, new, or gone while its key holds glass: the next frame sweeps
        assert!(r.report(item(1, 5.0)));
        assert!(r.report(item(2, 0.0)));
        assert!(r.report(gone(1, 0.0)));
        // and the list waits for that sweep
        assert_eq!(r.snapshot(), [item(1, 0.0)]);
        assert_eq!(r.end(), None, "no sweep open");
    }
}
