# Surface store

Part of the roadmap ([TODO.md](../../TODO.md)). Legend: `[ ]` open · `[~]` in progress. Done items are removed: `git log` has them.

One store for every shell surface, `wubble_core::overlay` ([docs/research/surface-store.md](../research/surface-store.md);
the owner calls it the "surface manager" in conversation; code and docs keep the current name for now).
Parts and state on `store/lane`, tested on Linux and macOS: the table, hover, page-attached, drag; of the overlays,
downloads and Spaces.

- [~] Overlays written blind (never compiled) on branch `untested/store-step2`, with the content map: extensions, menu, permission, dialog, find bar, sheets, region picker, link hints, notice, error and Boost panels: compile, test on Linux and macOS, merge
- [ ] The command bar through the store (not written)
- [ ] Focus and key ownership through the store (not written)
- [ ] Remove the `Env::others` / `Env::machine` bridge
- [ ] The surface lock's last 4 lines (`tests/surface_lock.rs` baseline)
- [ ] A host caller for `pointer_at`
- [ ] Gallery and `--shot` scenes as action lists
- [ ] Hand-check on Wayland and macOS the owner's decisions: Escape on an unmoved tab press no longer cancels the click; a lost window focus (pointer outside) cancels a tab drag
- [ ] After the store work, decide (owner, 2026-11-23: "add if we need to"): one despawn path for keyed host state, the one ECS idea worth having. Every per-tab and per-window table in the host is in one registered list that the single "tab gone" / "window gone" effect clears, with a test that fails when a keyed table is not in the list. Three bugs of this shape were found on 2026-11-23 (the clipboard read mark, the input-method flag, private site choices). No ECS framework: state stays one enum per lifecycle and one reducer
