# Surface manager (the surface store)

Legend: `[ ]` open · `[~]` in progress. How items move: [todo/README.md](../README.md).

One store for every shell surface ([docs/research/surface-store.md](../../docs/research/surface-store.md)). The owner
calls it the "surface manager" in conversation; code and docs keep `wubble_core::overlay` and "surface store".

## State (2026-11-23)
- On `store/lane`, tested on Linux and macOS: the table, hover, page-attached and drag parts; of the overlays, downloads and Spaces.
- On `untested/store-step2`, written blind (never compiled): the other overlays; the writer's doubt list is in `docs/research/surface-store.md` on that branch.
- Not written: the command bar and focus/key ownership through the store.
- Still there: the `Env` bridge; the surface lock's baseline is 4; `pointer_at` has no host caller.
- Decision pending: one despawn path for keyed host state (below).

## Open
- [~] Overlays written blind (never compiled) on branch `untested/store-step2`, with the content map: extensions, menu, permission, dialog, find bar, sheets, region picker, link hints, notice, error and Boost panels: compile, test on Linux and macOS, merge
- [ ] The command bar through the store (not written)
- [ ] Focus and key ownership through the store (not written)
- [ ] Remove the `Env::others` / `Env::machine` bridge
- [ ] The surface lock's last 4 lines (`tests/surface_lock.rs` baseline)
- [ ] A host caller for `pointer_at`
- [ ] Gallery and `--shot` scenes as action lists
- [ ] Hand-check on Wayland and macOS the owner's decisions: Escape on an unmoved tab press no longer cancels the click; a lost window focus (pointer outside) cancels a tab drag
- [ ] After the store work, decide (owner, 2026-11-23: "add if we need to"): one despawn path for keyed host state, the one ECS idea worth having. Every per-tab and per-window table in the host is in one registered list that the single "tab gone" / "window gone" effect clears, with a test that fails when a keyed table is not in the list. Three bugs of this shape were found on 2026-11-23 (the clipboard read mark, the input-method flag, private site choices). No ECS framework: state stays one enum per lifecycle and one reducer
