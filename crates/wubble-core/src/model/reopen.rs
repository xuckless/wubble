//! Recently-closed windows, folders and Spaces back in the model (`crate::closed`): the tabs of
//! a grouped entry come back in their order and places, asleep (Discarded, no page yet) except
//! the one shown, which `settle` restores. That is Chromium's lazy tab restore
//! (`TabRestoreServiceHelper::RestoreEntryById` opens a window's or group's tabs with only the
//! selected one loaded; BSD-3-Clause, behaviour only).

use super::{Dest, Effect, Model};
use crate::closed::{ClosedFolder, ClosedSpace, ClosedWindow, Slot, SpaceSnap, TabSnap};
use crate::space::{ContainerId, Folder, FolderId, Item, SpaceId, SpaceKind};
use crate::tab::{Liveness, TabId};
use crate::window::Shown;

impl Model {
    /// A sleeping tab last in `space`'s top level, in `container` if that still exists (else
    /// the Space's own). Its page loads when it is first shown.
    fn reopen_tab(&mut self, space: SpaceId, tab: &TabSnap) -> Option<TabId> {
        let container = Some(tab.container).filter(|&c| self.has_container(c));
        let (id, _, _create) = self.place_new_tab(tab.url.clone(), container, None, Some(space))?;
        let wall_ms = self.wall_ms;
        if let Some(t) = self.tabs.get_mut(&id) {
            t.title = tab.title.clone();
            t.lifecycle.liveness = Liveness::Discarded;
            t.last_active_ms = wall_ms;
        }
        Some(id)
    }

    /// A new, empty folder last in the Space at `si`.
    fn push_folder(&mut self, si: usize, name: String) -> FolderId {
        let id = FolderId(self.next_folder);
        self.next_folder += 1;
        self.spaces[si].items.push(Item::Folder(Folder {
            id,
            name,
            tabs: vec![],
            collapsed: false,
        }));
        id
    }

    /// `tabs` appended to `space`: pinned ones after its pinned tabs, folder ones into new
    /// folders (one per folder they came from), the rest at the end, all in order. Returns the
    /// tabs opened, in order, and the effects of placing them.
    fn reopen_tabs(&mut self, space: SpaceId, tabs: &[TabSnap]) -> (Vec<TabId>, Vec<Effect>) {
        let mut folders: Vec<(u32, FolderId)> = vec![];
        let mut opened = vec![];
        let mut fx = vec![];
        for snap in tabs {
            let Some(id) = self.reopen_tab(space, snap) else {
                continue;
            };
            opened.push(id);
            let to = match &snap.slot {
                Slot::Normal => continue,
                Slot::Pinned => Dest::Pinned {
                    space,
                    index: usize::MAX,
                },
                Slot::Folder { folder, name } => {
                    let found = folders.iter().find(|(f, _)| f == folder).map(|&(_, id)| id);
                    let id = match found {
                        Some(id) => id,
                        None => {
                            let Some(si) = self.space_idx(space) else {
                                continue;
                            };
                            let id = self.push_folder(si, name.clone());
                            folders.push((*folder, id));
                            id
                        }
                    };
                    Dest::Folder {
                        folder: id,
                        index: usize::MAX,
                    }
                }
            };
            fx.extend(self.move_tab(id, to));
        }
        (opened, fx)
    }

    /// A new Space built from `snap`, with its tabs; its active tab the first of them.
    fn rebuild_space(&mut self, snap: &SpaceSnap) -> (SpaceId, Vec<Effect>) {
        let container = if self.has_container(snap.container) {
            snap.container
        } else {
            ContainerId::DEFAULT
        };
        let id = self.add_space(snap.name.clone(), snap.tint, container);
        let si = self.space_idx(id).expect("the new Space");
        self.spaces[si].icon = snap.icon;
        let (_, fx) = self.reopen_tabs(id, &snap.tabs);
        let first = self.order_idx(si).first().copied();
        self.spaces[si].active = first;
        (id, fx)
    }

    /// `space` if it is still the user's (not gone, not an agent's or a private one).
    fn own_space(&self, space: Option<SpaceId>) -> Option<SpaceId> {
        space.filter(|&s| {
            self.space_idx(s)
                .is_some_and(|si| self.spaces[si].kind == SpaceKind::Normal)
        })
    }

    /// The window comes back on its Space at its geometry; the Space is built again when it is
    /// gone. A window already showing the Space is focused instead (one Space, one window).
    pub(super) fn reopen_window(&mut self, w: ClosedWindow) -> Vec<Effect> {
        let (space, mut fx) = match self.own_space(Some(w.space)) {
            Some(space) => (space, vec![]),
            None => self.rebuild_space(&w.snap),
        };
        if let Some(i) = self.shown_by(space) {
            fx.extend(self.focus(self.windows[i].id));
            return fx;
        }
        let id = self.push_window(Shown::Space(space), w.geometry);
        let si = self.space_idx(space).expect("space");
        fx.push(Effect::WindowOpened(id));
        fx.push(Effect::SpaceChanged(space));
        fx.push(self.show(self.spaces[si].active));
        fx
    }

    /// The Space comes back with its tabs and is shown.
    pub(super) fn reopen_space(&mut self, s: ClosedSpace) -> Vec<Effect> {
        let (space, mut fx) = self.rebuild_space(&s.snap);
        fx.extend(self.switch_space(space));
        fx
    }

    /// The folder comes back last in its Space (the active one when that is gone) with its tabs,
    /// and its first tab is shown.
    pub(super) fn reopen_folder(&mut self, f: ClosedFolder) -> Vec<Effect> {
        let space = self
            .own_space(f.space)
            .unwrap_or_else(|| self.active_space());
        let Some(si) = self.space_idx(space) else {
            return vec![];
        };
        let folder = self.push_folder(si, f.name);
        let mut fx = vec![];
        let mut first = None;
        for snap in &f.tabs {
            let Some(id) = self.reopen_tab(space, snap) else {
                continue;
            };
            first.get_or_insert(id);
            fx.extend(self.move_tab(
                id,
                Dest::Folder {
                    folder,
                    index: usize::MAX,
                },
            ));
        }
        match first {
            Some(tab) => fx.extend(self.activate_tab(tab)),
            // nothing could be opened: no empty folder is left behind
            None => fx.extend(self.delete_folder(folder)),
        }
        fx
    }

    /// "Close Folder": its tabs close and the folder goes with them.
    pub(super) fn close_folder(&mut self, folder: FolderId) -> Vec<Effect> {
        let Some(si) = self.folder_space(folder) else {
            return vec![];
        };
        let tabs = self.spaces[si]
            .folder_mut(folder)
            .map(|f| f.tabs.clone())
            .unwrap_or_default();
        let mut fx: Vec<Effect> = tabs.into_iter().flat_map(|t| self.close_tab(t)).collect();
        fx.extend(self.delete_folder(folder));
        fx
    }
}
