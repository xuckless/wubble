//! The scene's glass as data: every `GlassPanel` reports its [`GlassItem`] through
//! `GlassLayer.report`, and [`GlassRegistry`] keeps the live ones by key, back to front. The
//! driver asks it once per frame for the list, when it changed, and hands that to the host,
//! which passes it to the scene's `set_glass`. No widget calls into the scene.
//!
//! Slint has no "destroyed" callback, so a panel a `for` or `if` removes cannot take itself
//! out. After the models change the registry sweeps instead: at the next frame it forgets
//! everything and the driver bumps `GlassLayer.generation`, on which every live panel reports
//! again (and new ones report on `init`). The list is held back for that one frame, so the
//! scene keeps the previous glass instead of flashing empty.

use std::collections::BTreeMap;

use crate::GlassItem;

/// Where the registry is in a sweep. One explicit state, moved only by [`GlassRegistry`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sweep {
    Idle,
    /// The models changed; the next frame forgets the items and bumps the generation.
    Requested,
    /// Forgotten this frame; the panels report again before the next one.
    Holding,
}

/// What one frame does with the glass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlassFrame {
    /// Bump `GlassLayer.generation`: every live panel reports again.
    pub bump: bool,
    /// The new list, back to front, when it changed since the last frame.
    pub glass: Option<Vec<GlassItem>>,
}

#[derive(Debug)]
pub struct GlassRegistry {
    items: BTreeMap<i32, GlassItem>,
    dirty: bool,
    sweep: Sweep,
}

impl Default for GlassRegistry {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
            dirty: false,
            sweep: Sweep::Idle,
        }
    }
}

impl GlassRegistry {
    /// A panel's current item: kept when present, removed when not.
    pub fn report(&mut self, item: GlassItem) {
        if item.present {
            if self.items.get(&item.key) != Some(&item) {
                self.items.insert(item.key, item);
                self.dirty = true;
            }
        } else if self.items.remove(&item.key).is_some() {
            self.dirty = true;
        }
    }

    /// The models changed: sweep at the next frame.
    pub fn request_sweep(&mut self) {
        self.sweep = Sweep::Requested;
    }

    #[cfg(test)]
    fn sweep(&self) -> Sweep {
        self.sweep
    }

    /// The live items, back to front (by key).
    pub fn snapshot(&self) -> Vec<GlassItem> {
        self.items.values().cloned().collect()
    }

    pub fn get(&self, key: i32) -> Option<&GlassItem> {
        self.items.get(&key)
    }

    /// Once per rendered frame, before the host's hook.
    pub fn frame(&mut self) -> GlassFrame {
        match self.sweep {
            Sweep::Requested => {
                self.items.clear();
                self.dirty = true;
                self.sweep = Sweep::Holding;
                GlassFrame {
                    bump: true,
                    glass: None,
                }
            }
            Sweep::Holding | Sweep::Idle => {
                self.sweep = Sweep::Idle;
                let glass = std::mem::take(&mut self.dirty).then(|| self.snapshot());
                GlassFrame { bump: false, glass }
            }
        }
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

    fn keys(v: &[GlassItem]) -> Vec<i32> {
        v.iter().map(|i| i.key).collect()
    }

    #[test]
    fn items_are_kept_by_key_back_to_front() {
        let mut r = GlassRegistry::default();
        r.report(item(101, 0.0));
        r.report(item(0, 0.0));
        r.report(item(1, 0.0));
        assert_eq!(keys(&r.frame().glass.unwrap()), [0, 1, 101]);
        // nothing changed: no new list
        assert_eq!(r.frame(), GlassFrame::default());
        // the same item again changes nothing either
        r.report(item(1, 0.0));
        assert_eq!(r.frame().glass, None);
    }

    #[test]
    fn a_report_replaces_and_absence_removes() {
        let mut r = GlassRegistry::default();
        r.report(item(1, 0.0));
        r.report(item(1, 5.0));
        let g = r.frame().glass.unwrap();
        assert_eq!((g.len(), g[0].x), (1, 5.0));
        r.report(GlassItem {
            present: false,
            ..item(1, 5.0)
        });
        assert_eq!(r.frame().glass, Some(vec![]));
        // removing what is not there is no change
        r.report(GlassItem {
            present: false,
            ..item(7, 0.0)
        });
        assert_eq!(r.frame().glass, None);
    }

    #[test]
    fn a_sweep_holds_one_frame_then_shows_only_what_reported_again() {
        let mut r = GlassRegistry::default();
        r.report(item(0, 0.0));
        r.report(item(105, 0.0));
        r.frame();
        r.request_sweep();
        assert_eq!(r.sweep(), Sweep::Requested);
        // the sweep frame forgets everything, bumps, and keeps the scene's last list
        assert_eq!(
            r.frame(),
            GlassFrame {
                bump: true,
                glass: None
            }
        );
        assert_eq!(r.sweep(), Sweep::Holding);
        // only the sidebar is still alive and reports on the bump
        r.report(item(0, 0.0));
        assert_eq!(keys(&r.frame().glass.unwrap()), [0]);
        assert_eq!(r.sweep(), Sweep::Idle);
        // a sweep with no panel left still clears the scene
        r.request_sweep();
        r.frame();
        assert_eq!(r.frame().glass, Some(vec![]));
    }
}
